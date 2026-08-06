//! Text injection: drop dictated text into whatever app currently has focus by
//! placing it on the clipboard, synthesising a Cmd+V paste, and then restoring
//! the clipboard's previous contents. Also used in
//! reverse by polish-selection: Cmd+C copies the focused app's current
//! selection out to the clipboard so it can be read back and polished. Both
//! need the Accessibility permission (System Events keystroke); without it
//! macOS blocks the keystroke and `osascript` reports an error we surface
//! with a stable code.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSPasteboard, NSPasteboardItem, NSPasteboardWriting,
    NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{NSArray, NSData, NSString};

use crate::domain::AppError;

/// The process id of whichever app was frontmost just before dictation
/// started, i.e. the paste target.
pub type TargetAppPid = libc::pid_t;

/// How often the background tracker in [`spawn_frontmost_app_tracker`]
/// polls `NSWorkspace.frontmostApplication`.
const FRONTMOST_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// The most recent frontmost app that was NOT Scribe itself, kept fresh by
/// [`spawn_frontmost_app_tracker`]. Read by [`last_external_frontmost_app`].
static LAST_EXTERNAL_FRONTMOST: StdMutex<Option<TargetAppPid>> = StdMutex::new(None);

/// A single point-in-time read of `NSWorkspace.frontmostApplication`'s pid.
fn frontmost_app_pid() -> Option<TargetAppPid> {
    Some(NSWorkspace::sharedWorkspace().frontmostApplication()?.processIdentifier())
}

/// Starts a background thread that keeps [`LAST_EXTERNAL_FRONTMOST`] fresh
/// for the lifetime of the app. Must be called once at startup.
///
/// A point-in-time "what's frontmost right now" query, taken only when
/// dictation starts, is too late for the pill's click-to-start path: clicking
/// the pill's mic button routes through Scribe's own WKWebView, which makes
/// Scribe itself `NSWorkspace.frontmostApplication` by the time the resulting
/// IPC call reaches Rust (confirmed live 2026-08-06 — the point-in-time
/// version captured pid/name of "Scribe" itself as the paste target, every
/// time dictation was started by clicking the pill). Continuously tracking
/// the last *external* (non-Scribe) frontmost app sidesteps this: whatever
/// the user was in right before touching the pill is still the most recent
/// value recorded, regardless of what Scribe's own click momentarily did to
/// activation. The hotkey-triggered start path never touches Scribe's own
/// windows at all, so it's unaffected either way — the tracked value is
/// already correct there too.
///
/// Polling (like the pill's hover detection) rather than an
/// `NSWorkspaceDidActivateApplicationNotification` observer, for the same
/// reason: it avoids Cocoa notification/observer plumbing for a value that's
/// cheap to re-read a few times a second.
pub fn spawn_frontmost_app_tracker() {
    let own_pid = std::process::id() as TargetAppPid;
    std::thread::spawn(move || loop {
        if let Some(pid) = frontmost_app_pid() {
            if pid != own_pid {
                if let Ok(mut last) = LAST_EXTERNAL_FRONTMOST.lock() {
                    *last = Some(pid);
                }
            }
        }
        std::thread::sleep(FRONTMOST_POLL_INTERVAL);
    });
}

/// The most recently observed non-Scribe frontmost app, to be handed back to
/// [`reactivate`] right before pasting. Returns `None` if none has been
/// observed yet (e.g. dictation started within the first poll tick after
/// launch), in which case paste falls back to the prior implicit
/// hide-the-pill-and-hope behavior.
pub fn capture_frontmost_app() -> Option<TargetAppPid> {
    LAST_EXTERNAL_FRONTMOST.lock().ok().and_then(|guard| *guard)
}

/// Explicitly reactivates the app captured by [`capture_frontmost_app`] so
/// the synthesised Cmd+V has a real target to land in, instead of relying on
/// key focus implicitly falling back to "whatever window was previously
/// active" after hiding the pill — a race that gets less reliable across
/// multiple displays/Spaces, since there is more than one place focus could
/// land. A no-op (returns `false`) if the app has since quit.
pub fn reactivate(pid: TargetAppPid) -> bool {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return false;
    };
    app.activateWithOptions(NSApplicationActivationOptions::empty())
}

/// How long to wait after the Cmd+V keystroke before restoring the previous
/// clipboard. The keystroke call returning only means the event was posted;
/// the target app reads the clipboard asynchronously, and restoring too early
/// would paste the old contents instead of the transcript.
const PASTE_SETTLE: Duration = Duration::from_millis(300);

/// How long to wait after [`reactivate`] succeeds before firing the paste
/// keystroke, so the target app has actually become key by the time it does.
const TARGET_ACTIVATION_SETTLE: Duration = Duration::from_millis(100);

/// Every flavor of every item on the pasteboard, as raw bytes keyed by UTI.
/// Full fidelity — screenshots, rich text, and multi-item copies all survive.
type ClipboardSnapshot = Vec<Vec<(String, Vec<u8>)>>;

/// Injects `text` into `target` (the app that was frontmost when dictation
/// started, from [`capture_frontmost_app`]). Blank text is a no-op so a
/// silent dictation doesn't clear the clipboard or fire a stray paste.
///
/// The previous clipboard contents — including images and other non-text
/// flavors — are saved first and put back once the paste has landed, so
/// dictating doesn't eat whatever the user last copied.
pub fn inject_text(text: &str, target: Option<TargetAppPid>) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    // Explicitly reactivate the app that was frontmost when dictation
    // started, rather than trusting key focus to fall back there on its own
    // once the pill hides — that implicit handoff gets less reliable the
    // more displays/Spaces are in play. Best-effort: if the app already quit
    // or there was no captured target, fall through to the caller's existing
    // hide-the-pill-and-settle behavior.
    if let Some(pid) = target {
        if reactivate(pid) {
            // activateWithOptions returning true only means the request was
            // accepted; the window server still processes the actual key
            // window handoff asynchronously. Same order of magnitude as the
            // settle this replaces (the old hide-pill sleep).
            std::thread::sleep(TARGET_ACTIVATION_SETTLE);
        }
    }
    // Best-effort save: an unreadable pasteboard just means there is nothing
    // to put back.
    let saved = snapshot_clipboard();
    set_clipboard(text)?;
    // On paste failure, return with the transcript still on the clipboard so
    // the user can paste it by hand instead of losing the dictation outright.
    send_cmd_keystroke("v", "dictation_paste_failed")?;
    if !saved.is_empty() {
        std::thread::sleep(PASTE_SETTLE);
        // Best-effort restore: the paste already succeeded, and failing the
        // whole dictation over a restore hiccup would be worse than leaving
        // the transcript on the clipboard.
        restore_clipboard(&saved);
    }
    Ok(())
}

/// Reads every item and flavor off the general pasteboard. Lazily-promised
/// flavors are materialised by `dataForType`; anything unreadable is skipped.
fn snapshot_clipboard() -> ClipboardSnapshot {
    let pasteboard = NSPasteboard::generalPasteboard();
    let Some(items) = pasteboard.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            item.types()
                .iter()
                .filter_map(|flavor| {
                    item.dataForType(&flavor)
                        .map(|data| (flavor.to_string(), data.to_vec()))
                })
                .collect()
        })
        .collect()
}

/// Replaces the general pasteboard's contents with a previously captured
/// snapshot. Returns whether the write was accepted.
fn restore_clipboard(snapshot: &ClipboardSnapshot) -> bool {
    let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
        .iter()
        .map(|flavors| {
            let item = NSPasteboardItem::new();
            for (flavor, bytes) in flavors {
                item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(flavor));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.writeObjects(&NSArray::from_retained_slice(&objects))
}

/// Sends Cmd+C to the focused app via System Events, so its current selection
/// (if any) lands on the clipboard. Used by polish-selection to read out
/// whatever the user has highlighted.
pub fn copy_selection() -> Result<(), AppError> {
    send_cmd_keystroke("c", "polish_selection_copy_failed")
}

/// Reads the current macOS clipboard contents via `pbpaste`.
pub fn read_clipboard() -> Result<String, AppError> {
    let output = Command::new("pbpaste").output().map_err(|error| AppError {
        code: "polish_selection_clipboard_read_failed".to_string(),
        message: "Could not start pbpaste to read the clipboard.".to_string(),
        details: Some(error.to_string()),
    })?;

    if !output.status.success() {
        return Err(AppError {
            code: "polish_selection_clipboard_read_failed".to_string(),
            message: "pbpaste failed to read the clipboard.".to_string(),
            details: output.status.code().map(|code| format!("exit_code={code}")),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

/// Synthesises a Cmd+`key` keystroke into the focused app via System Events —
/// `"v"` to paste, `"c"` to copy. A failure here usually means the
/// Accessibility permission has not been granted; `failure_code` is returned
/// for any other failure so callers can tell the two apart.
fn send_cmd_keystroke(key: &str, failure_code: &str) -> Result<(), AppError> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            r#"tell application "System Events" to keystroke "{key}" using command down"#
        ))
        .output()
        .map_err(|error| AppError {
            code: failure_code.to_string(),
            message: format!(
                "Could not start osascript to send Cmd+{}.",
                key.to_uppercase()
            ),
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
        failure_code
    };
    Err(AppError {
        code: code.to_string(),
        message: format!(
            "Could not send Cmd+{} to the focused app.",
            key.to_uppercase()
        ),
        details: if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        },
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
        details: if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        },
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
        inject_text("   \n\t", None).expect("blank input is a no-op");
    }

    /// Minimal valid 1x1 transparent PNG, standing in for a screenshot.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// Flavor order within an item is not guaranteed across the pasteboard
    /// round-trip, so compare snapshots with each item's flavors sorted.
    fn sorted(mut snapshot: ClipboardSnapshot) -> ClipboardSnapshot {
        for item in &mut snapshot {
            item.sort();
        }
        snapshot
    }

    #[test]
    #[ignore = "mutates the real macOS general pasteboard"]
    fn snapshot_restore_round_trips_non_text_flavors() {
        let original = snapshot_clipboard();

        let planted: ClipboardSnapshot = vec![vec![
            ("public.png".to_string(), TINY_PNG.to_vec()),
            ("public.utf8-plain-text".to_string(), b"round-trip".to_vec()),
        ]];
        assert!(restore_clipboard(&planted), "planting test flavors failed");
        assert_eq!(sorted(snapshot_clipboard()), sorted(planted));

        // Put the user's clipboard back the way we found it.
        if !original.is_empty() {
            assert!(restore_clipboard(&original), "restoring original failed");
        }
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
