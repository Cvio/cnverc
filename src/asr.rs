//! Speech recognition, in two deliberately different shapes (SPEC §11).
//!
//! Whisper and Parakeet TDT are whole-utterance recognizers: a complete
//! segment goes in, text comes out. Streaming recognizers are fed continuously
//! and polled for a revisable partial hypothesis. Collapsing both into one
//! `transcribe()` would wall us off from the low-latency path in Milestone 8,
//! so both traits and the enum exist from here on even though only
//! [`SegmentAsr`] has implementations yet.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
    OfflineWhisperModelConfig,
};
use tracing::info;

use crate::audio::SAMPLE_RATE;
use crate::models::{AsrBackend, Backend, Engine, EngineKind};

/// Threads each recognizer may use. One engine is resident per selection, but
/// `--compare` loads several at once and they share a laptop with a desktop
/// session (SPEC §3).
const NUM_THREADS: i32 = 2;

/// A loaded recognizer, in whichever of the two shapes it has.
///
/// `Stream` has no constructor before Milestone 8. It is here from Milestone 2
/// so downstream stages are written against the right shape from the start.
#[allow(dead_code, reason = "Stream is constructed from Milestone 8; SPEC §11")]
pub enum AsrEngine {
    Segment(Box<dyn SegmentAsr + Send>),
    Stream(Box<dyn StreamAsr + Send>),
}

pub trait SegmentAsr {
    /// `pcm` is a complete utterance, 16 kHz mono f32 in [-1.0, 1.0].
    fn transcribe(&mut self, pcm: &[f32], language: &str) -> anyhow::Result<String>;
}

#[allow(dead_code, reason = "implemented in Milestone 8; SPEC §11")]
pub trait StreamAsr {
    fn feed(&mut self, pcm: &[f32]) -> anyhow::Result<()>;
    /// Current best guess. May be revised on subsequent calls.
    fn partial(&self) -> &str;
    /// True when the recognizer considers the utterance complete.
    fn endpoint(&mut self) -> anyhow::Result<bool>;
    fn reset(&mut self) -> anyhow::Result<()>;
}

/// Load the engine described by a discovered model directory.
///
/// Refuses anything the discovery pass marked disabled: a half-extracted model
/// must fail here, naming the file, rather than inside sherpa-onnx.
pub fn load(engine: &Engine) -> Result<AsrEngine> {
    let missing = engine.missing_files();
    if !missing.is_empty() {
        return Err(anyhow!(
            "model \"{}\" is incomplete; these files are not in {}: {}",
            engine.dir_name,
            engine.dir.display(),
            missing.join(", ")
        ));
    }

    match engine.backend {
        Backend::Asr(AsrBackend::NemoTransducer) => {
            let asr = NemoTransducerAsr::load(engine)?;
            Ok(AsrEngine::Segment(Box::new(asr)))
        }
        Backend::Asr(AsrBackend::Whisper) => {
            let asr = WhisperAsr::load(engine)?;
            Ok(AsrEngine::Segment(Box::new(asr)))
        }
        Backend::Tts(_) => Err(anyhow!(
            "\"{}\" is a TTS model, not a recognizer",
            engine.dir_name
        )),
    }
}

/// Load every discovered engine of `kind` = segment that is usable, for the
/// dual-run harness (SPEC §12). An engine that fails to load is reported and
/// skipped: one broken model must not deny the comparison to the others.
pub fn load_all_segment_engines(engines: &[Engine]) -> Vec<(String, Box<dyn SegmentAsr + Send>)> {
    let mut loaded = Vec::new();
    for engine in engines {
        if engine.kind != EngineKind::Segment || !engine.enabled() {
            continue;
        }
        let started = Instant::now();
        match load(engine) {
            Ok(AsrEngine::Segment(asr)) => {
                info!(
                    "loaded \"{}\" in {} ms",
                    engine.dir_name,
                    started.elapsed().as_millis()
                );
                loaded.push((engine.dir_name.clone(), asr));
            }
            Ok(AsrEngine::Stream(_)) => {}
            Err(e) => tracing::warn!("cannot load \"{}\": {e:#}", engine.dir_name),
        }
    }
    loaded
}

/// The absolute path of one declared file, as a string sherpa-onnx accepts.
fn file(engine: &Engine, role: &str) -> Result<String> {
    let entry = engine
        .files
        .iter()
        .find(|f| f.role == role)
        .ok_or_else(|| {
            anyhow!(
                "{} does not declare [files].{role}",
                engine.dir.join(crate::models::ENGINE_TOML).display()
            )
        })?;
    path_string(&entry.path)
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(|s| s.to_string())
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// Run one utterance through a configured recognizer.
fn decode(recognizer: &OfflineRecognizer, pcm: &[f32]) -> Result<String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, pcm);
    recognizer.decode(&stream);
    let result = stream
        .get_result()
        .ok_or_else(|| anyhow!("the recognizer returned no result"))?;
    Ok(result.text.trim().to_string())
}

/// Parakeet TDT and other NeMo transducers.
///
/// Multilingual by construction: the model detects the language itself, so the
/// `language` argument is not used. It is part of the trait because Whisper
/// needs it.
struct NemoTransducerAsr {
    recognizer: OfflineRecognizer,
    name: String,
}

impl NemoTransducerAsr {
    fn load(engine: &Engine) -> Result<Self> {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(file(engine, "encoder")?),
            decoder: Some(file(engine, "decoder")?),
            joiner: Some(file(engine, "joiner")?),
        };
        config.model_config.tokens = Some(file(engine, "tokens")?);
        config.model_config.model_type = Some("nemo_transducer".to_string());
        config.model_config.num_threads = NUM_THREADS;
        config.model_config.provider = Some("cpu".to_string());

        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
            anyhow!(
                "sherpa-onnx refused to load the transducer model in {}",
                engine.dir.display()
            )
        })?;
        Ok(Self {
            recognizer,
            name: engine.dir_name.clone(),
        })
    }
}

impl SegmentAsr for NemoTransducerAsr {
    fn transcribe(&mut self, pcm: &[f32], _language: &str) -> Result<String> {
        decode(&self.recognizer, pcm)
            .with_context(|| format!("\"{}\" failed to transcribe", self.name))
    }
}

/// Whisper.
///
/// The language is baked into the recognizer at creation, but the trait passes
/// it per utterance, so a change of language rebuilds the recognizer. That
/// costs seconds and happens when the user changes a selector, not per
/// utterance.
struct WhisperAsr {
    recognizer: OfflineRecognizer,
    language: String,
    encoder: String,
    decoder: String,
    tokens: String,
    name: String,
    dir: PathBuf,
}

impl WhisperAsr {
    fn load(engine: &Engine) -> Result<Self> {
        let encoder = file(engine, "encoder")?;
        let decoder = file(engine, "decoder")?;
        let tokens = file(engine, "tokens")?;
        // Whichever language the first utterance asks for replaces this.
        let language = engine
            .languages
            .first()
            .cloned()
            .unwrap_or_else(|| "en".to_string());

        let recognizer = Self::build(&encoder, &decoder, &tokens, &language, &engine.dir)?;
        info!(
            "\"{}\" is Whisper: it pads every utterance to 30 s internally, so a 1 s utterance \
             costs about what a 20 s one does",
            engine.dir_name
        );
        Ok(Self {
            recognizer,
            language,
            encoder,
            decoder,
            tokens,
            name: engine.dir_name.clone(),
            dir: engine.dir.clone(),
        })
    }

    fn build(
        encoder: &str,
        decoder: &str,
        tokens: &str,
        language: &str,
        dir: &Path,
    ) -> Result<OfflineRecognizer> {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.whisper = OfflineWhisperModelConfig {
            encoder: Some(encoder.to_string()),
            decoder: Some(decoder.to_string()),
            language: Some(language.to_string()),
            task: Some("transcribe".to_string()),
            tail_paddings: 0,
            enable_token_timestamps: false,
            enable_segment_timestamps: false,
        };
        config.model_config.tokens = Some(tokens.to_string());
        config.model_config.num_threads = NUM_THREADS;
        config.model_config.provider = Some("cpu".to_string());

        OfflineRecognizer::create(&config).ok_or_else(|| {
            anyhow!(
                "sherpa-onnx refused to load the Whisper model in {} for language \"{language}\"",
                dir.display()
            )
        })
    }
}

impl SegmentAsr for WhisperAsr {
    fn transcribe(&mut self, pcm: &[f32], language: &str) -> Result<String> {
        if !language.is_empty() && language != self.language {
            info!(
                "\"{}\": rebuilding the recognizer for language \"{language}\"",
                self.name
            );
            self.recognizer = Self::build(
                &self.encoder,
                &self.decoder,
                &self.tokens,
                language,
                &self.dir,
            )?;
            self.language = language.to_string();
        }
        decode(&self.recognizer, pcm)
            .with_context(|| format!("\"{}\" failed to transcribe", self.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{self, Role};

    /// Both engines against the same Spanish utterance, which is Milestone 2's
    /// check. Needs a models tree and a 16 kHz mono recording:
    ///
    /// ```bash
    /// CONVERS_TEST_MODELS=/abs/path/models \
    /// CONVERS_TEST_WAV_ES=/abs/path/spanish.wav \
    /// cargo test -- --ignored --nocapture both_engines
    /// ```
    #[test]
    #[ignore = "needs the ASR models and a recording; see the doc comment"]
    fn both_engines_transcribe_the_same_utterance() {
        let models_root = std::env::var("CONVERS_TEST_MODELS").expect("CONVERS_TEST_MODELS");
        let wav = std::env::var("CONVERS_TEST_WAV_ES").expect("CONVERS_TEST_WAV_ES");
        let pcm = crate::wav::read_16k_mono(Path::new(&wav)).expect("read the recording");

        let entries = models::discover(&Path::new(&models_root).join("asr"), Role::Asr);
        let engines: Vec<Engine> = entries
            .into_iter()
            .filter_map(|e| match e {
                models::Entry::Loaded(engine) => Some(engine),
                models::Entry::Failed { .. } => None,
            })
            .collect();

        let mut loaded = load_all_segment_engines(&engines);
        assert!(
            loaded.len() >= 2,
            "expected two usable segment engines, found {}",
            loaded.len()
        );

        // Through the harness itself, so this exercises what `--compare`
        // prints rather than a parallel code path.
        let mut ring = crate::ring::UtteranceRing::new(1);
        let utterance = ring.push(0, pcm);
        let comparison = crate::compare::run_all(&mut loaded, &utterance, "es");
        crate::compare::print(&comparison, "parakeet-tdt-0.6b-v3-int8");

        assert_eq!(comparison.runs.len(), loaded.len());
        for run in &comparison.runs {
            assert!(run.ok, "{} failed: {}", run.engine, run.text);
            assert!(!run.text.is_empty(), "{} produced no text", run.engine);
            // Spanish in, Spanish out: no engine may quietly translate.
            assert!(
                run.text.to_lowercase().contains("país"),
                "{} did not transcribe Spanish: {}",
                run.engine,
                run.text
            );
        }
    }
}
