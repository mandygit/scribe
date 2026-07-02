# ADR-001: Capture system audio through a ScreenCaptureKit sidecar

## Status

Accepted

## Date

2026-05-08

## Context

Scribe needs to capture remote participant/system audio on macOS without installing an audio driver or asking for administrator privileges. The earlier ScreenCaptureKit spike proved that `SCStream` can capture non-silent system audio, but it also exposed two production constraints:

- ScreenCaptureKit is a Swift/Apple-framework integration that is not directly available from the existing Rust audio backend.
- macOS Screen Recording permission is tied to the app bundle identity, so the production flow must run from Scribe rather than a loose terminal binary.
- Audio should remain separately identifiable from the microphone WAV so later echo cancellation, channel attribution, and privacy controls can reason about source channels.

## Decision

Use a small Swift ScreenCaptureKit helper sidecar, built by `src-tauri/build.rs` and launched by the Rust audio backend as a child process. The existing Rust recording manager remains the orchestration point:

- Microphone audio continues to record as `{meetingId}.wav`.
- System audio records as `{meetingId}.system.m4a`.
- Rust stores optional system-audio metadata next to the existing meeting audio metadata.
- The helper is stopped through stdin when the recording stops; Rust treats helper errors as explicit metadata rather than hiding them.

This keeps the Rust backend's Strategy/Adapter shape intact while isolating macOS-specific ScreenCaptureKit code in the smallest native boundary.

## Alternatives Considered

### Rust FFI directly into ScreenCaptureKit

- Pros: Single process, direct error handling, no helper lifecycle.
- Cons: Higher unsafe/Objective-C interop complexity, harder to test, larger native surface in the Rust backend.
- Rejected because the sidecar preserves a smaller and easier-to-review boundary for the first production slice.

### Virtual audio driver

- Pros: Mature pattern for routing system audio into standard capture APIs.
- Cons: Requires installation/admin-like setup, conflicts with the no-driver local-first V1 requirement.
- Rejected because Scribe should work without requiring users to install audio infrastructure.

### Merge mic and system audio immediately

- Pros: One file for transcription and analysis.
- Cons: Loses channel identity before AEC and speaker attribution are implemented.
- Rejected because separate raw channels are safer for future echo cancellation and privacy controls.

## Consequences

- Packaging must include a target-specific helper binary generated before Tauri bundling.
- Screen Recording permission failures are expected user-facing errors and should remain actionable.
- The current transcription source remains the mic WAV until AEC/mixing is implemented.
- Future work can replace the helper with direct FFI or a richer native capture service without changing the frontend recording contract.
