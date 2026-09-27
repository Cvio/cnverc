//! Model discovery: the filesystem is the index (SPEC §6).
//!
//! `Volis` enumerates the subdirectories of `models/asr/` and `models/tts/`,
//! parses the `engine.toml` in each, and reports what it found. Adding a model
//! means dropping a folder in and restarting. There is no registry, no
//! database, no cache, and no download UI - and every file on disk keeps the
//! name it was published with.
//!
//! Three outcomes per directory, and the difference between them matters:
//!
//! * **Skipped** - no `engine.toml`. Warned about by directory name.
//! * **Failed** - `engine.toml` is present but wrong (unknown backend, missing
//!   required file role, malformed TOML). A hard error *for that entry*, never
//!   a panic, and listed so the user can see why.
//! * **Loaded** - usable, unless a declared file is absent from disk, in which
//!   case the entry is listed but **disabled** with the missing filename shown.
//!   The user's most likely question is "why isn't my model in the list", and
//!   the answer has to be on screen.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use tracing::warn;

use crate::varieties;

/// The file that describes a model directory to `Volis`.
pub const ENGINE_TOML: &str = "engine.toml";

/// Interaction shape of a recognizer (SPEC §11). Whisper and Parakeet hand
/// back a whole utterance; streaming recognizers are fed and polled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Segment,
    Stream,
}

impl fmt::Display for EngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineKind::Segment => f.write_str("segment"),
            EngineKind::Stream => f.write_str("stream"),
        }
    }
}

/// Which sherpa-onnx model config variant an ASR directory maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrBackend {
    NemoTransducer,
    Whisper,
}

/// Which sherpa-onnx `OfflineTts` model config variant a TTS directory maps to.
///
/// Only what Volis can actually load is listed. sherpa-onnx supports several
/// more, but claiming them in discovery and then failing at load time would
/// put the error in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsBackend {
    Vits,
}

/// A backend, whichever side of the pipeline it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Asr(AsrBackend),
    Tts(TtsBackend),
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Backend::Asr(AsrBackend::NemoTransducer) => "nemo_transducer",
            Backend::Asr(AsrBackend::Whisper) => "whisper",
            Backend::Tts(TtsBackend::Vits) => "vits",
        })
    }
}

/// Which part of the model tree is being enumerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Asr,
    Tts,
}

impl Role {
    fn parse_backend(self, s: &str) -> Option<Backend> {
        match (self, s) {
            (Role::Asr, "nemo_transducer") => Some(Backend::Asr(AsrBackend::NemoTransducer)),
            (Role::Asr, "whisper") => Some(Backend::Asr(AsrBackend::Whisper)),
            (Role::Tts, "vits") => Some(Backend::Tts(TtsBackend::Vits)),
            _ => None,
        }
    }

    fn known_backends(self) -> &'static [&'static str] {
        match self {
            Role::Asr => &["nemo_transducer", "whisper"],
            Role::Tts => &["vits"],
        }
    }
}

/// The file roles a backend cannot work without. A role listed here but absent
/// from `[files]` is a hard error for the entry; a role present but pointing at
/// a file that is not on disk disables the entry instead.
fn required_files(backend: Backend) -> &'static [&'static str] {
    match backend {
        Backend::Asr(AsrBackend::NemoTransducer) => &["encoder", "decoder", "joiner", "tokens"],
        Backend::Asr(AsrBackend::Whisper) => &["encoder", "decoder", "tokens"],
        Backend::Tts(TtsBackend::Vits) => &["model", "tokens"],
    }
}

/// Why one `engine.toml` could not be turned into an engine description.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("unknown backend \"{backend}\" in {path} (known: {})", known.join(", "))]
    UnknownBackend {
        path: PathBuf,
        backend: String,
        known: &'static [&'static str],
    },
    #[error("{path} declares backend \"{backend}\" but [files].{role} is missing")]
    MissingRole {
        path: PathBuf,
        backend: Backend,
        role: &'static str,
    },
    #[error("{path} lists variety \"{variety}\": {reason}")]
    BadVariety {
        path: PathBuf,
        variety: String,
        reason: String,
    },
    #[error("{path} has [files].{role} = \"{value}\"; it must be a plain filename inside the model directory")]
    NotAPlainFilename {
        path: PathBuf,
        role: String,
        value: String,
    },
}

/// One declared file and whether it is actually there.
#[derive(Debug, Clone)]
pub struct ModelFile {
    /// The role the `engine.toml` gave it: `encoder`, `tokens`, …
    pub role: String,
    /// The published filename, kept verbatim (SPEC §2.5).
    pub name: String,
    pub path: PathBuf,
    pub present: bool,
}

/// A model directory that parsed cleanly.
#[derive(Debug, Clone)]
pub struct Engine {
    /// A directory the model needs beside its files, named by `data_dir` in
    /// `engine.toml`. Piper voices need `espeak-ng-data/` for pronunciation,
    /// and a directory cannot be declared under `[files]`.
    pub data_dir: Option<ModelDir>,
    /// Directory name - the identity used in `volis.toml` (SPEC §7).
    pub dir_name: String,
    pub dir: PathBuf,
    /// Human-readable name from `engine.toml`.
    pub name: String,
    pub kind: EngineKind,
    pub backend: Backend,
    pub languages: Vec<String>,
    /// The varieties (dialects) it is tuned for, e.g. `es-MX`. Optional; each
    /// belongs to one of `languages`. Empty means general.
    pub varieties: Vec<String>,
    /// Declared files, keyed by role, in a stable order for display.
    pub files: Vec<ModelFile>,
}

/// How well a model fits a language or variety, best first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// Tuned for exactly the variety asked for.
    Tuned,
    /// Covers the language, with no variety of it declared.
    General,
    /// Tuned for a different variety of the same language, named so nobody
    /// picks it by accident.
    Other(String),
}

impl Fit {
    /// How a picker labels it.
    pub fn label(&self, asked: &str) -> String {
        match self {
            Fit::Tuned => format!("tuned for {}", varieties::display_name(asked)),
            Fit::General => "general".to_string(),
            Fit::Other(variety) => format!("tuned for {}", varieties::display_name(variety)),
        }
    }
}

/// A model and how it fits.
#[derive(Debug, Clone)]
pub struct Ranked<'a> {
    pub engine: &'a Engine,
    pub fit: Fit,
}

/// The usable models for a language or variety (`es`, `es-MX`), best first:
/// tuned for exactly that variety, then general models of its language, then
/// models tuned for another variety of it. For a plain language, general
/// models come first. Models that don't cover the language at all aren't
/// listed. Within each group, discovery order (folder name) is kept.
pub fn rank<'a>(tag: &str, engines: &'a [Engine]) -> Vec<Ranked<'a>> {
    let language = varieties::language_of(tag);
    let mut ranked: Vec<Ranked<'a>> = engines
        .iter()
        .filter(|e| e.enabled() && e.languages.iter().any(|l| l.eq_ignore_ascii_case(language)))
        .map(|engine| {
            let own: Vec<&String> = engine
                .varieties
                .iter()
                .filter(|v| varieties::language_of(v).eq_ignore_ascii_case(language))
                .collect();
            let fit = if own.iter().any(|v| v.eq_ignore_ascii_case(tag.trim())) {
                Fit::Tuned
            } else if let Some(other) = own.first() {
                Fit::Other((*other).clone())
            } else {
                Fit::General
            };
            Ranked { engine, fit }
        })
        .collect();
    ranked.sort_by_key(|r| match r.fit {
        Fit::Tuned => 0,
        Fit::General => 1,
        Fit::Other(_) => 2,
    });
    ranked
}

/// A declared support directory and whether it is actually there.
#[derive(Debug, Clone)]
pub struct ModelDir {
    pub name: String,
    pub path: PathBuf,
    pub present: bool,
}

impl Engine {
    /// Declared files and directories that are not on disk.
    pub fn missing_files(&self) -> Vec<&str> {
        let mut missing: Vec<&str> = self
            .files
            .iter()
            .filter(|f| !f.present)
            .map(|f| f.name.as_str())
            .collect();
        if let Some(dir) = &self.data_dir {
            if !dir.present {
                missing.push(dir.name.as_str());
            }
        }
        missing
    }

    /// Usable: everything it declares is present.
    pub fn enabled(&self) -> bool {
        self.missing_files().is_empty()
    }
}

/// A directory under `models/asr/` or `models/tts/` that had an `engine.toml`,
/// whether or not that file made sense.
#[derive(Debug)]
pub enum Entry {
    Loaded(Engine),
    /// `engine.toml` present but unusable. Listed, never silently dropped.
    Failed {
        dir_name: String,
        error: EngineError,
    },
}

impl Entry {
    pub fn dir_name(&self) -> &str {
        match self {
            Entry::Loaded(e) => &e.dir_name,
            Entry::Failed { dir_name, .. } => dir_name,
        }
    }
}

/// The literal shape of an `engine.toml` (SPEC §6). Unknown keys are rejected
/// so a typo surfaces as a listed error instead of a silently ignored line.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEngine {
    name: String,
    kind: EngineKind,
    backend: String,
    #[serde(default)]
    languages: Vec<String>,
    /// Optional: the varieties it is tuned for, e.g. `["es-MX"]`.
    #[serde(default)]
    varieties: Vec<String>,
    /// Optional support directory, e.g. `espeak-ng-data` for a Piper voice.
    #[serde(default)]
    data_dir: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

/// Parse one model directory's `engine.toml`.
pub fn load_engine(dir: &Path, role: Role) -> Result<Engine, EngineError> {
    let toml_path = dir.join(ENGINE_TOML);
    let text = std::fs::read_to_string(&toml_path).map_err(|source| EngineError::Read {
        path: toml_path.clone(),
        source,
    })?;
    let raw: RawEngine = toml::from_str(&text).map_err(|source| EngineError::Parse {
        path: toml_path.clone(),
        source: Box::new(source),
    })?;

    let backend = role
        .parse_backend(&raw.backend)
        .ok_or_else(|| EngineError::UnknownBackend {
            path: toml_path.clone(),
            backend: raw.backend.clone(),
            known: role.known_backends(),
        })?;

    for role_name in required_files(backend) {
        if !raw.files.contains_key(*role_name) {
            return Err(EngineError::MissingRole {
                path: toml_path.clone(),
                backend,
                role: role_name,
            });
        }
    }

    let mut files = Vec::with_capacity(raw.files.len());
    for (file_role, name) in raw.files {
        if !is_plain_filename(&name) {
            return Err(EngineError::NotAPlainFilename {
                path: toml_path.clone(),
                role: file_role,
                value: name,
            });
        }
        let path = dir.join(&name);
        let present = path.is_file();
        files.push(ModelFile {
            role: file_role,
            name,
            path,
            present,
        });
    }

    let data_dir = match raw.data_dir {
        None => None,
        Some(name) => {
            if !is_plain_filename(&name) {
                return Err(EngineError::NotAPlainFilename {
                    path: toml_path.clone(),
                    role: "data_dir".to_string(),
                    value: name,
                });
            }
            let path = dir.join(&name);
            let present = path.is_dir();
            Some(ModelDir {
                name,
                path,
                present,
            })
        }
    };

    // Every variety must be one Volis knows, and belong to a language the
    // model lists: a Spanish voice tuned for "fr-FR" is a mistake to show,
    // not a model to use.
    for variety in &raw.varieties {
        let reason = match varieties::lookup(variety) {
            None => Some("not a variety Volis knows (add it to src/varieties.rs)".to_string()),
            Some(_) if !varieties::has_variety(variety) => Some(
                "a variety names a region, like \"es-MX\"; plain languages go in `languages`"
                    .to_string(),
            ),
            Some(_) => {
                let language = varieties::language_of(variety);
                (!raw
                    .languages
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(language)))
                .then(|| {
                    format!(
                        "its language \"{language}\" is not in `languages` ({})",
                        raw.languages.join(", ")
                    )
                })
            }
        };
        if let Some(reason) = reason {
            return Err(EngineError::BadVariety {
                path: toml_path.clone(),
                variety: variety.clone(),
                reason,
            });
        }
    }

    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(Engine {
        dir_name,
        data_dir,
        dir: dir.to_path_buf(),
        name: raw.name,
        kind: raw.kind,
        backend,
        languages: raw.languages,
        varieties: raw.varieties,
        files,
    })
}

/// A `[files]` value must name a file inside the model directory: no
/// separators, no drive letters, no `..`. Keeps the model tree readable and
/// keeps a hand-edited `engine.toml` from reaching outside its own folder.
fn is_plain_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
}

/// Enumerate one model root. A missing root is reported as an empty list plus
/// a warning naming the absolute path; it is the caller's business whether
/// that is fatal.
pub fn discover(root: &Path, role: Role) -> Vec<Entry> {
    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) => {
            warn!("cannot read model directory {}: {e}", root.display());
            return Vec::new();
        }
    };

    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) if entry.path().is_dir() => dirs.push(entry.path()),
            Ok(_) => {}
            Err(e) => warn!("cannot read an entry of {}: {e}", root.display()),
        }
    }
    // Filesystem order is not guaranteed; the UI list must be stable.
    dirs.sort();

    let mut entries = Vec::new();
    for dir in dirs {
        if !dir.join(ENGINE_TOML).is_file() {
            warn!(
                "skipping {}: no {ENGINE_TOML} (expected {})",
                dir.display(),
                dir.join(ENGINE_TOML).display()
            );
            continue;
        }
        let dir_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match load_engine(&dir, role) {
            Ok(engine) => entries.push(Entry::Loaded(engine)),
            Err(error) => {
                warn!("{error}");
                entries.push(Entry::Failed { dir_name, error })
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A scratch model directory under the OS temp dir, removed on drop.
    /// Test-only: nothing in the application itself writes outside app_root().
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "volis-test-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch dir");
            Scratch(dir)
        }

        fn model(&self, name: &str, engine_toml: &str, files: &[&str]) -> PathBuf {
            let dir = self.0.join(name);
            fs::create_dir_all(&dir).expect("model dir");
            fs::write(dir.join(ENGINE_TOML), engine_toml).expect("engine.toml");
            for f in files {
                fs::write(dir.join(f), b"stub").expect("model file");
            }
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const PARAKEET: &str = r#"
name = "Parakeet TDT 0.6B v3 (int8)"
kind = "segment"
backend = "nemo_transducer"
languages = ["es", "en"]

[files]
encoder = "encoder.int8.onnx"
decoder = "decoder.int8.onnx"
joiner  = "joiner.int8.onnx"
tokens  = "tokens.txt"
"#;

    const PARAKEET_FILES: &[&str] = &[
        "encoder.int8.onnx",
        "decoder.int8.onnx",
        "joiner.int8.onnx",
        "tokens.txt",
    ];

    #[test]
    fn loads_the_spec_example() {
        let scratch = Scratch::new("parakeet");
        let dir = scratch.model("parakeet-tdt-0.6b-v3-int8", PARAKEET, PARAKEET_FILES);

        let engine = load_engine(&dir, Role::Asr).expect("spec example must load");
        assert_eq!(engine.dir_name, "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(engine.kind, EngineKind::Segment);
        assert_eq!(engine.backend, Backend::Asr(AsrBackend::NemoTransducer));
        assert_eq!(engine.files.len(), 4);
        assert!(engine.enabled());
        assert!(engine.missing_files().is_empty());
    }

    #[test]
    fn a_missing_file_disables_rather_than_hides() {
        let scratch = Scratch::new("incomplete");
        let dir = scratch.model("parakeet", PARAKEET, &PARAKEET_FILES[..3]);

        let engine = load_engine(&dir, Role::Asr).expect("must still load");
        assert!(!engine.enabled());
        assert_eq!(engine.missing_files(), vec!["tokens.txt"]);
    }

    #[test]
    fn an_unknown_backend_is_an_error_for_that_entry_only() {
        let scratch = Scratch::new("backends");
        scratch.model(
            "bogus",
            "name = \"B\"\nkind = \"segment\"\nbackend = \"transformers\"\n",
            &[],
        );
        scratch.model("parakeet", PARAKEET, PARAKEET_FILES);

        let entries = discover(&scratch.0, Role::Asr);
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], Entry::Failed { .. }));
        assert!(matches!(&entries[1], Entry::Loaded(e) if e.enabled()));
    }

    fn voice_tuned(dir_name: &str, varieties: &[&str]) -> Engine {
        Engine {
            data_dir: None,
            dir_name: dir_name.to_string(),
            dir: PathBuf::from(dir_name),
            name: dir_name.to_string(),
            kind: EngineKind::Segment,
            backend: Backend::Tts(TtsBackend::Vits),
            languages: vec!["es".to_string()],
            varieties: varieties.iter().map(|v| v.to_string()).collect(),
            files: Vec::new(),
        }
    }

    #[test]
    fn models_are_ranked_tuned_then_general_then_other_varieties() {
        // Deliberately in the "wrong" folder order.
        let voices = [
            voice_tuned("a-spain", &["es-ES"]),
            voice_tuned("b-general", &[]),
            voice_tuned("c-mexico", &["es-MX"]),
        ];
        let order = |tag: &str| -> Vec<(String, Fit)> {
            rank(tag, &voices)
                .into_iter()
                .map(|r| (r.engine.dir_name.clone(), r.fit))
                .collect()
        };
        assert_eq!(
            order("es-MX"),
            vec![
                ("c-mexico".to_string(), Fit::Tuned),
                ("b-general".to_string(), Fit::General),
                ("a-spain".to_string(), Fit::Other("es-ES".to_string())),
            ]
        );
        // A plain language: general first, then the varieties, labelled.
        assert_eq!(order("es")[0], ("b-general".to_string(), Fit::General));
        assert_eq!(
            Fit::Other("es-ES".into()).label("es-MX"),
            "tuned for Spanish (Spain)"
        );
        assert_eq!(Fit::Tuned.label("es-MX"), "tuned for Spanish (Mexico)");
        // Another language entirely: nothing listed.
        assert!(rank("ar-IQ", &voices).is_empty());
    }

    #[test]
    fn varieties_must_belong_to_a_listed_language() {
        let voice = |varieties: &str| {
            format!(
                "name = \"V\"
kind = \"segment\"
backend = \"vits\"
languages = [\"es\"]
                 varieties = {varieties}

[files]
model = \"m.onnx\"
tokens = \"t.txt\"
"
            )
        };
        let scratch = Scratch::new("varieties");

        // The example from dialect-per-side.md: French variety, Spanish voice.
        let dir = scratch.model("wrong", &voice("[\"fr-FR\"]"), &[]);
        let err = load_engine(&dir, Role::Tts).expect_err("fr-FR is not Spanish");
        assert!(matches!(err, EngineError::BadVariety { .. }), "{err}");
        assert!(err.to_string().contains("fr-FR"), "{err}");

        let dir = scratch.model("plain", &voice("[\"es\"]"), &[]);
        assert!(
            load_engine(&dir, Role::Tts).is_err(),
            "a plain language is not a variety"
        );

        let dir = scratch.model("mexico", &voice("[\"es-MX\"]"), &[]);
        let engine = load_engine(&dir, Role::Tts).expect("es-MX belongs to es");
        assert_eq!(engine.varieties, vec!["es-MX"]);

        let dir = scratch.model("general", &voice("[]"), &[]);
        assert!(load_engine(&dir, Role::Tts)
            .expect("no varieties")
            .varieties
            .is_empty());

        // And through discovery, so --report shows it as broken, with the reason.
        let entries = discover(&scratch.0, Role::Tts);
        assert!(entries
            .iter()
            .any(|e| matches!(e, Entry::Failed { dir_name, .. } if dir_name == "wrong")));
    }

    #[test]
    fn a_required_role_absent_from_files_is_an_error() {
        let scratch = Scratch::new("role");
        let dir = scratch.model(
            "parakeet",
            "name = \"P\"\nkind = \"segment\"\nbackend = \"nemo_transducer\"\n\n[files]\nencoder = \"e.onnx\"\ndecoder = \"d.onnx\"\ntokens = \"t.txt\"\n",
            &[],
        );
        let err = load_engine(&dir, Role::Asr).expect_err("joiner is required");
        assert!(matches!(
            err,
            EngineError::MissingRole { role: "joiner", .. }
        ));
    }

    #[test]
    fn a_files_value_may_not_escape_the_model_directory() {
        let scratch = Scratch::new("escape");
        let dir = scratch.model(
            "vits",
            "name = \"V\"\nkind = \"segment\"\nbackend = \"vits\"\n\n[files]\nmodel = \"../elsewhere/model.onnx\"\ntokens = \"tokens.txt\"\n",
            &[],
        );
        let err = load_engine(&dir, Role::Tts).expect_err("must reject a path");
        assert!(matches!(err, EngineError::NotAPlainFilename { .. }));
    }

    #[test]
    fn a_directory_without_engine_toml_is_skipped() {
        let scratch = Scratch::new("skip");
        fs::create_dir_all(scratch.0.join("random-folder")).expect("dir");
        scratch.model("parakeet", PARAKEET, PARAKEET_FILES);

        let entries = discover(&scratch.0, Role::Asr);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].dir_name(), "parakeet");
    }

    #[test]
    fn an_asr_backend_is_not_a_tts_backend() {
        let scratch = Scratch::new("roles");
        let dir = scratch.model("parakeet", PARAKEET, PARAKEET_FILES);
        assert!(load_engine(&dir, Role::Tts).is_err());
    }

    #[test]
    fn a_missing_model_root_yields_nothing_and_does_not_panic() {
        let scratch = Scratch::new("absent");
        let entries = discover(&scratch.0.join("no-such-dir"), Role::Asr);
        assert!(entries.is_empty());
    }
}

/// The translation model: a single GGUF in `models/mt/` (SPEC §5).
///
/// There is no `volis.toml` key naming it, because §7 does not define one,
/// so the filesystem is the index here too: exactly one `.gguf` means that is
/// the model. Two means the user has to say which by removing one, and being
/// told that is better than Volis picking for them (SPEC §15).
pub fn find_translation_model(mt_dir: &Path) -> Result<PathBuf, TranslationModelError> {
    let read_dir =
        std::fs::read_dir(mt_dir).map_err(|source| TranslationModelError::Unreadable {
            path: mt_dir.to_path_buf(),
            source,
        })?;

    let mut found: Vec<PathBuf> = read_dir
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        })
        .collect();
    found.sort();

    match found.len() {
        0 => Err(TranslationModelError::None {
            path: mt_dir.to_path_buf(),
        }),
        1 => Ok(found.remove(0)),
        _ => Err(TranslationModelError::Several {
            path: mt_dir.to_path_buf(),
            names: found
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect(),
        }),
    }
}

/// Why no single translation model could be identified.
#[derive(Debug, Error)]
pub enum TranslationModelError {
    #[error("cannot read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "no .gguf translation model in {path}\nVolis never downloads models; put one there \
         (see README.md) and run again."
    )]
    None { path: PathBuf },
    #[error(
        "{path} holds {} translation models ({}); volis.toml has no key to choose between \
         them, so leave exactly one in place",
        names.len(), names.join(", ")
    )]
    Several { path: PathBuf, names: Vec<String> },
}
