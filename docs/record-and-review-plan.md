# Spec and Implementation Plan: Record and Review

## Objective

Build **Record and Review** for Resonance: a self-practice mode where a user records a 1-15 minute camera video or imports an existing self-recorded video, then gets a full review of both **audio speech delivery** and **visual presentation**.

The goal is to help the user answer:

- Was I speaking too fast or too slowly?
- Did I use filler words or hedging?
- Was my posture steady and confident?
- Was I making eye contact with the camera?
- Were my hand gestures helpful, distracting, or absent?
- Did my framing, lighting, and body position support the message?
- Where exactly in the video should I improve?

The output should be a **full report plus timeline annotations**, not just a score. Each finding should include a timestamp range, evidence, severity, and a concrete suggestion.

## Confirmed product decisions

- **Input modes:** support both camera recording inside Resonance and importing an existing self-recorded video file.
- **Review output:** full report plus timeline annotations.
- **Retention:** use the same raw-audio retention setting for review videos and derived review artifacts.
- **Cloud policy:** cloud vision/video analysis is allowed only with explicit opt-in.
- **Cloud payload:** full video may be sent when the user explicitly enables cloud video review.
- **Provider strategy:** build a provider-agnostic review interface first; concrete cloud provider adapters come after the interface is stable.

## Key assumptions

1. Record and Review is for **self-practice**, not meetings with multiple participants.
2. Initial target remains macOS via Tauri.
3. A 15-minute maximum recording length is a hard v1 limit.
4. Existing local transcription, metrics, scoring, retention, history, and privacy patterns should be reused.
5. Robust visual feedback for posture, gestures, and eye contact requires either a local vision stack or a cloud multimodal model. Since cloud is allowed with explicit opt-in, v1 should create the interface and UI boundary for cloud video review, then add provider adapters incrementally.
6. Audio review can be useful before visual review is fully implemented because Resonance already has local transcription, deterministic metrics, and Ollama-based text coaching.

## Non-goals for the first implementation wave

- Real-time visual coaching while recording.
- Multi-person diarization inside self-practice videos.
- Automatic slide/screen-content analysis.
- Browser-based video editing.
- Uploading video without explicit video-analysis opt-in and per-run confirmation.
- Replacing the existing meeting recording flow.

## Success criteria

The feature is done when:

1. User can record a camera practice video up to 15 minutes from inside Resonance.
2. User can import an existing local practice video.
3. Video and derived artifacts are stored under app data and cleaned up by the same retention policy used for raw audio.
4. User can run an audio review locally using existing transcription, metrics, and local analysis.
5. User can run a provider-agnostic video review path only when explicit cloud video review is enabled.
6. Review report contains:
   - Overall practice score.
   - Audio delivery section.
   - Visual delivery section.
   - Timeline annotations.
   - Concrete suggestions.
   - Privacy disclosure showing whether video stayed local or was sent to a configured cloud reviewer.
7. Review history can list prior practice recordings and reopen reports.
8. Validation passes:
   - `bun run test:frontend`
   - `bun run lint`
   - `bun run build`
   - `cd src-tauri && cargo fmt -- --check`
   - `cd src-tauri && MACOSX_DEPLOYMENT_TARGET=11.0 CARGO_BUILD_JOBS=1 cargo check --quiet`
    - `cd src-tauri && MACOSX_DEPLOYMENT_TARGET=11.0 CARGO_BUILD_JOBS=1 cargo test --quiet`

## Implementation status

The first implementation wave is in place:

- Camera practice uses WebView `getUserMedia`/`MediaRecorder` and persists video bytes through Rust under app data.
- Practice video import supports `.mp4`, `.mov`, and `.webm`, copies files under `practice-recordings/`, and extracts audio with `ffmpeg`.
- SQLite schema version 12 adds practice recordings, reports, annotations, and the separate `cloud_video_review_enabled` setting.
- Local audio review extracts audio, transcribes with the existing whisper.cpp adapter, calculates deterministic metrics, persists a report, and creates timestamped audio annotations.
- The provider-agnostic visual review boundary exists in `src-tauri/src/video_review.rs` with explicit consent gates, OpenAI sampled-frame review, and fixture test support.
- The UI renders Record and Review capture/import controls, explicit cloud-video warnings, practice history, and report details.
- Retention cleanup deletes expired practice video/audio artifacts only when paths are safely under app data.

Remaining follow-up items:

- Manually validate OpenAI visual review with short 1-2 minute practice videos and tune frame interval/max-frame defaults for cost and quality.
- Decide whether to replace WebView `MediaRecorder` with native AVFoundation capture after packaged-app manual verification.
- Add specialized local Ollama solo-practice coaching if transcript-grounded freeform delivery suggestions are needed beyond deterministic audio metrics.
- Add a delete-practice-recording command/UI if users need manual cleanup before retention runs.

## Architecture overview

Record and Review should be a sibling workflow to the existing meeting and imported-recording flows, not a replacement.

```text
Camera / imported video
        |
        v
Review recording row + video metadata in SQLite
        |
        +--> local ffmpeg audio extraction
        |        |
        |        +--> whisper.cpp transcription
        |        +--> deterministic speech metrics
        |        +--> local Ollama speech coaching
        |
        +--> optional explicit cloud video reviewer
                 |
                 +--> posture / eye contact / gesture / framing annotations
        |
        v
Combined review report + timeline annotations + history
```

## Proposed module boundaries

### Frontend

| Area | Files |
| --- | --- |
| Types/contracts | `src/contracts.ts` |
| Tauri wrappers | `src/tauri-commands.ts` |
| Main state orchestration | `src/App.tsx` |
| UI panel | `src/components/RecordReviewPanel.tsx` |
| Review report component | `src/components/PracticeReviewReport.tsx` |
| Component tests | `tests/frontend/components.test.tsx` |

### Rust backend

| Area | Files |
| --- | --- |
| Tauri commands/orchestration | `src-tauri/src/lib.rs` |
| Persistence | `src-tauri/src/persistence/mod.rs` |
| Domain DTOs | `src-tauri/src/domain.rs` |
| Video import/extraction reuse | `src-tauri/src/media_import.rs` |
| Camera recording adapter | `src-tauri/src/video/` or `src-tauri/native/camera-capture/` |
| Review analyzer interface | `src-tauri/src/practice_review.rs` or `src-tauri/src/video_review.rs` |
| Optional cloud reviewer adapters | `src-tauri/src/video_review/` |
| Retention cleanup | `src-tauri/src/lib.rs`, `src-tauri/src/persistence/mod.rs` |

## Capture strategy

### Recommended camera capture path

Use a native macOS camera capture adapter rather than relying only on WebView `MediaRecorder`.

Reason:

- Resonance already uses native capture adapters for microphone and system audio.
- Tauri's WebView media APIs can vary across macOS/WebKit versions.
- Native AVFoundation capture can write a predictable local `.mov` or `.mp4` file directly under app data.
- The existing ScreenCaptureKit sidecar pattern proves this repo can safely isolate native Apple-framework code in a small helper boundary.

Proposed implementation:

- Add a small native AVFoundation camera helper or Rust-native video adapter.
- Store recordings under app data, for example `practice-recordings/{practiceId}.mov`.
- Add a maximum duration guard at 15 minutes.
- Add camera permission copy to setup guidance and `Info.plist`.

Current v1 note: the implemented MVP uses WebView `MediaRecorder` to avoid a new native helper/dependency while proving the end-to-end workflow. Native AVFoundation remains the hardening path if runtime testing shows WebView capture is unreliable.

### Import path

Reuse the existing imported-recording path pattern:

- User provides an absolute local video path.
- Rust validates the path and supported extension.
- Resonance copies or references the source based on chosen storage policy.
- For retention consistency, v1 should copy imported practice videos into app data so cleanup can be reliable and path deletion remains app-data scoped.

## Analysis strategy

### Audio review

Audio review can be local in v1:

1. Extract audio from the practice video using `ffmpeg`.
2. Transcribe with `WhisperShellTranscriber`.
3. Run deterministic metrics from `src-tauri/src/rules/mod.rs`.
4. Run local Ollama coaching using an analysis prompt specialized for solo practice delivery.

Current v1 note: the implemented audio review stops at deterministic transcript metrics and score-based suggestions. This keeps the first slice fully local and dependency-free beyond the existing whisper/ffmpeg tools, while leaving specialized Ollama practice prompts as a follow-up.

Audio feedback categories:

- Pace / words per minute.
- Filler words.
- Hedging.
- Long pauses.
- Clarity and concise phrasing.
- Energy and confidence cues if transcript evidence supports it.

### Visual review

Visual review categories:

- Eye contact.
- Posture.
- Hand gestures.
- Framing.
- Lighting/background distractions.
- Facial expressiveness.
- Movement stability.

Robust visual review is hard to do with simple image heuristics. The plan should create a provider-agnostic interface:

```rust
pub trait VideoReviewAnalyzer {
    fn analyze_practice_video(
        &self,
        request: PracticeVideoReviewRequest,
    ) -> Result<PracticeVideoReview, AppError>;
}
```

Implementation includes:

- `OpenAiVideoReviewer`: extracts sampled frames with `ffmpeg`, sends them to OpenAI Responses API, and validates strict JSON before persistence.
- `FixtureVideoReviewer` for tests.
- Environment cost controls: `RESONANCE_OPENAI_MODEL`, `RESONANCE_OPENAI_MAX_FRAMES`, and `RESONANCE_OPENAI_FRAME_INTERVAL_SECONDS`.

### Explicit cloud-video consent

Cloud video review must require both:

1. A persisted setting, for example `cloudVideoReviewEnabled`.
2. A per-review confirmation, for example `allowCloudVideoForThisReview`.

Reason:

- Existing `cloudAnalysisEnabled` historically refers to transcript/text cloud analysis.
- Sending a full video is more sensitive than sending transcript text.
- A separate setting prevents accidental broadening of the existing cloud opt-in.

The UI should say clearly:

> Visual review sends the selected full practice video to your configured cloud video reviewer. Use this only if you are comfortable sharing the video with that provider.

## Data model proposal

Add tables:

### `practice_recordings`

Stores one self-practice video.

Fields:

- `id TEXT PRIMARY KEY`
- `title TEXT`
- `source_kind TEXT NOT NULL` (`camera` or `imported`)
- `video_file_path TEXT NOT NULL`
- `extracted_audio_file_path TEXT`
- `duration_ms INTEGER`
- `byte_size INTEGER`
- `recorded_at_ms INTEGER NOT NULL`
- `created_at_ms INTEGER NOT NULL`
- `updated_at_ms INTEGER NOT NULL`
- `analysis_status TEXT NOT NULL` (`recorded`, `extracting`, `transcribing`, `reviewing`, `complete`, `failed_partial`)
- `cloud_video_used INTEGER NOT NULL CHECK (cloud_video_used IN (0,1))`
- `pipeline_failure_code TEXT`
- `pipeline_failure_message TEXT`

### `practice_review_reports`

Stores the combined audio + visual report.

Fields:

- `id TEXT PRIMARY KEY`
- `practice_recording_id TEXT NOT NULL UNIQUE`
- `overall_score INTEGER`
- `audio_score INTEGER`
- `visual_score INTEGER`
- `body_json TEXT NOT NULL`
- `generated_at_ms INTEGER NOT NULL`

### `practice_timeline_annotations`

Stores queryable timeline annotations separately from the report JSON.

Fields:

- `id TEXT PRIMARY KEY`
- `practice_recording_id TEXT NOT NULL`
- `started_at_ms INTEGER NOT NULL`
- `ended_at_ms INTEGER NOT NULL`
- `category TEXT NOT NULL`
- `severity TEXT NOT NULL`
- `evidence TEXT NOT NULL`
- `suggestion TEXT NOT NULL`
- `source TEXT NOT NULL` (`audioLocal`, `videoCloud`, `videoLocal`)

## Contract proposal

Add TypeScript/Rust DTOs:

- `PracticeRecording`
- `PracticeRecordingSourceKind`
- `PracticeReviewStatus`
- `PracticeReviewReport`
- `PracticeTimelineAnnotation`
- `PracticeReviewResult`
- `CloudVideoReviewSettings`
- `CameraDevice`
- `StartPracticeRecordingResult`
- `StopPracticeRecordingResult`
- `ImportPracticeVideoResult`

Tauri commands:

- `list_camera_devices`
- `start_practice_video_recording`
- `stop_practice_video_recording`
- `import_practice_video`
- `analyze_practice_recording_audio`
- `analyze_practice_recording_video`
- `analyze_practice_recording`
- `list_practice_recordings`
- `get_practice_review_detail`
- `delete_practice_recording`
- `update_video_review_settings`

## UX proposal

Add a new panel: **Record and Review**.

Sections:

1. **Practice setup**
   - Title/topic input.
   - Duration limit copy: 1-15 minutes.
   - Camera device selector.
   - Microphone/camera permission guidance.

2. **Record**
   - Camera preview.
   - Start/stop recording.
   - Countdown/duration indicator.
   - Saved video path/status.

3. **Import**
   - Local video path input.
   - Copy into app data.
   - Show duration and size.

4. **Review controls**
   - Local audio review.
   - Cloud visual review opt-in toggle.
   - Per-review cloud confirmation checkbox.
   - Combined review button.

5. **Report**
   - Overall score.
   - Audio delivery score.
   - Visual delivery score.
   - Timeline annotations.
   - Category sections: pace, filler, clarity, posture, eye contact, gestures, framing.
   - Privacy badge: local-only or cloud-video-used.

6. **History**
   - Practice recordings list.
   - Status, date, duration, score, cloud-used indicator.

## Code style example

Prefer typed, small boundary functions with explicit errors:

```rust
fn ensure_practice_duration_allowed(duration_ms: u64) -> Result<(), AppError> {
    if duration_ms > MAX_PRACTICE_REVIEW_DURATION_MS {
        return Err(AppError {
            code: "practice_video_too_long".to_string(),
            message: "Practice videos must be 15 minutes or shorter.".to_string(),
            details: Some(format!("duration_ms={duration_ms}")),
        });
    }
    Ok(())
}
```

## Testing strategy

### Rust tests

Cover:

- Schema migration from scratch and idempotence.
- Practice recording create/read/list.
- Retention cleanup includes practice videos and extracted audio.
- Duration limit rejects videos longer than 15 minutes.
- Imported practice video path validation.
- Cloud video review refuses to run without explicit setting and per-review confirmation.
- Practice report JSON validation.
- Timeline annotation boundary validation.
- Legacy raw-audio retention behavior still works.

### Frontend tests

Extend `tests/frontend/components.test.tsx` to cover:

- Record and Review panel renders camera/import paths.
- Cloud video warning appears when visual review is enabled.
- Report renders audio and visual sections.
- Timeline annotations render timestamp, category, evidence, and suggestion.
- Local-only privacy badge renders when cloud video was not used.
- Cloud-video-used badge renders when cloud review was used.

### Manual checks

- Record 1-minute camera practice.
- Stop before 15 minutes.
- Confirm video file is stored under app data.
- Import a local `.mov` or `.mp4`.
- Run audio review locally.
- Try visual review without cloud opt-in and confirm explicit refusal.
- Enable cloud video review and confirm per-review warning appears.

## Boundaries

### Always do

- Store practice videos under app data before analysis.
- Use the existing retention setting for practice videos and extracted practice audio.
- Keep audio review local by default.
- Require explicit persisted setting plus per-review confirmation before sending full video to cloud.
- Validate all user-provided file paths.
- Preserve source video when downstream analysis fails.
- Persist partial failures.
- Use parameterized SQLite queries.
- Add tests for migrations and retention changes.

### Ask first

- Adding a concrete cloud provider dependency or SDK.
- Adding a local computer-vision dependency such as MediaPipe, OpenCV, or a pose-estimation model.
- Changing the max duration above 15 minutes.
- Sending anything besides full video to a cloud reviewer.
- Adding native file dialog dependencies.

### Never do

- Send video to cloud by default.
- Reuse transcript-only cloud opt-in as video consent.
- Delete videos outside app data.
- Silently skip camera permission failures.
- Claim posture/eye-contact/gesture accuracy without a configured video reviewer.
- Store API keys in source code or SQLite plaintext without a deliberate secrets strategy.

## Implementation plan

### Phase 1: Data model and contracts

#### Task 1: Practice review domain and SQLite schema

**Description:** Add Rust/TypeScript DTOs and SQLite tables for practice recordings, reports, timeline annotations, and video review settings.

**Acceptance criteria:**

- Practice recording rows can be created, read, and listed.
- Report and annotation rows can be persisted and read back.
- Schema migration is idempotent.

**Verification:**

- `cd src-tauri && cargo test practice`
- `bun run build`

**Dependencies:** None  
**Files likely touched:** `src-tauri/src/persistence/mod.rs`, `src-tauri/src/domain.rs`, `src/contracts.ts`  
**Scope:** Medium

#### Task 2: Retention policy includes practice video artifacts

**Description:** Extend retention cleanup so practice videos and extracted practice audio follow the same configured retention days as raw meeting audio.

**Acceptance criteria:**

- Expired practice video artifacts are deleted only when under app data.
- Transcript/report rows remain.
- Existing audio retention tests still pass.

**Verification:**

- `cd src-tauri && cargo test retention`

**Dependencies:** Task 1  
**Files likely touched:** `src-tauri/src/lib.rs`, `src-tauri/src/persistence/mod.rs`  
**Scope:** Medium

### Checkpoint: Foundation

- SQLite migration passes.
- Existing meeting/audio retention behavior is unchanged.
- No UI yet required.

### Phase 2: Import-first review path

#### Task 3: Import practice video

**Description:** Add a backend command and frontend wrapper to import/copy a local practice video into app data and persist metadata.

**Acceptance criteria:**

- Supported local video path can be imported.
- Video is copied under app data.
- Duration over 15 minutes is rejected once duration probing is available; until then, the command stores unknown duration and analysis enforces the limit.

**Verification:**

- Rust tests for path validation and persistence.
- `bun run build`

**Dependencies:** Task 1  
**Files likely touched:** `src-tauri/src/lib.rs`, `src-tauri/src/media_import.rs`, `src/tauri-commands.ts`, `src/contracts.ts`  
**Scope:** Medium

#### Task 4: Local audio review for imported practice video

**Description:** Extract audio from the imported practice video, transcribe it locally, calculate speech metrics, and generate an audio-only practice report.

**Acceptance criteria:**

- Audio is extracted with `ffmpeg`.
- Transcript and metrics are produced.
- Report includes pace/filler/clarity suggestions and timeline annotations from transcript timestamps.

**Verification:**

- Rust tests with fake transcriber/analyzer boundaries.
- `bun run test:frontend`

**Dependencies:** Task 3  
**Files likely touched:** `src-tauri/src/lib.rs`, `src-tauri/src/analysis/mod.rs`, `src-tauri/src/rules/mod.rs`, `src-tauri/src/persistence/mod.rs`  
**Scope:** Medium

#### Task 5: Record and Review UI for import + audio report

**Description:** Add a frontend panel where the user imports a self-recorded video and runs local audio review.

**Acceptance criteria:**

- Panel appears in the manual workflow.
- Import status, review status, and audio report render.
- Timeline annotations render with timestamps.

**Verification:**

- `bun run test:frontend`
- `bun run lint`
- `bun run build`

**Dependencies:** Task 4  
**Files likely touched:** `src/App.tsx`, `src/components/ManualVerificationPanel.tsx`, `src/components/RecordReviewPanel.tsx`, `tests/frontend/components.test.tsx`  
**Scope:** Medium

### Checkpoint: Import + local audio review

- User can import a practice video.
- User can get useful local audio/speech feedback.
- No cloud video has been sent.

### Phase 3: Camera recording

#### Task 6: Camera device readiness and permission UI

**Description:** Add camera setup UI and backend capability checks without recording yet.

**Acceptance criteria:**

- UI explains camera permission requirement.
- App can surface camera availability or unsupported state.
- `Info.plist` includes camera usage copy.

**Verification:**

- Component tests for setup copy.
- Build/package config validation.

**Dependencies:** Task 1  
**Files likely touched:** `src-tauri/Info.plist`, `src/components/SetupGuidePanel.tsx`, `src/components/RecordReviewPanel.tsx`, `tests/frontend/components.test.tsx`  
**Scope:** Small/Medium

#### Task 7: Native camera recording adapter

**Description:** Add macOS camera recording support that writes practice video under app data with a 15-minute guard.

**Acceptance criteria:**

- User can start and stop a practice recording.
- Recording auto-stops or refuses to exceed 15 minutes.
- Saved video row is persisted.
- Permission errors are actionable.

**Verification:**

- Rust unit tests for duration/path/state behavior.
- Manual camera record/stop check.

**Dependencies:** Task 6  
**Files likely touched:** `src-tauri/src/lib.rs`, `src-tauri/src/video/`, `src-tauri/native/camera-capture/`, `src-tauri/build.rs`, `src/tauri-commands.ts`  
**Scope:** Large, split further during implementation if needed

#### Task 8: Camera recording UI

**Description:** Add camera preview/record controls to the Record and Review panel.

**Acceptance criteria:**

- User can choose camera, start recording, see duration, and stop.
- Saved recording can immediately run the same audio review path as imported videos.

**Verification:**

- Component tests for recording states.
- Manual recording flow.

**Dependencies:** Task 7  
**Files likely touched:** `src/components/RecordReviewPanel.tsx`, `src/App.tsx`, `src/tauri-commands.ts`, `tests/frontend/components.test.tsx`  
**Scope:** Medium

### Checkpoint: Camera + local audio review

- User can record inside Resonance.
- User can import existing video.
- Both paths can run local audio review.

### Phase 4: Provider-agnostic visual review

#### Task 9: Video review analyzer interface and OpenAI adapter

**Description:** Add a provider-agnostic video review analyzer trait/interface with OpenAI sampled-frame and fixture implementations.

**Acceptance criteria:**

- Visual review cannot run without explicit cloud video setting and per-review confirmation.
- OpenAI adapter reads its API key from the runtime environment, extracts bounded sampled frames, and validates strict JSON before persistence.
- Fixture adapter can produce deterministic posture/eye-contact/gesture annotations for tests.

**Verification:**

- Rust tests for consent gates and fixture output.

**Dependencies:** Task 1  
**Files likely touched:** `src-tauri/src/video_review.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/domain.rs`  
**Scope:** Medium

#### Task 10: Visual review settings and privacy UI

**Description:** Add explicit cloud video review settings separate from transcript cloud analysis.

**Acceptance criteria:**

- User sees a clear full-video cloud disclosure.
- Visual review requires both saved setting and per-review checkbox.
- Existing transcript cloud setting remains separate.

**Verification:**

- Component tests for warning copy and disabled states.
- `bun run test:frontend`

**Dependencies:** Task 9  
**Files likely touched:** `src/components/PrivacySettingsPanel.tsx`, `src/components/RecordReviewPanel.tsx`, `src/contracts.ts`, `tests/frontend/components.test.tsx`  
**Scope:** Medium

#### Task 11: Combined review report and timeline

**Description:** Merge local audio review output and video reviewer output into one report with timeline annotations.

**Acceptance criteria:**

- Report displays audio and visual scores.
- Timeline annotations include category, timestamp range, severity, evidence, suggestion, and source.
- Privacy badge indicates whether cloud video was used.

**Verification:**

- Rust tests for report merge.
- Frontend component tests for report rendering.

**Dependencies:** Tasks 4, 9, 10  
**Files likely touched:** `src-tauri/src/lib.rs`, `src-tauri/src/video_review.rs`, `src/components/PracticeReviewReport.tsx`, `tests/frontend/components.test.tsx`  
**Scope:** Medium

### Checkpoint: Visual review interface

- The app has the complete UX and data model for visual review.
- No concrete cloud provider is required yet.
- Visual review output can be tested through a fixture adapter.

### Phase 5: Provider adapter

#### Task 12: Add first concrete cloud video reviewer

**Description:** After choosing a provider, implement the first adapter behind the provider-agnostic interface.

**Acceptance criteria:**

- Adapter uploads full video only when explicit video opt-in and per-review confirmation are true.
- Response is schema-validated.
- Invalid or low-confidence findings fail safely.
- API keys are configured outside source code.

**Verification:**

- Adapter tests with mocked HTTP.
- Manual review with a short sample video.

**Dependencies:** Tasks 9, 10, 11 and provider selection  
**Files likely touched:** `src-tauri/src/video_review/`, `src-tauri/src/lib.rs`, settings UI, docs  
**Scope:** Medium/Large

### Checkpoint: Full Record and Review

- Camera and import paths work.
- Audio review works locally.
- Visual review works through the configured provider.
- Retention cleanup covers practice videos.
- Reports and timeline annotations are persisted and reopenable.

## Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Native camera capture is more complex than expected | High | Start with import-first path; isolate camera adapter like ScreenCaptureKit sidecar. |
| WebView camera/MediaRecorder support is inconsistent | Medium | Prefer native AVFoundation capture for persistence; use WebView preview only if reliable. |
| Visual model feedback is vague or inaccurate | High | Require timestamped evidence, category schema, confidence/severity, and exact rubric prompts. |
| Cloud video privacy surprise | High | Separate cloud video setting, per-review confirmation, clear UI badge, no default upload. |
| Large video files slow analysis | Medium | 15-minute cap, file-size display, async status, failure persistence. |
| Retention accidentally deletes external imports | High | Copy imported practice videos into app data and delete only canonical app-data paths. |
| Provider lock-in | Medium | Provider-agnostic analyzer trait before first adapter. |

## Open questions before implementation

1. Which concrete cloud multimodal provider should be implemented first after the provider-agnostic interface is ready?
2. Should camera recording use a native AVFoundation helper only, or should we first validate WebView `getUserMedia`/`MediaRecorder` support in the current Tauri WebView?
3. Should local visual heuristics be explored later with MediaPipe/OpenCV, or is cloud visual review acceptable as the primary visual path?
4. Should reports compare against a chosen speaking context, such as interview, standup, pitch, presentation, or YouTube-style delivery?

## Suggested first implementation slice

Start with **Task 1: Practice review domain and SQLite schema**.

Reason:

- It creates the foundation without native camera risk.
- It lets import, retention, reports, history, and review annotations build on stable IDs and contracts.
- It can be tested quickly and does not require choosing a cloud provider.
