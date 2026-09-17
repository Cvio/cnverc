//! The dual-run comparison harness (SPEC §12).
//!
//! Runs one captured utterance through every enabled segment-kind engine and
//! reports, side by side: the transcript, the wall clock milliseconds, and the
//! duration of the segment itself.
//!
//! The segment duration is not decoration. Whisper pads every input to a
//! 30-second window internally, so a 1.2-second utterance costs roughly what a
//! 20-second one does (SPEC §10). Without the duration beside the timing, the
//! comparison reads as nonsense.

use std::time::Instant;

use crate::asr::SegmentAsr;
use crate::ring::Utterance;

/// What one engine made of one utterance.
#[derive(Debug, Clone)]
pub struct Run {
    pub engine: String,
    pub text: String,
    pub elapsed_ms: u128,
    /// `None` when the engine failed; the message is in `text`.
    pub ok: bool,
}

/// Every engine's attempt at one utterance.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub utterance_index: usize,
    /// When the utterance started, in milliseconds since capture began. The
    /// ring is keyed by it, so the comparison quotes it.
    pub start_ms: u64,
    pub segment_ms: u64,
    pub runs: Vec<Run>,
}

/// Run `utterance` through each engine in turn. Engines run sequentially on
/// purpose: two large models decoding at once on an 8GB laptop would measure
/// contention rather than the models.
pub fn run_all(
    engines: &mut [(String, Box<dyn SegmentAsr + Send>)],
    utterance: &Utterance,
    language: &str,
) -> Comparison {
    let mut runs = Vec::with_capacity(engines.len());
    for (name, asr) in engines.iter_mut() {
        let started = Instant::now();
        let result = asr.transcribe(&utterance.pcm, language);
        let elapsed_ms = started.elapsed().as_millis();
        runs.push(match result {
            Ok(text) => Run {
                engine: name.clone(),
                text,
                elapsed_ms,
                ok: true,
            },
            Err(e) => Run {
                engine: name.clone(),
                text: format!("{e:#}"),
                elapsed_ms,
                ok: false,
            },
        });
    }
    Comparison {
        utterance_index: utterance.index,
        start_ms: utterance.start_ms,
        segment_ms: utterance.duration_ms(),
        runs,
    }
}

/// Print a comparison to stdout. This is program output, not a log line: it is
/// the thing the harness exists to produce.
pub fn print(comparison: &Comparison, selected_engine: &str) {
    println!();
    println!(
        "utterance {} at {} ms — {} ms of audio",
        comparison.utterance_index, comparison.start_ms, comparison.segment_ms
    );

    let width = comparison
        .runs
        .iter()
        .map(|r| r.engine.len())
        .max()
        .unwrap_or(0);

    for run in &comparison.runs {
        let realtime = if comparison.segment_ms > 0 {
            format!(
                " ({:.2}x realtime)",
                run.elapsed_ms as f64 / comparison.segment_ms as f64
            )
        } else {
            String::new()
        };
        // The configured engine is marked, so the comparison also answers
        // "which of these am I actually using?".
        println!(
            "  {} {:<width$}  {:>6} ms{realtime}",
            if run.engine == selected_engine {
                "*"
            } else {
                " "
            },
            run.engine,
            run.elapsed_ms
        );
        println!("      {}{}", if run.ok { "" } else { "FAILED: " }, run.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SAMPLE_RATE;
    use crate::ring::UtteranceRing;
    use anyhow::anyhow;

    struct Fake {
        reply: &'static str,
        fail: bool,
    }

    impl SegmentAsr for Fake {
        fn transcribe(&mut self, _pcm: &[f32], _language: &str) -> anyhow::Result<String> {
            if self.fail {
                Err(anyhow!("no model loaded"))
            } else {
                Ok(self.reply.to_string())
            }
        }
    }

    fn utterance() -> Utterance {
        let mut ring = UtteranceRing::new(1);
        ring.push(0, vec![0.0; SAMPLE_RATE as usize])
    }

    #[test]
    fn every_engine_gets_the_same_audio() {
        let mut engines: Vec<(String, Box<dyn SegmentAsr + Send>)> = vec![
            (
                "parakeet".to_string(),
                Box::new(Fake {
                    reply: "hola",
                    fail: false,
                }),
            ),
            (
                "whisper".to_string(),
                Box::new(Fake {
                    reply: "Hola.",
                    fail: false,
                }),
            ),
        ];
        let comparison = run_all(&mut engines, &utterance(), "es");
        assert_eq!(comparison.segment_ms, 1000);
        assert_eq!(comparison.runs.len(), 2);
        assert_eq!(comparison.runs[0].text, "hola");
        assert_eq!(comparison.runs[1].text, "Hola.");
        assert!(comparison.runs.iter().all(|r| r.ok));
    }

    #[test]
    fn one_failing_engine_does_not_lose_the_others() {
        let mut engines: Vec<(String, Box<dyn SegmentAsr + Send>)> = vec![
            (
                "broken".to_string(),
                Box::new(Fake {
                    reply: "",
                    fail: true,
                }),
            ),
            (
                "working".to_string(),
                Box::new(Fake {
                    reply: "hola",
                    fail: false,
                }),
            ),
        ];
        let comparison = run_all(&mut engines, &utterance(), "es");
        assert!(!comparison.runs[0].ok);
        assert!(comparison.runs[0].text.contains("no model loaded"));
        assert!(comparison.runs[1].ok);
    }
}
