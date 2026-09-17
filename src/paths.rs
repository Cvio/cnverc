//! Every path in `convers` derives from the executable's own location.
//!
//! SPEC §5: never the current working directory, never `%APPDATA%`, never the
//! `dirs`/`directories` crates. Double-clicking from Explorer and launching
//! from a shell must behave identically, and the whole folder must be movable
//! to any drive without changing behaviour.
//!
//! `app_root()` is the *only* path-derivation function in the codebase. Every
//! other location is a join off it, and those joins live here too so there is
//! one place to read.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The directory containing `convers.exe`.
pub fn app_root() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let root = exe.parent().context("exe has no parent")?;
    Ok(strip_verbatim(root))
}

/// `canonicalize()` on Windows hands back a verbatim path (`\\?\C:\...`). It is
/// correct but unreadable, and every error message in convers quotes an
/// absolute path back to the user (SPEC §14), so the prefix is dropped when
/// what remains is an ordinary drive path. Anything else passes through
/// untouched, including UNC paths, where the prefix is load-bearing.
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path.to_path_buf();
    };
    let mut chars = rest.chars();
    let looks_like_a_drive = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.next() == Some(':')
        && chars.next() == Some('\\');
    if looks_like_a_drive {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// `<root>/convers.toml` — user selections (SPEC §7).
pub fn config_file(root: &Path) -> PathBuf {
    root.join("convers.toml")
}

/// `<root>/models` — the model tree (SPEC §5).
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("models")
}

/// `<root>/models/vad/silero_vad.onnx`.
pub fn vad_model_file(root: &Path) -> PathBuf {
    models_dir(root).join("vad").join("silero_vad.onnx")
}

/// `<root>/models/asr` — one subdirectory per ASR engine.
pub fn asr_dir(root: &Path) -> PathBuf {
    models_dir(root).join("asr")
}

/// `<root>/models/tts` — one subdirectory per TTS voice.
pub fn tts_dir(root: &Path) -> PathBuf {
    models_dir(root).join("tts")
}

/// `<root>/models/mt` — translation GGUF files.
pub fn mt_dir(root: &Path) -> PathBuf {
    models_dir(root).join("mt")
}

/// `<root>/logs` — rolling log files, alongside stdout.
pub fn logs_dir(root: &Path) -> PathBuf {
    root.join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verbatim_drive_path_is_made_readable() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\D:\AI_Data\convers")),
            PathBuf::from(r"D:\AI_Data\convers")
        );
    }

    #[test]
    fn a_verbatim_unc_path_keeps_its_prefix() {
        let unc = Path::new(r"\\?\UNC\server\share\convers");
        assert_eq!(strip_verbatim(unc), unc.to_path_buf());
    }

    #[test]
    fn an_ordinary_path_is_untouched() {
        let plain = Path::new("/opt/convers");
        assert_eq!(strip_verbatim(plain), plain.to_path_buf());
    }

    #[test]
    fn every_location_hangs_off_the_root_it_is_given() {
        let root = Path::new(r"X:\somewhere\convers");
        assert_eq!(config_file(root), root.join("convers.toml"));
        assert!(asr_dir(root).starts_with(root));
        assert!(tts_dir(root).starts_with(root));
        assert!(mt_dir(root).starts_with(root));
        assert!(vad_model_file(root).starts_with(root));
        assert!(logs_dir(root).starts_with(root));
    }
}
