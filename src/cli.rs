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
    Help,
}

pub const HELP: &str = "\
cnverc — offline speech-to-speech translation

USAGE:
    cnverc [COMMAND]

COMMANDS:
    (none)              Open the window
    --report            Print the discovered models and exit
    --devices           List audio input and output devices and exit
    --listen            Capture from the microphone and transcribe

OPTIONS FOR --listen:
    --seconds <N>       Stop cleanly after N seconds (otherwise: Ctrl-C)
    --wav               Write each detected utterance to logs/segments/
    --compare           Run every enabled engine on each utterance and print
                        transcripts, timings and segment durations side by side

    -h, --help          Show this message

cnverc never accesses the internet. Models are read from models/ beside the
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
        let err = parse_args(&["--transcribe"]).unwrap_err().to_string();
        assert!(err.contains("--transcribe"), "{err}");
        assert!(err.contains("USAGE"), "{err}");
        assert!(parse_args(&["--seconds"]).is_err());
        assert!(parse_args(&["--listen", "--seconds", "soon"]).is_err());
        assert!(parse_args(&["--devices", "--wav"]).is_err());
    }
}
