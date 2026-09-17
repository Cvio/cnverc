//! WAV files, for ears and for tests.
//!
//! convers does not need WAV at runtime — it exists so a captured utterance
//! can be played back and judged by a human (Milestone 1's check) and so the
//! dual-run harness (SPEC §12) has something to feed.

use std::path::Path;

#[cfg(test)]
use anyhow::anyhow;
use anyhow::{Context, Result};

use crate::audio::SAMPLE_RATE;

/// Write 16 kHz mono f32 samples as a 16-bit PCM WAV, the format every player
/// on the target machine opens without complaint.
pub fn write_16k_mono(path: &Path, samples: &[f32]) -> Result<()> {
    write_any(path, samples, SAMPLE_RATE)
}

/// The same, at whatever rate the samples are. Synthesised speech comes out at
/// the voice's own rate, not the pipeline's.
pub fn write_any(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("cannot write {}", path.display()))?;
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32).round() as i16;
        writer
            .write_sample(value)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    writer
        .finalize()
        .with_context(|| format!("cannot finish writing {}", path.display()))?;
    Ok(())
}

/// Read a mono WAV that is already at 16 kHz. Test support only for now: the
/// pipeline resamples at the capture boundary and nowhere else, so this
/// deliberately refuses to resample rather than quietly introducing a second
/// path.
#[cfg(test)]
pub fn read_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let reader =
        hound::WavReader::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(anyhow!(
            "{} has {} channels; this reader handles mono only",
            path.display(),
            spec.channels
        ));
    }
    if spec.sample_rate != SAMPLE_RATE {
        return Err(anyhow!(
            "{} is {} Hz; this reader handles {SAMPLE_RATE} Hz only",
            path.display(),
            spec.sample_rate
        ));
    }

    let mut reader = reader;
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("cannot decode {}", path.display())),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<_>, _>>()
                .with_context(|| format!("cannot decode {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_wav_reads_back_as_what_went_in() {
        let path = std::env::temp_dir().join(format!("convers-wav-{}.wav", std::process::id()));
        let original: Vec<f32> = (0..SAMPLE_RATE as usize / 100)
            .map(|i| (i as f32 * 0.05).sin() * 0.8)
            .collect();

        write_16k_mono(&path, &original).expect("write");
        let read_back = read_16k_mono(&path).expect("read");
        let _ = std::fs::remove_file(&path);

        assert_eq!(read_back.len(), original.len());
        for (a, b) in original.iter().zip(read_back.iter()) {
            // 16-bit quantisation is the only difference allowed.
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn samples_outside_the_range_are_clamped_not_wrapped() {
        let path = std::env::temp_dir().join(format!("convers-clamp-{}.wav", std::process::id()));
        write_16k_mono(&path, &[2.0, -2.0]).expect("write");
        let read_back = read_16k_mono(&path).expect("read");
        let _ = std::fs::remove_file(&path);

        assert!(read_back[0] > 0.99, "{read_back:?}");
        assert!(read_back[1] < -0.99, "{read_back:?}");
    }
}
