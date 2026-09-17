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

        Ok(Self {
            vad,
            pending: Vec::with_capacity(WINDOW),
        })
    }

    /// Feed 16 kHz mono audio. Returns whatever utterances completed as a
    /// result; usually none.
    pub fn push(&mut self, pcm: &[f32]) -> Vec<Segment> {
        for sample in pcm {
            self.pending.push(*sample);
            if self.pending.len() == WINDOW {
                self.vad.accept_waveform(&self.pending);
                self.pending.clear();
            }
        }
        self.drain()
    }

    /// End of input: push out any utterance still being accumulated. Used when
    /// capture stops, and in turn-based mode when the user ends a turn.
    pub fn flush(&mut self) -> Vec<Segment> {
        if !self.pending.is_empty() {
            // Pad the last partial window; the detector only accepts full ones.
            self.pending.resize(WINDOW, 0.0);
            self.vad.accept_waveform(&self.pending);
            self.pending.clear();
        }
        self.vad.flush();
        self.drain()
    }

    fn drain(&mut self) -> Vec<Segment> {
        let mut segments = Vec::new();
        while let Some(front) = self.vad.front() {
            let start = front.start();
            segments.push(Segment {
                start_sample: start.max(0) as u64,
                samples: front.samples().to_vec(),
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
