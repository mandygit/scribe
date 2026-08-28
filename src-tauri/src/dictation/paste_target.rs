//! Answers one question before dictation pastes: is there somewhere for the
//! text to go?
//!
//! This exists because a synthesised paste gives no feedback. `osascript`
//! exits 0 whenever System Events *accepted* the keystroke - it has no idea
//! whether anything consumed it (verified 2026-08-20: Cmd+V with Finder's file
//! list focused exits 0 and pastes nothing). Dictation used to read that exit
//! code as "the paste landed", record the session as pasted, and then restore
//! the pre-dictation clipboard, so a dictation aimed at a window with no text
//! field vanished silently - not pasted, not on the clipboard, not flagged.
//!
//! # Why this cannot be a test for "is the focused thing editable"
//!
//! Two earlier versions tried to identify an editable target and paste only
//! into that. Both were wrong, because plenty of apps that accept text expose
//! nothing that looks editable - measured against the real thing:
//!
//! | App, with a caret visibly blinking | What Accessibility says |
//! | --- | --- |
//! | VS Code, cursor in the editor      | no focused element at all |
//! | A terminal, cursor at the prompt   | focused element is the *window* |
//! | TextEdit / Claude's composer       | `AXValue` settable |
//!
//! Requiring positive proof of editability therefore refuses to paste into
//! editors and terminals, which is the worst outcome available: the user
//! watches their dictation not arrive somewhere it plainly should have.
//!
//! # The rule, inverted
//!
//! So the default is to paste, and text is withheld on exactly one piece of
//! positive evidence: **a concrete, non-text element holds focus** - a file
//! list, a link, a button. Something specific has the keystrokes and it is
//! demonstrably not text.
//!
//! Everything else pastes. In particular, silence is not evidence: an app that
//! declines to name its focused element may be handling keys itself, or may
//! simply not have finished coming forward, and those are indistinguishable.
//! An earlier version tried to separate them with `AXFocusedWindow` and got it
//! badly wrong - a backgrounded app has no focused window either, so a
//! dictation aimed at Claude's message box was refused with "nothing can
//! receive keys" when the truth was that Claude had not been activated yet.
//!
//! A third answer exists alongside "paste" and "withhold": a *container* role
//! (`AXWebArea`) means the app named the box the focused thing lives in rather
//! than the thing itself. One look at that decides nothing, so it is re-asked
//! until the specific element turns up or the budget runs out. Chromium apps
//! answer this way for the first few tens of milliseconds after coming forward,
//! and deciding on that first look made pasting into Teams and Claude fail at
//! random (2026-08-20).
//!
//! How long it lasts is the signal. Measured in Claude, from the app's own log:
//! with the composer focused the answer sharpens to a settable `AXValue` in
//! 18ms; with the user clicked away from it, `AXWebArea` stood for the whole
//! 250ms. So a container that survives the entire poll IS the positive evidence
//! this module looks for - web content with no field focused - and it withholds.
//! That is what makes the recovery widget reachable inside a Chromium app,
//! which it otherwise never would be.

use std::time::{Duration, Instant};

use super::ax::{self, Element};
use super::inject::TargetAppPid;

/// Roles that are text entry outright. Checked after `AXValue` settability for
/// controls that expose the role but not a writable value.
const TEXT_ROLES: [&str; 4] = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];

/// Roles meaning "the app itself owns the keystrokes" rather than any widget
/// inside it. Terminals report their window as the focused element while the
/// cursor sits at a live prompt, so this must count as a paste target.
const APP_LEVEL_ROLES: [&str; 2] = ["AXWindow", "AXApplication"];

/// Roles that are a *container* for the thing that really has focus, not the
/// thing itself. Chromium and Electron name the whole web area for a beat
/// after their app comes forward, before resolving to the field inside it -
/// measured 2026-08-20 in Teams and Claude, where reads 21-26ms after
/// activation said `AXWebArea` and reads 63-66ms later said the composer had a
/// settable `AXValue`. Treating that transient as "a non-text element holds
/// focus" withheld the paste at random, which is what "it works intermittently,
/// especially coming from another app" looked like from the outside.
///
/// One look at a container decides nothing - it is re-asked until the real
/// element turns up. Only a container that survives the ENTIRE poll withholds,
/// because by then it is not a transient: it is a page with no field focused.
const CONTAINER_ROLES: [&str; 1] = ["AXWebArea"];

/// How long to keep asking an app to describe its focused element before
/// treating it as one that handles keys itself. An app that was just activated
/// needs a moment: Microsoft Teams answers 10 times out of 10 within the
/// activation settle, but never while it is in the background.
///
/// Raised from 250ms, which was exactly the larger of the two measurements it
/// had to separate (a composer sharpening in 18ms, a blurred one standing for
/// the full 250ms) and so had no margin at all: a cold or busy page resolving
/// at 300ms was indistinguishable from one with nothing focused, and got its
/// dictation withheld. The only cost of the extra budget is a slightly later
/// recovery widget in the case that really has nowhere to go.
const FOCUS_POLL_TIMEOUT: Duration = Duration::from_millis(400);

/// Gap between those attempts.
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(60);

/// Whether a Cmd+V sent to `pid` has somewhere to land.
///
/// Call this *after* the target app has been reactivated: the focused element
/// is read live, and an app that is not frontmost generally will not describe
/// one at all.
///
/// Returns `false` only on positive evidence that there is no target - see the
/// module docs. Anything unclear returns `true`, because failing to paste into
/// an editor is worse than pasting into nothing.
///
/// Every decision is written to the app debug log with the evidence behind it.
/// A wrong answer here is invisible from the outside - the user just sees a
/// paste that did not happen, or a widget that should not have appeared - and
/// this is the only way to tell which rule fired without guessing.
pub fn has_paste_target(pid: TargetAppPid) -> bool {
    // AX makes an element for any pid, and one for a dead or unresponsive
    // process simply answers errors to everything - which reads the same as an
    // app with nothing to say, and is handled as such below.
    let Some(app) = Element::for_app(pid) else {
        log_decision(pid, true, "no application element; not withholding on that");
        return true;
    };

    // Poll: an app that was just activated needs a moment before it will name
    // its focused element. Deliberately the *first* thing tried, and the only
    // thing retried - the window check below is a conclusion drawn after this
    // has had its chance, not a precondition. Checking for a focused window up
    // front raced the activation and withheld pastes from apps that were
    // simply still waking up.
    let deadline = Instant::now() + FOCUS_POLL_TIMEOUT;
    let mut unresolved_role: Option<String> = None;
    loop {
        if let Some(focused) = app.focused() {
            match classify(&focused) {
                Verdict::Paste(reason) => {
                    log_decision(pid, true, &reason);
                    return true;
                }
                Verdict::Widget(reason) => {
                    log_decision(pid, false, &reason);
                    return false;
                }
                // A container answered instead of whatever is inside it. Keep
                // asking: the specific element usually turns up within a poll
                // or two, and that is the difference between pasting into the
                // user's text box and refusing to.
                Verdict::Unresolved(role) => unresolved_role = Some(role),
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(FOCUS_POLL_INTERVAL);
    }

    // A container that never resolved, after the full budget of re-asks. That
    // is not the transient this poll exists to ride out - it is web content
    // holding focus with nothing inside it that does, i.e. a page with no field
    // focused. Measured 2026-08-20 in Claude: with the composer focused the
    // answer sharpens to a settable `AXValue` in 18ms, and with the user
    // clicked away from it `AXWebArea` stood for the whole 250ms.
    //
    // This is the one place a container withholds, and it is what makes the
    // recovery widget appear at all in a Chromium app - the case the user
    // reaches for by clicking outside the message box.
    if let Some(role) = unresolved_role {
        log_decision(
            pid,
            false,
            &format!(
                "web content held focus for the whole poll and never named a field (role {role})"
            ),
        );
        return false;
    }

    // Distinguish "this app has nothing to say" from "we are not allowed to
    // ask". Both look identical from here, and conflating them is what made a
    // revoked permission masquerade as a paste-target bug.
    if !ax::is_trusted() {
        log_decision(
            pid,
            true,
            "ACCESSIBILITY NOT TRUSTED - cannot read any app's focus; paste targets are undetectable",
        );
        return true;
    }

    // The app never named a focused element. That is not evidence of anything:
    // an app that handles keys itself answers this way (VS Code's editor, a
    // terminal), and so does one that simply has not finished coming forward.
    //
    // `AXFocusedWindow` was briefly used here to tell those apart, and it
    // could not: a backgrounded app has no focused window either, so dictating
    // into Claude's message box was refused with "nothing can receive keys"
    // when the real problem was that Claude had not been activated yet
    // (observed 2026-08-20). Both silences still mean paste.
    //
    // That earlier reading was taken before anything guaranteed the target was
    // frontmost, which is exactly what made it worthless. `inject_text` now
    // waits for the handoff and logs `arrived=true`, so the same attribute is
    // being re-measured under the one condition it was never measured under.
    // DIAGNOSTIC ONLY - it decides nothing here. The open question it exists to
    // answer: VS Code mid-edit (must paste) and a blurred Claude composer
    // (should show the widget) are indistinguishable today, both reporting no
    // focused element. If they differ on the focused window, that separates
    // them; if they match, this cannot be solved from Accessibility and the
    // probe comes out again.
    log_silent_app_diagnostics(pid, &app);

    log_decision(pid, true, "app named no focused element; not withholding");
    true
}

/// The verdict for an app that did name its focused element.
/// What one look at the focused element established.
enum Verdict {
    /// Somewhere to paste; carries the evidence for the log.
    Paste(String),
    /// Positive evidence of nowhere to paste - the only thing that withholds.
    Widget(String),
    /// A container answered for whatever is really focused inside it, so this
    /// look established nothing. Worth another look before giving up.
    Unresolved(String),
}

/// The role half of [`classify`], split out so the role table can be tested
/// without a live AX element (settability needs a real one).
fn classify_role(role: Option<&str>) -> Verdict {
    let Some(role) = role else {
        return Verdict::Paste("focused element has no readable role".to_string());
    };
    if TEXT_ROLES.contains(&role) {
        return Verdict::Paste(format!("focused element is text (role {role})"));
    }
    if APP_LEVEL_ROLES.contains(&role) {
        return Verdict::Paste(format!("app-level focus (role {role})"));
    }
    if CONTAINER_ROLES.contains(&role) {
        return Verdict::Unresolved(role.to_string());
    }
    Verdict::Widget(format!(
        "a concrete non-text element holds focus (role {role})"
    ))
}

fn classify(element: &Element) -> Verdict {
    if element.attribute_is_settable("AXValue") {
        return Verdict::Paste("focused element has a settable AXValue".to_string());
    }
    // The invariant that keeps this module honest with `inject`: never withhold
    // from an element injection could have written to. `inject` writes through
    // `AXSelectedText`, so its settability is proof of a paste target no matter
    // what role the element claims.
    //
    // This is not the same question as `AXValue` above, and Chromium is where
    // they come apart. A `contenteditable` composer has no single settable
    // value - measured 2026-08-26 in Claude, whose focused web node reports
    // role `AXGroup` with `AXValue` unsettable - so it falls past that check to
    // a role table that has never heard of `AXGroup` and withholds. Asking the
    // insertion question directly gets those composers pasted into, while the
    // read-only article regions that share the same role (subrole
    // `AXDocumentArticle`, `AXSelectedText` unsettable) still correctly do not.
    if element.attribute_is_settable(ax::SELECTED_TEXT) {
        return Verdict::Paste("focused element has settable AXSelectedText".to_string());
    }
    classify_role(element.attribute_string("AXRole").as_deref())
}

/// Dumps what a frontmost app that named no focused element *will* say about
/// itself. Purely diagnostic - see the call site for the question it answers.
///
/// Logs the window's role and subrole but NOT its title: a window title is
/// user content (a document name, a chat partner, a page heading) and this log
/// is deliberately free of anything the user said or is looking at.
fn log_silent_app_diagnostics(pid: TargetAppPid, app: &Element) {
    let focused_window = app.attribute_element("AXFocusedWindow");
    let window_role = focused_window
        .as_ref()
        .and_then(|window| window.attribute_string("AXRole"))
        .unwrap_or_else(|| "-".to_string());
    let window_subrole = focused_window
        .as_ref()
        .and_then(|window| window.attribute_string("AXSubrole"))
        .unwrap_or_else(|| "-".to_string());
    // Whether the app answers anything at all, so a dead or unresponsive
    // target is not mistaken for a meaningful "no".
    let app_role = app
        .attribute_string("AXRole")
        .unwrap_or_else(|| "-".to_string());
    let main_window = app.attribute_element("AXMainWindow").is_some();

    #[cfg(target_os = "macos")]
    crate::debug_log(&format!(
        "paste_target_probe pid={pid} focused_window={} window_role={window_role} \
         window_subrole={window_subrole} main_window={main_window} app_role={app_role}",
        focused_window.is_some()
    ));
    #[cfg(not(target_os = "macos"))]
    let _ = (pid, window_role, window_subrole, app_role, main_window);
}

/// Records one verdict and its evidence. Never logs any dictated text.
fn log_decision(pid: TargetAppPid, paste: bool, why: &str) {
    #[cfg(target_os = "macos")]
    crate::debug_log(&format!(
        "paste_target pid={pid} verdict={} reason={why}",
        if paste { "PASTE" } else { "WIDGET" }
    ));
    #[cfg(not(target_os = "macos"))]
    let _ = (pid, paste, why);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_says_nothing_is_never_withheld_from() {
        // pid 1 (launchd) names no focused element. Silence must not withhold
        // a paste: a busy or not-yet-activated app looks exactly like this,
        // and refusing those is what broke pasting into real text boxes.
        assert!(has_paste_target(1));
    }

    #[test]
    fn a_container_role_is_never_decided_on_a_single_look() {
        // AXWebArea is what Chromium names for a beat after its app comes
        // forward, before resolving to the field inside. Deciding on that first
        // look - either way - is wrong: reading it as "a non-text element holds
        // focus" made pasting into Teams and Claude intermittent, and reading it
        // as "paste anyway" meant the widget could never appear there
        // (both observed 2026-08-20). It must be re-asked.
        assert!(matches!(
            classify_role(Some("AXWebArea")),
            Verdict::Unresolved(_)
        ));
    }

    #[test]
    fn a_concrete_non_text_role_still_withholds() {
        // The one signal that must keep working: a file list or a link really
        // does have the keystrokes and really is not text.
        assert!(matches!(
            classify_role(Some("AXOutline")),
            Verdict::Widget(_)
        ));
        assert!(matches!(classify_role(Some("AXLink")), Verdict::Widget(_)));
    }

    #[test]
    fn text_and_app_level_roles_paste() {
        for role in ["AXTextField", "AXTextArea", "AXWindow", "AXApplication"] {
            assert!(
                matches!(classify_role(Some(role)), Verdict::Paste(_)),
                "{role} must be a paste target"
            );
        }
    }

    #[test]
    fn a_live_process_answers_within_the_poll_budget() {
        // The poll runs against a live app and must stay bounded: dictation
        // blocks on this answer, and an app that keeps saying "container"
        // forever must not hold the transcript hostage with it. Scribe's own
        // windows are not focused during tests, so the answer itself is not
        // asserted -- only that it arrives, promptly. (The FFI ownership path
        // this walks is covered directly in `ax`.)
        let started = Instant::now();
        let _ = has_paste_target(std::process::id() as TargetAppPid);
        assert!(
            started.elapsed() < FOCUS_POLL_TIMEOUT + Duration::from_millis(400),
            "has_paste_target must be bounded by its poll budget"
        );
    }
}
