//! Voice activity detection: Silero through sherpa-onnx.
//!
//! sherpa's `VoiceActivityDetector` does the segmenting itself — it is fed
//! fixed windows and hands back complete speech segments with their start
//! offset — so convers does not run a second segmentation state machine on top
//! of it. The `[vad]` keys in `convers.toml` map straight onto its config.
//!
//! In turn-based mode (Milestone 6) this same detector is used only to trim
//! leading and trailing silence, not to decide boundaries. That is a question
//! of who calls it, not of a different detector.
//!
//! One thing is added on top: a pre-roll. The detector reports a segment from
//! the point where it became confident someone was speaking, and a soft onset
//! comes before that point: the breath of "Hola", the "¿D" of "¿Dónde", a
//! short quiet "No". Live sessions lost exactly those words, and a lost "No"
//! reverses the meaning of what was said. So the audio just before each
//! segment is kept and put back on the front of it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

use crate::audio::SAMPLE_RATE;

/// Samples per call into the detector. Silero is trained on 512-sample windows
/// at 16 kHz and the upstream examples feed exactly that.
const WINDOW: usize = 512;

/// Seconds of audio the detector may buffer internally.
const BUFFER_SECONDS: f32 = 30.0;

/// Longest single segment before the detector cuts one anyway. A speaker who
/// never pauses would otherwise grow an unbounded buffer and produce no
/// transcript at all.
const MAX_SPEECH_SECONDS: f32 = 20.0;

/// Audio restored in front of each segment. Recordings from a live session
/// began mid-word on "¿Dónde"; 300 ms covers a soft first syllable with room to
/// spare, and extra lead-in costs a recognizer nothing but a little silence.
const PRE_ROLL_MS: u64 = 300;
const PRE_ROLL: u64 = PRE_ROLL_MS * SAMPLE_RATE as u64 / 1000;

/// One complete utterance as the detector cut it.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Offset of the first sample, counted from the start of capture.
    pub start_sample: u64,
    /// 16 kHz mono f32, the utterance only.
    pub samples: Vec<f32>,
}

impl Segment {
    pub fn start_ms(&self) -> u64 {
        self.start_sample * 1000 / SAMPLE_RATE as u64
    }

    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / SAMPLE_RATE as u64
    }

    pub fn end_ms(&self) -> u64 {
        self.start_ms() + self.duration_ms()
    }
}

/// Tunables from `[vad]` in `convers.toml`, plus where the model lives.
#[derive(Debug, Clone)]
pub struct VadSettings {
    pub model: PathBuf,
    pub threshold: f32,
    pub min_silence_ms: u32,
    pub min_speech_ms: u32,
}

/// Feeds audio to Silero and yields the utterances it cuts.
pub struct Segmenter {
    vad: VoiceActivityDetector,
    /// Partial window carried between `push` calls.
    pending: Vec<f32>,
    /// Recent audio exactly as the detector received it, so the lead-in of a
    /// segment can be recovered after the detector reports it. It reaches back
    /// past the longest segment plus the silence that ends it, because that is
    /// how late a segment's start is learned.
    history: VecDeque<f32>,
    history_cap: usize,
    /// Samples the detector has accepted since it was created or last reset.
    /// Its segment offsets count in the same units.
    fed: u64,
    /// Where the previous segment ended. Pre-roll never reaches back past it,
    /// so two segments cut close together cannot share audio.
    last_end: u64,
}

impl Segmenter {
    /// Load the model and configure the detector. A missing model file is an
    /// error naming the absolute path; convers never downloads it (SPEC §2.1).
    pub fn new(settings: &VadSettings) -> Result<Self> {
        let model = absolute(&settings.model);
        if !model.is_file() {
            return Err(anyhow!(
                "VAD model not found: {}\nconvers never downloads models; place silero_vad.onnx \
                 at that exact path (see README.md) and run again.",
                model.display()
            ));
        }
        let model_str = model
            .to_str()
            .with_context(|| {
                format!(
                    "the path to the VAD model is not valid UTF-8: {}",
                    model.display()
                )
            })?
            .to_string();

        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model_str),
                threshold: settings.threshold,
                min_silence_duration: settings.min_silence_ms as f32 / 1000.0,
                min_speech_duration: settings.min_speech_ms as f32 / 1000.0,
                window_size: WINDOW as i32,
                max_speech_duration: MAX_SPEECH_SECONDS,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some("cpu".to_string()),
            ..Default::default()
        };

        let vad = VoiceActivityDetector::create(&config, BUFFER_SECONDS).ok_or_else(|| {
            anyhow!(
                "sherpa-onnx refused to create the voice activity detector from {}",
                model.display()
            )
        })?;

        let history_seconds = MAX_SPEECH_SECONDS as f64
            + settings.min_silence_ms as f64 / 1000.0
            + PRE_ROLL_MS as f64 / 1000.0
            + 1.0;
        let history_cap = (history_seconds * SAMPLE_RATE as f64) as usize;

        Ok(Self {
            vad,
            pending: Vec::with_capacity(WINDOW),
            history: VecDeque::with_capacity(history_cap),
            history_cap,
            fed: 0,
            last_end: 0,
        })
    }

    /// Feed 16 kHz mono audio. Returns whatever utterances completed as a
    /// result; usually none.
    pub fn push(&mut self, pcm: &[f32]) -> Vec<Segment> {
        for sample in pcm {
            self.pending.push(*sample);
            if self.pending.len() == WINDOW {
                self.accept_pending();
            }
        }
        self.drain()
    }

    /// End of input: push out any utterance still being accumulated. Used when
    /// capture stops, and in turn-based mode when the user ends a turn.
    pub fn flush(&mut self) -> Vec<Segment> {
        // Where captured audio ends, before any padding goes in.
        let real_end = self.fed + self.pending.len() as u64;
        if !self.pending.is_empty() {
            // Pad the last partial window; the detector only accepts full ones.
            self.pending.resize(WINDOW, 0.0);
            self.accept_pending();
        }
        self.vad.flush();

        // A segment holds only audio that was captured, never the padding.
        let mut segments = self.drain();
        for segment in &mut segments {
            let captured = real_end.saturating_sub(segment.start_sample) as usize;
            segment.samples.truncate(captured);
        }
        segments.retain(|s| !s.samples.is_empty());
        segments
    }

    /// Hand one full window to the detector, and remember it.
    fn accept_pending(&mut self) {
        self.vad.accept_waveform(&self.pending);
        self.history.extend(self.pending.iter().copied());
        self.fed += self.pending.len() as u64;
        let excess = self.history.len().saturating_sub(self.history_cap);
        self.history.drain(..excess);
        self.pending.clear();
    }

    /// True while the detector believes someone is speaking. The pipeline sends
    /// `SpeechStarted` on its rising edge (SPEC §11).
    pub fn speech_in_progress(&self) -> bool {
        self.vad.detected()
    }

    /// Forget all state and any queued segments.
    ///
    /// The half-duplex gate (SPEC §10) holds the detector reset while TTS is
    /// speaking, so no fragment of our own voice can survive into the next
    /// utterance.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.vad.clear();
        self.vad.reset();
        // The detector counts from zero again after a reset, so its history
        // and the offsets into it start over with it.
        self.history.clear();
        self.fed = 0;
        self.last_end = 0;
    }

    fn drain(&mut self) -> Vec<Segment> {
        let mut segments = Vec::new();
        while let Some(front) = self.vad.front() {
            let start = front.start().max(0) as u64;
            let body = front.samples();

            // How much lead-in to restore: the pre-roll, but never back past
            // the previous segment, and never further than history reaches.
            let oldest = self.fed - self.history.len() as u64;
            let lead = PRE_ROLL
                .min(start.saturating_sub(self.last_end))
                .min(start.saturating_sub(oldest));

            let from = (start - lead - oldest) as usize;
            let to = (start - oldest) as usize;
            let mut samples = Vec::with_capacity(lead as usize + body.len());
            samples.extend(self.history.range(from..to));
            samples.extend_from_slice(body);

            self.last_end = start + body.len() as u64;
            segments.push(Segment {
                start_sample: start - lead,
                samples,
            });
            drop(front);
            self.vad.pop();
        }
        segments
    }
}

/// Make a path absolute for error messages without requiring it to exist.
/// `canonicalize()` fails on a missing file, which is exactly the case where
/// the user most needs to be told the full path (SPEC §14).
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(model: &str) -> VadSettings {
        VadSettings {
            model: PathBuf::from(model),
            threshold: 0.5,
            min_silence_ms: 500,
            min_speech_ms: 250,
        }
    }

    #[test]
    fn a_missing_model_names_the_absolute_path() {
        let message = match Segmenter::new(&settings("models/vad/definitely-not-here.onnx")) {
            Ok(_) => panic!("a missing model must not produce a working segmenter"),
            Err(e) => e.to_string(),
        };
        assert!(message.contains("definitely-not-here.onnx"), "{message}");
        assert!(
            Path::new(
                message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim_start_matches("VAD model not found: ")
            )
            .is_absolute(),
            "{message}"
        );
        // It says convers will not fetch it; it never offers a URL to fetch.
        assert!(message.contains("never downloads"), "{message}");
        assert!(!message.contains("http"), "{message}");
    }

    #[test]
    fn segment_timings_are_derived_from_the_sample_rate() {
        let segment = Segment {
            start_sample: SAMPLE_RATE as u64,             // one second in
            samples: vec![0.0; SAMPLE_RATE as usize / 2], // half a second long
        };
        assert_eq!(segment.start_ms(), 1000);
        assert_eq!(segment.duration_ms(), 500);
        assert_eq!(segment.end_ms(), 1500);
    }

    /// Silence must produce nothing at all (Milestone 1's second check).
    /// Needs only the model:
    ///
    /// ```bash
    /// CONVERS_TEST_VAD_MODEL=/abs/silero_vad.onnx     /// cargo test -- --ignored --nocapture silence
    /// ```
    #[test]
    #[ignore = "needs the Silero model; see the doc comment"]
    fn silence_produces_no_segments() {
        let model = std::env::var("CONVERS_TEST_VAD_MODEL").expect("CONVERS_TEST_VAD_MODEL");
        let mut segmenter = Segmenter::new(&settings(&model)).expect("load the VAD");

        // Ten seconds of digital silence, then ten of quiet room noise.
        let mut noise: f32 = 0.0;
        let mut segments = Vec::new();
        for i in 0..(10 * SAMPLE_RATE as usize / 1024) {
            segments.extend(segmenter.push(&[0.0; 1024]));
            let _ = i;
        }
        for _ in 0..(10 * SAMPLE_RATE as usize / 1024) {
            // A deterministic low-level hiss at about -60 dBFS.
            let chunk: Vec<f32> = (0..1024)
                .map(|_| {
                    noise = (noise * 7.0 + 0.3).fract();
                    (noise - 0.5) * 0.002
                })
                .collect();
            segments.extend(segmenter.push(&chunk));
        }
        segments.extend(segmenter.flush());

        assert!(
            segments.is_empty(),
            "silence produced {} segment(s): {:?}",
            segments.len(),
            segments
                .iter()
                .map(|s| (s.start_ms(), s.duration_ms()))
                .collect::<Vec<_>>()
        );
    }

    /// The pre-roll must splice exactly: every segment, lead-in included, is
    /// the audio at the position it claims, and no two segments share audio.
    ///
    /// This replaces an earlier onset measurement that compared the first
    /// sample above a fixed loudness threshold with the detector's start. A
    /// soft onset like the "H" of "Hola" is quieter than any such threshold,
    /// so it passed on exactly the recordings that were never at risk and said
    /// nothing about the live ones that were.
    ///
    /// A long recording is the useful input here: many segments, some close
    /// together, and history that has to be trimmed as it runs.
    ///
    /// ```bash
    /// CONVERS_TEST_VAD_MODEL=/abs/silero_vad.onnx \
    /// CONVERS_TEST_WAV=/abs/speech.wav \
    /// cargo test --release -- --ignored --nocapture pre_roll
    /// ```
    #[test]
    #[ignore = "needs the Silero model and a recording; see the doc comment"]
    fn pre_roll_splices_exactly_and_never_overlaps() {
        let model = std::env::var("CONVERS_TEST_VAD_MODEL").expect("CONVERS_TEST_VAD_MODEL");
        let wav = std::env::var("CONVERS_TEST_WAV").expect("CONVERS_TEST_WAV");
        let audio = crate::wav::read_16k_mono(Path::new(&wav)).expect("read the test wav");

        let mut segmenter = Segmenter::new(&settings(&model)).expect("load the VAD");
        let mut segments = Vec::new();
        // Device-sized chunks that do not divide the detector's window, so
        // the carried-over partial window is exercised too.
        for chunk in audio.chunks(700) {
            segments.extend(segmenter.push(chunk));
        }
        segments.extend(segmenter.flush());
        assert!(!segments.is_empty(), "no speech found");

        let mut previous_end = 0u64;
        for (i, segment) in segments.iter().enumerate() {
            let start = segment.start_sample as usize;
            let end = start + segment.samples.len();
            assert!(
                end <= audio.len(),
                "segment {i} runs {} samples past the end of the recording",
                end - audio.len()
            );
            assert!(
                segment.samples[..] == audio[start..end],
                "segment {i} at {} ms is not the audio it claims to be",
                segment.start_ms()
            );
            assert!(
                segment.start_sample >= previous_end,
                "segment {i} starts at {} ms, inside the previous segment",
                segment.start_ms()
            );
            previous_end = end as u64;
        }

        println!(
            "{} segments, every one an exact slice of the recording, none overlapping",
            segments.len()
        );
        for segment in segments.iter().take(5) {
            println!(
                "  {:>6} ms .. {:>6} ms",
                segment.start_ms(),
                segment.end_ms()
            );
        }
    }

    /// End-to-end against the real model and a real recording. Both paths come
    /// from the environment because neither belongs in the repository:
    ///
    /// ```bash
    /// CONVERS_TEST_VAD_MODEL=/abs/silero_vad.onnx \
    /// CONVERS_TEST_WAV=/abs/speech.wav \
    /// cargo test -- --ignored --nocapture vad_cuts
    /// ```
    #[test]
    #[ignore = "needs the Silero model and a speech recording; see the doc comment"]
    fn vad_cuts_speech_out_of_a_recording() {
        let model = std::env::var("CONVERS_TEST_VAD_MODEL").expect("CONVERS_TEST_VAD_MODEL");
        let wav = std::env::var("CONVERS_TEST_WAV").expect("CONVERS_TEST_WAV");

        let audio = crate::wav::read_16k_mono(Path::new(&wav)).expect("read the test wav");
        let total_ms = audio.len() as u64 * 1000 / SAMPLE_RATE as u64;

        let mut segmenter = Segmenter::new(&settings(&model)).expect("load the VAD");
        let mut segments = Vec::new();
        // Feed it in device-sized chunks rather than all at once, the way the
        // capture thread will.
        for chunk in audio.chunks(1024) {
            segments.extend(segmenter.push(chunk));
        }
        segments.extend(segmenter.flush());

        for segment in &segments {
            println!(
                "segment {:>6} ms .. {:>6} ms  ({} ms)",
                segment.start_ms(),
                segment.end_ms(),
                segment.duration_ms()
            );
        }

        assert!(!segments.is_empty(), "no speech found in {wav}");
        let speech_ms: u64 = segments.iter().map(|s| s.duration_ms()).sum();
        assert!(
            speech_ms <= total_ms,
            "cut {speech_ms} ms of speech out of a {total_ms} ms recording"
        );
        assert!(
            segments.iter().all(|s| s.end_ms() <= total_ms + 1),
            "a segment ended past the end of the recording"
        );
    }
}
