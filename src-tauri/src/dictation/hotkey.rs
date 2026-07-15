//! Wispr-style press-pattern detector for the dictation hotkey. A double-press
//! of the shortcut starts recording; a single press while recording stops it.
//! This module owns only the timing logic so it can be unit-tested without the
//! global-shortcut plugin or any audio devices.

use std::time::Duration;

/// How close two presses must be to count as the double-press that starts a
/// dictation. Generous enough for a deliberate two-tap of a single key.
pub const DOUBLE_PRESS_WINDOW: Duration = Duration::from_millis(600);

/// Shortest a dictation can run before a press is allowed to stop it. This guards
/// against the start double-press's second tap (or key auto-repeat) instantly
/// stopping the recording it just started.
pub const MIN_RECORDING_BEFORE_STOP: Duration = Duration::from_millis(400);

/// What a hotkey press should do, given the current dictation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Ignore this press (a lone idle tap, or a too-soon stop).
    None,
    /// Begin capturing dictation audio.
    StartRecording,
    /// Stop capturing and transcribe + inject.
    StopRecording,
}

/// Tracks dictation hotkey presses and decides what each one means.
#[derive(Debug, Default)]
pub struct DictationHotkey {
    /// `Some(started_ms)` while a dictation is recording.
    recording_since_ms: Option<u64>,
    /// The last idle press awaiting a partner to form a double-press.
    pending_press_ms: Option<u64>,
}

impl DictationHotkey {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_recording(&self) -> bool {
        self.recording_since_ms.is_some()
    }

    /// Marks a dictation as started at `now_ms` from outside the press flow (the
    /// pill's toggle button), so a subsequent hotkey press correctly stops it.
    pub fn mark_recording_started(&mut self, now_ms: u64) {
        self.recording_since_ms = Some(now_ms);
        self.pending_press_ms = None;
    }

    /// Marks the dictation stopped from outside the press flow (the pill's toggle
    /// button), so a subsequent hotkey press starts a fresh dictation.
    pub fn mark_recording_stopped(&mut self) {
        self.recording_since_ms = None;
        self.pending_press_ms = None;
    }

    /// Records a hotkey press at `now_ms` and returns the action to take.
    pub fn on_press(&mut self, now_ms: u64) -> HotkeyAction {
        if let Some(started_ms) = self.recording_since_ms {
            // While recording, a press stops — but not until the recording has
            // run long enough that this can't be the double-press's second tap.
            if now_ms.saturating_sub(started_ms) < MIN_RECORDING_BEFORE_STOP.as_millis() as u64 {
                return HotkeyAction::None;
            }
            self.recording_since_ms = None;
            self.pending_press_ms = None;
            return HotkeyAction::StopRecording;
        }

        // Idle: a second press close behind the first arms recording.
        let is_double = self.pending_press_ms.is_some_and(|first| {
            now_ms.saturating_sub(first) <= DOUBLE_PRESS_WINDOW.as_millis() as u64
        });
        if is_double {
            self.recording_since_ms = Some(now_ms);
            self.pending_press_ms = None;
            HotkeyAction::StartRecording
        } else {
            self.pending_press_ms = Some(now_ms);
            HotkeyAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_idle_press_does_nothing() {
        let mut hotkey = DictationHotkey::new();
        assert_eq!(hotkey.on_press(1_000), HotkeyAction::None);
        assert!(!hotkey.is_recording());
    }

    #[test]
    fn double_press_starts_recording() {
        let mut hotkey = DictationHotkey::new();
        assert_eq!(hotkey.on_press(1_000), HotkeyAction::None);
        assert_eq!(hotkey.on_press(1_300), HotkeyAction::StartRecording);
        assert!(hotkey.is_recording());
    }

    #[test]
    fn two_slow_presses_do_not_start() {
        let mut hotkey = DictationHotkey::new();
        assert_eq!(hotkey.on_press(1_000), HotkeyAction::None);
        // 800 ms apart is past the double-press window.
        assert_eq!(hotkey.on_press(1_800), HotkeyAction::None);
        assert!(!hotkey.is_recording());
    }

    #[test]
    fn single_press_stops_after_recording() {
        let mut hotkey = DictationHotkey::new();
        hotkey.on_press(1_000);
        hotkey.on_press(1_300); // start at 1_300
        assert_eq!(hotkey.on_press(5_000), HotkeyAction::StopRecording);
        assert!(!hotkey.is_recording());
    }

    #[test]
    fn too_soon_press_does_not_stop_recording() {
        let mut hotkey = DictationHotkey::new();
        hotkey.on_press(1_000);
        hotkey.on_press(1_300); // start at 1_300
                                // Auto-repeat / second tap 100 ms later must not stop it.
        assert_eq!(hotkey.on_press(1_400), HotkeyAction::None);
        assert!(hotkey.is_recording());
    }

    #[test]
    fn lone_press_soon_after_stop_does_not_restart() {
        let mut hotkey = DictationHotkey::new();
        hotkey.on_press(1_000);
        hotkey.on_press(1_300); // start
        assert_eq!(hotkey.on_press(5_000), HotkeyAction::StopRecording);
        // A single press 200 ms after stopping must not look like a double-press
        // relative to the stop press.
        assert_eq!(hotkey.on_press(5_200), HotkeyAction::None);
        assert!(!hotkey.is_recording());
    }

    #[test]
    fn pill_start_then_hotkey_press_stops() {
        // Started via the pill toggle, the tracker must treat the next hotkey
        // press (after the minimum recording time) as a stop.
        let mut hotkey = DictationHotkey::new();
        hotkey.mark_recording_started(1_000);
        assert!(hotkey.is_recording());
        assert_eq!(hotkey.on_press(5_000), HotkeyAction::StopRecording);
        assert!(!hotkey.is_recording());
    }

    #[test]
    fn pill_stop_then_hotkey_needs_fresh_double_press() {
        // Stopped via the pill toggle, a lone hotkey press must not restart; a
        // fresh double-press is required.
        let mut hotkey = DictationHotkey::new();
        hotkey.mark_recording_started(1_000);
        hotkey.mark_recording_stopped();
        assert!(!hotkey.is_recording());
        assert_eq!(hotkey.on_press(2_000), HotkeyAction::None);
        assert_eq!(hotkey.on_press(2_200), HotkeyAction::StartRecording);
    }

    #[test]
    fn full_cycle_can_repeat() {
        let mut hotkey = DictationHotkey::new();
        hotkey.on_press(1_000);
        assert_eq!(hotkey.on_press(1_200), HotkeyAction::StartRecording);
        assert_eq!(hotkey.on_press(4_000), HotkeyAction::StopRecording);
        // New dictation: fresh double-press.
        assert_eq!(hotkey.on_press(6_000), HotkeyAction::None);
        assert_eq!(hotkey.on_press(6_200), HotkeyAction::StartRecording);
    }
}
