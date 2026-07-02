//! Text injection: drop dictated text into whatever app currently has focus by
//! placing it on the clipboard and synthesising a Cmd+V paste. This needs the
//! Accessibility permission (System Events keystroke); without it macOS blocks
//! the paste and `osascript` reports an error we surface with a stable code.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::domain::AppError;

/// Injects `text` into the focused app. Blank text is a no-op so a silent
/// dictation doesn't clear the clipboard or fire a stray paste.
pub fn inject_text(text: &str) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    set_clipboard(text)?;
    paste_with_keystroke()
}

/// Copies `text` to the macOS clipboard via `pbcopy`.
fn set_clipboard(text: &str) -> Result<(), AppError> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| AppError {
            code: "dictation_clipboard_failed".to_string(),
            message: "Could not start pbcopy to set the clipboard.".to_string(),
            details: Some(error.to_string()),
        })?;

    child
        .stdin
        .take()
        .ok_or_else(|| AppError {
            code: "dictation_clipboard_failed".to_string(),
            message: "Could not write to the clipboard.".to_string(),
            details: None,
        })?
        .write_all(text.as_bytes())
        .map_err(|error| AppError {
            code: "dictation_clipboard_failed".to_string(),
            message: "Could not send text to the clipboard.".to_string(),
            details: Some(error.to_string()),
        })?;

    let status = child.wait().map_err(|error| AppError {
        code: "dictation_clipboard_failed".to_string(),
        message: "pbcopy did not finish.".to_string(),
        details: Some(error.to_string()),
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(AppError {
            code: "dictation_clipboard_failed".to_string(),
            message: "pbcopy failed to set the clipboard.".to_string(),
            details: status.code().map(|code| format!("exit_code={code}")),
        })
    }
}

/// Synthesises a Cmd+V paste into the focused app via System Events. A failure
/// here usually means the Accessibility permission has not been granted.
fn paste_with_keystroke() -> Result<(), AppError> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
        .output()
        .map_err(|error| AppError {
            code: "dictation_paste_failed".to_string(),
            message: "Could not start osascript to paste the dictation.".to_string(),
            details: Some(error.to_string()),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    // When the app lacks the Accessibility permission, System Events refuses the
    // keystroke. The wording varies by macOS version: "not allowed to send
    // keystrokes" (error 1002) or "not allowed assistive access" (error -1719).
    // Flag both as the permission case so the UI can prompt the user to grant it.
    let code = if is_accessibility_denied(&stderr) {
        "dictation_accessibility_permission_required"
    } else {
        "dictation_paste_failed"
    };
    Err(AppError {
        code: code.to_string(),
        message: "Could not paste the dictation into the focused app.".to_string(),
        details: if stderr.is_empty() { None } else { Some(stderr) },
    })
}

/// Checks the Accessibility permission via `System Events`'s `UI elements
/// enabled` property — a read-only query with no clipboard writes, no
/// synthesized keystrokes, nothing visible in the focused app. Unlike
/// Microphone/Screen Recording, macOS does not show an automatic system
/// prompt (or add the app to the Accessibility list) for this indirect,
/// AppleScript-mediated path — the user has to add the app themselves via
/// System Settings. This only reports the current status; it does not
/// trigger that dialog.
///
/// Deliberately does *not* use `get name of first process whose frontmost is
/// true`: that query only requires the Automation permission (Scribe talking
/// to System Events), not real Accessibility, so it would report "granted"
/// even when Accessibility itself is missing.
pub fn probe_accessibility() -> Result<(), AppError> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to UI elements enabled"#)
        .output()
        .map_err(|error| AppError {
            code: "dictation_accessibility_probe_failed".to_string(),
            message: "Could not start osascript to check the Accessibility permission.".to_string(),
            details: Some(error.to_string()),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && stdout == "true" {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let code = if stdout == "false" || is_accessibility_denied(&stderr) {
        "dictation_accessibility_permission_required"
    } else {
        "dictation_accessibility_probe_failed"
    };
    Err(AppError {
        code: code.to_string(),
        message: "The Accessibility permission has not been granted.".to_string(),
        details: if stderr.is_empty() { None } else { Some(stderr) },
    })
}

/// Recognises the System Events errors that mean the Accessibility permission
/// has not been granted, across the macOS version wording variants.
fn is_accessibility_denied(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("not allowed to send keystrokes")
        || stderr.contains("assistive access")
        || stderr.contains("1002")
        || stderr.contains("-1719")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_text_is_a_no_op() {
        // No clipboard write and no paste keystroke should fire for blank input.
        inject_text("   \n\t").expect("blank input is a no-op");
    }

    #[test]
    fn keystroke_denied_message_is_classified_as_permission() {
        assert!(is_accessibility_denied(
            "execution error: System Events got an error: osascript is not allowed to send keystrokes. (1002)"
        ));
    }

    #[test]
    fn assistive_access_message_is_classified_as_permission() {
        assert!(is_accessibility_denied(
            "System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
    }

    #[test]
    fn unrelated_error_is_not_a_permission_problem() {
        assert!(!is_accessibility_denied("some other applescript failure"));
    }
}
