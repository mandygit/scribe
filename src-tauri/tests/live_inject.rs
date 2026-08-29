//! Live text-injection checks against real apps: `inject_text` must land the
//! dictation in a focused text field, must refuse to fire a paste at an app
//! that has nowhere to put it, and must not spend the user's clipboard doing
//! either.
//!
//! # What a test binary cannot check here
//!
//! Injection reactivates its target and refuses to paste unless macOS agrees
//! that target is frontmost. A test binary cannot always produce that state:
//! activation is cooperative, an app may only hand it to another while it is
//! itself active, and a command-line binary never is. Measured 2026-08-26 on
//! this machine - `activate` for Edge and Finder returned cleanly, System
//! Events confirmed they were in front, and `NSWorkspace.frontmostApplication`
//! (what injection reads) never moved off the app the test process had itself
//! last activated. Pumping a run loop does not fix it either: `NSWorkspace`
//! updates on the *main* thread's loop, and libtest runs tests on spawned
//! threads even at `--test-threads=1`.
//!
//! So a `TargetNotFrontmost` here means the harness could not stage the test,
//! not that injection is broken - `skip_if_never_fronted` says so and bails.
//! Scribe itself is unaffected: it is a GUI app whose main loop keeps that
//! value fresh, and its own logs show the handoff arriving on 68 of 69 real
//! dictations.
//!
//! Ignored by default because they need the Accessibility permission and drive
//! the GUI. Run them with:
//!
//! ```sh
//! cargo test --test live_inject -- --ignored --nocapture
//! ```

use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use scribe_lib::dictation::{inject_text, polish_selection, InjectOutcome, SelectionPolishOutcome};

/// How long to let a freshly opened window take key focus inside an app that is
/// already frontmost. Unlike activation, this is an intra-app handoff with
/// nothing to poll from outside, so it stays a short fixed wait.
const WINDOW_SETTLE: Duration = Duration::from_millis(500);

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

/// The app macOS currently considers frontmost.
fn frontmost_app() -> String {
    osascript(
        "tell application \"System Events\" to get name of first process whose frontmost is true",
    )
}

/// Brings `app` to the front, waits until macOS agrees it is there, and returns
/// its pid.
///
/// Every test here depends on its target actually being frontmost - injection
/// reads the target's live focus and sends keystrokes to whatever is in front -
/// and a fixed sleep after `activate` only assumes that happened. It is not a
/// safe assumption: activation is cooperative on modern macOS, and a request
/// from a background process (which a test binary is) can simply not be
/// honoured. Measured 2026-08-26: `activate` returned cleanly for Edge and
/// Finder and left *TextEdit* frontmost for the next several seconds, so
/// `inject_text` correctly refused to paste - and the tests reported it as a
/// paste-target bug. Assert the precondition instead, and name it when the
/// environment will not meet it.
fn activate_and_wait(app: &str) -> i32 {
    osascript(&format!("tell application \"{app}\" to activate"));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let front = frontmost_app();
        if front == app {
            return pid_of(app);
        }
        assert!(
            Instant::now() < deadline,
            "the environment never brought {app} to the front (frontmost is {front:?}); \
             injection cannot be tested against an app that is not in front"
        );
        sleep(Duration::from_millis(100));
    }
}

/// Bails out of a test that this process cannot set up, rather than reporting
/// the product as broken for it. See the module docs on cooperative activation.
fn skip_if_never_fronted(outcome: InjectOutcome, app: &str) -> bool {
    if outcome == InjectOutcome::TargetNotFrontmost {
        eprintln!(
            "\nSKIPPED: this test binary could not bring {app} forward. macOS only lets an \
             app hand activation to another while it is itself active, and a test binary \
             never is, so injection correctly withheld the text. Nothing is proven either \
             way here - see the module docs.\n"
        );
        return true;
    }
    false
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
    let pid = activate_and_wait("TextEdit");
    osascript("tell application \"TextEdit\" to make new document");
    sleep(WINDOW_SETTLE);

    let outcome =
        inject_text(&marker, Some(pid)).expect("inject_text succeeds with Accessibility granted");
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
    let pid = activate_and_wait("Finder");
    sleep(WINDOW_SETTLE);

    let outcome = inject_text(&marker, Some(pid)).expect("inject_text does not error");
    if skip_if_never_fronted(outcome, "Finder") {
        return;
    }

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
    let marker = format!("scribe-browser-{}", std::process::id());

    let pid = activate_and_wait("Microsoft Edge");
    sleep(WINDOW_SETTLE);

    let outcome = inject_text(&marker, Some(pid)).expect("inject_text does not error");
    if skip_if_never_fronted(outcome, "Microsoft Edge") {
        return;
    }
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

    let pid = activate_and_wait("TextEdit");
    // Two documents, so there is a sibling window available to lose the paste
    // to. The second one made is frontmost, and is where the text must land.
    osascript("tell application \"TextEdit\" to make new document");
    osascript(
        "tell application \"TextEdit\" to set text of front document to \"sibling-document\"",
    );
    osascript("tell application \"TextEdit\" to make new document");
    sleep(WINDOW_SETTLE);

    let outcome =
        inject_text(&marker, Some(pid)).expect("inject_text succeeds with Accessibility granted");
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

/// Dictating must not cost the user their clipboard.
///
/// Text goes into the focused element through Accessibility now, and only apps
/// that cannot be addressed that way fall back to the pasteboard. TextEdit can
/// be, so nothing here should touch the clipboard at all - where the old path
/// wrote the transcript to it, pasted, and wrote the previous contents back,
/// which is three chances to lose whatever the user had copied and one for a
/// clipboard manager to archive the dictation forever.
#[test]
#[ignore = "requires Accessibility permission and drives TextEdit"]
fn live_inject_leaves_the_clipboard_alone() {
    let marker = format!("scribe-clipboard-{}", std::process::id());
    let sentinel = format!("clipboard-sentinel-{}", std::process::id());
    osascript(&format!("set the clipboard to \"{sentinel}\""));

    let pid = activate_and_wait("TextEdit");
    osascript("tell application \"TextEdit\" to make new document");
    sleep(WINDOW_SETTLE);

    let outcome =
        inject_text(&marker, Some(pid)).expect("inject_text succeeds with Accessibility granted");
    sleep(Duration::from_millis(500));

    let contents = osascript("tell application \"TextEdit\" to get text of front document");
    let clipboard_after = clipboard();
    osascript("tell application \"TextEdit\" to close front document saving no");

    assert_eq!(outcome, InjectOutcome::Pasted);
    assert!(
        contents.contains(&marker),
        "expected {marker:?} in TextEdit, got {contents:?}"
    );
    assert_eq!(
        clipboard_after, sentinel,
        "dictating must leave the user's clipboard exactly as it found it"
    );
}

/// Polishing a selection must leave the clipboard exactly as it found it, the
/// same guarantee `live_inject_leaves_the_clipboard_alone` makes for dictation.
///
/// This one is easy to get wrong in a way that looks right: `inject_text`
/// already snapshots and restores, so the clipboard *is* restored - just to the
/// raw selection that `copy_selection` put there moments earlier, rather than
/// to whatever the user had copied. The assertion below is the difference.
#[test]
#[ignore = "requires Accessibility permission, Apple Intelligence, and drives TextEdit"]
fn live_polish_selection_leaves_the_clipboard_alone() {
    let sentinel = format!("clipboard-sentinel-{}", std::process::id());
    let messy = "so um i think we should like maybe ship it on tuesday";
    osascript(&format!("set the clipboard to \"{sentinel}\""));

    activate_and_wait("TextEdit");
    osascript("tell application \"TextEdit\" to make new document");
    sleep(WINDOW_SETTLE);
    osascript(&format!(
        "tell application \"TextEdit\" to set text of front document to \"{messy}\""
    ));
    sleep(WINDOW_SETTLE);
    osascript("tell application \"System Events\" to keystroke \"a\" using command down");
    sleep(WINDOW_SETTLE);

    let outcome = polish_selection().expect("polish_selection runs");

    let contents = osascript("tell application \"TextEdit\" to get text of front document");
    let clipboard_after = clipboard();
    osascript("tell application \"TextEdit\" to close front document saving no");

    assert_eq!(outcome, SelectionPolishOutcome::Applied);
    assert_ne!(
        contents.trim(),
        messy,
        "the selection should have been replaced with polished text"
    );
    assert_eq!(
        clipboard_after, sentinel,
        "polishing must leave the user's clipboard exactly as it found it"
    );
}
