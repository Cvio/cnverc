//! Command line. Deliberately tiny: the GUI is Milestone 5 and is where the
//! controls belong. These flags exist so each milestone can be verified from a
//! shell before there is anything to click.

use anyhow::{anyhow, Result};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Open the window. What a double-click from Explorer does.
    Gui,
    /// Print the model discovery table and exit (Milestone 0).
    Report,
    /// List audio devices and exit.
    Devices,
    /// Capture from the microphone, transcribe, and log the result.
    Listen {
        /// Stop after this many seconds. `None` = until Ctrl-C.
        seconds: Option<u64>,
        /// Write each utterance to `logs/segments/` as a WAV.
        write_wav: bool,
        /// Run every enabled segment engine on each utterance (SPEC §12).
        compare: bool,
    },
    /// Print the exact prompt the translator is sent, with `{text}` where the
    /// user's words go, and exit (M7.8, for training the translator on it).
    PrintPrompt {
        source: String,
        target: String,
    },
    /// Translate stdin, one sentence per line, to stdout, one line each (M7.8,
    /// for scoring a trained translator the way a live session runs it).
    Translate {
        source: String,
        target: String,
    },
    /// Transcribe WAV files (16 kHz mono) with one recognizer, one line per
    /// file (M7.9, so model-bench can prove it hears exactly as volis does).
    Transcribe {
        engine: String,
        language: String,
        wavs: Vec<String>,
    },
    Help,
}

impl Command {
    /// Commands whose stdout is data for another program: nothing but their
    /// result may go there, so the banner and log lines go to stderr.
    pub fn stdout_is_data(&self) -> bool {
        matches!(
            self,
            Command::PrintPrompt { .. } | Command::Translate { .. } | Command::Transcribe { .. }
        )
    }
}

pub const HELP: &str = "\
Volis - offline speech-to-speech translation

USAGE:
    volis [COMMAND]

COMMANDS:
    (none)              Open the window
    --report            Print the discovered models and exit
    --devices           List audio input and output devices and exit
    --listen            Capture from the microphone and transcribe
    --print-prompt <SOURCE> <TARGET>
                        Print the exact prompt the translator is sent, with
                        {text} where the words go (for training a translator)
    --translate <SOURCE> <TARGET>
                        Translate stdin, one sentence per line, to stdout, one
                        line each; a refused line is printed empty and the
                        reason goes to stderr (for scoring a translator)
    --transcribe <RECOGNIZER> <LANGUAGE> <FILE.wav>...
                        Transcribe 16 kHz mono WAV files with the recognizer in
                        models/asr/<RECOGNIZER>, one line per file (for
                        checking a test bench hears as volis does)

SOURCE and TARGET are language or variety tags, e.g. en, es, es-MX.

OPTIONS FOR --listen:
    --seconds <N>       Stop cleanly after N seconds (otherwise: Ctrl-C)
    --wav               Write each detected utterance to logs/segments/
    --compare           Run every enabled engine on each utterance and print
                        transcripts, timings and segment durations side by side

    -h, --help          Show this message

Volis never accesses the internet. Models are read from models/ beside the
executable; see README.md for what to put there.
";

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Gui);
    };

    match first.as_str() {
        "-h" | "--help" => Ok(Command::Help),
        "--report" => reject_extra(args).map(|()| Command::Report),
        "--devices" => reject_extra(args).map(|()| Command::Devices),
        "--print-prompt" => {
            let (source, target) = two_tags(&mut args, "--print-prompt")?;
            Ok(Command::PrintPrompt { source, target })
        }
        "--translate" => {
            let (source, target) = two_tags(&mut args, "--translate")?;
            Ok(Command::Translate { source, target })
        }
        "--transcribe" => {
            let usage = "--transcribe needs a recognizer, a language and at least one WAV file, \
                         e.g. --transcribe whisper-large-v3-turbo fa clip.wav";
            let (Some(engine), Some(language)) = (args.next(), args.next()) else {
                return Err(anyhow!("{usage}"));
            };
            crate::varieties::require(&language).map_err(|e| anyhow!(e))?;
            let wavs: Vec<String> = args.collect();
            if wavs.is_empty() {
                return Err(anyhow!("{usage}"));
            }
            Ok(Command::Transcribe {
                engine,
                language,
                wavs,
            })
        }
        "--listen" => {
            let mut seconds = None;
            let mut write_wav = false;
            let mut compare = false;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--wav" => write_wav = true,
                    "--compare" => compare = true,
                    "--seconds" => {
                        let value = args
                            .next()
                            .ok_or_else(|| anyhow!("--seconds needs a number of seconds"))?;
                        seconds = Some(
                            value
                                .parse::<u64>()
                                .map_err(|_| anyhow!("--seconds {value:?} is not a number"))?,
                        );
                    }
                    other => return Err(unknown(other)),
                }
            }
            Ok(Command::Listen {
                seconds,
                write_wav,
                compare,
            })
        }
        other => Err(unknown(other)),
    }
}

/// SOURCE and TARGET for --print-prompt and --translate, each checked against
/// the varieties table, so an unknown tag is refused by name before anything
/// loads (the translator would refuse it anyway).
fn two_tags<I: Iterator<Item = String>>(args: &mut I, flag: &str) -> Result<(String, String)> {
    let (Some(source), Some(target)) = (args.next(), args.next()) else {
        return Err(anyhow!(
            "{flag} needs a source and a target, e.g. {flag} en es-MX"
        ));
    };
    for tag in [&source, &target] {
        crate::varieties::require(tag).map_err(|e| anyhow!(e))?;
    }
    reject_extra(args)?;
    Ok((source, target))
}

fn reject_extra<I: Iterator<Item = String>>(mut args: I) -> Result<()> {
    match args.next() {
        None => Ok(()),
        Some(other) => Err(unknown(&other)),
    }
}

fn unknown(arg: &str) -> anyhow::Error {
    anyhow!("unknown argument {arg:?}\n\n{HELP}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_opens_the_window() {
        // A double-click from Explorer passes no arguments.
        assert_eq!(parse_args(&[]).unwrap(), Command::Gui);
        assert_eq!(parse_args(&["--report"]).unwrap(), Command::Report);
    }

    #[test]
    fn listen_takes_its_options_in_any_order() {
        assert_eq!(
            parse_args(&["--listen", "--wav", "--seconds", "20"]).unwrap(),
            Command::Listen {
                seconds: Some(20),
                write_wav: true,
                compare: false
            }
        );
        assert_eq!(
            parse_args(&["--listen", "--seconds", "5"]).unwrap(),
            Command::Listen {
                seconds: Some(5),
                write_wav: false,
                compare: false
            }
        );
    }

    #[test]
    fn compare_is_a_listen_option() {
        assert_eq!(
            parse_args(&["--listen", "--compare"]).unwrap(),
            Command::Listen {
                seconds: None,
                write_wav: false,
                compare: true
            }
        );
    }

    #[test]
    fn a_bad_argument_is_rejected_with_the_help_text() {
        let err = parse_args(&["--not-a-command"]).unwrap_err().to_string();
        assert!(err.contains("--not-a-command"), "{err}");
        assert!(err.contains("USAGE"), "{err}");
        assert!(parse_args(&["--seconds"]).is_err());
        assert!(parse_args(&["--listen", "--seconds", "soon"]).is_err());
        assert!(parse_args(&["--devices", "--wav"]).is_err());
    }

    #[test]
    fn print_prompt_and_translate_take_two_known_tags() {
        assert_eq!(
            parse_args(&["--print-prompt", "en", "es-MX"]).unwrap(),
            Command::PrintPrompt {
                source: "en".into(),
                target: "es-MX".into()
            }
        );
        assert_eq!(
            parse_args(&["--translate", "es-MX", "en"]).unwrap(),
            Command::Translate {
                source: "es-MX".into(),
                target: "en".into()
            }
        );
        assert!(Command::PrintPrompt {
            source: "en".into(),
            target: "es".into()
        }
        .stdout_is_data());
        assert!(!Command::Report.stdout_is_data());
    }

    #[test]
    fn transcribe_takes_a_recognizer_a_language_and_files() {
        assert_eq!(
            parse_args(&[
                "--transcribe",
                "whisper-large-v3-turbo",
                "fa",
                "a.wav",
                "b.wav"
            ])
            .unwrap(),
            Command::Transcribe {
                engine: "whisper-large-v3-turbo".into(),
                language: "fa".into(),
                wavs: vec!["a.wav".into(), "b.wav".into()]
            }
        );
        assert!(parse_args(&["--transcribe", "whisper-large-v3-turbo", "fa"]).is_err());
        assert!(parse_args(&["--transcribe", "whisper-large-v3-turbo"]).is_err());
        let unknown = parse_args(&["--transcribe", "w", "xx", "a.wav"])
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("xx"), "{unknown}");
    }

    #[test]
    fn print_prompt_and_translate_refuse_a_missing_or_unknown_tag() {
        let missing = parse_args(&["--print-prompt", "en"])
            .unwrap_err()
            .to_string();
        assert!(missing.contains("needs a source and a target"), "{missing}");
        assert!(parse_args(&["--translate"]).is_err());
        let unknown = parse_args(&["--print-prompt", "en", "xx-XX"])
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("xx-XX"), "{unknown}");
        assert!(parse_args(&["--translate", "zz", "en"]).is_err());
        assert!(parse_args(&["--translate", "en", "es", "fr"]).is_err());
    }
}
