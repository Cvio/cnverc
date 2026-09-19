//! The pipeline: microphone → VAD → ASR → translation → TTS → speakers.
//!
//! Runs on its own threads and reports what it does as [`PipelineMsg`]s on a
//! channel. That channel is the only thing a front end sees (SPEC §11): the
//! command line logs the messages and waits, the GUI draws them.
//!
//! Threads, per SPEC §11:
//!
//! * capture — the cpal callback, in `audio.rs`; never blocks.
//! * pipeline — owns the VAD and the recognizer. Models load here too, so a
//!   front end stays responsive while they do.
//! * translate — translation and synthesis, so a slow token stream cannot
//!   stall recognition.
//! * playback — the output stream and the half-duplex gate, in `playback.rs`.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError,
};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info, warn};

use crate::asr::{self, AsrEngine, SegmentAsr};
use crate::audio::{self, SAMPLE_RATE};
use crate::compare::{self, Comparison};
use crate::config::{Config, ModeKind};
use crate::models::{self, Engine, Entry, Role};
use crate::paths;
use crate::playback::{Gate, PlaybackControl, Player};
use crate::ring::{Utterance, UtteranceRing, DEFAULT_CAPACITY};
use crate::translate::{LlamaTranslator, Translator};
use crate::tts::{self, Voice};
use crate::vad::{Segment, Segmenter, VadSettings};
use crate::wav;

/// Chunks of 16 kHz audio queued between capture and the VAD.
const PIPELINE_QUEUE_CHUNKS: usize = 64;

/// How often the pipeline loop wakes to check whether it should stop, while
/// the microphone is closed and there is nothing but commands to wait for.
const POLL: Duration = Duration::from_millis(100);

/// How long to wait for audio before looking at commands again, while the
/// microphone is open. Short, so ending a turn takes effect at once.
const AUDIO_POLL: Duration = Duration::from_millis(20);

/// How often the input level goes to the log while nothing is being said.
/// Without it, a muted microphone and a silent room look identical.
const LEVEL_LOG: Duration = Duration::from_secs(5);

/// How often the input level goes to the front end, for a live meter.
const LEVEL_EVENT: Duration = Duration::from_millis(200);

/// Transcripts queued for translation. Short on purpose: if the translator
/// falls this far behind, the conversation has already moved on and saying so
/// is better than growing a backlog.
const TRANSLATION_QUEUE: usize = 4;

/// Everything a front end hears from the pipeline.
///
/// The names follow SPEC §11. Variants §11 lists for later milestones — turns,
/// partials, remote utterances, the floor, peer state — arrive with those
/// milestones, and so do §11's `at` timestamps and the `pcm` on `Final`: no
/// consumer reads them yet, and a field nothing reads is only a place for a
/// bug to hide. The additions here are what a front end needs that §11 left
/// implicit: loading progress, a level meter, the per-stage timings behind the
/// latency readout, and the comparison harness's results (SPEC §12).
#[derive(Debug, Clone)]
pub enum PipelineMsg {
    /// Models are loading. Carries what is being loaded, for a status line.
    Loading(String),
    /// Loaded and ready. Whether the microphone is open depends on the mode.
    Listening,
    /// Listening continuously, or waiting for turns (SPEC §8). Sent once the
    /// models are loaded, and again whenever the mode changes.
    Mode(ModeKind),
    /// A turn has begun: the microphone is open.
    TurnStarted,
    /// The turn has ended: the microphone is closed, and what was said is on
    /// its way through recognition and translation.
    TurnEnded,
    /// Peak input level over the last fraction of a second, in dBFS.
    Level(f32),
    /// The VAD has started hearing speech.
    SpeechStarted,
    /// A complete utterance and what the recognizer made of it.
    Final {
        index: usize,
        text: String,
        /// Length of the audio. Always travels with `asr_ms`: Whisper pads to
        /// 30 s internally, and a timing without its duration misleads.
        speech_ms: u64,
        asr_ms: u128,
    },
    /// A translation, keyed to the utterance it came from.
    Translated {
        index: usize,
        target: String,
        translate_ms: u128,
    },
    /// Speech the VAD cut, in which the recognizer found no words. Usually a
    /// cough or a chair; sometimes two seconds of real speech the recognizer
    /// failed on. Shown either way, because the second must never pass
    /// unnoticed.
    NothingRecognized { index: usize, speech_ms: u64 },
    /// An utterance that was recognised but will not be translated or spoken,
    /// and why. Shown rather than silently dropped.
    NotTranslated { index: usize, reason: String },
    /// Speech reached the sound card.
    SpeakingStarted {
        index: usize,
        /// From the moment the utterance was cut to the first sample playing.
        first_audio_ms: u128,
    },
    /// The last sample has played; the microphone reopens after the tail.
    SpeakingEnded,
    /// Every enabled engine's transcript of one utterance (SPEC §12).
    Comparison(Comparison),
    /// Something went wrong. Always shown, never only logged.
    Error(String),
    /// The pipeline has stopped and released every device.
    Stopped,
}

/// What a caller can ask of a run beyond `cnverc.toml`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Write each utterance to `logs/segments/` as a WAV.
    pub write_wav: bool,
    /// Run every enabled segment engine on each utterance (SPEC §12).
    pub compare: bool,
}

/// What a front end can ask of a running pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCmd {
    /// Switch between continuous and turn-based listening without a restart
    /// (SPEC §8).
    SetMode(ModeKind),
    /// Open the microphone for a turn.
    BeginTurn,
    /// Close it, and treat everything said in the turn as one utterance.
    EndTurn,
}

/// A running pipeline. Dropping it stops it.
pub struct Pipeline {
    stop: Arc<AtomicBool>,
    commands: Sender<PipelineCmd>,
    thread: Option<JoinHandle<()>>,
}

impl Pipeline {
    /// Start the pipeline on its own thread. Returns at once; progress and
    /// failure both arrive as messages, ending with [`PipelineMsg::Stopped`].
    pub fn start(
        root: PathBuf,
        config: Config,
        options: Options,
        events: Sender<PipelineMsg>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (commands, command_rx) = channel();
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("cnverc-pipeline".to_string())
                .spawn(move || {
                    if let Err(e) = run(&root, &config, &options, &stop, &command_rx, &events) {
                        warn!("{e:#}");
                        let _ = events.send(PipelineMsg::Error(format!("{e:#}")));
                    }
                    let _ = events.send(PipelineMsg::Stopped);
                })
                .context("cannot spawn the pipeline thread")?
        };
        Ok(Self {
            stop,
            commands,
            thread: Some(thread),
        })
    }

    /// Ask something of the running pipeline. A pipeline that has already
    /// stopped ignores it.
    pub fn send(&self, command: PipelineCmd) {
        let _ = self.commands.send(command);
    }

    /// Ask the pipeline to stop. It flushes what it has, releases the devices
    /// and sends [`PipelineMsg::Stopped`].
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    root: &Path,
    config: &Config,
    options: &Options,
    stop: &AtomicBool,
    commands: &Receiver<PipelineCmd>,
    events: &Sender<PipelineMsg>,
) -> Result<()> {
    let engines = discovered_engines(root);
    let language = config.languages.source.clone();
    let selected_name = config.asr.engine.trim().to_string();

    // Resolve the selection before loading anything: a missing or
    // half-extracted model should fail before the device lights up.
    check_selection(&engines, &selected_name)?;

    // In comparison mode every engine runs on every utterance, the configured
    // one included, so loading it separately would hold a second copy of the
    // same model in memory for nothing.
    let mut asr = if options.compare {
        let _ = events.send(PipelineMsg::Loading(
            "every recognizer, for comparison".to_string(),
        ));
        let loaded = asr::load_all_segment_engines(&engines);
        if loaded.len() < 2 {
            let message = format!(
                "comparison needs at least two usable recognizers; {} found",
                loaded.len()
            );
            warn!("{message}");
            let _ = events.send(PipelineMsg::Error(message));
        }
        Recognizers {
            selected: None,
            comparison: loaded,
        }
    } else {
        if !selected_name.is_empty() {
            let _ = events.send(PipelineMsg::Loading(format!(
                "recognizer \"{selected_name}\""
            )));
        }
        Recognizers {
            selected: load_selected(&engines, &selected_name)?,
            comparison: Vec::new(),
        }
    };

    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }

    // The gate exists whether or not TTS does, so the capture loop has one
    // thing to ask rather than two.
    let gate = Arc::new(Gate::new(config.tts.half_duplex));

    let speaking = if config.tts.enabled && !options.compare && asr.selected.is_some() {
        let _ = events.send(PipelineMsg::Loading(format!(
            "a voice for \"{}\"",
            config.languages.target
        )));
        let voices = discovered_voices(root);
        let engine = tts::for_language(&voices, &config.languages.target)?;
        let voice = Voice::load(engine)?;
        let player = Player::open(
            &config.audio.output_device,
            gate.clone(),
            Some(events.clone()),
        )?;
        if !config.tts.half_duplex {
            warn!(
                "[tts].half_duplex is off: cnverc will hear its own speech and transcribe it \
                 unless you are wearing headphones (SPEC §10)"
            );
        }
        Some((voice, player))
    } else {
        None
    };
    // Taking a turn stops and holds playback; the pipeline keeps the handle
    // for that, and the player itself goes to the thread that speaks.
    let playback = speaking.as_ref().map(|(_, player)| player.control());

    // No translator in comparison mode: with several transcripts of the same
    // utterance there is no single one to translate, and comparison is a
    // recognizer harness (SPEC §12), not the conversation path.
    let translation = if options.compare || asr.selected.is_none() {
        None
    } else {
        let _ = events.send(PipelineMsg::Loading("the translation model".to_string()));
        let model = models::find_translation_model(&paths::mt_dir(root))?;
        Some(spawn_translator(
            &model,
            config.languages.source.clone(),
            config.languages.target.clone(),
            speaking,
            events.clone(),
        )?)
    };
    let translate_tx = translation.as_ref().map(|(tx, _)| tx.clone());

    let settings = VadSettings {
        model: paths::vad_model_file(root),
        threshold: config.vad.threshold,
        min_silence_ms: config.vad.min_silence_ms,
        min_speech_ms: config.vad.min_speech_ms,
    };
    let mut segmenter = Segmenter::new(&settings)?;
    info!(
        "VAD ready: threshold {}, min silence {} ms, min speech {} ms",
        settings.threshold, settings.min_silence_ms, settings.min_speech_ms
    );

    let segments_dir = paths::logs_dir(root).join("segments");
    if options.write_wav {
        std::fs::create_dir_all(&segments_dir)
            .with_context(|| format!("cannot create {}", segments_dir.display()))?;
        info!("writing utterances to {}", segments_dir.display());
    }

    let mut ring = UtteranceRing::new(DEFAULT_CAPACITY);

    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }

    let stage = Stage {
        selected_name: &selected_name,
        language: &language,
        translate_tx: translate_tx.as_ref(),
        segments_dir: options.write_wav.then_some(&segments_dir),
        events,
    };
    let device = config.audio.input_device.as_str();

    // Continuous mode keeps the microphone open. Turn mode keeps the device
    // closed until a turn is taken and closes it again when the turn ends, so
    // between turns the microphone is released, not merely ignored (SPEC §8).
    let mut mode = config.mode.kind;
    let mut mic: Option<Mic> = None;
    let mut turn: Option<Turn> = None;
    if mode == ModeKind::Continuous {
        mic = Some(Mic::open(device)?);
    }
    info!("ready: {}", describe_mode(mode));
    let _ = events.send(PipelineMsg::Listening);
    let _ = events.send(PipelineMsg::Mode(mode));

    let started = Instant::now();
    let mut log_level = LevelMeter::new(LEVEL_LOG);
    let mut event_level = LevelMeter::new(LEVEL_EVENT);
    let mut was_gated = false;
    let mut hearing_speech = false;
    let mut dropped: u64 = 0;
    // Every sample captured, fed or gated, so the detector can be told where
    // the capture is when it is reset.
    let mut captured: u64 = 0;

    while !stop.load(Ordering::Relaxed) {
        // Commands first. With the microphone closed there is no audio to
        // wait on, so wait on commands instead; a turn then opens the moment
        // it is asked for.
        let first = if mic.is_none() {
            commands.recv_timeout(POLL).ok()
        } else {
            commands.try_recv().ok()
        };
        let pending: Vec<PipelineCmd> = first
            .into_iter()
            .chain(std::iter::from_fn(|| commands.try_recv().ok()))
            .collect();

        for command in pending {
            match command {
                PipelineCmd::SetMode(next) if next != mode => {
                    // Finish whatever the old mode had in progress first.
                    if let Some(active) = turn.take() {
                        dropped += finish_turn(
                            active,
                            &mut mic,
                            &mut segmenter,
                            playback.as_ref(),
                            &stage,
                            &mut ring,
                            &mut asr,
                        );
                    } else {
                        for segment in segmenter.flush() {
                            stage.handle(segment, None, &mut ring, &mut asr);
                        }
                    }
                    if let Some(open) = mic.take() {
                        dropped += open.close();
                    }
                    mode = next;
                    segmenter.reset(captured);
                    hearing_speech = false;
                    if mode == ModeKind::Continuous {
                        mic = Some(Mic::open(device)?);
                    }
                    info!("now {}", describe_mode(mode));
                    let _ = events.send(PipelineMsg::Mode(mode));
                }
                PipelineCmd::BeginTurn if mode == ModeKind::Turn && turn.is_none() => {
                    if let Some(playback) = &playback {
                        playback.begin_turn();
                    }
                    match Mic::open(device) {
                        Ok(open) => {
                            segmenter.reset(captured);
                            hearing_speech = false;
                            mic = Some(open);
                            turn = Some(Turn {
                                origin: captured,
                                audio: Vec::new(),
                                segments: Vec::new(),
                            });
                            info!("turn started: microphone open");
                            let _ = events.send(PipelineMsg::TurnStarted);
                        }
                        // A device that will not open fails this turn, not the
                        // session: the user can plug it back in and try again.
                        Err(e) => {
                            if let Some(playback) = &playback {
                                playback.end_turn();
                            }
                            warn!("cannot start a turn: {e:#}");
                            let _ = events
                                .send(PipelineMsg::Error(format!("cannot start a turn: {e:#}")));
                        }
                    }
                }
                PipelineCmd::EndTurn => {
                    if let Some(active) = turn.take() {
                        dropped += finish_turn(
                            active,
                            &mut mic,
                            &mut segmenter,
                            playback.as_ref(),
                            &stage,
                            &mut ring,
                            &mut asr,
                        );
                    }
                }
                // Already in that mode, or a turn asked for outside turn mode
                // or while one is already open: nothing to do.
                PipelineCmd::SetMode(_) | PipelineCmd::BeginTurn => {}
            }
        }

        let Some(open) = mic.as_ref() else {
            continue;
        };

        match open.rx.recv_timeout(AUDIO_POLL) {
            Ok(chunk) => {
                // Half-duplex (SPEC §10): while our own speech is playing,
                // captured audio is discarded and the detector is held reset,
                // so nothing of it can survive into the next utterance.
                if gate.is_closed() {
                    if !was_gated {
                        debug!("microphone gated while speaking");
                        was_gated = true;
                    }
                    // A turn holds playback, so this should not happen during
                    // one; if it does, silence keeps the turn's audio aligned
                    // with the capture timeline its segments are stamped on.
                    if let Some(active) = turn.as_mut() {
                        active.audio.resize(active.audio.len() + chunk.len(), 0.0);
                    }
                    captured += chunk.len() as u64;
                    segmenter.reset(captured);
                    hearing_speech = false;
                    continue;
                }
                if was_gated {
                    was_gated = false;
                    debug!("microphone live again");
                }

                log_level.observe(&chunk);
                event_level.observe(&chunk);
                let segments = segmenter.push(&chunk);
                match turn.as_mut() {
                    // In a turn the user decides where the utterance ends; the
                    // detector's segments only mark where the speech is.
                    Some(active) => {
                        active.audio.extend_from_slice(&chunk);
                        active.segments.extend(segments);
                    }
                    None => {
                        for segment in segments {
                            stage.handle(segment, None, &mut ring, &mut asr);
                        }
                    }
                }
                captured += chunk.len() as u64;

                // A segment engine emits SpeechStarted, then one Final
                // (SPEC §11). The rising edge is what matters.
                let in_speech = segmenter.speech_in_progress();
                if in_speech && !hearing_speech {
                    let _ = events.send(PipelineMsg::SpeechStarted);
                }
                hearing_speech = in_speech;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("the audio input stopped unexpectedly"));
            }
        }

        if let Some(peak) = log_level.take_if_due() {
            match peak {
                None => warn!(
                    "input level: digital silence over the last {} s — is the microphone muted?",
                    LEVEL_LOG.as_secs()
                ),
                Some(dbfs) => info!(
                    "input level: peak {dbfs:.1} dBFS over the last {} s",
                    LEVEL_LOG.as_secs()
                ),
            }
        }
        if let Some(peak) = event_level.take_if_due() {
            let _ = events.send(PipelineMsg::Level(peak.unwrap_or(f32::NEG_INFINITY)));
        }
    }

    // Whatever was still being said when the stop came.
    if let Some(active) = turn.take() {
        dropped += finish_turn(
            active,
            &mut mic,
            &mut segmenter,
            playback.as_ref(),
            &stage,
            &mut ring,
            &mut asr,
        );
    } else {
        for segment in segmenter.flush() {
            stage.handle(segment, None, &mut ring, &mut asr);
        }
    }
    if let Some(open) = mic.take() {
        dropped += open.close();
    }

    // Close the queue and let the translator finish what it already has.
    drop(translate_tx);
    if let Some((tx, thread)) = translation {
        drop(tx);
        let _ = thread.join();
    }

    info!(
        "listened for {:.1} s, {} utterance(s) retained, dropped {dropped} chunk(s)",
        started.elapsed().as_secs_f32(),
        ring.len()
    );
    Ok(())
}

fn describe_mode(mode: ModeKind) -> &'static str {
    match mode {
        ModeKind::Continuous => "listening continuously",
        ModeKind::Turn => "waiting for a turn, microphone closed",
    }
}

/// An open microphone.
struct Mic {
    capture: audio::CaptureHandle,
    rx: Receiver<Vec<f32>>,
}

impl Mic {
    fn open(device: &str) -> Result<Self> {
        let (tx, rx) = sync_channel::<Vec<f32>>(PIPELINE_QUEUE_CHUNKS);
        let capture = audio::spawn_capture(device, tx)?;
        Ok(Self { capture, rx })
    }

    /// Release the device. Returns the chunks it had to drop.
    fn close(self) -> u64 {
        let dropped = self.capture.dropped_chunks();
        self.capture.stop();
        dropped
    }
}

/// A turn in progress: everything captured since it began, and where the
/// detector heard speech in it.
struct Turn {
    /// Capture position of the turn's first sample.
    origin: u64,
    audio: Vec<f32>,
    segments: Vec<Segment>,
}

/// The recognizers a run has loaded.
struct Recognizers {
    selected: Option<Box<dyn SegmentAsr + Send>>,
    comparison: Vec<(String, Box<dyn SegmentAsr + Send>)>,
}

/// The longest stretch handed to a recognizer at once. Whisper hears 30
/// seconds and no more; a long turn is split at its pauses into parts below
/// that, transcribed in order and joined.
const MAX_PART: usize = 25 * SAMPLE_RATE as usize;

/// End a turn: close the microphone, then treat everything said in it as one
/// utterance, trimmed of silence at either end. The user decided where it
/// begins and ends; the detector only finds where the speech is inside it
/// (SPEC §8). Returns the chunks the microphone dropped.
fn finish_turn(
    active: Turn,
    mic: &mut Option<Mic>,
    segmenter: &mut Segmenter,
    playback: Option<&PlaybackControl>,
    stage: &Stage,
    ring: &mut UtteranceRing,
    asr: &mut Recognizers,
) -> u64 {
    let dropped = mic.take().map(Mic::close).unwrap_or(0);
    info!("turn ended: microphone closed");
    let _ = stage.events.send(PipelineMsg::TurnEnded);
    // Anything that arrived for the speaker during the turn plays now.
    if let Some(playback) = playback {
        playback.end_turn();
    }

    let Turn {
        origin,
        audio,
        mut segments,
    } = active;
    segments.extend(segmenter.flush());
    let turn_ms = audio.len() as u64 * 1000 / SAMPLE_RATE as u64;

    match trim_and_split(origin, audio.len(), &segments, MAX_PART) {
        None => {
            info!("turn: {turn_ms} ms captured, no speech in it");
            let utterance = ring.push(origin * 1000 / SAMPLE_RATE as u64, audio);
            let _ = stage.events.send(PipelineMsg::NothingRecognized {
                index: utterance.index,
                speech_ms: turn_ms,
            });
        }
        Some((speech, parts)) => {
            info!(
                "turn: {turn_ms} ms captured, {} ms of speech in {} part(s)",
                (speech.end - speech.start) as u64 * 1000 / SAMPLE_RATE as u64,
                parts.len()
            );
            let segment = Segment {
                start_sample: origin + speech.start as u64,
                samples: audio[speech].to_vec(),
            };
            stage.handle(segment, Some(&parts), ring, asr);
        }
    }
    dropped
}

/// Where the speech is in a turn, and how to split it for the recognizer.
///
/// Returns the span from the first speech to the last, relative to the start
/// of the turn's audio, and that span divided into parts no longer than
/// `max_part`, split at the pauses between segments. Parts are relative to
/// the span. `None` when the detector heard no speech at all.
fn trim_and_split(
    origin: u64,
    len: usize,
    segments: &[Segment],
    max_part: usize,
) -> Option<(Range<usize>, Vec<Range<usize>>)> {
    let spans: Vec<Range<usize>> = segments
        .iter()
        .filter_map(|s| {
            let start = usize::try_from(s.start_sample.checked_sub(origin)?).ok()?;
            let end = (start + s.samples.len()).min(len);
            (start < end).then_some(start..end)
        })
        .collect();
    let first = spans.first()?.start;
    let last = spans.iter().map(|r| r.end).max()?;

    let mut parts = Vec::new();
    let mut current = spans[0].clone();
    for span in &spans[1..] {
        if span.end - current.start > max_part {
            parts.push(current);
            current = span.clone();
        } else {
            current.end = current.end.max(span.end);
        }
    }
    parts.push(current);

    let parts = parts
        .into_iter()
        .map(|r| (r.start - first)..(r.end - first))
        .collect();
    Some((first..last, parts))
}

/// Transcribe an utterance, part by part when it has been split.
fn transcribe_parts(
    asr: &mut Box<dyn SegmentAsr + Send>,
    pcm: &[f32],
    parts: Option<&[Range<usize>]>,
    language: &str,
) -> Result<String> {
    match parts {
        Some(parts) if parts.len() > 1 => {
            let mut texts = Vec::with_capacity(parts.len());
            for part in parts {
                let text = asr.transcribe(&pcm[part.clone()], language)?;
                let text = text.trim();
                if !text.is_empty() {
                    texts.push(text.to_string());
                }
            }
            Ok(texts.join(" "))
        }
        _ => asr.transcribe(pcm, language),
    }
}

/// One transcript on its way to the translator.
struct ToTranslate {
    index: usize,
    text: String,
    /// When the utterance was cut, so the speaking stage can report the whole
    /// end-of-speech to first-audio latency.
    captured_at: Instant,
}

/// The translation stage runs on its own thread (SPEC §11): a slow token
/// stream must never stall recognition. The model is loaded here, on the
/// caller's thread, so a missing or broken GGUF is an error from this call
/// rather than a thread that quietly dies later.
fn spawn_translator(
    model: &Path,
    source: String,
    target: String,
    speaking: Option<(Voice, Player)>,
    events: Sender<PipelineMsg>,
) -> Result<(SyncSender<ToTranslate>, JoinHandle<()>)> {
    let mut translator = LlamaTranslator::load(model)?;
    let (tx, rx) = sync_channel::<ToTranslate>(TRANSLATION_QUEUE);

    let thread = std::thread::Builder::new()
        .name("cnverc-translate".to_string())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                let began = Instant::now();
                let text = match translator.translate(&job.text, &source, &target) {
                    Ok(text) if text.is_empty() => {
                        warn!("utterance {}: translated to nothing", job.index);
                        let _ = events.send(PipelineMsg::NotTranslated {
                            index: job.index,
                            reason: "the translation came back empty".to_string(),
                        });
                        continue;
                    }
                    Ok(text) => text,
                    Err(e) => {
                        warn!("utterance {}: translation failed: {e:#}", job.index);
                        let _ = events.send(PipelineMsg::NotTranslated {
                            index: job.index,
                            reason: format!("{e:#}"),
                        });
                        continue;
                    }
                };
                let translate_ms = began.elapsed().as_millis();
                info!(
                    "utterance {}\n  [{source}] {}\n  [{target}] {text}\n  ({translate_ms} ms \
                     to translate)",
                    job.index, job.text
                );
                let _ = events.send(PipelineMsg::Translated {
                    index: job.index,
                    target: text.clone(),
                    translate_ms,
                });

                // Synthesis shares this thread with translation (SPEC §11), so
                // neither can stall recognition.
                let Some((voice, player)) = speaking.as_ref() else {
                    continue;
                };
                match voice.speak(&text) {
                    Ok(speech) if speech.samples.is_empty() => {
                        warn!("utterance {}: the voice produced no audio", job.index);
                        let _ = events.send(PipelineMsg::Error(format!(
                            "utterance {}: the voice produced no audio",
                            job.index
                        )));
                    }
                    Ok(speech) => {
                        let synthesised_ms = began.elapsed().as_millis() - translate_ms;
                        let speech_ms = speech.duration_ms();
                        match player.play(&speech.samples, speech.sample_rate) {
                            // Time to first audio runs from the moment the
                            // utterance was cut, through recognition,
                            // translation and synthesis, to the sound card.
                            Ok(()) => {
                                let first_audio_ms = job.captured_at.elapsed().as_millis();
                                info!(
                                    "  speaking {speech_ms} ms ({synthesised_ms} ms to \
                                     synthesise, {first_audio_ms} ms from end of speech to \
                                     first audio)"
                                );
                                let _ = events.send(PipelineMsg::SpeakingStarted {
                                    index: job.index,
                                    first_audio_ms,
                                });
                            }
                            Err(e) => {
                                warn!("utterance {}: cannot play: {e:#}", job.index);
                                let _ = events.send(PipelineMsg::Error(format!(
                                    "cannot play utterance {}: {e:#}",
                                    job.index
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        warn!("utterance {}: synthesis failed: {e:#}", job.index);
                        let _ = events.send(PipelineMsg::Error(format!(
                            "utterance {}: synthesis failed: {e:#}",
                            job.index
                        )));
                    }
                }
            }
            info!("translation stopped");
        })
        .context("cannot spawn the translation thread")?;

    Ok((tx, thread))
}

/// What handling one utterance needs that does not change between them.
struct Stage<'a> {
    selected_name: &'a str,
    language: &'a str,
    translate_tx: Option<&'a SyncSender<ToTranslate>>,
    segments_dir: Option<&'a PathBuf>,
    events: &'a Sender<PipelineMsg>,
}

impl Stage<'_> {
    /// Transcribe one utterance, keep it, and report it. `parts` splits a
    /// long turn for the recognizer; a detector segment is never long enough
    /// to need it.
    fn handle(
        &self,
        segment: Segment,
        parts: Option<&[Range<usize>]>,
        ring: &mut UtteranceRing,
        asr: &mut Recognizers,
    ) {
        let captured_at = Instant::now();
        let (start_ms, end_ms, duration_ms) =
            (segment.start_ms(), segment.end_ms(), segment.duration_ms());
        let utterance = ring.push(start_ms, segment.samples);

        info!(
            "utterance {}: {start_ms} ms .. {end_ms} ms ({duration_ms} ms)",
            utterance.index
        );

        if let Some(selected) = asr.selected.as_mut() {
            let began = Instant::now();
            match transcribe_parts(selected, &utterance.pcm, parts, self.language) {
                // No words: usually a cough, sometimes real speech the
                // recognizer failed on. Neither is sent to the translator, and
                // both are reported, because a failed utterance that vanishes
                // silently looks exactly like one that was never spoken.
                Ok(text) if text.trim().is_empty() => {
                    info!(
                        "  no words recognised in {} ms of audio ({} ms to decide)",
                        utterance.duration_ms(),
                        began.elapsed().as_millis()
                    );
                    let _ = self.events.send(PipelineMsg::NothingRecognized {
                        index: utterance.index,
                        speech_ms: utterance.duration_ms(),
                    });
                }
                // The segment duration travels with the timing, always:
                // Whisper pads to 30 s internally (SPEC §10).
                Ok(text) => {
                    let asr_ms = began.elapsed().as_millis();
                    info!(
                        "  [{}] {text}\n  ({asr_ms} ms to transcribe {} ms of audio)",
                        self.language,
                        utterance.duration_ms()
                    );
                    let _ = self.events.send(PipelineMsg::Final {
                        index: utterance.index,
                        text: text.clone(),
                        speech_ms: utterance.duration_ms(),
                        asr_ms,
                    });
                    self.send_to_translator(utterance.index, text, captured_at);
                }
                Err(e) => {
                    warn!("  transcription failed: {e:#}");
                    let _ = self.events.send(PipelineMsg::Error(format!(
                        "utterance {}: transcription failed: {e:#}",
                        utterance.index
                    )));
                }
            }
        }

        if !asr.comparison.is_empty() {
            let comparison = compare::run_all(&mut asr.comparison, &utterance, self.language);
            compare::report(&comparison, self.selected_name);
            let _ = self.events.send(PipelineMsg::Comparison(comparison));
        }

        write_segment(&utterance, self.segments_dir);
    }

    fn send_to_translator(&self, index: usize, text: String, captured_at: Instant) {
        let Some(tx) = self.translate_tx else {
            return;
        };
        // Never block the pipeline thread on the translator.
        match tx.try_send(ToTranslate {
            index,
            text,
            captured_at,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(job)) => {
                warn!(
                    "translation is behind; utterance {} not translated",
                    job.index
                );
                let _ = self.events.send(PipelineMsg::NotTranslated {
                    index: job.index,
                    reason: "translation fell behind the conversation".to_string(),
                });
            }
            Err(TrySendError::Disconnected(_)) => warn!("the translation thread has stopped"),
        }
    }
}

fn write_segment(utterance: &Utterance, segments_dir: Option<&PathBuf>) {
    let Some(dir) = segments_dir else {
        return;
    };
    // Named by its timestamp as well as its index: the ring is keyed by when
    // the utterance happened, and the files should match (SPEC §12).
    let path = dir.join(format!(
        "utterance-{:04}-at-{}ms-for-{}ms.wav",
        utterance.index,
        utterance.start_ms,
        utterance.duration_ms()
    ));
    match wav::write_16k_mono(&path, &utterance.pcm) {
        Ok(()) => info!("  wrote {}", path.display()),
        Err(e) => warn!("  {e:#}"),
    }
}

/// Every TTS directory that parsed, whether or not its files are all present.
pub fn discovered_voices(root: &Path) -> Vec<Engine> {
    models::discover(&paths::tts_dir(root), Role::Tts)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Loaded(engine) => Some(engine),
            Entry::Failed { .. } => None,
        })
        .collect()
}

/// Every ASR directory that parsed, whether or not its files are all present.
pub fn discovered_engines(root: &Path) -> Vec<Engine> {
    models::discover(&paths::asr_dir(root), Role::Asr)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Loaded(engine) => Some(engine),
            Entry::Failed { .. } => None,
        })
        .collect()
}

/// Resolve `[asr].engine` to a discovered model. Never substitutes another one
/// (SPEC §15).
fn find_selected<'a>(engines: &'a [Engine], selected: &str) -> Result<&'a Engine> {
    engines
        .iter()
        .find(|e| e.dir_name == selected)
        .ok_or_else(|| {
            anyhow!(
                "[asr].engine = \"{selected}\" was not found. Discovered: {}",
                if engines.is_empty() {
                    "nothing".to_string()
                } else {
                    engines
                        .iter()
                        .map(|e| e.dir_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )
        })
}

/// Fail on a selection that cannot work, without paying to load it first. An
/// unset selection is allowed: it listens without transcribing, which is still
/// the way to check a microphone.
fn check_selection(engines: &[Engine], selected: &str) -> Result<()> {
    if selected.is_empty() {
        warn!("[asr].engine is unset; listening without transcribing");
        return Ok(());
    }
    let engine = find_selected(engines, selected)?;
    let missing = engine.missing_files();
    if !missing.is_empty() {
        return Err(anyhow!(
            "[asr].engine = \"{selected}\" is incomplete; these files are not in {}: {}",
            engine.dir.display(),
            missing.join(", ")
        ));
    }
    Ok(())
}

/// Load the configured engine for ordinary (non-comparison) listening.
fn load_selected(engines: &[Engine], selected: &str) -> Result<Option<Box<dyn SegmentAsr + Send>>> {
    if selected.is_empty() {
        return Ok(None);
    }
    let engine = find_selected(engines, selected)?;

    let began = Instant::now();
    match asr::load(engine)? {
        AsrEngine::Segment(asr) => {
            info!(
                "loaded \"{}\" ({}) in {} ms",
                engine.dir_name,
                engine.name,
                began.elapsed().as_millis()
            );
            Ok(Some(asr))
        }
        AsrEngine::Stream(_) => Err(anyhow!(
            "\"{}\" is a streaming engine; streaming arrives in Milestone 8",
            engine.dir_name
        )),
    }
}

/// Peak input level over a window.
struct LevelMeter {
    peak: f32,
    every: Duration,
    since: Instant,
}

impl LevelMeter {
    fn new(every: Duration) -> Self {
        Self {
            peak: 0.0,
            every,
            since: Instant::now(),
        }
    }

    fn observe(&mut self, chunk: &[f32]) {
        for sample in chunk {
            self.peak = self.peak.max(sample.abs());
        }
    }

    /// When the window has elapsed: the peak in dBFS, or `None` for digital
    /// silence. Resets for the next window.
    fn take_if_due(&mut self) -> Option<Option<f32>> {
        if self.since.elapsed() < self.every {
            return None;
        }
        let peak = self.peak;
        self.peak = 0.0;
        self.since = Instant::now();
        Some((peak > 0.0).then(|| 20.0 * peak.log10()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A detector segment at `start` for `len` samples, on the capture timeline.
    fn segment_at(start: u64, len: usize) -> Segment {
        Segment {
            start_sample: start,
            samples: vec![0.0; len],
        }
    }

    #[test]
    fn a_turn_is_trimmed_to_its_speech() {
        // A turn starting at capture position 5000: silence, speech, a pause,
        // more speech, silence.
        let origin = 5_000;
        let segments = [
            segment_at(origin + 1_000, 2_000),
            segment_at(origin + 10_000, 2_000),
        ];
        let (speech, parts) =
            trim_and_split(origin, 20_000, &segments, MAX_PART).expect("speech was heard");

        assert_eq!(speech, 1_000..12_000, "silence at either end is trimmed");
        assert_eq!(
            parts,
            vec![0..11_000],
            "the pause inside is kept: the user decided the boundary, not the detector"
        );
    }

    #[test]
    fn a_long_turn_is_split_at_its_pauses() {
        let segments = [
            segment_at(1_000, 2_000),
            segment_at(4_000, 2_000),
            segment_at(10_000, 2_000),
        ];
        let (speech, parts) = trim_and_split(0, 20_000, &segments, 6_000).expect("speech");
        assert_eq!(speech, 1_000..12_000);
        // The first two fit together under the limit; the third starts a
        // second part. Parts are relative to the trimmed speech.
        assert_eq!(parts, vec![0..5_000, 9_000..11_000]);
        assert!(parts.iter().all(|p| p.end - p.start <= 6_000));
    }

    #[test]
    fn a_turn_with_no_speech_has_nothing_to_send() {
        assert!(trim_and_split(0, 20_000, &[], MAX_PART).is_none());
    }

    #[test]
    fn a_segment_running_past_the_turn_is_clipped_to_it() {
        let (speech, parts) =
            trim_and_split(0, 5_000, &[segment_at(3_000, 4_000)], MAX_PART).expect("speech");
        assert_eq!(speech, 3_000..5_000);
        assert_eq!(parts, vec![0..2_000]);
    }

    #[test]
    fn the_level_meter_reports_once_per_window_and_resets() {
        let mut meter = LevelMeter::new(Duration::from_millis(20));
        meter.observe(&[0.5, -0.1]);
        assert_eq!(meter.take_if_due(), None, "not due yet");

        std::thread::sleep(Duration::from_millis(30));
        let reported = meter.take_if_due().expect("due").expect("not silence");
        assert!((reported - 20.0 * 0.5f32.log10()).abs() < 1e-3);

        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            meter.take_if_due(),
            Some(None),
            "nothing observed since the last report is digital silence"
        );
    }

    /// A turn of several sentences, with pauses between them, is one utterance.
    /// In continuous mode the detector would cut it into several; in a turn
    /// the user decides where it ends, and the detector only trims the silence
    /// at either end (SPEC §8, Milestone 6's check).
    ///
    /// The sentences are separate recordings, joined with pauses between them.
    /// The turn is transcribed as cnverc does it, in one pass up to 25 s, and
    /// also one sentence at a time; the single pass must not lose what the
    /// sentence-by-sentence pass keeps.
    ///
    /// Give it different sentences. Given the same sentence three times,
    /// Whisper's single pass keeps only one, taking the repetition for a
    /// decoding loop, and this test fails. That quirk is accepted: people do
    /// not repeat a sentence word for word in a turn, and transcribing every
    /// sentence separately to guard against it measured 1.5 s slower on a
    /// three-sentence turn of real speech, for the same words.
    ///
    /// ```bash
    /// CNVERC_TEST_MODELS=/abs/path/models \
    /// CNVERC_TEST_TURN_WAVS="/abs/a.wav;/abs/b.wav;/abs/c.wav" \
    /// cargo test --release -- --ignored --nocapture multi_sentence
    /// ```
    #[test]
    #[ignore = "needs the models and Spanish recordings; see the doc comment"]
    fn a_multi_sentence_turn_is_one_utterance() {
        let models_root = std::env::var("CNVERC_TEST_MODELS").expect("CNVERC_TEST_MODELS");
        let wavs = std::env::var("CNVERC_TEST_TURN_WAVS").expect("CNVERC_TEST_TURN_WAVS");
        let root = Path::new(&models_root)
            .parent()
            .expect("parent")
            .to_path_buf();
        let sentences: Vec<Vec<f32>> = wavs
            .split(';')
            .map(|path| wav::read_16k_mono(Path::new(path)).expect("read a recording"))
            .collect();
        assert!(
            sentences.len() >= 2,
            "a multi-sentence turn needs two or more"
        );

        // 1.5 s of silence, the sentences with a second between each, and
        // 1.5 s of silence: a turn someone took and then read into.
        let silence = |seconds: f32| vec![0.0f32; (seconds * SAMPLE_RATE as f32) as usize];
        let mut audio = silence(1.5);
        for (i, sentence) in sentences.iter().enumerate() {
            if i > 0 {
                audio.extend(silence(1.0));
            }
            audio.extend_from_slice(sentence);
        }
        audio.extend(silence(1.5));

        let settings = VadSettings {
            model: paths::vad_model_file(&root),
            threshold: 0.5,
            min_silence_ms: 500,
            min_speech_ms: 250,
        };
        let mut segmenter = Segmenter::new(&settings).expect("load the VAD");
        let origin = 123_456; // wherever the capture had got to
        segmenter.reset(origin);
        let mut segments = Vec::new();
        for chunk in audio.chunks(700) {
            segments.extend(segmenter.push(chunk));
        }
        segments.extend(segmenter.flush());

        let (speech, parts) =
            trim_and_split(origin, audio.len(), &segments, MAX_PART).expect("speech");
        let lead = speech.start as f32 / SAMPLE_RATE as f32;
        let tail = (audio.len() - speech.end) as f32 / SAMPLE_RATE as f32;
        println!(
            "{:.1} s turn of {} sentences: speech {:.1} s, trimmed {lead:.1} s before and {tail:.1} s after, {} part(s)",
            audio.len() as f32 / SAMPLE_RATE as f32,
            sentences.len(),
            (speech.end - speech.start) as f32 / SAMPLE_RATE as f32,
            parts.len()
        );
        assert!(
            lead > 0.5 && tail > 0.5,
            "the silence at the ends was not trimmed"
        );

        let engines = discovered_engines(&root);
        let mut asr = load_selected(&engines, "whisper-large-v3-turbo")
            .expect("load Whisper")
            .expect("Whisper");
        let speech_audio = &audio[speech];

        let began = Instant::now();
        let one_pass =
            transcribe_parts(&mut asr, speech_audio, Some(&parts), "es").expect("transcribe");
        println!(
            "as configured ({} ms): {one_pass}",
            began.elapsed().as_millis()
        );

        let (_, per_sentence) = trim_and_split(origin, audio.len(), &segments, 1).expect("speech");
        let began = Instant::now();
        let split = transcribe_parts(&mut asr, speech_audio, Some(&per_sentence), "es")
            .expect("transcribe");
        println!(
            "one sentence at a time, {} parts ({} ms): {split}",
            per_sentence.len(),
            began.elapsed().as_millis()
        );
        assert!(
            one_pass.chars().count() * 10 >= split.chars().count() * 9,
            "the single pass lost words the sentence-by-sentence pass kept:
  {one_pass}
  {split}"
        );
    }

    /// The microphone is closed between turns, not merely ignored: speech
    /// played to it while idle is not heard at all. A virtual audio cable
    /// makes this exact and silent, as it did for the half-duplex gate.
    ///
    /// ```bash
    /// CNVERC_TEST_MODELS=/abs/path/models \
    /// CNVERC_TEST_WAV_ES=/abs/path/spanish-16k.wav \
    /// CNVERC_TEST_OUT_DEVICE="CABLE Input (VB-Audio Virtual Cable)" \
    /// CNVERC_TEST_IN_DEVICE="CABLE Output (VB-Audio Virtual Cable)" \
    /// cargo test --release -- --ignored --nocapture closed_between_turns
    /// ```
    #[test]
    #[ignore = "needs a loopback audio device, the models and a recording"]
    fn the_microphone_is_closed_between_turns() {
        use crate::config::{Config, ModeKind};
        use crate::playback::Gate;
        use std::sync::mpsc::channel;

        let models_root = std::env::var("CNVERC_TEST_MODELS").expect("CNVERC_TEST_MODELS");
        let wav_path = std::env::var("CNVERC_TEST_WAV_ES").expect("CNVERC_TEST_WAV_ES");
        let out_device = std::env::var("CNVERC_TEST_OUT_DEVICE").expect("CNVERC_TEST_OUT_DEVICE");
        let in_device = std::env::var("CNVERC_TEST_IN_DEVICE").expect("CNVERC_TEST_IN_DEVICE");
        let root = Path::new(&models_root)
            .parent()
            .expect("parent")
            .to_path_buf();
        let sentence = wav::read_16k_mono(Path::new(&wav_path)).expect("read the recording");
        let sentence_time =
            Duration::from_millis(sentence.len() as u64 * 1000 / SAMPLE_RATE as u64 + 1500);

        let mut config = Config::default();
        config.asr.engine = "parakeet-tdt-0.6b-v3-int8".to_string();
        config.languages.source = "es".to_string();
        config.languages.target = "en".to_string();
        config.audio.input_device = in_device;
        config.mode.kind = ModeKind::Turn;
        // Nothing may be spoken into the cable but the test's own sentence.
        config.tts.enabled = false;

        let (tx, rx) = channel();
        let pipeline =
            Pipeline::start(root, config, Options::default(), tx).expect("start the pipeline");
        let wait_for = |want: &dyn Fn(&PipelineMsg) -> bool, within: Duration| {
            let deadline = Instant::now() + within;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(msg) if want(&msg) => return Some(msg),
                    Ok(PipelineMsg::Error(e)) => panic!("the pipeline failed: {e}"),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
            None
        };
        wait_for(
            &|m| matches!(m, PipelineMsg::Mode(ModeKind::Turn)),
            Duration::from_secs(60),
        )
        .expect("the pipeline never became ready in turn mode");

        // The "speaker": the test's own player, writing into the cable.
        let speaker = Player::open(&out_device, Arc::new(Gate::new(false)), None)
            .expect("open the cable's input");

        // 1. Idle: speak to it, and nothing at all may happen.
        speaker.play(&sentence, SAMPLE_RATE).expect("play");
        let heard_while_idle = wait_for(
            &|m| {
                matches!(
                    m,
                    PipelineMsg::Level(_)
                        | PipelineMsg::SpeechStarted
                        | PipelineMsg::Final { .. }
                        | PipelineMsg::NothingRecognized { .. }
                        | PipelineMsg::TurnStarted
                )
            },
            sentence_time,
        );
        assert!(
            heard_while_idle.is_none(),
            "the microphone heard something between turns: {heard_while_idle:?}"
        );
        println!(
            "idle: {} ms of speech played, nothing heard",
            sentence_time.as_millis()
        );

        // 2. A turn: the same sentence is heard, whole.
        pipeline.send(PipelineCmd::BeginTurn);
        wait_for(
            &|m| matches!(m, PipelineMsg::TurnStarted),
            Duration::from_secs(5),
        )
        .expect("the turn never started");
        speaker.play(&sentence, SAMPLE_RATE).expect("play");
        std::thread::sleep(sentence_time);
        pipeline.send(PipelineCmd::EndTurn);

        let final_msg = wait_for(
            &|m| matches!(m, PipelineMsg::Final { .. }),
            Duration::from_secs(30),
        )
        .expect("the turn produced no transcript");
        let PipelineMsg::Final { text, .. } = final_msg else {
            unreachable!()
        };
        println!("turn: {text}");
        assert!(
            text.contains("país"),
            "the turn was not heard properly: {text}"
        );
    }

    /// The whole pipeline on a recording: VAD, recognition, translation and
    /// synthesis, with the latency Milestone 4 asks to be measured. No audio
    /// device is involved, so this runs anywhere the models are installed.
    ///
    /// ```bash
    /// CNVERC_TEST_MODELS=/abs/path/models \
    /// CNVERC_TEST_WAV_ES=/abs/path/spanish-16k.wav \
    /// cargo test --release -- --ignored --nocapture end_to_end
    /// ```
    #[test]
    #[ignore = "needs every model and a Spanish recording; see the doc comment"]
    fn end_to_end_on_a_recording() {
        let models_root = std::env::var("CNVERC_TEST_MODELS").expect("CNVERC_TEST_MODELS");
        let wav_path = std::env::var("CNVERC_TEST_WAV_ES").expect("CNVERC_TEST_WAV_ES");
        let root = Path::new(&models_root)
            .parent()
            .expect("models/ has a parent")
            .to_path_buf();

        let audio = wav::read_16k_mono(Path::new(&wav_path)).expect("read the recording");

        let engines = discovered_engines(&root);
        let mut asr = load_selected(&engines, "parakeet-tdt-0.6b-v3-int8")
            .expect("load the recognizer")
            .expect("a recognizer");
        let model = models::find_translation_model(&paths::mt_dir(&root)).expect("a GGUF");
        let mut translator = LlamaTranslator::load(&model).expect("load the translator");
        let voices = discovered_voices(&root);
        let voice = Voice::load(tts::for_language(&voices, "en").expect("an English voice"))
            .expect("load the voice");

        let settings = VadSettings {
            model: paths::vad_model_file(&root),
            threshold: 0.5,
            min_silence_ms: 500,
            min_speech_ms: 250,
        };
        let mut segmenter = Segmenter::new(&settings).expect("load the VAD");

        let mut segments = Vec::new();
        for chunk in audio.chunks(1024) {
            segments.extend(segmenter.push(chunk));
        }
        segments.extend(segmenter.flush());
        assert!(!segments.is_empty(), "no speech in the recording");

        for segment in segments.iter().take(3) {
            // The clock starts where it starts in the real pipeline: the
            // moment the utterance has been cut.
            let began = Instant::now();

            let spanish = asr.transcribe(&segment.samples, "es").expect("transcribe");
            let after_asr = began.elapsed().as_millis();

            // An echo is refused by the translator; report it rather than
            // failing the whole measurement.
            let english = match translator.translate(&spanish, "es", "en") {
                Ok(text) => text,
                Err(e) => {
                    println!("  [es] {spanish}\n  not translated: {e:#}");
                    continue;
                }
            };
            let after_mt = began.elapsed().as_millis();

            let speech = voice.speak(&english).expect("synthesise");
            let to_first_audio = began.elapsed().as_millis();

            println!(
                "{} ms of speech\n  [es] {spanish}\n  [en] {english}\n  \
                 asr {after_asr} ms, +translate {} ms, +synthesise {} ms \
                 = {to_first_audio} ms to first audio for {} ms of speech",
                segment.duration_ms(),
                after_mt - after_asr,
                to_first_audio - after_mt,
                speech.duration_ms()
            );
            assert!(!speech.samples.is_empty(), "no audio");
        }
    }
}
