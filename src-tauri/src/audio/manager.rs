use std::path::PathBuf;

use crate::domain::AppError;

use super::{
    audio_error,
    capture::{ActiveCapture, AudioCaptureBackend},
    domain::{AudioDevice, RecordingMetadata, RecordingStarted},
    system::{ActiveSystemAudioCapture, SystemAudioCaptureBackend},
};

pub struct RecordingManager<B, S> {
    backend: B,
    system_backend: S,
    active: Option<ActiveRecording>,
}

struct ActiveRecording {
    meeting_id: String,
    capture: ActiveCapture,
    system_capture: Option<ActiveSystemAudioCapture>,
}

impl<B: AudioCaptureBackend, S: SystemAudioCaptureBackend> RecordingManager<B, S> {
    pub fn new(backend: B, system_backend: S) -> Self {
        Self {
            backend,
            system_backend,
            active: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    pub fn list_audio_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
        self.backend.list_input_devices()
    }

    pub fn start_recording(
        &mut self,
        meeting_id: String,
        file_path: PathBuf,
        system_audio_file_path: Option<PathBuf>,
        device_id: Option<String>,
    ) -> Result<RecordingStarted, AppError> {
        if self.active.is_some() {
            return Err(audio_error(
                "recording_already_active",
                "Cannot start a new recording while another meeting is recording.",
                None,
            ));
        }

        let system_capture = system_audio_file_path
            .clone()
            .map(|path| self.system_backend.start_capture(path))
            .transpose()?;
        let capture = match self.backend.start_recording(file_path, device_id) {
            Ok(capture) => capture,
            Err(error) => {
                if let Some(system_capture) = system_capture {
                    let _ = system_capture.handle.stop();
                }
                return Err(error);
            }
        };
        let started = RecordingStarted {
            meeting_id: meeting_id.clone(),
            file_path: capture.file_path.to_string_lossy().to_string(),
            system_audio_file_path: system_capture
                .as_ref()
                .map(|capture| capture.file_path.to_string_lossy().to_string()),
            started_at_ms: capture.started_at_ms,
            sample_rate_hz: capture.sample_rate_hz,
        };
        self.active = Some(ActiveRecording {
            meeting_id,
            capture,
            system_capture,
        });

        Ok(started)
    }

    pub fn stop_recording(&mut self, stopped_at_ms: u64) -> Result<RecordingMetadata, AppError> {
        let active = self.active.take().ok_or_else(|| {
            audio_error(
                "recording_not_active",
                "Cannot stop recording because no meeting is currently recording.",
                None,
            )
        })?;
        let file_path = active.capture.file_path.clone();
        let started_at_ms = active.capture.started_at_ms;
        let sample_rate_hz = active.capture.sample_rate_hz;
        let summary = active.capture.handle.stop()?;
        let system_summary = active.system_capture.map(|capture| {
            let file_path = capture.file_path.clone();
            let summary = capture.handle.stop();
            let byte_size = std::fs::metadata(&file_path)
                .ok()
                .map(|metadata| metadata.len());
            (
                file_path,
                byte_size.or(summary.byte_size),
                summary.stream_error,
            )
        });
        if sample_rate_hz == 0 {
            return Err(audio_error(
                "audio_input_config_invalid",
                "Microphone input sample rate must be greater than zero.",
                None,
            ));
        }
        let byte_size = std::fs::metadata(&file_path)
            .map_err(|error| {
                audio_error(
                    "audio_file_metadata_failed",
                    "Could not read finalized WAV recording metadata.",
                    Some(error.to_string()),
                )
            })?
            .len();

        Ok(RecordingMetadata {
            meeting_id: active.meeting_id,
            file_path: file_path.to_string_lossy().to_string(),
            system_audio_file_path: system_summary
                .as_ref()
                .map(|(path, _, _)| path.to_string_lossy().to_string()),
            duration_ms: summary.sample_count.saturating_mul(1_000) / u64::from(sample_rate_hz),
            sample_rate_hz,
            byte_size,
            system_audio_byte_size: system_summary
                .as_ref()
                .and_then(|(_, byte_size, _)| *byte_size),
            started_at_ms,
            stopped_at_ms,
            dropped_sample_count: summary.dropped_sample_count,
            stream_error: summary.stream_error,
            system_audio_stream_error: system_summary.and_then(|(_, _, stream_error)| stream_error),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    };

    use crate::audio::{
        capture::{CaptureSessionHandle, CaptureSummary},
        system::{SystemAudioCaptureSummary, SystemAudioSessionHandle},
    };

    use super::*;

    struct FakeBackend {
        starts: Mutex<u32>,
    }

    #[derive(Default)]
    struct FakeSystemBackend;

    struct FailingBackend;

    struct CountingSystemBackend {
        stopped_count: Arc<AtomicU64>,
    }

    impl AudioCaptureBackend for FakeBackend {
        fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
            Ok(vec![AudioDevice {
                id: "input-0".to_string(),
                name: "Fake Mic".to_string(),
                is_default_input: true,
            }])
        }

        fn start_recording(
            &self,
            file_path: PathBuf,
            _device_id: Option<String>,
        ) -> Result<ActiveCapture, AppError> {
            *self.starts.lock().expect("starts lock is healthy") += 1;
            std::fs::write(&file_path, [0_u8; 44]).expect("fake wav exists");
            Ok(ActiveCapture {
                file_path,
                sample_rate_hz: 1_000,
                started_at_ms: 10,
                handle: Box::new(FakeSession),
            })
        }
    }

    impl SystemAudioCaptureBackend for FakeSystemBackend {
        fn start_capture(&self, file_path: PathBuf) -> Result<ActiveSystemAudioCapture, AppError> {
            std::fs::write(&file_path, [1_u8; 16]).expect("fake system audio exists");
            Ok(ActiveSystemAudioCapture {
                file_path,
                sample_rate_hz: 48_000,
                handle: Box::new(FakeSystemSession),
            })
        }
    }

    impl AudioCaptureBackend for FailingBackend {
        fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
            Ok(Vec::new())
        }

        fn start_recording(
            &self,
            _file_path: PathBuf,
            _device_id: Option<String>,
        ) -> Result<ActiveCapture, AppError> {
            Err(audio_error(
                "fake_microphone_failed",
                "Fake microphone failed to start.",
                None,
            ))
        }
    }

    impl SystemAudioCaptureBackend for CountingSystemBackend {
        fn start_capture(&self, file_path: PathBuf) -> Result<ActiveSystemAudioCapture, AppError> {
            std::fs::write(&file_path, [1_u8; 16]).expect("fake system audio exists");
            Ok(ActiveSystemAudioCapture {
                file_path,
                sample_rate_hz: 48_000,
                handle: Box::new(CountingSystemSession {
                    stopped_count: Arc::clone(&self.stopped_count),
                }),
            })
        }
    }

    struct FakeSession;

    impl CaptureSessionHandle for FakeSession {
        fn stop(self: Box<Self>) -> Result<CaptureSummary, AppError> {
            Ok(CaptureSummary {
                sample_count: 250,
                dropped_sample_count: 2,
                stream_error: None,
            })
        }
    }

    struct FakeSystemSession;

    impl SystemAudioSessionHandle for FakeSystemSession {
        fn stop(self: Box<Self>) -> SystemAudioCaptureSummary {
            SystemAudioCaptureSummary {
                byte_size: None,
                stream_error: None,
            }
        }
    }

    struct CountingSystemSession {
        stopped_count: Arc<AtomicU64>,
    }

    impl SystemAudioSessionHandle for CountingSystemSession {
        fn stop(self: Box<Self>) -> SystemAudioCaptureSummary {
            self.stopped_count.fetch_add(1, Ordering::Relaxed);
            SystemAudioCaptureSummary {
                byte_size: None,
                stream_error: None,
            }
        }
    }

    #[test]
    fn recording_manager_prevents_double_start_and_stop_without_active_recording() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/manager");
        std::fs::create_dir_all(&base).expect("test directory can be created");
        let mut manager = RecordingManager::new(
            FakeBackend {
                starts: Mutex::new(0),
            },
            FakeSystemBackend,
        );

        let started = manager
            .start_recording(
                "meeting-1".to_string(),
                base.join("meeting-1.wav"),
                Some(base.join("meeting-1.system.m4a")),
                None,
            )
            .expect("first recording starts");
        assert_eq!(started.sample_rate_hz, 1_000);
        assert_eq!(
            started.system_audio_file_path.as_deref(),
            Some(base.join("meeting-1.system.m4a").to_string_lossy().as_ref())
        );

        let double_start = manager
            .start_recording(
                "meeting-2".to_string(),
                base.join("meeting-2.wav"),
                None,
                None,
            )
            .expect_err("second recording is rejected");
        assert_eq!(double_start.code, "recording_already_active");

        let metadata = manager
            .stop_recording(260)
            .expect("active recording can stop");
        assert_eq!(metadata.duration_ms, 250);
        assert_eq!(metadata.dropped_sample_count, 2);
        assert_eq!(metadata.system_audio_byte_size, Some(16));

        let stop_without_recording = manager
            .stop_recording(300)
            .expect_err("inactive recording cannot stop");
        assert_eq!(stop_without_recording.code, "recording_not_active");
    }

    #[test]
    fn recording_manager_stops_system_capture_when_microphone_start_fails() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/manager-cleanup");
        std::fs::create_dir_all(&base).expect("test directory can be created");
        let stopped_count = Arc::new(AtomicU64::new(0));
        let mut manager = RecordingManager::new(
            FailingBackend,
            CountingSystemBackend {
                stopped_count: Arc::clone(&stopped_count),
            },
        );

        let error = manager
            .start_recording(
                "meeting-cleanup".to_string(),
                base.join("meeting-cleanup.wav"),
                Some(base.join("meeting-cleanup.system.m4a")),
                None,
            )
            .expect_err("microphone failure is returned");

        assert_eq!(error.code, "fake_microphone_failed");
        assert_eq!(stopped_count.load(Ordering::Relaxed), 1);
        assert!(!manager.is_recording());
    }
}
