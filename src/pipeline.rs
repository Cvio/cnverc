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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info, warn};

use crate::asr::{self, AsrEngine, SegmentAsr};
use crate::audio;
use crate::compare::{self, Comparison};
use crate::config::Config;
use crate::models::{self, Engine, Entry, Role};
use crate::paths;
use crate::playback::{Gate, Player};
use crate::ring::{Utterance, UtteranceRing, DEFAULT_CAPACITY};
use crate::translate::{LlamaTranslator, Translator};
use crate::tts::{self, Voice};
use crate::vad::{Segment, Segmenter, VadSettings};
use crate::wav;

/// Chunks of 16 kHz audio queued between capture and the VAD.
const PIPELINE_QUEUE_CHUNKS: usize = 64;

/// How often the pipeline loop wakes to check whether it should stop.
const POLL: Duration = Duration::from_millis(100);

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
    /// Loaded, device open, listening.
    Listening,
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

/// What a caller can ask of a run beyond `convers.toml`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Write each utterance to `logs/segments/` as a WAV.
    pub write_wav: bool,
    /// Run every enabled segment engine on each utterance (SPEC §12).
    pub compare: bool,
}

/// A running pipeline. Dropping it stops it.
pub struct Pipeline {
    stop: Arc<AtomicBool>,
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
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("convers-pipeline".to_string())
                .spawn(move || {
                    if let Err(e) = run(&root, &config, &options, &stop, &events) {
                        warn!("{e:#}");
                        let _ = events.send(PipelineMsg::Error(format!("{e:#}")));
                    }
                    let _ = events.send(PipelineMsg::Stopped);
                })
                .context("cannot spawn the pipeline thread")?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
        })
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
    let (mut selected, mut comparison_engines) = if options.compare {
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
        (None, loaded)
    } else {
        if !selected_name.is_empty() {
            let _ = events.send(PipelineMsg::Loading(format!(
                "recognizer \"{selected_name}\""
            )));
        }
        (load_selected(&engines, &selected_name)?, Vec::new())
    };

    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }

    // The gate exists whether or not TTS does, so the capture loop has one
    // thing to ask rather than two.
    let gate = Arc::new(Gate::new(config.tts.half_duplex));

    let speaking = if config.tts.enabled && !options.compare && selected.is_some() {
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
                "[tts].half_duplex is off: convers will hear its own speech and transcribe it \
                 unless you are wearing headphones (SPEC §10)"
            );
        }
        Some((voice, player))
    } else {
        None
    };

    // No translator in comparison mode: with several transcripts of the same
    // utterance there is no single one to translate, and comparison is a
    // recognizer harness (SPEC §12), not the conversation path.
    let translation = if options.compare || selected.is_none() {
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

    let (tx, rx) = sync_channel::<Vec<f32>>(PIPELINE_QUEUE_CHUNKS);
    let capture = audio::spawn_capture(&config.audio.input_device, tx)?;
    info!("listening — speak now");
    let _ = events.send(PipelineMsg::Listening);

    let started = Instant::now();
    let mut log_level = LevelMeter::new(LEVEL_LOG);
    let mut event_level = LevelMeter::new(LEVEL_EVENT);
    let mut was_gated = false;
    let mut hearing_speech = false;
    // Every sample captured, fed or gated, so the detector can be told where
    // the capture is when it is reset.
    let mut captured: u64 = 0;

    let stage = Stage {
        selected_name: &selected_name,
        language: &language,
        translate_tx: translate_tx.as_ref(),
        segments_dir: options.write_wav.then_some(&segments_dir),
        events,
    };

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(POLL) {
            Ok(chunk) => {
                // Half-duplex (SPEC §10): while our own speech is playing,
                // captured audio is discarded and the detector is held reset,
                // so nothing of it can survive into the next utterance.
                if gate.is_closed() {
                    if !was_gated {
                        debug!("microphone gated while speaking");
                        was_gated = true;
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
                for segment in segmenter.push(&chunk) {
                    stage.handle(
                        segment,
                        &mut ring,
                        selected.as_mut(),
                        &mut comparison_engines,
                    );
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

    // Whatever was still being spoken when the stop came.
    for segment in segmenter.flush() {
        stage.handle(
            segment,
            &mut ring,
            selected.as_mut(),
            &mut comparison_engines,
        );
    }

    let dropped = capture.dropped_chunks();
    capture.stop();

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
        .name("convers-translate".to_string())
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
                        warn!("utterance {}: the voice produced no audio", job.index)
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
                    Err(e) => warn!("utterance {}: synthesis failed: {e:#}", job.index),
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
    /// Transcribe one utterance, keep it, and report it.
    fn handle(
        &self,
        segment: Segment,
        ring: &mut UtteranceRing,
        selected: Option<&mut Box<dyn SegmentAsr + Send>>,
        comparison_engines: &mut [(String, Box<dyn SegmentAsr + Send>)],
    ) {
        let captured_at = Instant::now();
        let (start_ms, end_ms, duration_ms) =
            (segment.start_ms(), segment.end_ms(), segment.duration_ms());
        let utterance = ring.push(start_ms, segment.samples);

        info!(
            "utterance {}: {start_ms} ms .. {end_ms} ms ({duration_ms} ms)",
            utterance.index
        );

        if let Some(asr) = selected {
            let began = Instant::now();
            match asr.transcribe(&utterance.pcm, self.language) {
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

        if !comparison_engines.is_empty() {
            let comparison = compare::run_all(comparison_engines, &utterance, self.language);
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

    /// The whole pipeline on a recording: VAD, recognition, translation and
    /// synthesis, with the latency Milestone 4 asks to be measured. No audio
    /// device is involved, so this runs anywhere the models are installed.
    ///
    /// ```bash
    /// CONVERS_TEST_MODELS=/abs/path/models \
    /// CONVERS_TEST_WAV_ES=/abs/path/spanish-16k.wav \
    /// cargo test --release -- --ignored --nocapture end_to_end
    /// ```
    #[test]
    #[ignore = "needs every model and a Spanish recording; see the doc comment"]
    fn end_to_end_on_a_recording() {
        let models_root = std::env::var("CONVERS_TEST_MODELS").expect("CONVERS_TEST_MODELS");
        let wav_path = std::env::var("CONVERS_TEST_WAV_ES").expect("CONVERS_TEST_WAV_ES");
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
