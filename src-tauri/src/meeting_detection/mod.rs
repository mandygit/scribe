//! Detects a live Microsoft Teams call and drives the "Record this meeting?"
//! prompt, mirroring Notion's meeting-note popup. All privileged window
//! inspection happens in a Swift sidecar (`native/meeting-detector/main.swift`)
//! per the codebase's established pattern (see ADR-001) rather than raw Rust
//! FFI; this module owns the sidecar's process lifecycle and the pure
//! state machine that turns its stdout lines into prompt show/hide actions.
//! See `docs/decisions/adr-004-teams-meeting-detection.md` for the full
//! rationale, including why detection fires on "live call", not "pre-join".

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::JoinHandle,
};

use crate::domain::AppError;

/// One transition reported by the sidecar, or a user action that also
/// affects the prompt's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorEvent {
    /// The sidecar reported a transition into a live call.
    CallStarted,
    /// The sidecar reported a transition out of a call (or Teams quit).
    CallEnded,
    /// The user clicked "Dismiss" on the prompt.
    Dismissed,
}

impl DetectorEvent {
    /// Parses one line of sidecar stdout. Anything unrecognised (a future
    /// sidecar build emitting something new) is ignored rather than treated
    /// as a state change.
    pub fn from_sidecar_line(line: &str) -> Option<Self> {
        match line.trim() {
            "IN_CALL" => Some(Self::CallStarted),
            "NOT_IN_CALL" => Some(Self::CallEnded),
            _ => None,
        }
    }
}

/// The call-prompt state machine's state. `Dismissed` covers both "the user
/// said no" and "a recording was already active when the call started" --
/// both suppress the prompt for the remainder of the same call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallPromptState {
    #[default]
    NotInCall,
    InCall,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptAction {
    None,
    ShowPrompt,
    HidePrompt,
}

/// Advances the call-prompt state machine by one event. Pure and
/// side-effect-free so it can be unit tested against a fixed sequence of
/// sidecar lines without a real sidecar process.
///
/// `recording_already_active` only matters for the `NotInCall -> CallStarted`
/// transition: if a recording is already running -- started manually, or
/// from an earlier prompt this same call -- the prompt must never show for
/// that call.
pub fn advance(
    state: CallPromptState,
    event: DetectorEvent,
    recording_already_active: bool,
) -> (CallPromptState, PromptAction) {
    use CallPromptState::{Dismissed, InCall, NotInCall};
    use DetectorEvent::{CallEnded, CallStarted, Dismissed as DismissedEvent};

    match (state, event) {
        (NotInCall, CallStarted) => {
            if recording_already_active {
                (Dismissed, PromptAction::None)
            } else {
                (InCall, PromptAction::ShowPrompt)
            }
        }
        (InCall, CallEnded) => (NotInCall, PromptAction::HidePrompt),
        (InCall, DismissedEvent) => (Dismissed, PromptAction::None),
        (Dismissed, CallEnded) => (NotInCall, PromptAction::None),
        // Anything else (e.g. a stray CallStarted while already InCall) is a
        // no-op: the sidecar only emits on its own transitions, so a
        // duplicate signals drift, not a new call.
        (state, _) => (state, PromptAction::None),
    }
}

fn meeting_detector_helper_unavailable() -> AppError {
    AppError {
        code: "meeting_detector_helper_unavailable".to_string(),
        message: "The meeting detector helper was not built for this platform.".to_string(),
        details: None,
    }
}

/// Resolves the bundled meeting-detector Swift sidecar, preferring the dev
/// build under `binaries/` and falling back to the path next to the app
/// executable -- the same resolution `system_audio_helper_path` and
/// `polish_helper_path` use.
pub fn meeting_detector_helper_path() -> Result<PathBuf, AppError> {
    let helper_name = option_env!("SCRIBE_MEETING_DETECTOR_HELPER_NAME")
        .ok_or_else(meeting_detector_helper_unavailable)?;
    let target = option_env!("SCRIBE_MEETING_DETECTOR_HELPER_TARGET")
        .ok_or_else(meeting_detector_helper_unavailable)?;

    let development_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("{helper_name}-{target}"));
    if development_path.is_file() {
        return Ok(development_path);
    }

    let executable_path = std::env::current_exe().map_err(|error| AppError {
        code: "meeting_detector_helper_unavailable".to_string(),
        message: "Could not locate the app executable to resolve the meeting detector helper."
            .to_string(),
        details: Some(error.to_string()),
    })?;
    let bundled_path = executable_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(helper_name);
    if bundled_path.is_file() {
        Ok(bundled_path)
    } else {
        Err(meeting_detector_helper_unavailable())
    }
}

/// Owns the meeting-detector sidecar's process and its stdout-reading
/// thread. Carries no knowledge of Tauri or the prompt state machine --
/// callers supply a plain `on_line` callback, matching this codebase's
/// pattern of keeping domain modules Tauri-agnostic (see `dictation/`).
#[derive(Default)]
pub struct TeamsCallDetector {
    child: Option<Child>,
    thread: Option<JoinHandle<()>>,
}

impl TeamsCallDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    /// Starts the sidecar and a background thread that calls `on_line` for
    /// every (already-trimmed) stdout line it prints. No-op if already
    /// running, so callers can call this unconditionally when a setting
    /// turns on.
    pub fn start(&mut self, on_line: impl Fn(String) + Send + 'static) -> Result<(), AppError> {
        if self.child.is_some() {
            return Ok(());
        }

        let helper_path = meeting_detector_helper_path()?;
        let mut child = Command::new(&helper_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| AppError {
                code: "meeting_detector_start_failed".to_string(),
                message: "Could not start the meeting detector helper.".to_string(),
                details: Some(format!("helper={}, error={error}", helper_path.display())),
            })?;

        let stdout = child.stdout.take().ok_or_else(|| AppError {
            code: "meeting_detector_start_failed".to_string(),
            message: "Meeting detector helper did not provide stdout.".to_string(),
            details: None,
        })?;

        let thread = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                on_line(line);
            }
        });

        self.child = Some(child);
        self.thread = Some(thread);
        Ok(())
    }

    /// Stops the sidecar (if running) and waits for its reader thread to
    /// exit. Best-effort: a failed kill is logged, never propagated --
    /// stopping detection must never block a settings change or app quit.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Err(error) = child.kill() {
                eprintln!("meeting detector: failed to stop helper: {error}");
            }
            let _ = child.wait();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TeamsCallDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_started_shows_prompt_when_idle() {
        let (state, action) = advance(CallPromptState::NotInCall, DetectorEvent::CallStarted, false);
        assert_eq!(state, CallPromptState::InCall);
        assert_eq!(action, PromptAction::ShowPrompt);
    }

    #[test]
    fn call_started_does_not_prompt_when_already_recording() {
        let (state, action) = advance(CallPromptState::NotInCall, DetectorEvent::CallStarted, true);
        assert_eq!(state, CallPromptState::Dismissed);
        assert_eq!(action, PromptAction::None);
    }

    #[test]
    fn call_ended_hides_prompt_when_shown() {
        let (state, action) = advance(CallPromptState::InCall, DetectorEvent::CallEnded, false);
        assert_eq!(state, CallPromptState::NotInCall);
        assert_eq!(action, PromptAction::HidePrompt);
    }

    #[test]
    fn dismiss_suppresses_further_prompts_until_call_ends() {
        let (state, action) = advance(CallPromptState::InCall, DetectorEvent::Dismissed, false);
        assert_eq!(state, CallPromptState::Dismissed);
        assert_eq!(action, PromptAction::None);

        // The call continuing (no further sidecar lines until it ends) must
        // never re-show the prompt.
        let (state, action) = advance(state, DetectorEvent::Dismissed, false);
        assert_eq!(state, CallPromptState::Dismissed);
        assert_eq!(action, PromptAction::None);
    }

    #[test]
    fn call_ending_after_dismiss_resets_without_hiding() {
        let dismissed = CallPromptState::Dismissed;
        let (state, action) = advance(dismissed, DetectorEvent::CallEnded, false);
        assert_eq!(state, CallPromptState::NotInCall);
        // Nothing to hide -- the prompt was never shown for this call.
        assert_eq!(action, PromptAction::None);
    }

    #[test]
    fn full_call_lifecycle_prompts_exactly_once() {
        let mut state = CallPromptState::NotInCall;
        let mut shows = 0;
        let mut hides = 0;
        let lines = ["IN_CALL", "IN_CALL", "NOT_IN_CALL", "IN_CALL", "NOT_IN_CALL"];
        for line in lines {
            let event = DetectorEvent::from_sidecar_line(line).expect("valid line");
            let (next_state, action) = advance(state, event, false);
            state = next_state;
            match action {
                PromptAction::ShowPrompt => shows += 1,
                PromptAction::HidePrompt => hides += 1,
                PromptAction::None => {}
            }
        }
        assert_eq!(shows, 2, "one prompt per distinct call, not per line");
        assert_eq!(hides, 2);
        assert_eq!(state, CallPromptState::NotInCall);
    }

    #[test]
    fn unrecognised_line_is_ignored() {
        assert_eq!(DetectorEvent::from_sidecar_line("garbage"), None);
        assert_eq!(DetectorEvent::from_sidecar_line(""), None);
    }
}
