use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use analysis::{
    classify_transcript_speaker_role, AnalysisContext, AnalysisMetricContext,
    AnalysisTranscriptSegment, Analyzer, CoachingAnalysis, MeetingSummarizer, MeetingSummary,
    OllamaAnalyzer,
};
use audio::{
    aec::{EchoCancellationBackend, SpeexEchoCancellationBackend},
    storage::{safe_system_audio_path, safe_wav_path, validate_recording_file_stem},
    AudioDevice, CpalCaptureBackend, RecordingManager, RecordingMetadata, RecordingStarted,
    ScreenCaptureKitSystemAudioBackend,
};
use domain::{
    AnalyzerProvider, AppError, MeetingId, MeetingLifecycleState, PracticeAnnotationId,
    PracticeRecordingId, PracticeReviewReportId, ProcessingStage, ReportId, ResonanceSettings,
    Score, SummaryId,
};
use nudges::{
    LiveNudgeEvent, LiveNudgePipeline, NudgeEventSink, NudgeTranscriptEventSink, LIVE_NUDGE_EVENT,
};
use path_detection::hydrate_settings_with_local_defaults;
use persistence::{
    AudioMetadata, CreateImportedMeetingSummary, CreateMeeting, CreateMetric,
    CreatePipelineFailure, CreatePracticeRecording, CreatePracticeReviewReport,
    CreatePracticeTimelineAnnotation, CreateReport, CreateTranscriptSegment, MeetingHistoryRecord,
    MeetingTrendRecord, MetricRecord, PipelineFailureRecord, PracticeRecordingRecord,
    PracticeReviewReportRecord, PracticeTimelineAnnotationRecord, SqliteRepository,
    VoiceProfileRecord,
};
use rules::{MetricsSummary, RuleTranscriptSegment};
use scoring::{calculate_scorecard, Scorecard, ScoringInput};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use transcription::{
    Transcriber, TranscriptEventSink, TranscriptSegment, TranscriptStreamEvent,
    TranscriptStreamSummary, TranscriptionOutput, WhisperShellTranscriber,
    TRANSCRIPT_SEGMENT_EVENT, TRANSCRIPT_STREAM_COMPLETE_EVENT,
};
use video_review::{
    OpenAiVideoReviewer, PracticeVideoAnnotation, PracticeVideoReview, PracticeVideoReviewRequest,
    VideoReviewAnalyzer, VideoReviewWindow,
};
use voice_matching::{
    compare_voice_embeddings, diarize_speakers, match_diarized_speakers, prepare_voice_embedding,
    voice_diarization_status, voice_matcher_status, VoiceDiarizationMatchResult,
    VoiceDiarizationResult, VoiceDiarizationStatus, VoiceMatchResult, VoiceMatcherStatus,
};

pub mod analysis;
pub mod audio;
pub mod domain;
pub mod media_import;
pub mod nudges;
pub mod path_detection;
pub mod persistence;
pub mod rules;
pub mod scoring;
pub mod transcription;
pub mod video_review;
pub mod voice_matching;

struct AppState {
    repository: Mutex<SqliteRepository>,
    recordings: Mutex<RecordingManager<CpalCaptureBackend, ScreenCaptureKitSystemAudioBackend>>,
    echo_cancellation: SpeexEchoCancellationBackend,
}

fn load_effective_settings(repository: &SqliteRepository) -> Result<ResonanceSettings, AppError> {
    Ok(hydrate_settings_with_local_defaults(repository.get_settings()?))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    state: String,
    detail: String,
    current_lifecycle: MeetingLifecycleState,
    default_settings: ResonanceSettings,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    meeting_id: MeetingId,
    segment_count: u32,
    segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsCalculationResult {
    meeting_id: MeetingId,
    summary: MetricsSummary,
    metrics: Vec<MetricRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    meeting_id: MeetingId,
    report_id: ReportId,
    analysis: CoachingAnalysis,
    scorecard: Scorecard,
    generated_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedRecordingSummaryResult {
    meeting_id: MeetingId,
    summary_id: SummaryId,
    source_file_path: String,
    extracted_audio_file_path: String,
    segment_count: u32,
    speaking_improvements_requested: bool,
    speaking_improvements_source: ImportedSpeakingImprovementsSource,
    summary: MeetingSummary,
    visual_review: Option<ImportedMeetingVisualReview>,
    generated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportedSpeakingImprovementsSource {
    None,
    MainSpeaker,
    VoiceMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedMeetingVisualReview {
    status: ImportedMeetingVisualReviewStatus,
    visual_score: Option<Score>,
    summary: String,
    privacy_note: String,
    annotations: Vec<ImportedMeetingVisualAnnotation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportedMeetingVisualReviewStatus {
    NotRequested,
    AudioOnly,
    UserNotVisible,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedMeetingVisualAnnotation {
    started_at_ms: u64,
    ended_at_ms: u64,
    category: String,
    severity: String,
    evidence: String,
    suggestion: String,
    source: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingHistoryStatus {
    Recording,
    Recorded,
    Transcribed,
    Analyzed,
    FailedPartial,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingHistoryItem {
    meeting_id: MeetingId,
    title: Option<String>,
    started_at_ms: u64,
    stopped_at_ms: Option<u64>,
    updated_at_ms: u64,
    duration_ms: Option<u64>,
    audio_file_path: Option<String>,
    status: MeetingHistoryStatus,
    transcript_segment_count: u32,
    latest_report_id: Option<ReportId>,
    latest_report_score: Option<domain::Score>,
    latest_report_generated_at_ms: Option<u64>,
    pipeline_failure: Option<PipelineFailureRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingHistoryPage {
    items: Vec<MeetingHistoryItem>,
    next_offset: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingHistoryDetail {
    meeting: MeetingHistoryItem,
    transcript_segments: Vec<TranscriptSegment>,
    transcript_truncated: bool,
    report: Option<AnalysisResult>,
    imported_summary: Option<ImportedMeetingSummaryHistory>,
    audio_file_path: Option<String>,
    system_audio_file_path: Option<String>,
    pipeline_failure: Option<PipelineFailureRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedMeetingSummaryHistory {
    summary_id: SummaryId,
    source_file_path: String,
    extracted_audio_file_path: String,
    speaking_improvements_source: ImportedSpeakingImprovementsSource,
    summary: MeetingSummary,
    generated_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTrendPoint {
    meeting_id: MeetingId,
    title: Option<String>,
    started_at_ms: u64,
    filler_word_count: Option<f64>,
    words_per_minute: Option<f64>,
    overall_score: Option<domain::Score>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTrendsResult {
    points: Vec<MeetingTrendPoint>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionCleanupSummary {
    deleted_audio_file_count: u32,
    deleted_practice_file_count: u32,
    removed_audio_metadata_count: u32,
    skipped_audio_file_count: u32,
    skipped_practice_file_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettingsUpdateResult {
    settings: ResonanceSettings,
    cleanup: RetentionCleanupSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDevice {
    id: String,
    name: String,
    is_default: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeRecording {
    id: PracticeRecordingId,
    title: Option<String>,
    source_kind: String,
    video_file_path: String,
    extracted_audio_file_path: Option<String>,
    duration_ms: Option<u64>,
    byte_size: Option<u64>,
    recorded_at_ms: u64,
    created_at_ms: u64,
    updated_at_ms: u64,
    analysis_status: String,
    cloud_video_used: bool,
    pipeline_failure_code: Option<String>,
    pipeline_failure_message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeTimelineAnnotation {
    id: PracticeAnnotationId,
    practice_recording_id: PracticeRecordingId,
    started_at_ms: u64,
    ended_at_ms: u64,
    category: String,
    severity: String,
    evidence: String,
    suggestion: String,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeReviewReport {
    id: PracticeReviewReportId,
    practice_recording_id: PracticeRecordingId,
    overall_score: Option<Score>,
    audio_score: Option<Score>,
    visual_score: Option<Score>,
    body: PracticeReviewBody,
    generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeReviewBody {
    summary: String,
    audio_summary: String,
    visual_summary: String,
    suggestions: Vec<String>,
    privacy_note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeReviewResult {
    recording: PracticeRecording,
    report: PracticeReviewReport,
    annotations: Vec<PracticeTimelineAnnotation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeRecordingsPage {
    items: Vec<PracticeRecording>,
    next_offset: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeReviewDetail {
    recording: PracticeRecording,
    report: Option<PracticeReviewReport>,
    annotations: Vec<PracticeTimelineAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfileStatus {
    is_enrolled: bool,
    enrolled_at_ms: Option<u64>,
    sample_duration_ms: Option<u64>,
    sample_byte_size: Option<u64>,
    matching_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VoiceMatchWindow {
    started_at_ms: u64,
    ended_at_ms: u64,
    similarity_score: f32,
    threshold: f32,
}

const DEFAULT_HISTORY_LIMIT: u32 = 10;
const MAX_HISTORY_LIMIT: u32 = 50;
const HISTORY_DETAIL_TRANSCRIPT_LIMIT: u32 = 200;
const DEFAULT_TRENDS_LIMIT: u32 = 12;
const MAX_TRENDS_LIMIT: u32 = 50;
const MAX_RAW_AUDIO_RETENTION_DAYS: u16 = 365;
const MAX_PRACTICE_REVIEW_DURATION_MS: u64 = 15 * 60 * 1000;
const DEFAULT_PRACTICE_HISTORY_LIMIT: u32 = 10;
const MAX_PRACTICE_HISTORY_LIMIT: u32 = 50;
const DEFAULT_SPEAKER_MATCH_THRESHOLD: f32 = 0.75;
const MILLIS_PER_DAY: u64 = 86_400_000;
const VOICE_PROFILE_DIR_NAME: &str = "voice-profile";
const VOICE_PROFILE_SAMPLE_FILE_NAME: &str = "enrollment-sample.wav";
const RESONANCE_DATABASE_FILE_NAME: &str = "resonance.sqlite3";
const LEGACY_APP_IDENTIFIER: &str = "com.orator.meetingcoach";
const LEGACY_APP_NAME: &str = "Orator";
const LEGACY_DATABASE_FILE_NAME: &str = "orator.sqlite3";
static IMPORT_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PRACTICE_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
fn get_app_status(state: State<'_, AppState>) -> Result<AppStatus, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let settings = load_effective_settings(&repository)?;
    Ok(AppStatus {
        state: "Native shell ready".to_string(),
        detail: "Tauri command bridge and macOS menubar scaffold are connected.".to_string(),
        current_lifecycle: MeetingLifecycleState::Idle,
        default_settings: settings,
    })
}

#[tauri::command]
fn list_meeting_history(
    state: State<'_, AppState>,
    search_query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<MeetingHistoryPage, AppError> {
    let requested_limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    let offset_value = offset.unwrap_or(0);
    let fetch_limit = requested_limit + 1;
    let normalized_search = normalize_search_query(search_query);
    let mut rows = state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .list_meeting_history(normalized_search.as_deref(), fetch_limit, offset_value)?;
    let has_more = rows.len() > requested_limit as usize;
    rows.truncate(requested_limit as usize);

    Ok(MeetingHistoryPage {
        items: rows.into_iter().map(history_record_to_item).collect(),
        next_offset: if has_more {
            offset_value.checked_add(requested_limit)
        } else {
            None
        },
    })
}

#[tauri::command]
fn get_meeting_history_detail(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingHistoryDetail, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let meeting = repository
        .get_meeting(&meeting_id_value)?
        .ok_or_else(|| AppError {
            code: "meeting_not_found".to_string(),
            message:
                "Meeting history detail could not be loaded because the meeting does not exist."
                    .to_string(),
            details: Some(format!("meeting_id={}", meeting_id_value.as_str())),
        })?;
    let audio_metadata = repository.get_audio_metadata(&meeting_id_value)?;
    let mut transcript_segments = repository.list_transcript_segments_page(
        &meeting_id_value,
        HISTORY_DETAIL_TRANSCRIPT_LIMIT + 1,
        0,
    )?;
    let transcript_truncated = transcript_segments.len() > HISTORY_DETAIL_TRANSCRIPT_LIMIT as usize;
    transcript_segments.truncate(HISTORY_DETAIL_TRANSCRIPT_LIMIT as usize);
    let transcript_segment_count = repository.count_transcript_segments(&meeting_id_value)?;
    let pipeline_failure = repository.get_pipeline_failure(&meeting_id_value)?;
    let report = repository
        .list_reports_for_meeting(&meeting_id_value)?
        .into_iter()
        .next()
        .map(|report| analysis_result_from_report(&repository, report))
        .transpose()?;
    let imported_summary = repository
        .get_imported_meeting_summary_for_meeting(&meeting_id_value)?
        .map(imported_summary_history_from_record)
        .transpose()?;
    let history_item = MeetingHistoryItem {
        meeting_id: meeting.id,
        title: meeting.title,
        started_at_ms: meeting.started_at_ms,
        stopped_at_ms: meeting.stopped_at_ms,
        updated_at_ms: meeting.updated_at_ms,
        duration_ms: audio_metadata
            .as_ref()
            .and_then(|metadata| metadata.duration_ms)
            .or_else(|| {
                meeting
                    .stopped_at_ms
                    .map(|stopped_at_ms| stopped_at_ms.saturating_sub(meeting.started_at_ms))
            }),
        audio_file_path: audio_metadata
            .as_ref()
            .map(|metadata| metadata.file_path.clone()),
        status: meeting_history_status(
            meeting.stopped_at_ms,
            transcript_segment_count as usize,
            report.is_some(),
            pipeline_failure.is_some(),
        ),
        transcript_segment_count,
        latest_report_id: report.as_ref().map(|item| item.report_id.clone()),
        latest_report_score: report.as_ref().map(|item| item.analysis.overall_score),
        latest_report_generated_at_ms: report.as_ref().map(|item| item.generated_at_ms),
        pipeline_failure: pipeline_failure.clone(),
    };
    let segments = transcript_segments
        .into_iter()
        .map(|segment| TranscriptSegment {
            sequence_number: segment.sequence_number,
            speaker_label: segment.speaker_label,
            text: segment.text,
            started_at_ms: segment.started_at_ms,
            ended_at_ms: segment.ended_at_ms,
        })
        .collect();
    let audio_file_path = audio_metadata
        .as_ref()
        .map(|metadata| metadata.file_path.clone());
    let system_audio_file_path = audio_metadata
        .as_ref()
        .and_then(|metadata| metadata.system_audio_file_path.clone());

    Ok(MeetingHistoryDetail {
        meeting: history_item,
        transcript_segments: segments,
        transcript_truncated,
        report,
        imported_summary,
        audio_file_path,
        system_audio_file_path,
        pipeline_failure,
    })
}

#[tauri::command]
fn list_meeting_trends(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<MeetingTrendsResult, AppError> {
    let requested_limit = limit
        .unwrap_or(DEFAULT_TRENDS_LIMIT)
        .clamp(1, MAX_TRENDS_LIMIT);
    let rows = state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .list_meeting_trends(requested_limit)?;

    Ok(MeetingTrendsResult {
        points: rows.into_iter().rev().map(trend_record_to_point).collect(),
    })
}

#[tauri::command]
fn list_audio_devices(state: State<'_, AppState>) -> Result<Vec<AudioDevice>, AppError> {
    state
        .recordings
        .lock()
        .map_err(map_lock_error)?
        .list_audio_devices()
}

#[tauri::command]
fn list_camera_devices() -> Vec<CameraDevice> {
    vec![CameraDevice {
        id: "webview-camera".to_string(),
        name: "Camera preview and recording through the Resonance window".to_string(),
        is_default: true,
    }]
}

#[tauri::command]
fn import_practice_video(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    title: Option<String>,
) -> Result<PracticeRecording, AppError> {
    let source_path = std::path::PathBuf::from(source_path);
    media_import::validate_video_source_path(&source_path)?;
    let now_ms = current_time_ms()?;
    let practice_id = next_practice_recording_id("practice-imported", now_ms)?;
    let app_data_dir = app_data_dir(&app)?;
    let video_path =
        media_import::copy_practice_video(&source_path, &app_data_dir, practice_id.as_str())?;
    let metadata = fs::metadata(&video_path).map_err(|error| AppError {
        code: "practice_video_metadata_failed".to_string(),
        message: "Could not inspect the copied practice video.".to_string(),
        details: Some(error.to_string()),
    })?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let recording = repository.create_practice_recording(&CreatePracticeRecording {
        id: practice_id,
        title: normalize_optional_title(title),
        source_kind: "imported".to_string(),
        video_file_path: video_path.to_string_lossy().into_owned(),
        extracted_audio_file_path: None,
        duration_ms: None,
        byte_size: Some(metadata.len()),
        recorded_at_ms: now_ms,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        analysis_status: "recorded".to_string(),
        cloud_video_used: false,
        pipeline_failure_code: None,
        pipeline_failure_message: None,
    })?;
    Ok(practice_recording_from_record(recording))
}

#[tauri::command]
fn save_practice_camera_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    video_bytes: Vec<u8>,
    duration_ms: u64,
    extension: Option<String>,
) -> Result<PracticeRecording, AppError> {
    ensure_practice_duration_allowed(duration_ms)?;
    if video_bytes.is_empty() {
        return Err(AppError {
            code: "practice_video_empty".to_string(),
            message: "Camera recording did not produce video data.".to_string(),
            details: None,
        });
    }
    let now_ms = current_time_ms()?;
    let practice_id = next_practice_recording_id("practice-camera", now_ms)?;
    let app_data_dir = app_data_dir(&app)?;
    let extension = extension.as_deref().unwrap_or("webm");
    let video_path = media_import::write_practice_video_bytes(
        &video_bytes,
        extension,
        &app_data_dir,
        practice_id.as_str(),
    )?;
    let metadata = fs::metadata(&video_path).map_err(|error| AppError {
        code: "practice_video_metadata_failed".to_string(),
        message: "Could not inspect the saved practice video.".to_string(),
        details: Some(error.to_string()),
    })?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let recording = repository.create_practice_recording(&CreatePracticeRecording {
        id: practice_id,
        title: normalize_optional_title(title),
        source_kind: "camera".to_string(),
        video_file_path: video_path.to_string_lossy().into_owned(),
        extracted_audio_file_path: None,
        duration_ms: Some(duration_ms),
        byte_size: Some(metadata.len()),
        recorded_at_ms: now_ms,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        analysis_status: "recorded".to_string(),
        cloud_video_used: false,
        pipeline_failure_code: None,
        pipeline_failure_message: None,
    })?;
    Ok(practice_recording_from_record(recording))
}

#[tauri::command]
fn analyze_practice_recording_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    practice_recording_id: String,
    ffmpeg_bin_path: Option<String>,
) -> Result<PracticeReviewResult, AppError> {
    validate_recording_file_stem(&practice_recording_id)?;
    let practice_id = PracticeRecordingId::new(practice_recording_id);
    let app_data_dir = app_data_dir(&app)?;
    let (recording, settings) = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let recording = repository
            .get_practice_recording(&practice_id)?
            .ok_or_else(|| practice_not_found_error(&practice_id))?;
        ensure_practice_recording_under_duration(&recording)?;
        repository.update_practice_recording_analysis_state(
            &practice_id,
            None,
            "extracting",
            recording.cloud_video_used,
            None,
            current_time_ms()?,
        )?;
        (recording, load_effective_settings(&repository)?)
    };
    let video_path = Path::new(&recording.video_file_path);
    ensure_path_under_app_data(&app_data_dir, video_path, "practice_video_path_rejected")?;
    let extracted_audio_path = match media_import::extract_practice_video_audio(
        video_path,
        ffmpeg_bin_path.as_deref(),
        &app_data_dir,
        practice_id.as_str(),
    ) {
        Ok(path) => path,
        Err(error) => {
            persist_practice_failure(&state, &practice_id, &error)?;
            return Err(error);
        }
    };
    let transcriber = WhisperShellTranscriber::from_settings(&settings)?;
    let transcription = match transcriber.transcribe(&extracted_audio_path) {
        Ok(output) => output,
        Err(error) => {
            persist_practice_failure(&state, &practice_id, &error)?;
            return Err(error);
        }
    };
    let generated_at_ms = current_time_ms()?;
    let review =
        practice_audio_review_from_transcript(&practice_id, transcription, generated_at_ms)?;
    persist_practice_review(
        &state,
        &practice_id,
        Some(extracted_audio_path.to_string_lossy().as_ref()),
        review,
        false,
        generated_at_ms,
    )
}

#[tauri::command]
fn analyze_practice_recording_video(
    state: State<'_, AppState>,
    practice_recording_id: String,
    ffmpeg_bin_path: Option<String>,
    allow_cloud_video_for_this_review: bool,
) -> Result<PracticeReviewResult, AppError> {
    validate_recording_file_stem(&practice_recording_id)?;
    let practice_id = PracticeRecordingId::new(practice_recording_id);
    let (recording, settings) = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        (
            repository
                .get_practice_recording(&practice_id)?
                .ok_or_else(|| practice_not_found_error(&practice_id))?,
            load_effective_settings(&repository)?,
        )
    };
    let request = PracticeVideoReviewRequest {
        practice_recording_id: practice_id.as_str().to_string(),
        video_file_path: recording.video_file_path,
        ffmpeg_bin_path,
        matched_speech_windows: Vec::new(),
        allow_cloud_video_for_this_review,
        cloud_video_review_enabled: settings.cloud_video_review_enabled,
    };
    let reviewer = OpenAiVideoReviewer::from_environment();
    let video_review = match reviewer.analyze_practice_video(&request) {
        Ok(review) => review,
        Err(error) => {
            persist_practice_failure(&state, &practice_id, &error)?;
            return Err(error);
        }
    };
    let generated_at_ms = current_time_ms()?;
    persist_practice_review(
        &state,
        &practice_id,
        None,
        practice_review_from_video_review(&practice_id, video_review, generated_at_ms)?,
        true,
        generated_at_ms,
    )
}

#[tauri::command]
fn analyze_practice_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    practice_recording_id: String,
    ffmpeg_bin_path: Option<String>,
    allow_cloud_video_for_this_review: bool,
) -> Result<PracticeReviewResult, AppError> {
    if allow_cloud_video_for_this_review {
        return analyze_practice_recording_video(
            state,
            practice_recording_id,
            ffmpeg_bin_path,
            allow_cloud_video_for_this_review,
        );
    }
    analyze_practice_recording_audio(app, state, practice_recording_id, ffmpeg_bin_path)
}

#[tauri::command]
fn list_practice_recordings(
    state: State<'_, AppState>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<PracticeRecordingsPage, AppError> {
    let requested_limit = limit
        .unwrap_or(DEFAULT_PRACTICE_HISTORY_LIMIT)
        .clamp(1, MAX_PRACTICE_HISTORY_LIMIT);
    let offset_value = offset.unwrap_or(0);
    let mut rows = state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .list_practice_recordings(requested_limit + 1, offset_value)?;
    let has_more = rows.len() > requested_limit as usize;
    rows.truncate(requested_limit as usize);
    Ok(PracticeRecordingsPage {
        items: rows
            .into_iter()
            .map(practice_recording_from_record)
            .collect(),
        next_offset: if has_more {
            offset_value.checked_add(requested_limit)
        } else {
            None
        },
    })
}

#[tauri::command]
fn get_practice_review_detail(
    state: State<'_, AppState>,
    practice_recording_id: String,
) -> Result<PracticeReviewDetail, AppError> {
    validate_recording_file_stem(&practice_recording_id)?;
    let practice_id = PracticeRecordingId::new(practice_recording_id);
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let recording = repository
        .get_practice_recording(&practice_id)?
        .ok_or_else(|| practice_not_found_error(&practice_id))?;
    let report = repository
        .get_practice_review_report_for_recording(&practice_id)?
        .map(practice_report_from_record)
        .transpose()?;
    let annotations = repository
        .list_practice_timeline_annotations(&practice_id)?
        .into_iter()
        .map(practice_annotation_from_record)
        .collect();
    Ok(PracticeReviewDetail {
        recording: practice_recording_from_record(recording),
        report,
        annotations,
    })
}

#[tauri::command]
fn update_video_review_settings(
    state: State<'_, AppState>,
    cloud_video_review_enabled: bool,
) -> Result<ResonanceSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.cloud_video_review_enabled = cloud_video_review_enabled;
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

#[tauri::command]
fn start_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    device_id: Option<String>,
) -> Result<RecordingStarted, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let app_data_dir = app_data_dir(&app)?;
    let file_path = safe_wav_path(&app_data_dir, &meeting_id)?;
    let system_audio_file_path = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        if repository.get_settings()?.enable_system_audio {
            Some(safe_system_audio_path(&app_data_dir, &meeting_id)?)
        } else {
            None
        }
    };
    let started_at_ms = current_time_ms()?;
    let meeting_id_value = MeetingId::new(meeting_id.clone());
    let mut recordings = state.recordings.lock().map_err(map_lock_error)?;

    if recordings.is_recording() {
        return Err(AppError {
            code: "recording_already_active".to_string(),
            message: "Cannot start a new recording while another meeting is recording.".to_string(),
            details: None,
        });
    }

    {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        ensure_meeting_exists(&repository, &meeting_id_value, started_at_ms)?;
    }

    recordings.start_recording(meeting_id, file_path, system_audio_file_path, device_id)
}

#[tauri::command]
fn transcribe_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<TranscriptionResult, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id.clone());
    let (settings, metadata) = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let settings = load_effective_settings(&repository)?;
        let metadata = load_meeting_audio_metadata(&repository, &meeting_id_value)?;
        ensure_transcript_is_empty(&repository, &meeting_id_value)?;
        (settings, metadata)
    };
    let audio_path =
        select_transcription_audio_path(&settings, &metadata, &state.echo_cancellation);
    let transcriber = WhisperShellTranscriber::from_settings(&settings)?;
    let now_ms = current_time_ms()?;
    let result = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        transcribe_meeting_with_transcriber_path(
            &repository,
            meeting_id_value.clone(),
            std::path::Path::new(&audio_path),
            &transcriber,
            now_ms,
        )?
    };
    let transcript_sink = TauriTranscriptEventSink { app: &app };
    let nudge_sink = TauriNudgeEventSink { app: &app };
    let mut event_sink =
        NudgeTranscriptEventSink::new(transcript_sink, nudge_sink, LiveNudgePipeline::default());
    let persisted_output = TranscriptionOutput {
        segments: result.segments.clone(),
    };
    transcription::replay_transcription_output(
        meeting_id_value.as_str(),
        &persisted_output,
        &mut event_sink,
    )?;
    Ok(result)
}

#[tauri::command]
fn calculate_metrics(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MetricsCalculationResult, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let repository = state.repository.lock().map_err(map_lock_error)?;
    calculate_metrics_for_meeting_resilient(&repository, &meeting_id_value, current_time_ms()?)
}

#[tauri::command]
fn analyze_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<AnalysisResult, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let context = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        ensure_analysis_provider_available(&load_effective_settings(&repository)?)?;
        match prepare_analysis_context(&repository, &meeting_id_value) {
            Ok(context) => context,
            Err(error) => {
                persist_pipeline_failure(
                    &repository,
                    &meeting_id_value,
                    ProcessingStage::Analyzing,
                    &error,
                    current_time_ms()?,
                )?;
                return Err(error);
            }
        }
    };
    let generated_at_ms = current_time_ms()?;
    let analysis = match run_blocking_ollama_analysis(context) {
        Ok(analysis) => analysis,
        Err(error) => {
            let repository = state.repository.lock().map_err(map_lock_error)?;
            persist_pipeline_failure(
                &repository,
                &meeting_id_value,
                ProcessingStage::Analyzing,
                &error,
                generated_at_ms,
            )?;
            return Err(error);
        }
    };
    let repository = state.repository.lock().map_err(map_lock_error)?;
    persist_analysis_report_resilient(&repository, &meeting_id_value, analysis, generated_at_ms)
}

#[tauri::command]
fn get_voice_profile_status(state: State<'_, AppState>) -> Result<VoiceProfileStatus, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let settings = load_effective_settings(&repository)?;
    voice_profile_status(
        repository.get_voice_profile()?,
        settings.speaker_embedding_model_path.as_deref(),
    )
}

#[tauri::command]
fn enroll_voice_profile_from_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<VoiceProfileStatus, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let now_ms = current_time_ms()?;
    let app_data_dir = app_data_dir(&app)?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    enroll_voice_profile_from_meeting_record(&repository, &app_data_dir, &meeting_id_value, now_ms)
}

#[tauri::command]
fn prepare_voice_profile_for_matching(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<VoiceProfileStatus, AppError> {
    let now_ms = current_time_ms()?;
    let app_data_dir = app_data_dir(&app)?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    prepare_voice_profile_for_matching_record(&repository, &app_data_dir, now_ms)
}

#[tauri::command]
fn match_voice_profile_from_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    threshold: Option<f32>,
) -> Result<VoiceMatchResult, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let app_data_dir = app_data_dir(&app)?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    match_voice_profile_from_meeting_record(
        &repository,
        &app_data_dir,
        &meeting_id_value,
        threshold.unwrap_or(DEFAULT_SPEAKER_MATCH_THRESHOLD),
    )
}

#[tauri::command]
fn match_imported_recording_voice(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    ffmpeg_bin_path: Option<String>,
    threshold: Option<f32>,
) -> Result<VoiceMatchResult, AppError> {
    let source_path = std::path::PathBuf::from(source_path);
    media_import::validate_media_source_path(&source_path)?;
    let now_ms = current_time_ms()?;
    let import_sequence = IMPORT_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let match_id = format!("voice-match-{now_ms}-{import_sequence}");
    validate_recording_file_stem(&match_id)?;
    let app_data_dir = app_data_dir(&app)?;
    let extracted_audio_path = media_import::extract_recording_audio(
        &source_path,
        ffmpeg_bin_path.as_deref(),
        &app_data_dir,
        &match_id,
    )?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    match_voice_profile_audio_path_record(
        &repository,
        &app_data_dir,
        &extracted_audio_path,
        threshold.unwrap_or(DEFAULT_SPEAKER_MATCH_THRESHOLD),
    )
}

#[tauri::command]
fn diarize_imported_recording_speakers(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    ffmpeg_bin_path: Option<String>,
) -> Result<VoiceDiarizationResult, AppError> {
    let source_path = std::path::PathBuf::from(source_path);
    media_import::validate_media_source_path(&source_path)?;
    let now_ms = current_time_ms()?;
    let import_sequence = IMPORT_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let diarization_id = format!("speaker-diarization-{now_ms}-{import_sequence}");
    validate_recording_file_stem(&diarization_id)?;
    let app_data_dir = app_data_dir(&app)?;
    let extracted_audio_path = media_import::extract_recording_audio(
        &source_path,
        ffmpeg_bin_path.as_deref(),
        &app_data_dir,
        &diarization_id,
    )?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    diarize_imported_audio_recording(&repository, &app_data_dir, &extracted_audio_path)
}

#[tauri::command]
fn match_imported_recording_speaker_segments(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    ffmpeg_bin_path: Option<String>,
    threshold: Option<f32>,
) -> Result<VoiceDiarizationMatchResult, AppError> {
    let source_path = std::path::PathBuf::from(source_path);
    media_import::validate_media_source_path(&source_path)?;
    let now_ms = current_time_ms()?;
    let import_sequence = IMPORT_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let match_id = format!("speaker-segment-match-{now_ms}-{import_sequence}");
    validate_recording_file_stem(&match_id)?;
    let app_data_dir = app_data_dir(&app)?;
    let extracted_audio_path = media_import::extract_recording_audio(
        &source_path,
        ffmpeg_bin_path.as_deref(),
        &app_data_dir,
        &match_id,
    )?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    match_imported_audio_speaker_segments(
        &repository,
        &app_data_dir,
        &extracted_audio_path,
        threshold.unwrap_or(DEFAULT_SPEAKER_MATCH_THRESHOLD),
    )
}

#[tauri::command]
fn delete_voice_profile(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<VoiceProfileStatus, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let profile = repository.get_voice_profile()?;
    if let Some(profile) = profile {
        delete_voice_profile_sample(&app_data_dir(&app)?, &profile.sample_audio_file_path)?;
    }
    repository.delete_voice_profile()?;
    voice_profile_status(None, None)
}

#[tauri::command]
fn import_recording_summary(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    ffmpeg_bin_path: Option<String>,
    include_speaking_improvements: bool,
    use_voice_matched_speaking_improvements: bool,
    allow_cloud_video_for_this_review: bool,
) -> Result<ImportedRecordingSummaryResult, AppError> {
    let source_path = std::path::PathBuf::from(source_path);
    media_import::validate_media_source_path(&source_path)?;
    let now_ms = current_time_ms()?;
    let import_sequence = IMPORT_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let meeting_id = MeetingId::new(format!("imported-{now_ms}-{import_sequence}"));
    validate_recording_file_stem(meeting_id.as_str())?;
    let source_title = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("Imported recording: {name}"));
    let app_data_dir = app_data_dir(&app)?;
    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let settings = load_effective_settings(&repository)?;
        ensure_analysis_provider_available(&settings)?;
        repository.create_meeting(&CreateMeeting {
            id: meeting_id.clone(),
            title: source_title,
            started_at_ms: now_ms,
            stopped_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        })?;
        hydrate_settings_with_local_defaults(settings)
    };

    let extracted_audio_path = match media_import::extract_recording_audio(
        &source_path,
        ffmpeg_bin_path.as_deref(),
        &app_data_dir,
        meeting_id.as_str(),
    ) {
        Ok(path) => path,
        Err(error) => {
            let repository = state.repository.lock().map_err(map_lock_error)?;
            persist_pipeline_failure(
                &repository,
                &meeting_id,
                ProcessingStage::Transcribing,
                &error,
                current_time_ms()?,
            )?;
            return Err(error);
        }
    };
    let extracted_audio_metadata =
        fs::metadata(&extracted_audio_path).map_err(|error| AppError {
            code: "imported_audio_metadata_failed".to_string(),
            message: "Could not inspect the extracted imported-recording audio.".to_string(),
            details: Some(error.to_string()),
        })?;
    let voice_match_result = if use_voice_matched_speaking_improvements {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        Some(match_imported_audio_speaker_segments(
            &repository,
            &app_data_dir,
            &extracted_audio_path,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        )?)
    } else {
        None
    };
    let transcriber = WhisperShellTranscriber::from_settings(&settings)?;
    let transcribed_at_ms = current_time_ms()?;
    let transcription_result = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        repository.upsert_audio_metadata(&AudioMetadata {
            meeting_id: meeting_id.clone(),
            file_path: extracted_audio_path.to_string_lossy().into_owned(),
            system_audio_file_path: None,
            duration_ms: None,
            sample_rate_hz: Some(16_000),
            byte_size: Some(extracted_audio_metadata.len()),
            system_audio_byte_size: None,
            system_audio_stream_error: None,
            created_at_ms: transcribed_at_ms,
        })?;
        transcribe_meeting_with_transcriber_path(
            &repository,
            meeting_id.clone(),
            &extracted_audio_path,
            &transcriber,
            transcribed_at_ms,
        )?
    };
    let voice_match_windows = voice_match_result
        .as_ref()
        .map(imported_voice_match_windows)
        .unwrap_or_default();
    let speaking_improvements_source = imported_speaking_improvements_source(
        include_speaking_improvements,
        !voice_match_windows.is_empty(),
    );
    let summary_context =
        if speaking_improvements_source == ImportedSpeakingImprovementsSource::VoiceMatch {
            transcript_segments_to_analysis_segments_with_voice_matches(
                &transcription_result.segments,
                &voice_match_windows,
                DEFAULT_SPEAKER_MATCH_THRESHOLD,
            )
        } else {
            transcript_segments_to_analysis_segments(&transcription_result.segments)
        };
    if summary_context.is_empty() {
        let error = AppError {
            code: "transcript_not_found".to_string(),
            message: "Cannot summarize an imported recording without transcript segments."
                .to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        };
        let repository = state.repository.lock().map_err(map_lock_error)?;
        persist_pipeline_failure(
            &repository,
            &meeting_id,
            ProcessingStage::Analyzing,
            &error,
            current_time_ms()?,
        )?;
        return Err(error);
    }
    let generated_at_ms = current_time_ms()?;
    let speaking_improvements_requested =
        speaking_improvements_source != ImportedSpeakingImprovementsSource::None;
    let visual_review = imported_meeting_visual_review(
        &source_path,
        ffmpeg_bin_path.as_deref(),
        &meeting_id,
        &settings,
        &voice_match_windows,
        allow_cloud_video_for_this_review,
    )?;
    let summary =
        match run_blocking_ollama_summary(summary_context, speaking_improvements_requested) {
            Ok(summary) => summary,
            Err(error) => {
                let repository = state.repository.lock().map_err(map_lock_error)?;
                persist_pipeline_failure(
                    &repository,
                    &meeting_id,
                    ProcessingStage::Analyzing,
                    &error,
                    generated_at_ms,
                )?;
                return Err(error);
            }
        };
    let summary_id = SummaryId::new(format!("{}-summary", meeting_id.as_str()));
    let body_json = serde_json::to_string(&summary).map_err(|error| AppError {
        code: "summary_serialization_failed".to_string(),
        message: "Could not serialize the imported-recording summary.".to_string(),
        details: Some(error.to_string()),
    })?;
    {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        repository.mark_meeting_stopped(&meeting_id, generated_at_ms, generated_at_ms)?;
        repository.create_imported_meeting_summary(&CreateImportedMeetingSummary {
            id: summary_id.clone(),
            meeting_id: meeting_id.clone(),
            source_file_path: source_path.to_string_lossy().into_owned(),
            extracted_audio_file_path: extracted_audio_path.to_string_lossy().into_owned(),
            speaking_improvements_source: imported_speaking_improvements_source_to_db(
                speaking_improvements_source,
            )
            .to_string(),
            body_json,
            generated_at_ms,
        })?;
        clear_pipeline_failure_after_success(&repository, &meeting_id);
    }

    Ok(ImportedRecordingSummaryResult {
        meeting_id,
        summary_id,
        source_file_path: source_path.to_string_lossy().into_owned(),
        extracted_audio_file_path: extracted_audio_path.to_string_lossy().into_owned(),
        segment_count: transcription_result.segment_count,
        speaking_improvements_requested,
        speaking_improvements_source,
        summary,
        visual_review: Some(visual_review),
        generated_at_ms,
    })
}

#[tauri::command]
fn update_transcriber_settings(
    state: State<'_, AppState>,
    transcriber_bin_path: Option<String>,
    transcriber_model_path: Option<String>,
    speaker_embedding_model_path: Option<String>,
    speaker_segmentation_model_path: Option<String>,
) -> Result<ResonanceSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.transcriber_bin_path = normalize_optional_path(transcriber_bin_path);
    settings.transcriber_model_path = normalize_optional_path(transcriber_model_path);
    settings.speaker_embedding_model_path = normalize_optional_path(speaker_embedding_model_path);
    settings.speaker_segmentation_model_path =
        normalize_optional_path(speaker_segmentation_model_path);
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

#[tauri::command]
fn get_voice_matcher_status(state: State<'_, AppState>) -> Result<VoiceMatcherStatus, AppError> {
    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        load_effective_settings(&repository)?
    };
    Ok(voice_matcher_status(&settings))
}

#[tauri::command]
fn get_voice_diarization_status(
    state: State<'_, AppState>,
) -> Result<VoiceDiarizationStatus, AppError> {
    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        load_effective_settings(&repository)?
    };
    Ok(voice_diarization_status(&settings))
}

#[tauri::command]
fn update_audio_processing_settings(
    state: State<'_, AppState>,
    enable_system_audio: bool,
    enable_echo_cancellation: bool,
) -> Result<ResonanceSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.enable_system_audio = enable_system_audio;
    settings.enable_echo_cancellation = enable_echo_cancellation;
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

#[tauri::command]
fn update_privacy_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    raw_audio_retention_days: u16,
    analyzer_provider: AnalyzerProvider,
    cloud_analysis_enabled: bool,
) -> Result<PrivacySettingsUpdateResult, AppError> {
    let retention_days = validate_retention_days(raw_audio_retention_days)?;
    let is_recording = state
        .recordings
        .lock()
        .map_err(map_lock_error)?
        .is_recording();
    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let mut settings = repository.get_settings()?;
        settings.raw_audio_retention_days = retention_days;
        settings.cloud_analysis_enabled = cloud_analysis_enabled;
        settings.analyzer_provider = if cloud_analysis_enabled {
            analyzer_provider
        } else {
            AnalyzerProvider::LocalOllama
        };
        repository.upsert_settings(&settings, current_time_ms()?)?;
        settings
    };

    let cleanup = if is_recording {
        RetentionCleanupSummary {
            deleted_audio_file_count: 0,
            deleted_practice_file_count: 0,
            removed_audio_metadata_count: 0,
            skipped_audio_file_count: 0,
            skipped_practice_file_count: 0,
        }
    } else {
        let cutoff_ms = retention_cutoff_ms(retention_days, current_time_ms()?);
        let (expired_metadata, expired_practice_recordings) = {
            let repository = state.repository.lock().map_err(map_lock_error)?;
            (
                repository.list_audio_metadata_before(cutoff_ms)?,
                repository.list_practice_recordings_before(cutoff_ms)?,
            )
        };
        let mut cleanup = delete_retained_audio_files(&expired_metadata, &app_data_dir(&app)?)?;
        delete_retained_practice_files(
            &expired_practice_recordings,
            &app_data_dir(&app)?,
            &mut cleanup,
        )?;
        let repository = state.repository.lock().map_err(map_lock_error)?;
        remove_retained_audio_metadata(&repository, &expired_metadata, &mut cleanup)?;
        cleanup
    };

    Ok(PrivacySettingsUpdateResult { settings, cleanup })
}

#[tauri::command]
fn send_completion_notification(title: String, body: String) -> Result<(), AppError> {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() || body.is_empty() {
        return Err(AppError {
            code: "invalid_notification".to_string(),
            message: "Notification title and body are required.".to_string(),
            details: None,
        });
    }

    send_platform_notification(title, body)
}

#[cfg(target_os = "macos")]
fn send_platform_notification(title: &str, body: &str) -> Result<(), AppError> {
    let script = format!(
        "display notification {} with title {}",
        apple_script_string_literal(body),
        apple_script_string_literal(title)
    );
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| AppError {
            code: "notification_command_failed".to_string(),
            message: "Could not start the macOS notification command.".to_string(),
            details: Some(error.to_string()),
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(AppError {
            code: "notification_command_failed".to_string(),
            message: "macOS notification command failed.".to_string(),
            details: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn send_platform_notification(_title: &str, _body: &str) -> Result<(), AppError> {
    Err(AppError {
        code: "notification_unsupported".to_string(),
        message: "Basic completion notifications are only supported on macOS.".to_string(),
        details: None,
    })
}

fn apple_script_string_literal(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
            .replace('\r', " ")
    )
}

#[tauri::command]
fn stop_recording(state: State<'_, AppState>) -> Result<RecordingMetadata, AppError> {
    let stopped_at_ms = current_time_ms()?;
    let metadata = state
        .recordings
        .lock()
        .map_err(map_lock_error)?
        .stop_recording(stopped_at_ms)?;
    let meeting_id = MeetingId::new(metadata.meeting_id.clone());

    {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        repository.mark_meeting_stopped(&meeting_id, stopped_at_ms, stopped_at_ms)?;
        repository.upsert_audio_metadata(&AudioMetadata {
            meeting_id,
            file_path: metadata.file_path.clone(),
            system_audio_file_path: metadata.system_audio_file_path.clone(),
            duration_ms: Some(metadata.duration_ms),
            sample_rate_hz: Some(metadata.sample_rate_hz),
            byte_size: Some(metadata.byte_size),
            system_audio_byte_size: metadata.system_audio_byte_size,
            system_audio_stream_error: metadata.system_audio_stream_error.clone(),
            created_at_ms: stopped_at_ms,
        })?;
    }

    Ok(metadata)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            list_meeting_history,
            get_meeting_history_detail,
            list_meeting_trends,
            list_audio_devices,
            list_camera_devices,
            import_practice_video,
            save_practice_camera_recording,
            analyze_practice_recording_audio,
            analyze_practice_recording_video,
            analyze_practice_recording,
            list_practice_recordings,
            get_practice_review_detail,
            update_video_review_settings,
            start_recording,
            stop_recording,
            transcribe_meeting,
            calculate_metrics,
            analyze_meeting,
            get_voice_profile_status,
            enroll_voice_profile_from_meeting,
            prepare_voice_profile_for_matching,
            match_voice_profile_from_meeting,
            match_imported_recording_voice,
            diarize_imported_recording_speakers,
            match_imported_recording_speaker_segments,
            delete_voice_profile,
            get_voice_matcher_status,
            get_voice_diarization_status,
            import_recording_summary,
            update_transcriber_settings,
            update_audio_processing_settings,
            update_privacy_settings,
            send_completion_notification
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let legacy_app_data_dir = migrate_legacy_app_data_to_resonance(&app_data_dir)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.message))?;
            let database_path = app_data_dir.join(RESONANCE_DATABASE_FILE_NAME);
            let repository = SqliteRepository::open(&database_path)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.message))?;
            if let Some(legacy_dir) = legacy_app_data_dir {
                repository
                    .rewrite_app_data_file_paths(
                        &legacy_dir.to_string_lossy(),
                        &app_data_dir.to_string_lossy(),
                    )
                    .map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::Other, error.message)
                    })?;
            }
            let saved_settings = repository
                .get_settings()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.message))?;
            let hydrated_settings = hydrate_settings_with_local_defaults(saved_settings.clone());
            if hydrated_settings != saved_settings {
                repository
                    .upsert_settings(&hydrated_settings, current_time_ms().map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::Other, error.message)
                    })?)
                    .map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::Other, error.message)
                    })?;
            }
            app.manage(AppState {
                repository: Mutex::new(repository),
                recordings: Mutex::new(RecordingManager::new(
                    CpalCaptureBackend::new(),
                    ScreenCaptureKitSystemAudioBackend::new(),
                )),
                echo_cancellation: SpeexEchoCancellationBackend,
            });
            spawn_audio_retention_cleanup(database_path, app_data_dir.clone());

            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show = MenuItem::with_id(app, "show", "Show Resonance", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &quit])?;
                let tray_icon =
                    tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))
                        .expect("embedded tray icon should be a valid PNG");

                TrayIconBuilder::with_id("resonance")
                    .icon(tray_icon)
                    .menu(&menu)
                    .tooltip("Resonance")
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Resonance");
}

#[cfg(test)]
fn transcribe_meeting_with_transcriber<T: transcription::Transcriber>(
    repository: &SqliteRepository,
    meeting_id: &str,
    transcriber: &T,
    created_at_ms: u64,
) -> Result<TranscriptionResult, AppError> {
    validate_recording_file_stem(meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id.to_string());
    let metadata = load_meeting_audio_metadata(repository, &meeting_id_value)?;
    ensure_transcript_is_empty(repository, &meeting_id_value)?;
    transcribe_meeting_with_transcriber_path(
        repository,
        meeting_id_value,
        std::path::Path::new(&metadata.file_path),
        transcriber,
        created_at_ms,
    )
}

fn transcribe_meeting_with_transcriber_path<T: transcription::Transcriber>(
    repository: &SqliteRepository,
    meeting_id: MeetingId,
    audio_path: &std::path::Path,
    transcriber: &T,
    created_at_ms: u64,
) -> Result<TranscriptionResult, AppError> {
    match transcribe_audio_with_retry(transcriber, audio_path) {
        Ok(output) => {
            let result = persist_transcription_output(
                repository,
                meeting_id.clone(),
                output,
                created_at_ms,
            )?;
            clear_pipeline_failure_after_success(repository, &meeting_id);
            Ok(result)
        }
        Err(error) => {
            persist_pipeline_failure(
                repository,
                &meeting_id,
                ProcessingStage::Transcribing,
                &error,
                created_at_ms,
            )?;
            Err(error)
        }
    }
}

fn transcribe_audio_with_retry<T: transcription::Transcriber>(
    transcriber: &T,
    audio_path: &std::path::Path,
) -> Result<TranscriptionOutput, AppError> {
    match transcriber.transcribe(audio_path) {
        Ok(output) => Ok(output),
        Err(first_error) => transcriber
            .transcribe(audio_path)
            .map_err(|second_error| AppError {
                details: Some(format!(
                    "first_attempt_code={}, second_attempt_code={}",
                    first_error.code, second_error.code
                )),
                code: second_error.code,
                message: second_error.message,
            }),
    }
}

struct TauriTranscriptEventSink<'a> {
    app: &'a AppHandle,
}

impl TranscriptEventSink for TauriTranscriptEventSink<'_> {
    fn emit_segment(&mut self, event: TranscriptStreamEvent) -> Result<(), AppError> {
        self.app
            .emit(TRANSCRIPT_SEGMENT_EVENT, event)
            .map_err(|error| AppError {
                code: "transcript_stream_emit_failed".to_string(),
                message: "Could not emit transcript segment event to the UI.".to_string(),
                details: Some(error.to_string()),
            })
    }

    fn complete(&mut self, summary: TranscriptStreamSummary) -> Result<(), AppError> {
        self.app
            .emit(TRANSCRIPT_STREAM_COMPLETE_EVENT, summary)
            .map_err(|error| AppError {
                code: "transcript_stream_emit_failed".to_string(),
                message: "Could not emit transcript stream completion event to the UI.".to_string(),
                details: Some(error.to_string()),
            })
    }
}

struct TauriNudgeEventSink<'a> {
    app: &'a AppHandle,
}

impl NudgeEventSink for TauriNudgeEventSink<'_> {
    fn emit_nudge(&mut self, event: LiveNudgeEvent) -> Result<(), AppError> {
        self.app
            .emit(LIVE_NUDGE_EVENT, event)
            .map_err(|error| AppError {
                code: "live_nudge_emit_failed".to_string(),
                message: "Could not emit live nudge event to the UI.".to_string(),
                details: Some(error.to_string()),
            })
    }
}

fn load_meeting_audio_metadata(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
) -> Result<AudioMetadata, AppError> {
    repository
        .get_meeting(meeting_id)?
        .ok_or_else(|| AppError {
            code: "meeting_not_found".to_string(),
            message: "Cannot transcribe a meeting that does not exist.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        })?;
    repository
        .get_audio_metadata(meeting_id)?
        .ok_or_else(|| AppError {
            code: "audio_metadata_not_found".to_string(),
            message: "Cannot transcribe a meeting without saved audio metadata.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        })
}

fn select_transcription_audio_path(
    settings: &ResonanceSettings,
    metadata: &AudioMetadata,
    echo_cancellation: &impl EchoCancellationBackend,
) -> String {
    if !settings.enable_echo_cancellation {
        return metadata.file_path.clone();
    }

    let Some(system_audio_file_path) = metadata.system_audio_file_path.as_deref() else {
        return metadata.file_path.clone();
    };

    match echo_cancellation.process(
        std::path::Path::new(&metadata.file_path),
        std::path::Path::new(system_audio_file_path),
    ) {
        Ok(processed_path) => processed_path.to_string_lossy().into_owned(),
        Err(error) => {
            eprintln!(
                "Echo cancellation failed for meeting {}: code={}, message={}",
                metadata.meeting_id.as_str(),
                error.code,
                error.message
            );
            metadata.file_path.clone()
        }
    }
}

fn ensure_transcript_is_empty(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
) -> Result<(), AppError> {
    let existing_segments = repository.list_transcript_segments(meeting_id)?;
    if existing_segments.is_empty() {
        return Ok(());
    }

    Err(AppError {
        code: "transcript_already_exists".to_string(),
        message: "Meeting already has transcript segments. Re-transcription is not supported yet."
            .to_string(),
        details: Some(format!(
            "meeting_id={}, segment_count={}",
            meeting_id.as_str(),
            existing_segments.len()
        )),
    })
}

fn persist_transcription_output(
    repository: &SqliteRepository,
    meeting_id: MeetingId,
    output: TranscriptionOutput,
    created_at_ms: u64,
) -> Result<TranscriptionResult, AppError> {
    ensure_transcript_is_empty(repository, &meeting_id)?;
    let segments = transcript_segments_for_persistence(&meeting_id, &output, created_at_ms)?;
    repository.create_transcript_segments(&segments)?;

    Ok(TranscriptionResult {
        meeting_id,
        segment_count: u32::try_from(output.segments.len()).map_err(|error| AppError {
            code: "too_many_transcript_segments".to_string(),
            message: "Transcription produced too many transcript segments.".to_string(),
            details: Some(error.to_string()),
        })?,
        segments: output.segments,
    })
}

fn transcript_segments_for_persistence(
    meeting_id: &MeetingId,
    output: &TranscriptionOutput,
    created_at_ms: u64,
) -> Result<Vec<CreateTranscriptSegment>, AppError> {
    output
        .segments
        .iter()
        .map(|segment| {
            Ok(CreateTranscriptSegment {
                id: domain::SegmentId::new(format!(
                    "{}-segment-{}",
                    meeting_id.as_str(),
                    segment.sequence_number
                )),
                meeting_id: meeting_id.clone(),
                sequence_number: segment.sequence_number,
                speaker_label: segment.speaker_label.clone(),
                text: segment.text.clone(),
                started_at_ms: segment.started_at_ms,
                ended_at_ms: segment.ended_at_ms,
                created_at_ms,
            })
        })
        .collect()
}

fn calculate_metrics_for_meeting(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    created_at_ms: u64,
) -> Result<MetricsCalculationResult, AppError> {
    repository
        .get_meeting(meeting_id)?
        .ok_or_else(|| AppError {
            code: "meeting_not_found".to_string(),
            message: "Cannot calculate metrics for a meeting that does not exist.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        })?;
    ensure_deterministic_metrics_are_absent(repository, meeting_id)?;

    let segments = repository
        .list_transcript_segments(meeting_id)?
        .into_iter()
        .map(|segment| RuleTranscriptSegment {
            text: segment.text,
            started_at_ms: segment.started_at_ms,
            ended_at_ms: segment.ended_at_ms,
        })
        .collect::<Vec<_>>();
    let summary = rules::calculate_metrics(&segments);
    let persisted_metrics = rules::metrics_for_persistence(&summary)
        .into_iter()
        .map(|metric| {
            repository.create_metric(&CreateMetric {
                id: domain::MetricId::new(format!(
                    "{}-metric-{}",
                    meeting_id.as_str(),
                    metric.name
                )),
                meeting_id: meeting_id.clone(),
                name: metric.name,
                value: metric.value,
                unit: metric.unit,
                created_at_ms,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MetricsCalculationResult {
        meeting_id: meeting_id.clone(),
        summary,
        metrics: persisted_metrics,
    })
}

fn calculate_metrics_for_meeting_resilient(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    created_at_ms: u64,
) -> Result<MetricsCalculationResult, AppError> {
    match calculate_metrics_for_meeting(repository, meeting_id, created_at_ms) {
        Ok(result) => {
            clear_pipeline_failure_after_success(repository, meeting_id);
            Ok(result)
        }
        Err(error) => {
            persist_pipeline_failure(
                repository,
                meeting_id,
                ProcessingStage::Metrics,
                &error,
                created_at_ms,
            )?;
            Err(error)
        }
    }
}

#[cfg(test)]
fn analyze_meeting_with_analyzer<A: Analyzer>(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    analyzer: &A,
    generated_at_ms: u64,
) -> Result<AnalysisResult, AppError> {
    let context = match prepare_analysis_context(repository, meeting_id) {
        Ok(context) => context,
        Err(error) => {
            persist_pipeline_failure(
                repository,
                meeting_id,
                ProcessingStage::Analyzing,
                &error,
                generated_at_ms,
            )?;
            return Err(error);
        }
    };
    let analysis = match analyzer.analyze(&context) {
        Ok(analysis) => analysis,
        Err(error) => {
            persist_pipeline_failure(
                repository,
                meeting_id,
                ProcessingStage::Analyzing,
                &error,
                generated_at_ms,
            )?;
            return Err(error);
        }
    };
    persist_analysis_report_resilient(repository, meeting_id, analysis, generated_at_ms)
}

fn prepare_analysis_context(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
) -> Result<AnalysisContext, AppError> {
    repository
        .get_meeting(meeting_id)?
        .ok_or_else(|| AppError {
            code: "meeting_not_found".to_string(),
            message: "Cannot analyze a meeting that does not exist.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        })?;
    ensure_report_absent(repository, meeting_id)?;

    let transcript_segments = repository.list_transcript_segments(meeting_id)?;
    if transcript_segments.is_empty() {
        return Err(AppError {
            code: "transcript_not_found".to_string(),
            message: "Cannot analyze a meeting without transcript segments.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        });
    }

    let metrics = repository.list_metrics(meeting_id)?;
    if metrics.is_empty() {
        return Err(AppError {
            code: "metrics_not_found".to_string(),
            message: "Cannot analyze a meeting without deterministic metrics.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id.as_str())),
        });
    }

    Ok(AnalysisContext {
        transcript_segments: transcript_segments
            .into_iter()
            .map(|segment| AnalysisTranscriptSegment {
                sequence_number: segment.sequence_number,
                speaker_role: classify_transcript_speaker_role(segment.speaker_label.as_deref()),
                speaker_label: segment.speaker_label,
                text: segment.text,
                started_at_ms: segment.started_at_ms,
                ended_at_ms: segment.ended_at_ms,
            })
            .collect(),
        metrics: metrics
            .into_iter()
            .map(|metric| AnalysisMetricContext {
                name: metric.name,
                value: metric.value,
                unit: metric.unit,
            })
            .collect(),
    })
}

fn transcript_segments_to_analysis_segments(
    segments: &[TranscriptSegment],
) -> Vec<AnalysisTranscriptSegment> {
    transcript_segments_to_analysis_segments_with_voice_matches(
        segments,
        &[],
        DEFAULT_SPEAKER_MATCH_THRESHOLD,
    )
}

fn imported_speaking_improvements_source(
    include_main_speaker_improvements: bool,
    has_voice_matched_windows: bool,
) -> ImportedSpeakingImprovementsSource {
    if has_voice_matched_windows {
        ImportedSpeakingImprovementsSource::VoiceMatch
    } else if include_main_speaker_improvements {
        ImportedSpeakingImprovementsSource::MainSpeaker
    } else {
        ImportedSpeakingImprovementsSource::None
    }
}

fn imported_speaking_improvements_source_to_db(
    source: ImportedSpeakingImprovementsSource,
) -> &'static str {
    match source {
        ImportedSpeakingImprovementsSource::None => "none",
        ImportedSpeakingImprovementsSource::MainSpeaker => "main_speaker",
        ImportedSpeakingImprovementsSource::VoiceMatch => "voice_match",
    }
}

fn imported_speaking_improvements_source_from_db(
    source: &str,
) -> Result<ImportedSpeakingImprovementsSource, AppError> {
    match source {
        "none" => Ok(ImportedSpeakingImprovementsSource::None),
        "main_speaker" => Ok(ImportedSpeakingImprovementsSource::MainSpeaker),
        "voice_match" => Ok(ImportedSpeakingImprovementsSource::VoiceMatch),
        _ => Err(AppError {
            code: "imported_summary_source_invalid".to_string(),
            message: "Saved imported-recording speaking coaching source is invalid.".to_string(),
            details: Some(format!("source={source}")),
        }),
    }
}

fn imported_meeting_visual_review(
    source_path: &Path,
    ffmpeg_bin_path: Option<&str>,
    meeting_id: &MeetingId,
    settings: &ResonanceSettings,
    voice_match_windows: &[VoiceMatchWindow],
    allow_cloud_video_for_this_review: bool,
) -> Result<ImportedMeetingVisualReview, AppError> {
    if !allow_cloud_video_for_this_review {
        return Ok(imported_meeting_visual_review_status(
            ImportedMeetingVisualReviewStatus::NotRequested,
            None,
            "Visual review was not requested for this imported meeting recording.",
            "No meeting video frames were sent to OpenAI.",
            Vec::new(),
        ));
    }
    if !is_supported_imported_video_path(source_path) {
        return Ok(imported_meeting_visual_review_status(
            ImportedMeetingVisualReviewStatus::AudioOnly,
            None,
            "Visual review is only available for imported meeting video files.",
            "No meeting video frames were sent to OpenAI.",
            Vec::new(),
        ));
    }
    if voice_match_windows.is_empty() {
        return Ok(imported_meeting_visual_review_status(
            ImportedMeetingVisualReviewStatus::AudioOnly,
            None,
            "Audio-only review: no locally matched user speech windows were found, so Resonance cannot connect sampled meeting frames to you.",
            "No meeting video frames were sent to OpenAI.",
            Vec::new(),
        ));
    }

    let request = PracticeVideoReviewRequest {
        practice_recording_id: meeting_id.as_str().to_string(),
        video_file_path: source_path.to_string_lossy().into_owned(),
        ffmpeg_bin_path: ffmpeg_bin_path.map(str::to_string),
        matched_speech_windows: voice_match_windows
            .iter()
            .map(|window| VideoReviewWindow {
                started_at_ms: window.started_at_ms,
                ended_at_ms: window.ended_at_ms,
            })
            .collect(),
        allow_cloud_video_for_this_review,
        cloud_video_review_enabled: settings.cloud_video_review_enabled,
    };
    match OpenAiVideoReviewer::from_environment().analyze_practice_video(&request) {
        Ok(review) => Ok(imported_meeting_visual_review_from_video_review(review)),
        Err(error) => Ok(imported_meeting_visual_review_status(
            ImportedMeetingVisualReviewStatus::AudioOnly,
            None,
            format!(
                "Audio-only review: visual review could not run ({}) {}",
                error.code, error.message
            ),
            "No visual feedback was saved because the cloud visual review request failed.",
            Vec::new(),
        )),
    }
}

fn imported_meeting_visual_review_from_video_review(
    review: PracticeVideoReview,
) -> ImportedMeetingVisualReview {
    if review.user_visible == Some(false) {
        return imported_meeting_visual_review_status(
            ImportedMeetingVisualReviewStatus::UserNotVisible,
            None,
            review.summary,
            "Sampled meeting frames were sent to OpenAI after explicit consent, but the user was not visibly identifiable on camera.",
            Vec::new(),
        );
    }

    imported_meeting_visual_review_status(
        ImportedMeetingVisualReviewStatus::Complete,
        review.visual_score,
        review.summary,
        "Sampled meeting frames were sent to OpenAI after explicit consent.",
        review
            .annotations
            .into_iter()
            .map(|annotation| ImportedMeetingVisualAnnotation {
                started_at_ms: annotation.started_at_ms,
                ended_at_ms: annotation.ended_at_ms,
                category: annotation.category,
                severity: annotation.severity,
                evidence: annotation.evidence,
                suggestion: annotation.suggestion,
                source: annotation.source,
            })
            .collect(),
    )
}

fn imported_meeting_visual_review_status(
    status: ImportedMeetingVisualReviewStatus,
    visual_score: Option<Score>,
    summary: impl Into<String>,
    privacy_note: impl Into<String>,
    annotations: Vec<ImportedMeetingVisualAnnotation>,
) -> ImportedMeetingVisualReview {
    ImportedMeetingVisualReview {
        status,
        visual_score,
        summary: summary.into(),
        privacy_note: privacy_note.into(),
        annotations,
    }
}

fn is_supported_imported_video_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("mov" | "mp4" | "webm")
    )
}

fn imported_voice_match_windows(
    match_result: &VoiceDiarizationMatchResult,
) -> Vec<VoiceMatchWindow> {
    match_result
        .matched_windows
        .iter()
        .map(|window| VoiceMatchWindow {
            started_at_ms: window.started_at_ms,
            ended_at_ms: window.ended_at_ms,
            similarity_score: window.similarity_score,
            threshold: window.threshold,
        })
        .collect()
}

fn transcript_segments_to_analysis_segments_with_voice_matches(
    segments: &[TranscriptSegment],
    matched_windows: &[VoiceMatchWindow],
    threshold: f32,
) -> Vec<AnalysisTranscriptSegment> {
    segments
        .iter()
        .map(|segment| {
            let is_user_match = matched_windows
                .iter()
                .any(|window| voice_match_window_overlaps_segment(window, segment, threshold));
            let speaker_label = if is_user_match {
                Some("User".to_string())
            } else {
                segment.speaker_label.clone()
            };
            let speaker_role = if !matched_windows.is_empty() && !is_user_match {
                analysis::TranscriptSpeakerRole::Context
            } else {
                classify_transcript_speaker_role(speaker_label.as_deref())
            };
            AnalysisTranscriptSegment {
                sequence_number: segment.sequence_number,
                speaker_role,
                speaker_label,
                text: segment.text.clone(),
                started_at_ms: segment.started_at_ms,
                ended_at_ms: segment.ended_at_ms,
            }
        })
        .collect()
}

fn voice_match_window_overlaps_segment(
    window: &VoiceMatchWindow,
    segment: &TranscriptSegment,
    threshold: f32,
) -> bool {
    window.similarity_score >= threshold
        && window.started_at_ms < segment.ended_at_ms
        && window.ended_at_ms > segment.started_at_ms
}

fn run_blocking_ollama_analysis(context: AnalysisContext) -> Result<CoachingAnalysis, AppError> {
    std::thread::spawn(move || {
        let analyzer = OllamaAnalyzer::default_local();
        analyzer.analyze(&context)
    })
    .join()
    .map_err(|_| AppError {
        code: "analysis_thread_failed".to_string(),
        message: "Local analysis worker failed before producing a report.".to_string(),
        details: None,
    })?
}

fn run_blocking_ollama_summary(
    transcript_segments: Vec<AnalysisTranscriptSegment>,
    include_speaking_improvements: bool,
) -> Result<MeetingSummary, AppError> {
    std::thread::spawn(move || {
        let analyzer = OllamaAnalyzer::default_local();
        analyzer.summarize(&transcript_segments, include_speaking_improvements)
    })
    .join()
    .map_err(|_| AppError {
        code: "summary_thread_failed".to_string(),
        message: "Local summary worker failed before producing a report.".to_string(),
        details: None,
    })?
}

fn enroll_voice_profile_from_meeting_record(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    meeting_id: &MeetingId,
    enrolled_at_ms: u64,
) -> Result<VoiceProfileStatus, AppError> {
    let metadata = load_meeting_audio_metadata(repository, meeting_id)?;
    let source_path = Path::new(&metadata.file_path);
    let source_metadata = fs::metadata(source_path).map_err(|error| AppError {
        code: "voice_enrollment_source_unavailable".to_string(),
        message: "Could not inspect the selected voice enrollment recording.".to_string(),
        details: Some(error.to_string()),
    })?;
    if !source_metadata.is_file() {
        return Err(AppError {
            code: "voice_enrollment_source_unavailable".to_string(),
            message: "Voice enrollment requires an existing microphone recording file.".to_string(),
            details: None,
        });
    }
    ensure_path_under_app_data(
        app_data_dir,
        source_path,
        "voice_enrollment_source_rejected",
    )?;

    let sample_path = safe_voice_profile_sample_path(app_data_dir)?;
    fs::copy(source_path, &sample_path).map_err(|error| AppError {
        code: "voice_enrollment_copy_failed".to_string(),
        message: "Could not save the local voice enrollment sample.".to_string(),
        details: Some(error.to_string()),
    })?;
    let copied_metadata = fs::metadata(&sample_path).map_err(|error| AppError {
        code: "voice_enrollment_sample_unavailable".to_string(),
        message: "Could not inspect the saved voice enrollment sample.".to_string(),
        details: Some(error.to_string()),
    })?;

    let profile = repository.upsert_voice_profile(&VoiceProfileRecord {
        sample_audio_file_path: sample_path.to_string_lossy().into_owned(),
        sample_duration_ms: metadata.duration_ms,
        sample_byte_size: copied_metadata.len(),
        enrolled_at_ms,
        embedding_json: None,
        embedding_dimension: None,
        embedding_model_path: None,
        embedding_computed_at_ms: None,
    })?;
    let settings = load_effective_settings(repository)?;
    voice_profile_status(
        Some(profile),
        settings.speaker_embedding_model_path.as_deref(),
    )
}

fn voice_profile_status(
    profile: Option<VoiceProfileRecord>,
    configured_model_path: Option<&str>,
) -> Result<VoiceProfileStatus, AppError> {
    Ok(match profile {
        Some(profile) => VoiceProfileStatus {
            is_enrolled: true,
            enrolled_at_ms: Some(profile.enrolled_at_ms),
            sample_duration_ms: profile.sample_duration_ms,
            sample_byte_size: Some(profile.sample_byte_size),
            matching_ready: voice_profile_embedding_matches_model(&profile, configured_model_path),
        },
        None => VoiceProfileStatus {
            is_enrolled: false,
            enrolled_at_ms: None,
            sample_duration_ms: None,
            sample_byte_size: None,
            matching_ready: false,
        },
    })
}

fn prepare_voice_profile_for_matching_record(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    computed_at_ms: u64,
) -> Result<VoiceProfileStatus, AppError> {
    let mut profile = repository.get_voice_profile()?.ok_or_else(|| AppError {
        code: "voice_profile_not_enrolled".to_string(),
        message: "Enroll a local voice profile before preparing voice matching.".to_string(),
        details: None,
    })?;
    let settings = load_effective_settings(repository)?;
    let model_path_text = settings
        .speaker_embedding_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| AppError {
            code: "speaker_embedding_model_not_configured".to_string(),
            message: "Configure a local speaker embedding model before preparing voice matching."
                .to_string(),
            details: None,
        })?;
    let sample_path = Path::new(&profile.sample_audio_file_path);
    ensure_path_under_app_data(
        app_data_dir,
        sample_path,
        "voice_profile_prepare_sample_rejected",
    )?;
    let prepared_embedding = prepare_voice_embedding(sample_path, Path::new(model_path_text))?;
    profile.embedding_json = Some(prepared_embedding.embedding_json);
    profile.embedding_dimension = Some(prepared_embedding.embedding_dimension);
    profile.embedding_model_path = Some(prepared_embedding.embedding_model_path);
    profile.embedding_computed_at_ms = Some(computed_at_ms);

    let updated_profile = repository.upsert_voice_profile(&profile)?;
    voice_profile_status(Some(updated_profile), Some(model_path_text))
}

fn match_voice_profile_from_meeting_record(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    meeting_id: &MeetingId,
    threshold: f32,
) -> Result<VoiceMatchResult, AppError> {
    let (enrolled_embedding, model_path_text) = read_prepared_voice_profile_embedding(repository)?;
    let metadata = load_meeting_audio_metadata(repository, meeting_id)?;
    let candidate_path = Path::new(&metadata.file_path);
    match_voice_profile_audio_path_with_embedding(
        app_data_dir,
        candidate_path,
        threshold,
        &enrolled_embedding,
        &model_path_text,
    )
}

fn match_voice_profile_audio_path_record(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    candidate_path: &Path,
    threshold: f32,
) -> Result<VoiceMatchResult, AppError> {
    let (enrolled_embedding, model_path_text) = read_prepared_voice_profile_embedding(repository)?;
    match_voice_profile_audio_path_with_embedding(
        app_data_dir,
        candidate_path,
        threshold,
        &enrolled_embedding,
        &model_path_text,
    )
}

fn diarize_imported_audio_recording(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    audio_path: &Path,
) -> Result<VoiceDiarizationResult, AppError> {
    ensure_path_under_app_data(
        app_data_dir,
        audio_path,
        "speaker_diarization_audio_rejected",
    )?;
    let settings = load_effective_settings(repository)?;
    let segmentation_model_path = settings
        .speaker_segmentation_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| AppError {
            code: "speaker_segmentation_model_not_configured".to_string(),
            message: "Configure a local speaker segmentation model before diarizing recordings."
                .to_string(),
            details: None,
        })?;
    let embedding_model_path = settings
        .speaker_embedding_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| AppError {
            code: "speaker_embedding_model_not_configured".to_string(),
            message: "Configure a local speaker embedding model before diarizing recordings."
                .to_string(),
            details: None,
        })?;

    diarize_speakers(
        audio_path,
        Path::new(segmentation_model_path),
        Path::new(embedding_model_path),
    )
}

fn match_imported_audio_speaker_segments(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    audio_path: &Path,
    threshold: f32,
) -> Result<VoiceDiarizationMatchResult, AppError> {
    ensure_path_under_app_data(
        app_data_dir,
        audio_path,
        "speaker_diarization_audio_rejected",
    )?;
    let (enrolled_embedding, embedding_model_path) =
        read_prepared_voice_profile_embedding(repository)?;
    let settings = load_effective_settings(repository)?;
    let segmentation_model_path = settings
        .speaker_segmentation_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| AppError {
            code: "speaker_segmentation_model_not_configured".to_string(),
            message:
                "Configure a local speaker segmentation model before matching diarized speakers."
                    .to_string(),
            details: None,
        })?;

    match_diarized_speakers(
        audio_path,
        Path::new(segmentation_model_path),
        Path::new(&embedding_model_path),
        &enrolled_embedding,
        threshold,
    )
}

fn read_prepared_voice_profile_embedding(
    repository: &SqliteRepository,
) -> Result<(Vec<f32>, String), AppError> {
    let profile = repository.get_voice_profile()?.ok_or_else(|| AppError {
        code: "voice_profile_not_enrolled".to_string(),
        message: "Enroll a local voice profile before matching candidate audio.".to_string(),
        details: None,
    })?;
    let settings = load_effective_settings(repository)?;
    let model_path_text = settings
        .speaker_embedding_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| AppError {
            code: "speaker_embedding_model_not_configured".to_string(),
            message: "Configure a local speaker embedding model before matching candidate audio."
                .to_string(),
            details: None,
        })?;
    if !voice_profile_embedding_matches_model(&profile, Some(model_path_text)) {
        return Err(AppError {
            code: "voice_profile_not_ready_for_matching".to_string(),
            message: "Prepare the local voice profile before matching candidate audio.".to_string(),
            details: None,
        });
    }

    let enrolled_embedding = profile
        .embedding_json
        .as_deref()
        .ok_or_else(|| AppError {
            code: "voice_profile_not_ready_for_matching".to_string(),
            message: "Prepare the local voice profile before matching candidate audio.".to_string(),
            details: None,
        })
        .and_then(parse_persisted_voice_embedding)?;
    Ok((enrolled_embedding, model_path_text.to_string()))
}

fn match_voice_profile_audio_path_with_embedding(
    app_data_dir: &Path,
    candidate_path: &Path,
    threshold: f32,
    enrolled_embedding: &[f32],
    model_path_text: &str,
) -> Result<VoiceMatchResult, AppError> {
    ensure_path_under_app_data(
        app_data_dir,
        candidate_path,
        "voice_match_candidate_rejected",
    )?;
    let candidate_embedding = prepare_voice_embedding(candidate_path, Path::new(model_path_text))?;
    let candidate_embedding_values =
        parse_persisted_voice_embedding(&candidate_embedding.embedding_json)?;

    compare_voice_embeddings(&enrolled_embedding, &candidate_embedding_values, threshold)
}

fn parse_persisted_voice_embedding(embedding_json: &str) -> Result<Vec<f32>, AppError> {
    let embedding = serde_json::from_str::<Vec<f32>>(embedding_json).map_err(|error| AppError {
        code: "voice_profile_embedding_invalid".to_string(),
        message: "Saved voice profile embedding could not be read.".to_string(),
        details: Some(error.to_string()),
    })?;
    if embedding.is_empty() {
        return Err(AppError {
            code: "voice_profile_embedding_invalid".to_string(),
            message: "Saved voice profile embedding is empty.".to_string(),
            details: None,
        });
    }
    Ok(embedding)
}

fn voice_profile_embedding_matches_model(
    profile: &VoiceProfileRecord,
    configured_model_path: Option<&str>,
) -> bool {
    profile.embedding_json.is_some()
        && profile
            .embedding_model_path
            .as_deref()
            .zip(configured_model_path)
            .is_some_and(|(embedded_model_path, configured_model_path)| {
                embedded_model_path == configured_model_path
            })
}

fn safe_voice_profile_sample_path(app_data_dir: &Path) -> Result<PathBuf, AppError> {
    let directory = app_data_dir.join(VOICE_PROFILE_DIR_NAME);
    fs::create_dir_all(&directory).map_err(|error| AppError {
        code: "voice_profile_directory_error".to_string(),
        message: "Could not create the local voice profile directory.".to_string(),
        details: Some(error.to_string()),
    })?;
    Ok(directory.join(VOICE_PROFILE_SAMPLE_FILE_NAME))
}

fn delete_voice_profile_sample(
    app_data_dir: &Path,
    sample_audio_file_path: &str,
) -> Result<(), AppError> {
    let sample_path = Path::new(sample_audio_file_path);
    if !sample_path.exists() {
        return Ok(());
    }
    let canonical_sample =
        ensure_path_under_app_data(app_data_dir, sample_path, "voice_profile_delete_rejected")?;
    fs::remove_file(canonical_sample).map_err(|error| AppError {
        code: "voice_profile_delete_failed".to_string(),
        message: "Could not delete the local voice sample.".to_string(),
        details: Some(error.to_string()),
    })?;
    Ok(())
}

fn ensure_path_under_app_data(
    app_data_dir: &Path,
    path: &Path,
    rejected_code: &str,
) -> Result<PathBuf, AppError> {
    let canonical_root = app_data_dir.canonicalize().map_err(|error| AppError {
        code: "app_data_path_validation_failed".to_string(),
        message: "Could not inspect the application data directory.".to_string(),
        details: Some(error.to_string()),
    })?;
    let canonical_path = path.canonicalize().map_err(|error| AppError {
        code: "local_path_validation_failed".to_string(),
        message: "Could not inspect the local file path.".to_string(),
        details: Some(error.to_string()),
    })?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AppError {
            code: rejected_code.to_string(),
            message: "Refusing to use a local file outside Resonance app data.".to_string(),
            details: None,
        });
    }
    Ok(canonical_path)
}

fn normalize_search_query(search_query: Option<String>) -> Option<String> {
    search_query
        .map(|query| query.trim().chars().take(120).collect::<String>())
        .filter(|query| !query.is_empty())
}

fn history_record_to_item(record: MeetingHistoryRecord) -> MeetingHistoryItem {
    MeetingHistoryItem {
        meeting_id: record.id,
        title: record.title,
        started_at_ms: record.started_at_ms,
        stopped_at_ms: record.stopped_at_ms,
        updated_at_ms: record.updated_at_ms,
        duration_ms: record.duration_ms.or_else(|| {
            record
                .stopped_at_ms
                .map(|stopped_at_ms| stopped_at_ms.saturating_sub(record.started_at_ms))
        }),
        audio_file_path: record.audio_file_path,
        status: meeting_history_status(
            record.stopped_at_ms,
            record.transcript_segment_count as usize,
            record.report_id.is_some(),
            record.pipeline_failure.is_some(),
        ),
        transcript_segment_count: record.transcript_segment_count,
        latest_report_id: record.report_id,
        latest_report_score: record.overall_score,
        latest_report_generated_at_ms: record.report_generated_at_ms,
        pipeline_failure: record.pipeline_failure,
    }
}

fn trend_record_to_point(record: MeetingTrendRecord) -> MeetingTrendPoint {
    MeetingTrendPoint {
        meeting_id: record.id,
        title: record.title,
        started_at_ms: record.started_at_ms,
        filler_word_count: record.filler_word_count,
        words_per_minute: record.words_per_minute,
        overall_score: record.overall_score,
    }
}

fn meeting_history_status(
    stopped_at_ms: Option<u64>,
    transcript_segment_count: usize,
    has_report: bool,
    has_pipeline_failure: bool,
) -> MeetingHistoryStatus {
    if has_pipeline_failure {
        return MeetingHistoryStatus::FailedPartial;
    }
    if stopped_at_ms.is_none() {
        return MeetingHistoryStatus::Recording;
    }
    if has_report {
        return MeetingHistoryStatus::Analyzed;
    }
    if transcript_segment_count > 0 {
        return MeetingHistoryStatus::Transcribed;
    }
    MeetingHistoryStatus::Recorded
}

fn analysis_result_from_report(
    repository: &SqliteRepository,
    report: persistence::ReportRecord,
) -> Result<AnalysisResult, AppError> {
    let analysis =
        serde_json::from_str::<CoachingAnalysis>(&report.body_json).map_err(|error| AppError {
            code: "report_deserialization_failed".to_string(),
            message: "Could not load the saved coaching analysis for this meeting.".to_string(),
            details: Some(error.to_string()),
        })?;
    let scorecard = scorecard_for_meeting(repository, &report.meeting_id, report.overall_score)?;

    Ok(AnalysisResult {
        meeting_id: report.meeting_id,
        report_id: report.id,
        analysis,
        scorecard,
        generated_at_ms: report.generated_at_ms,
    })
}

fn imported_summary_history_from_record(
    summary: persistence::ImportedMeetingSummaryRecord,
) -> Result<ImportedMeetingSummaryHistory, AppError> {
    let body_json =
        serde_json::from_str::<MeetingSummary>(&summary.body_json).map_err(|error| AppError {
            code: "imported_summary_deserialization_failed".to_string(),
            message: "Could not load the saved imported-recording summary.".to_string(),
            details: Some(error.to_string()),
        })?;

    Ok(ImportedMeetingSummaryHistory {
        summary_id: summary.id,
        source_file_path: summary.source_file_path,
        extracted_audio_file_path: summary.extracted_audio_file_path,
        speaking_improvements_source: imported_speaking_improvements_source_from_db(
            &summary.speaking_improvements_source,
        )?,
        summary: body_json,
        generated_at_ms: summary.generated_at_ms,
    })
}

fn persist_analysis_report(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    analysis: CoachingAnalysis,
    generated_at_ms: u64,
) -> Result<AnalysisResult, AppError> {
    ensure_report_absent(repository, meeting_id)?;
    let report_id = ReportId::new(format!("{}-report-v1", meeting_id.as_str()));
    let scorecard = scorecard_for_meeting(repository, meeting_id, analysis.overall_score)?;
    let body_json = serde_json::to_string(&analysis).map_err(|error| AppError {
        code: "report_serialization_failed".to_string(),
        message: "Could not serialize coaching analysis for report persistence.".to_string(),
        details: Some(error.to_string()),
    })?;

    repository.create_report(&CreateReport {
        id: report_id.clone(),
        meeting_id: meeting_id.clone(),
        overall_score: analysis.overall_score,
        body_json,
        generated_at_ms,
    })?;

    Ok(AnalysisResult {
        meeting_id: meeting_id.clone(),
        report_id,
        analysis,
        scorecard,
        generated_at_ms,
    })
}

fn persist_analysis_report_resilient(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    analysis: CoachingAnalysis,
    generated_at_ms: u64,
) -> Result<AnalysisResult, AppError> {
    match persist_analysis_report(repository, meeting_id, analysis, generated_at_ms) {
        Ok(result) => {
            clear_pipeline_failure_after_success(repository, meeting_id);
            Ok(result)
        }
        Err(error) => {
            persist_pipeline_failure(
                repository,
                meeting_id,
                ProcessingStage::Analyzing,
                &error,
                generated_at_ms,
            )?;
            Err(error)
        }
    }
}

fn persist_pipeline_failure(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    failed_stage: ProcessingStage,
    error: &AppError,
    failed_at_ms: u64,
) -> Result<PipelineFailureRecord, AppError> {
    repository.upsert_pipeline_failure(&CreatePipelineFailure {
        meeting_id: meeting_id.clone(),
        failed_stage,
        error_code: error.code.clone(),
        error_message: error.message.clone(),
        error_details: error.details.clone(),
        failed_at_ms,
    })
}

fn clear_pipeline_failure_after_success(repository: &SqliteRepository, meeting_id: &MeetingId) {
    if let Err(error) = repository.clear_pipeline_failure(meeting_id) {
        eprintln!(
            "Pipeline failure cleanup failed after successful retry: meeting_id={}, error_code={}",
            meeting_id.as_str(),
            error.code
        );
    }
}

fn scorecard_for_meeting(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    analyzer_overall_score: domain::Score,
) -> Result<Scorecard, AppError> {
    let metrics = repository
        .list_metrics(meeting_id)?
        .into_iter()
        .map(|metric| rules::RuleMetric {
            name: metric.name,
            value: metric.value,
            unit: metric.unit,
        })
        .collect::<Vec<_>>();

    Ok(calculate_scorecard(&ScoringInput::from_rule_metrics(
        &metrics,
        Some(analyzer_overall_score),
    )))
}

fn ensure_report_absent(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
) -> Result<(), AppError> {
    let reports = repository.list_reports_for_meeting(meeting_id)?;
    if reports.is_empty() {
        return Ok(());
    }

    Err(AppError {
        code: "report_already_exists".to_string(),
        message: "Meeting already has a coaching report. Re-analysis is not supported yet."
            .to_string(),
        details: Some(format!(
            "meeting_id={}, report_count={}",
            meeting_id.as_str(),
            reports.len()
        )),
    })
}

fn ensure_deterministic_metrics_are_absent(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
) -> Result<(), AppError> {
    let deterministic_names = rules::deterministic_metric_names();
    let existing_metric_names = repository
        .list_metrics(meeting_id)?
        .into_iter()
        .map(|metric| metric.name)
        .collect::<Vec<_>>();
    let duplicate_count = existing_metric_names
        .iter()
        .filter(|name| deterministic_names.contains(name))
        .count();

    if duplicate_count == 0 {
        return Ok(());
    }

    Err(AppError {
        code: "metrics_already_exist".to_string(),
        message: "Meeting already has deterministic metrics. Recalculation is not supported yet."
            .to_string(),
        details: Some(format!(
            "meeting_id={}, metric_count={}",
            meeting_id.as_str(),
            duplicate_count
        )),
    })
}

fn normalize_optional_path(path: Option<String>) -> Option<String> {
    path.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_retention_days(retention_days: u16) -> Result<u16, AppError> {
    if retention_days <= MAX_RAW_AUDIO_RETENTION_DAYS {
        return Ok(retention_days);
    }

    Err(AppError {
        code: "invalid_retention_days".to_string(),
        message: "Raw audio retention must be between 0 and 365 days.".to_string(),
        details: Some(format!("raw_audio_retention_days={retention_days}")),
    })
}

fn ensure_analysis_provider_available(settings: &ResonanceSettings) -> Result<(), AppError> {
    match (settings.analyzer_provider, settings.cloud_analysis_enabled) {
        (AnalyzerProvider::LocalOllama, _) => Ok(()),
        (_, false) => Err(AppError {
            code: "cloud_analysis_disabled".to_string(),
            message: "Cloud analysis requires an explicit opt-in before transcript text can leave this Mac."
                .to_string(),
            details: None,
        }),
        (_, true) => Err(AppError {
            code: "cloud_analyzer_unavailable".to_string(),
            message: "Cloud analyzer adapters are not connected yet. Switch back to Local Ollama."
                .to_string(),
            details: Some(format!("analyzer_provider={:?}", settings.analyzer_provider)),
        }),
    }
}

fn next_practice_recording_id(prefix: &str, now_ms: u64) -> Result<PracticeRecordingId, AppError> {
    let sequence = PRACTICE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let id = format!("{prefix}-{now_ms}-{sequence}");
    validate_recording_file_stem(&id)?;
    Ok(PracticeRecordingId::new(id))
}

fn normalize_optional_title(title: Option<String>) -> Option<String> {
    title
        .map(|value| value.trim().chars().take(120).collect::<String>())
        .filter(|value| !value.is_empty())
}

fn ensure_practice_duration_allowed(duration_ms: u64) -> Result<(), AppError> {
    if duration_ms <= MAX_PRACTICE_REVIEW_DURATION_MS {
        return Ok(());
    }
    Err(AppError {
        code: "practice_video_too_long".to_string(),
        message: "Practice videos must be 15 minutes or shorter.".to_string(),
        details: Some(format!("duration_ms={duration_ms}")),
    })
}

fn ensure_practice_recording_under_duration(
    recording: &PracticeRecordingRecord,
) -> Result<(), AppError> {
    if let Some(duration_ms) = recording.duration_ms {
        ensure_practice_duration_allowed(duration_ms)?;
    }
    Ok(())
}

fn practice_not_found_error(practice_id: &PracticeRecordingId) -> AppError {
    AppError {
        code: "practice_recording_not_found".to_string(),
        message: "Practice recording could not be found.".to_string(),
        details: Some(format!("practice_recording_id={}", practice_id.as_str())),
    }
}

fn persist_practice_failure(
    state: &State<'_, AppState>,
    practice_id: &PracticeRecordingId,
    error: &AppError,
) -> Result<(), AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    repository.update_practice_recording_analysis_state(
        practice_id,
        None,
        "failed_partial",
        false,
        Some((&error.code, &error.message)),
        current_time_ms()?,
    )?;
    Ok(())
}

struct PreparedPracticeReview {
    body: PracticeReviewBody,
    overall_score: Option<Score>,
    audio_score: Option<Score>,
    visual_score: Option<Score>,
    annotations: Vec<PracticeVideoAnnotation>,
}

fn practice_audio_review_from_transcript(
    practice_id: &PracticeRecordingId,
    transcription: TranscriptionOutput,
    _generated_at_ms: u64,
) -> Result<PreparedPracticeReview, AppError> {
    if transcription.segments.is_empty() {
        return Err(AppError {
            code: "practice_transcript_not_found".to_string(),
            message: "Practice audio review needs at least one transcript segment.".to_string(),
            details: Some(format!("practice_recording_id={}", practice_id.as_str())),
        });
    }
    let rule_segments = transcription
        .segments
        .iter()
        .map(|segment| RuleTranscriptSegment {
            text: segment.text.clone(),
            started_at_ms: segment.started_at_ms,
            ended_at_ms: segment.ended_at_ms,
        })
        .collect::<Vec<_>>();
    let metrics = rules::calculate_metrics(&rule_segments);
    let scorecard = calculate_scorecard(&ScoringInput::from(&metrics));
    let audio_score = scorecard.overall.score;
    let annotations = practice_audio_annotations(&metrics, &transcription.segments);
    let pace_text = format!("Pace averaged {:.0} WPM.", metrics.words_per_minute);
    let filler_text = if metrics.filler_word_count > 0 {
        format!("Detected {} filler word(s).", metrics.filler_word_count)
    } else {
        "No filler words were detected in the transcript.".to_string()
    };
    let hedging_text = if metrics.hedging_phrase_count > 0 {
        format!(
            "Detected {} hedging phrase(s).",
            metrics.hedging_phrase_count
        )
    } else {
        "No hedging phrases were detected in the transcript.".to_string()
    };
    let suggestions = practice_audio_suggestions(&metrics);

    Ok(PreparedPracticeReview {
        body: PracticeReviewBody {
            summary: "Local audio review completed for this practice recording.".to_string(),
            audio_summary: format!("{pace_text} {filler_text} {hedging_text}"),
            visual_summary: "Visual review has not run. Enable cloud video review and confirm a specific review to request posture, eye-contact, gesture, and framing feedback.".to_string(),
            suggestions,
            privacy_note: "Audio was extracted and reviewed locally. No video was sent to a cloud reviewer.".to_string(),
        },
        overall_score: audio_score,
        audio_score,
        visual_score: None,
        annotations,
    })
}

fn practice_audio_annotations(
    metrics: &MetricsSummary,
    segments: &[TranscriptSegment],
) -> Vec<PracticeVideoAnnotation> {
    let mut annotations = Vec::new();
    let (started_at_ms, ended_at_ms, evidence) = segments
        .first()
        .map(|segment| {
            (
                segment.started_at_ms,
                segment.ended_at_ms,
                segment.text.chars().take(160).collect::<String>(),
            )
        })
        .unwrap_or((0, 0, String::new()));

    if metrics.words_per_minute > 170.0
        || (metrics.words_per_minute > 0.0 && metrics.words_per_minute < 120.0)
    {
        annotations.push(PracticeVideoAnnotation {
            started_at_ms,
            ended_at_ms,
            category: "pace".to_string(),
            severity: "caution".to_string(),
            evidence: format!("{:.0} words per minute", metrics.words_per_minute),
            suggestion: if metrics.words_per_minute > 170.0 {
                "Slow down and add short pauses after key points.".to_string()
            } else {
                "Increase energy slightly so the delivery does not drag.".to_string()
            },
            source: "audioLocal".to_string(),
        });
    }
    if metrics.filler_word_count > 0 {
        annotations.push(PracticeVideoAnnotation {
            started_at_ms,
            ended_at_ms,
            category: "fillerWords".to_string(),
            severity: if metrics.filler_word_count > 5 {
                "strong"
            } else {
                "caution"
            }
            .to_string(),
            evidence: evidence.clone(),
            suggestion: "Replace fillers with a silent pause before continuing.".to_string(),
            source: "audioLocal".to_string(),
        });
    }
    if metrics.hedging_phrase_count > 0 {
        annotations.push(PracticeVideoAnnotation {
            started_at_ms,
            ended_at_ms,
            category: "clarity".to_string(),
            severity: "caution".to_string(),
            evidence,
            suggestion: "Lead with the assertion first, then add nuance if needed.".to_string(),
            source: "audioLocal".to_string(),
        });
    }
    annotations
}

fn practice_audio_suggestions(metrics: &MetricsSummary) -> Vec<String> {
    let mut suggestions = Vec::new();
    if metrics.words_per_minute > 170.0 {
        suggestions.push("Reduce pace by pausing after important sentences.".to_string());
    } else if metrics.words_per_minute > 0.0 && metrics.words_per_minute < 120.0 {
        suggestions.push("Raise energy and tighten phrasing to keep momentum.".to_string());
    }
    if metrics.filler_word_count > 0 {
        suggestions.push("Use a silent pause instead of filler words.".to_string());
    }
    if metrics.hedging_phrase_count > 0 {
        suggestions.push("Replace hedging with a direct recommendation.".to_string());
    }
    if suggestions.is_empty() {
        suggestions.push("Keep the current pace and clarity; add visual review for posture and eye-contact feedback.".to_string());
    }
    suggestions
}

fn practice_review_from_video_review(
    _practice_id: &PracticeRecordingId,
    video_review: PracticeVideoReview,
    _generated_at_ms: u64,
) -> Result<PreparedPracticeReview, AppError> {
    let visual_score = video_review.visual_score;
    Ok(PreparedPracticeReview {
        body: PracticeReviewBody {
            summary: "Visual review completed for this practice recording.".to_string(),
            audio_summary: "Audio review has not run in this pass.".to_string(),
            visual_summary: video_review.summary,
            suggestions: video_review
                .annotations
                .iter()
                .map(|annotation| annotation.suggestion.clone())
                .collect(),
            privacy_note: if video_review.cloud_video_used {
                "Cloud video review was used after saved opt-in and per-review confirmation."
                    .to_string()
            } else {
                "Video stayed local.".to_string()
            },
        },
        overall_score: visual_score,
        audio_score: None,
        visual_score,
        annotations: video_review
            .annotations
            .into_iter()
            .map(|annotation| PracticeVideoAnnotation {
                source: annotation.source,
                ..annotation
            })
            .collect::<Vec<_>>(),
    })
}

fn persist_practice_review(
    state: &State<'_, AppState>,
    practice_id: &PracticeRecordingId,
    extracted_audio_file_path: Option<&str>,
    review: PreparedPracticeReview,
    cloud_video_used: bool,
    generated_at_ms: u64,
) -> Result<PracticeReviewResult, AppError> {
    let body_json = serde_json::to_string(&review.body).map_err(|error| AppError {
        code: "practice_review_serialization_failed".to_string(),
        message: "Could not serialize the practice review report.".to_string(),
        details: Some(error.to_string()),
    })?;
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let report = repository.create_practice_review_report(&CreatePracticeReviewReport {
        id: PracticeReviewReportId::new(format!("{}-review", practice_id.as_str())),
        practice_recording_id: practice_id.clone(),
        overall_score: review.overall_score,
        audio_score: review.audio_score,
        visual_score: review.visual_score,
        body_json,
        generated_at_ms,
    })?;
    let annotations = review
        .annotations
        .into_iter()
        .enumerate()
        .map(|(index, annotation)| CreatePracticeTimelineAnnotation {
            id: PracticeAnnotationId::new(format!("{}-annotation-{index}", practice_id.as_str())),
            practice_recording_id: practice_id.clone(),
            started_at_ms: annotation.started_at_ms,
            ended_at_ms: annotation.ended_at_ms,
            category: annotation.category,
            severity: annotation.severity,
            evidence: annotation.evidence,
            suggestion: annotation.suggestion,
            source: annotation.source,
        })
        .collect::<Vec<_>>();
    let annotations =
        repository.replace_practice_timeline_annotations(practice_id, &annotations)?;
    let recording = repository.update_practice_recording_analysis_state(
        practice_id,
        extracted_audio_file_path,
        "complete",
        cloud_video_used,
        None,
        generated_at_ms,
    )?;

    Ok(PracticeReviewResult {
        recording: practice_recording_from_record(recording),
        report: practice_report_from_record(report)?,
        annotations: annotations
            .into_iter()
            .map(practice_annotation_from_record)
            .collect(),
    })
}

fn practice_recording_from_record(record: PracticeRecordingRecord) -> PracticeRecording {
    PracticeRecording {
        id: record.id,
        title: record.title,
        source_kind: record.source_kind,
        video_file_path: record.video_file_path,
        extracted_audio_file_path: record.extracted_audio_file_path,
        duration_ms: record.duration_ms,
        byte_size: record.byte_size,
        recorded_at_ms: record.recorded_at_ms,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
        analysis_status: record.analysis_status,
        cloud_video_used: record.cloud_video_used,
        pipeline_failure_code: record.pipeline_failure_code,
        pipeline_failure_message: record.pipeline_failure_message,
    }
}

fn practice_report_from_record(
    record: PracticeReviewReportRecord,
) -> Result<PracticeReviewReport, AppError> {
    let body = serde_json::from_str::<PracticeReviewBody>(&record.body_json).map_err(|error| {
        AppError {
            code: "practice_review_deserialization_failed".to_string(),
            message: "Could not load the saved practice review body.".to_string(),
            details: Some(error.to_string()),
        }
    })?;
    Ok(PracticeReviewReport {
        id: record.id,
        practice_recording_id: record.practice_recording_id,
        overall_score: record.overall_score,
        audio_score: record.audio_score,
        visual_score: record.visual_score,
        body,
        generated_at_ms: record.generated_at_ms,
    })
}

fn practice_annotation_from_record(
    record: PracticeTimelineAnnotationRecord,
) -> PracticeTimelineAnnotation {
    PracticeTimelineAnnotation {
        id: record.id,
        practice_recording_id: record.practice_recording_id,
        started_at_ms: record.started_at_ms,
        ended_at_ms: record.ended_at_ms,
        category: record.category,
        severity: record.severity,
        evidence: record.evidence,
        suggestion: record.suggestion,
        source: record.source,
    }
}

fn apply_audio_retention_policy(
    repository: &SqliteRepository,
    app_data_dir: &Path,
    retention_days: u16,
    now_ms: u64,
) -> Result<RetentionCleanupSummary, AppError> {
    let cutoff_ms = retention_cutoff_ms(retention_days, now_ms);
    let expired_metadata = repository.list_audio_metadata_before(cutoff_ms)?;
    let mut summary = delete_retained_audio_files(&expired_metadata, app_data_dir)?;
    remove_retained_audio_metadata(repository, &expired_metadata, &mut summary)?;
    let expired_practice_recordings = repository.list_practice_recordings_before(cutoff_ms)?;
    delete_retained_practice_files(&expired_practice_recordings, app_data_dir, &mut summary)?;
    Ok(summary)
}

fn spawn_audio_retention_cleanup(database_path: PathBuf, app_data_dir: PathBuf) {
    std::thread::spawn(move || {
        if let Err(error) = run_audio_retention_cleanup(database_path, app_data_dir) {
            eprintln!("Audio retention cleanup failed: {}", error.message);
        }
    });
}

fn run_audio_retention_cleanup(
    database_path: PathBuf,
    app_data_dir: PathBuf,
) -> Result<(), AppError> {
    let repository = SqliteRepository::open(database_path)?;
    let settings = repository.get_settings()?;
    apply_audio_retention_policy(
        &repository,
        &app_data_dir,
        settings.raw_audio_retention_days,
        current_time_ms()?,
    )?;
    Ok(())
}

fn delete_retained_audio_files(
    expired_metadata: &[AudioMetadata],
    app_data_dir: &Path,
) -> Result<RetentionCleanupSummary, AppError> {
    let mut summary = RetentionCleanupSummary {
        deleted_audio_file_count: 0,
        deleted_practice_file_count: 0,
        removed_audio_metadata_count: 0,
        skipped_audio_file_count: 0,
        skipped_practice_file_count: 0,
    };

    for metadata in expired_metadata {
        for file_path in retention_file_paths(&metadata) {
            match delete_retained_audio_file(&file_path, app_data_dir)? {
                RetainedAudioDeleteOutcome::Deleted => {
                    summary.deleted_audio_file_count += 1;
                }
                RetainedAudioDeleteOutcome::Missing => {}
                RetainedAudioDeleteOutcome::Skipped => {
                    summary.skipped_audio_file_count += 1;
                }
            }
        }
    }

    Ok(summary)
}

fn delete_retained_practice_files(
    expired_recordings: &[PracticeRecordingRecord],
    app_data_dir: &Path,
    summary: &mut RetentionCleanupSummary,
) -> Result<(), AppError> {
    for recording in expired_recordings {
        for file_path in practice_retention_file_paths(recording) {
            match delete_retained_audio_file(&file_path, app_data_dir)? {
                RetainedAudioDeleteOutcome::Deleted => summary.deleted_practice_file_count += 1,
                RetainedAudioDeleteOutcome::Missing => {}
                RetainedAudioDeleteOutcome::Skipped => summary.skipped_practice_file_count += 1,
            }
        }
    }
    Ok(())
}

fn practice_retention_file_paths(recording: &PracticeRecordingRecord) -> Vec<String> {
    let mut paths = vec![recording.video_file_path.clone()];
    if let Some(extracted_audio_file_path) = &recording.extracted_audio_file_path {
        paths.push(extracted_audio_file_path.clone());
    }
    paths
}

fn remove_retained_audio_metadata(
    repository: &SqliteRepository,
    expired_metadata: &[AudioMetadata],
    summary: &mut RetentionCleanupSummary,
) -> Result<(), AppError> {
    for metadata in expired_metadata {
        if repository.delete_audio_metadata(&metadata.meeting_id)? {
            summary.removed_audio_metadata_count += 1;
        }
    }
    Ok(())
}

fn retention_cutoff_ms(retention_days: u16, now_ms: u64) -> u64 {
    now_ms.saturating_sub(u64::from(retention_days) * MILLIS_PER_DAY)
}

fn retention_file_paths(metadata: &AudioMetadata) -> Vec<String> {
    let mut paths = vec![metadata.file_path.clone()];
    if let Some(system_audio_file_path) = &metadata.system_audio_file_path {
        paths.push(system_audio_file_path.clone());
    }
    paths
}

enum RetainedAudioDeleteOutcome {
    Deleted,
    Missing,
    Skipped,
}

fn delete_retained_audio_file(
    file_path: &str,
    app_data_dir: &Path,
) -> Result<RetainedAudioDeleteOutcome, AppError> {
    let path = PathBuf::from(file_path);
    if !path.is_absolute() {
        return Ok(RetainedAudioDeleteOutcome::Skipped);
    }

    let canonical_root = app_data_dir.canonicalize().map_err(|error| AppError {
        code: "retention_root_unavailable".to_string(),
        message: "Could not verify the application data directory before deleting retained audio."
            .to_string(),
        details: Some(error.to_string()),
    })?;
    let canonical_path = match path.canonicalize() {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RetainedAudioDeleteOutcome::Missing);
        }
        Err(error) => {
            return Err(AppError {
                code: "retained_audio_path_unavailable".to_string(),
                message: "Could not verify retained audio path before deletion.".to_string(),
                details: Some(format!("path={}, error={error}", path.display())),
            });
        }
    };

    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        return Ok(RetainedAudioDeleteOutcome::Skipped);
    }

    fs::remove_file(&canonical_path).map_err(|error| AppError {
        code: "retained_audio_delete_failed".to_string(),
        message: "Could not delete retained raw audio.".to_string(),
        details: Some(format!("path={}, error={error}", canonical_path.display())),
    })?;
    Ok(RetainedAudioDeleteOutcome::Deleted)
}

fn ensure_meeting_exists(
    repository: &SqliteRepository,
    meeting_id: &MeetingId,
    started_at_ms: u64,
) -> Result<(), AppError> {
    if repository.get_meeting(meeting_id)?.is_some() {
        return Ok(());
    }

    repository.create_meeting(&CreateMeeting {
        id: meeting_id.clone(),
        title: None,
        started_at_ms,
        stopped_at_ms: None,
        created_at_ms: started_at_ms,
        updated_at_ms: started_at_ms,
    })?;
    Ok(())
}

fn app_data_dir(app: &AppHandle) -> Result<std::path::PathBuf, AppError> {
    app.path().app_data_dir().map_err(|error| AppError {
        code: "app_data_dir_unavailable".to_string(),
        message: "Could not resolve the application data directory.".to_string(),
        details: Some(error.to_string()),
    })
}

fn migrate_legacy_app_data_to_resonance(app_data_dir: &Path) -> Result<Option<PathBuf>, AppError> {
    if app_data_dir.join(RESONANCE_DATABASE_FILE_NAME).is_file() {
        return Ok(None);
    }

    let Some(legacy_dir) = legacy_app_data_dir_candidates(app_data_dir)
        .into_iter()
        .find(|candidate| {
            candidate != app_data_dir && candidate.join(LEGACY_DATABASE_FILE_NAME).is_file()
        })
    else {
        return Ok(None);
    };

    copy_legacy_app_data_dir(&legacy_dir, app_data_dir)?;
    Ok(Some(legacy_dir))
}

fn legacy_app_data_dir_candidates(app_data_dir: &Path) -> Vec<PathBuf> {
    let Some(parent) = app_data_dir.parent() else {
        return Vec::new();
    };
    vec![
        parent.join(LEGACY_APP_IDENTIFIER),
        parent.join(LEGACY_APP_NAME),
    ]
}

fn copy_legacy_app_data_dir(legacy_dir: &Path, app_data_dir: &Path) -> Result<(), AppError> {
    fs::create_dir_all(app_data_dir)
        .map_err(|error| legacy_migration_error(app_data_dir, error))?;
    copy_legacy_app_data_entries(legacy_dir, app_data_dir)
}

fn copy_legacy_app_data_entries(source_dir: &Path, target_dir: &Path) -> Result<(), AppError> {
    for entry in
        fs::read_dir(source_dir).map_err(|error| legacy_migration_error(source_dir, error))?
    {
        let entry = entry.map_err(|error| legacy_migration_error(source_dir, error))?;
        let source_path = entry.path();
        let target_path = target_dir.join(legacy_migration_target_file_name(&entry.file_name()));

        if source_path.is_dir() {
            fs::create_dir_all(&target_path)
                .map_err(|error| legacy_migration_error(&target_path, error))?;
            copy_legacy_app_data_entries(&source_path, &target_path)?;
        } else if source_path.is_file() && !target_path.exists() {
            fs::copy(&source_path, &target_path)
                .map_err(|error| legacy_migration_error(&source_path, error))?;
        }
    }
    Ok(())
}

fn legacy_migration_target_file_name(file_name: &std::ffi::OsStr) -> std::ffi::OsString {
    match file_name.to_str() {
        Some(LEGACY_DATABASE_FILE_NAME) => RESONANCE_DATABASE_FILE_NAME.into(),
        Some("orator.sqlite3-wal") => "resonance.sqlite3-wal".into(),
        Some("orator.sqlite3-shm") => "resonance.sqlite3-shm".into(),
        _ => file_name.to_os_string(),
    }
}

fn legacy_migration_error(path: &Path, error: std::io::Error) -> AppError {
    AppError {
        code: "legacy_app_data_migration_failed".to_string(),
        message: "Could not migrate local data from the previous app name.".to_string(),
        details: Some(format!("path={}, error={error}", path.display())),
    }
}

fn current_time_ms() -> Result<u64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| AppError {
            code: "system_time_error".to_string(),
            message: "System clock is before the Unix epoch.".to_string(),
            details: Some(error.to_string()),
        })
}

fn map_lock_error<T>(error: std::sync::PoisonError<T>) -> AppError {
    AppError {
        code: "app_state_lock_failed".to_string(),
        message: "Could not acquire application state lock.".to_string(),
        details: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU8, Ordering},
    };

    use crate::analysis::TranscriptSpeakerRole;

    use super::*;
    struct MockTranscriber {
        output: TranscriptionOutput,
    }

    struct FailingOnceTranscriber {
        attempts: Cell<u8>,
        output: TranscriptionOutput,
    }

    struct AlwaysFailingTranscriber;

    struct StubEchoCancellation {
        calls: AtomicU8,
        result: Result<PathBuf, AppError>,
    }

    struct StubAnalyzer {
        analysis: CoachingAnalysis,
    }

    struct FailingAnalyzer;

    impl transcription::Transcriber for MockTranscriber {
        fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
            if !audio_path.exists() {
                return Err(AppError {
                    code: "mock_audio_missing".to_string(),
                    message: "Mock transcriber expected an existing audio path.".to_string(),
                    details: Some(format!("path={}", audio_path.display())),
                });
            }
            Ok(self.output.clone())
        }
    }

    impl transcription::Transcriber for FailingOnceTranscriber {
        fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
            if !audio_path.exists() {
                return Err(AppError {
                    code: "mock_audio_missing".to_string(),
                    message: "Mock transcriber expected an existing audio path.".to_string(),
                    details: Some(format!("path={}", audio_path.display())),
                });
            }
            let attempts = self.attempts.get();
            self.attempts.set(attempts + 1);
            if attempts == 0 {
                return Err(AppError {
                    code: "transient_transcriber_error".to_string(),
                    message: "Temporary transcription failure.".to_string(),
                    details: None,
                });
            }
            Ok(self.output.clone())
        }
    }

    impl transcription::Transcriber for AlwaysFailingTranscriber {
        fn transcribe(&self, _audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
            Err(AppError {
                code: "transcriber_failed".to_string(),
                message: "Transcription failed after retry.".to_string(),
                details: Some("exit_status=1".to_string()),
            })
        }
    }

    impl EchoCancellationBackend for StubEchoCancellation {
        fn process(
            &self,
            _microphone_wav_path: &Path,
            _reference_wav_path: &Path,
        ) -> Result<PathBuf, AppError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result.clone()
        }
    }

    impl Analyzer for StubAnalyzer {
        fn analyze(&self, context: &AnalysisContext) -> Result<CoachingAnalysis, AppError> {
            if context.transcript_segments.is_empty() || context.metrics.is_empty() {
                return Err(AppError {
                    code: "stub_missing_context".to_string(),
                    message: "Stub analyzer expected transcript and metrics context.".to_string(),
                    details: None,
                });
            }
            Ok(self.analysis.clone())
        }
    }

    impl Analyzer for FailingAnalyzer {
        fn analyze(&self, _context: &AnalysisContext) -> Result<CoachingAnalysis, AppError> {
            Err(AppError {
                code: "analyzer_failed".to_string(),
                message: "Analyzer failed after receiving context.".to_string(),
                details: None,
            })
        }
    }

    #[test]
    fn transcribe_meeting_with_transcriber_persists_segments() {
        let repository = test_repository("transcribe-meeting");
        let meeting_id = MeetingId::new("meeting-transcribe");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(3_000),
                created_at_ms: 1_000,
                updated_at_ms: 3_000,
            })
            .expect("meeting can be created");
        let audio_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/lib-transcription/meeting-transcribe.wav");
        std::fs::create_dir_all(audio_path.parent().expect("audio path has parent"))
            .expect("test audio directory can be created");
        std::fs::write(&audio_path, b"RIFF test wav").expect("test audio can be written");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: audio_path.to_string_lossy().to_string(),
                system_audio_file_path: None,
                duration_ms: Some(2_000),
                sample_rate_hz: Some(16_000),
                byte_size: Some(13),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 3_000,
            })
            .expect("audio metadata can be created");
        let transcriber = MockTranscriber {
            output: TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Clear next step.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_500,
                }],
            },
        };

        let result = transcribe_meeting_with_transcriber(
            &repository,
            meeting_id.as_str(),
            &transcriber,
            4_000,
        )
        .expect("meeting can be transcribed");

        assert_eq!(result.segment_count, 1);
        assert_eq!(result.segments[0].text, "Clear next step.");
        let persisted = repository
            .list_transcript_segments(&meeting_id)
            .expect("transcript segments can be listed");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].text, "Clear next step.");
        assert_eq!(persisted[0].created_at_ms, 4_000);
    }

    #[test]
    fn transcribe_meeting_retries_once_before_persisting_segments() {
        let repository = test_repository("transcribe-meeting-retry");
        let meeting_id = MeetingId::new("meeting-transcribe-retry");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(3_000),
                created_at_ms: 1_000,
                updated_at_ms: 3_000,
            })
            .expect("meeting can be created");
        let audio_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/lib-transcription/meeting-transcribe-retry.wav");
        std::fs::create_dir_all(audio_path.parent().expect("audio path has parent"))
            .expect("test audio directory can be created");
        std::fs::write(&audio_path, b"RIFF test wav").expect("test audio can be written");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: audio_path.to_string_lossy().to_string(),
                system_audio_file_path: None,
                duration_ms: Some(2_000),
                sample_rate_hz: Some(16_000),
                byte_size: Some(13),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 3_000,
            })
            .expect("audio metadata can be created");
        let transcriber = FailingOnceTranscriber {
            attempts: Cell::new(0),
            output: TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Retry recovered transcript.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_500,
                }],
            },
        };

        let result = transcribe_meeting_with_transcriber(
            &repository,
            meeting_id.as_str(),
            &transcriber,
            4_000,
        )
        .expect("second transcription attempt succeeds");

        assert_eq!(result.segment_count, 1);
        assert_eq!(transcriber.attempts.get(), 2);
        assert!(repository
            .get_pipeline_failure(&meeting_id)
            .expect("failure state can be read")
            .is_none());
    }

    #[test]
    fn transcribe_meeting_persists_failure_after_retry_exhaustion() {
        let repository = test_repository("transcribe-meeting-failure");
        let meeting_id = MeetingId::new("meeting-transcribe-failure");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(3_000),
                created_at_ms: 1_000,
                updated_at_ms: 3_000,
            })
            .expect("meeting can be created");
        let audio_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/lib-transcription/meeting-transcribe-failure.wav");
        std::fs::create_dir_all(audio_path.parent().expect("audio path has parent"))
            .expect("test audio directory can be created");
        std::fs::write(&audio_path, b"RIFF test wav").expect("test audio can be written");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: audio_path.to_string_lossy().to_string(),
                system_audio_file_path: None,
                duration_ms: Some(2_000),
                sample_rate_hz: Some(16_000),
                byte_size: Some(13),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 3_000,
            })
            .expect("audio metadata can be created");

        let error = transcribe_meeting_with_transcriber(
            &repository,
            meeting_id.as_str(),
            &AlwaysFailingTranscriber,
            4_000,
        )
        .expect_err("retry exhaustion is returned");

        assert_eq!(error.code, "transcriber_failed");
        let failure = repository
            .get_pipeline_failure(&meeting_id)
            .expect("failure state can be read")
            .expect("failure state is persisted");
        assert_eq!(failure.failed_stage, ProcessingStage::Transcribing);
        assert_eq!(failure.error_code, "transcriber_failed");
        assert_eq!(
            failure.error_details.as_deref(),
            Some("first_attempt_code=transcriber_failed, second_attempt_code=transcriber_failed")
        );
        assert!(!failure
            .error_details
            .as_deref()
            .unwrap_or_default()
            .contains("exit_status"));
        assert!(repository
            .get_audio_metadata(&meeting_id)
            .expect("audio metadata can be read")
            .is_some());
    }

    #[test]
    fn select_transcription_audio_path_uses_aec_output_when_enabled() {
        let settings = ResonanceSettings::default();
        let metadata = audio_metadata_with_system_reference("meeting-aec-enabled");
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/resonance/meeting-aec-enabled.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, "/tmp/resonance/meeting-aec-enabled.aec.wav");
    }

    #[test]
    fn select_transcription_audio_path_falls_back_to_raw_mic_when_aec_fails() {
        let settings = ResonanceSettings::default();
        let metadata = audio_metadata_with_system_reference("meeting-aec-fallback");
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Err(AppError {
                code: "aec_failed".to_string(),
                message: "AEC failed.".to_string(),
                details: None,
            }),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, metadata.file_path);
    }

    #[test]
    fn select_transcription_audio_path_uses_raw_mic_when_aec_disabled() {
        let settings = ResonanceSettings {
            enable_echo_cancellation: false,
            ..ResonanceSettings::default()
        };
        let metadata = audio_metadata_with_system_reference("meeting-aec-disabled");
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/resonance/meeting-aec-disabled.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, metadata.file_path);
    }

    #[test]
    fn select_transcription_audio_path_attempts_aec_for_m4a_system_audio() {
        let settings = ResonanceSettings::default();
        let metadata = AudioMetadata {
            system_audio_file_path: Some("/tmp/resonance/meeting-aec-m4a.system.m4a".to_string()),
            ..audio_metadata_with_system_reference("meeting-aec-m4a")
        };
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/resonance/meeting-aec-m4a.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, "/tmp/resonance/meeting-aec-m4a.aec.wav");
        assert_eq!(echo_cancellation.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn analysis_provider_requires_explicit_available_local_provider() {
        assert!(ensure_analysis_provider_available(&ResonanceSettings::default()).is_ok());

        let disabled_cloud_settings = ResonanceSettings {
            analyzer_provider: AnalyzerProvider::CloudClaude,
            cloud_analysis_enabled: false,
            ..ResonanceSettings::default()
        };
        let disabled_error = ensure_analysis_provider_available(&disabled_cloud_settings)
            .expect_err("cloud provider requires explicit opt-in");
        assert_eq!(disabled_error.code, "cloud_analysis_disabled");

        let enabled_cloud_settings = ResonanceSettings {
            analyzer_provider: AnalyzerProvider::CloudOpenAi,
            cloud_analysis_enabled: true,
            ..ResonanceSettings::default()
        };
        let unavailable_error = ensure_analysis_provider_available(&enabled_cloud_settings)
            .expect_err("cloud provider is not implemented yet");
        assert_eq!(unavailable_error.code, "cloud_analyzer_unavailable");
    }

    #[test]
    fn apple_script_string_literal_escapes_notification_text() {
        assert_eq!(
            apple_script_string_literal("Score \"81\"\\100\nready"),
            "\"Score \\\"81\\\"\\\\100 ready\""
        );
    }

    #[test]
    fn audio_retention_policy_deletes_only_expired_app_audio() {
        let repository = test_repository("audio-retention-policy");
        let app_data_dir = test_data_dir("audio-retention-policy");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let expired_audio_path = app_data_dir.join("expired.wav");
        let expired_system_path = app_data_dir.join("expired.system.m4a");
        let fresh_audio_path = app_data_dir.join("fresh.wav");
        std::fs::write(&expired_audio_path, b"old mic").expect("expired mic file can be written");
        std::fs::write(&expired_system_path, b"old system")
            .expect("expired system file can be written");
        std::fs::write(&fresh_audio_path, b"fresh mic").expect("fresh mic file can be written");

        let expired_meeting_id = MeetingId::new("retention-expired");
        let fresh_meeting_id = MeetingId::new("retention-fresh");
        seed_meeting(&repository, &expired_meeting_id);
        seed_meeting(&repository, &fresh_meeting_id);
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: expired_meeting_id.clone(),
                file_path: expired_audio_path.to_string_lossy().into_owned(),
                system_audio_file_path: Some(expired_system_path.to_string_lossy().into_owned()),
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(32),
                system_audio_byte_size: Some(16),
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("expired metadata can be persisted");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: fresh_meeting_id.clone(),
                file_path: fresh_audio_path.to_string_lossy().into_owned(),
                system_audio_file_path: None,
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(32),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: MILLIS_PER_DAY * 3,
            })
            .expect("fresh metadata can be persisted");

        let summary =
            apply_audio_retention_policy(&repository, &app_data_dir, 1, MILLIS_PER_DAY * 2)
                .expect("retention policy can run");

        assert_eq!(summary.deleted_audio_file_count, 2);
        assert_eq!(summary.removed_audio_metadata_count, 1);
        assert_eq!(summary.skipped_audio_file_count, 0);
        assert!(!expired_audio_path.exists());
        assert!(!expired_system_path.exists());
        assert!(fresh_audio_path.exists());
        assert!(repository
            .get_audio_metadata(&expired_meeting_id)
            .expect("expired metadata lookup succeeds")
            .is_none());
        assert!(repository
            .get_audio_metadata(&fresh_meeting_id)
            .expect("fresh metadata lookup succeeds")
            .is_some());
    }

    #[test]
    fn practice_duration_rejects_videos_over_fifteen_minutes() {
        let error = ensure_practice_duration_allowed(MAX_PRACTICE_REVIEW_DURATION_MS + 1)
            .expect_err("practice duration cap is enforced");

        assert_eq!(error.code, "practice_video_too_long");
    }

    #[test]
    fn practice_audio_review_generates_scores_and_timeline_annotations() {
        let practice_id = PracticeRecordingId::new("practice-audio-review");
        let review = practice_audio_review_from_transcript(
            &practice_id,
            TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 0,
                    speaker_label: Some("User".to_string()),
                    text: "um I think we can probably ship this now".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 2_000,
                }],
            },
            3_000,
        )
        .expect("practice audio review can be generated");

        assert!(review.audio_score.is_some());
        assert!(review
            .annotations
            .iter()
            .any(|annotation| annotation.category == "fillerWords"));
        assert!(review
            .annotations
            .iter()
            .any(|annotation| annotation.category == "clarity"));
        assert!(review.body.privacy_note.contains("No video was sent"));
    }

    #[test]
    fn retention_policy_deletes_expired_practice_artifacts_under_app_data() {
        let repository = test_repository("practice-retention-policy");
        let app_data_dir = test_data_dir("practice-retention-policy");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let expired_video_path = app_data_dir.join("practice-old.mp4");
        let expired_audio_path = app_data_dir.join("practice-old.audio.wav");
        let fresh_video_path = app_data_dir.join("practice-fresh.mp4");
        std::fs::write(&expired_video_path, b"old video").expect("expired video can be written");
        std::fs::write(&expired_audio_path, b"old audio").expect("expired audio can be written");
        std::fs::write(&fresh_video_path, b"fresh video").expect("fresh video can be written");

        repository
            .create_practice_recording(&CreatePracticeRecording {
                id: PracticeRecordingId::new("practice-retention-old"),
                title: None,
                source_kind: "imported".to_string(),
                video_file_path: expired_video_path.to_string_lossy().into_owned(),
                extracted_audio_file_path: Some(expired_audio_path.to_string_lossy().into_owned()),
                duration_ms: Some(30_000),
                byte_size: Some(128),
                recorded_at_ms: 1_000,
                created_at_ms: 1_000,
                updated_at_ms: 1_000,
                analysis_status: "complete".to_string(),
                cloud_video_used: false,
                pipeline_failure_code: None,
                pipeline_failure_message: None,
            })
            .expect("expired practice recording can be stored");
        repository
            .create_practice_recording(&CreatePracticeRecording {
                id: PracticeRecordingId::new("practice-retention-fresh"),
                title: None,
                source_kind: "imported".to_string(),
                video_file_path: fresh_video_path.to_string_lossy().into_owned(),
                extracted_audio_file_path: None,
                duration_ms: Some(30_000),
                byte_size: Some(128),
                recorded_at_ms: MILLIS_PER_DAY * 3,
                created_at_ms: MILLIS_PER_DAY * 3,
                updated_at_ms: MILLIS_PER_DAY * 3,
                analysis_status: "recorded".to_string(),
                cloud_video_used: false,
                pipeline_failure_code: None,
                pipeline_failure_message: None,
            })
            .expect("fresh practice recording can be stored");

        let summary =
            apply_audio_retention_policy(&repository, &app_data_dir, 1, MILLIS_PER_DAY * 2)
                .expect("retention policy can run");

        assert_eq!(summary.deleted_practice_file_count, 2);
        assert_eq!(summary.skipped_practice_file_count, 0);
        assert!(!expired_video_path.exists());
        assert!(!expired_audio_path.exists());
        assert!(fresh_video_path.exists());
        assert_eq!(
            repository
                .list_practice_recordings(10, 0)
                .expect("practice rows remain")
                .len(),
            2
        );
    }

    #[test]
    fn transcribe_meeting_with_transcriber_rejects_existing_transcript() {
        let repository = test_repository("transcribe-meeting-existing");
        let meeting_id = MeetingId::new("meeting-transcribe-existing");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(3_000),
                created_at_ms: 1_000,
                updated_at_ms: 3_000,
            })
            .expect("meeting can be created");
        let audio_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/lib-transcription/meeting-transcribe-existing.wav");
        std::fs::create_dir_all(audio_path.parent().expect("audio path has parent"))
            .expect("test audio directory can be created");
        std::fs::write(&audio_path, b"RIFF test wav").expect("test audio can be written");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: audio_path.to_string_lossy().to_string(),
                system_audio_file_path: None,
                duration_ms: Some(2_000),
                sample_rate_hz: Some(16_000),
                byte_size: Some(13),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 3_000,
            })
            .expect("audio metadata can be created");
        repository
            .create_transcript_segments(&[CreateTranscriptSegment {
                id: domain::SegmentId::new("meeting-transcribe-existing-segment-1"),
                meeting_id: meeting_id.clone(),
                sequence_number: 1,
                speaker_label: None,
                text: "Existing transcript.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 1_000,
                created_at_ms: 4_000,
            }])
            .expect("existing transcript can be created");
        let transcriber = MockTranscriber {
            output: TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "New transcript.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                }],
            },
        };

        let error = transcribe_meeting_with_transcriber(
            &repository,
            meeting_id.as_str(),
            &transcriber,
            5_000,
        )
        .expect_err("existing transcript is rejected before re-transcription");

        assert_eq!(error.code, "transcript_already_exists");
    }

    #[test]
    fn calculate_metrics_for_meeting_persists_deterministic_metrics() {
        let repository = test_repository("calculate-metrics");
        let meeting_id = MeetingId::new("meeting-calculate-metrics");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(61_000),
                created_at_ms: 1_000,
                updated_at_ms: 61_000,
            })
            .expect("meeting can be created");
        repository
            .create_transcript_segments(&[
                CreateTranscriptSegment {
                    id: domain::SegmentId::new("meeting-calculate-metrics-segment-1"),
                    meeting_id: meeting_id.clone(),
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Um, I think we should, kind of, start now.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 30_000,
                    created_at_ms: 62_000,
                },
                CreateTranscriptSegment {
                    id: domain::SegmentId::new("meeting-calculate-metrics-segment-2"),
                    meeting_id: meeting_id.clone(),
                    sequence_number: 2,
                    speaker_label: None,
                    text: "Like, maybe we move fast. Probably, uh, yes.".to_string(),
                    started_at_ms: 30_000,
                    ended_at_ms: 60_000,
                    created_at_ms: 62_000,
                },
            ])
            .expect("transcript can be created");

        let result = calculate_metrics_for_meeting(&repository, &meeting_id, 63_000)
            .expect("metrics can be calculated");

        assert_eq!(result.summary.word_count, 17);
        assert_eq!(result.summary.filler_word_count, 3);
        assert_eq!(result.summary.hedging_phrase_count, 4);
        assert_eq!(result.metrics.len(), 8);
        let persisted = repository
            .list_metrics(&meeting_id)
            .expect("metrics can be listed");
        assert_eq!(persisted.len(), 8);
        assert!(persisted.iter().any(|metric| {
            metric.id == domain::MetricId::new("meeting-calculate-metrics-metric-word_count")
                && metric.name == "word_count"
                && (metric.value - 17.0).abs() < f64::EPSILON
                && metric.unit == Some("count".to_string())
        }));
    }

    #[test]
    fn calculate_metrics_for_meeting_rejects_duplicate_metrics() {
        let repository = test_repository("calculate-metrics-duplicate");
        let meeting_id = MeetingId::new("meeting-calculate-metrics-duplicate");
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("meeting can be created");
        repository
            .create_transcript_segments(&[CreateTranscriptSegment {
                id: domain::SegmentId::new("meeting-calculate-metrics-duplicate-segment-1"),
                meeting_id: meeting_id.clone(),
                sequence_number: 1,
                speaker_label: None,
                text: "Maybe yes.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 1_000,
                created_at_ms: 3_000,
            }])
            .expect("transcript can be created");

        calculate_metrics_for_meeting(&repository, &meeting_id, 4_000)
            .expect("first metrics calculation succeeds");
        let error = calculate_metrics_for_meeting(&repository, &meeting_id, 5_000)
            .expect_err("second metrics calculation is rejected");

        assert_eq!(error.code, "metrics_already_exist");
    }

    #[test]
    fn analyze_meeting_with_analyzer_persists_report_when_transcript_and_metrics_exist() {
        let repository = test_repository("analyze-meeting");
        let meeting_id = MeetingId::new("meeting-analyze");
        seed_meeting(&repository, &meeting_id);
        seed_transcript(&repository, &meeting_id);
        seed_metric(&repository, &meeting_id);
        let analyzer = StubAnalyzer {
            analysis: coaching_analysis(88),
        };

        let result = analyze_meeting_with_analyzer(&repository, &meeting_id, &analyzer, 9_000)
            .expect("analysis report can be persisted");

        assert_eq!(result.report_id, ReportId::new("meeting-analyze-report-v1"));
        assert_eq!(result.analysis.overall_score.value(), 88);
        assert_eq!(
            result
                .scorecard
                .analysis
                .score
                .expect("analyzer score is available")
                .value(),
            88
        );
        assert!(result.scorecard.filler.score.is_some());
        assert!(result.scorecard.overall.score.is_some());
        let reports = repository
            .list_reports_for_meeting(&meeting_id)
            .expect("reports can be listed");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, ReportId::new("meeting-analyze-report-v1"));
        assert_eq!(reports[0].overall_score.value(), 88);
        assert!(reports[0].body_json.contains("\"overallScore\":88"));
    }

    #[test]
    fn prepare_analysis_context_marks_non_user_speakers_as_context() {
        let repository = test_repository("analysis-context-speaker-roles");
        let meeting_id = MeetingId::new("meeting-analysis-context");
        seed_meeting(&repository, &meeting_id);
        repository
            .create_transcript_segments(&[
                CreateTranscriptSegment {
                    id: domain::SegmentId::new("meeting-analysis-context-segment-1"),
                    meeting_id: meeting_id.clone(),
                    sequence_number: 1,
                    speaker_label: Some("Speaker 2".to_string()),
                    text: "Can we commit to the smaller launch today?".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                    created_at_ms: 3_000,
                },
                CreateTranscriptSegment {
                    id: domain::SegmentId::new("meeting-analysis-context-segment-2"),
                    meeting_id: meeting_id.clone(),
                    sequence_number: 2,
                    speaker_label: Some("User".to_string()),
                    text: "I think we can maybe do that.".to_string(),
                    started_at_ms: 1_100,
                    ended_at_ms: 2_000,
                    created_at_ms: 3_000,
                },
            ])
            .expect("contextual transcript can be created");
        seed_metric(&repository, &meeting_id);

        let context = prepare_analysis_context(&repository, &meeting_id)
            .expect("analysis context can be prepared");

        assert_eq!(context.transcript_segments.len(), 2);
        assert_eq!(
            context.transcript_segments[0].speaker_role,
            analysis::TranscriptSpeakerRole::Context
        );
        assert_eq!(
            context.transcript_segments[1].speaker_role,
            analysis::TranscriptSpeakerRole::User
        );
    }

    #[test]
    fn analyze_meeting_with_analyzer_rejects_duplicate_report_before_db_unique_error() {
        let repository = test_repository("analyze-meeting-duplicate");
        let meeting_id = MeetingId::new("meeting-analyze-duplicate");
        seed_meeting(&repository, &meeting_id);
        seed_transcript(&repository, &meeting_id);
        seed_metric(&repository, &meeting_id);
        repository
            .create_report(&CreateReport {
                id: ReportId::new("custom-existing-report"),
                meeting_id: meeting_id.clone(),
                overall_score: domain::Score::new(72).expect("score is valid"),
                body_json: "{}".to_string(),
                generated_at_ms: 8_000,
            })
            .expect("existing report can be created");
        let analyzer = StubAnalyzer {
            analysis: coaching_analysis(88),
        };

        let error = analyze_meeting_with_analyzer(&repository, &meeting_id, &analyzer, 9_000)
            .expect_err("duplicate report is rejected explicitly");

        assert_eq!(error.code, "report_already_exists");
    }

    #[test]
    fn analyze_meeting_with_analyzer_persists_failure_without_deleting_audio() {
        let repository = test_repository("analyze-meeting-failure");
        let meeting_id = MeetingId::new("meeting-analyze-failure");
        seed_meeting(&repository, &meeting_id);
        seed_transcript(&repository, &meeting_id);
        seed_metric(&repository, &meeting_id);
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: "/tmp/meeting-analyze-failure.wav".to_string(),
                system_audio_file_path: None,
                duration_ms: Some(60_000),
                sample_rate_hz: Some(16_000),
                byte_size: Some(1024),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 4_000,
            })
            .expect("audio metadata can be stored");

        let error =
            analyze_meeting_with_analyzer(&repository, &meeting_id, &FailingAnalyzer, 9_000)
                .expect_err("analyzer failure is returned");

        assert_eq!(error.code, "analyzer_failed");
        let failure = repository
            .get_pipeline_failure(&meeting_id)
            .expect("failure state can be read")
            .expect("failure state is persisted");
        assert_eq!(failure.failed_stage, ProcessingStage::Analyzing);
        assert_eq!(failure.error_code, "analyzer_failed");
        assert!(repository
            .get_audio_metadata(&meeting_id)
            .expect("audio metadata can be read")
            .is_some());
    }

    #[test]
    fn analyze_meeting_with_analyzer_rejects_missing_transcript() {
        let repository = test_repository("analyze-meeting-no-transcript");
        let meeting_id = MeetingId::new("meeting-analyze-no-transcript");
        seed_meeting(&repository, &meeting_id);
        seed_metric(&repository, &meeting_id);
        let analyzer = StubAnalyzer {
            analysis: coaching_analysis(88),
        };

        let error = analyze_meeting_with_analyzer(&repository, &meeting_id, &analyzer, 9_000)
            .expect_err("missing transcript is rejected");

        assert_eq!(error.code, "transcript_not_found");
    }

    #[test]
    fn analyze_meeting_with_analyzer_rejects_missing_metrics() {
        let repository = test_repository("analyze-meeting-no-metrics");
        let meeting_id = MeetingId::new("meeting-analyze-no-metrics");
        seed_meeting(&repository, &meeting_id);
        seed_transcript(&repository, &meeting_id);
        let analyzer = StubAnalyzer {
            analysis: coaching_analysis(88),
        };

        let error = analyze_meeting_with_analyzer(&repository, &meeting_id, &analyzer, 9_000)
            .expect_err("missing metrics are rejected");

        assert_eq!(error.code, "metrics_not_found");
    }

    #[test]
    fn enroll_voice_profile_copies_mic_sample_and_persists_local_status() {
        let repository = test_repository("voice-profile-enrollment");
        let app_data_dir = test_data_dir("voice-profile-enrollment");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let meeting_id = MeetingId::new("voice-enrollment-meeting");
        seed_meeting(&repository, &meeting_id);
        let source_audio_path = app_data_dir.join("source-voice.wav");
        std::fs::write(&source_audio_path, b"local voice sample")
            .expect("source audio sample can be written");
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: source_audio_path.to_string_lossy().into_owned(),
                system_audio_file_path: None,
                duration_ms: Some(15_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(18),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("audio metadata can be stored");

        let status = enroll_voice_profile_from_meeting_record(
            &repository,
            &app_data_dir,
            &meeting_id,
            9_000,
        )
        .expect("voice profile can be enrolled from a mic recording");

        assert!(status.is_enrolled);
        assert_eq!(status.enrolled_at_ms, Some(9_000));
        assert_eq!(status.sample_duration_ms, Some(15_000));
        assert_eq!(status.sample_byte_size, Some(18));
        assert!(!status.matching_ready);
        let profile = repository
            .get_voice_profile()
            .expect("profile can be read")
            .expect("profile exists");
        assert_eq!(
            std::fs::read(profile.sample_audio_file_path).expect("copied sample can be read"),
            b"local voice sample"
        );
    }

    #[test]
    fn delete_voice_profile_sample_refuses_paths_outside_app_data() {
        let app_data_dir = test_data_dir("voice-profile-delete-root");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let outside_dir = test_data_dir("voice-profile-delete-outside");
        std::fs::create_dir_all(&outside_dir).expect("outside dir can be created");
        let outside_sample = outside_dir.join("sample.wav");
        std::fs::write(&outside_sample, b"do not delete").expect("outside sample can be written");

        let error = delete_voice_profile_sample(&app_data_dir, &outside_sample.to_string_lossy())
            .expect_err("outside sample deletion is rejected");

        assert_eq!(error.code, "voice_profile_delete_rejected");
        assert!(outside_sample.exists());
    }

    #[test]
    fn enroll_voice_profile_rejects_audio_metadata_outside_app_data() {
        let repository = test_repository("voice-profile-enrollment-outside");
        let app_data_dir = test_data_dir("voice-profile-enrollment-root");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let outside_dir = test_data_dir("voice-profile-enrollment-outside");
        std::fs::create_dir_all(&outside_dir).expect("outside dir can be created");
        let outside_sample = outside_dir.join("source-voice.wav");
        std::fs::write(&outside_sample, b"outside sample").expect("outside sample can be written");
        let meeting_id = MeetingId::new("voice-enrollment-outside-meeting");
        seed_meeting(&repository, &meeting_id);
        repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting_id.clone(),
                file_path: outside_sample.to_string_lossy().into_owned(),
                system_audio_file_path: None,
                duration_ms: Some(15_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(14),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("audio metadata can be stored");

        let error = enroll_voice_profile_from_meeting_record(
            &repository,
            &app_data_dir,
            &meeting_id,
            9_000,
        )
        .expect_err("outside enrollment source is rejected");

        assert_eq!(error.code, "voice_enrollment_source_rejected");
    }

    #[test]
    fn prepare_voice_profile_for_matching_requires_enrolled_profile() {
        let repository = test_repository("voice-profile-prepare-no-profile");
        let app_data_dir = test_data_dir("voice-profile-prepare-no-profile");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");

        let error = prepare_voice_profile_for_matching_record(&repository, &app_data_dir, 10_000)
            .expect_err("preparing matching requires enrollment");

        assert_eq!(error.code, "voice_profile_not_enrolled");
    }

    #[test]
    fn prepare_voice_profile_for_matching_requires_model_path() {
        let repository = test_repository("voice-profile-prepare-no-model");
        let app_data_dir = test_data_dir("voice-profile-prepare-no-model");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let sample_path = app_data_dir
            .join("voice-profile")
            .join("enrollment-sample.wav");
        std::fs::create_dir_all(sample_path.parent().expect("sample has parent"))
            .expect("voice profile dir can be created");
        std::fs::write(&sample_path, b"local voice sample").expect("profile sample can be written");
        repository
            .upsert_voice_profile(&VoiceProfileRecord {
                sample_audio_file_path: sample_path.to_string_lossy().into_owned(),
                sample_duration_ms: Some(15_000),
                sample_byte_size: 18,
                enrolled_at_ms: 9_000,
                embedding_json: None,
                embedding_dimension: None,
                embedding_model_path: None,
                embedding_computed_at_ms: None,
            })
            .expect("voice profile can be stored");

        let error = prepare_voice_profile_for_matching_record(&repository, &app_data_dir, 10_000)
            .expect_err("preparing matching requires a model path");

        assert_eq!(error.code, "speaker_embedding_model_not_configured");
    }

    #[test]
    fn match_voice_profile_from_meeting_requires_prepared_profile() {
        let repository = test_repository("voice-profile-match-not-ready");
        let app_data_dir = test_data_dir("voice-profile-match-not-ready");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let sample_path = app_data_dir
            .join("voice-profile")
            .join("enrollment-sample.wav");
        std::fs::create_dir_all(sample_path.parent().expect("sample has parent"))
            .expect("voice profile dir can be created");
        std::fs::write(&sample_path, b"local voice sample").expect("profile sample can be written");
        repository
            .upsert_settings(
                &ResonanceSettings {
                    speaker_embedding_model_path: Some("/models/speaker.onnx".to_string()),
                    ..ResonanceSettings::default()
                },
                9_000,
            )
            .expect("settings can be saved");
        repository
            .upsert_voice_profile(&VoiceProfileRecord {
                sample_audio_file_path: sample_path.to_string_lossy().into_owned(),
                sample_duration_ms: Some(15_000),
                sample_byte_size: 18,
                enrolled_at_ms: 9_000,
                embedding_json: None,
                embedding_dimension: None,
                embedding_model_path: None,
                embedding_computed_at_ms: None,
            })
            .expect("voice profile can be stored");
        let meeting_id = MeetingId::new("voice-match-meeting");
        seed_meeting(&repository, &meeting_id);

        let error = match_voice_profile_from_meeting_record(
            &repository,
            &app_data_dir,
            &meeting_id,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        )
        .expect_err("matching requires a prepared profile embedding");

        assert_eq!(error.code, "voice_profile_not_ready_for_matching");
    }

    #[test]
    fn diarize_imported_audio_requires_segmentation_model_path() {
        let repository = test_repository("diarization-preview-no-segmentation-model");
        let app_data_dir = test_data_dir("diarization-preview-no-segmentation-model");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let audio_path = app_data_dir.join("imported.wav");
        std::fs::write(&audio_path, b"placeholder").expect("audio placeholder can be written");

        let error = diarize_imported_audio_recording(&repository, &app_data_dir, &audio_path)
            .expect_err("diarization requires a segmentation model");

        assert_eq!(error.code, "speaker_segmentation_model_not_configured");
    }

    #[test]
    fn match_imported_audio_speaker_segments_requires_prepared_profile() {
        let repository = test_repository("diarization-match-no-profile");
        let app_data_dir = test_data_dir("diarization-match-no-profile");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let audio_path = app_data_dir.join("imported.wav");
        std::fs::write(&audio_path, b"placeholder").expect("audio placeholder can be written");

        let error = match_imported_audio_speaker_segments(
            &repository,
            &app_data_dir,
            &audio_path,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        )
        .expect_err("matching diarized speakers requires a prepared profile");

        assert_eq!(error.code, "voice_profile_not_enrolled");
    }

    #[test]
    fn voice_match_windows_label_overlapped_transcript_segments_as_user() {
        let segments = vec![
            transcript_segment(0, None, "Before the matching window", 0, 900),
            transcript_segment(
                1,
                None,
                "This overlaps the matched voice window",
                1_000,
                2_000,
            ),
            transcript_segment(2, Some("Speaker 2"), "This stays context", 3_000, 4_000),
        ];
        let windows = vec![VoiceMatchWindow {
            started_at_ms: 1_500,
            ended_at_ms: 2_500,
            similarity_score: 0.86,
            threshold: DEFAULT_SPEAKER_MATCH_THRESHOLD,
        }];

        let analysis_segments = transcript_segments_to_analysis_segments_with_voice_matches(
            &segments,
            &windows,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        );

        assert_eq!(
            analysis_segments
                .iter()
                .map(|segment| (segment.speaker_label.clone(), segment.speaker_role))
                .collect::<Vec<_>>(),
            vec![
                (None, TranscriptSpeakerRole::Context),
                (Some("User".to_string()), TranscriptSpeakerRole::User),
                (
                    Some("Speaker 2".to_string()),
                    TranscriptSpeakerRole::Context
                ),
            ]
        );
    }

    #[test]
    fn voice_match_windows_can_be_evaluated_with_lower_thresholds() {
        let segments = vec![transcript_segment(
            0,
            Some("Speaker 1"),
            "This should become user speech",
            1_000,
            2_000,
        )];
        let windows = vec![VoiceMatchWindow {
            started_at_ms: 1_100,
            ended_at_ms: 1_900,
            similarity_score: 0.85,
            threshold: 0.80,
        }];

        let analysis_segments =
            transcript_segments_to_analysis_segments_with_voice_matches(&segments, &windows, 0.70);

        assert_eq!(analysis_segments[0].speaker_label, Some("User".to_string()));
        assert_eq!(
            analysis_segments[0].speaker_role,
            TranscriptSpeakerRole::User
        );
    }

    #[test]
    fn imported_meeting_visual_review_stays_audio_only_without_user_speech_match() {
        let review = imported_meeting_visual_review(
            Path::new("/tmp/team-meeting.mp4"),
            None,
            &MeetingId::new("imported-meeting-visual"),
            &ResonanceSettings {
                cloud_video_review_enabled: true,
                ..ResonanceSettings::default()
            },
            &[],
            true,
        )
        .expect("audio-only visual review can be created");

        assert_eq!(review.status, ImportedMeetingVisualReviewStatus::AudioOnly);
        assert_eq!(review.visual_score, None);
        assert!(review.summary.contains("no locally matched user speech"));
        assert!(review.privacy_note.contains("No meeting video frames"));
    }

    #[test]
    fn imported_meeting_visual_review_marks_user_not_visible_as_audio_only_context() {
        let review = imported_meeting_visual_review_from_video_review(PracticeVideoReview {
            visual_score: None,
            summary: "The sampled frames show slides and participant tiles, but not the matched speaker on camera."
                .to_string(),
            annotations: Vec::new(),
            cloud_video_used: true,
            user_visible: Some(false),
        });

        assert_eq!(
            review.status,
            ImportedMeetingVisualReviewStatus::UserNotVisible
        );
        assert_eq!(review.visual_score, None);
        assert!(review.summary.contains("not the matched speaker"));
        assert!(review.privacy_note.contains("not visibly identifiable"));
    }

    #[test]
    fn imported_summary_uses_voice_matched_windows_for_user_speech() {
        let segments = vec![
            transcript_segment(0, Some("Speaker 1"), "Matched user speech", 1_000, 2_000),
            transcript_segment(1, Some("Speaker 2"), "Other speaker context", 3_000, 4_000),
        ];
        let match_result = VoiceDiarizationMatchResult {
            speaker_count: 2,
            segment_count: 2,
            matched_window_count: 1,
            speaker_matches: Vec::new(),
            matched_windows: vec![voice_matching::DiarizedVoiceMatchWindow {
                started_at_ms: 1_100,
                ended_at_ms: 1_900,
                speaker: 0,
                similarity_score: 0.86,
                threshold: DEFAULT_SPEAKER_MATCH_THRESHOLD,
            }],
        };
        let windows = imported_voice_match_windows(&match_result);

        let analysis_segments = transcript_segments_to_analysis_segments_with_voice_matches(
            &segments,
            &windows,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        );

        assert_eq!(analysis_segments[0].speaker_label, Some("User".to_string()));
        assert_eq!(
            analysis_segments[0].speaker_role,
            TranscriptSpeakerRole::User
        );
        assert_eq!(
            analysis_segments[1].speaker_role,
            TranscriptSpeakerRole::Context
        );
    }

    #[test]
    fn imported_summary_prefers_voice_match_source_over_manual_main_speaker() {
        assert_eq!(
            imported_speaking_improvements_source(true, true),
            ImportedSpeakingImprovementsSource::VoiceMatch
        );
        assert_eq!(
            imported_speaking_improvements_source(true, false),
            ImportedSpeakingImprovementsSource::MainSpeaker
        );
        assert_eq!(
            imported_speaking_improvements_source(false, false),
            ImportedSpeakingImprovementsSource::None
        );
    }

    #[test]
    fn app_data_path_validation_uses_generic_code_for_missing_local_path() {
        let app_data_dir = test_data_dir("path-validation-generic-code-root");
        std::fs::create_dir_all(&app_data_dir).expect("app data dir can be created");
        let missing_path = app_data_dir.join("missing.wav");

        let error = ensure_path_under_app_data(
            &app_data_dir,
            &missing_path,
            "speaker_diarization_audio_rejected",
        )
        .expect_err("missing local path cannot be canonicalized");

        assert_eq!(error.code, "local_path_validation_failed");
    }

    #[test]
    fn voice_match_windows_below_threshold_do_not_label_user_segments() {
        let segments = vec![transcript_segment(
            0,
            None,
            "This overlaps a low-confidence voice window",
            1_000,
            2_000,
        )];
        let windows = vec![VoiceMatchWindow {
            started_at_ms: 1_100,
            ended_at_ms: 1_900,
            similarity_score: 0.60,
            threshold: DEFAULT_SPEAKER_MATCH_THRESHOLD,
        }];

        let analysis_segments = transcript_segments_to_analysis_segments_with_voice_matches(
            &segments,
            &windows,
            DEFAULT_SPEAKER_MATCH_THRESHOLD,
        );

        assert_eq!(
            analysis_segments[0].speaker_role,
            TranscriptSpeakerRole::Context
        );
        assert_eq!(analysis_segments[0].speaker_label, None);
    }

    #[test]
    fn legacy_app_data_migration_copies_database_and_owned_files() {
        let root = test_data_dir("legacy-app-data-migration");
        if root.exists() {
            std::fs::remove_dir_all(&root).expect("old migration test dir can be removed");
        }
        let legacy_dir = root.join(LEGACY_APP_IDENTIFIER);
        let app_data_dir = root.join("com.resonance.meetingcoach");
        std::fs::create_dir_all(legacy_dir.join("voice-profile"))
            .expect("legacy voice profile dir can be created");
        std::fs::write(
            legacy_dir.join(LEGACY_DATABASE_FILE_NAME),
            b"legacy database",
        )
        .expect("legacy database can be written");
        std::fs::write(
            legacy_dir
                .join("voice-profile")
                .join(VOICE_PROFILE_SAMPLE_FILE_NAME),
            b"voice sample",
        )
        .expect("legacy voice sample can be written");

        let migrated_from = migrate_legacy_app_data_to_resonance(&app_data_dir)
            .expect("legacy app data can be migrated")
            .expect("legacy app data is detected");

        assert_eq!(migrated_from, legacy_dir);
        assert_eq!(
            std::fs::read(app_data_dir.join(RESONANCE_DATABASE_FILE_NAME))
                .expect("migrated database can be read"),
            b"legacy database"
        );
        assert_eq!(
            std::fs::read(
                app_data_dir
                    .join("voice-profile")
                    .join(VOICE_PROFILE_SAMPLE_FILE_NAME)
            )
            .expect("migrated voice sample can be read"),
            b"voice sample"
        );
        assert!(migrate_legacy_app_data_to_resonance(&app_data_dir)
            .expect("second migration check succeeds")
            .is_none());
    }

    fn test_repository(name: &str) -> SqliteRepository {
        let directory = test_data_dir("lib-transcription");
        std::fs::create_dir_all(&directory).expect("test database directory can be created");
        let database_path = directory.join(format!("{}-{}.sqlite3", name, std::process::id()));
        for path in [
            database_path.clone(),
            database_path.with_extension("sqlite3-shm"),
            database_path.with_extension("sqlite3-wal"),
        ] {
            if path.exists() {
                std::fs::remove_file(&path).expect("old test database can be removed");
            }
        }
        SqliteRepository::open(database_path).expect("test repository can be opened")
    }

    fn test_data_dir(name: &str) -> PathBuf {
        std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data")
            .join(name)
    }

    fn audio_metadata_with_system_reference(meeting_id: &str) -> AudioMetadata {
        AudioMetadata {
            meeting_id: MeetingId::new(meeting_id),
            file_path: format!("/tmp/resonance/{meeting_id}.wav"),
            system_audio_file_path: Some(format!("/tmp/resonance/{meeting_id}.system.wav")),
            duration_ms: Some(1_000),
            sample_rate_hz: Some(48_000),
            byte_size: Some(96_000),
            system_audio_byte_size: Some(96_000),
            system_audio_stream_error: None,
            created_at_ms: 1_000,
        }
    }

    fn seed_meeting(repository: &SqliteRepository, meeting_id: &MeetingId) {
        repository
            .create_meeting(&CreateMeeting {
                id: meeting_id.clone(),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("meeting can be created");
    }

    fn seed_transcript(repository: &SqliteRepository, meeting_id: &MeetingId) {
        repository
            .create_transcript_segments(&[CreateTranscriptSegment {
                id: domain::SegmentId::new(format!("{}-segment-1", meeting_id.as_str())),
                meeting_id: meeting_id.clone(),
                sequence_number: 1,
                speaker_label: Some("User".to_string()),
                text: "I think we should ship the small slice first.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 1_000,
                created_at_ms: 3_000,
            }])
            .expect("transcript can be created");
    }

    fn transcript_segment(
        sequence_number: u32,
        speaker_label: Option<&str>,
        text: &str,
        started_at_ms: u64,
        ended_at_ms: u64,
    ) -> TranscriptSegment {
        TranscriptSegment {
            sequence_number,
            speaker_label: speaker_label.map(str::to_string),
            text: text.to_string(),
            started_at_ms,
            ended_at_ms,
        }
    }

    fn seed_metric(repository: &SqliteRepository, meeting_id: &MeetingId) {
        for (name, value, unit) in [
            ("word_count", 10.0, Some("count")),
            ("filler_word_rate", 0.0, Some("ratio")),
            ("words_per_minute", 150.0, Some("wpm")),
            ("hedging_phrase_count", 1.0, Some("count")),
            ("duration_ms", 4_000.0, Some("ms")),
            ("user_talk_time_ms", 3_000.0, Some("ms")),
        ] {
            repository
                .create_metric(&CreateMetric {
                    id: domain::MetricId::new(format!("{}-metric-{name}", meeting_id.as_str())),
                    meeting_id: meeting_id.clone(),
                    name: name.to_string(),
                    value,
                    unit: unit.map(str::to_string),
                    created_at_ms: 4_000,
                })
                .expect("metric can be created");
        }
    }

    fn coaching_analysis(overall_score: u8) -> CoachingAnalysis {
        CoachingAnalysis {
            overall_score: domain::Score::new(overall_score).expect("score is valid"),
            observations: vec![analysis::CoachingObservation {
                category: "clarity".to_string(),
                score: domain::Score::new(76).expect("score is valid"),
                quote: "I think we should ship the small slice first.".to_string(),
                speaker_label: Some("User".to_string()),
                context_quote: None,
                context_speaker_label: None,
                suggestion: "Say: We should ship the small slice first.".to_string(),
            }],
        }
    }
}
