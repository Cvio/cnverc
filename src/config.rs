//! `convers.toml` — user selections only, by directory name (SPEC §7).
//!
//! The file holds exactly the keys §7 lists and nothing else. Models are
//! referenced by the name of their directory under `models/`; no paths, no
//! hashes, no ids.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub asr: Asr,
    #[serde(default)]
    pub languages: Languages,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub vad: Vad,
    #[serde(default)]
    pub tts: Tts,
    #[serde(default)]
    pub peer: Peer,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asr {
    /// Directory name under `models/asr/`. Empty = nothing selected yet.
    pub engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Languages {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeKind {
    Continuous,
    Turn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnStyle {
    Toggle,
    Hold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    pub kind: ModeKind,
    /// Window-focused key only. Never a global hotkey (SPEC §8).
    pub turn_key: String,
    pub turn_style: TurnStyle,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    /// Empty = system default.
    pub input_device: String,
    pub output_device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vad {
    pub threshold: f32,
    pub min_silence_ms: u32,
    pub min_speech_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tts {
    pub enabled: bool,
    /// Gate capture while speaking (SPEC §10). False is for headphones only.
    pub half_duplex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub enabled: bool,
    pub listen_addr: String,
    /// e.g. "192.168.50.2:47800". Empty = not dialling anyone.
    pub peer_addr: String,
    /// Empty = hostname.
    pub display_name: String,
    /// mDNS/broadcast convenience; manual entry always available.
    pub discovery: bool,
}

impl Default for Languages {
    fn default() -> Self {
        Self {
            source: "es".to_string(),
            target: "en".to_string(),
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            kind: ModeKind::Turn,
            turn_key: "Space".to_string(),
            turn_style: TurnStyle::Toggle,
        }
    }
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            min_silence_ms: 500,
            min_speech_ms: 250,
        }
    }
}

impl Default for Tts {
    fn default() -> Self {
        Self {
            enabled: true,
            half_duplex: true,
        }
    }
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_addr: "0.0.0.0:47800".to_string(),
            peer_addr: String::new(),
            display_name: String::new(),
            discovery: true,
        }
    }
}

impl Config {
    /// Read `convers.toml`. A missing file is not an error — the defaults in
    /// §7 apply — but the absolute path that was tried is reported so the user
    /// knows where to create it. A malformed file *is* an error: silently
    /// falling back to defaults would hide the user's selections.
    pub fn load(path: &Path) -> Result<(Self, bool)> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config: Config = toml::from_str(&text)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                Ok((config, true))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Config::default(), false)),
            Err(e) => {
                Err(anyhow::Error::new(e).context(format!("failed to read {}", path.display())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact file from SPEC §7 must round-trip.
    const SPEC_EXAMPLE: &str = r#"
[asr]
engine = "parakeet-tdt-0.6b-v3-int8"

[languages]
source = "es"
target = "en"

[mode]
kind = "turn"
turn_key = "Space"
turn_style = "toggle"

[audio]
input_device = ""
output_device = ""

[vad]
threshold = 0.5
min_silence_ms = 500
min_speech_ms = 250

[tts]
enabled = true
half_duplex = true

[peer]
enabled = false
listen_addr = "0.0.0.0:47800"
peer_addr = ""
display_name = ""
discovery = true
"#;

    #[test]
    fn parses_the_spec_example() {
        let config: Config = toml::from_str(SPEC_EXAMPLE).expect("spec example must parse");
        assert_eq!(config.asr.engine, "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(config.languages.source, "es");
        assert_eq!(config.mode.kind, ModeKind::Turn);
        assert_eq!(config.mode.turn_style, TurnStyle::Toggle);
        assert_eq!(config.vad.min_silence_ms, 500);
        assert!(config.tts.half_duplex);
        assert_eq!(config.peer.listen_addr, "0.0.0.0:47800");
    }

    #[test]
    fn an_empty_file_is_the_documented_defaults() {
        let config: Config = toml::from_str("").expect("empty config must parse");
        assert_eq!(config.languages.target, "en");
        assert_eq!(config.mode.turn_key, "Space");
        assert!(config.asr.engine.is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected_rather_than_ignored() {
        assert!(toml::from_str::<Config>("[asr]\nengine = \"x\"\nmodel = \"y\"\n").is_err());
        assert!(toml::from_str::<Config>("[nonsense]\nx = 1\n").is_err());
    }
}
