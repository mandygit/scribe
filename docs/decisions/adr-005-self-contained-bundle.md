# ADR-005: Ship a self-contained app bundle by relinking Homebrew binaries

## Status

Accepted

## Date

2026-07-17

## Context

Until now a Scribe install only worked on machines that already had
`whisper-cli` (Homebrew), a whisper model somewhere on disk, ffmpeg, and
optionally SpeexDSP. That made the DMG useless to anyone who wasn't also a
developer of this project. We want "install the DMG, record, get a
transcript" with zero terminal steps for recipients, while keeping local-only
processing.

Three independent problems had to be solved: the whisper binary (and its
dylib tree), the whisper model, and audio conversion (previously ffmpeg).

## Decision

1. **whisper-cli**: `scripts/bundle-whisper-cli.sh` copies the pinned
   Homebrew keg (whisper-cpp 1.8.4 when this was decided, plus its ggml
   libraries, the dlopen'd
   Metal/BLAS/per-CPU-generation backends, and libomp) into
   `src-tauri/resources/whisper/` and rewrites every non-system load path to
   `@loader_path`, then re-signs each file ad hoc. ggml finds its backends by
   searching the executable's own directory once the compiled-in Homebrew
   path doesn't exist, so a flat folder is sufficient.
2. **Model**: `scripts/fetch-whisper-model.sh` stages a checksum-pinned
   `ggml-small-q5_1.bin` (~190 MB) into `src-tauri/resources/models/`;
   `small` fixes the proper-noun errors `base` makes, `q5_1` keeps the DMG
   size acceptable.
3. **Audio conversion**: conversion to mono s16 WAV goes through macOS's
   built-in `afconvert` first (`media_import::convert_to_mono_s16_wav`);
   ffmpeg remains only as a fallback for formats CoreAudio cannot decode
   (e.g. ogg vorbis).
4. **Path resolution**: `path_detection` prefers the bundled binary and model
   (dev tree `src-tauri/resources/` or `Contents/Resources/` in the packaged
   app) over Homebrew/`$PATH`/Spotlight, resolved at settings load time and
   never persisted, so moving the app cannot strand stored paths.
5. **SpeexDSP**: bundled the same relink-and-resign way
   (`scripts/bundle-speexdsp.sh`); the AEC dlopen candidate list checks the
   bundled copy first.

## Alternatives Considered

- **Build whisper.cpp from source (static, Metal-embedded)**: cleaner
  artifact and the original plan, but corporate endpoint security on the
  build machine deterministically SIGKILLs optimized clang compiles of large
  translation units (verified: `-O0` passes, `-O1+` killed, also when
  spawned via launchd). Relinking the Homebrew bottle sidesteps compilation
  entirely and ships byte-identical binaries to what we verified against.
- **Bundle ffmpeg**: 60+ MB and LGPL redistribution obligations for
  functionality macOS already provides via `afconvert`.
- **Download the model on first launch**: smaller DMG but adds a network
  dependency, progress UI, and failure modes; rejected in favor of a bigger
  but fully offline installer.
- **Rely on `GGML_BACKEND_PATH`**: the env var dlopens a single backend
  *file*, not a directory, so it cannot replace the executable-directory
  search.

## Updates

- **2026-08-28**: pinned whisper-cpp bumped 1.8.4 -> 1.9.2 after re-verifying
  transcription against it. The decision and mechanism are unchanged.

## Consequences

- The DMG grows to roughly 180 MB but works on a fresh Mac with no Homebrew.
- Verified by running the relocated `whisper-cli` under a `sandbox-exec`
  profile that denies reading `/opt/homebrew`: all backends load from the
  bundle and a real transcription succeeds.
- The build machine must have the pinned `whisper-cpp` installed via Homebrew.
  The version lives in `EXPECTED_WHISPER_VERSION` in
  `scripts/bundle-whisper-cli.sh`, which is the source of truth and fails
  loudly on drift, so transcription flags (`-mc` sizing, JSON output) stay
  verified against the shipped binary. Bumping it means re-verifying
  transcription first, then updating that constant.
- Recipients still install a local LLM server themselves for summaries
  (LM Studio/Ollama) - deliberately out of scope.
- Builds remain ad-hoc signed until there's an Apple Developer ID, so
  recipients do the one-time Gatekeeper unblock documented in
  `docs/installing.md`.
