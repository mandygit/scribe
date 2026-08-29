# ADR-006: Drive dictation from the Fn/Globe key with a listen-only event tap

## Status

Accepted

## Date

2026-08-29

## Context

Dictation was bound to a modifier chord (`⌃⌥D` by default). The gesture we
actually want is the one Wispr Flow uses: double-tap the Fn/Globe key, speak,
tap once to stop. A chord is worse for a gesture performed dozens of times a
day, and the Fn key is otherwise idle on most Macs.

macOS does not let a global-shortcut API bind it. Fn never produces a key-down
event: it surfaces only as a `flagsChanged` event carrying keycode 63 and the
`secondaryFn` flag. Carbon's `RegisterEventHotKey`, which backs
`tauri-plugin-global-shortcut`, fires on key *down*, and the crate's keycode
table has no entry for Fn at all. There is no configuration that makes this
work.

## Decision

1. **A `CGEventTap` of Scribe's own** (`dictation/fn_tap`) watches
   `flagsChanged` and feeds bare taps into the existing `DictationHotkey`
   state machine. Chords keep using the global-shortcut plugin;
   `register_dictation_hotkey` routes on the token and tears down whichever
   source it is replacing.
2. **The tap is listen-only.** Such a tap cannot alter or drop an event, so
   whatever the Globe key already does keeps working, and no other app's Fn
   binding can break. The worst failure is that Scribe fails to notice a
   press.
3. **Fn is the default for new installs.** Existing installs keep whatever
   they have persisted; the default governs fresh settings rows only.
4. **The system's own Globe action is reported, never rewritten.**
   `globe_key_is_free` reads `AppleFnUsageType`; when the key is taken,
   Settings explains what will happen and deep-links to the Keyboard pane.
5. **A self-test confirms the key arrives**, because a healthy tap proves
   nothing on its own, and offers a one-click fallback to `⌃⌥D` when it does
   not.

## Alternatives Considered

- **An active (consuming) tap that swallows bare Fn taps.** Removes the need
  for any system-settings change, since macOS never sees the tap. Rejected:
  it is precisely what would break other people's setups - a user with
  Raycast or superwhisper on Fn would find it dead whenever Scribe runs, with
  no visible culprit. Scribe cannot know at Fn-down whether a tap will turn
  out to be bare, so suppression also risks desyncing modifier state
  system-wide.
- **`NSEvent.addGlobalMonitorForEvents`.** Simpler API, same permission, but
  global monitors do not fire while Scribe itself is frontmost, so it would
  need a local monitor too. An event tap sees everything regardless of which
  app is front.
- **Leaving Fn as an opt-in and keeping a chord as the default.** Safer, but
  it makes the better gesture the one nobody discovers.
- **Requiring Input Monitoring.** Not needed: the tap is created under the
  Accessibility grant dictation already requires.

## Consequences

- Fn detection works only where macOS sees the key. Remapping tools
  (Karabiner) and many non-Apple external keyboards handle Fn in firmware and
  never pass it up, which is why the self-test and chord fallback exist.
- On a stock Mac the Globe key already does something, so a fresh install
  collides with the system until the user changes that one setting. Settings
  surfaces it; onboarding does not force it.
- The tap callback runs on the main thread under the system's tap-timeout. It
  therefore only decides what a press means and dispatches the actual work to
  the next run-loop turn - doing it inline would let the system disable the
  tap and cost the *next* press, most likely the one meant to stop the
  dictation it just started.
- The tap must be re-enabled on `tapDisabledByTimeout`, and torn down on the
  main thread, since freeing its state from another thread could race a press
  already in flight.
- Verified end to end on a MacBook Pro (M3 Pro, macOS 26.6): real double-tap
  starts a dictation, single tap stops and inserts, and `fn`+arrow / `fn`+F3
  do not misfire.
