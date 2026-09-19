//! Playback, and the half-duplex gate that keeps cnverc from hearing itself.
//!
//! SPEC §10: with TTS playing through speakers and the microphone live, the
//! VAD triggers on the translated speech and the recognizer transcribes it,
//! which in a two-language pipeline is a loop. The fix is not echo
//! cancellation, it is not listening while speaking.
//!
//! So playback owns a [`Gate`]. It closes when the first sample is handed to
//! the sound card and opens 150 ms after the last one has played. While it is
//! closed the pipeline thread discards captured audio and holds the VAD reset,
//! so no fragment of our own voice survives into the next utterance.
//!
//! The 150 ms tail is for the speaker and the room: the sound card is still
//! emptying its own buffer when our queue runs dry.
//!
//! In turn-based mode (SPEC §8) the user decides when the microphone is live,
//! and that decision wins. Taking a turn stops any reply mid-word and holds
//! whatever arrives during the turn until it ends, through a
//! [`PlaybackControl`]. Otherwise a reply playing into the gate would throw
//! away the start of the very turn the user just asked for.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use sherpa_onnx::LinearResampler;

use crate::pipeline::PipelineMsg;
use tracing::{debug, error, info, warn};

/// How long after the last sample the microphone stays shut (SPEC §10).
const TAIL: Duration = Duration::from_millis(150);

/// How often the player thread checks whether the queue has drained.
const POLL: Duration = Duration::from_millis(10);

/// Ceiling on queued audio, in samples at the device rate. Roughly 30 seconds
/// at 48 kHz: an utterance is a second or two, so hitting this means something
/// is wrong and dropping is better than growing without bound.
const MAX_QUEUED: usize = 48_000 * 30;

/// Whether the microphone should be ignored right now.
///
/// Shared between the player, which closes it, and the pipeline thread, which
/// reads it on every chunk.
#[derive(Debug)]
pub struct Gate {
    speaking: AtomicBool,
    /// When the tail expires, as milliseconds on a shared monotonic base.
    open_at_ms: AtomicU64,
    base: Instant,
    /// False when the user has turned half-duplex off for headphone use.
    enabled: bool,
}

impl Gate {
    pub fn new(enabled: bool) -> Self {
        Self {
            speaking: AtomicBool::new(false),
            open_at_ms: AtomicU64::new(0),
            base: Instant::now(),
            enabled,
        }
    }

    /// True while captured audio must be discarded and the VAD held reset.
    pub fn is_closed(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.speaking.load(Ordering::Acquire) {
            return true;
        }
        let open_at = self.open_at_ms.load(Ordering::Acquire);
        open_at > 0 && self.elapsed_ms() < open_at
    }

    fn elapsed_ms(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }

    fn speaking_started(&self) {
        self.speaking.store(true, Ordering::Release);
    }

    fn speaking_ended(&self) {
        self.open_at_ms.store(
            self.elapsed_ms() + TAIL.as_millis() as u64,
            Ordering::Release,
        );
        self.speaking.store(false, Ordering::Release);
    }

    /// Open at once, with no tail: a reply has been cut short because the user
    /// took a turn, and nothing more is coming out of the speaker.
    fn open_now(&self) {
        self.open_at_ms.store(0, Ordering::Release);
        self.speaking.store(false, Ordering::Release);
    }
}

/// The queue the cpal output callback drains.
struct Queue {
    samples: VecDeque<f32>,
}

/// A running output stream. Dropping it stops playback and releases the device.
pub struct Player {
    queue: Arc<Mutex<Queue>>,
    gate: Arc<Gate>,
    /// True during a user's turn: nothing plays, and anything queued waits.
    hold: Arc<AtomicBool>,
    channels: usize,
    device_rate: u32,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Player {
    /// Open `device_name` (empty = system default) and start an output stream.
    ///
    /// Like capture, the stream is built on its own thread because cpal
    /// streams are `!Send`, and any failure to open the device is reported
    /// back here rather than vanishing into that thread.
    ///
    /// `events`, when given, hears [`PipelineMsg::SpeakingEnded`] each time the
    /// queue drains.
    pub fn open(
        device_name: &str,
        gate: Arc<Gate>,
        events: Option<Sender<PipelineMsg>>,
    ) -> Result<Self> {
        let device = select_output_device(device_name)?;
        let name = device.name().unwrap_or_else(|_| "<unnamed>".to_string());
        let supported = device
            .default_output_config()
            .with_context(|| format!("output device \"{name}\" has no default configuration"))?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let channels = config.channels as usize;
        let device_rate = config.sample_rate.0;

        let queue = Arc::new(Mutex::new(Queue {
            samples: VecDeque::new(),
        }));
        let hold = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let thread = {
            let queue = queue.clone();
            let gate = gate.clone();
            let hold = hold.clone();
            let stop = stop.clone();
            let name = name.clone();
            std::thread::Builder::new()
                .name("cnverc-playback".to_string())
                .spawn(move || {
                    player_thread(
                        device,
                        name,
                        config,
                        sample_format,
                        queue,
                        gate,
                        hold,
                        events,
                        stop,
                        ready_tx,
                    )
                })
                .context("cannot spawn the playback thread")?
        };

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = thread.join();
                return Err(anyhow!(e));
            }
            Err(_) => {
                let _ = thread.join();
                return Err(anyhow!(
                    "the playback thread exited before opening the device"
                ));
            }
        }

        info!("playback started: \"{name}\" at {device_rate} Hz, {channels} ch");

        Ok(Self {
            queue,
            gate,
            hold,
            channels,
            device_rate,
            stop,
            thread: Some(thread),
        })
    }

    /// Queue an utterance. Returns immediately; the sound card does the rest.
    ///
    /// The gate closes here rather than in the callback, so the microphone is
    /// already shut before the first sample reaches the speakers.
    pub fn play(&self, samples: &[f32], sample_rate: u32) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }

        let resampled = if sample_rate == self.device_rate {
            samples.to_vec()
        } else {
            let resampler = LinearResampler::create(sample_rate as i32, self.device_rate as i32)
                .ok_or_else(|| {
                    anyhow!(
                        "cannot resample speech from {sample_rate} Hz to {} Hz",
                        self.device_rate
                    )
                })?;
            resampler.resample(samples, true)
        };

        // A reply that arrives during the user's turn waits, and must not
        // close the gate on the turn's audio. It closes the gate when it plays.
        if !self.hold.load(Ordering::Acquire) {
            self.gate.speaking_started();
        }

        let mut queue = self
            .queue
            .lock()
            .map_err(|_| anyhow!("the playback queue is poisoned"))?;
        if queue.samples.len() + resampled.len() * self.channels > MAX_QUEUED {
            warn!("playback queue is full; dropping an utterance");
            return Ok(());
        }
        // The device wants interleaved frames; the same mono sample goes to
        // every channel.
        for sample in resampled {
            for _ in 0..self.channels {
                queue.samples.push_back(sample);
            }
        }
        Ok(())
    }
}

impl Player {
    /// A handle the pipeline thread keeps, to stop and hold playback when the
    /// user takes a turn. The player itself stays with the thread that speaks.
    pub fn control(&self) -> PlaybackControl {
        PlaybackControl {
            queue: self.queue.clone(),
            gate: self.gate.clone(),
            hold: self.hold.clone(),
        }
    }
}

/// What taking a turn does to playback (SPEC §8).
#[derive(Clone)]
pub struct PlaybackControl {
    queue: Arc<Mutex<Queue>>,
    gate: Arc<Gate>,
    hold: Arc<AtomicBool>,
}

impl PlaybackControl {
    /// The user is taking a turn: cut off any reply mid-word, open the gate at
    /// once, and hold whatever arrives until the turn ends.
    pub fn begin_turn(&self) {
        self.hold.store(true, Ordering::Release);
        if let Ok(mut queue) = self.queue.lock() {
            queue.samples.clear();
        }
        self.gate.open_now();
    }

    /// The turn is over: play anything that arrived during it.
    pub fn end_turn(&self) {
        let waiting = self
            .queue
            .lock()
            .map(|queue| !queue.samples.is_empty())
            .unwrap_or(false);
        if waiting {
            self.gate.speaking_started();
        }
        self.hold.store(false, Ordering::Release);
    }
}

/// Dropping the player stops the stream and releases the device. The speaking
/// stage owns it for the life of the session, so there is nothing to stop it
/// earlier than that.
impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn select_output_device(device_name: &str) -> Result<Device> {
    let host = cpal::default_host();
    if device_name.trim().is_empty() {
        return host
            .default_output_device()
            .context("no default output device; is anything connected?");
    }
    for device in host
        .output_devices()
        .context("cannot enumerate output devices")?
    {
        if device.name().ok().as_deref() == Some(device_name) {
            return Ok(device);
        }
    }
    Err(anyhow!(
        "output device \"{device_name}\" was not found; leave [audio].output_device empty for \
         the system default"
    ))
}

#[allow(clippy::too_many_arguments)]
fn player_thread(
    device: Device,
    name: String,
    config: StreamConfig,
    sample_format: SampleFormat,
    queue: Arc<Mutex<Queue>>,
    gate: Arc<Gate>,
    hold: Arc<AtomicBool>,
    events: Option<Sender<PipelineMsg>>,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let stream =
        match build_output_stream(&device, &config, sample_format, queue.clone(), hold.clone()) {
            Ok(stream) => stream,
            Err(e) => {
                let _ = ready.send(Err(format!("cannot open output device \"{name}\": {e}")));
                return;
            }
        };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("cannot start output device \"{name}\": {e}")));
        return;
    }
    let _ = ready.send(Ok(()));

    // Watch for the queue draining. Doing this here rather than in the
    // callback keeps the realtime thread free of clocks and logging.
    let mut was_speaking = false;
    while !stop.load(Ordering::Relaxed) {
        // During a turn nothing plays. A reply that was cut off has ended as
        // far as anyone listening is concerned; the gate was already opened
        // by begin_turn, so it is not touched here.
        if hold.load(Ordering::Acquire) {
            if was_speaking {
                was_speaking = false;
                if let Some(events) = &events {
                    let _ = events.send(PipelineMsg::SpeakingEnded);
                }
            }
            std::thread::sleep(POLL);
            continue;
        }

        let empty = match queue.lock() {
            Ok(queue) => queue.samples.is_empty(),
            Err(_) => {
                error!("the playback queue is poisoned");
                break;
            }
        };

        if !empty {
            was_speaking = true;
        } else if was_speaking {
            was_speaking = false;
            gate.speaking_ended();
            debug!(
                "speaking ended; the microphone reopens in {} ms",
                TAIL.as_millis()
            );
            if let Some(events) = &events {
                let _ = events.send(PipelineMsg::SpeakingEnded);
            }
        }

        std::thread::sleep(POLL);
    }

    drop(stream);
    gate.speaking_ended();
    info!("playback stopped: \"{name}\"");
}

fn build_output_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    queue: Arc<Mutex<Queue>>,
    hold: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    match sample_format {
        SampleFormat::F32 => stream_of::<f32>(device, config, queue, hold),
        SampleFormat::I16 => stream_of::<i16>(device, config, queue, hold),
        SampleFormat::U16 => stream_of::<u16>(device, config, queue, hold),
        SampleFormat::I32 => stream_of::<i32>(device, config, queue, hold),
        SampleFormat::I8 => stream_of::<i8>(device, config, queue, hold),
        SampleFormat::U8 => stream_of::<u8>(device, config, queue, hold),
        SampleFormat::F64 => stream_of::<f64>(device, config, queue, hold),
        other => Err(anyhow!("unsupported output sample format {other:?}")),
    }
}

fn stream_of<T>(
    device: &Device,
    config: &StreamConfig,
    queue: Arc<Mutex<Queue>>,
    hold: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                // Realtime thread: a lock that is never held long, and silence
                // if it is somehow unavailable. Never block, never allocate.
                // Held for the user's turn: silence, and the queue waits.
                if hold.load(Ordering::Relaxed) {
                    for sample in data.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                    return;
                }
                let Ok(mut queue) = queue.try_lock() else {
                    for sample in data.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                    return;
                };
                for slot in data.iter_mut() {
                    let value = queue.samples.pop_front().unwrap_or(0.0);
                    *slot = T::from_sample(value);
                }
            },
            |e| error!("audio output stream error: {e}"),
            None,
        )
        .context("cannot build the output stream")?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_is_closed_while_speaking_and_through_the_tail() {
        let gate = Gate::new(true);
        assert!(!gate.is_closed(), "idle");

        gate.speaking_started();
        assert!(gate.is_closed(), "speaking");

        gate.speaking_ended();
        assert!(gate.is_closed(), "still closed during the tail");

        std::thread::sleep(TAIL + Duration::from_millis(60));
        assert!(!gate.is_closed(), "open again after the tail");
    }

    #[test]
    fn a_disabled_gate_never_closes() {
        // tts.half_duplex = false, which the tooltip says is for headphones.
        let gate = Gate::new(false);
        gate.speaking_started();
        assert!(!gate.is_closed());
        gate.speaking_ended();
        assert!(!gate.is_closed());
    }

    /// The whole point of Milestone 4, against real devices.
    ///
    /// A virtual audio cable makes the feedback path of SPEC §10 exact and
    /// silent: playback goes into the cable's input, capture comes out of its
    /// output, so cnverc hears its own voice at full level with no speakers
    /// involved. The test runs twice — with the gate off, to prove the loop is
    /// real and the recognizer would hear itself, and with the gate on, which
    /// must produce nothing at all.
    ///
    /// ```bash
    /// cnverc_TEST_MODELS=/abs/path/models     /// cnverc_TEST_OUT_DEVICE="CABLE Input (VB-Audio Virtual Cable)"     /// cnverc_TEST_IN_DEVICE="CABLE Output (VB-Audio Virtual Cable)"     /// cargo test --release -- --ignored --nocapture hearing_itself
    /// ```
    #[test]
    #[ignore = "needs a loopback audio device and a voice; see the doc comment"]
    fn the_gate_stops_cnverc_hearing_itself() {
        use crate::audio;
        use crate::models::{self, Entry, Role};
        use crate::tts::{self, Voice};
        use crate::vad::{Segmenter, VadSettings};
        use std::path::{Path, PathBuf};
        use std::sync::mpsc::{sync_channel, RecvTimeoutError};

        let models_root = std::env::var("cnverc_TEST_MODELS").expect("cnverc_TEST_MODELS");
        let out_device = std::env::var("cnverc_TEST_OUT_DEVICE").expect("cnverc_TEST_OUT_DEVICE");
        let in_device = std::env::var("cnverc_TEST_IN_DEVICE").expect("cnverc_TEST_IN_DEVICE");

        let voices: Vec<crate::models::Engine> =
            models::discover(&Path::new(&models_root).join("tts"), Role::Tts)
                .into_iter()
                .filter_map(|e| match e {
                    Entry::Loaded(engine) => Some(engine),
                    Entry::Failed { .. } => None,
                })
                .collect();
        let voice = Voice::load(tts::for_language(&voices, "en").expect("an English voice"))
            .expect("load the voice");
        let speech = voice
            .speak("Where is the station? The train leaves at nine o'clock.")
            .expect("synthesise");
        assert!(speech.duration_ms() > 1000, "need a few seconds of speech");

        let vad_settings = VadSettings {
            model: PathBuf::from(&models_root)
                .join("vad")
                .join("silero_vad.onnx"),
            threshold: 0.5,
            min_silence_ms: 500,
            min_speech_ms: 250,
        };

        for gate_enabled in [false, true] {
            let gate = Arc::new(Gate::new(gate_enabled));
            let player = Player::open(&out_device, gate.clone(), None).expect("open playback");
            let (tx, rx) = sync_channel::<Vec<f32>>(64);
            let capture = audio::spawn_capture(&in_device, tx).expect("open capture");
            let mut segmenter = Segmenter::new(&vad_settings).expect("load the VAD");

            player
                .play(&speech.samples, speech.sample_rate)
                .expect("play");

            let mut heard = 0usize;
            let deadline = Instant::now() + Duration::from_millis(speech.duration_ms() + 3000);
            while Instant::now() < deadline {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(chunk) => {
                        if gate.is_closed() {
                            // Only the count of utterances matters here,
                            // not where they sit on the timeline.
                            segmenter.reset(0);
                            continue;
                        }
                        heard += segmenter.push(&chunk).len();
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            heard += segmenter.flush().len();

            capture.stop();
            drop(player);

            println!(
                "half_duplex {}: heard {heard} utterance(s) of our own speech",
                if gate_enabled { "on " } else { "off" }
            );
            if gate_enabled {
                assert_eq!(heard, 0, "the gate let our own speech through");
            } else {
                assert!(
                    heard > 0,
                    "the loopback delivered nothing, so this test proves nothing: check that \
                     playback and capture really are two ends of one cable"
                );
            }
        }
    }

    #[test]
    fn taking_a_turn_opens_the_gate_at_once() {
        // A reply is playing; the user takes a turn. There is no tail to wait
        // out, because the reply has stopped.
        let gate = Gate::new(true);
        gate.speaking_started();
        assert!(gate.is_closed());
        gate.open_now();
        assert!(!gate.is_closed(), "the turn's first words would be lost");
    }

    #[test]
    fn a_reply_held_through_a_turn_closes_the_gate_only_when_it_plays() {
        let gate = Arc::new(Gate::new(true));
        let control = PlaybackControl {
            queue: Arc::new(Mutex::new(Queue {
                samples: VecDeque::new(),
            })),
            gate: gate.clone(),
            hold: Arc::new(AtomicBool::new(false)),
        };

        control.begin_turn();
        // A reply arrives mid-turn and is queued, but held.
        if let Ok(mut queue) = control.queue.lock() {
            queue.samples.extend([0.1, 0.2, 0.3]);
        }
        assert!(!gate.is_closed(), "a held reply must not gate the turn");

        control.end_turn();
        assert!(
            gate.is_closed(),
            "the held reply now plays, and gates the mic"
        );
        assert!(!control.hold.load(Ordering::Acquire));
    }

    #[test]
    fn taking_a_turn_cuts_off_the_reply_that_was_playing() {
        let control = PlaybackControl {
            queue: Arc::new(Mutex::new(Queue {
                samples: VecDeque::from(vec![0.5; 480]),
            })),
            gate: Arc::new(Gate::new(true)),
            hold: Arc::new(AtomicBool::new(false)),
        };
        control.begin_turn();
        let empty = control.queue.lock().map(|q| q.samples.is_empty());
        assert_eq!(empty.ok(), Some(true));
    }

    #[test]
    fn a_second_utterance_keeps_the_gate_closed() {
        let gate = Gate::new(true);
        gate.speaking_started();
        gate.speaking_ended();
        gate.speaking_started();
        std::thread::sleep(TAIL + Duration::from_millis(60));
        // The tail from the first utterance has expired, but we are speaking
        // again, so the microphone must stay shut.
        assert!(gate.is_closed());
    }
}
