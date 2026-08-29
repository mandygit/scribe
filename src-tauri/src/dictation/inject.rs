//! Text injection: drop dictated text into the app the user was typing in.
//!
//! Two mechanisms, tried in that order:
//!
//! 1. **Write it into the focused element via Accessibility** - see
//!    `insert_via_accessibility`. The text goes straight where the caret is,
//!    the clipboard is never touched, and the app either accepts the write or
//!    says why it did not.
//! 2. **Clipboard and a synthesised Cmd+V**, restoring the clipboard's previous
//!    contents afterwards. The fallback for everything the first mechanism
//!    cannot address - terminals, VS Code, anything that names no focused
//!    element - and the only mechanism that existed before.
//!
//! The second is also used in reverse by polish-selection: Cmd+C copies the
//! focused app's current selection out to the clipboard so it can be read back
//! and polished. Both need the Accessibility permission; without it macOS
//! blocks the keystroke and `osascript` reports an error we surface with a
//! stable code, and the AX writes silently fail too.

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

use super::ax;
use super::paste_target;

// Secure input state lives in Carbon's HIToolbox. Deprecated, still the only
// way to ask, and still exactly right.
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

/// Whether macOS is currently refusing synthesised keystrokes process-wide.
///
/// Deliberately consulted only on the clipboard path: this blocks *events*, and
/// an Accessibility write is not an event. It is also a global condition rather
/// than a property of the focused field - any app holding secure input turns it
/// on for everyone - which is precisely why it has to be asked rather than
/// inferred from what has focus.
fn secure_input_is_active() -> bool {
    // SAFETY: a plain predicate with no arguments and no ownership transfer.
    unsafe { IsSecureEventInputEnabled() }
}

/// What [`inject_text`] actually did, so the caller can tell a real paste from
/// one that was deliberately not attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectOutcome {
    /// The text is in the target app - written straight into its focused
    /// element, or failing that pasted with Cmd+V.
    Pasted,
    /// The target app had no editable focus, so nothing was pasted and the
    /// clipboard was left untouched. The text is still held in memory for the
    /// recovery widget - see `AppState::last_dictation`.
    NoPasteTarget,
    /// The app that was frontmost when dictation started never came back to the
    /// front, so nothing was sent and the clipboard was left as it was found.
    ///
    /// A synthesised Cmd+V lands in whatever is frontmost *at that instant*,
    /// not in whatever we aimed at, so pasting without the target in front is
    /// how a dictation ends up in a different app entirely. Observed in the
    /// wild on 2026-08-20: `arrived=false`, `frontmost_now` naming another app,
    /// and 15 characters typed into it, with the previous clipboard restored
    /// over them 300ms later.
    TargetNotFrontmost,
    /// macOS secure input was active, so a synthesised Cmd+V could not have
    /// been delivered. Nothing was sent and the clipboard was left alone.
    ///
    /// The silent-loss case this exists to end: with a password field focused
    /// (or a terminal in Secure Keyboard Entry), the window server drops
    /// synthesised keystrokes, and `osascript` reports success anyway because
    /// System Events did accept the event. Dictation read that as a paste,
    /// played the done cue, and restored the previous clipboard over the
    /// transcript.
    SecureInputActive,
    /// Cmd+V was delivered and the focused element did not change: the app
    /// took the keystroke and inserted nothing. The transcript is deliberately
    /// left on the clipboard.
    ///
    /// `osascript` cannot report this - it exits 0 once System Events accepts
    /// the event, which says nothing about whether anything consumed it. Read
    /// only text views (a log pane, a disabled field) and apps that bind Cmd+V
    /// to something else both land here, and both used to be recorded as a
    /// successful paste with the transcript then wiped by the clipboard
    /// restore.
    PasteDidNotLand,
}

/// The process id of whichever app was frontmost just before dictation
/// started, i.e. the paste target.
pub type TargetAppPid = libc::pid_t;

/// How often the background tracker in [`spawn_frontmost_app_tracker`]
/// polls `NSWorkspace.frontmostApplication`.
const FRONTMOST_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// The most recent frontmost app that was NOT Scribe itself, kept fresh by
/// [`spawn_frontmost_app_tracker`].
static LAST_EXTERNAL_FRONTMOST: StdMutex<Option<TargetAppPid>> = StdMutex::new(None);

/// Records one step of an injection to the app debug log. A paste that goes
/// missing leaves no other trace: the installed app's stderr is discarded, and
/// `osascript` reports success for a keystroke nothing consumed.
fn log_inject(message: &str) {
    #[cfg(target_os = "macos")]
    crate::debug_log(&format!("inject {message}"));
    #[cfg(not(target_os = "macos"))]
    let _ = message;
}

/// A single point-in-time read of `NSWorkspace.frontmostApplication`'s pid.
fn frontmost_app_pid() -> Option<TargetAppPid> {
    Some(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier(),
    )
}

/// Starts a background thread that keeps `LAST_EXTERNAL_FRONTMOST` fresh
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

/// Where the dictation about to start should end up, to be handed back to
/// [`reactivate`] right before pasting.
///
/// The most recently observed non-Scribe frontmost app, falling back to a live
/// reading when the tracker has not had a tick yet.
///
/// That fallback matters more than it looks: with no target at all, injection
/// pastes blind into whatever is in front and reports success whatever
/// happened, because every check it has - the activation guard, the paste
/// target, the Accessibility write - needs a pid to ask about. There is always
/// some frontmost app, so there is no reason to ever run without one.
///
/// Says nothing about the case where the user is typing in Scribe's own
/// window; `dictation_target_app` handles that before calling this.
pub fn capture_frontmost_app() -> Option<TargetAppPid> {
    LAST_EXTERNAL_FRONTMOST
        .lock()
        .ok()
        .and_then(|guard| *guard)
        .or_else(frontmost_app_pid)
}

/// Explicitly reactivates the app captured by [`capture_frontmost_app`] so
/// the synthesised Cmd+V has a real target to land in, instead of relying on
/// key focus implicitly falling back to "whatever window was previously
/// active" - a race that gets less reliable across multiple displays/Spaces,
/// since there is more than one place focus could land. Returns `false` if the
/// app has since quit.
///
/// Activation is cooperative on modern macOS - an app can only bring another
/// forward while it is itself active, and `ActivateIgnoringOtherApps` has been
/// deprecated to a no-op since macOS 14, so there is no way to force it. That
/// is survivable: the path that needs the handoff is the one where the user
/// clicked the pill, and Scribe *is* active then. What is not survivable is
/// assuming it happened - see `wait_until_frontmost`, and note that a
/// backgrounded app describes neither a focused element nor a focused window,
/// which reads exactly like having nothing to paste into (observed
/// 2026-08-20: a dictation aimed at Claude's message box was refused with
/// "nothing can receive keys" because Claude had not come forward yet).
pub fn reactivate(pid: TargetAppPid) -> bool {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return false;
    };
    // Deliberately plain activation, NOT `ActivateAllWindows`: that option
    // raises every window the app owns, which in a multi-window app can
    // reorder them and hand key status to one the user was not typing in.
    // Restoring the right window is the target app's own job, so ask for the
    // narrowest thing that gets the app forward.
    app.activateWithOptions(NSApplicationActivationOptions::empty())
}

/// Blocks until `pid` is actually the frontmost app, or the deadline passes.
///
/// `activateWithOptions` returning only means the request was accepted; the
/// window server processes the handoff asynchronously. A fixed sleep either
/// wastes time or is too short, and the old code skipped the wait entirely
/// whenever the activation call returned `false` - which is exactly when the
/// target needed the most time.
fn wait_until_frontmost(pid: TargetAppPid) -> bool {
    let deadline = std::time::Instant::now() + TARGET_ACTIVATION_TIMEOUT;
    loop {
        if frontmost_app_pid() == Some(pid) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(TARGET_ACTIVATION_POLL);
    }
}

/// How long to wait after the Cmd+V keystroke before restoring the previous
/// clipboard. The keystroke call returning only means the event was posted;
/// the target app reads the clipboard asynchronously, and restoring too early
/// would paste the old contents instead of the transcript.
const PASTE_SETTLE: Duration = Duration::from_millis(300);

/// How long to wait for the target app to actually become frontmost after
/// [`reactivate`].
///
/// This used to be a gamble timer - it expired and the paste fired anyway, at
/// whatever was in front. Now expiry withholds the text instead
/// ([`InjectOutcome::TargetNotFrontmost`]), which changes what the number is
/// for: every millisecond here buys a genuinely slow handoff the chance to
/// finish, and the only cost of waiting too long is showing the recovery
/// widget slightly later on a dictation that was already lost. Raised from
/// 700ms on that basis, because the slow handoff is real - activating an app
/// on another Space waits out the switch animation before it counts as
/// frontmost.
const TARGET_ACTIVATION_TIMEOUT: Duration = Duration::from_millis(1200);

/// Gap between frontmost checks while waiting.
const TARGET_ACTIVATION_POLL: Duration = Duration::from_millis(25);

/// Every flavor of every item on the pasteboard, as raw bytes keyed by UTI.
/// Full fidelity — screenshots, rich text, and multi-item copies all survive.
pub(super) type ClipboardSnapshot = Vec<Vec<(String, Vec<u8>)>>;

/// Injects `text` into `target` (the app that was frontmost when dictation
/// started, from [`capture_frontmost_app`]). Blank text is a no-op so a
/// silent dictation doesn't clear the clipboard or fire a stray paste.
///
/// The previous clipboard contents — including images and other non-text
/// flavors — are saved first and put back once the paste has landed, so
/// dictating doesn't eat whatever the user last copied.
pub fn inject_text(text: &str, target: Option<TargetAppPid>) -> Result<InjectOutcome, AppError> {
    if text.trim().is_empty() {
        return Ok(InjectOutcome::Pasted);
    }
    // Explicitly reactivate the app that was frontmost when dictation
    // started, rather than trusting key focus to fall back there on its own
    // once the pill hides — that implicit handoff gets less reliable the
    // more displays/Spaces are in play. With no captured target at all there
    // is nothing to hand focus back to, and injection proceeds blind into
    // whatever is frontmost, exactly as it did before any of this existed.
    if let Some(pid) = target {
        let before = frontmost_app_pid();
        let accepted = reactivate(pid);
        // Wait for the handoff to actually happen rather than sleeping a fixed
        // guess - everything below reads the target's live focus state, and
        // reading it before the app is frontmost answers about the wrong app.
        let arrived = wait_until_frontmost(pid);
        log_inject(&format!(
            "activation target={pid} ({}) frontmost_before={before:?} accepted={accepted} arrived={arrived}",
            app_name(pid)
        ));
        // Activation is cooperative and can simply not happen - the app quit
        // mid-dictation, or Scribe was not itself active and macOS declined.
        // Everything past this point either reads the target's focus (which
        // answers about an app the user is not looking at) or sends a
        // keystroke to whatever is frontmost *instead*. Stop here and let the
        // recovery widget hold the text.
        if !arrived {
            return Ok(InjectOutcome::TargetNotFrontmost);
        }
    }
    // Only now, with the target app actually frontmost, is it meaningful to
    // ask where the paste would land: the focused element is read live, and
    // asking any earlier answers for Scribe's own pill. Bailing out before
    // touching the pasteboard is the whole point - the original bug was that
    // this pasted into the void and then restored the previous clipboard over
    // the transcript, losing it entirely.
    if target.is_some_and(|pid| !paste_target::has_paste_target(pid)) {
        return Ok(InjectOutcome::NoPasteTarget);
    }
    // Preferred mechanism: write the text where the caret already is. Nothing
    // reaches the pasteboard, no keystroke is synthesised, it cannot land in
    // the wrong app because it addresses the element rather than "whatever is
    // frontmost", and the app answers whether it took. Only the apps this
    // cannot address fall through to the clipboard.
    if let Some(pid) = target {
        if insert_via_accessibility(pid, text) {
            return Ok(InjectOutcome::Pasted);
        }
    }
    // Past here the only remaining mechanism is a synthesised keystroke, and
    // secure input means the window server will drop it. Stop before touching
    // the pasteboard: pasting anyway is not a harmless retry, it is how the
    // transcript gets overwritten by the restore and lost.
    if secure_input_is_active() {
        log_inject("secure input is active; a synthesised paste cannot be delivered");
        return Ok(InjectOutcome::SecureInputActive);
    }
    // Read the caret before the keystroke so the paste can be confirmed
    // afterwards. Held across the paste deliberately: re-finding the focused
    // element after the fact would ask a different question, since a paste can
    // change which element has focus.
    let focused = target.and_then(focused_element);
    let caret_before = caret(focused.as_ref());
    // Best-effort save: an unreadable pasteboard just means there is nothing
    // to put back.
    let saved = snapshot_clipboard();
    set_clipboard(text)?;
    // The single most useful line in the log when a paste goes missing: a
    // synthesised Cmd+V lands in whatever is frontmost *now*, so if this does
    // not name the target, the text went somewhere else entirely and no amount
    // of reasoning about focused elements explains it.
    let frontmost_now = frontmost_app_pid();
    log_inject(&format!(
        "keystroke target={target:?} frontmost_now={frontmost_now:?} clipboard_chars={}",
        text.chars().count()
    ));
    // The last possible moment to check, and the only one that counts: the
    // keystroke is about to go to whatever is frontmost, and the target could
    // have lost the front at any point during the focus poll above. Put the
    // clipboard back first - nothing was pasted, so restoring it cannot race
    // anything.
    if target.is_some_and(|pid| frontmost_now != Some(pid)) {
        if !saved.is_empty() {
            restore_clipboard(&saved);
        }
        return Ok(InjectOutcome::TargetNotFrontmost);
    }
    // On paste failure, return with the transcript still on the clipboard so
    // the user can paste it by hand instead of losing the dictation outright.
    send_cmd_keystroke("v", "dictation_paste_failed")?;
    // Unconditional now, not just when there is a clipboard to put back: the
    // app reads the pasteboard asynchronously, so this is also how long the
    // check below has to wait before the answer means anything.
    std::thread::sleep(PASTE_SETTLE);
    // The keystroke was accepted. Whether anything consumed it is a different
    // question, and this is the only chance to ask it - an unchanged caret is
    // positive evidence that nothing was inserted, because inserting even one
    // character moves it. An element that will not report a caret decides
    // nothing and the paste is trusted, exactly as it was before this existed.
    if let (Some(before), Some(after)) = (caret_before, caret(focused.as_ref())) {
        if before == after {
            log_inject(&format!(
                "paste keystroke accepted but the caret never moved (still at {}+{}); \
                 leaving the transcript on the clipboard",
                before.location, before.length
            ));
            // Deliberately no restore: the clipboard is now the only place this
            // dictation exists outside Scribe, and putting the old contents
            // back over it is precisely the silent loss this check exists to
            // stop.
            return Ok(InjectOutcome::PasteDidNotLand);
        }
    }
    if !saved.is_empty() {
        // Best-effort restore: the paste already succeeded, and failing the
        // whole dictation over a restore hiccup would be worse than leaving
        // the transcript on the clipboard.
        restore_clipboard(&saved);
    }
    Ok(InjectOutcome::Pasted)
}

/// Writes `text` straight into the focused element of `pid`, returning whether
/// it landed there.
///
/// This is the mechanism a paste has always been a stand-in for. Setting
/// `AXSelectedText` replaces the current selection, or inserts at the caret
/// when nothing is selected - the same edit Cmd+V performs, except that it
/// names the element it is editing instead of hoping the right window has key
/// focus, leaves the pasteboard alone, and reports back.
///
/// Deliberately `AXSelectedText` and not `AXValue`, even though `AXValue`
/// settability is what `paste_target` reads as "this takes text": writing
/// `AXValue` replaces everything in the field, which would silently delete
/// whatever the user had already typed there.
///
/// `false` means nothing was written and the caller must fall back - the app
/// named no focused element (VS Code, terminals), the element does not accept
/// this write, or it accepted and then did nothing. That last case is why the
/// caret is read before and afterwards: an unchanged selection range is
/// positive evidence that no text was inserted, since inserting even one
/// character moves the caret past it. An unreadable range decides nothing and
/// the write is trusted, because a needless fall-back would paste the text a
/// second time.
/// The element currently holding focus inside `pid`, if the app names one.
fn focused_element(pid: TargetAppPid) -> Option<ax::Element> {
    ax::Element::for_app(pid).and_then(|app| app.focused())
}

/// Where the caret sits in `element`, if it will say. `None` is not a failure -
/// plenty of elements do not expose a selection - it just means an edit to this
/// element cannot be confirmed or denied afterwards.
fn caret(element: Option<&ax::Element>) -> Option<ax::CfRange> {
    element?.attribute_range(ax::SELECTED_TEXT_RANGE)
}

fn insert_via_accessibility(pid: TargetAppPid, text: &str) -> bool {
    let Some(focused) = focused_element(pid) else {
        log_inject("ax_insert unavailable: app named no focused element");
        return false;
    };
    if !focused.attribute_is_settable(ax::SELECTED_TEXT) {
        log_inject("ax_insert unavailable: AXSelectedText is not settable");
        return false;
    }
    let before = focused.attribute_range(ax::SELECTED_TEXT_RANGE);
    if let Err(error) = focused.set_attribute_string(ax::SELECTED_TEXT, text) {
        log_inject(&format!("ax_insert refused: AXError {error}"));
        return false;
    }
    let after = focused.attribute_range(ax::SELECTED_TEXT_RANGE);
    if let (Some(before), Some(after)) = (before, after) {
        if before == after {
            log_inject(&format!(
                "ax_insert accepted but the caret never moved (still at {}+{}); falling back to paste",
                before.location, before.length
            ));
            return false;
        }
    }
    log_inject(&format!(
        "ax_insert wrote {} characters, caret {:?} -> {:?}",
        text.chars().count(),
        before.map(|range| range.location),
        after.map(|range| range.location)
    ));
    true
}

/// The target app's name, for the log. A pid alone stops meaning anything the
/// moment the machine reboots, and the activation line is the one place a
/// paste that went to the wrong app can be traced back to an app at all.
/// Deliberately the app name and nothing from inside its windows - see
/// `paste_target::log_silent_app_diagnostics` on why titles stay out of here.
fn app_name(pid: TargetAppPid) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.localizedName())
        .map(|name| name.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Puts `text` on the clipboard and nothing else - no paste, no restore. Backs
/// the recovery widget's Copy button, where the user is explicitly asking for
/// the transcript and the clipboard replacement is the point.
pub fn copy_to_clipboard(text: &str) -> Result<(), AppError> {
    set_clipboard(text)
}

/// Reads every item and flavor off the general pasteboard. Lazily-promised
/// flavors are materialised by `dataForType`; anything unreadable is skipped.
pub(super) fn snapshot_clipboard() -> ClipboardSnapshot {
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
pub(super) fn restore_clipboard(snapshot: &ClipboardSnapshot) -> bool {
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

/// Posts a Cmd+`key` keystroke to the focused app - `"v"` to paste, `"c"` to
/// copy - as a real CoreGraphics event.
///
/// This used to go through System Events (`osascript ... keystroke "v" using
/// command down`), and that is why dictating into Claude silently did nothing:
/// **Electron apps ignore AppleScript-synthesised keystrokes**, while
/// `osascript` exits 0 regardless, because System Events accepting an event
/// says nothing about anything consuming it. Proven by A/B on 2026-08-28
/// against Claude's composer with the app frontmost and the field focused: the
/// CoreGraphics event pasted, the System Events one did not, same clipboard,
/// seconds apart.
///
/// A posted event is also layout-independent, where `keystroke "v"` asks the
/// current keyboard layout where "v" lives and can send a different physical
/// key on Dvorak or a non-Latin layout. And it costs no subprocess and no Apple
/// Event round trip per paste.
///
/// The trade is that posting reports nothing: `CGEventPost` returns void, and
/// without the Accessibility permission the event is dropped in silence rather
/// than refused with a message. So the permission is checked up front instead
/// of inferred from an error string - a direct question, and a more reliable
/// answer than parsing System Events' wording across macOS versions.
fn send_cmd_keystroke(key: &str, failure_code: &str) -> Result<(), AppError> {
    // kVK_ANSI_C / kVK_ANSI_V: physical key positions, not characters, so the
    // keyboard layout cannot redirect them.
    let keycode: u16 = match key {
        "c" => 8,
        "v" => 9,
        other => {
            return Err(AppError {
                code: failure_code.to_string(),
                message: format!("No key code is mapped for Cmd+{}.", other.to_uppercase()),
                details: None,
            })
        }
    };
    if !ax::is_trusted() {
        return Err(AppError {
            code: "dictation_accessibility_permission_required".to_string(),
            message: format!(
                "Cannot send Cmd+{} without the Accessibility permission.",
                key.to_uppercase()
            ),
            details: None,
        });
    }
    post_command_key(keycode).ok_or_else(|| AppError {
        code: failure_code.to_string(),
        message: format!("Could not create the Cmd+{} event.", key.to_uppercase()),
        details: None,
    })
}

/// `kCGEventSourceStateHIDSystemState` - the same source a real keyboard uses.
const CG_EVENT_SOURCE_HID_SYSTEM_STATE: i32 = 1;
/// `kCGHIDEventTap` - posted at the lowest point, so the event reaches apps
/// that filter out higher-level synthetic input.
const CG_HID_EVENT_TAP: u32 = 0;
/// `kCGEventFlagMaskCommand`.
const CG_FLAG_COMMAND: u64 = 1 << 20;
/// Gap between key-down and key-up. Zero works on most apps and not on all;
/// this is a keypress, and a keypress has a duration.
const KEY_HOLD: Duration = Duration::from_millis(20);

/// Posts one Cmd+keycode press and release. `None` if the events could not be
/// created, which in practice only happens when the process is out of event
/// sources. CoreGraphics is already linked by wry/tauri, so the symbols resolve
/// without adding them to the link line (same approach as `macos_cursor`).
fn post_command_key(keycode: u16) -> Option<()> {
    unsafe extern "C" {
        fn CGEventSourceCreate(state_id: i32) -> *const std::ffi::c_void;
        fn CGEventCreateKeyboardEvent(
            source: *const std::ffi::c_void,
            keycode: u16,
            key_down: bool,
        ) -> *const std::ffi::c_void;
        fn CGEventSetFlags(event: *const std::ffi::c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *const std::ffi::c_void);
        fn CFRelease(object: *const std::ffi::c_void);
    }
    // SAFETY: every pointer below is checked for null before use, each Create
    // call returns +1 and is released exactly once, and the flags/tap values
    // are the documented CoreGraphics constants.
    unsafe {
        let source = CGEventSourceCreate(CG_EVENT_SOURCE_HID_SYSTEM_STATE);
        let down = CGEventCreateKeyboardEvent(source, keycode, true);
        let up = CGEventCreateKeyboardEvent(source, keycode, false);
        let posted = if down.is_null() || up.is_null() {
            None
        } else {
            // Set explicitly rather than inherited: whatever modifiers the user
            // is physically holding - the dictation hotkey's own, moments
            // earlier - must not turn this into a different shortcut.
            CGEventSetFlags(down, CG_FLAG_COMMAND);
            CGEventSetFlags(up, CG_FLAG_COMMAND);
            CGEventPost(CG_HID_EVENT_TAP, down);
            std::thread::sleep(KEY_HOLD);
            CGEventPost(CG_HID_EVENT_TAP, up);
            Some(())
        };
        for event in [down, up, source] {
            if !event.is_null() {
                CFRelease(event);
            }
        }
        posted
    }
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

    #[test]
    fn a_target_that_never_comes_forward_is_never_pasted_into() {
        // The regression this whole guard exists for, from the app's own log
        // (2026-08-20): the target was accepted for activation, never actually
        // arrived, and Cmd+V went to whichever app was in front instead - 15
        // characters into a window the user was not dictating at, with their
        // previous clipboard restored over them 300ms later.
        //
        // pid 1 (launchd) can never become frontmost, so it stands in for any
        // target that does not come forward: an app that quit mid-dictation,
        // or one macOS declined to activate.
        let before = snapshot_clipboard();
        assert_eq!(
            inject_text("this must not go anywhere", Some(1))
                .expect("a target that never arrives is not an error"),
            InjectOutcome::TargetNotFrontmost
        );
        // ...and it must cost the user nothing: no transcript left on the
        // pasteboard, no clipboard of theirs replaced.
        assert_eq!(
            sorted(snapshot_clipboard()),
            sorted(before),
            "withholding the paste must leave the clipboard exactly as it was"
        );
    }

    #[test]
    fn there_is_always_a_paste_target_to_ask_about() {
        // Something is always frontmost, so the tracker having no reading yet
        // must not degrade into a target-less injection - that is the one path
        // with no activation guard, no paste-target check and no Accessibility
        // write, which pastes blind and calls it a success either way.
        assert!(
            capture_frontmost_app().is_some(),
            "capture_frontmost_app must fall back to a live reading"
        );
    }

    #[test]
    fn secure_input_state_is_readable() {
        // The FFI declaration is the whole risk here - a wrong symbol or
        // signature would be a link error or a garbage answer, and the result
        // decides whether a dictation is delivered or held back.
        let _: bool = secure_input_is_active();
    }

    #[test]
    fn accessibility_insertion_declines_rather_than_claiming_success() {
        // pid 1 names no focused element, so there is nothing to write into.
        // Answering `true` here would report a dictation as delivered while
        // skipping the clipboard fallback that would have delivered it.
        assert!(!insert_via_accessibility(1, "nowhere to put this"));
    }

    #[test]
    fn no_captured_target_still_pastes() {
        // Without a target pid there is nothing to ask about, so the paste
        // must go ahead exactly as it did before the check existed rather
        // than being suppressed on a guess.
        assert_eq!(
            inject_text("", None).expect("blank is a no-op"),
            InjectOutcome::Pasted
        );
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
