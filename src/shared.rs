//! Shared-machine mode (M7.5): two people who speak different languages use
//! one machine, each with their own key.
//!
//! The key says who is talking, and so which language they speak. Each side
//! has its own recognizer, chosen for that side's language, so a model tuned
//! for one language can hear one person while another hears the other.
//!
//! This module is the rule that turns "which side pressed" into a direction:
//! the recognizer and language to hear it with, the language to translate
//! into, and the voice to speak it with. It has no window, device or model in it, so every case is
//! tested directly.

use std::fmt;

use crate::config::Shared;
use crate::models::{AsrBackend, Backend, Engine};

/// Which person pressed their key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// This side's language in the settings.
    pub fn language(self, settings: &Shared) -> &str {
        match self {
            Side::Left => &settings.left_language,
            Side::Right => &settings.right_language,
        }
    }

    /// The voice that speaks this side's words, which is a voice in the other
    /// side's language.
    pub fn voice(self, settings: &Shared) -> &str {
        match self {
            Side::Left => &settings.left_voice,
            Side::Right => &settings.right_voice,
        }
    }

    /// The recognizer configured for this side: a folder name, or empty for
    /// the best match.
    pub fn asr(self, settings: &Shared) -> &str {
        match self {
            Side::Left => &settings.left_asr,
            Side::Right => &settings.right_asr,
        }
    }

    pub fn key(self, settings: &Shared) -> &str {
        match self {
            Side::Left => &settings.left_key,
            Side::Right => &settings.right_key,
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Left => "left",
            Side::Right => "right",
        })
    }
}

/// Everything one turn needs to know about where its words go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direction {
    pub side: Side,
    /// The recognizer's folder name under `models/asr/`.
    pub asr: String,
    /// What the speaker is saying, passed to the recognizer as its language.
    pub source: String,
    /// What it is translated into and spoken in.
    pub target: String,
    /// The voice's folder name under `models/tts/`.
    pub voice: String,
}

/// A direction that can be used, possibly with something the side's column
/// should show, such as a configured voice that has gone missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub direction: Direction,
    /// Things the side's column should show, such as a voice that fell back.
    pub warnings: Vec<String>,
}

/// The recognizer that hears one side, or why there is none.
///
/// The configured one when set; it must be installed, complete, and list the
/// side's language, and it is never swapped for another (SPEC §15). When
/// unset, the best match: `preferred` (the main recognizer) if it covers the
/// language, otherwise the first usable recognizer that does.
pub fn recognizer_for<'a>(
    side: Side,
    settings: &Shared,
    recognizers: &'a [Engine],
    preferred: &str,
) -> Result<&'a Engine, String> {
    let language = side.language(settings).trim();
    let configured = side.asr(settings).trim();
    if !configured.is_empty() {
        let engine = recognizers
            .iter()
            .find(|e| e.dir_name == configured)
            .ok_or_else(|| format!("the recognizer \"{configured}\" is not installed"))?;
        if !engine.enabled() {
            return Err(format!(
                "{} is missing files: {}",
                engine.name,
                engine.missing_files().join(", ")
            ));
        }
        if !declares(engine, language) {
            return Err(format!(
                "{} does not list \"{language}\" among its languages, so it cannot hear this side",
                engine.name
            ));
        }
        return Ok(engine);
    }
    let covers = |e: &&Engine| e.enabled() && declares(e, language);
    recognizers
        .iter()
        .filter(covers)
        .find(|e| e.dir_name == preferred)
        .or_else(|| recognizers.iter().find(covers))
        .ok_or_else(|| format!("no installed recognizer lists \"{language}\" among its languages"))
}

/// The usable recognizers that list `language`, for a side's picker.
pub fn recognizers_for<'a>(language: &str, recognizers: &'a [Engine]) -> Vec<&'a Engine> {
    recognizers
        .iter()
        .filter(|e| e.enabled() && declares(e, language))
        .collect()
}

/// Work out where one turn's words go, or why there can be no turn.
///
/// `recognizers` and `voices` are every discovered model, broken ones
/// included; only usable ones are chosen. `preferred` is the main
/// recognizer, the first choice for a side that has none configured.
pub fn direction(
    side: Side,
    settings: &Shared,
    recognizers: &[Engine],
    preferred: &str,
    voices: &[Engine],
) -> Result<Resolved, String> {
    let source = side.language(settings).trim().to_string();
    let target = side.other().language(settings).trim().to_string();
    if source.is_empty() || target.is_empty() {
        return Err("choose a language for both sides".to_string());
    }
    if source == target {
        return Err(format!(
            "both sides are set to \"{source}\"; choose a different language for each"
        ));
    }

    let recognizer = recognizer_for(side, settings, recognizers, preferred)?;
    let mut warnings = Vec::new();
    if recognizer.backend != Backend::Asr(AsrBackend::Whisper) {
        warnings.push(format!(
            "{} decides the language itself rather than being told it, so it may mishear a \
             short sentence",
            recognizer.name
        ));
    }

    let configured = side.voice(settings).trim();
    let chosen = voices
        .iter()
        .find(|v| v.dir_name == configured && v.enabled() && declares(v, &target));
    let voice = match chosen {
        Some(voice) => voice,
        None => {
            let fallback = voices_for(&target, voices)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    format!(
                    "no installed voice speaks \"{target}\", so this side's words could not be \
                     spoken"
                )
                })?;
            if !configured.is_empty() {
                warnings.push(format!(
                    "the voice \"{configured}\" is not installed, or does not speak \
                     \"{target}\"; using \"{}\" instead",
                    fallback.dir_name
                ));
            }
            fallback
        }
    };

    Ok(Resolved {
        direction: Direction {
            side,
            asr: recognizer.dir_name.clone(),
            source,
            target,
            voice: voice.dir_name.clone(),
        },
        warnings,
    })
}

/// The usable voices that speak `language`, in discovery order, for a side's
/// voice picker.
pub fn voices_for<'a>(language: &str, voices: &'a [Engine]) -> Vec<&'a Engine> {
    voices
        .iter()
        .filter(|v| v.enabled() && declares(v, language))
        .collect()
}

fn declares(engine: &Engine, language: &str) -> bool {
    engine
        .languages
        .iter()
        .any(|l| l.eq_ignore_ascii_case(language))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EngineKind, ModelFile, TtsBackend};
    use std::path::PathBuf;

    fn engine(dir_name: &str, backend: Backend, languages: &[&str], present: bool) -> Engine {
        Engine {
            data_dir: None,
            dir_name: dir_name.to_string(),
            dir: PathBuf::from(dir_name),
            name: format!("{dir_name} (test)"),
            kind: EngineKind::Segment,
            backend,
            languages: languages.iter().map(|l| l.to_string()).collect(),
            varieties: Vec::new(),
            files: vec![ModelFile {
                role: "model".to_string(),
                name: "model.onnx".to_string(),
                path: PathBuf::from("model.onnx"),
                present,
            }],
        }
    }

    fn whisper() -> Engine {
        engine(
            "whisper",
            Backend::Asr(AsrBackend::Whisper),
            &["en", "es"],
            true,
        )
    }

    fn voice(dir_name: &str, language: &str) -> Engine {
        engine(dir_name, Backend::Tts(TtsBackend::Vits), &[language], true)
    }

    fn voices() -> Vec<Engine> {
        vec![
            voice("piper-en", "en"),
            voice("piper-es-es", "es"),
            voice("piper-es-mx", "es"),
        ]
    }

    /// English on the left, Spanish on the right, the defaults.
    fn settings() -> Shared {
        Shared::default()
    }

    #[test]
    fn the_left_key_translates_left_into_right() {
        let r =
            direction(Side::Left, &settings(), &[whisper()], "whisper", &voices()).expect("a turn");
        assert_eq!(
            r.direction,
            Direction {
                side: Side::Left,
                asr: "whisper".into(),
                source: "en".into(),
                target: "es".into(),
                voice: "piper-es-es".into(),
            }
        );
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn the_right_key_translates_right_into_left() {
        let r = direction(Side::Right, &settings(), &[whisper()], "whisper", &voices())
            .expect("a turn");
        assert_eq!(r.direction.source, "es");
        assert_eq!(r.direction.target, "en");
        assert_eq!(
            r.direction.voice, "piper-en",
            "a voice in the LEFT person's language"
        );
    }

    #[test]
    fn a_voice_is_chosen_by_folder_so_two_spanish_voices_can_be_told_apart() {
        let mut s = settings();
        s.left_voice = "piper-es-mx".into();
        let r = direction(Side::Left, &s, &[whisper()], "whisper", &voices()).expect("a turn");
        assert_eq!(r.direction.voice, "piper-es-mx");
    }

    #[test]
    fn a_missing_voice_falls_back_and_says_so() {
        let mut s = settings();
        s.left_voice = "deleted-voice".into();
        let r =
            direction(Side::Left, &s, &[whisper()], "whisper", &voices()).expect("still a turn");
        assert_eq!(r.direction.voice, "piper-es-es", "the first Spanish voice");
        let warning = r.warnings.join(" ");
        assert!(warning.contains("deleted-voice"), "{warning}");

        // A voice in the wrong language is no better than a missing one.
        s.left_voice = "piper-en".into();
        let r =
            direction(Side::Left, &s, &[whisper()], "whisper", &voices()).expect("still a turn");
        assert_eq!(r.direction.voice, "piper-es-es");
        assert!(!r.warnings.is_empty());
    }

    #[test]
    fn a_recognizer_that_guesses_the_language_is_allowed_with_a_note() {
        let parakeet = engine(
            "parakeet",
            Backend::Asr(AsrBackend::NemoTransducer),
            &["en", "es"],
            true,
        );
        let r = direction(Side::Left, &settings(), &[parakeet], "parakeet", &voices())
            .expect("Parakeet covers English");
        assert_eq!(r.direction.asr, "parakeet");
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("decides the language itself")));
    }

    #[test]
    fn each_side_gets_its_own_recognizer() {
        // English on the left with Parakeet, Spanish on the right with a
        // Spanish-only Whisper: the example from dialect-per-side.md.
        let parakeet = engine(
            "parakeet",
            Backend::Asr(AsrBackend::NemoTransducer),
            &["en"],
            true,
        );
        let spanish = engine(
            "whisper-es",
            Backend::Asr(AsrBackend::Whisper),
            &["es"],
            true,
        );
        let all = [parakeet, spanish];
        let mut s = settings();
        s.left_asr = "parakeet".into();
        s.right_asr = "whisper-es".into();
        let left = direction(Side::Left, &s, &all, "", &voices()).expect("left");
        let right = direction(Side::Right, &s, &all, "", &voices()).expect("right");
        assert_eq!(left.direction.asr, "parakeet");
        assert_eq!(right.direction.asr, "whisper-es");

        // Left unset: the best match for English, never the Spanish-only model.
        s.left_asr.clear();
        let best = direction(Side::Left, &s, &all, "whisper-es", &voices()).expect("left");
        assert_eq!(best.direction.asr, "parakeet");
    }

    #[test]
    fn a_configured_recognizer_is_never_swapped_for_another() {
        let mut s = settings();
        s.left_asr = "deleted-model".into();
        let why = direction(Side::Left, &s, &[whisper()], "whisper", &voices())
            .expect_err("a missing configured recognizer is refused");
        assert!(why.contains("deleted-model"), "{why}");
    }

    #[test]
    fn a_recognizer_missing_one_language_refuses_only_that_side() {
        let english_only = engine(
            "whisper-en",
            Backend::Asr(AsrBackend::Whisper),
            &["en"],
            true,
        );
        assert!(direction(
            Side::Left,
            &settings(),
            std::slice::from_ref(&english_only),
            "whisper-en",
            &voices()
        )
        .is_ok());
        let why = direction(
            Side::Right,
            &settings(),
            std::slice::from_ref(&english_only),
            "whisper-en",
            &voices(),
        )
        .expect_err("it cannot hear Spanish");
        assert!(why.contains("\"es\""), "{why}");
    }

    #[test]
    fn no_voice_for_the_target_language_is_refused() {
        let only_english = vec![voice("piper-en", "en")];
        let why = direction(
            Side::Left,
            &settings(),
            &[whisper()],
            "whisper",
            &only_english,
        )
        .expect_err("nothing can speak Spanish");
        assert!(why.contains("\"es\""), "{why}");
    }

    #[test]
    fn broken_models_and_bad_settings_are_refused_with_a_reason() {
        let broken = engine(
            "whisper",
            Backend::Asr(AsrBackend::Whisper),
            &["en", "es"],
            false,
        );
        assert!(direction(Side::Left, &settings(), &[broken], "whisper", &voices()).is_err());
        assert!(direction(Side::Left, &settings(), &[], "", &voices()).is_err());

        let mut same = settings();
        same.right_language = "en".into();
        assert!(direction(Side::Left, &same, &[whisper()], "whisper", &voices()).is_err());

        // A broken voice is never chosen, even when it is the configured one.
        let mut s = settings();
        s.left_voice = "piper-es-mx".into();
        let mut vs = voices();
        vs[2] = engine(
            "piper-es-mx",
            Backend::Tts(TtsBackend::Vits),
            &["es"],
            false,
        );
        let r = direction(Side::Left, &s, &[whisper()], "whisper", &vs).expect("falls back");
        assert_eq!(r.direction.voice, "piper-es-es");
        assert!(!r.warnings.is_empty());
    }

    #[test]
    fn the_picker_lists_only_usable_voices_for_the_language() {
        let names: Vec<_> = voices_for("es", &voices())
            .iter()
            .map(|v| v.dir_name.clone())
            .collect();
        assert_eq!(names, vec!["piper-es-es", "piper-es-mx"]);
    }
}
