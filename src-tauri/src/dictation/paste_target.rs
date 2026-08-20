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

use std::ffi::c_void;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_foundation::NSString;

use super::inject::TargetAppPid;

type CFTypeRef = *const c_void;
type AXUIElementRef = CFTypeRef;
type AXError = i32;

const AX_SUCCESS: AXError = 0;

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
const FOCUS_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// Gap between those attempts.
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(60);

// AX lives in ApplicationServices (HIServices). Linked explicitly rather than
// relying on AppKit to drag it in.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: *const NSString,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: *const NSString,
        settable: *mut u8,
    ) -> AXError;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn AXIsProcessTrusted() -> bool;
}

/// Whether this process may read other apps' Accessibility trees at all.
///
/// Without it every `AXUIElementCopyAttributeValue` below returns nothing, so
/// every app looks like it "named no focused element" and the paste target is
/// undetectable - which silently degrades this module into "always paste,
/// never warn". That is not a hypothetical: on 2026-08-20 an ad-hoc-signed
/// reinstall dropped the grant, and because nothing checked or reported it,
/// several rounds of debugging went into the paste-target rules while the real
/// answer was that Scribe could not see anything at all.
pub fn accessibility_is_trusted() -> bool {
    // SAFETY: a plain predicate with no arguments and no ownership transfer.
    unsafe { AXIsProcessTrusted() }
}

/// A CoreFoundation value owned by us, released on drop. AX hands back +1
/// references from every `Copy` call.
struct OwnedCfType(CFTypeRef);

impl Drop for OwnedCfType {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: non-null and owned - every construction site is a
            // CoreFoundation `Copy`/`Create` call, which returns +1.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Reads one attribute off an element, taking ownership of the result.
/// `None` for any error, including "no value".
fn copy_attribute(element: AXUIElementRef, attribute: &str) -> Option<OwnedCfType> {
    let name = NSString::from_str(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is a live AX element, `name` outlives the call, and
    // `value` is only read when the call reports success.
    let error = unsafe {
        AXUIElementCopyAttributeValue(element, Retained::as_ptr(&name), &mut value as *mut _)
    };
    if error != AX_SUCCESS || value.is_null() {
        return None;
    }
    Some(OwnedCfType(value))
}

/// Reads a CFString-valued attribute as a Rust string, via the toll-free
/// bridge to `NSString`. The type is checked rather than assumed: AX returns
/// whatever the app put there.
fn string_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    let value = copy_attribute(element, attribute)?;
    // SAFETY: CFStringGetTypeID/CFGetTypeID are pure reads of live CF values.
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    // SAFETY: confirmed a CFString above, which is toll-free bridged to
    // NSString; the borrow ends before `value` is released.
    let string: &NSString = unsafe { &*(value.0 as *const NSString) };
    Some(string.to_string())
}

/// Whether the element's `AXValue` can be written - the most direct "this
/// takes text" signal, and the one that works for custom controls with
/// non-standard roles.
fn value_is_settable(element: AXUIElementRef) -> bool {
    let name = NSString::from_str("AXValue");
    let mut settable: u8 = 0;
    // SAFETY: as `copy_attribute`; `settable` is only read on success.
    let error = unsafe {
        AXUIElementIsAttributeSettable(element, Retained::as_ptr(&name), &mut settable as *mut _)
    };
    error == AX_SUCCESS && settable != 0
}

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
    // SAFETY: creating an application element is safe for any pid; AX returns
    // an element that simply answers errors if the process is gone.
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        log_decision(pid, true, "no application element; not withholding on that");
        return true;
    }
    let app = OwnedCfType(app);

    // Poll: an app that was just activated needs a moment before it will name
    // its focused element. Deliberately the *first* thing tried, and the only
    // thing retried - the window check below is a conclusion drawn after this
    // has had its chance, not a precondition. Checking for a focused window up
    // front raced the activation and withheld pastes from apps that were
    // simply still waking up.
    let deadline = Instant::now() + FOCUS_POLL_TIMEOUT;
    let mut unresolved_role: Option<String> = None;
    loop {
        if let Some(focused) = copy_attribute(app.0, "AXFocusedUIElement") {
            match classify(focused.0) {
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
    if !accessibility_is_trusted() {
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
    log_silent_app_diagnostics(pid, app.0);

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

fn classify(element: AXUIElementRef) -> Verdict {
    if value_is_settable(element) {
        return Verdict::Paste("focused element has a settable AXValue".to_string());
    }
    classify_role(string_attribute(element, "AXRole").as_deref())
}

/// Dumps what a frontmost app that named no focused element *will* say about
/// itself. Purely diagnostic - see the call site for the question it answers.
///
/// Logs the window's role and subrole but NOT its title: a window title is
/// user content (a document name, a chat partner, a page heading) and this log
/// is deliberately free of anything the user said or is looking at.
fn log_silent_app_diagnostics(pid: TargetAppPid, app: AXUIElementRef) {
    let focused_window = copy_attribute(app, "AXFocusedWindow");
    let window_role = focused_window
        .as_ref()
        .and_then(|window| string_attribute(window.0, "AXRole"))
        .unwrap_or_else(|| "-".to_string());
    let window_subrole = focused_window
        .as_ref()
        .and_then(|window| string_attribute(window.0, "AXSubrole"))
        .unwrap_or_else(|| "-".to_string());
    // Whether the app answers anything at all, so a dead or unresponsive
    // target is not mistaken for a meaningful "no".
    let app_role = string_attribute(app, "AXRole").unwrap_or_else(|| "-".to_string());
    let main_window = copy_attribute(app, "AXMainWindow").is_some();

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
        // Exercises the full FFI path (element creation, attribute copy, role
        // read, settable query, release) against a live process, so a mistake
        // in the ownership handling shows up as a crash here rather than in
        // dictation. Scribe's own windows are not focused during tests, so the
        // answer itself is not asserted -- only that it returns, promptly.
        let started = Instant::now();
        let _ = has_paste_target(std::process::id() as TargetAppPid);
        assert!(
            started.elapsed() < FOCUS_POLL_TIMEOUT + Duration::from_millis(400),
            "has_paste_target must be bounded by its poll budget"
        );
    }
}
