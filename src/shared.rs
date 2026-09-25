//! Shared-machine mode (M7.5): two people who speak different languages use
//! one machine, each with their own key.
//!
//! The key says who is talking, and so which language they speak. Nothing is
//! detected: the language from the key goes to the recognizer, which must be
//! Whisper, because Whisper is the recognizer that takes a language rather
//! than guessing one.
//!
//! This module is the rule that turns "which side pressed" into a direction:
//! the language to recognise, the language to translate into, and the voice to
//! speak it with. It has no window, device or model in it, so every case is
//! tested directly.

#![allow(
    dead_code,
    reason = "used by the window from M7.5 step 3 and the pipeline from step 4"
)]

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
    /// What the speaker is saying, passed to Whisper as its language.
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
    pub warning: Option<String>,
}

/// Work out where one turn's words go, or why there can be no turn.
///
/// `recognizer` is the selected recognizer, shared by both sides. `voices` is
/// every discovered voice, broken ones included; only usable ones are chosen.
pub fn direction(
    side: Side,
    settings: &Shared,
    recognizer: Option<&Engine>,
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

    let recognizer = recognizer.ok_or_else(|| "no recognizer is selected".to_string())?;
    if recognizer.backend != Backend::Asr(AsrBackend::Whisper) {
        return Err(format!(
            "{} decides the language itself and cannot be told which one is being spoken. \
             Shared machine mode needs a Whisper recognizer.",
            recognizer.name
        ));
    }
    if !recognizer.enabled() {
        return Err(format!(
            "{} is missing files: {}",
            recognizer.name,
            recognizer.missing_files().join(", ")
        ));
    }
    if !declares(recognizer, &source) {
        return Err(format!(
            "{} does not list \"{source}\" among its languages, so it cannot hear this side",
            recognizer.name
        ));
    }

    let configured = side.voice(settings).trim();
    let chosen = voices
        .iter()
        .find(|v| v.dir_name == configured && v.enabled() && declares(v, &target));
    let (voice, warning) = match chosen {
        Some(voice) => (voice, None),
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
            let warning = (!configured.is_empty()).then(|| {
                format!(
                    "the voice \"{configured}\" is not installed, or does not speak \
                     \"{target}\"; using \"{}\" instead",
                    fallback.dir_name
                )
            });
            (fallback, warning)
        }
    };

    Ok(Resolved {
        direction: Direction {
            side,
            source,
            target,
            voice: voice.dir_name.clone(),
        },
        warning,
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
        let r = direction(Side::Left, &settings(), Some(&whisper()), &voices()).expect("a turn");
        assert_eq!(
            r.direction,
            Direction {
                side: Side::Left,
                source: "en".into(),
                target: "es".into(),
                voice: "piper-es-es".into(),
            }
        );
        assert!(r.warning.is_none());
    }

    #[test]
    fn the_right_key_translates_right_into_left() {
        let r = direction(Side::Right, &settings(), Some(&whisper()), &voices()).expect("a turn");
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
        let r = direction(Side::Left, &s, Some(&whisper()), &voices()).expect("a turn");
        assert_eq!(r.direction.voice, "piper-es-mx");
    }

    #[test]
    fn a_missing_voice_falls_back_and_says_so() {
        let mut s = settings();
        s.left_voice = "deleted-voice".into();
        let r = direction(Side::Left, &s, Some(&whisper()), &voices()).expect("still a turn");
        assert_eq!(r.direction.voice, "piper-es-es", "the first Spanish voice");
        let warning = r.warning.expect("the column must say what happened");
        assert!(warning.contains("deleted-voice"), "{warning}");

        // A voice in the wrong language is no better than a missing one.
        s.left_voice = "piper-en".into();
        let r = direction(Side::Left, &s, Some(&whisper()), &voices()).expect("still a turn");
        assert_eq!(r.direction.voice, "piper-es-es");
        assert!(r.warning.is_some());
    }

    #[test]
    fn a_recognizer_that_guesses_the_language_is_refused() {
        let parakeet = engine(
            "parakeet",
            Backend::Asr(AsrBackend::NemoTransducer),
            &["en", "es"],
            true,
        );
        let why = direction(Side::Left, &settings(), Some(&parakeet), &voices())
            .expect_err("Parakeet can't be told the language");
        assert!(why.contains("Whisper"), "{why}");
    }

    #[test]
    fn a_recognizer_missing_one_language_refuses_only_that_side() {
        let english_only = engine(
            "whisper-en",
            Backend::Asr(AsrBackend::Whisper),
            &["en"],
            true,
        );
        assert!(direction(Side::Left, &settings(), Some(&english_only), &voices()).is_ok());
        let why = direction(Side::Right, &settings(), Some(&english_only), &voices())
            .expect_err("it cannot hear Spanish");
        assert!(why.contains("\"es\""), "{why}");
    }

    #[test]
    fn no_voice_for_the_target_language_is_refused() {
        let only_english = vec![voice("piper-en", "en")];
        let why = direction(Side::Left, &settings(), Some(&whisper()), &only_english)
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
        assert!(direction(Side::Left, &settings(), Some(&broken), &voices()).is_err());
        assert!(direction(Side::Left, &settings(), None, &voices()).is_err());

        let mut same = settings();
        same.right_language = "en".into();
        assert!(direction(Side::Left, &same, Some(&whisper()), &voices()).is_err());

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
        let r = direction(Side::Left, &s, Some(&whisper()), &vs).expect("falls back");
        assert_eq!(r.direction.voice, "piper-es-es");
        assert!(r.warning.is_some());
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
