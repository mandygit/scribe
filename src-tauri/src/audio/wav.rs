use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    thread::{self, JoinHandle},
};

use crate::domain::AppError;

use super::audio_error;

pub struct WavWriterHandle {
    join_handle: JoinHandle<Result<WavWriterSummary, AppError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavWriterSummary {
    pub sample_count: u64,
}

impl WavWriterHandle {
    pub fn join(self) -> Result<WavWriterSummary, AppError> {
        self.join_handle.join().map_err(|_| {
            audio_error(
                "audio_writer_thread_panicked",
                "Audio writer thread panicked while finalizing the WAV file.",
                None,
            )
        })?
    }
}

pub fn spawn_i16_mono_wav_writer(
    file_path: PathBuf,
    sample_rate_hz: u32,
    samples: Receiver<i16>,
) -> WavWriterHandle {
    let join_handle = thread::spawn(move || write_receiver(file_path, sample_rate_hz, samples));
    WavWriterHandle { join_handle }
}

pub fn write_i16_mono_wav(
    file_path: &Path,
    sample_rate_hz: u32,
    samples: &[i16],
) -> Result<WavWriterSummary, AppError> {
    let spec = wav_spec(sample_rate_hz);
    let mut writer = hound::WavWriter::create(file_path, spec).map_err(map_wav_error)?;
    for sample in samples {
        writer.write_sample(*sample).map_err(map_wav_error)?;
    }
    writer.finalize().map_err(map_wav_error)?;

    Ok(WavWriterSummary {
        sample_count: samples.len() as u64,
    })
}

fn write_receiver(
    file_path: PathBuf,
    sample_rate_hz: u32,
    samples: Receiver<i16>,
) -> Result<WavWriterSummary, AppError> {
    let spec = wav_spec(sample_rate_hz);
    let file = File::create(&file_path).map_err(|error| {
        audio_error(
            "audio_file_create_failed",
            "Could not create the WAV recording file.",
            Some(error.to_string()),
        )
    })?;
    let mut writer = hound::WavWriter::new(BufWriter::new(file), spec).map_err(map_wav_error)?;
    let mut sample_count = 0_u64;

    for sample in samples {
        writer.write_sample(sample).map_err(map_wav_error)?;
        sample_count = sample_count.saturating_add(1);
    }

    writer.finalize().map_err(map_wav_error)?;
    Ok(WavWriterSummary { sample_count })
}

fn wav_spec(sample_rate_hz: u32) -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate_hz,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn map_wav_error(error: hound::Error) -> AppError {
    audio_error(
        "audio_wav_write_failed",
        "Could not write the WAV recording file.",
        Some(error.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_i16_mono_wav_writes_readable_pcm_file() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/wav-writer");
        std::fs::create_dir_all(&base).expect("test directory can be created");
        let path = base.join("valid.wav");

        let summary =
            write_i16_mono_wav(&path, 16_000, &[0, i16::MAX, i16::MIN]).expect("wav writes");

        assert_eq!(summary.sample_count, 3);

        let reader = hound::WavReader::open(path).expect("wav can be read");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
    }
}
