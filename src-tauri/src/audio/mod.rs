pub mod aec;
pub mod capture;
pub mod domain;
pub mod manager;
pub mod storage;
pub mod system;
pub mod wav;

pub use capture::CpalCaptureBackend;
pub use domain::{AudioDevice, RecordingMetadata, RecordingStarted};
pub use manager::RecordingManager;
pub use system::ScreenCaptureKitSystemAudioBackend;

pub(crate) fn audio_error(
    code: &str,
    message: &str,
    details: Option<String>,
) -> crate::domain::AppError {
    crate::domain::AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}
