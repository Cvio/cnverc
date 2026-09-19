//! Speech synthesis through sherpa-onnx's `OfflineTts`.
//!
//! Which voice speaks is decided by language, not by a setting: `cnverc.toml`
//! has no key naming a TTS model, and SPEC §9 says the language on an incoming
//! utterance is what tells the receiver which voice to use. So the voice is
//! the discovered model that declares the target language, and the filesystem
//! stays the index (SPEC §6).

use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, Context as _, Result};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig,
};
use tracing::info;

use crate::models::{Backend, Engine, TtsBackend};

/// Threads for synthesis. It shares a thread with translation and runs while
/// the recognizer may be working (SPEC §3, §11).
const NUM_THREADS: i32 = 2;

/// Synthesised speech, ready for the sound card.
pub struct Speech {
    pub samples: Vec<f32>,
    /// The model's own rate. Resampling to the device happens at playback.
    pub sample_rate: u32,
}

impl Speech {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.samples.len() as u64 * 1000 / self.sample_rate as u64
    }
}

pub struct Voice {
    tts: OfflineTts,
    sample_rate: u32,
    pub name: String,
}

impl Voice {
    /// Load the voice described by a discovered TTS directory.
    pub fn load(engine: &Engine) -> Result<Self> {
        let missing = engine.missing_files();
        if !missing.is_empty() {
            return Err(anyhow!(
                "voice \"{}\" is incomplete; these are not in {}: {}",
                engine.dir_name,
                engine.dir.display(),
                missing.join(", ")
            ));
        }

        let Backend::Tts(backend) = engine.backend else {
            return Err(anyhow!(
                "\"{}\" is a recognizer, not a voice",
                engine.dir_name
            ));
        };

        let began = Instant::now();
        let config = match backend {
            TtsBackend::Vits => OfflineTtsConfig {
                model: OfflineTtsModelConfig {
                    vits: OfflineTtsVitsModelConfig {
                        model: Some(file(engine, "model")?),
                        tokens: Some(file(engine, "tokens")?),
                        // Piper voices pronounce through espeak-ng and are
                        // silent nonsense without it.
                        data_dir: engine
                            .data_dir
                            .as_ref()
                            .map(|d| path_string(&d.path))
                            .transpose()?,
                        lexicon: optional_file(engine, "lexicon")?,
                        dict_dir: None,
                        ..Default::default()
                    },
                    num_threads: NUM_THREADS,
                    provider: Some("cpu".to_string()),
                    ..Default::default()
                },
                // One sentence at a time keeps latency down: the pipeline
                // speaks an utterance, not a paragraph.
                max_num_sentences: 1,
                ..Default::default()
            },
        };

        let tts = OfflineTts::create(&config).ok_or_else(|| {
            anyhow!(
                "sherpa-onnx refused to load the voice in {}",
                engine.dir.display()
            )
        })?;
        let sample_rate = u32::try_from(tts.sample_rate()).unwrap_or(0);
        if sample_rate == 0 {
            return Err(anyhow!(
                "the voice in {} reports a sample rate of {}",
                engine.dir.display(),
                tts.sample_rate()
            ));
        }

        info!(
            "loaded voice \"{}\" ({}) in {} ms: {} Hz, {} speaker(s)",
            engine.dir_name,
            engine.name,
            began.elapsed().as_millis(),
            sample_rate,
            tts.num_speakers()
        );

        Ok(Self {
            tts,
            sample_rate,
            name: engine.dir_name.clone(),
        })
    }

    /// Synthesise one utterance.
    pub fn speak(&self, text: &str) -> Result<Speech> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Speech {
                samples: Vec::new(),
                sample_rate: self.sample_rate,
            });
        }

        let config = GenerationConfig::default();
        let audio = self
            .tts
            // No progress callback: the generic parameter still needs a type.
            .generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| anyhow!("\"{}\" produced no audio for {text:?}", self.name))?;

        Ok(Speech {
            samples: audio.samples().to_vec(),
            sample_rate: u32::try_from(audio.sample_rate()).unwrap_or(self.sample_rate),
        })
    }
}

/// Pick the voice for a language: the discovered model that declares it.
///
/// Never substitutes a voice in another language (SPEC §15) — being told that
/// no English voice is installed is more useful than hearing Spanish.
pub fn for_language<'a>(engines: &'a [Engine], language: &str) -> Result<&'a Engine> {
    let usable: Vec<&Engine> = engines.iter().filter(|e| e.enabled()).collect();

    let matching: Vec<&&Engine> = usable
        .iter()
        .filter(|e| e.languages.iter().any(|l| l.eq_ignore_ascii_case(language)))
        .collect();

    match matching.first() {
        Some(engine) => {
            if matching.len() > 1 {
                info!(
                    "{} voices speak \"{language}\"; using \"{}\"",
                    matching.len(),
                    engine.dir_name
                );
            }
            Ok(engine)
        }
        None => Err(anyhow!(
            "no installed voice speaks \"{language}\". Voices found: {}",
            if usable.is_empty() {
                "none".to_string()
            } else {
                usable
                    .iter()
                    .map(|e| format!("{} ({})", e.dir_name, e.languages.join("/")))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
    }
}

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

fn optional_file(engine: &Engine, role: &str) -> Result<Option<String>> {
    match engine.files.iter().find(|f| f.role == role) {
        Some(entry) => Ok(Some(path_string(&entry.path)?)),
        None => Ok(None),
    }
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(|s| s.to_string())
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{self, Entry, Role};

    fn engines_from(models_root: &str) -> Vec<Engine> {
        models::discover(&Path::new(models_root).join("tts"), Role::Tts)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Loaded(engine) => Some(engine),
                Entry::Failed { .. } => None,
            })
            .collect()
    }

    /// Synthesis against the real voice:
    ///
    /// ```bash
    /// CNVERC_TEST_MODELS=/abs/path/models \
    /// cargo test --release -- --ignored --nocapture speaks
    /// ```
    #[test]
    #[ignore = "needs an installed voice; see the doc comment"]
    fn speaks_the_target_language() {
        let models_root = std::env::var("CNVERC_TEST_MODELS").expect("CNVERC_TEST_MODELS");
        let engines = engines_from(&models_root);
        let engine = for_language(&engines, "en").expect("an English voice");
        let voice = Voice::load(engine).expect("load the voice");

        let began = Instant::now();
        let speech = voice.speak("Where is the station?").expect("synthesise");
        println!(
            "{} Hz, {} samples, {} ms of speech, generated in {} ms",
            speech.sample_rate,
            speech.samples.len(),
            speech.duration_ms(),
            began.elapsed().as_millis()
        );

        assert!(!speech.samples.is_empty(), "no audio");
        assert!(
            speech.duration_ms() > 400,
            "only {} ms of audio for a four-word question",
            speech.duration_ms()
        );
        let peak = speech.samples.iter().fold(0.0_f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.05, "the audio is silent (peak {peak})");

        // Worth hearing rather than only asserting about.
        let out = std::env::temp_dir().join("cnverc-tts-check.wav");
        crate::wav::write_any(&out, &speech.samples, speech.sample_rate).expect("write");
        println!("wrote {}", out.display());
    }

    #[test]
    #[ignore = "needs installed voices; see the doc comment"]
    fn an_uninstalled_language_is_refused_not_substituted() {
        let models_root = std::env::var("CNVERC_TEST_MODELS").expect("CNVERC_TEST_MODELS");
        let engines = engines_from(&models_root);
        let message = match for_language(&engines, "ja") {
            Ok(engine) => panic!("substituted {} for Japanese", engine.dir_name),
            Err(e) => e.to_string(),
        };
        assert!(message.contains("no installed voice"), "{message}");
    }
}
