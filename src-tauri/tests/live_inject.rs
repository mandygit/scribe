//! Live text-injection checks against real apps: `inject_text` must land the
//! dictation in a focused text field, and must refuse to fire a paste at an
//! app that has nowhere to put it.
//!
//! Ignored by default because they need the Accessibility permission and drive
//! the GUI. Run them with:
//!
//! ```sh
//! cargo test --test live_inject -- --ignored --nocapture
//! ```

use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use scribe_lib::dictation::{inject_text, InjectOutcome};

fn osascript(script: &str) -> String {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .expect("osascript runs");
    assert!(
        output.status.success(),
        "osascript failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The pid of a running app, as `inject_text` expects its target.
fn pid_of(app: &str) -> i32 {
    osascript(&format!(
        "tell application \"System Events\" to get unix id of process \"{app}\""
    ))
    .parse()
    .expect("System Events reports a numeric pid")
}

fn clipboard() -> String {
    String::from_utf8_lossy(
        &Command::new("pbpaste")
            .output()
            .expect("pbpaste runs")
            .stdout,
    )
    .to_string()
}

#[test]
#[ignore = "requires Accessibility permission and drives TextEdit"]
fn live_inject_into_textedit() {
    let marker = format!("scribe-inject-{}", std::process::id());

    // Focus a fresh TextEdit document as the injection target.
    osascript("tell application \"TextEdit\" to activate");
    osascript("tell application \"TextEdit\" to make new document");
    sleep(Duration::from_millis(800));

    let outcome = inject_text(&marker, Some(pid_of("TextEdit")))
        .expect("inject_text succeeds with Accessibility granted");
    sleep(Duration::from_millis(500));

    let contents = osascript("tell application \"TextEdit\" to get text of front document");
    println!("\n===== TEXTEDIT CONTENTS =====\n{contents}\n=============================");

    // Clean up before asserting so a failure doesn't leave a stray document.
    osascript("tell application \"TextEdit\" to close front document saving no");

    assert_eq!(
        outcome,
        InjectOutcome::Pasted,
        "a focused text area must be recognised as a paste target"
    );
    assert!(
        contents.contains(&marker),
        "expected injected marker {marker:?} in TextEdit, got {contents:?}"
    );
}

/// The regression test for the silent-loss bug: dictating while no text field
/// is focused used to fire Cmd+V into the void, read `osascript`'s exit code 0
/// as success, and then restore the pre-dictation clipboard over the
/// transcript - so the text was neither pasted nor recoverable from the
/// clipboard, and nothing anywhere said so.
#[test]
#[ignore = "requires Accessibility permission and drives Finder"]
fn live_inject_refuses_an_app_with_no_editable_focus() {
    let marker = format!("scribe-no-target-{}", std::process::id());

    let sentinel = format!("clipboard-sentinel-{}", std::process::id());
    osascript(&format!("set the clipboard to \"{sentinel}\""));

    // A Finder *window* is opened explicitly, not just Finder activated: its
    // file list is a real focused element that takes no typed text, which is
    // the only thing that legitimately suppresses a paste. Finder with no
    // window reports no focused element at all, which is deliberately treated
    // as "cannot tell" (see `paste_target`) and would paste.
    osascript("tell application \"Finder\" to open home");
    osascript("tell application \"Finder\" to activate");
    sleep(Duration::from_millis(1500));

    let outcome = inject_text(&marker, Some(pid_of("Finder"))).expect("inject_text does not error");

    assert_eq!(
        outcome,
        InjectOutcome::NoPasteTarget,
        "Finder's file list must be recognised as having no paste target"
    );
    assert_eq!(
        clipboard(),
        sentinel,
        "a refused paste must leave the user's clipboard exactly as it found it"
    );
}

/// The regression test for the fix to the fix: an app that answers "no focused
/// element" must still be pasted into. VS Code and Edge both do that while a
/// text field is genuinely focused, and reading it as "no paste target" made
/// dictation refuse to paste into the text box the user was typing in.
#[test]
#[ignore = "requires Accessibility permission and a running Microsoft Edge"]
fn live_inject_pastes_into_a_browser_text_field() {
    let pid = pid_of("Microsoft Edge");
    let marker = format!("scribe-browser-{}", std::process::id());

    osascript("tell application \"Microsoft Edge\" to activate");
    sleep(Duration::from_millis(1200));

    let outcome = inject_text(&marker, Some(pid)).expect("inject_text does not error");
    assert_eq!(
        outcome,
        InjectOutcome::Pasted,
        "a browser must never be classified as having no paste target"
    );
}

/// The regression test for the multi-window activation bug: `inject_text` must
/// land the dictation in the window the user was actually typing in, not some
/// sibling window of the same app.
///
/// Activating with `ActivateAllWindows` raises *every* window the target app
/// owns, which reorders them and can hand key status to a different one. That
/// was tried on 2026-08-20 and broke dictation in every multi-window app the
/// user had (VS Code, Teams, Claude): the paste fired into a sibling window, so
/// nothing appeared where the caret was blinking and no widget explained why.
#[test]
#[ignore = "requires Accessibility permission and drives TextEdit"]
fn live_inject_lands_in_the_focused_window_not_a_sibling() {
    let marker = format!("scribe-window-{}", std::process::id());

    osascript("tell application \"TextEdit\" to activate");
    // Two documents, so there is a sibling window available to lose the paste
    // to. The second one made is frontmost, and is where the text must land.
    osascript("tell application \"TextEdit\" to make new document");
    osascript(
        "tell application \"TextEdit\" to set text of front document to \"sibling-document\"",
    );
    osascript("tell application \"TextEdit\" to make new document");
    sleep(Duration::from_millis(900));

    let outcome = inject_text(&marker, Some(pid_of("TextEdit")))
        .expect("inject_text succeeds with Accessibility granted");
    sleep(Duration::from_millis(600));

    let front = osascript("tell application \"TextEdit\" to get text of front document");
    let all = osascript("tell application \"TextEdit\" to get text of every document as string");

    osascript("tell application \"TextEdit\" to close every document saving no");

    assert_eq!(outcome, InjectOutcome::Pasted);
    assert!(
        front.contains(&marker),
        "dictation must land in the frontmost document; front={front:?} all_documents={all:?}"
    );
}
