# Scribe

**Private, on-device meeting notes and dictation for macOS.**

Scribe is a Tauri desktop app that records meetings locally, transcribes them with
`whisper.cpp`, and writes concise notes (executive summary, decisions, open
questions, action items) with a local LLM — nothing leaves your Mac. It also
ships a system-wide dictation mode: hold a hotkey, speak, and the transcribed
(optionally polished) text is pasted into whatever app you were typing into.

The dictation mode makes Scribe a private, local alternative to
[Wispr Flow](https://wisprflow.ai): the same speak-anywhere workflow (hotkey,
floating pill, AI-polished text injected into the focused app), but
transcription and polish run entirely on-device — no subscription, no audio
leaving your machine.

## What it does today

| Capability | What it gives you |
| --- | --- |
| Meeting recording | Local microphone capture, plus separate system/remote-participant audio via ScreenCaptureKit. |
| Echo cancellation | Optional offline SpeexDSP pass that derives a cleaned mic WAV before transcription, when a compatible reference track is available. |
| Local transcription | Runs a configured `whisper-cli` binary + model over the recording, in the background so "stop meeting" doesn't block the UI. |
| Local summarization | Sends the transcript to a local OpenAI-compatible chat server (LM Studio, Ollama, or any custom endpoint) and produces structured notes. Long transcripts are automatically map-reduced into chunks. |
| Copy to Slack/Teams | One click copies the generated notes (bold section labels, bullet lists, no tables) as rich text that pastes cleanly into chat apps. |
| Live nudges | Deterministic, rule-based (not LLM) feedback during transcript playback: filler words, hedging, pace, long monologues. |
| History & trends | Every meeting's transcript, metrics, and notes are stored locally in SQLite; a trends view charts pace/filler words/score over time. |
| Dictation | Global hotkey (double-press to toggle), floating non-activating pill UI, optional on-device polish via Apple Intelligence (macOS 15+), paste-based injection into the focused app. |
| Retention controls | Configure how long raw audio is kept; transcripts and notes are kept regardless. |
| Permission onboarding | First-run flow for Microphone / Screen Recording / Accessibility, with a clear explanation of what's degraded without each. |

### Not implemented yet

A couple of things referenced in older docs or leftover code
**have no UI and no reachable commands today** — don't be alarmed if you spot
them while poking around:

- **Cloud video review** (sampled-frame review via OpenAI) — mentioned in older docs, not present in code.

See `docs/technical-architecture.md` § "Known dead code and unshipped features" for the full, current status of each.

## Requirements

- macOS 13+ (ScreenCaptureKit system-audio capture; the dictation Apple Intelligence polish helper additionally needs macOS 15+, and degrades gracefully without it).
- [Bun](https://bun.sh) for the frontend.
- Rust + Cargo (`rustc --version` to check) for the Tauri backend.
- Xcode Command Line Tools (`xcode-select --install`) — needed to compile the two Swift sidecar helpers at build time.
- [`whisper.cpp`](https://github.com/ggml-org/whisper.cpp) via Homebrew (`brew install whisper-cpp`, which pulls in `ggml` and `libomp`) — only needed on the **build machine**: `bun run package:prepare` copies and relinks these into the app bundle, so installed builds need no Homebrew. A `ggml-*.bin` model is fetched automatically by the same step (or auto-detected from disk in dev).
- A local LLM server for summarization — any one of:
  - [LM Studio](https://lmstudio.ai) (default; the app can start/stop/load models for it via the `lms` CLI), or
  - [Ollama](https://ollama.com), or
  - any other server that speaks the OpenAI-compatible `/v1/chat/completions` API.
- SpeexDSP (`brew install speexdsp`) for echo cancellation — like whisper-cpp, build-machine-only; it gets bundled. Recording and transcription work fine without it, just without the cleaned-mic pass.

## Quick start (development)

```bash
# 1. Install JS dependencies
bun install

# 2. Confirm your Rust toolchain
rustc --version && cargo --version

# 3. Install whisper.cpp + speexdsp and stage the bundled tooling and model
brew install whisper-cpp speexdsp
bun run package:prepare

# 4. Install and start a local LLM server, e.g. LM Studio, and load a model
#    (Qwen3-14B-MLX-4bit is a good fit for 32GB+ Apple Silicon Macs — see
#    docs/technical-architecture.md § Summarization for sizing notes)

# 5. Run the app
bun run tauri dev
```

On first launch, Scribe walks you through granting Microphone / Screen
Recording / Accessibility permissions, and its Settings screen lets you point
at your `whisper-cli` binary/model path and your LLM server's host/port/model
(with a "Detect" button to list what's available).

## Everyday commands

| Command | Purpose |
| --- | --- |
| `bun run tauri dev` | Run the desktop app in development (hot-reloads the frontend; a Rust change needs a restart). |
| `bun run lint` / `bun run lint:fix` | Biome checks / auto-fix. |
| `bun run build` | Type-check (`tsc --noEmit`) and build the frontend bundle. |
| `bun run test:frontend` | Bun-based frontend tests (`tests/frontend/`). |
| `cd src-tauri && cargo check` | Fast Rust compile check. |
| `cd src-tauri && cargo test` | Rust unit/integration tests (150+; a few native ones are `#[ignore]`d since they need real hardware/whisper-cli/LM Studio). |
| `bun run package:prepare` | Assemble the bundled tooling (relinked `whisper-cli`, whisper model, libspeexdsp) into `src-tauri/resources/`. Runs automatically before the two package commands. |
| `bun run package:mac` | Build a local unsigned `.app` bundle with all tooling included. |
| `bun run package:mac:dmg` | Build a self-contained `.dmg` for handing to another Mac. |

## Building and installing a distributable build

The DMG is **self-contained**: it bundles the Swift sidecar helpers, a
relocatable `whisper-cli` (with its ggml Metal/CPU/BLAS backends), the
`ggml-small-q5_1` whisper model, and libspeexdsp for echo cancellation.
The only things a recipient installs themselves are a local LLM server
(LM Studio/Ollama) for summaries — transcription works out of the box.

There's no Apple Developer ID yet, so builds are **ad-hoc signed** (Tauri's
default) rather than notarized. That's fine for handing the app to yourself
or a teammate, but macOS's Gatekeeper and TCC (permissions) behave a little
differently than with a notarized app — follow this exactly to avoid
"app is damaged" dialogs and permission dead-ends.

### 1. Build the DMG

```bash
bun run package:mac:dmg
```

This produces `src-tauri/target/release/bundle/dmg/Scribe_<version>_aarch64.dmg`
(or `x64` on Intel).

### 2. Install it and clear the quarantine flag

```bash
# Mount the DMG, then copy (or drag) Scribe.app into /Applications, then:
xattr -cr /Applications/Scribe.app
```

Without this, Gatekeeper blocks the app as coming from an "unidentified
developer." (Right-click → Open and confirming the dialog once has the same
effect, if you'd rather not use Terminal.)

### 3. Grant permissions — and expect to re-grant them after every rebuild

Launch the app and step through the permission onboarding (Microphone,
Screen Recording, Accessibility — none are hard requirements; the app
degrades gracefully without each, see the table above).

**The gotcha:** every `cargo build`/`tauri build` changes the app's ad-hoc
code-signature hash. macOS ties Screen Recording and Accessibility grants to
that hash, so **each rebuild silently invalidates both** — System Settings
still shows Scribe toggled on, but it's an orphaned grant that doesn't match
the new binary, and the app just quietly loses the permission.

When that happens (you'll notice system audio stop capturing, or dictation
stop pasting into other apps):

1. Open **System Settings → Privacy & Security → Screen Recording** (or
   **Accessibility**), select the stale Scribe row, and remove it with `−`.
2. Trigger a fresh permission request from inside the app — start a meeting
   recording (for Screen Recording) or attempt a dictation paste (for
   Accessibility).
3. Grant the permission in the native prompt that appears.
4. **Fully quit the app (Cmd+Q) and relaunch it.** TCC grants don't take
   effect on an already-running process.

If you get an Apple Developer ID later, this whole dance goes away — add
`signingIdentity` under `bundle.macOS` in `src-tauri/tauri.conf.json` and
notarize with `xcrun notarytool` as a build step. See `docs/distributing.md`
for the day-to-day version of this section aimed at a team installing builds
you hand them, and `docs/technical-architecture.md` for why we haven't done
this yet.

## Project layout

| Path | Purpose |
| --- | --- |
| `src/App.tsx` | The entire frontend UI — recording, history, trends, settings, dictation panel. Single file by design (see technical doc). |
| `src/DictationPill.tsx` | The floating, non-activating dictation pill window. |
| `src/contracts.ts` | TypeScript types mirroring every Rust `Serialize` DTO — the typed IPC boundary. |
| `src/tauri-commands.ts` | Thin `invoke()` wrappers, one per Tauri command. |
| `src/summary-clipboard.ts` | Builds the rich-text/plain-text clipboard payload for the "Copy" button. |
| `src-tauri/src/lib.rs` | Tauri command handlers, app wiring, settings. |
| `src-tauri/src/audio/` | Mic capture (cpal), system audio (ScreenCaptureKit sidecar), AEC (SpeexDSP), WAV I/O. |
| `src-tauri/src/transcription/` | `whisper-cli` subprocess wrapper, streaming replay. |
| `src-tauri/src/summarizer/` | LM Studio/Ollama/custom chat client, map-reduce summarization. |
| `src-tauri/src/dictation/` | Hotkey detection, capture, clipboard injection, Apple Intelligence polish. |
| `src-tauri/src/rules/`, `src-tauri/src/nudges/` | Deterministic transcript metrics and live coaching nudges. |
| `src-tauri/src/persistence/` | `SqliteRepository` — all SQL lives here. |
| `src-tauri/native/` | Swift sidecar sources (system audio capture, dictation polish). |
| `docs/technical-architecture.md` | The detailed engineering doc — read this next. |
| `docs/decisions/` | ADRs for the architecturally significant calls. |
| `docs/distributing.md` | The short version of "Building and installing a distributable build" above, written for handing builds to teammates. |

## Known limitations

- macOS only.
- System audio capture requires the Screen Recording permission; without it, recording silently falls back to mic-only.
- Echo cancellation only runs when SpeexDSP is available (bundled in packaged builds; `brew install speexdsp` in dev) and a compatible reference track exists; otherwise it safely falls back to the raw mic recording.
- Summarization quality and speed depend entirely on your local model choice and hardware — see `docs/technical-architecture.md` § Summarization for sizing guidance.
- Not notarized — see the rebuild/re-grant dance above.

## Learn more

Read `docs/technical-architecture.md` for the full architecture: why each
dependency was chosen, how the pieces fit together (with diagrams), the
Tauri command/event surface, and an honest accounting of what's dead code vs.
shipped.

## License

[MIT](LICENSE). Bundled third-party components keep their own licenses
(whisper.cpp and ggml are MIT, libomp is Apache 2.0 with LLVM exception,
SpeexDSP is BSD); copies ship inside the app bundle next to the binaries.
