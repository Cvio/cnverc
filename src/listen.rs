//! `--listen`: the pipeline with no window, for shells and for logs.
//!
//! The pipeline logs everything worth reading itself, so this front end only
//! has to start it, stop it when time is up, and make sure an error reaches
//! the terminal and the exit code rather than only the log.

use std::path::Path;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tracing::info;

use crate::config::{Config, ModeKind};
use crate::pipeline::{Options, Pipeline, PipelineMsg};

/// How often to check whether the time is up.
const POLL: Duration = Duration::from_millis(100);

pub fn run(root: &Path, config: &Config, seconds: Option<u64>, options: Options) -> Result<()> {
    // A terminal has no turn key, so --listen always listens continuously.
    // Taking turns, alone or on a shared machine, needs the window (SPEC §8).
    let mut config = config.clone();
    if config.mode.kind != ModeKind::Continuous {
        info!("--listen listens continuously; turn-taking is in the window");
        config.mode.kind = ModeKind::Continuous;
    }
    // Paired mode is in the window too: pairing needs a Connect button and a
    // turn key, and a terminal has neither.
    if config.peer.enabled {
        info!("--listen does not pair; paired mode is in the window");
        config.peer.enabled = false;
    }
    let config = &config;

    let (tx, rx) = channel();
    let pipeline = Pipeline::start(root.to_path_buf(), config.clone(), options, tx)?;

    let mut deadline = None;
    let mut errors = Vec::new();
    let mut heard = 0usize;

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            pipeline.stop();
        }
        match rx.recv_timeout(POLL) {
            // The clock starts once the device is open, not while models load,
            // so --seconds means seconds of listening.
            Ok(PipelineMsg::Listening) => {
                if let Some(n) = seconds {
                    info!("listening for {n} s");
                    deadline = Some(Instant::now() + Duration::from_secs(n));
                } else {
                    info!("Ctrl-C to stop");
                }
            }
            Ok(PipelineMsg::Final { .. }) => heard += 1,
            Ok(PipelineMsg::Error(e)) => errors.push(e),
            Ok(PipelineMsg::Stopped) => break,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    if heard == 0 && errors.is_empty() {
        info!(
            "no speech recognised. If that is wrong, check --devices, check the level reports \
             above, and consider lowering [vad].threshold from {}.",
            config.vad.threshold
        );
    }
    // A pipeline that failed to start is a failed run, and the exit code
    // should say so. Errors about single utterances are already in the log.
    match errors.first() {
        Some(first) if heard == 0 => Err(anyhow!("{first}")),
        _ => Ok(()),
    }
}
