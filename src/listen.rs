//! The runnable pipeline so far: capture → VAD → ASR → log.
//!
//! This thread is the pipeline thread of SPEC §11: it owns the VAD and the
//! recognizer. Translation and TTS get their own thread from Milestone 3, so a
//! slow LLM cannot stall recognition.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info, warn};

use crate::asr::{self, AsrEngine, SegmentAsr};
use crate::audio::{self, SAMPLE_RATE};
use crate::compare;
use crate::config::Config;
use crate::models::{self, Engine, Entry, Role};
use crate::paths;
use crate::ring::{Utterance, UtteranceRing, DEFAULT_CAPACITY};
use crate::translate::{LlamaTranslator, Translator};
use crate::vad::{Segment, Segmenter, VadSettings};
use crate::wav;

/// Chunks of 16 kHz audio queued between capture and the VAD.
const PIPELINE_QUEUE_CHUNKS: usize = 64;

/// How often the idle loop wakes to check whether it is time to stop.
const POLL: Duration = Duration::from_millis(100);

/// How often to report the input level while nothing is being said. Without
/// this, a muted microphone and a silent room look identical.
const LEVEL_REPORT: Duration = Duration::from_secs(5);

/// Transcripts queued for translation. Short on purpose: if the translator
/// falls this far behind, the conversation has already moved on and saying so
/// is better than growing a backlog.
const TRANSLATION_QUEUE: usize = 4;

pub fn run(
    root: &Path,
    config: &Config,
    seconds: Option<u64>,
    write_wav: bool,
    compare_engines: bool,
) -> Result<()> {
    let engines = discovered_engines(root);
    let language = config.languages.source.clone();
    let selected_name = config.asr.engine.trim().to_string();

    // Resolve the selection before opening the microphone: a missing or
    // half-extracted model should fail before the device lights up.
    check_selection(&engines, &selected_name)?;

    // In comparison mode every engine runs on every utterance, the configured
    // one included, so loading it separately would hold a second copy of the
    // same model in memory for nothing.
    let (mut selected, mut comparison_engines) = if compare_engines {
        let loaded = asr::load_all_segment_engines(&engines);
        if loaded.len() < 2 {
            warn!(
                "--compare has only {} usable segment engine(s); the comparison needs at least two",
                loaded.len()
            );
        }
        (None, loaded)
    } else {
        (load_selected(&engines, &selected_name)?, Vec::new())
    };

    // No translator in comparison mode: with several transcripts of the same
    // utterance there is no single one to translate, and --compare is a
    // recognizer harness (SPEC §12), not the conversation path.
    let translation = if compare_engines || selected.is_none() {
        None
    } else {
        let model = models::find_translation_model(&paths::mt_dir(root))?;
        Some(spawn_translator(
            &model,
            config.languages.source.clone(),
            config.languages.target.clone(),
        )?)
    };
    let translate_tx = translation.as_ref().map(|(tx, _)| tx.clone());

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

    let mut ring = UtteranceRing::new(DEFAULT_CAPACITY);

    let (tx, rx) = sync_channel::<Vec<f32>>(PIPELINE_QUEUE_CHUNKS);
    let capture = audio::spawn_capture(&config.audio.input_device, tx)?;

    match seconds {
        Some(n) => info!("listening for {n} s — speak now"),
        None => info!("listening — speak now; Ctrl-C to stop"),
    }

    let started = Instant::now();
    let deadline = seconds.map(|n| started + Duration::from_secs(n));
    let mut level = LevelMeter::new();

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }

        match rx.recv_timeout(POLL) {
            Ok(chunk) => {
                level.observe(&chunk);
                for segment in segmenter.push(&chunk) {
                    handle(
                        segment,
                        &mut ring,
                        selected.as_mut(),
                        &mut comparison_engines,
                        &selected_name,
                        &language,
                        translate_tx.as_ref(),
                        write_wav.then_some(&segments_dir),
                    );
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
        handle(
            segment,
            &mut ring,
            selected.as_mut(),
            &mut comparison_engines,
            &selected_name,
            &language,
            translate_tx.as_ref(),
            write_wav.then_some(&segments_dir),
        );
    }

    let dropped = capture.dropped_chunks();
    capture.stop();

    // Close the queue and let the translator finish what it already has.
    drop(translate_tx);
    if let Some((tx, thread)) = translation {
        drop(tx);
        let _ = thread.join();
    }

    info!(
        "listened for {:.1} s, {} utterance(s) retained, dropped {dropped} chunk(s)",
        started.elapsed().as_secs_f32(),
        ring.len()
    );
    if ring.len() == 0 {
        info!(
            "no speech detected. If that is wrong, check --devices, check the level reports \
             above, and consider lowering [vad].threshold from {}.",
            config.vad.threshold
        );
    }
    Ok(())
}

/// One transcript on its way to the translator.
struct ToTranslate {
    index: usize,
    text: String,
}

/// The translation stage runs on its own thread (SPEC §11): a slow token
/// stream must never stall recognition. The model is loaded here, on the
/// caller's thread, so a missing or broken GGUF is an error from this call
/// rather than a thread that quietly dies later.
fn spawn_translator(
    model: &Path,
    source: String,
    target: String,
) -> Result<(SyncSender<ToTranslate>, std::thread::JoinHandle<()>)> {
    let mut translator = LlamaTranslator::load(model)?;
    let (tx, rx) = sync_channel::<ToTranslate>(TRANSLATION_QUEUE);

    let thread = std::thread::Builder::new()
        .name("convers-translate".to_string())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                let began = Instant::now();
                match translator.translate(&job.text, &source, &target) {
                    Ok(text) if text.is_empty() => {
                        warn!("utterance {}: translated to nothing", job.index)
                    }
                    Ok(text) => info!(
                        "utterance {}\n  [{source}] {}\n  [{target}] {text}\n  ({} ms to \
                         translate)",
                        job.index,
                        job.text,
                        began.elapsed().as_millis()
                    ),
                    Err(e) => warn!("utterance {}: translation failed: {e:#}", job.index),
                }
            }
            info!("translation stopped");
        })
        .context("cannot spawn the translation thread")?;

    Ok((tx, thread))
}

/// Transcribe one utterance, keep it, and report it.
#[allow(clippy::too_many_arguments)]
fn handle(
    segment: Segment,
    ring: &mut UtteranceRing,
    selected: Option<&mut Box<dyn SegmentAsr + Send>>,
    comparison_engines: &mut [(String, Box<dyn SegmentAsr + Send>)],
    selected_name: &str,
    language: &str,
    translate_tx: Option<&SyncSender<ToTranslate>>,
    segments_dir: Option<&PathBuf>,
) {
    let (start_ms, end_ms, duration_ms) =
        (segment.start_ms(), segment.end_ms(), segment.duration_ms());
    let utterance = ring.push(start_ms, segment.samples);

    info!(
        "utterance {}: {start_ms} ms .. {end_ms} ms ({duration_ms} ms)",
        utterance.index
    );

    if let Some(asr) = selected {
        let began = Instant::now();
        match asr.transcribe(&utterance.pcm, language) {
            // A recognizer finding no words is normal: the VAD cuts on energy,
            // so a cough or a chair passes its threshold. Say so quietly rather
            // than emitting an empty transcript that every later stage — the
            // translator above all — would have to special-case.
            Ok(text) if text.trim().is_empty() => debug!(
                "  no words in {} ms of audio ({} ms to decide)",
                utterance.duration_ms(),
                began.elapsed().as_millis()
            ),
            // The segment duration travels with the timing, always: Whisper
            // pads to 30 s internally and the number is meaningless alone
            // (SPEC §10).
            Ok(text) => {
                info!(
                    "  [{language}] {text}\n  ({} ms to transcribe {} ms of audio)",
                    began.elapsed().as_millis(),
                    utterance.duration_ms()
                );
                if let Some(tx) = translate_tx {
                    // Never block the pipeline thread on the translator.
                    match tx.try_send(ToTranslate {
                        index: utterance.index,
                        text,
                    }) {
                        Ok(()) => {}
                        Err(TrySendError::Full(job)) => warn!(
                            "translation is behind; utterance {} not translated",
                            job.index
                        ),
                        Err(TrySendError::Disconnected(_)) => {
                            warn!("the translation thread has stopped")
                        }
                    }
                }
            }
            Err(e) => warn!("  transcription failed: {e:#}"),
        }
    }

    if !comparison_engines.is_empty() {
        let comparison = compare::run_all(comparison_engines, &utterance, language);
        compare::report(&comparison, selected_name);
    }

    write_segment(&utterance, segments_dir);
}

fn write_segment(utterance: &Utterance, segments_dir: Option<&PathBuf>) {
    let Some(dir) = segments_dir else {
        return;
    };
    // Named by its timestamp as well as its index: the ring is keyed by when
    // the utterance happened, and the files should match (SPEC §12).
    let path = dir.join(format!(
        "utterance-{:04}-at-{}ms-for-{}ms.wav",
        utterance.index,
        utterance.start_ms,
        utterance.duration_ms()
    ));
    match wav::write_16k_mono(&path, &utterance.pcm) {
        Ok(()) => info!("  wrote {}", path.display()),
        Err(e) => warn!("  {e:#}"),
    }
}

/// Every ASR directory that parsed, whether or not its files are all present.
fn discovered_engines(root: &Path) -> Vec<Engine> {
    models::discover(&paths::asr_dir(root), Role::Asr)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Loaded(engine) => Some(engine),
            Entry::Failed { .. } => None,
        })
        .collect()
}

/// Resolve `[asr].engine` to a discovered model. Never substitutes another one
/// (SPEC §15).
fn find_selected<'a>(engines: &'a [Engine], selected: &str) -> Result<&'a Engine> {
    engines
        .iter()
        .find(|e| e.dir_name == selected)
        .ok_or_else(|| {
            anyhow!(
                "[asr].engine = \"{selected}\" was not found. Discovered: {}",
                if engines.is_empty() {
                    "nothing".to_string()
                } else {
                    engines
                        .iter()
                        .map(|e| e.dir_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )
        })
}

/// Fail on a selection that cannot work, without paying to load it first. An
/// unset selection is allowed: it listens without transcribing, which is the
/// Milestone 1 behaviour and still the way to check a microphone.
fn check_selection(engines: &[Engine], selected: &str) -> Result<()> {
    if selected.is_empty() {
        warn!("[asr].engine is unset; listening without transcribing");
        return Ok(());
    }
    let engine = find_selected(engines, selected)?;
    let missing = engine.missing_files();
    if !missing.is_empty() {
        return Err(anyhow!(
            "[asr].engine = \"{selected}\" is incomplete; these files are not in {}: {}",
            engine.dir.display(),
            missing.join(", ")
        ));
    }
    Ok(())
}

/// Load the configured engine for ordinary (non-comparison) listening.
fn load_selected(engines: &[Engine], selected: &str) -> Result<Option<Box<dyn SegmentAsr + Send>>> {
    if selected.is_empty() {
        return Ok(None);
    }
    let engine = find_selected(engines, selected)?;

    let began = Instant::now();
    match asr::load(engine)? {
        AsrEngine::Segment(asr) => {
            info!(
                "loaded \"{}\" ({}) in {} ms",
                engine.dir_name,
                engine.name,
                began.elapsed().as_millis()
            );
            Ok(Some(asr))
        }
        AsrEngine::Stream(_) => Err(anyhow!(
            "\"{}\" is a streaming engine; streaming arrives in Milestone 8",
            engine.dir_name
        )),
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
            warn!(
                "input level: digital silence over the last {seconds:.0} s — is the microphone \
                 muted?"
            );
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
