//! Watches the Fn/Globe key so it can drive dictation, since no global-shortcut
//! API on macOS can bind it.
//!
//! Fn never produces a key-down event. It surfaces only as a `flagsChanged`
//! event carrying keycode 63 and the `secondaryFn` flag, and Carbon's
//! `RegisterEventHotKey` - which is what `tauri-plugin-global-shortcut` uses -
//! fires on key-down. That is why `hotkey_shortcut_for`'s allowlist can
//! never express this key, and why watching for it needs a CGEventTap here.
//!
//! The tap is deliberately **listen-only**. A listen-only tap cannot alter or
//! drop an event, so whatever the Globe key already does on the user's Mac -
//! switch input source, open the emoji picker, trigger another app's hotkey -
//! keeps working untouched. The worst this module can do is fail to notice a
//! press; it can never make a key stop working, which is the one failure mode
//! that would read as "Scribe broke my keyboard". The flip side is that Scribe
//! cannot suppress the system's own Globe action - that is a setting only the
//! user can change, so we detect it and explain rather than rewrite it.
//!
//! The timing logic that turns taps into start/stop lives in
//! [`super::hotkey::DictationHotkey`]; this module only decides what counts as
//! one deliberate tap of the key.

use std::ffi::c_void;
use std::time::Duration;

use objc2_foundation::NSString;

use crate::domain::AppError;

/// Longest a bare Fn press may be held and still count as a tap. Real taps
/// measure 76-110 ms on this hardware, so half a second is generous; the cap
/// exists so that resting a finger on the Globe key and letting go - a thing
/// that happens while reaching for fn+arrow and changing your mind - cannot
/// stop a dictation that is running.
pub const MAX_TAP_HOLD: Duration = Duration::from_millis(500);

/// Decides which Fn presses are deliberate bare taps, as opposed to Fn being
/// held as a modifier (fn+arrow, fn+F3, globe+E). Pure timing and bookkeeping,
/// so it is unit-testable without a tap, a keyboard, or a run loop - the same
/// split [`super::hotkey`] makes.
#[derive(Debug, Default)]
pub struct BareTapDetector {
    /// `Some(pressed_at_ms)` while Fn is held.
    pressed_at_ms: Option<u64>,
    /// Set when something during the hold ruled out a bare tap.
    disqualified: bool,
}

impl BareTapDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fn went down. Any other modifier already held makes this a chord.
    pub fn on_fn_down(&mut self, now_ms: u64, other_modifiers_held: bool) {
        self.pressed_at_ms = Some(now_ms);
        self.disqualified = other_modifiers_held;
    }

    /// Fn came up. Returns whether that completed one deliberate bare tap.
    pub fn on_fn_up(&mut self, now_ms: u64, other_modifiers_held: bool) -> bool {
        // No matching press: the tap was created while Fn was already held, or
        // an event was missed. Not something to act on.
        let Some(pressed_at_ms) = self.pressed_at_ms.take() else {
            return false;
        };
        let disqualified = std::mem::replace(&mut self.disqualified, false);
        !disqualified
            && !other_modifiers_held
            && now_ms.saturating_sub(pressed_at_ms) <= MAX_TAP_HOLD.as_millis() as u64
    }

    /// Any other keyboard event happened. While Fn is held this makes the press
    /// a modifier chord rather than a tap; at any other time it is irrelevant -
    /// which matters, because the system's own Globe action posts a key event
    /// just *after* each tap, and that must not poison the next one.
    pub fn on_other_key(&mut self) {
        if self.pressed_at_ms.is_some() {
            self.disqualified = true;
        }
    }
}

/// Whether the system leaves the Globe/Fn key alone, so a tap of it means only
/// what Scribe makes it mean.
///
/// A listen-only tap cannot stop macOS acting on the key as well, so with any
/// other setting a dictation double-tap also fires the system's action twice
/// and the single tap that stops it fires it once more. Switching input source
/// is the damaging one: the stop tap leaves the user on a different keyboard
/// layout than they started typing in. Scribe reports this and points at the
/// setting; it never rewrites a system preference on the user's behalf.
///
/// `AppleFnUsageType` is unset by default, and that default is not "do nothing",
/// so an absent key reads as taken rather than free. `0` was confirmed to be
/// what System Settings writes for "Do Nothing" (macOS 26.6); the other values
/// are deliberately not enumerated, since the only question here is whether the
/// key is ours.
pub fn globe_key_is_free() -> bool {
    const DO_NOTHING: isize = 0;
    let key = NSString::from_str("AppleFnUsageType");
    let application_id = NSString::from_str("com.apple.HIToolbox");
    let mut valid = false;
    unsafe {
        // The user can change this while Scribe runs, and CFPreferences caches
        // aggressively; without this the answer can be stale for the life of
        // the process, which is exactly the moment the user has just gone and
        // fixed it and come back expecting the warning to clear.
        CFPreferencesAppSynchronize(application_id.as_ref());
        let value = CFPreferencesGetAppIntegerValue(
            key.as_ref(),
            application_id.as_ref(),
            &mut valid as *mut bool,
        );
        valid && value == DO_NOTHING
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFPreferencesGetAppIntegerValue(
        key: *const NSString,
        application_id: *const NSString,
        key_exists_and_has_valid_format: *mut bool,
    ) -> isize;
    fn CFPreferencesAppSynchronize(application_id: *const NSString) -> bool;
}

// --- CoreGraphics / CoreFoundation FFI ------------------------------------
//
// Declared here rather than pulled in as a crate, matching how `super::ax`
// talks to the Accessibility APIs. Every constant below was read out of the
// SDK's own headers rather than transcribed from memory.

/// `kVK_Function`: the keycode Fn reports on its `flagsChanged` event.
const VK_FUNCTION: i64 = 63;
/// `kCGKeyboardEventKeycode`.
const KEYBOARD_EVENT_KEYCODE: u32 = 9;

/// `kCGEventFlagMaskSecondaryFn` - set while Fn is down.
const FLAG_SECONDARY_FN: u64 = 0x0080_0000;
/// The modifiers whose presence turns an Fn press into a chord.
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_ALTERNATE: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;
const OTHER_MODIFIER_FLAGS: u64 = FLAG_SHIFT | FLAG_CONTROL | FLAG_ALTERNATE | FLAG_COMMAND;

#[repr(C)]
#[derive(Clone, Copy)]
enum CGEventTapLocation {
    Session = 1,
}

#[repr(u32)]
#[derive(Clone, Copy)]
enum CGEventTapPlacement {
    HeadInsertEventTap = 0,
}

#[repr(u32)]
#[derive(Clone, Copy)]
enum CGEventTapOptions {
    ListenOnly = 1,
}

const EVENT_TYPE_KEY_DOWN: u32 = 10;
const EVENT_TYPE_FLAGS_CHANGED: u32 = 12;
/// The out-of-band types the system delivers when it switches a tap off.
const EVENT_TYPE_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const EVENT_TYPE_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

enum CGEvent {}
type CGEventRef = *const CGEvent;
type CGEventTapProxy = *const c_void;
type CGEventMask = u64;
type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *const c_void,
) -> CGEventRef;

enum CFMachPort {}
type CFMachPortRef = *mut CFMachPort;
enum CFRunLoop {}
type CFRunLoopRef = *mut CFRunLoop;
enum CFRunLoopSource {}
type CFRunLoopSourceRef = *mut CFRunLoopSource;
enum CFAllocator {}
type CFAllocatorRef = *mut CFAllocator;
enum CFString {}
type CFStringRef = *const CFString;
type CFTypeRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *const c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFAllocatorDefault: CFAllocatorRef;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRelease(cf: CFTypeRef);
}

/// Everything the C callback needs, kept alive by [`FnKeyTap`] for exactly as
/// long as the tap can fire.
struct TapState {
    detector: BareTapDetector,
    on_bare_tap: Box<dyn Fn() + Send>,
    /// Needed to switch the tap back on after the system disables it.
    port: CFMachPortRef,
}

/// A live listen-only tap on the Fn key. Dropping it tears the tap down.
///
/// **Create and drop this on the main thread.** It adds and removes a source on
/// the main run loop, and its callback runs there; tearing down from another
/// thread could free `TapState` while a callback is mid-flight.
pub struct FnKeyTap {
    port: CFMachPortRef,
    source: CFRunLoopSourceRef,
    /// Owned here so it outlives every callback, and is freed only in `drop`
    /// after the port has been invalidated.
    state: *mut TapState,
}

// The raw pointers are only ever touched on the main thread (see the type's
// docs); this lets the handle live in `AppState` alongside everything else.
unsafe impl Send for FnKeyTap {}
unsafe impl Sync for FnKeyTap {}

impl FnKeyTap {
    /// Starts watching for bare Fn taps, calling `on_bare_tap` for each one.
    ///
    /// `on_bare_tap` runs on the main thread inside the event tap callback, so
    /// it must return promptly and must not run a nested run loop: a callback
    /// that takes too long gets the tap switched off by the system (handled,
    /// but it drops presses), and a nested run loop could re-enter it.
    pub fn start<F>(on_bare_tap: F) -> Result<Self, AppError>
    where
        F: Fn() + Send + 'static,
    {
        let state = Box::into_raw(Box::new(TapState {
            detector: BareTapDetector::new(),
            on_bare_tap: Box::new(on_bare_tap),
            port: std::ptr::null_mut(),
        }));

        let mask: CGEventMask =
            (1 << EVENT_TYPE_FLAGS_CHANGED as u64) | (1 << EVENT_TYPE_KEY_DOWN as u64);

        let port = unsafe {
            CGEventTapCreate(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                mask,
                tap_callback,
                state as *const c_void,
            )
        };
        if port.is_null() {
            // Reclaim the state the tap never took ownership of.
            drop(unsafe { Box::from_raw(state) });
            return Err(AppError {
                code: "dictation_fn_tap_unavailable".to_string(),
                message: "Could not watch the Fn key. Scribe needs Accessibility permission."
                    .to_string(),
                details: None,
            });
        }
        unsafe { (*state).port = port };

        let source = unsafe { CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0) };
        if source.is_null() {
            unsafe {
                CFMachPortInvalidate(port);
                CFRelease(port as CFTypeRef);
            }
            drop(unsafe { Box::from_raw(state) });
            return Err(AppError {
                code: "dictation_fn_tap_unavailable".to_string(),
                message: "Could not attach the Fn key watcher to the run loop.".to_string(),
                details: None,
            });
        }

        unsafe {
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(port, true);
        }

        Ok(Self {
            port,
            source,
            state,
        })
    }
}

impl Drop for FnKeyTap {
    fn drop(&mut self) {
        unsafe {
            // Order matters: stop the tap and unhook the source before freeing
            // the state the callback reads, or a queued callback reads freed
            // memory.
            CGEventTapEnable(self.port, false);
            CFRunLoopRemoveSource(CFRunLoopGetMain(), self.source, kCFRunLoopCommonModes);
            CFMachPortInvalidate(self.port);
            CFRelease(self.source as CFTypeRef);
            CFRelease(self.port as CFTypeRef);
            drop(Box::from_raw(self.state));
        }
    }
}

/// The C entry point. Kept to bookkeeping and one call out, so the tap's
/// timeout budget is never at risk from work done here.
unsafe extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *const c_void,
) -> CGEventRef {
    let state = &mut *(user_info as *mut TapState);

    // The system switches a tap off if its callback overruns, or across some
    // user input. Neither is fatal, but the tap stays dead until re-enabled -
    // which is the classic way an event tap silently stops working.
    if event_type == EVENT_TYPE_TAP_DISABLED_BY_TIMEOUT
        || event_type == EVENT_TYPE_TAP_DISABLED_BY_USER_INPUT
    {
        CGEventTapEnable(state.port, true);
        return event;
    }

    let Ok(now_ms) = crate::current_time_ms() else {
        return event;
    };
    let flags = CGEventGetFlags(event);
    let other_modifiers_held = flags & OTHER_MODIFIER_FLAGS != 0;
    let is_fn_key = event_type == EVENT_TYPE_FLAGS_CHANGED
        && CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) == VK_FUNCTION;

    let completed_tap = if is_fn_key {
        if flags & FLAG_SECONDARY_FN != 0 {
            state.detector.on_fn_down(now_ms, other_modifiers_held);
            false
        } else {
            state.detector.on_fn_up(now_ms, other_modifiers_held)
        }
    } else {
        // Every other keyboard event, including other modifiers changing.
        state.detector.on_other_key();
        false
    };

    if completed_tap {
        (state.on_bare_tap)();
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::hotkey::{DictationHotkey, HotkeyAction};

    #[test]
    fn a_quick_press_and_release_is_a_tap() {
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        assert!(detector.on_fn_up(1_090, false));
    }

    #[test]
    fn a_key_pressed_during_the_hold_makes_it_a_chord() {
        // fn+ArrowLeft, fn+F3: the other key lands between down and up.
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        detector.on_other_key();
        assert!(!detector.on_fn_up(1_090, false));
    }

    #[test]
    fn a_key_pressed_after_release_does_not_poison_the_next_tap() {
        // The system's own Globe action posts a key event just after each tap.
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        assert!(detector.on_fn_up(1_090, false));
        detector.on_other_key();
        detector.on_fn_down(1_270, false);
        assert!(detector.on_fn_up(1_360, false));
    }

    #[test]
    fn holding_fn_too_long_is_not_a_tap() {
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        assert!(!detector.on_fn_up(1_000 + MAX_TAP_HOLD.as_millis() as u64 + 1, false));
    }

    #[test]
    fn holding_fn_right_up_to_the_limit_still_taps() {
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        assert!(detector.on_fn_up(1_000 + MAX_TAP_HOLD.as_millis() as u64, false));
    }

    #[test]
    fn another_modifier_at_either_end_makes_it_a_chord() {
        let mut held_at_down = BareTapDetector::new();
        held_at_down.on_fn_down(1_000, true);
        assert!(!held_at_down.on_fn_up(1_090, false));

        let mut held_at_up = BareTapDetector::new();
        held_at_up.on_fn_down(1_000, false);
        assert!(!held_at_up.on_fn_up(1_090, true));
    }

    #[test]
    fn a_release_without_a_press_is_not_a_tap() {
        // The tap can be created while Fn is already held down.
        let mut detector = BareTapDetector::new();
        assert!(!detector.on_fn_up(1_000, false));
    }

    #[test]
    fn a_chord_does_not_disqualify_the_tap_after_it() {
        let mut detector = BareTapDetector::new();
        detector.on_fn_down(1_000, false);
        detector.on_other_key();
        assert!(!detector.on_fn_up(1_090, false));
        detector.on_fn_down(2_000, false);
        assert!(detector.on_fn_up(2_090, false));
    }

    #[test]
    fn an_absent_preference_is_not_mistaken_for_do_nothing() {
        // The trap this guards: CFPreferences returns 0 for a key that isn't
        // there, and 0 is exactly the "Do Nothing" value. Without checking
        // `keyExistsAndHasValidFormat`, every default install - where the key
        // is unset - would report the Globe key as free and never warn.
        let key = NSString::from_str("ScribeNoSuchPreferenceKey");
        let application_id = NSString::from_str("com.apple.HIToolbox");
        let mut valid = true;
        let value = unsafe {
            CFPreferencesGetAppIntegerValue(
                key.as_ref(),
                application_id.as_ref(),
                &mut valid as *mut bool,
            )
        };
        assert!(!valid, "an absent key must not report a valid value");
        assert_eq!(value, 0, "and it returns the same 0 that means Do Nothing");
    }

    #[test]
    fn two_taps_drive_the_dictation_hotkey_to_start() {
        // The whole point, end to end: the gaps and holds are the ones the
        // probe measured from a real Globe key (~90 ms held, ~180 ms apart).
        let mut detector = BareTapDetector::new();
        let mut hotkey = DictationHotkey::new();

        detector.on_fn_down(1_000, false);
        assert!(detector.on_fn_up(1_090, false));
        assert_eq!(hotkey.on_press(1_090), HotkeyAction::None);

        detector.on_fn_down(1_270, false);
        assert!(detector.on_fn_up(1_360, false));
        assert_eq!(hotkey.on_press(1_360), HotkeyAction::StartRecording);

        // A single later tap stops it.
        detector.on_fn_down(5_000, false);
        assert!(detector.on_fn_up(5_090, false));
        assert_eq!(hotkey.on_press(5_090), HotkeyAction::StopRecording);
    }
}


