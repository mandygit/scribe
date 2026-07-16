//! macOS permission priming for first-run onboarding: microphone, screen
//! recording, and Accessibility.
//!
//! Microphone reuses the same capture code path the app already runs during
//! normal use — starting (and immediately discarding) a short real capture
//! is what actually triggers the OS's permission prompt for that category.
//!
//! Accessibility is different: it goes through `osascript`/System Events
//! rather than a native Accessibility API call, and macOS does not show an
//! automatic system prompt (or add the app to the Accessibility list) for
//! that indirect path — only for a direct `AXIsProcessTrustedWithOptions`
//! call, which this app does not make. `probe_accessibility` only reports
//! current status; granting it requires the user to add the app themselves
//! in System Settings.
//!
//! Screen Recording is different again: `CGRequestScreenCaptureAccess()` (what a
//! real capture attempt triggers) shows the system prompt and returns
//! immediately, before the user answers it. Calling that from an automatic
//! background check — which runs on every app launch — would silently
//! re-fire the system dialog every single time regardless of what the user
//! answered last time. So its status check goes through a dedicated
//! `CGPreflightScreenCaptureAccess()`-only helper mode instead (see
//! `audio::system::check_screen_recording_permission`); only an explicit,
//! user-initiated recording attempt goes through the real request path.

use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::audio::capture::AudioCaptureBackend;
use crate::audio::system::check_screen_recording_permission;
use crate::audio::CpalCaptureBackend;
use crate::dictation;
use crate::domain::AppError;

/// How long a probe capture runs before being stopped and discarded. Long
/// enough for the OS to actually open the audio/video session (and prompt for
/// permission if needed), short enough to be unnoticeable.
const PROBE_CAPTURE_DURATION: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionStatus {
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsSnapshot {
    pub microphone: PermissionStatus,
    pub screen_recording: PermissionStatus,
    pub accessibility: PermissionStatus,
}

/// Runs all three probes and returns a snapshot for the onboarding screen.
pub fn check_permissions() -> PermissionsSnapshot {
    PermissionsSnapshot {
        microphone: probe_microphone(),
        screen_recording: probe_screen_recording(),
        accessibility: probe_accessibility(),
    }
}

pub fn probe_microphone() -> PermissionStatus {
    probe_microphone_with(&CpalCaptureBackend::new())
}

fn probe_microphone_with<B: AudioCaptureBackend>(backend: &B) -> PermissionStatus {
    let path = probe_temp_path("scribe-permission-probe-mic", "wav");
    let outcome = backend
        .start_recording(path.clone(), None)
        .and_then(|capture| {
            std::thread::sleep(PROBE_CAPTURE_DURATION);
            capture.handle.stop()
        });
    let _ = std::fs::remove_file(&path);
    status_from(outcome)
}

pub fn probe_screen_recording() -> PermissionStatus {
    match check_screen_recording_permission() {
        Ok(true) => PermissionStatus::Granted,
        Ok(false) | Err(_) => PermissionStatus::Denied,
    }
}

pub fn probe_accessibility() -> PermissionStatus {
    status_from(dictation::probe_accessibility())
}

fn status_from<T>(result: Result<T, AppError>) -> PermissionStatus {
    match result {
        Ok(_) => PermissionStatus::Granted,
        Err(_) => PermissionStatus::Denied,
    }
}

fn probe_temp_path(stem: &str, extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{stem}-{}.{extension}",
        crate::audio::capture::current_time_ms().unwrap_or_default()
    ))
}

/// Panes accepted by [`open_system_settings_pane`], matching macOS's
/// `x-apple.systempreferences` Privacy sub-panes.
const VALID_SETTINGS_PANES: &[&str] = &["Microphone", "ScreenCapture", "Accessibility"];

/// Deep-links to the relevant Privacy & Security pane in System Settings, for
/// when a permission has already been denied and only the user can undo that
/// (macOS does not re-prompt after an explicit deny).
pub fn open_system_settings_pane(pane: &str) -> Result<(), AppError> {
    if !VALID_SETTINGS_PANES.contains(&pane) {
        return Err(AppError {
            code: "permissions_unknown_pane".to_string(),
            message: format!("Unknown System Settings pane: {pane}"),
            details: None,
        });
    }
    Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}"
        ))
        .status()
        .map_err(|error| AppError {
            code: "permissions_open_settings_failed".to_string(),
            message: "Could not open System Settings.".to_string(),
            details: Some(error.to_string()),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::audio::capture::{ActiveCapture, CaptureSessionHandle, CaptureSummary};
    use crate::audio::domain::AudioDevice;

    struct GrantedMicBackend;
    struct DeniedMicBackend;

    struct StubSession;
    impl CaptureSessionHandle for StubSession {
        fn stop(self: Box<Self>) -> Result<CaptureSummary, AppError> {
            Ok(CaptureSummary {
                sample_count: 100,
                dropped_sample_count: 0,
                stream_error: None,
            })
        }
    }

    impl AudioCaptureBackend for GrantedMicBackend {
        fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
            Ok(Vec::new())
        }
        fn start_recording(
            &self,
            file_path: PathBuf,
            _device_id: Option<String>,
        ) -> Result<ActiveCapture, AppError> {
            Ok(ActiveCapture {
                file_path,
                sample_rate_hz: 16_000,
                started_at_ms: 0,
                level: Default::default(),
                handle: Box::new(StubSession),
            })
        }
    }

    impl AudioCaptureBackend for DeniedMicBackend {
        fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
            Ok(Vec::new())
        }
        fn start_recording(
            &self,
            _file_path: PathBuf,
            _device_id: Option<String>,
        ) -> Result<ActiveCapture, AppError> {
            Err(AppError {
                code: "audio_input_config_unavailable".to_string(),
                message: "Microphone access denied.".to_string(),
                details: None,
            })
        }
    }

    #[test]
    fn microphone_probe_reports_granted_when_capture_starts() {
        assert_eq!(
            probe_microphone_with(&GrantedMicBackend),
            PermissionStatus::Granted
        );
    }

    #[test]
    fn microphone_probe_reports_denied_when_capture_fails() {
        assert_eq!(
            probe_microphone_with(&DeniedMicBackend),
            PermissionStatus::Denied
        );
    }

    #[test]
    fn open_system_settings_pane_rejects_unknown_panes() {
        let error = open_system_settings_pane("NotAPane").expect_err("unknown pane is rejected");
        assert_eq!(error.code, "permissions_unknown_pane");
    }
}
