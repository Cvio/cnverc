//! Milestone 1's runnable path: capture → VAD → log.
//!
//! This is the shape the pipeline thread will keep. It owns the VAD, reads
//! 16 kHz mono chunks from the capture thread, and reports each utterance the
//! detector cuts. From Milestone 2 the utterances go to an ASR engine instead
//! of only to the log.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::audio::{self, SAMPLE_RATE};
use crate::config::Config;
use crate::paths;
use crate::vad::{Segment, Segmenter, VadSettings};
use crate::wav;

/// Chunks of 16 kHz audio queued between capture and the VAD.
const PIPELINE_QUEUE_CHUNKS: usize = 64;

/// How often the idle loop wakes to check whether it is time to stop.
const POLL: Duration = Duration::from_millis(100);

/// How often to report the input level while nothing is being said. Without
/// this, a muted microphone and a silent room look identical.
const LEVEL_REPORT: Duration = Duration::from_secs(5);

pub fn run(root: &Path, config: &Config, seconds: Option<u64>, write_wav: bool) -> Result<()> {
    let settings = VadSettings {
        model: paths::vad_model_file(root),
        threshold: config.vad.threshold,
        min_silence_ms: config.vad.min_silence_ms,
        min_speech_ms: config.vad.min_speech_ms,
    };
    let mut segmenter = Segmenter::new(&settings)?;
    info!(
        "VAD ready: threshold {}, min silence {} ms, min speech {} ms",
        settings.threshold, settings.min_silence_ms, settings.min_speech_ms
    );

    let segments_dir = paths::logs_dir(root).join("segments");
    if write_wav {
        std::fs::create_dir_all(&segments_dir)
            .with_context(|| format!("cannot create {}", segments_dir.display()))?;
        info!("writing utterances to {}", segments_dir.display());
    }

    let (tx, rx) = sync_channel::<Vec<f32>>(PIPELINE_QUEUE_CHUNKS);
    let capture = audio::spawn_capture(&config.audio.input_device, tx)?;

    match seconds {
        Some(n) => info!("listening for {n} s — speak now"),
        None => info!("listening — speak now; Ctrl-C to stop"),
    }

    let started = Instant::now();
    let deadline = seconds.map(|n| started + Duration::from_secs(n));
    let mut level = LevelMeter::new();
    let mut count = 0usize;

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }

        match rx.recv_timeout(POLL) {
            Ok(chunk) => {
                level.observe(&chunk);
                for segment in segmenter.push(&chunk) {
                    count += 1;
                    report(count, &segment, write_wav.then_some(&segments_dir));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                warn!("the capture thread stopped");
                break;
            }
        }

        level.report_if_due();
    }

    // Whatever was still being spoken when time ran out.
    for segment in segmenter.flush() {
        count += 1;
        report(count, &segment, write_wav.then_some(&segments_dir));
    }

    let dropped = capture.dropped_chunks();
    capture.stop();

    info!(
        "listened for {:.1} s, detected {count} utterance(s), dropped {dropped} chunk(s)",
        started.elapsed().as_secs_f32()
    );
    if count == 0 {
        info!(
            "no speech detected. If that is wrong, check --devices, check the level reports \
             above, and consider lowering [vad].threshold from {}.",
            config.vad.threshold
        );
    }
    Ok(())
}

fn report(index: usize, segment: &Segment, segments_dir: Option<&PathBuf>) {
    info!(
        "utterance {index}: {} ms .. {} ms ({} ms)",
        segment.start_ms(),
        segment.end_ms(),
        segment.duration_ms()
    );
    let Some(dir) = segments_dir else {
        return;
    };
    let path = dir.join(format!(
        "utterance-{index:04}-{}ms.wav",
        segment.duration_ms()
    ));
    match wav::write_16k_mono(&path, &segment.samples) {
        Ok(()) => info!("  wrote {}", path.display()),
        Err(e) => warn!("  {e:#}"),
    }
}

/// Peak level over the last reporting window, in dBFS.
struct LevelMeter {
    peak: f32,
    samples: u64,
    last_report: Instant,
}

impl LevelMeter {
    fn new() -> Self {
        Self {
            peak: 0.0,
            samples: 0,
            last_report: Instant::now(),
        }
    }

    fn observe(&mut self, chunk: &[f32]) {
        for sample in chunk {
            let magnitude = sample.abs();
            if magnitude > self.peak {
                self.peak = magnitude;
            }
        }
        self.samples += chunk.len() as u64;
    }

    fn report_if_due(&mut self) {
        if self.last_report.elapsed() < LEVEL_REPORT {
            return;
        }
        let seconds = self.samples as f32 / SAMPLE_RATE as f32;
        if self.peak <= 0.0 {
            warn!("input level: digital silence over the last {seconds:.0} s — is the microphone muted?");
        } else {
            info!(
                "input level: peak {:.1} dBFS over the last {seconds:.0} s",
                20.0 * self.peak.log10()
            );
        }
        self.peak = 0.0;
        self.samples = 0;
        self.last_report = Instant::now();
    }
}
