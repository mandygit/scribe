# Distributing to other Macs

## What's inside the package

The DMG is self-contained. `bun run package:prepare` (run automatically by
the package commands) assembles into `src-tauri/resources/`:

- `whisper/` — a relocatable copy of Homebrew's `whisper-cli` 1.8.4 with its
  ggml libraries and dlopen'd backends (Metal, BLAS, per-CPU variants), all
  load paths rewritten to `@loader_path` so no Homebrew is needed at runtime
  (`scripts/bundle-whisper-cli.sh`).
- `models/ggml-small-q5_1.bin` — the default whisper model (~190 MB,
  checksum-verified; `scripts/fetch-whisper-model.sh`).
- `lib/libspeexdsp.dylib` — echo cancellation (`scripts/bundle-speexdsp.sh`).

The three Swift sidecar helpers (system audio capture, dictation polish,
meeting detection) are compiled by `cargo` and bundled as Tauri
`externalBin`s as before. Audio conversion uses macOS's built-in `afconvert`
(ffmpeg is only an optional fallback for exotic formats like ogg).

Recipients therefore install **nothing** for recording and transcription.
The one thing they still need is a local LLM server for summaries
(LM Studio or Ollama) — see "On first launch" below.

Scribe isn't signed with a paid Apple Developer ID yet, so builds are ad-hoc
signed (Tauri's default when no `signingIdentity` is set in
`tauri.conf.json`) rather than notarized. That's enough to hand the app to
your own team, but Gatekeeper will flag it as coming from an "unidentified
developer" the first time each person opens it.

## Building the DMG

```bash
bun run package:mac:dmg
```

This produces `src-tauri/target/release/bundle/dmg/Scribe_<version>_aarch64.dmg`
(or `x64` on Intel Macs). Share that file however you'd share any other
internal build, ideally together with [installing.md](installing.md) - the
standalone install guide for recipients (prerequisites, the Gatekeeper
one-time step, permissions, and local-model setup).

## One-time step for each dev

After copying the app to `/Applications`, macOS blocks it from opening
normally because it isn't notarized. Either:

- **Right-click the app → Open**, then confirm in the dialog that appears (only needed once), or
- Run in Terminal:

  ```bash
  xattr -cr /Applications/Scribe.app
  ```

After that, it opens like any other app — no warning, no terminal needed for
subsequent launches.

## On first launch

Scribe checks Microphone, Screen Recording, and Accessibility permissions and
walks through granting them. None of these are hard requirements: without
Microphone, meeting recording is disabled with a clear reason; without Screen
Recording, meetings record mic-only; without Accessibility, dictation can't
insert text into other apps but everything else works. Devs can skip the
onboarding screen and revisit it later from **Settings → Permissions**.

Each dev also needs a local model server running — see the **Local model**
settings group for LM Studio, Ollama, or a custom OpenAI-compatible endpoint.

## Upgrading to notarized builds later

If the team gets an Apple Developer ID, no code changes are needed — add
`signingIdentity` under `bundle.macOS` in `src-tauri/tauri.conf.json` and set
`APPLE_ID` / `APPLE_PASSWORD` (an app-specific password) / `APPLE_TEAM_ID` in
the build environment, then notarize with `xcrun notarytool submit` after
`bun run package:mac:dmg` as an additional CI step.
