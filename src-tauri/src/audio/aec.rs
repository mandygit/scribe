use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::AppError;

const SPEEX_ECHO_SET_SAMPLING_RATE: c_int = 24;
const EXPECTED_SAMPLE_RATE: u32 = 48_000;
const EXPECTED_CHANNELS: u16 = 1;
const EXPECTED_BITS_PER_SAMPLE: u16 = 16;
const DEFAULT_FRAME_SIZE: usize = 480;
const DEFAULT_FILTER_LENGTH: i32 = 9_600;

#[repr(C)]
struct SpeexEchoState {
    _private: [u8; 0],
}

const RTLD_LAZY: c_int = 0x1;
const SPEEX_LIBRARY_CANDIDATES: [&str; 3] = [
    "libspeexdsp.dylib",
    "/opt/homebrew/lib/libspeexdsp.dylib",
    "/usr/local/lib/libspeexdsp.dylib",
];

type SpeexEchoStateInit = unsafe extern "C" fn(c_int, c_int) -> *mut SpeexEchoState;
type SpeexEchoStateDestroy = unsafe extern "C" fn(*mut SpeexEchoState);
type SpeexEchoCancellation =
    unsafe extern "C" fn(*mut SpeexEchoState, *const i16, *const i16, *mut i16);
type SpeexEchoCtl = unsafe extern "C" fn(*mut SpeexEchoState, c_int, *mut c_void) -> c_int;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

/// Offline acoustic echo cancellation adapter.
pub trait EchoCancellationBackend: Send + Sync + 'static {
    fn process(
        &self,
        microphone_wav_path: &Path,
        reference_wav_path: &Path,
    ) -> Result<PathBuf, AppError>;
}

/// SpeexDSP-backed offline adapter for 48 kHz mono signed 16-bit PCM WAV files.
#[derive(Debug, Default)]
pub struct SpeexEchoCancellationBackend;

impl EchoCancellationBackend for SpeexEchoCancellationBackend {
    fn process(
        &self,
        microphone_wav_path: &Path,
        reference_wav_path: &Path,
    ) -> Result<PathBuf, AppError> {
        let output_path = echo_cancelled_path(microphone_wav_path)?;
        let prepared_reference = prepare_audio_for_aec(
            reference_wav_path,
            &FfmpegReferenceAudioNormalizer,
            AEC_REFERENCE_SUFFIX,
        )?;
        // Bluetooth hands-free microphones record at 16 kHz, so the mic track
        // may need the same resampling as the compressed reference.
        let prepared_microphone = prepare_audio_for_aec(
            microphone_wav_path,
            &FfmpegReferenceAudioNormalizer,
            AEC_MICROPHONE_SUFFIX,
        )?;
        process_wav_pair(
            prepared_reference.path(),
            prepared_microphone.path(),
            &output_path,
        )?;
        Ok(output_path)
    }
}

trait ReferenceAudioNormalizer {
    fn normalize_to_wav(&self, source_path: &Path, output_path: &Path) -> Result<(), AppError>;
}

struct FfmpegReferenceAudioNormalizer;

impl ReferenceAudioNormalizer for FfmpegReferenceAudioNormalizer {
    fn normalize_to_wav(&self, source_path: &Path, output_path: &Path) -> Result<(), AppError> {
        let ffmpeg_path = crate::media_import::resolve_ffmpeg_path(None)?;
        let output = Command::new(ffmpeg_path)
            .arg("-y")
            .arg("-i")
            .arg(source_path)
            .arg("-vn")
            .arg("-ac")
            .arg("1")
            .arg("-ar")
            .arg(EXPECTED_SAMPLE_RATE.to_string())
            .arg("-sample_fmt")
            .arg("s16")
            .arg("-f")
            .arg("wav")
            .arg(output_path)
            .output()
            .map_err(|error| AppError {
                code: "aec_reference_normalization_start_failed".to_string(),
                message: "Could not start ffmpeg for echo cancellation.".to_string(),
                details: Some(error.to_string()),
            })?;

        if !output.status.success() {
            return Err(AppError {
                code: "aec_reference_normalization_failed".to_string(),
                message: "ffmpeg could not convert system audio for echo cancellation.".to_string(),
                details: Some(format!("path={}", source_path.display())),
            });
        }

        Ok(())
    }
}

struct PreparedReferenceAudio {
    path: PathBuf,
    cleanup_path: Option<PathBuf>,
}

impl PreparedReferenceAudio {
    fn existing(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            cleanup_path: None,
        }
    }

    fn temporary(path: PathBuf) -> Self {
        Self {
            cleanup_path: Some(path.clone()),
            path,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PreparedReferenceAudio {
    fn drop(&mut self) {
        if let Some(path) = &self.cleanup_path {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    eprintln!(
                        "Echo cancellation temp cleanup failed: path={}, error={error}",
                        path.display()
                    );
                }
            }
        }
    }
}

const AEC_REFERENCE_SUFFIX: &str = "aec-reference";
const AEC_MICROPHONE_SUFFIX: &str = "aec-input";

fn prepare_audio_for_aec(
    source_path: &Path,
    normalizer: &impl ReferenceAudioNormalizer,
    normalized_suffix: &str,
) -> Result<PreparedReferenceAudio, AppError> {
    if path_has_wav_extension(source_path) && wav_matches_aec_spec(source_path) {
        return Ok(PreparedReferenceAudio::existing(source_path));
    }

    let normalized_path = normalized_aec_wav_path(source_path, normalized_suffix)?;
    normalizer.normalize_to_wav(source_path, &normalized_path)?;
    Ok(PreparedReferenceAudio::temporary(normalized_path))
}

fn wav_matches_aec_spec(path: &Path) -> bool {
    WavReader::open(path)
        .map(|reader| {
            let spec = reader.spec();
            spec.channels == EXPECTED_CHANNELS
                && spec.sample_rate == EXPECTED_SAMPLE_RATE
                && spec.bits_per_sample == EXPECTED_BITS_PER_SAMPLE
                && spec.sample_format == SampleFormat::Int
        })
        .unwrap_or(false)
}

fn normalized_aec_wav_path(source_path: &Path, suffix: &str) -> Result<PathBuf, AppError> {
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError {
            code: "aec_invalid_reference_path".to_string(),
            message: "Could not derive the normalized audio path for echo cancellation."
                .to_string(),
            details: Some(format!("path={}", source_path.display())),
        })?;

    Ok(source_path.with_file_name(format!("{stem}.{suffix}.wav")))
}

fn path_has_wav_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
}

pub fn echo_cancelled_path(microphone_wav_path: &Path) -> Result<PathBuf, AppError> {
    let extension = microphone_wav_path
        .extension()
        .and_then(|value| value.to_str());
    if extension != Some("wav") {
        return Err(AppError {
            code: "aec_unsupported_mic_format".to_string(),
            message: "Echo cancellation requires a microphone WAV file.".to_string(),
            details: Some(format!("path={}", microphone_wav_path.display())),
        });
    }

    let stem = microphone_wav_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError {
            code: "aec_invalid_mic_path".to_string(),
            message: "Could not derive echo-cancelled microphone path.".to_string(),
            details: Some(format!("path={}", microphone_wav_path.display())),
        })?;

    Ok(microphone_wav_path.with_file_name(format!("{stem}.aec.wav")))
}

fn process_wav_pair(
    reference_path: &Path,
    recorded_path: &Path,
    output_path: &Path,
) -> Result<(), AppError> {
    let reference = read_mono_i16_48k(reference_path)?;
    let recorded = read_mono_i16_48k(recorded_path)?;

    let output = cancel_echo(
        &reference,
        &recorded,
        DEFAULT_FRAME_SIZE,
        DEFAULT_FILTER_LENGTH,
    )?;
    write_mono_i16_48k(output_path, &output)
}

fn cancel_echo(
    reference: &[i16],
    recorded: &[i16],
    frame_size: usize,
    filter_length: i32,
) -> Result<Vec<i16>, AppError> {
    let mut aec = SpeexAec::new(frame_size, filter_length, EXPECTED_SAMPLE_RATE)?;
    let mut output = Vec::with_capacity(recorded.len());

    for (index, recorded_frame) in recorded.chunks(frame_size).enumerate() {
        let reference_start = index * frame_size;
        let reference_end = (reference_start + frame_size).min(reference.len());
        let reference_frame = if reference_start < reference.len() {
            &reference[reference_start..reference_end]
        } else {
            &[]
        };
        let mut padded_reference = vec![0_i16; frame_size];
        let mut padded_recorded = vec![0_i16; frame_size];
        padded_reference[..reference_frame.len()].copy_from_slice(reference_frame);
        padded_recorded[..recorded_frame.len()].copy_from_slice(recorded_frame);

        let mut cleaned_frame = vec![0_i16; frame_size];
        aec.cancel_echo(&padded_recorded, &padded_reference, &mut cleaned_frame);
        output.extend_from_slice(&cleaned_frame[..recorded_frame.len()]);
    }

    Ok(output)
}

fn read_mono_i16_48k(path: &Path) -> Result<Vec<i16>, AppError> {
    let mut reader = WavReader::open(path).map_err(|error| AppError {
        code: "aec_read_failed".to_string(),
        message: "Could not read audio for echo cancellation.".to_string(),
        details: Some(format!("path={}, error={error}", path.display())),
    })?;
    let spec = reader.spec();
    if spec.channels != EXPECTED_CHANNELS
        || spec.sample_rate != EXPECTED_SAMPLE_RATE
        || spec.bits_per_sample != EXPECTED_BITS_PER_SAMPLE
        || spec.sample_format != SampleFormat::Int
    {
        return Err(AppError {
            code: "aec_unsupported_wav_format".to_string(),
            message: "Echo cancellation requires 48 kHz mono signed 16-bit PCM WAV inputs."
                .to_string(),
            details: Some(format!(
                "path={}, channels={}, sample_rate={}, bits_per_sample={}, format={:?}",
                path.display(),
                spec.channels,
                spec.sample_rate,
                spec.bits_per_sample,
                spec.sample_format
            )),
        });
    }

    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError {
            code: "aec_read_failed".to_string(),
            message: "Could not decode audio samples for echo cancellation.".to_string(),
            details: Some(format!("path={}, error={error}", path.display())),
        })
}

fn write_mono_i16_48k(path: &Path, samples: &[i16]) -> Result<(), AppError> {
    let spec = WavSpec {
        channels: EXPECTED_CHANNELS,
        sample_rate: EXPECTED_SAMPLE_RATE,
        bits_per_sample: EXPECTED_BITS_PER_SAMPLE,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).map_err(|error| AppError {
        code: "aec_write_failed".to_string(),
        message: "Could not create echo-cancelled microphone WAV.".to_string(),
        details: Some(format!("path={}, error={error}", path.display())),
    })?;
    for sample in samples {
        writer.write_sample(*sample).map_err(|error| AppError {
            code: "aec_write_failed".to_string(),
            message: "Could not write echo-cancelled microphone samples.".to_string(),
            details: Some(format!("path={}, error={error}", path.display())),
        })?;
    }
    writer.finalize().map_err(|error| AppError {
        code: "aec_write_failed".to_string(),
        message: "Could not finalize echo-cancelled microphone WAV.".to_string(),
        details: Some(format!("path={}, error={error}", path.display())),
    })
}

struct SpeexAec {
    state: *mut SpeexEchoState,
    library: SpeexLibrary,
}

impl SpeexAec {
    fn new(frame_size: usize, filter_length: i32, sample_rate: u32) -> Result<Self, AppError> {
        let frame_size = c_int::try_from(frame_size).map_err(|error| AppError {
            code: "aec_invalid_frame_size".to_string(),
            message: "Echo cancellation frame size is too large.".to_string(),
            details: Some(error.to_string()),
        })?;
        let filter_length = c_int::try_from(filter_length).map_err(|error| AppError {
            code: "aec_invalid_filter_length".to_string(),
            message: "Echo cancellation filter length is too large.".to_string(),
            details: Some(error.to_string()),
        })?;
        let library = SpeexLibrary::load()?;
        let state = unsafe { (library.echo_state_init)(frame_size, filter_length) };
        if state.is_null() {
            return Err(AppError {
                code: "aec_initialization_failed".to_string(),
                message: "Could not initialize SpeexDSP echo cancellation.".to_string(),
                details: None,
            });
        }

        let mut sample_rate = c_int::try_from(sample_rate).map_err(|error| AppError {
            code: "aec_invalid_sample_rate".to_string(),
            message: "Echo cancellation sample rate is too large.".to_string(),
            details: Some(error.to_string()),
        })?;
        let control_result = unsafe {
            (library.echo_ctl)(
                state,
                SPEEX_ECHO_SET_SAMPLING_RATE,
                (&mut sample_rate as *mut c_int).cast::<c_void>(),
            )
        };
        if control_result != 0 {
            unsafe { (library.echo_state_destroy)(state) };
            return Err(AppError {
                code: "aec_configuration_failed".to_string(),
                message: "Could not configure SpeexDSP echo cancellation.".to_string(),
                details: Some(format!("speex_result={control_result}")),
            });
        }

        Ok(Self { state, library })
    }

    fn cancel_echo(&mut self, recorded: &[i16], reference: &[i16], output: &mut [i16]) {
        unsafe {
            (self.library.echo_cancellation)(
                self.state,
                recorded.as_ptr(),
                reference.as_ptr(),
                output.as_mut_ptr(),
            )
        };
    }
}

impl Drop for SpeexAec {
    fn drop(&mut self) {
        unsafe { (self.library.echo_state_destroy)(self.state) };
    }
}

struct SpeexLibrary {
    handle: *mut c_void,
    echo_state_init: SpeexEchoStateInit,
    echo_state_destroy: SpeexEchoStateDestroy,
    echo_cancellation: SpeexEchoCancellation,
    echo_ctl: SpeexEchoCtl,
}

impl SpeexLibrary {
    fn load() -> Result<Self, AppError> {
        let mut attempted = Vec::with_capacity(SPEEX_LIBRARY_CANDIDATES.len());
        for candidate in SPEEX_LIBRARY_CANDIDATES {
            attempted.push(candidate.to_string());
            let path = CString::new(candidate).map_err(|error| AppError {
                code: "aec_library_path_invalid".to_string(),
                message: "SpeexDSP library path is invalid.".to_string(),
                details: Some(error.to_string()),
            })?;
            let handle = unsafe { dlopen(path.as_ptr(), RTLD_LAZY) };
            if handle.is_null() {
                continue;
            }

            return Self::from_handle(handle);
        }

        Err(AppError {
            code: "aec_library_unavailable".to_string(),
            message: "SpeexDSP is not available; using raw microphone audio for transcription."
                .to_string(),
            details: Some(format!("attempted={}", attempted.join(","))),
        })
    }

    fn from_handle(handle: *mut c_void) -> Result<Self, AppError> {
        let result = (|| {
            let echo_state_init = unsafe {
                std::mem::transmute::<*mut c_void, SpeexEchoStateInit>(load_symbol_ptr(
                    handle,
                    "speex_echo_state_init",
                )?)
            };
            let echo_state_destroy = unsafe {
                std::mem::transmute::<*mut c_void, SpeexEchoStateDestroy>(load_symbol_ptr(
                    handle,
                    "speex_echo_state_destroy",
                )?)
            };
            let echo_cancellation = unsafe {
                std::mem::transmute::<*mut c_void, SpeexEchoCancellation>(load_symbol_ptr(
                    handle,
                    "speex_echo_cancellation",
                )?)
            };
            let echo_ctl = unsafe {
                std::mem::transmute::<*mut c_void, SpeexEchoCtl>(load_symbol_ptr(
                    handle,
                    "speex_echo_ctl",
                )?)
            };

            Ok(Self {
                handle,
                echo_state_init,
                echo_state_destroy,
                echo_cancellation,
                echo_ctl,
            })
        })();

        if result.is_err() {
            unsafe {
                dlclose(handle);
            }
        }

        result
    }
}

impl Drop for SpeexLibrary {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.handle);
        }
    }
}

fn load_symbol_ptr(handle: *mut c_void, name: &str) -> Result<*mut c_void, AppError> {
    let symbol_name = CString::new(name).map_err(|error| AppError {
        code: "aec_symbol_name_invalid".to_string(),
        message: "SpeexDSP symbol name is invalid.".to_string(),
        details: Some(error.to_string()),
    })?;
    let symbol = unsafe { dlsym(handle, symbol_name.as_ptr()) };
    if symbol.is_null() {
        return Err(AppError {
            code: "aec_symbol_unavailable".to_string(),
            message: "SpeexDSP is missing a required echo cancellation symbol.".to_string(),
            details: Some(format!("symbol={name}")),
        });
    }

    Ok(symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::storage::safe_wav_path;
    use crate::domain::MeetingId;
    use std::cell::Cell;
    use tempfile::tempdir;

    struct StubReferenceAudioNormalizer {
        calls: Cell<u8>,
        samples: Vec<i16>,
    }

    impl ReferenceAudioNormalizer for StubReferenceAudioNormalizer {
        fn normalize_to_wav(
            &self,
            _source_path: &Path,
            output_path: &Path,
        ) -> Result<(), AppError> {
            self.calls.set(self.calls.get().saturating_add(1));
            write_mono_i16_48k(output_path, &self.samples)
        }
    }

    #[test]
    fn derives_echo_cancelled_path_next_to_microphone_wav() {
        let path = Path::new("/tmp/scribe/meeting-1.wav");

        let derived = echo_cancelled_path(path).expect("path is derived");

        assert_eq!(derived, PathBuf::from("/tmp/scribe/meeting-1.aec.wav"));
    }

    #[test]
    fn rejects_non_wav_microphone_input() {
        let error = echo_cancelled_path(Path::new("/tmp/scribe/meeting-1.m4a"))
            .expect_err("m4a microphone input is rejected");

        assert_eq!(error.code, "aec_unsupported_mic_format");
    }

    #[test]
    fn normalizes_non_wav_reference_audio_for_aec_and_cleans_it_up() {
        let directory = tempdir().expect("temp directory");
        let reference_path = directory.path().join("reference.m4a");
        std::fs::write(&reference_path, b"fake m4a").expect("reference placeholder");
        let expected_path = directory.path().join("reference.aec-reference.wav");
        let normalizer = StubReferenceAudioNormalizer {
            calls: Cell::new(0),
            samples: vec![0_i16; DEFAULT_FRAME_SIZE],
        };

        {
            let prepared =
                prepare_audio_for_aec(&reference_path, &normalizer, AEC_REFERENCE_SUFFIX)
                    .expect("reference audio is normalized");
            assert_eq!(prepared.path(), expected_path);
            assert!(expected_path.exists());
        }

        assert_eq!(normalizer.calls.get(), 1);
        assert!(!expected_path.exists());
    }

    #[test]
    fn normalizes_non_48k_microphone_wav_for_aec() {
        // Bluetooth hands-free microphones record at 16 kHz; that WAV must be
        // resampled rather than rejected with aec_unsupported_wav_format.
        let directory = tempdir().expect("temp directory");
        let microphone_path = directory.path().join("microphone.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer =
            WavWriter::create(&microphone_path, spec).expect("16 kHz wav can be created");
        for _ in 0..spec.sample_rate {
            writer.write_sample(0_i16).expect("sample can be written");
        }
        writer.finalize().expect("16 kHz wav can be finalized");
        let expected_path = directory.path().join("microphone.aec-input.wav");
        let normalizer = StubReferenceAudioNormalizer {
            calls: Cell::new(0),
            samples: vec![0_i16; DEFAULT_FRAME_SIZE],
        };

        let prepared = prepare_audio_for_aec(&microphone_path, &normalizer, AEC_MICROPHONE_SUFFIX)
            .expect("16 kHz microphone audio is normalized");

        assert_eq!(prepared.path(), expected_path);
        assert_eq!(normalizer.calls.get(), 1);
    }

    #[test]
    fn keeps_conforming_48k_wav_without_normalizing() {
        let directory = tempdir().expect("temp directory");
        let wav_path = directory.path().join("conforming.wav");
        write_mono_i16_48k(&wav_path, &[0_i16; 480]).expect("48 kHz wav can be written");
        let normalizer = StubReferenceAudioNormalizer {
            calls: Cell::new(0),
            samples: Vec::new(),
        };

        let prepared = prepare_audio_for_aec(&wav_path, &normalizer, AEC_MICROPHONE_SUFFIX)
            .expect("conforming audio passes through");

        assert_eq!(prepared.path(), wav_path);
        assert_eq!(normalizer.calls.get(), 0);
    }

    #[test]
    fn adapter_writes_cleaned_wav_without_changing_raw_microphone_file() {
        let directory = tempdir().expect("temp directory");
        let meeting_id = MeetingId::new("aec-adapter");
        let microphone_path =
            safe_wav_path(directory.path(), meeting_id.as_str()).expect("safe mic path");
        let reference_path = directory.path().join("reference.wav");
        let samples = vec![0_i16; DEFAULT_FRAME_SIZE + 17];
        write_mono_i16_48k(&microphone_path, &samples).expect("mic wav");
        write_mono_i16_48k(&reference_path, &samples).expect("reference wav");

        match SpeexEchoCancellationBackend.process(&microphone_path, &reference_path) {
            Ok(output_path) => {
                assert_eq!(
                    output_path,
                    directory
                        .path()
                        .join("recordings")
                        .join("aec-adapter.aec.wav")
                );
                assert!(output_path.exists());
                assert_eq!(
                    read_mono_i16_48k(&output_path)
                        .expect("output samples")
                        .len(),
                    samples.len()
                );
            }
            Err(error) => assert_eq!(error.code, "aec_library_unavailable"),
        }
        assert!(microphone_path.exists());
    }

    #[test]
    fn adapter_handles_unaligned_reference_audio_lengths() {
        let directory = tempdir().expect("temp directory");
        let meeting_id = MeetingId::new("aec-adapter-mismatch");
        let microphone_path =
            safe_wav_path(directory.path(), meeting_id.as_str()).expect("safe mic path");
        let reference_path = directory.path().join("reference-short.wav");
        let microphone_samples = vec![0_i16; DEFAULT_FRAME_SIZE * 2];
        let reference_samples = vec![0_i16; DEFAULT_FRAME_SIZE + 17];
        write_mono_i16_48k(&microphone_path, &microphone_samples).expect("mic wav");
        write_mono_i16_48k(&reference_path, &reference_samples).expect("reference wav");

        match SpeexEchoCancellationBackend.process(&microphone_path, &reference_path) {
            Ok(output_path) => {
                assert!(output_path.exists());
                assert_eq!(
                    read_mono_i16_48k(&output_path)
                        .expect("output samples")
                        .len(),
                    microphone_samples.len()
                );
            }
            Err(error) => assert_ne!(error.code, "aec_channel_length_mismatch"),
        }
    }
}
