//! `convers` — offline speech-to-speech translation.
//!
//! Milestones so far: model discovery (M0), and microphone capture through
//! voice activity detection (M1). Nothing transcribes, translates or speaks
//! yet.

mod audio;
mod cli;
mod config;
mod listen;
mod models;
mod paths;
mod report;
mod vad;
mod wav;

use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::models::{Entry, Role};

fn main() -> Result<()> {
    let command = cli::parse(std::env::args().skip(1))?;
    if command == cli::Command::Help {
        print!("{}", cli::HELP);
        return Ok(());
    }

    let root = paths::app_root().context("failed to locate the application directory")?;

    // Logging first, so everything after it is on the record. Held for the
    // lifetime of main: dropping the guard would stop the file writer.
    let _log_guard = init_logging(&root);

    println!(
        "convers {} — offline, no network required",
        env!("CARGO_PKG_VERSION")
    );
    println!("app root: {}", root.display());

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

    if let cli::Command::Listen { seconds, write_wav } = command {
        return listen::run(&root, &config, seconds, write_wav);
    }

    let models_root = paths::models_dir(&root);
    if !models_root.is_dir() {
        anyhow::bail!(
            "model directory not found: {}\n\
             convers never downloads models. Create that directory and place the model \
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

/// Input devices, so the user can put an exact name in `[audio].input_device`.
fn print_devices() -> Result<()> {
    let devices = audio::list_input_devices()?;
    println!();
    println!("Audio input devices");
    if devices.is_empty() {
        println!("  (none)");
        return Ok(());
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
    println!();
    println!("  * = system default. Put a name in [audio].input_device to pin one.");
    Ok(())
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

/// stdout plus a rolling file in `<root>/logs` (SPEC §4). If the log directory
/// cannot be created, `convers` still runs and still logs to stdout — losing
/// the file is not worth refusing to start over.
fn init_logging(root: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    let logs = paths::logs_dir(root);
    match std::fs::create_dir_all(&logs) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&logs, "convers.log");
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
