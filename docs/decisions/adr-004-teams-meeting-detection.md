# ADR-004: Detect live Microsoft Teams calls via a window-geometry Swift sidecar

## Status

Accepted

## Date

2026-07-14

## Context

The user wants Scribe to prompt "Record this meeting?" as soon as they join a
Microsoft Teams call, mirroring Notion's meeting-note popup. Two decisions
shaped the design:

- **Which platforms.** V1 covers only the Microsoft Teams desktop app. Zoom
  and browser-based Google Meet were explicitly out of scope for this slice.
- **Which moment to detect.** Notion's popup appears on the pre-join screen,
  before the user clicks "Join". Reaching that exact moment reliably would
  require reading in-app UI text (button labels like "Join now") via the
  macOS Accessibility API, walking Teams' UI element tree. That approach is
  fragile — it breaks silently whenever Microsoft ships a UI update — and
  needs broader Accessibility permission scope than anything else in Scribe.
  Detecting "the user is now in a live call" is a few seconds later but
  depends only on window ownership and geometry, which survives Teams UI/copy
  redesigns far better than reading textual content would.

## Decision

Detection runs entirely in a new Swift sidecar,
`native/meeting-detector/main.swift`, compiled and bundled the same way as
the existing two sidecars (`build.rs`, `tauri.conf.json`'s `externalBin`) —
consistent with ADR-001's precedent that privileged macOS API access belongs
in a small Swift binary, not raw Rust FFI.

The sidecar polls every ~2 seconds:

1. `NSWorkspace.shared.runningApplications` for Microsoft Teams' bundle
   identifier (`com.microsoft.teams2`, with a fallback check for the retired
   classic client's `com.microsoft.teams`). No permission required.
2. If Teams is running, `CGWindowListCopyWindowInfo` enumerates its on-screen
   windows and checks for one matching the geometry of Teams' floating
   meeting-controls toolbar (a small, fixed-size window that appears
   specifically while in a call) — a **structural**, not textual, signature.
3. Prints exactly one line (`IN_CALL` / `NOT_IN_CALL`) to stdout, only on a
   transition, which the Rust side reads via a blocking `BufReader::lines()`
   thread.

Reading other processes' window titles via `CGWindowListCopyWindowInfo`
needs Screen Recording permission on macOS 10.15+ — the same permission
Scribe already requests for system audio capture (ADR-001), so this feature
adds no new permission prompt for most users.

On the Rust side (`src-tauri/src/meeting_detection/mod.rs`), a pure state
machine (`advance(state, event, recording_already_active)`) turns sidecar
lines into `ShowPrompt` / `HidePrompt` actions, decoupled from Tauri so it's
directly unit-testable against a fixed sequence of lines. `lib.rs` wires this
to a floating, non-activating popup window (`create_meeting_popup`,
`set_meeting_popup_visible`) built the same way as the existing dictation
pill — a `WebviewWindowBuilder` window converted to an NSPanel via
`tauri-nspanel`, so a click never steals focus from the Teams call the user
is in.

Detection is opt-in via `ScribeSettings.prompt_on_teams_meeting` (default
`true`), toggleable live from Settings without an app restart
(`update_meeting_detection_settings` starts/stops the sidecar immediately).
The sidecar is also stopped explicitly on app quit via a `tauri::RunEvent::Exit`
handler — unlike the system-audio-capture sidecar, which only runs for the
bounded duration of an active recording, this one loops for the app's entire
lifetime whenever the setting is on, so leaving it running past app quit
would leak an orphaned process.

**The exact call-toolbar window dimensions are a starting estimate, not a
confirmed signature** — Teams' actual on-screen geometry for this window
couldn't be observed without a live call during development. The sidecar
ships a `--debug-log-windows` mode that dumps every Teams-owned window's
title/bounds every tick; this must be run against one real Teams meeting to
confirm or correct the `callToolbarWidthRange` / `callToolbarHeightRange`
constants before the detection can be trusted end-to-end.

## Alternatives Considered

### Accessibility-tree text scraping for the pre-join screen

- Pros: Matches Notion's exact pre-join timing; the original ask.
- Cons: Ties detection to Teams' current button labels/UI structure, which
  changes across releases with no warning; needs broader Accessibility scope
  than anything else in Scribe requests today.
- Rejected: an extra few seconds of latency (prompting on live-call instead
  of pre-join) is a small UX cost next to a detector that silently breaks on
  the next Teams update.

### Raw Rust FFI to Core Graphics/AppKit instead of a Swift sidecar

- Pros: One fewer compiled binary; avoids the `xcrun swiftc` build step.
- Cons: Breaks the established pattern (ADR-001) of keeping privileged
  Apple-framework calls in Swift sidecars, and would need `objc`/`core-graphics`
  Rust crates the codebase currently avoids entirely.
- Rejected for consistency with the existing two sidecars.

### Polling via a persistently-spawned one-shot process instead of a long-lived sidecar

- Pros: No long-lived child process to manage or clean up on quit.
- Cons: Spawning a fresh process every ~2 seconds for the app's entire
  runtime is far heavier than one persistent process with an internal sleep
  loop, and loses the "only print on transition" simplification.
- Rejected in favor of a persistent sidecar, matching how
  `system-audio-capture` is already a long-lived controlled process.

## Consequences

- Detection latency is "a few seconds into the call," not pre-join — a
  known, deliberate trade-off, not a bug.
- The floating-toolbar geometry heuristic needs empirical confirmation
  (`--debug-log-windows`) before it can be trusted, and may need
  re-tuning if Microsoft changes the toolbar's size in a future Teams
  release — a one-line constant change in `main.swift`, not a redesign.
- Zoom, Google Meet, and other meeting platforms are unhandled; adding one
  means teaching the same sidecar (or a sibling one) that platform's
  process/window signature, following the same structural-geometry-over-text
  principle established here.
- The meeting-detector sidecar is the first long-lived (app-lifetime)
  child process Scribe manages, which is why it needed explicit
  `RunEvent::Exit` cleanup that the bounded-lifetime sidecars didn't.

## Addendum (2026-07-14/15): signals confirmed against real Teams sessions

The toolbar-geometry heuristic above never matched: the new Teams client has
no separate floating call-controls window at all. Two live sessions
(diagnostic log `~/Library/Logs/Scribe/meeting-detector.log`) established
what actually exists, and detection now works as follows:

1. **Primary signal — audio input.** Any Core Audio HAL process entry whose
   bundle id belongs to Teams (`com.microsoft.teams2`, including `.helper`
   subprocesses, where the capture actually runs) with
   `kAudioProcessPropertyIsRunningInput` true. This is the same mechanism as
   the system's orange mic indicator, and fires from the pre-join mic check
   onward.
2. **Fallback signal — a meeting window.** Joining a call opens a *second*
   Teams window titled `<Meeting Name> | Microsoft Teams`, alongside the
   main nav window that stays open throughout.

The nav window's own title is `<Tab> | Microsoft Teams` while idling on a
left-rail tab (`Chat`, `Calendar`, …) **and `<Tab> | <open item> |
Microsoft Teams` when something is open in that tab** — e.g. viewing the
conversation "AIE" titles it `Chat | AIE | Microsoft Teams`. A window is
therefore treated as a meeting window only when the *first* `" | "`-separated
segment of its title is not a known nav-tab name. Comparing the whole
prefix (the original implementation) misread every open conversation as a
live call, popping the record prompt when the user merely clicked around
Teams.

## Addendum (2026-08-06): title fallback false-positives on popped-out chat windows

Confirmed live: using Teams' "Open in new window" on a chat pops it into its
own window titled `<Chat name> | Microsoft Teams` — with no "Chat |" nav
prefix, since it's no longer inside the nav window. That's structurally
identical to a real meeting window's `<Meeting Name> | Microsoft Teams`
title; nothing in the text distinguishes them. The fallback signal
misfired, showing the record prompt for a window that was never a call.

Fix: `currentCallState()` now only trusts the title-based fallback while
Teams' mic was seen running within the last 60 seconds
(`lastAudioActiveAt` / `fallbackAudioRecencyWindow` in `main.swift`). A real
call always runs the mic at least briefly (the primary signal's own
pre-join mic-check trigger), so this preserves the fallback's original
purpose — bridging a momentary mic gap mid-call — while a chat/channel
window that never had any mic activity can no longer trip it. A call
joined muted from the very start, with the mic literally never active even
during the pre-join screen, would go undetected until audio starts (a
known, accepted trade-off — no live session has exhibited that pattern so
far, only ordinary Teams navigation being misread as a call).
