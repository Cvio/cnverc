//! The M0 report: a table of everything discovery found (SPEC §13, M0).
//!
//! Printed to stdout, not through `tracing`, because it is the program's
//! output rather than a log of its behaviour. Every complaint about a file
//! names the absolute path that was tried (SPEC §14).

use std::path::Path;

use crate::models::{Entry, Role};

/// Column widths chosen so the common case fits an 80-column console without
/// wrapping; longer values push the row wider rather than being truncated.
const NAME_W: usize = 34;
const KIND_W: usize = 7;
const BACKEND_W: usize = 15;

pub fn print_table(title: &str, root: &Path, entries: &[Entry]) {
    println!();
    println!("{title}  ({})", root.display());

    if entries.is_empty() {
        println!("  (none)");
        return;
    }

    println!(
        "  {:<NAME_W$}  {:<KIND_W$}  {:<BACKEND_W$}  STATUS",
        "DIRECTORY", "KIND", "BACKEND"
    );

    for entry in entries {
        match entry {
            Entry::Loaded(engine) => {
                let missing = engine.missing_files();
                let status = if missing.is_empty() {
                    "ok".to_string()
                } else {
                    format!("DISABLED - missing: {}", missing.join(", "))
                };
                println!(
                    "  {:<NAME_W$}  {:<KIND_W$}  {:<BACKEND_W$}  {}",
                    engine.dir_name,
                    engine.kind.to_string(),
                    engine.backend.to_string(),
                    status
                );
                println!("  {:<NAME_W$}  {}", "", engine.name);
                println!("  {:<NAME_W$}  {}", "", engine.dir.display());
                if !engine.languages.is_empty() {
                    println!(
                        "  {:<NAME_W$}  languages: {}",
                        "",
                        engine.languages.join(", ")
                    );
                }
                for file in &engine.files {
                    println!(
                        "  {:<NAME_W$}  [{}] {} {}",
                        "",
                        if file.present { '+' } else { '!' },
                        file.role,
                        file.path.display()
                    );
                }
            }
            Entry::Failed { dir_name, error } => {
                println!(
                    "  {:<NAME_W$}  {:<KIND_W$}  {:<BACKEND_W$}  ERROR",
                    dir_name, "-", "-"
                );
                println!("  {:<NAME_W$}  {error}", "");
            }
        }
    }
}

/// One line per model root, summarising what the selector will offer.
pub fn print_summary(role: Role, entries: &[Entry]) {
    let total = entries.len();
    let usable = entries
        .iter()
        .filter(|e| matches!(e, Entry::Loaded(engine) if engine.enabled()))
        .count();
    let label = match role {
        Role::Asr => "ASR",
        Role::Tts => "TTS",
    };
    println!("  {label}: {usable} of {total} entries usable");
}
