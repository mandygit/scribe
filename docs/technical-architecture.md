# Resonance Technical Architecture

**Product:** Resonance  
**Tagline:** Capture what happened. Improve what you said.  
**Scope:** Local-first macOS meeting capture, transcription, summarization, speaking coaching, live nudges, history, trends, retention, local voice-aware imported-recording coaching, and Record and Review self-practice video feedback.

This document explains why the project uses each major library or integration, what problem it solves, what alternatives were considered or rejected, and where the implementation lives.

## 1. Design goals and constraints

Resonance is optimized around these constraints:

1. **Local-first privacy.** Raw audio should stay on the user's Mac. The default analyzer is local Ollama. Voice identity is local speaker verification, not a cloud voice API.
2. **macOS-native capture.** The app must access microphone and system audio with user-granted macOS permissions.
3. **No data loss.** Raw audio is saved before transcription, AEC, analysis, and summaries. Optional processing failures must not delete source recordings.
4. **Progressive degradation.** Missing Ollama, whisper.cpp, SpeexDSP, ScreenCaptureKit permission, or speaker models should produce actionable errors or fallback behavior.
5. **Typed boundaries.** Frontend/backend contracts use explicit DTOs and Tauri commands rather than ad hoc JSON.
6. **Testable slices.** Pure logic is isolated in Rust or TypeScript helpers where possible, and native/external adapters are narrow.

Core orchestration lives in `src-tauri/src/lib.rs`. It wires the Rust modules, exposes Tauri commands, owns the app state, sets up the SQLite repository, and contains the pipeline glue between recording, transcription, metrics, analysis, imported summaries, and voice matching.

## 2. High-level runtime flow

### Live/local meeting flow

1. React calls `start_recording` through `src/tauri-commands.ts`.
2. Rust command handling in `src-tauri/src/lib.rs` creates a meeting row and safe audio paths.
3. `RecordingManager` in `src-tauri/src/audio/manager.rs` starts microphone capture through `CpalCaptureBackend` and optional system audio through `ScreenCaptureKitSystemAudioBackend`.
4. Audio metadata is persisted through `SqliteRepository` in `src-tauri/src/persistence/mod.rs`.
5. After stop, `transcribe_meeting` uses `WhisperShellTranscriber` in `src-tauri/src/transcription/mod.rs`.
6. Transcript stream events use `resonance://transcript-segment` and `resonance://transcript-stream-complete`.
7. Deterministic metrics run through `src-tauri/src/rules/mod.rs`.
8. Local analysis prompts and validation run through `src-tauri/src/analysis/mod.rs`.
9. Scorecards are produced by `src-tauri/src/scoring/mod.rs`.
10. Reports, transcript detail, history, trends, and failures are read back through Tauri commands in `src-tauri/src/lib.rs`.

### Imported/downloaded recording flow

1. The user pastes a local media path in `src/components/ImportedRecordingPanel.tsx`.
2. React calls `importRecordingSummary` in `src/tauri-commands.ts`.
3. Rust validates and extracts local audio using `ffmpeg` through `src-tauri/src/media_import.rs`.
4. The extracted audio is transcribed by the same whisper.cpp adapter.
5. Optional voice-matched coaching runs diarization and speaker matching before analysis.
6. The local summarizer in `src-tauri/src/analysis/mod.rs` asks Ollama for summary JSON.
7. If matched user speech exists and the user explicitly confirms cloud video review, `src-tauri/src/video_review.rs` samples frames from the meeting video and asks OpenAI for visual delivery feedback around those matched speech windows.
8. If the user is not visible, the source is audio-only, or no user speech is matched, Resonance returns an audio-only visual-review status instead of pretending to assess posture or eye contact.
9. The meeting summary is persisted to `imported_meeting_summaries`; the visual-review result is returned with the current UI response.

### Record and Review practice flow

1. The user records through the Tauri WebView camera APIs or imports a local `.mp4`, `.mov`, or `.webm` in `src/components/RecordReviewPanel.tsx`.
2. React calls `savePracticeCameraRecording` or `importPracticeVideo` in `src/tauri-commands.ts`.
3. Rust writes/copies the video under app data using helpers in `src-tauri/src/media_import.rs` and persists a `practice_recordings` row through `src-tauri/src/persistence/mod.rs`.
4. `analyze_practice_recording_audio` extracts mono 16 kHz WAV audio with `ffmpeg`, transcribes it with `WhisperShellTranscriber`, and calculates deterministic speech metrics with `src-tauri/src/rules/mod.rs`.
5. Rust builds a `practice_review_reports` row plus queryable `practice_timeline_annotations` for pace, filler, and clarity findings.
6. `src/components/PracticeReviewReport.tsx` renders the full practice report, audio/visual score slots, privacy badge, suggestions, and timeline evidence.
7. Visual review crosses `src-tauri/src/video_review.rs`, which enforces both saved cloud-video opt-in and per-review confirmation before any full video could be sent to a configured provider.

## 3. Frontend stack decisions

### React

**Files:**

- `src/App.tsx`
- `src/main.tsx`
- `src/components/*.tsx`

**Why React:** The app needs a stateful desktop UI with many panels: setup, recording, transcript stream, nudges, metrics, scorecards, history, trends, privacy, and imported recordings. React gives a simple component model and works smoothly with Tauri's WebView surface.

**What it solves:**

- Component composition for panels such as `ManualVerificationPanel`, `ImportedRecordingPanel`, `MeetingHistoryPanel`, and `ScorecardReport`.
- Camera practice and review composition through `RecordReviewPanel` and `PracticeReviewReport`.
- Local state orchestration in `src/App.tsx`.
- Server-renderable component tests via `renderToStaticMarkup` in `tests/frontend/components.test.tsx`.

**Why not a heavier UI framework:** The app does not currently need routing, server rendering, or a design-system dependency. Keeping the frontend dependency set small reduces bundle and maintenance risk.

### TypeScript strict contracts

**Files:**

- `src/contracts.ts`
- `src/tauri-commands.ts`
- Rust DTOs in `src-tauri/src/lib.rs`

**Why TypeScript:** Tauri command payloads cross a process boundary. TypeScript interfaces mirror Rust `Serialize` DTOs so UI code knows exact shapes.

**What it solves:**

- Prevents accidental frontend usage of missing fields.
- Documents event payloads such as `TranscriptStreamEvent`, `LiveNudgeEvent`, `VoiceDiarizationResult`, and `ImportedRecordingSummaryResult`.
- Documents practice DTOs such as `PracticeRecording`, `PracticeReviewReport`, `PracticeTimelineAnnotation`, and `PracticeReviewResult`.
- Keeps UI tests strongly typed.

**Important detail:** TypeScript types are compile-time only. Rust remains the source of truth at runtime. The Rust side validates external/untrusted data such as Ollama JSON and file paths.

### Vite

**Files:**

- `vite.config.ts`
- `package.json`
- `src-tauri/tauri.conf.json`

**Why Vite:** Tauri needs a frontend dev server and a static production bundle. Vite provides fast local dev and a small production build path with minimal configuration.

**What it solves:**

- `bun run dev` starts the frontend on `127.0.0.1`.
- `bun run build` type-checks and builds `dist/`.
- Tauri consumes `frontendDist: "../dist"` in `src-tauri/tauri.conf.json`.

### Bun

**Files:**

- `package.json`
- `bun.lock`
- `tests/frontend/*.test.tsx`

**Why Bun:** Bun is the chosen JS runtime and package manager for this repo. It runs dependency install, frontend tests, and scripts quickly.

**What it solves:**

- Fast `bun test tests/frontend`.
- Lockfile for JS dependencies.
- Simple script surface: `test:frontend`, `lint`, `build`, `tauri`, `package:mac`.

### Biome

**Files:**

- `biome.json`
- `package.json`

**Why Biome:** Bun does not provide lint/format rules by itself. Biome supplies a fast formatter, import organizer, and lint check without adding ESLint/Prettier complexity.

**What it solves:**

- `bun run lint` checks formatting and import order.
- `bun run lint:fix` applies safe formatting fixes.
- Keeps TypeScript and TSX formatting consistent after broad changes like the Resonance rebrand.

## 4. Desktop shell and native boundary

### Tauri 2

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `src-tauri/src/main.rs`
- `src-tauri/src/lib.rs`
- `src/tauri-commands.ts`

**Why Tauri:** Resonance needs a macOS desktop app with native permissions, filesystem access, process spawning, a tray/menu affordance, and a React UI. Tauri gives a small native Rust shell around a web UI.

**What it solves:**

- Native command bridge for recording, transcription, analysis, settings, history, notifications, imported summaries, voice enrollment, and voice matching.
- macOS bundling through `bun run package:mac`.
- Tray icon/menu in `src-tauri/src/lib.rs`.
- App data directory resolution through `app.path().app_data_dir()`.

**Why not Electron:** Electron would make native capture and Rust audio integration heavier, increase bundle size, and duplicate Node/Rust boundaries. Tauri keeps Rust as the native integration layer.

**Security note:** Tauri commands remain the boundary for local filesystem and native execution. User-provided paths are validated in Rust before use.

### Swift ScreenCaptureKit sidecar

**Files:**

- `src-tauri/native/system-audio-capture/main.swift`
- `src-tauri/build.rs`
- `src-tauri/src/audio/system.rs`
- `src-tauri/tauri.conf.json`
- `docs/decisions/adr-001-system-audio-sidecar.md`

**Why ScreenCaptureKit:** macOS system audio capture requires Apple-native APIs and Screen Recording permission. ScreenCaptureKit can capture system/app audio without a virtual audio driver.

**Why a Swift sidecar:** Rust can call native frameworks, but direct Objective-C/Swift interop would enlarge the unsafe/native surface. The sidecar isolates the macOS-specific code in a small Swift program.

**What it solves:**

- Captures remote participant/system audio into `{meetingId}.system.m4a`.
- Keeps mic audio and system audio separately identifiable.
- Converts permission failures into actionable messages.
- Avoids a driver install.

**Minute implementation details:**

- `src-tauri/build.rs` compiles the Swift helper with `xcrun swiftc`.
- The helper binary base name is `resonance-system-audio-capture`.
- Rust resolves the development helper path in `system_audio_helper_path()` in `src-tauri/src/audio/system.rs`.
- Tauri bundles the external binary through `externalBin` in `src-tauri/tauri.conf.json`.
- The helper stops through stdin; the Rust backend records helper failure as metadata instead of failing the whole mic recording.

**Rejected alternatives:** Direct Rust FFI into ScreenCaptureKit and virtual audio drivers. See `docs/decisions/adr-001-system-audio-sidecar.md`.

## 5. Rust backend library choices

### cpal for microphone capture

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/src/audio/capture.rs`
- `src-tauri/src/audio/manager.rs`
- `src-tauri/src/audio/domain.rs`

**Why cpal:** It is a common Rust cross-platform audio input abstraction. Resonance needs microphone capture with device listing and default input support.

**What it solves:**

- Captures mic input without writing platform-specific mic code first.
- Supports a `CpalCaptureBackend` adapter behind the recording manager.
- Keeps the rest of the pipeline independent of the capture implementation.

**Why pinned to `=0.17.1`:** Audio APIs are sensitive. Pinning avoids accidental behavior changes from semver-compatible releases during this early native-capture phase.

### hound for WAV writing

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/src/audio/wav.rs`
- `src-tauri/src/audio/aec.rs`

**Why hound:** The app needs simple local PCM WAV writing and reading for mic audio and AEC output. `hound` is a focused Rust WAV crate.

**What it solves:**

- Saves raw mic WAV files.
- Writes derived AEC WAV files.
- Keeps audio persistence simple and inspectable.

**Why not a full media framework:** Full codecs are handled by external adapters (`ffmpeg` for imported media and ScreenCaptureKit/AVAssetWriter for system `.m4a`). The Rust app only needs reliable WAV handling internally.

### rusqlite for embedded persistence

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/src/persistence/mod.rs`

**Why SQLite through rusqlite:** Resonance is local-first and single-user. SQLite is embedded, durable, queryable, and does not require a server. `rusqlite` gives parameterized access without ORM overhead.

**What it solves:**

- Stores meetings, transcript segments, metrics, reports, imported summaries, practice recordings, practice review reports, practice timeline annotations, audio metadata, settings, pipeline failures, voice profile metadata, and schema versions.
- Supports history search, trend queries, retention cleanup, imported-summary provenance, and practice review history.
- Keeps all persistence local under app data.

**Minute implementation details:**

- Schema migrations live in `run_migrations()` in `src-tauri/src/persistence/mod.rs`.
- `CURRENT_SCHEMA_VERSION` tracks the latest migration.
- `schema_versions` records applied versions.
- `ensure_column()` validates static identifiers and column type allow-list before formatting migration SQL.
- Search uses escaped LIKE patterns through `like_contains_pattern()`.
- The Orator -> Resonance data migration uses `rewrite_app_data_file_paths()` to rewrite app-data-owned paths after copying legacy data.
- Record and Review adds `practice_recordings`, `practice_review_reports`, and `practice_timeline_annotations`; the report body is JSON for flexible UI rendering, while annotations remain queryable rows for timeline/history views.
- `cloud_video_review_enabled` is separate from transcript cloud analysis because full video is more sensitive than transcript text.
- Queries use `params![]`; user strings are not concatenated into SQL.

**Why not PostgreSQL:** The app is local desktop software. PostgreSQL would require installation, a service process, credentials, and backup concerns that conflict with local-first simplicity.

### serde and serde_json

**Files:**

- `src-tauri/src/domain.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/analysis/mod.rs`
- `src-tauri/src/voice_matching.rs`

**Why serde:** Tauri command responses, persisted report JSON, Ollama responses, and frontend DTOs require structured serialization.

**What it solves:**

- `#[serde(rename_all = "camelCase")]` maps Rust fields to TypeScript-friendly JSON.
- Analyzer and summary responses are parsed as strict JSON.
- Persisted report and summary bodies are round-tripped through SQLite.

**Security detail:** JSON from Ollama is treated as untrusted. `parse_coaching_analysis_response()` and `parse_meeting_summary_response()` in `src-tauri/src/analysis/mod.rs` validate schema shape and quote grounding before persistence.

### tempfile for tests

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/src/persistence/mod.rs`

**Why tempfile:** Repository tests need isolated SQLite files and migration scenarios without touching real app data.

**What it solves:**

- Creates throwaway databases for persistence tests.
- Enables legacy migration tests and schema idempotence tests.

## 6. External local tools

### whisper.cpp through `whisper-cli`

**Files:**

- `src-tauri/src/transcription/mod.rs`
- `src-tauri/src/lib.rs`
- `src/components/SetupGuidePanel.tsx`
- `src/components/ManualVerificationPanel.tsx`
- `spikes/whisper-benchmark/`

**Why whisper.cpp CLI:** Local transcription is central to privacy. Shelling out to a user-configured `whisper-cli` avoids embedding large model/runtime code in the app and keeps model choice configurable.

**What it solves:**

- Transcribes saved mic WAV and imported extracted audio.
- Transcribes extracted practice-video audio for local Record and Review reports.
- Produces timestamped transcript segments.
- Lets users choose model files based on accuracy/performance tradeoffs.

**Why not a cloud transcription API:** Raw meeting audio would leave the machine, conflicting with the primary privacy constraint.

**Why not link a Whisper Rust binding yet:** CLI integration is simpler to package incrementally and can be validated independently. A Rust binding would add native build complexity and model runtime coupling.

**Minute implementation details:**

- Settings include `transcriber_bin_path` and `transcriber_model_path` in `ResonanceSettings`.
- `WhisperShellTranscriber::from_settings()` validates configuration.
- Pipeline failures persist when transcription fails.
- Transcription retries once in the meeting pipeline before persisting final failure.

### Ollama for local text analysis and summaries

**Files:**

- `src-tauri/src/analysis/mod.rs`
- `src-tauri/src/lib.rs`
- `src/components/SetupGuidePanel.tsx`

**Why Ollama:** The app needs local LLM analysis without sending transcript text to a remote service. Ollama provides a localhost HTTP API and model management outside the app.

**What it solves:**

- Quote-grounded post-meeting coaching reports.
- Imported-recording summaries with action items, decisions, open questions, and optional speaking improvements.
- Local default analysis provider.

**Security and correctness details:**

- Prompts explicitly require strict JSON only.
- Responses are parsed and validated before persistence.
- Coaching observations must quote exact user transcript text.
- Context quotes must come from context transcript rows.
- The prompt instructs the model to analyze the user's communication, not other speakers' performance.

**Why not use Ollama for voice identity:** LLMs are text generators, not speaker-verification systems. Voice identity needs embeddings and diarization, implemented separately with sherpa-onnx.

### ffmpeg for imported media extraction

**Files:**

- `src-tauri/src/media_import.rs`
- `src-tauri/src/lib.rs`
- `src/components/ImportedRecordingPanel.tsx`
- `src/components/RecordReviewPanel.tsx`

**Why ffmpeg:** Downloaded recordings may be `.mp4`, `.mov`, `.m4a`, `.mp3`, or `.wav`, and practice videos may be `.mp4`, `.mov`, or `.webm`. The app needs a reliable local extractor to normalize audio for transcription, diarization, and practice speech review.

**What it solves:**

- Converts local media into app-owned extracted audio.
- Keeps imported media support broad without adding codec libraries to Rust.
- Extracts practice-video audio into `practice-recordings/{practiceId}.audio.wav` so retention cleanup can safely delete derived artifacts under app data.

**Security detail:** The Rust code uses `Command::arg()` rather than shell string interpolation, so user paths are passed as arguments rather than evaluated by a shell.

**Why path input instead of native file dialog:** Avoids a new dependency while the imported-recording pipeline is still being built. The UI clearly asks for an absolute local path.

### WebView MediaRecorder for first camera practice capture

**Files:**

- `src/App.tsx`
- `src/components/RecordReviewPanel.tsx`
- `src-tauri/src/lib.rs`
- `src-tauri/src/media_import.rs`
- `src-tauri/Info.plist`

**Why WebView MediaRecorder first:** The goal for the first Record and Review slice is a working camera practice path without adding native AVFoundation helper complexity or a new dependency. Tauri already provides a WebView surface, and browser media APIs can capture a camera/microphone stream into a Blob that Rust can persist under app data.

**What it solves:**

- Provides camera preview and start/stop controls in the existing React UI.
- Saves recorded bytes through `save_practice_camera_recording`.
- Enforces the 15-minute v1 cap in Rust before persistence.
- Stores camera recordings under `practice-recordings/` so retention uses the same app-data safety boundary as raw audio.

**Tradeoff:** WebView media support can vary by macOS/WebKit version. This is intentionally an incremental adapter. A native AVFoundation helper remains the hardening path if WebView recording proves unreliable in packaged builds.

### OpenAI sampled-frame video review

**Files:**

- `src-tauri/src/video_review.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/media_import.rs`
- `src/components/RecordReviewPanel.tsx`
- `src/components/PracticeReviewReport.tsx`

**Why a separate video-review interface:** Posture, eye contact, gestures, framing, and lighting need a multimodal vision provider or future local vision stack. Full video upload is materially more sensitive than transcript text, so it cannot reuse the existing transcript-cloud setting.

**What it solves:**

- `VideoReviewAnalyzer` defines the provider boundary.
- `OpenAiVideoReviewer` extracts sampled JPEG frames locally with `ffmpeg` and sends only those frames to the OpenAI Responses API.
- Tests use `FixtureVideoReviewer` for deterministic visual annotations without network access.
- Runtime visual review requires both `cloud_video_review_enabled` and `allow_cloud_video_for_this_review`.
- The API key is read from `RESONANCE_OPENAI_API_KEY` or `OPENAI_API_KEY` at runtime and is never persisted.
- Cost controls are environment-driven: `RESONANCE_OPENAI_MODEL`, `RESONANCE_OPENAI_MAX_FRAMES`, and `RESONANCE_OPENAI_FRAME_INTERVAL_SECONDS`.
- OpenAI responses must match a strict JSON schema before any score or annotation is persisted.

**Why sampled frames instead of full video upload:** A 1-15 minute video can be expensive and privacy-sensitive to send as a full asset. Sampling low-detail frames every few seconds is enough for MVP feedback on framing, posture, eye contact direction, lighting, and broad gesture/movement cues while bounding token/image cost. For imported meeting videos, the prompt includes locally matched user-speech windows so OpenAI can return `userVisible=false` when the matched speaker's camera is off or not identifiable in sampled frames.

**Security details:**

- The OpenAI secret is not passed as a command-line argument. `curl` receives request configuration through stdin so the authorization header is not exposed in ordinary process arguments.
- Third-party JSON is treated as untrusted and revalidated in Rust before persistence.
- Prompts explicitly prohibit inferring sensitive traits or identity.

### SpeexDSP for acoustic echo cancellation

**Files:**

- `src-tauri/src/audio/aec.rs`
- `src-tauri/src/lib.rs`
- `docs/decisions/adr-002-offline-aec-adapter.md`

**Why SpeexDSP:** A validation spike showed SpeexDSP could reduce synthetic echo enough for the V1 threshold. It is a focused native library for acoustic echo cancellation.

**What it solves:**

- Optionally creates `{meetingId}.aec.wav` from mic audio and a compatible reference.
- Reduces system-audio bleed in the user's mic transcript when prerequisites are met.

**Minute implementation details:**

- Resonance runtime-loads `libspeexdsp.dylib` instead of hard-linking it.
- If SpeexDSP is missing or input formats are incompatible, the pipeline falls back to raw mic WAV.
- The raw mic and system audio are preserved.
- `enableEchoCancellation` is separate from `enableSystemAudio`.

**Why optional:** Requiring SpeexDSP at build/install time would make the app fragile on machines without the library. Runtime loading preserves degraded-mode behavior.

## 7. Voice enrollment, matching, and diarization

### sherpa-onnx

**Files:**

- `src-tauri/Cargo.toml`
- `src-tauri/src/voice_matching.rs`
- `src-tauri/src/lib.rs`
- `src/components/ImportedRecordingPanel.tsx`
- `src/components/ManualVerificationPanel.tsx`

**Why sherpa-onnx:** Resonance needs local speaker embeddings and diarization to answer: "Was I speaking in this recording, and which transcript segments were mine?" `sherpa-onnx` provides local ONNX-backed speaker embedding extraction and offline speaker diarization with Rust APIs.

**What it solves:**

- Local voice profile preparation from an enrolled mic sample.
- Cosine-similarity speaker verification.
- Whole-recording coarse voice match checks.
- Diarization preview for imported recordings.
- Diarized speaker-to-profile matching.
- Matched windows that label transcript rows as `User` or context before summary/coaching.

**Why feature-gated:** Speaker matching is optional and model-dependent. The default build should not fail because a user has not configured or installed speaker models.

**Important compile detail:**

```toml
[features]
default = []
speaker-matching-sherpa = ["dep:sherpa-onnx"]
```

When the feature is disabled, commands fail safely with explicit dependency-disabled errors rather than pretending matching worked.

**Voice pipeline detail:**

1. `enroll_voice_profile_from_last_mic_test` copies a local mic sample into app data.
2. `prepare_voice_profile_for_matching` extracts and persists an embedding.
3. `match_voice_profile_from_meeting` compares a candidate local sample.
4. `match_imported_recording_voice` performs coarse whole-recording matching.
5. `diarize_imported_recording_speakers` previews speaker segments.
6. `match_imported_recording_speaker_segments` matches diarized speaker turns to the profile.
7. `import_recording_summary` can use matched windows to request speaking improvements only for matched user speech.

**Why not OpenAI voice models:** The current product promise is local-first raw-audio handling. Sending raw meeting audio to a cloud voice model would violate the default privacy posture. Cloud could be a future explicit opt-in, but not the base identity path.

**Why not Ollama:** Ollama handles text summaries/coaching. It cannot reliably perform speaker verification or diarization from audio.

**Threshold details:**

- Default speaker match threshold is `0.75` in `src-tauri/src/lib.rs`.
- Matching logic re-evaluates `similarity_score >= threshold`; it does not trust stale `is_match` flags.
- Transcript labeling only marks segments as user when matched windows overlap and pass threshold.
- If matched windows exist, non-overlapping speech becomes context rather than user speech.

## 8. Deterministic rules, live nudges, and scoring

### Rules module

**Files:**

- `src-tauri/src/rules/mod.rs`
- `src-tauri/src/lib.rs`

**Why deterministic rules:** Live meeting feedback must be low-latency and should not run an LLM during the call. Filler words, hedging phrases, WPM, and talk-time can be detected locally and deterministically.

**What it solves:**

- Filler count.
- Hedging count.
- Words per minute.
- Talk-time ratio.
- Longest monologue.

### Live nudge pipeline

**Files:**

- `src-tauri/src/nudges/mod.rs`
- `src-tauri/src/lib.rs`
- `src/components/LiveNudgePanel.tsx`
- `src/components/CoachDock.tsx`

**Why a deterministic event pipeline:** Nudges should be immediate, bounded, and low-distraction. A local pipeline avoids LLM latency and privacy exposure.

**What it solves:**

- Emits `resonance://live-nudge`.
- Throttles duplicate/burst nudges.
- Maintains bounded recent event history.
- Allows UI dismissal without mutating persisted meeting data.

### Scoring module

**Files:**

- `src-tauri/src/scoring/mod.rs`
- `src/components/ScorecardReport.tsx`

**Why separate scoring:** Scores should be deterministic, bounded, and testable independently from the LLM. Missing metrics should produce partial scores rather than broken reports.

**What it solves:**

- Availability-aware scoring.
- Bounded 0-100 score dimensions.
- Weighted overall score over available dimensions.
- UI warnings for missing signals.

## 9. Persistence and data model

**Primary file:** `src-tauri/src/persistence/mod.rs`

Main persisted concepts:

- `meetings`: meeting lifecycle metadata.
- `transcript_segments`: timestamped transcript rows.
- `metrics`: deterministic metric values.
- `reports`: local coaching analysis JSON and score.
- `audio_metadata`: mic/system audio paths and capture metadata.
- `pipeline_failures`: latest failed stage per meeting.
- `imported_meeting_summaries`: downloaded-recording summaries, extracted audio path, and speaking-coaching source.
- `practice_recordings`: app-data-owned self-practice videos from camera or import, extracted audio path, status, failure fields, and cloud-video provenance.
- `practice_review_reports`: combined practice report JSON with overall/audio/visual score slots.
- `practice_timeline_annotations`: queryable timestamped findings for audio-local and future video review evidence.
- `voice_profiles`: local voice enrollment metadata and persisted embedding.
- `settings`: privacy, transcription, system audio, AEC, voice model, analyzer, and explicit cloud-video-review settings.
- `schema_versions`: migration tracking.

## 10. Error handling and resilience

**Files:**

- `src-tauri/src/domain.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/persistence/mod.rs`

Errors use `AppError { code, message, details }`. The codebase prefers explicit, user-actionable errors over silent fallback.

Important examples:

- Transcription failures are retried once, then persisted as pipeline failures.
- Metrics and analysis failures preserve raw audio and completed artifacts.
- Retention cleanup skips unsafe paths rather than deleting outside app data.
- Practice retention deletes expired practice videos and extracted practice audio only when canonical paths are under app data; practice rows and reports remain.
- Voice matching reports disabled dependencies clearly when the Sherpa feature is not compiled.
- Visual review refuses to run without separate saved cloud-video opt-in and per-review confirmation.
- Legacy Orator app-data migration fails loudly if copying data fails.

## 11. Privacy and security boundaries

### Local-first raw audio

Raw mic audio, system audio, voice enrollment samples, imported extracted audio, practice videos, and practice extracted audio remain local app data. External adapters are local processes:

- `whisper-cli`
- `ffmpeg`
- Swift ScreenCaptureKit helper
- Optional SpeexDSP dynamic library
- Optional sherpa-onnx local model runtime

### Localhost analyzer

Ollama requests go to localhost. Prompts contain transcript text, not raw audio.

### Path safety

Relevant files:

- `src-tauri/src/audio/storage.rs`
- `src-tauri/src/media_import.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/persistence/mod.rs`

Important protections:

- Recording file stems are validated.
- Generated audio paths are under app data.
- Imported practice videos are copied under app data before analysis.
- Camera practice bytes are written by Rust under app data rather than leaving the app to reference a browser blob.
- Voice profile delete and retention cleanup canonicalize paths before deletion.
- Practice retention cleanup canonicalizes paths before deleting video or extracted audio.
- Imported media extraction uses process arguments, not shell interpolation.
- SQLite queries use parameterized `rusqlite::params`.
- Migration SQL helper only accepts static-safe identifiers and column types.
- LIKE patterns escape `%`, `_`, and `\` before using `ESCAPE '\\'`.

## 12. Resonance rebrand and legacy data migration

**Files:**

- `src-tauri/tauri.conf.json`
- `src-tauri/src/lib.rs`
- `src-tauri/src/persistence/mod.rs`
- `package.json`
- `src-tauri/Cargo.toml`

The app was renamed from Orator to Resonance. Because macOS app data paths depend on app identity, the rename could have stranded existing local user data.

The migration logic:

1. Uses new app identifier `com.resonance.meetingcoach`.
2. Looks for legacy app data directories for `com.orator.meetingcoach` and `Orator`.
3. If `resonance.sqlite3` does not exist and legacy `orator.sqlite3` does, copies legacy app data into the new directory.
4. Renames:
   - `orator.sqlite3` -> `resonance.sqlite3`
   - `orator.sqlite3-wal` -> `resonance.sqlite3-wal`
   - `orator.sqlite3-shm` -> `resonance.sqlite3-shm`
5. Opens the copied DB.
6. Rewrites app-data-owned file paths in:
   - `audio_metadata.file_path`
   - `audio_metadata.system_audio_file_path`
   - `voice_profiles.sample_audio_file_path`
   - `imported_meeting_summaries.extracted_audio_file_path`
7. Leaves external source paths, such as imported media files in Downloads, unchanged.

The remaining `Orator/orator` strings in source are intentional legacy migration constants and tests.

## 13. Why dependencies are intentionally limited

The codebase avoids adding libraries unless they solve a concrete boundary problem:

| Dependency/integration | Why it exists |
| --- | --- |
| Tauri | Native desktop shell and command bridge. |
| React | Stateful UI composition. |
| TypeScript | Frontend contract safety. |
| Vite | Fast frontend dev/build for Tauri. |
| Bun | Package manager, scripts, and tests. |
| Biome | Formatting/linting without ESLint/Prettier complexity. |
| cpal | Cross-platform mic capture abstraction. |
| hound | Focused WAV read/write. |
| rusqlite | Embedded local persistence. |
| serde/serde_json | DTO and persisted JSON serialization. |
| tempfile | Isolated persistence tests. |
| ScreenCaptureKit sidecar | macOS system audio without a virtual driver. |
| whisper.cpp CLI | Local transcription without bundling model runtime. |
| Ollama | Local LLM analysis and summaries. |
| ffmpeg | Local imported media extraction. |
| SpeexDSP | Optional offline AEC. |
| sherpa-onnx | Optional local speaker embeddings and diarization. |

## 14. Validation commands

Frontend:

```bash
bun run test:frontend
bun run lint
bun run build
```

Rust:

```bash
cd src-tauri
cargo fmt -- --check
cargo check --quiet
cargo check --features speaker-matching-sherpa --quiet
cargo test --quiet
```

## 15. Known gotchas for future engineers and agents

1. **Do not use Ollama for speaker identity.** Use local speaker embeddings and diarization.
2. **Do not delete legacy Orator constants casually.** They preserve upgrade data migration.
3. **Do not make SpeexDSP a hard build dependency.** AEC must remain optional.
4. **Do not merge mic/system audio too early.** Separate channels preserve attribution and privacy options.
5. **Do not treat imported whole-recording voice match as user-only proof.** It is a coarse pre-diarization signal.
6. **Do not label unmatched transcript rows as user when matched windows exist.** Unmatched rows are context.
7. **Do not shell-interpolate user paths.** Use `Command::arg()` and validate/canonicalize where needed.
8. **Do not add cloud audio analysis as a default path.** Raw audio privacy is a core product constraint.
9. **Do not scan/delete outside app data for retention or voice profile cleanup.** Canonical app-data checks are intentional.
10. **Do not remove schema migration tests when changing persistence.** SQLite migrations are the upgrade path for all local user data.

## 16. Reference map

| Concern | Main files |
| --- | --- |
| Tauri command orchestration | `src-tauri/src/lib.rs`, `src/tauri-commands.ts` |
| Shared domain and settings | `src-tauri/src/domain.rs`, `src/contracts.ts` |
| SQLite persistence | `src-tauri/src/persistence/mod.rs` |
| Mic/system audio | `src-tauri/src/audio/`, `src-tauri/native/system-audio-capture/main.swift` |
| AEC | `src-tauri/src/audio/aec.rs`, `docs/decisions/adr-002-offline-aec-adapter.md` |
| Transcription | `src-tauri/src/transcription/mod.rs` |
| Rules/metrics | `src-tauri/src/rules/mod.rs` |
| Live nudges | `src-tauri/src/nudges/mod.rs`, `src/components/LiveNudgePanel.tsx` |
| Analysis and summaries | `src-tauri/src/analysis/mod.rs`, `src/components/ScorecardReport.tsx` |
| Scoring | `src-tauri/src/scoring/mod.rs` |
| Imported recordings | `src-tauri/src/media_import.rs`, `src/components/ImportedRecordingPanel.tsx` |
| Voice matching/diarization | `src-tauri/src/voice_matching.rs`, `src/components/ImportedRecordingPanel.tsx` |
| History/trends | `src/components/MeetingHistoryPanel.tsx`, `src/components/TrendsDashboard.tsx` |
| Privacy settings | `src/components/PrivacySettingsPanel.tsx` |
| First-run setup | `src/components/SetupGuidePanel.tsx` |
| Packaging | `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/Info.plist` |
| Frontend tests | `tests/frontend/components.test.tsx`, `tests/frontend/notifications.test.ts`, `tests/frontend/formatting.test.ts` |
