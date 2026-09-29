//! `Volis` - offline speech-to-speech translation.
//!
//! Started with no arguments, as a double-click from Explorer does, it opens
//! the window. The command-line flags remain for shells, logs and the
//! milestone checks that were written against them.

mod asr;
mod audio;
mod cli;
mod compare;
mod config;
mod discovery;
mod floor;
mod gui;
mod listen;
mod models;
mod paths;
mod peer;
mod pipeline;
mod playback;
mod report;
mod ring;
mod shared;
mod translate;
mod tts;
mod vad;
mod varieties;
mod wav;
mod wire;

use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;

use crate::config::Config;
use crate::models::{Entry, Role};

fn main() -> Result<()> {
    let command = cli::parse(std::env::args().skip(1))?;
    if command == cli::Command::Help {
        print!("{}", cli::HELP);
        return Ok(());
    }
    // Needs no model and no files: exactly the prompt, nothing else on stdout.
    if let cli::Command::PrintPrompt { source, target } = &command {
        print!("{}", translate::prompt_template(source, target));
        return Ok(());
    }

    let root = paths::app_root().context("failed to locate the application directory")?;

    // Logging first, so everything after it is on the record. Held for the
    // lifetime of main: dropping the guard would stop the file writer. A
    // command whose stdout is data logs to stderr instead, and skips the banner.
    let data_on_stdout = command.stdout_is_data();
    let _log_guard = init_logging(&root, data_on_stdout);

    if !data_on_stdout {
        println!(
            "Volis {} - offline, no network required",
            env!("CARGO_PKG_VERSION")
        );
        println!("app root: {}", root.display());
    }

    if let cli::Command::Translate { source, target } = &command {
        return translate_stdin(&root, source, target);
    }
    if let cli::Command::Transcribe {
        engine,
        language,
        wavs,
    } = &command
    {
        return transcribe_files(&root, engine, language, wavs);
    }

    if command == cli::Command::Devices {
        return print_devices();
    }

    let config_path = paths::config_file(&root);
    let (config, config_found) = Config::load(&config_path)?;
    if config_found {
        println!("config:   {}", config_path.display());
    } else {
        println!(
            "config:   {} (not present; using defaults from SPEC §7)",
            config_path.display()
        );
    }

    match command {
        cli::Command::Gui => return gui::run(root, config),
        cli::Command::Listen {
            seconds,
            write_wav,
            compare,
        } => {
            let options = pipeline::Options { write_wav, compare };
            return listen::run(&root, &config, seconds, options);
        }
        cli::Command::Report
        | cli::Command::Devices
        | cli::Command::Help
        | cli::Command::PrintPrompt { .. }
        | cli::Command::Translate { .. }
        | cli::Command::Transcribe { .. } => {}
    }

    let models_root = paths::models_dir(&root);
    if !models_root.is_dir() {
        anyhow::bail!(
            "model directory not found: {}\n\
             Volis never downloads models. Create that directory and place the model \
             folders in it as described in README.md, then run again.",
            models_root.display()
        );
    }

    let asr_root = paths::asr_dir(&root);
    let tts_root = paths::tts_dir(&root);
    let asr = models::discover(&asr_root, Role::Asr);
    let tts = models::discover(&tts_root, Role::Tts);

    report::print_table("ASR engines", &asr_root, &asr);
    report::print_table("TTS voices", &tts_root, &tts);
    print_single_files(&root);

    println!();
    println!("summary:");
    report::print_summary(Role::Asr, &asr);
    report::print_summary(Role::Tts, &tts);
    report_selection(&config, &asr);

    Ok(())
}

/// `volis --translate`: stdin to stdout, one line each, with the model in
/// models/mt/ and every guard on, as a live session translates (M7.8).
fn translate_stdin(root: &Path, source: &str, target: &str) -> Result<()> {
    use crate::translate::LlamaTranslator;

    let mt_dir = paths::mt_dir(root);
    let model = models::find_translation_model(&mt_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut translator = LlamaTranslator::load(&model)?;
    tracing::info!(
        "translating stdin from {source} to {target} with {}",
        model.display()
    );
    translate::translate_lines(
        &mut translator,
        source,
        target,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        std::io::stderr(),
    )
}

/// `volis --transcribe`: WAV files through one recognizer, loaded once and
/// run exactly as a live session runs it, one line per file (M7.9). A file
/// that can't be read or transcribed gives an empty line and its reason on
/// stderr, so line N always belongs to file N.
fn transcribe_files(root: &Path, engine_name: &str, language: &str, wavs: &[String]) -> Result<()> {
    use std::io::Write as _;

    let asr_root = paths::asr_dir(root);
    let engines = models::discover(&asr_root, Role::Asr);
    let engine = engines
        .iter()
        .find_map(|e| match e {
            Entry::Loaded(engine) if engine.dir_name == engine_name => Some(engine),
            _ => None,
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no recognizer named \"{engine_name}\" in {}",
                asr_root.display()
            )
        })?;
    let base = varieties::language_of(language);
    if !engine
        .languages
        .iter()
        .any(|l| l.eq_ignore_ascii_case(base))
    {
        anyhow::bail!(
            "\"{engine_name}\" doesn't list the language \"{base}\" (it lists {}); see its engine.toml in {}",
            engine.languages.join(", "),
            engine.dir.display()
        );
    }
    let asr::AsrEngine::Segment(mut recognizer) = asr::load(engine)? else {
        anyhow::bail!("\"{engine_name}\" isn't a segment recognizer");
    };
    recognizer.prepare(language)?;
    tracing::info!(
        "transcribing {} file(s) with {engine_name} as {language}",
        wavs.len()
    );
    let mut out = std::io::stdout().lock();
    for (n, wav) in wavs.iter().enumerate() {
        let text = match wav::read_16k_mono(Path::new(wav))
            .and_then(|pcm| recognizer.transcribe(&pcm, language))
        {
            Ok(text) => text.split_whitespace().collect::<Vec<_>>().join(" "),
            Err(e) => {
                eprintln!("file {}: not transcribed: {e:#}", n + 1);
                String::new()
            }
        };
        writeln!(out, "{text}")?;
        out.flush()?;
    }
    Ok(())
}

/// Audio devices, so the user can put exact names in `[audio]`.
fn print_devices() -> Result<()> {
    print_device_list(
        "Audio input devices",
        "[audio].input_device",
        audio::list_input_devices()?,
    );
    print_device_list(
        "Audio output devices",
        "[audio].output_device",
        audio::list_output_devices()?,
    );
    println!();
    println!("  * = system default. Leave the setting empty to follow it.");
    Ok(())
}

fn print_device_list(title: &str, setting: &str, devices: Vec<audio::InputDevice>) {
    println!();
    println!("{title}  ({setting})");
    if devices.is_empty() {
        println!("  (none)");
        return;
    }
    for device in devices {
        println!(
            "  {} {}{}",
            if device.is_default { "*" } else { " " },
            device.name,
            device
                .default_config
                .map(|c| format!("  ({c})"))
                .unwrap_or_default()
        );
    }
}

/// VAD and translation are single files rather than model directories, so they
/// have no `engine.toml` and are reported on their own.
fn print_single_files(root: &Path) {
    let vad = paths::vad_model_file(root);
    println!();
    println!("VAD      ({})", vad.display());
    println!(
        "  [{}] silero_vad.onnx",
        if vad.is_file() { '+' } else { '!' }
    );

    let mt_root = paths::mt_dir(root);
    println!();
    println!("Translation  ({})", mt_root.display());
    match std::fs::read_dir(&mt_root) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
                })
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            if names.is_empty() {
                println!("  (no .gguf files)");
            } else {
                for name in names {
                    println!("  [+] {name}");
                }
            }
        }
        Err(e) => println!("  cannot read {}: {e}", mt_root.display()),
    }
}

/// Say plainly whether the configured engine exists and is usable. Never
/// substitute a different one (SPEC §15).
fn report_selection(config: &Config, asr: &[Entry]) {
    let selected = config.asr.engine.trim();
    if selected.is_empty() {
        println!("  [asr].engine is unset; nothing selected");
        return;
    }
    match asr.iter().find(|e| e.dir_name() == selected) {
        Some(Entry::Loaded(engine)) if engine.enabled() => {
            println!("  [asr].engine = \"{selected}\" -> {}", engine.name);
        }
        Some(Entry::Loaded(engine)) => {
            println!(
                "  [asr].engine = \"{selected}\" is DISABLED - missing: {}",
                engine.missing_files().join(", ")
            );
        }
        Some(Entry::Failed { error, .. }) => {
            println!("  [asr].engine = \"{selected}\" failed to load: {error}");
        }
        None => {
            println!("  [asr].engine = \"{selected}\" was not found among the discovered engines");
        }
    }
}

/// The console plus a rolling file in `<root>/logs` (SPEC §4). The console is
/// stdout, or stderr when `to_stderr` is set: for commands whose stdout is data
/// for another program (--translate). If the log directory cannot be created,
/// `Volis` still runs and still logs to the console - losing the file is not
/// worth refusing to start over.
fn init_logging(
    root: &Path,
    to_stderr: bool,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stdout_layer = if to_stderr {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr as fn() -> std::io::Stderr)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout as fn() -> std::io::Stdout)
            .boxed()
    };

    let logs = paths::logs_dir(root);
    match std::fs::create_dir_all(&logs) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&logs, "volis.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false);
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .init();
            warn!("cannot create log directory {}: {e}", logs.display());
            None
        }
    }
}
