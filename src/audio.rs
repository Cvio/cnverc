//! Audio capture: device enumeration and a capture thread that hands the rest
//! of the pipeline 16 kHz mono f32 (SPEC §11).
//!
//! Resampling happens here, once, at the capture boundary. Every stage
//! downstream may assume 16 kHz mono f32 in [-1.0, 1.0] and nothing else.
//!
//! Two threads are involved, and the split is deliberate:
//!
//! * The cpal callback runs on the audio device's own realtime thread. It
//!   downmixes to mono and hands the samples off. It never resamples, never
//!   locks, and never blocks — if the channel is full it drops the chunk and
//!   counts it, because stalling the audio callback is worse than losing 10 ms.
//! * The capture thread owns the cpal stream (streams are `!Send`) and the
//!   resampler, and forwards finished 16 kHz chunks downstream.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use sherpa_onnx::LinearResampler;
use tracing::{debug, error, info, warn};

/// The one sample rate the pipeline speaks. Chosen by the ASR and VAD models.
pub const SAMPLE_RATE: u32 = 16_000;

/// Chunks of device audio queued between the cpal callback and the capture
/// thread. Deep enough to ride out a scheduling hiccup, shallow enough that a
/// genuine stall is noticed rather than hidden behind seconds of latency.
const DEVICE_QUEUE_CHUNKS: usize = 64;

/// How often the capture thread wakes to check the stop flag while idle.
const POLL: Duration = Duration::from_millis(100);

/// An audio device as offered to the user, for input or output.
#[derive(Debug, Clone)]
pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
    /// Native configuration, as text for display. `None` if the device
    /// refused to describe itself, which happens with disconnected hardware.
    pub default_config: Option<String>,
}

/// Enumerate input devices. Failure to describe one device is not allowed to
/// hide the others.
pub fn list_input_devices() -> Result<Vec<InputDevice>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    for device in host
        .input_devices()
        .context("cannot enumerate input devices")?
    {
        let name = match device.name() {
            Ok(name) => name,
            Err(e) => {
                warn!("skipping an input device that has no name: {e}");
                continue;
            }
        };
        let default_config = device.default_input_config().ok().map(|c| {
            format!(
                "{} ch, {} Hz, {:?}",
                c.channels(),
                c.sample_rate().0,
                c.sample_format()
            )
        });
        devices.push(InputDevice {
            is_default: Some(&name) == default_name.as_ref(),
            name,
            default_config,
        });
    }
    Ok(devices)
}

/// Enumerate output devices, so a name can be put in `[audio].output_device`.
/// Playback lives in its own module, but device enumeration belongs beside
/// capture's: the two lists are read together and printed together.
pub fn list_output_devices() -> Result<Vec<InputDevice>> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    for device in host
        .output_devices()
        .context("cannot enumerate output devices")?
    {
        let name = match device.name() {
            Ok(name) => name,
            Err(e) => {
                warn!("skipping an output device that has no name: {e}");
                continue;
            }
        };
        let default_config = device.default_output_config().ok().map(|c| {
            format!(
                "{} ch, {} Hz, {:?}",
                c.channels(),
                c.sample_rate().0,
                c.sample_format()
            )
        });
        devices.push(InputDevice {
            is_default: Some(&name) == default_name.as_ref(),
            name,
            default_config,
        });
    }
    Ok(devices)
}

/// A running capture. Dropping the handle does not stop capture; call
/// [`CaptureHandle::stop`] so the device is released deterministically.
pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    dropped_chunks: Arc<AtomicU64>,
}

impl CaptureHandle {
    /// Chunks the cpal callback had to discard because the queue was full.
    /// Non-zero means the machine could not keep up.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks.load(Ordering::Relaxed)
    }

    /// Signal the capture thread to stop and wait for it to release the device.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Open `device_name` (empty = system default) and stream 16 kHz mono f32
/// chunks into `out` until the returned handle is stopped.
///
/// The cpal stream is built on the capture thread, but any failure to open the
/// device is reported back here, so a bad device name is an error from this
/// call rather than a silent no-op.
pub fn spawn_capture(device_name: &str, out: SyncSender<Vec<f32>>) -> Result<CaptureHandle> {
    let device = select_input_device(device_name)?;
    let name = device.name().unwrap_or_else(|_| "<unnamed>".to_string());
    let supported = device
        .default_input_config()
        .with_context(|| format!("input device \"{name}\" has no default configuration"))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;
    let device_rate = config.sample_rate.0;

    let stop = Arc::new(AtomicBool::new(false));
    let dropped_chunks = Arc::new(AtomicU64::new(0));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let thread = {
        let stop = stop.clone();
        let dropped_chunks = dropped_chunks.clone();
        let name = name.clone();
        std::thread::Builder::new()
            .name("cnverc-capture".to_string())
            .spawn(move || {
                capture_thread(
                    device,
                    name,
                    config,
                    sample_format,
                    channels,
                    device_rate,
                    stop,
                    dropped_chunks,
                    out,
                    ready_tx,
                )
            })
            .context("cannot spawn the capture thread")?
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
                "the capture thread exited before opening the device"
            ));
        }
    }

    info!(
        "capture started: \"{name}\" at {device_rate} Hz, {channels} ch -> {SAMPLE_RATE} Hz mono"
    );

    Ok(CaptureHandle {
        stop,
        thread: Some(thread),
        dropped_chunks,
    })
}

/// Empty name = the system default. A named device that is not present is an
/// error naming what was asked for; cnverc never quietly picks another one.
fn select_input_device(device_name: &str) -> Result<Device> {
    let host = cpal::default_host();
    if device_name.trim().is_empty() {
        return host
            .default_input_device()
            .context("no default input device; is a microphone connected?");
    }
    for device in host
        .input_devices()
        .context("cannot enumerate input devices")?
    {
        if device.name().ok().as_deref() == Some(device_name) {
            return Ok(device);
        }
    }
    Err(anyhow!(
        "input device \"{device_name}\" was not found; run with --devices to list what is available"
    ))
}

#[allow(clippy::too_many_arguments)]
fn capture_thread(
    device: Device,
    name: String,
    config: StreamConfig,
    sample_format: SampleFormat,
    channels: usize,
    device_rate: u32,
    stop: Arc<AtomicBool>,
    dropped_chunks: Arc<AtomicU64>,
    out: SyncSender<Vec<f32>>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let (raw_tx, raw_rx) = sync_channel::<Vec<f32>>(DEVICE_QUEUE_CHUNKS);

    let stream = match build_input_stream(
        &device,
        &config,
        sample_format,
        channels,
        raw_tx,
        dropped_chunks.clone(),
    ) {
        Ok(stream) => stream,
        Err(e) => {
            let _ = ready.send(Err(format!("cannot open input device \"{name}\": {e}")));
            return;
        }
    };

    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("cannot start input device \"{name}\": {e}")));
        return;
    }

    // Resample once, here, so nothing downstream has to care (SPEC §11).
    let resampler = if device_rate == SAMPLE_RATE {
        None
    } else {
        match LinearResampler::create(device_rate as i32, SAMPLE_RATE as i32) {
            Some(r) => Some(r),
            None => {
                let _ = ready.send(Err(format!(
                    "cannot create a resampler from {device_rate} Hz to {SAMPLE_RATE} Hz"
                )));
                return;
            }
        }
    };

    let _ = ready.send(Ok(()));

    pump(&raw_rx, resampler.as_ref(), &out, &stop);

    drop(stream); // releases the device
    let dropped = dropped_chunks.load(Ordering::Relaxed);
    if dropped > 0 {
        warn!("capture dropped {dropped} chunk(s): the machine could not keep up");
    }
    info!("capture stopped: \"{name}\"");
}

/// Move mono device audio through the resampler and downstream until stopped.
fn pump(
    raw_rx: &Receiver<Vec<f32>>,
    resampler: Option<&LinearResampler>,
    out: &SyncSender<Vec<f32>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        let chunk = match raw_rx.recv_timeout(POLL) {
            Ok(chunk) => chunk,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        let resampled = match resampler {
            Some(r) => r.resample(&chunk, false),
            None => chunk,
        };
        if resampled.is_empty() {
            continue;
        }

        // Downstream is the pipeline thread. If it has gone away there is
        // nothing left to capture for.
        match out.try_send(resampled) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                debug!("pipeline is behind; dropped a 16 kHz chunk");
            }
            Err(TrySendError::Disconnected(_)) => break,
        }
    }
}

/// Build the cpal stream for whatever sample format the device speaks.
fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: usize,
    raw_tx: SyncSender<Vec<f32>>,
    dropped: Arc<AtomicU64>,
) -> Result<cpal::Stream> {
    match sample_format {
        SampleFormat::F32 => stream_of::<f32>(device, config, channels, raw_tx, dropped),
        SampleFormat::I16 => stream_of::<i16>(device, config, channels, raw_tx, dropped),
        SampleFormat::U16 => stream_of::<u16>(device, config, channels, raw_tx, dropped),
        SampleFormat::I32 => stream_of::<i32>(device, config, channels, raw_tx, dropped),
        SampleFormat::I8 => stream_of::<i8>(device, config, channels, raw_tx, dropped),
        SampleFormat::U8 => stream_of::<u8>(device, config, channels, raw_tx, dropped),
        SampleFormat::F64 => stream_of::<f64>(device, config, channels, raw_tx, dropped),
        other => Err(anyhow!("unsupported input sample format {other:?}")),
    }
}

fn stream_of<T>(
    device: &Device,
    config: &StreamConfig,
    channels: usize,
    raw_tx: SyncSender<Vec<f32>>,
    dropped: Arc<AtomicU64>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mono = downmix::<T>(data, channels);
                if mono.is_empty() {
                    return;
                }
                // Realtime thread: never block, never log, never allocate more
                // than the one buffer above.
                if let Err(TrySendError::Full(_)) = raw_tx.try_send(mono) {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            },
            |e| error!("audio input stream error: {e}"),
            None,
        )
        .context("cannot build the input stream")?;
    Ok(stream)
}

/// Average the interleaved channels down to mono f32.
fn downmix<T>(data: &[T], channels: usize) -> Vec<f32>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    if channels <= 1 {
        return data.iter().map(|s| s.to_sample::<f32>()).collect();
    }
    let frames = data.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in data.chunks_exact(channels) {
        let sum: f32 = frame.iter().map(|s| s.to_sample::<f32>()).sum();
        mono.push(sum / channels as f32);
    }
    mono
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_the_channels() {
        let stereo: Vec<f32> = vec![1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(downmix::<f32>(&stereo, 2), vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn downmix_passes_mono_through() {
        let mono: Vec<f32> = vec![0.25, -0.25];
        assert_eq!(downmix::<f32>(&mono, 1), mono);
    }

    #[test]
    fn downmix_ignores_a_trailing_partial_frame() {
        // A short read must not produce a sample built from half a frame.
        let ragged: Vec<f32> = vec![1.0, 1.0, 0.0];
        assert_eq!(downmix::<f32>(&ragged, 2), vec![1.0]);
    }

    /// The capture boundary is the only place audio changes rate, so the
    /// cnvercion itself is worth pinning down: a 1 kHz tone recorded at
    /// 48 kHz must still be a 1 kHz tone after it.
    #[test]
    fn resampling_preserves_the_tone_and_the_duration() {
        const DEVICE_RATE: u32 = 48_000;
        const TONE_HZ: f32 = 1_000.0;
        let seconds = 0.5;

        let input: Vec<f32> = (0..(DEVICE_RATE as f32 * seconds) as usize)
            .map(|i| (std::f32::consts::TAU * TONE_HZ * i as f32 / DEVICE_RATE as f32).sin() * 0.5)
            .collect();

        let resampler = LinearResampler::create(DEVICE_RATE as i32, SAMPLE_RATE as i32)
            .expect("create the resampler");
        let output = resampler.resample(&input, true);

        let expected = (SAMPLE_RATE as f32 * seconds) as usize;
        let slack = SAMPLE_RATE as usize / 100; // 10 ms either way
        assert!(
            output.len().abs_diff(expected) < slack,
            "{} samples, expected about {expected}",
            output.len()
        );

        // Count zero crossings: a 1 kHz tone crosses zero 2000 times a second.
        let crossings = output
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        let measured_hz = crossings as f32 / 2.0 / seconds;
        assert!(
            (measured_hz - TONE_HZ).abs() < 20.0,
            "resampled tone measured {measured_hz} Hz"
        );
    }

    #[test]
    fn integer_samples_become_normalised_floats() {
        let pcm: Vec<i16> = vec![i16::MAX, 0, i16::MIN];
        let mono = downmix::<i16>(&pcm, 1);
        assert!((mono[0] - 1.0).abs() < 1e-3);
        assert_eq!(mono[1], 0.0);
        assert!((mono[2] + 1.0).abs() < 1e-3);
    }
}
