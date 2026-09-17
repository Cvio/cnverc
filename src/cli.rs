//! Command line. Deliberately tiny: the GUI is Milestone 5 and is where the
//! controls belong. These flags exist so each milestone can be verified from a
//! shell before there is anything to click.

use anyhow::{anyhow, Result};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Print the model discovery table and exit (Milestone 0).
    Report,
    /// List audio input devices and exit.
    Devices,
    /// Capture from the microphone and log the utterances the VAD cuts.
    Listen {
        /// Stop after this many seconds. `None` = until Ctrl-C.
        seconds: Option<u64>,
        /// Write each utterance to `logs/segments/` as a WAV.
        write_wav: bool,
    },
    Help,
}

pub const HELP: &str = "\
convers — offline speech-to-speech translation

USAGE:
    convers [COMMAND]

COMMANDS:
    (none)              Print the discovered models and exit
    --devices           List audio input devices and exit
    --listen            Capture from the microphone and log detected speech

OPTIONS FOR --listen:
    --seconds <N>       Stop cleanly after N seconds (otherwise: Ctrl-C)
    --wav               Write each detected utterance to logs/segments/

    -h, --help          Show this message

convers never accesses the internet. Models are read from models/ beside the
executable; see README.md for what to put there.
";

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Report);
    };

    match first.as_str() {
        "-h" | "--help" => Ok(Command::Help),
        "--devices" => reject_extra(args).map(|()| Command::Devices),
        "--listen" => {
            let mut seconds = None;
            let mut write_wav = false;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--wav" => write_wav = true,
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
            Ok(Command::Listen { seconds, write_wav })
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
    fn no_arguments_is_the_report() {
        assert_eq!(parse_args(&[]).unwrap(), Command::Report);
    }

    #[test]
    fn listen_takes_its_options_in_any_order() {
        assert_eq!(
            parse_args(&["--listen", "--wav", "--seconds", "20"]).unwrap(),
            Command::Listen {
                seconds: Some(20),
                write_wav: true
            }
        );
        assert_eq!(
            parse_args(&["--listen", "--seconds", "5"]).unwrap(),
            Command::Listen {
                seconds: Some(5),
                write_wav: false
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
