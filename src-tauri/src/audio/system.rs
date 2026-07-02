use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::domain::AppError;

use super::audio_error;

const SYSTEM_AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
const HELPER_STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub trait SystemAudioCaptureBackend: Send + Sync {
    fn start_capture(&self, file_path: PathBuf) -> Result<ActiveSystemAudioCapture, AppError>;
}

pub trait SystemAudioSessionHandle: Send {
    fn stop(self: Box<Self>) -> SystemAudioCaptureSummary;
}

pub struct ActiveSystemAudioCapture {
    pub file_path: PathBuf,
    pub sample_rate_hz: u32,
    pub handle: Box<dyn SystemAudioSessionHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAudioCaptureSummary {
    pub byte_size: Option<u64>,
    pub stream_error: Option<String>,
}

#[derive(Default)]
pub struct ScreenCaptureKitSystemAudioBackend;

impl ScreenCaptureKitSystemAudioBackend {
    pub fn new() -> Self {
        Self
    }
}

impl SystemAudioCaptureBackend for ScreenCaptureKitSystemAudioBackend {
    fn start_capture(&self, file_path: PathBuf) -> Result<ActiveSystemAudioCapture, AppError> {
        let helper_path = system_audio_helper_path()?;
        let child = Command::new(&helper_path)
            .arg(&file_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                audio_error(
                    "system_audio_capture_start_failed",
                    "Could not start ScreenCaptureKit system audio capture.",
                    Some(format!("helper={}, error={error}", helper_path.display())),
                )
            })?;

        Ok(ActiveSystemAudioCapture {
            file_path,
            sample_rate_hz: SYSTEM_AUDIO_SAMPLE_RATE_HZ,
            handle: Box::new(ScreenCaptureKitSystemAudioSession { child }),
        })
    }
}

struct ScreenCaptureKitSystemAudioSession {
    child: Child,
}

impl SystemAudioSessionHandle for ScreenCaptureKitSystemAudioSession {
    fn stop(mut self: Box<Self>) -> SystemAudioCaptureSummary {
        let mut signal_error = None;
        if let Some(mut stdin) = self.child.stdin.take() {
            if let Err(error) = writeln!(stdin, "stop") {
                signal_error = Some(format!(
                    "Could not signal system audio capture to stop: {error}"
                ));
            }
        }

        let started_waiting = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return helper_exit_summary(&mut self.child, status, signal_error);
                }
                Ok(None) if started_waiting.elapsed() < HELPER_STOP_TIMEOUT => {
                    thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let kill_error = self.child.kill().err().map(|error| error.to_string());
                    return SystemAudioCaptureSummary {
                        byte_size: None,
                        stream_error: Some(match kill_error {
                            Some(error) => format!(
                                "Timed out waiting for system audio capture to stop, then kill failed: {error}"
                            ),
                            None => "Timed out waiting for system audio capture to stop.".to_string(),
                        }),
                    };
                }
                Err(error) => {
                    return SystemAudioCaptureSummary {
                        byte_size: None,
                        stream_error: Some(format!(
                            "Could not wait for system audio helper: {error}"
                        )),
                    };
                }
            }
        }
    }
}

fn helper_exit_summary(
    child: &mut Child,
    status: ExitStatus,
    signal_error: Option<String>,
) -> SystemAudioCaptureSummary {
    let stderr = read_child_stderr(child);
    let stream_error = if status.success() {
        signal_error
    } else {
        Some(match (signal_error, stderr) {
            (Some(signal_error), Some(stderr)) => {
                format!("{signal_error}; system audio helper exited with {status}: {stderr}")
            }
            (Some(signal_error), None) => {
                format!("{signal_error}; system audio helper exited with {status}.")
            }
            (None, Some(stderr)) => format!("System audio helper exited with {status}: {stderr}"),
            (None, None) => format!("System audio helper exited with {status}."),
        })
    };

    SystemAudioCaptureSummary {
        byte_size: None,
        stream_error,
    }
}

fn read_child_stderr(child: &mut Child) -> Option<String> {
    let mut stderr = child.stderr.take()?;
    let mut output = String::new();
    stderr.read_to_string(&mut output).ok()?;
    let trimmed = output.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn system_audio_helper_path() -> Result<PathBuf, AppError> {
    let helper_name =
        option_env!("SCRIBE_SYSTEM_AUDIO_HELPER_NAME").ok_or_else(helper_unavailable)?;
    let target =
        option_env!("SCRIBE_SYSTEM_AUDIO_HELPER_TARGET").ok_or_else(helper_unavailable)?;

    let development_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("{helper_name}-{target}"));
    if development_path.is_file() {
        return Ok(development_path);
    }

    let executable_path = std::env::current_exe().map_err(|error| {
        audio_error(
            "system_audio_helper_unavailable",
            "Could not locate the current Scribe executable to resolve the bundled system audio helper.",
            Some(error.to_string()),
        )
    })?;
    let bundled_path = executable_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(helper_name);

    if bundled_path.is_file() {
        Ok(bundled_path)
    } else {
        Err(audio_error(
            "system_audio_helper_unavailable",
            "ScreenCaptureKit system audio helper is not bundled for this platform.",
            Some(format!(
                "checked={}, {}",
                development_path.display(),
                bundled_path.display()
            )),
        ))
    }
}

fn helper_unavailable() -> AppError {
    audio_error(
        "system_audio_helper_unavailable",
        "ScreenCaptureKit system audio helper is not available for this build.",
        None,
    )
}

/// Read-only Screen Recording status check: runs the helper's `--check-only`
/// mode, which calls only `CGPreflightScreenCaptureAccess()` and never
/// `CGRequestScreenCaptureAccess()`. The latter triggers the system prompt
/// and returns immediately, before the user answers it -- calling it from an
/// automatic background check (e.g. on every app launch) would silently
/// re-fire the system dialog every time regardless of the user's previous
/// answer. Real capture attempts (recording a meeting, or an explicit "Grant"
/// action) still go through the normal request-capable path.
pub fn check_screen_recording_permission() -> Result<bool, AppError> {
    let helper_path = system_audio_helper_path()?;
    let status = Command::new(&helper_path)
        .arg("--check-only")
        .status()
        .map_err(|error| {
            audio_error(
                "system_audio_permission_check_failed",
                "Could not run the ScreenCaptureKit permission check.",
                Some(format!("helper={}, error={error}", helper_path.display())),
            )
        })?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_audio_helper_path_resolves_development_sidecar() {
        if cfg!(target_os = "macos") {
            let path = system_audio_helper_path().expect("macOS build includes helper path");
            assert!(path.ends_with(format!(
                "{}-{}",
                env!("SCRIBE_SYSTEM_AUDIO_HELPER_NAME"),
                env!("SCRIBE_SYSTEM_AUDIO_HELPER_TARGET")
            )));
        }
    }

    #[test]
    fn check_screen_recording_permission_never_hangs_or_errors() {
        // Regression test for the bug where the onboarding's automatic status
        // check called CGRequestScreenCaptureAccess() (which prompts) instead
        // of CGPreflightScreenCaptureAccess() (read-only) -- this must return
        // promptly with a plain bool, whatever the current OS grant state is,
        // and never block on user interaction.
        if cfg!(target_os = "macos") {
            let result = check_screen_recording_permission();
            assert!(result.is_ok(), "check-only mode must not error: {result:?}");
        }
    }
}
