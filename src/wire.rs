//! The paired-mode wire protocol (SPEC §9): newline-delimited JSON over TCP.
//!
//! Chosen so a connection can be inspected and faked with `netcat`. Only text
//! crosses it: never audio, never models.
//!
//! Everything that arrives is untrusted. [`decode`] bounds every length,
//! refuses a line that is not UTF-8, refuses an unknown protocol version, and
//! strips control characters from every string, so what comes out is only
//! ever displayed or spoken. Received text is never a command, a path, or an
//! instruction to a model, and `source_text` is never translated again.

use serde::{Deserialize, Serialize};

/// The protocol version this build speaks.
pub const PROTO: u32 = 1;

/// The longest line accepted, in bytes, newline included. An utterance is a
/// few hundred bytes; anything near this is not one.
pub const MAX_LINE: usize = 16 * 1024;

/// The longest text or source text accepted, in characters.
pub const MAX_TEXT: usize = 2_000;

/// The longest name accepted, in characters.
pub const MAX_NAME: usize = 64;

/// The longest language code accepted. Codes are things like "en" or "pt-BR".
const MAX_LANG: usize = 16;

/// One message. The `t` field names the variant, as in SPEC §9.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Wire {
    /// Sent by both ends as soon as a connection is up.
    Hello {
        name: String,
        /// The language this end's person speaks.
        speaks: String,
        /// The language this end translates into, and so sends.
        sends: String,
        proto: u32,
    },
    /// One translated utterance. `lang` decides which voice speaks it; the
    /// receiver never infers it.
    Utterance {
        seq: u64,
        lang: String,
        text: String,
        source_lang: String,
        /// Display only. Never translated again.
        source_text: String,
    },
    FloorRequest {
        seq: u64,
    },
    FloorGrant {
        seq: u64,
    },
    FloorRelease {
        seq: u64,
    },
    Ping,
    Pong,
    /// Closing. `reason` is optional, so a bare `{"t":"Bye"}` is still valid;
    /// it is how a second connection learns why it was refused.
    Bye {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// Why a line was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WireError {
    #[error("a message longer than {MAX_LINE} bytes arrived")]
    TooLong,
    #[error("a message that is not UTF-8 text arrived")]
    NotUtf8,
    #[error(
        "the other side speaks protocol version {0}; this cnverc speaks version {PROTO}. \
         Use the same cnverc version on both PCs."
    )]
    UnknownProto(u64),
    #[error("a message cnverc does not understand arrived: {0}")]
    Malformed(String),
}

/// One message as a line, newline included.
pub fn encode(message: &Wire) -> String {
    // A Wire holds only strings and integers, so it always serialises.
    let mut line = serde_json::to_string(message).unwrap_or_else(|_| "{\"t\":\"Ping\"}".into());
    line.push('\n');
    line
}

/// Parse and check one received line. A trailing newline is allowed.
pub fn decode(line: &[u8]) -> Result<Wire, WireError> {
    if line.len() > MAX_LINE {
        return Err(WireError::TooLong);
    }
    let text = std::str::from_utf8(line).map_err(|_| WireError::NotUtf8)?;
    let value: serde_json::Value =
        serde_json::from_str(text.trim_end()).map_err(|e| WireError::Malformed(e.to_string()))?;

    // Any message carrying a version other than ours is refused, not only
    // Hello, so a newer peer is told plainly rather than half-understood.
    if let Some(proto) = value.get("proto") {
        match proto.as_u64() {
            Some(n) if n == u64::from(PROTO) => {}
            Some(n) => return Err(WireError::UnknownProto(n)),
            None => return Err(WireError::Malformed("\"proto\" is not a number".into())),
        }
    }

    let message: Wire =
        serde_json::from_value(value).map_err(|e| WireError::Malformed(e.to_string()))?;
    sanitise(message)
}

/// Bound and clean every string in a received message.
fn sanitise(message: Wire) -> Result<Wire, WireError> {
    Ok(match message {
        Wire::Hello {
            name,
            speaks,
            sends,
            proto,
        } => Wire::Hello {
            name: bounded(&name, MAX_NAME, "name")?,
            speaks: lang(&speaks)?,
            sends: lang(&sends)?,
            proto,
        },
        Wire::Utterance {
            seq,
            lang: l,
            text,
            source_lang,
            source_text,
        } => Wire::Utterance {
            seq,
            lang: lang(&l)?,
            text: bounded(&text, MAX_TEXT, "text")?,
            source_lang: lang(&source_lang)?,
            source_text: bounded(&source_text, MAX_TEXT, "source_text")?,
        },
        Wire::Bye { reason } => Wire::Bye {
            reason: match reason {
                Some(r) => Some(bounded(&r, MAX_TEXT, "reason")?),
                None => None,
            },
        },
        other => other,
    })
}

/// Control characters become spaces, runs of whitespace collapse, and
/// anything over `max` characters is refused rather than cut: a truncated
/// sentence would be spoken as if it were the whole one.
fn bounded(text: &str, max: usize, field: &str) -> Result<String, WireError> {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() > max {
        return Err(WireError::Malformed(format!(
            "\"{field}\" is longer than {max} characters"
        )));
    }
    Ok(cleaned)
}

/// A language code: letters and hyphens, short.
fn lang(code: &str) -> Result<String, WireError> {
    let ok = !code.is_empty()
        && code.len() <= MAX_LANG
        && code.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
    if ok {
        Ok(code.to_string())
    } else {
        Err(WireError::Malformed(format!(
            "{code:?} is not a language code"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_examples_parse() {
        let lines = [
            r#"{"t":"Hello","name":"laptop-a","speaks":"es","sends":"en","proto":1}"#,
            r#"{"t":"Utterance","seq":17,"lang":"en","text":"Where is the station?","source_lang":"es","source_text":"¿Dónde está la estación?"}"#,
            r#"{"t":"FloorRequest","seq":18}"#,
            r#"{"t":"FloorGrant","seq":18}"#,
            r#"{"t":"FloorRelease","seq":18}"#,
            r#"{"t":"Ping"}"#,
            r#"{"t":"Pong"}"#,
            r#"{"t":"Bye"}"#,
        ];
        for line in lines {
            decode(line.as_bytes()).unwrap_or_else(|e| panic!("{line}: {e}"));
        }
        assert_eq!(
            decode(lines[1].as_bytes()),
            Ok(Wire::Utterance {
                seq: 17,
                lang: "en".into(),
                text: "Where is the station?".into(),
                source_lang: "es".into(),
                source_text: "¿Dónde está la estación?".into(),
            })
        );
    }

    #[test]
    fn what_is_sent_is_one_line_that_decodes_to_itself() {
        let message = Wire::Utterance {
            seq: 3,
            lang: "es".into(),
            text: "Hola, \"amigo\".".into(),
            source_lang: "en".into(),
            source_text: "Hello, \"friend\".".into(),
        };
        let line = encode(&message);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1, "one message is one line");
        assert_eq!(decode(line.as_bytes()), Ok(message));
    }

    #[test]
    fn an_unknown_protocol_version_is_refused_by_name() {
        let error = decode(br#"{"t":"Hello","name":"x","speaks":"es","sends":"en","proto":2}"#)
            .expect_err("version 2");
        assert_eq!(error, WireError::UnknownProto(2));
        assert!(error.to_string().contains("version 2"));
    }

    #[test]
    fn hostile_input_is_refused_or_made_harmless() {
        assert_eq!(decode(b"\xff\xfe{}"), Err(WireError::NotUtf8));
        assert_eq!(decode(&vec![b' '; MAX_LINE + 1]), Err(WireError::TooLong));
        assert!(matches!(decode(b"not json"), Err(WireError::Malformed(_))));
        assert!(matches!(
            decode(br#"{"t":"Shutdown"}"#),
            Err(WireError::Malformed(_))
        ));

        // A language code is where a path or a prompt would try to hide.
        let path = r#"{"t":"Utterance","seq":1,"lang":"../../models","text":"x","source_lang":"es","source_text":"y"}"#;
        assert!(matches!(
            decode(path.as_bytes()),
            Err(WireError::Malformed(_))
        ));

        // Too long is refused, not truncated into a different sentence.
        let long = format!(
            r#"{{"t":"Utterance","seq":1,"lang":"en","text":"{}","source_lang":"es","source_text":"y"}}"#,
            "a".repeat(MAX_TEXT + 1)
        );
        assert!(matches!(
            decode(long.as_bytes()),
            Err(WireError::Malformed(_))
        ));

        // Control characters, including escaped newlines, become spaces.
        // The escapes are written into the JSON at run time, so the parser
        // accepts them and sanitising is what removes them: a newline and a
        // bell character, as JSON escapes.
        let sneaky = format!(
            r#"{{"t":"Hello","name":"a{e}nb{e}u0007c","speaks":"es","sends":"en","proto":1}}"#,
            e = char::from(92) // a backslash
        );
        let Ok(Wire::Hello { name, .. }) = decode(sneaky.as_bytes()) else {
            panic!("should parse");
        };
        assert_eq!(name, "a b c");
    }

    #[test]
    fn a_bye_may_say_why() {
        assert_eq!(
            decode(br#"{"t":"Bye","reason":"already paired with laptop-a"}"#),
            Ok(Wire::Bye {
                reason: Some("already paired with laptop-a".into())
            })
        );
        assert_eq!(encode(&Wire::Bye { reason: None }), "{\"t\":\"Bye\"}\n");
    }
}
