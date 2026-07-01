# Resonance

**Capture what happened. Improve what you said.**

Resonance is a privacy-first macOS meeting coach and practice reviewer. It records local meeting audio, transcribes it with local whisper.cpp, summarizes meetings with local Ollama, gives live communication nudges, tracks speaking trends, can use local voice enrollment plus speaker matching to focus coaching on your own speech in downloaded recordings, and can record or import self-practice videos for Record and Review.

Raw audio stays on your Mac by default. Cloud video review is available only as an explicit opt-in flow: Resonance samples video frames locally, sends those sampled images to OpenAI only after a saved setting plus per-review confirmation, and validates the JSON response before saving feedback.

## What Resonance does

| Capability | What it gives you |
| --- | --- |
| Local microphone recording | Saves raw mic audio before any processing. |
| System audio capture | Captures remote participant/system audio separately on macOS through ScreenCaptureKit. |
| Echo-cancellation preprocessing | Optionally derives a cleaned mic WAV when a compatible system-audio reference and SpeexDSP are available. |
| Local transcription | Uses a configured `whisper-cli` binary and model path. |
| Live nudges | Emits local rule-based nudges for filler words, hedging, pace, and talk-time patterns. |
| Post-meeting coaching | Uses local Ollama with quote-grounded JSON validation. |
| Meeting summaries | Summarizes downloaded recordings into executive summary, action items, decisions, open questions, and optional user-only delivery feedback. |
| Voice-aware imported coaching | Uses local voice enrollment, speaker embeddings, and diarization to coach only matched user speech. |
| Record and Review | Records or imports self-practice videos, extracts audio locally, and produces a practice report with timeline annotations. |
| History and trends | Stores local transcripts, reports, imported summaries, metrics, and trend points in SQLite. |
| Retention controls | Lets you configure raw-audio retention while preserving transcripts and reports. |

## Requirements

Resonance currently targets local macOS development and packaging.

- macOS 13 or newer for ScreenCaptureKit system-audio capture.
- Bun for frontend package scripts.
- Rust toolchain for the Tauri backend.
- Xcode Command Line Tools for the Swift ScreenCaptureKit helper.
- `ffmpeg` for imported/downloaded recording audio extraction.
- `whisper-cli` from whisper.cpp plus a local Whisper model file.
- Ollama running locally on `127.0.0.1:11434` with a local model for summaries/coaching.
- Optional: SpeexDSP installed locally for echo-cancellation preprocessing.
- Optional: local sherpa-onnx speaker embedding and speaker segmentation ONNX models for voice matching and diarization.
- Optional: OpenAI API key for cloud visual review of sampled video frames.

## Quick start for development

1. Install JavaScript dependencies:

   ```bash
   bun install
   ```

2. Install or verify Rust:

   ```bash
   rustc --version
   cargo --version
   ```

3. Install local external tools. Example Homebrew commands:

   ```bash
   brew install ffmpeg ollama
   ```

   Install whisper.cpp separately if your system does not already provide `whisper-cli`. Then download a local Whisper model such as `ggml-base.bin` or `ggml-small.bin`.

4. Start Ollama and pull a local model:

   ```bash
   ollama serve
   ollama pull llama3.2
   ```

5. Run the Tauri app:

   ```bash
   bun run tauri dev
   ```

6. In the app, open the setup panel and configure:

   - `whisper-cli` binary path, for example `/opt/homebrew/bin/whisper-cli`.
   - Whisper model path, for example `/Users/you/models/ggml-base.bin`.
   - Optional speaker embedding model path for voice matching.
   - Optional speaker segmentation model path for diarization.

## Running with optional local voice matching

The `sherpa-onnx` dependency is optional and feature-gated so normal builds do not require speaker-model native code.

To compile the Rust backend with the local speaker matching adapter:

```bash
cd src-tauri
cargo check --features speaker-matching-sherpa
```

For app development with the feature enabled, pass the feature to the Tauri CLI:

```bash
bun run tauri dev --features speaker-matching-sherpa
```

Then configure absolute paths for:

- Speaker embedding model: used to extract local speaker embeddings from your enrollment sample and candidate audio.
- Speaker segmentation model: used to diarize imported recordings into speaker turns before matching those turns against your local profile.

The app keeps this local. It does not use Ollama for identity matching; Ollama is only used for text summaries and coaching.

## Typical usage

### Record and coach a live meeting

1. Open Resonance from the Tauri app window.
2. Confirm macOS permissions:
   - Microphone permission for local mic capture.
   - Screen & System Audio Recording permission if system audio is enabled.
3. Click start recording.
4. Speak normally. Resonance records mic audio and, when enabled, system audio separately.
5. Stop recording.
6. Run transcription.
7. Run deterministic metrics.
8. Run local analysis to generate a scorecard report.
9. Review history and trends over time.

### Summarize a downloaded recording

1. Put the recording somewhere local, such as Downloads.
2. Paste the absolute media path into the downloaded-recording panel.
3. Confirm `ffmpeg` path if the default is not correct.
4. Click **Extract, transcribe, and summarize**.
5. Review the executive summary, action items, decisions, and open questions.

### Get speaking feedback only for your own speech in a downloaded recording

1. Record a short mic test in Resonance.
2. Click **Enroll from last mic test**.
3. Configure a local speaker embedding model path.
4. Click **Prepare matching**.
5. Configure a local speaker segmentation model path.
6. For a downloaded recording, use:
   - **Preview speaker segments** to inspect diarization readiness.
   - **Match my speaker segments** to identify likely user speech windows.
7. Enable **Use my matched voice profile for speaking coaching**.
8. Optional: enable cloud video review and confirm this specific meeting review may send sampled frames to OpenAI.
9. Run the imported summary.

If voice matching is not available, use **I am the main speaker/presenter in this recording** only when that is clearly true. That is a manual fallback, not identity detection.

For imported meeting videos, visual feedback is only attempted when Resonance finds matched user-speech windows. OpenAI is asked to review sampled frames around those windows and to return audio-only status when your camera appears off, hidden, or not identifiable. If the recording is audio-only or no user speech is matched, Resonance keeps the result audio-only and does not send frames.

### Record and review a practice video

1. Open **Record and Review**.
2. Enter an optional practice title.
3. Choose either:
   - **Start camera practice**, then **Stop and save practice** within 15 minutes.
   - Paste an absolute `.mp4`, `.mov`, or `.webm` path and click **Import practice video**.
4. Confirm the `ffmpeg` path if the default is not correct.
5. Click **Run local audio review** to extract audio locally, transcribe it, calculate delivery metrics, and create a practice report.
6. For visual feedback, launch Resonance from a terminal that has `RESONANCE_OPENAI_API_KEY` set, enable cloud video review, confirm the specific review, then click **Run combined review**.
7. Review the overall/audio/visual score, suggestions, privacy note, and timeline annotations.

Visual review uses sampled frames rather than uploading the whole video. Configure cost controls with optional environment variables:

```bash
export RESONANCE_OPENAI_API_KEY="your_api_key"
export RESONANCE_OPENAI_MODEL="gpt-4.1-mini"                  # optional default
export RESONANCE_OPENAI_MAX_FRAMES="12"                       # optional, capped at 16
export RESONANCE_OPENAI_FRAME_INTERVAL_SECONDS="10"           # optional, clamped 5-60
bun run tauri dev
```

## Commands

| Command | Purpose |
| --- | --- |
| `bun install` | Install frontend/tooling dependencies. |
| `bun run tauri dev` | Run the desktop app in development. |
| `bun run test:frontend` | Run Bun-based frontend/component tests. |
| `bun run lint` | Run Biome checks. |
| `bun run lint:fix` | Apply Biome-safe formatting/import fixes. |
| `bun run build` | Type-check and build the frontend. |
| `bun run package:mac` | Build a local macOS `.app` bundle. |
| `bun run package:mac:dmg` | Build a `.dmg` for handing to other Macs. See `docs/distributing.md`. |
| `cd src-tauri && cargo test` | Run Rust tests. |
| `cd src-tauri && cargo check --features speaker-matching-sherpa` | Validate the optional speaker matching build. |

## Local data and privacy

Resonance stores local app data under the macOS app data directory for `com.resonance.meetingcoach`. It uses:

- `resonance.sqlite3` for meetings, transcripts, metrics, reports, settings, imported summaries, failures, and voice profile metadata.
- Per-meeting audio files for raw mic audio and optional system audio.
- A local voice enrollment sample under the app data directory.
- Imported-recording extracted audio under the app data directory.
- Practice videos and extracted practice audio under `practice-recordings/` in the app data directory.

During the Orator -> Resonance rebrand, the app gained a legacy migration that copies local data from the previous app identity into the new Resonance app data directory and rewrites stored app-data paths. The legacy strings that remain in source code are only for that upgrade path.

## Validation checklist

Before relying on a local build, run:

```bash
bun run test:frontend
bun run lint
bun run build
cd src-tauri && cargo fmt -- --check
cd src-tauri && cargo check --quiet
cd src-tauri && cargo test --quiet
```

If you are working on voice matching:

```bash
cd src-tauri && cargo check --features speaker-matching-sherpa --quiet
```

## Project layout

| Path | Purpose |
| --- | --- |
| `src/` | React frontend, TypeScript contracts, Tauri command wrappers, notification helpers. |
| `src/components/` | User-facing panels for recording, setup, reports, history, trends, privacy, and imported recordings. |
| `src-tauri/src/` | Rust backend modules for commands, audio, persistence, transcription, analysis, scoring, rules, nudges, and voice matching. |
| `src-tauri/native/system-audio-capture/` | Swift ScreenCaptureKit sidecar source. |
| `src-tauri/tauri.conf.json` | Tauri product, bundle, window, and external helper configuration. |
| `docs/decisions/` | Architecture decision records. |
| `docs/technical-architecture.md` | Detailed engineering rationale and file map. |
| `tests/frontend/` | Bun-rendered React/component and notification tests. |
| `spikes/` | Historical validation spikes for Whisper, ScreenCaptureKit, and AEC. |

## Known limitations

- The app is macOS-focused.
- System audio capture requires macOS Screen Recording permission.
- Voice matching requires locally supplied speaker embedding and segmentation models.
- Echo cancellation only produces a cleaned file when all prerequisites are compatible; otherwise Resonance safely falls back to raw mic audio.
- Imported-recording input is path-based rather than a native file picker to avoid adding another dependency.
- Camera recording uses the Tauri WebView media APIs and saves the resulting video through a Rust command; native AVFoundation capture remains a future hardening path.
- Visual posture, eye-contact, gesture, framing, and lighting review uses OpenAI through sampled frames. It requires `curl`, `ffmpeg`, an OpenAI API key in the runtime environment, the saved cloud-video setting, and per-review confirmation.
- Ollama summaries depend on a local Ollama server and model quality.

## More technical detail

Read `docs/technical-architecture.md` for the full implementation rationale, library choices, alternatives, privacy boundaries, and file references.
