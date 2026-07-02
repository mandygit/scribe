# ADR-002: Use offline SpeexDSP AEC as an optional transcription preprocessor

## Status

Accepted

## Date

2026-05-08

## Context

Scribe needs acoustic echo cancellation so remote participant/system audio bleed in the microphone channel does not contaminate the user's transcript. The validation spike proved SpeexDSP can reduce synthetic echo by more than the V1 quality threshold, but production integration has two constraints:

- Raw microphone and system audio must remain intact even when AEC fails.
- The main app must still build and run on machines where SpeexDSP is not installed.
- The current ScreenCaptureKit sidecar persists system audio as `.m4a`; full AEC processing requires an aligned 48 kHz mono PCM WAV reference channel.

## Decision

Add an `EchoCancellationBackend` strategy with a `SpeexEchoCancellationBackend` adapter that runtime-loads `libspeexdsp.dylib` through `dlopen` instead of hard-linking it. Transcription selects the audio source as follows:

- If echo cancellation is disabled, transcribe the raw microphone WAV.
- If echo cancellation is enabled and a compatible reference WAV is available, write `{meetingId}.aec.wav` and transcribe that derived file.
- If SpeexDSP is unavailable, the reference format is unsupported, channels are not aligned, or processing fails, log the AEC error and transcribe the raw microphone WAV.

The settings API exposes `enableEchoCancellation` separately from `enableSystemAudio` so users can capture separate channels while leaving AEC off.

## Alternatives Considered

### Hard-link SpeexDSP at build time

- Pros: Simpler FFI calls and earlier failure if the native library is missing.
- Cons: Breaks app builds on machines without SpeexDSP installed.
- Rejected because AEC must be an optional enhancement with no-data-loss fallback.

### Convert `.m4a` system audio to WAV inside the Rust adapter

- Pros: Would let the current ScreenCaptureKit sidecar output feed AEC immediately.
- Cons: Requires adding or shelling out to an audio decoder, increasing dependency and packaging risk.
- Rejected for this slice because the user has not approved new dependencies and raw-channel persistence must remain the priority.

### Real-time AEC during capture

- Pros: Enables cleaned audio to feed future streaming transcription directly.
- Cons: Requires tighter timestamp alignment, frame buffering, and native dependency hardening.
- Deferred because the offline adapter establishes the Strategy boundary while preserving the current recording pipeline.

## Consequences

- AEC is additive: it creates a derived cleaned WAV only when all prerequisites are satisfied.
- Missing or incompatible AEC support never blocks recording or transcription.
- Future work should either make the system-audio sidecar produce an aligned PCM reference channel or add an approved local decoder before expecting AEC to run on the default `.m4a` reference.
