//! Polish-selection: grab whatever text is selected in the focused app, clean
//! it up the same way dictation is (Apple Intelligence polish, unconditional —
//! this is an explicit user action, not gated by the dictation-polish
//! setting), and paste the result back in place. Triggered by its own global
//! hotkey; no pill or focus-handback dance needed, since the trigger never
//! touches Scribe's own UI.

use std::time::{Duration, Instant};

use crate::domain::AppError;

use super::inject::{copy_selection, inject_text, read_clipboard};
use super::polish_text;

/// How often to re-check the clipboard while waiting for a synthesised
/// Cmd+C to land.
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Upper bound on how long to wait for the clipboard to change after Cmd+C
/// before concluding nothing was selected.
const COPY_MAX_WAIT: Duration = Duration::from_millis(500);

/// Result of a polish-selection attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum SelectionPolishOutcome {
    /// The selection was polished and pasted back successfully.
    Applied,
    /// Nothing was selected (the clipboard didn't change after Cmd+C).
    NoSelection,
    /// Polishing succeeded but the paste-back failed; the polished text was
    /// still left on the clipboard (by `inject_text`'s own clipboard write),
    /// so the user can paste it manually.
    PasteFailed,
}

/// Copies the focused app's current selection, polishes it, and pastes the
/// result back in place.
pub fn polish_selection() -> Result<SelectionPolishOutcome, AppError> {
    let before = read_clipboard().unwrap_or_default();
    eprintln!(
        "polish_selection: clipboard before Cmd+C has {} chars",
        before.chars().count()
    );

    copy_selection()?;
    eprintln!("polish_selection: Cmd+C keystroke sent successfully");

    let selected = wait_for_clipboard_change(&before)?;
    eprintln!(
        "polish_selection: clipboard after Cmd+C has {} chars (changed={})",
        selected.chars().count(),
        selected != before
    );

    if selected.trim().is_empty() || selected == before {
        return Ok(SelectionPolishOutcome::NoSelection);
    }

    let polished = polish_text(&selected)?;
    if polished.trim().is_empty() {
        return Ok(SelectionPolishOutcome::NoSelection);
    }

    match inject_text(&polished) {
        Ok(()) => Ok(SelectionPolishOutcome::Applied),
        Err(_) => Ok(SelectionPolishOutcome::PasteFailed),
    }
}

/// Polls the clipboard until it differs from `before` or `COPY_MAX_WAIT`
/// elapses. `osascript` returns as soon as it has posted the synthetic
/// keystroke, not once the focused app has actually handled it — the app's
/// `copy:` responder still has to run and write the pasteboard, which can
/// take a while longer, especially when the focused field is in a WKWebView
/// (the keystroke has to cross a process boundary first). A single fixed
/// sleep is a race that can misreport a real selection as "nothing
/// selected"; polling waits only as long as it actually needs to.
fn wait_for_clipboard_change(before: &str) -> Result<String, AppError> {
    let deadline = Instant::now() + COPY_MAX_WAIT;
    loop {
        let current = read_clipboard()?;
        if current != before || Instant::now() >= deadline {
            return Ok(current);
        }
        std::thread::sleep(COPY_POLL_INTERVAL);
    }
}
