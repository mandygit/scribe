use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use analysis::{AnalysisTranscriptSegment, MeetingSummarizer, MeetingSummary};
use audio::{
    aec::{EchoCancellationBackend, SpeexEchoCancellationBackend},
    storage::{safe_system_audio_path, safe_wav_path, validate_recording_file_stem},
    AudioDevice, CpalCaptureBackend, RecordingManager, RecordingMetadata, RecordingStarted,
    ScreenCaptureKitSystemAudioBackend,
};
use dictation::{DictationHotkey, DictationRecorder, HotkeyAction};
use domain::{
    AnalyzerProvider, AppError, DictationSessionId, MeetingId, MeetingLifecycleState,
    ProcessingStage, ReportId, ScribeSettings, SummarizerProvider, ThemePreference,
};
use meeting_detection::{advance, CallPromptState, DetectorEvent, PromptAction, TeamsCallDetector};
use nudges::{
    LiveNudgeEvent, LiveNudgePipeline, NudgeEventSink, NudgeTranscriptEventSink, LIVE_NUDGE_EVENT,
};
use path_detection::hydrate_settings_with_local_defaults;
use persistence::{
    AudioMetadata, CreateDictationSession, CreateMeeting, CreateMetric, CreatePipelineFailure,
    CreateTranscriptSegment, DictationSessionRecord, DictationStatsSummary, MeetingHistoryRecord,
    MeetingTrendRecord, MetricRecord, PipelineFailureRecord, SqliteRepository,
};
use rules::{MetricsSummary, RuleTranscriptSegment};
use serde::Serialize;
use summarizer::{
    list_models as list_summarizer_models_impl, LmStudioClient, LmStudioLifecycle,
    LmStudioSummarizer, OpenAiCompatibleClient, DEFAULT_SUMMARIZER_MODEL,
};
use tauri::{AppHandle, Emitter, Manager, State};
use transcription::{
    TranscriptEventSink, TranscriptSegment, TranscriptStreamEvent, TranscriptStreamSummary,
    TranscriptionOutput, WhisperShellTranscriber, TRANSCRIPT_SEGMENT_EVENT,
    TRANSCRIPT_STREAM_COMPLETE_EVENT,
};

pub mod analysis;
pub mod audio;
pub mod dictation;
pub mod domain;
pub mod media_import;
pub mod meeting_detection;
pub mod nudges;
pub mod path_detection;
pub mod permissions;
pub mod persistence;
pub mod rules;
pub mod summarizer;
pub mod transcription;

struct AppState {
    repository: Mutex<SqliteRepository>,
    recordings: Mutex<RecordingManager<CpalCaptureBackend, ScreenCaptureKitSystemAudioBackend>>,
    dictation: Mutex<DictationRecorder<CpalCaptureBackend>>,
    dictation_hotkey: Mutex<DictationHotkey>,
    /// Counts consecutive polish-selection presses with nothing selected, so
    /// the notice can start friendly and get terser after a few repeats.
    polish_selection_notice_count: Mutex<u32>,
    /// Owns the Teams-call-detector sidecar process, if currently running
    /// (only when the `promptOnTeamsMeeting` setting is on).
    meeting_detector: Mutex<TeamsCallDetector>,
    /// The "record this meeting?" prompt's state machine (see
    /// `meeting_detection::advance`).
    meeting_call_state: Mutex<CallPromptState>,
}

fn load_effective_settings(repository: &SqliteRepository) -> Result<ScribeSettings, AppError> {
    Ok(hydrate_settings_with_local_defaults(
        repository.get_settings()?,
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    state: String,
    detail: String,
    current_lifecycle: MeetingLifecycleState,
    default_settings: ScribeSettings,
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
    summary: Option<MeetingSummary>,
    summary_generated_at_ms: Option<u64>,
    user_notes: Option<String>,
    audio_file_path: Option<String>,
    system_audio_file_path: Option<String>,
    pipeline_failure: Option<PipelineFailureRecord>,
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
    removed_audio_metadata_count: u32,
    skipped_audio_file_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettingsUpdateResult {
    settings: ScribeSettings,
    cleanup: RetentionCleanupSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSessionPage {
    items: Vec<DictationSessionRecord>,
    next_offset: Option<u32>,
}

const DEFAULT_HISTORY_LIMIT: u32 = 10;
const MAX_HISTORY_LIMIT: u32 = 50;
// 200 segments (~40-50 minutes of normal conversation) was cutting off
// longer meetings silently in the detail view; 5000 covers meetings well
// past a full day of continuous speech while still bounding worst-case
// render cost.
const HISTORY_DETAIL_TRANSCRIPT_LIMIT: u32 = 5000;
const DEFAULT_TRENDS_LIMIT: u32 = 12;
const MAX_TRENDS_LIMIT: u32 = 50;
const MAX_RAW_AUDIO_RETENTION_DAYS: u16 = 365;
const MILLIS_PER_DAY: u64 = 86_400_000;
const SCRIBE_DATABASE_FILE_NAME: &str = "scribe.sqlite3";

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
fn list_dictation_sessions(
    state: State<'_, AppState>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<DictationSessionPage, AppError> {
    let requested_limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    let offset_value = offset.unwrap_or(0);
    let fetch_limit = requested_limit + 1;
    let mut rows = state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .list_dictation_sessions(fetch_limit, offset_value)?;
    let has_more = rows.len() > requested_limit as usize;
    rows.truncate(requested_limit as usize);

    Ok(DictationSessionPage {
        items: rows,
        next_offset: if has_more {
            offset_value.checked_add(requested_limit)
        } else {
            None
        },
    })
}

#[tauri::command]
fn get_dictation_stats_summary(
    state: State<'_, AppState>,
) -> Result<DictationStatsSummary, AppError> {
    state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .get_dictation_stats_summary()
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
    let stored_summary = repository.get_meeting_summary(&meeting_id_value)?;
    let summary = stored_summary
        .as_ref()
        .map(|record| {
            serde_json::from_str::<MeetingSummary>(&record.body_json).map_err(|error| AppError {
                code: "summary_decode_failed".to_string(),
                message: "Stored meeting notes could not be decoded.".to_string(),
                details: Some(error.to_string()),
            })
        })
        .transpose()?;
    let summary_generated_at_ms = stored_summary.as_ref().map(|record| record.generated_at_ms);
    let user_notes = repository
        .get_meeting_notes(&meeting_id_value)?
        .map(|record| record.content);
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
            false,
            pipeline_failure.is_some(),
        ),
        transcript_segment_count,
        latest_report_id: None,
        latest_report_score: None,
        latest_report_generated_at_ms: None,
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
        summary,
        summary_generated_at_ms,
        user_notes,
        audio_file_path,
        system_audio_file_path,
        pipeline_failure,
    })
}

/// Deletes a meeting: its raw audio files on disk, then the DB row (which
/// cascades to transcript segments, metrics, reports, and summaries). Refuses
/// to delete the meeting currently being recorded, since its audio file is
/// still being written.
#[tauri::command]
fn delete_meeting(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<(), AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id.clone());

    {
        let recordings = state.recordings.lock().map_err(map_lock_error)?;
        if recordings.active_meeting_id() == Some(meeting_id.as_str()) {
            return Err(AppError {
                code: "meeting_currently_recording".to_string(),
                message: "Cannot delete a meeting while it is being recorded.".to_string(),
                details: None,
            });
        }
    }

    let app_data_dir = app_data_dir(&app)?;
    let repository = state.repository.lock().map_err(map_lock_error)?;

    if let Some(metadata) = repository.get_audio_metadata(&meeting_id_value)? {
        for file_path in retention_file_paths(&metadata) {
            delete_retained_audio_file(&file_path, &app_data_dir)?;
        }
    }

    repository.delete_meeting(&meeting_id_value)?;
    Ok(())
}

/// Renames a meeting. An empty (or all-whitespace) title clears it, reverting
/// the meeting to its date-based display name in the UI.
#[tauri::command]
fn update_meeting_title(
    state: State<'_, AppState>,
    meeting_id: String,
    title: String,
) -> Result<(), AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let trimmed = title.trim();
    let title_value = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };

    let repository = state.repository.lock().map_err(map_lock_error)?;
    let updated_at_ms = current_time_ms()?;
    repository.update_meeting_title(&meeting_id_value, title_value, updated_at_ms)
}

/// Saves the notes the user typed for a meeting (the "Notes" tab). Empty
/// content clears them.
#[tauri::command]
fn update_meeting_user_notes(
    state: State<'_, AppState>,
    meeting_id: String,
    content: String,
) -> Result<(), AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let updated_at_ms = current_time_ms()?;
    repository.upsert_meeting_notes(&meeting_id_value, &content, updated_at_ms)
}

/// Deletes a single dictation session summary row.
#[tauri::command]
fn delete_dictation_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), AppError> {
    state
        .repository
        .lock()
        .map_err(map_lock_error)?
        .delete_dictation_session(&DictationSessionId::new(session_id))?;
    Ok(())
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

    let started =
        recordings.start_recording(meeting_id, file_path, system_audio_file_path, device_id)?;

    // Broadcast to every window, not just whichever one called this command —
    // the main window and the floating recording indicator both need to know
    // a recording is active regardless of whether it was started from the
    // main window's button or the meeting-detection popup.
    emit_recording_started(&app, &started);
    #[cfg(target_os = "macos")]
    set_recording_indicator_visible(&app, true);

    Ok(started)
}

#[tauri::command]
async fn transcribe_meeting(
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
    let transcriber = WhisperShellTranscriber::from_settings(&settings)?;
    let now_ms = current_time_ms()?;

    // whisper-cli (and the AEC pass ahead of it) can run for as long as the
    // meeting itself. Run both off the async runtime so this command doesn't
    // tie up the invoke thread for the whole duration, the way it used to
    // when "stop meeting" awaited this synchronously.
    let transcription_output = tauri::async_runtime::spawn_blocking(move || {
        transcribe_meeting_tracks(
            &settings,
            &metadata,
            &transcriber,
            &SpeexEchoCancellationBackend,
        )
    })
    .await
    .map_err(|error| AppError {
        code: "transcription_task_failed".to_string(),
        message: "The transcription task did not finish.".to_string(),
        details: Some(error.to_string()),
    })?;

    let result = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        match transcription_output {
            Ok(output) => {
                let persisted = persist_transcription_output(
                    &repository,
                    meeting_id_value.clone(),
                    output,
                    now_ms,
                )?;
                clear_pipeline_failure_after_success(&repository, &meeting_id_value);
                persisted
            }
            Err(error) => {
                persist_pipeline_failure(
                    &repository,
                    &meeting_id_value,
                    ProcessingStage::Transcribing,
                    &error,
                    now_ms,
                )?;
                return Err(error);
            }
        }
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingNotesResult {
    meeting_id: MeetingId,
    summary: MeetingSummary,
    generated_at_ms: u64,
}

#[tauri::command]
async fn summarize_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
    model: Option<String>,
) -> Result<MeetingNotesResult, AppError> {
    validate_recording_file_stem(&meeting_id)?;
    let meeting_id_value = MeetingId::new(meeting_id);

    let segments = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let records = repository.list_transcript_segments(&meeting_id_value)?;
        records
            .into_iter()
            .map(|record| AnalysisTranscriptSegment {
                sequence_number: record.sequence_number,
                speaker_label: record.speaker_label,
                speaker_role: analysis::TranscriptSpeakerRole::User,
                text: record.text,
                started_at_ms: record.started_at_ms,
                ended_at_ms: record.ended_at_ms,
            })
            .collect::<Vec<_>>()
    };

    if segments.is_empty() {
        return Err(AppError {
            code: "transcript_not_found".to_string(),
            message: "Transcribe this meeting before generating notes.".to_string(),
            details: Some(format!("meeting_id={}", meeting_id_value.as_str())),
        });
    }

    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        load_effective_settings(&repository)?
    };

    let model = match model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(explicit) => explicit,
        None => match settings.summarizer_model.clone() {
            Some(configured) => configured,
            None if matches!(settings.summarizer_provider, SummarizerProvider::LmStudio) => {
                DEFAULT_SUMMARIZER_MODEL.to_string()
            }
            None => {
                return Err(AppError {
                    code: "summarizer_model_not_configured".to_string(),
                    message: "Choose a local model in Settings before generating notes."
                        .to_string(),
                    details: None,
                })
            }
        },
    };

    let provider = settings.summarizer_provider;
    let host = settings.summarizer_host.clone();
    let port = settings.summarizer_port;

    let generated_at_ms = current_time_ms()?;
    // Run the blocking model load + summary off the main thread so the webview
    // UI stays responsive (a synchronous command would freeze it for ~1 minute).
    let summary_result = tauri::async_runtime::spawn_blocking(move || {
        run_summary(provider, &host, port, segments, model)
    })
    .await
    .map_err(|error| AppError {
        code: "summary_task_failed".to_string(),
        message: "The summarization task did not finish.".to_string(),
        details: Some(error.to_string()),
    })?;
    let summary = match summary_result {
        Ok(summary) => summary,
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

    let body_json = serde_json::to_string(&summary).map_err(|error| AppError {
        code: "summary_serialization_failed".to_string(),
        message: "Could not serialize the generated meeting notes.".to_string(),
        details: Some(error.to_string()),
    })?;
    {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        repository.upsert_meeting_summary(&meeting_id_value, &body_json, generated_at_ms)?;
        if let Some(title) = summary.meeting_title.as_deref() {
            repository.set_meeting_title_if_absent(&meeting_id_value, title, generated_at_ms)?;
        }
        clear_pipeline_failure_after_success(&repository, &meeting_id_value);
    }

    Ok(MeetingNotesResult {
        meeting_id: meeting_id_value,
        summary,
        generated_at_ms,
    })
}

/// Starts a push-to-talk dictation capture to a temporary WAV. Shared by the
/// `start_dictation` command and the global hotkey.
fn begin_dictation(state: &AppState) -> Result<(), AppError> {
    let wav_path = dictation::new_dictation_wav_path()?;
    let mut recorder = state.dictation.lock().map_err(map_lock_error)?;
    recorder.start(wav_path, None)
}

/// Stops the in-flight capture and resolves the transcriber settings, returning
/// the clip path and settings so the (blocking) transcription can run without
/// holding any locks.
fn stop_dictation_capture(state: &AppState) -> Result<(PathBuf, ScribeSettings, u64), AppError> {
    let (wav_path, started_at_ms) = {
        let mut recorder = state.dictation.lock().map_err(map_lock_error)?;
        recorder.finish()?
    };
    let settings = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        load_effective_settings(&repository)?
    };
    Ok((wav_path, settings, started_at_ms))
}

/// Shortest a dictation must run before its stats are worth persisting. Filters
/// out accidental double-taps rather than real dictations.
const MIN_DICTATION_DURATION_MS: u64 = 500;

/// Computes word count and words-per-minute for a finished dictation and
/// persists a stats-only summary row (never the transcript itself). Best-effort:
/// logs and continues on failure so persistence can never disrupt the inject
/// flow, and skips near-zero-duration captures that are likely stray taps.
fn record_dictation_session(state: &AppState, started_at_ms: u64, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let ended_at_ms = match current_time_ms() {
        Ok(ms) => ms,
        Err(error) => {
            eprintln!("dictation: could not timestamp session ({})", error.code);
            return;
        }
    };
    let duration_ms = ended_at_ms.saturating_sub(started_at_ms);
    if duration_ms < MIN_DICTATION_DURATION_MS {
        return;
    }
    let (word_count, words_per_minute) = dictation::session_stats(text, duration_ms);
    let id = match dictation::new_dictation_session_id() {
        Ok(id) => id,
        Err(error) => {
            eprintln!("dictation: could not id session ({})", error.code);
            return;
        }
    };
    let record = CreateDictationSession {
        id: DictationSessionId::new(id),
        started_at_ms,
        ended_at_ms,
        duration_ms,
        word_count,
        words_per_minute,
        created_at_ms: ended_at_ms,
    };
    let result = state
        .repository
        .lock()
        .map_err(map_lock_error)
        .and_then(|repository| repository.create_dictation_session(&record));
    if let Err(error) = result {
        eprintln!("dictation: could not save session stats ({})", error.code);
    }
}

/// Transcribes a dictation clip and deletes the temporary WAV afterwards. Blocks,
/// so callers run it off the main thread.
fn transcribe_dictation_wav(
    wav_path: &Path,
    settings: &ScribeSettings,
) -> Result<String, AppError> {
    let result = WhisperShellTranscriber::from_settings(settings)
        .and_then(|transcriber| dictation::transcribe_clip(&transcriber, wav_path));
    let _ = fs::remove_file(wav_path);
    result
}

/// Starts a push-to-talk dictation capture. Returns immediately; the matching
/// `stop_dictation` call transcribes the clip.
#[tauri::command]
fn start_dictation(state: State<'_, AppState>) -> Result<(), AppError> {
    begin_dictation(&state)
}

/// Stops the in-flight dictation capture, transcribes the clip off the main
/// thread, deletes the temporary WAV, and returns the raw transcript.
#[tauri::command]
async fn stop_dictation(state: State<'_, AppState>) -> Result<String, AppError> {
    let (wav_path, settings, _started_at_ms) = stop_dictation_capture(&state)?;
    tauri::async_runtime::spawn_blocking(move || transcribe_dictation_wav(&wav_path, &settings))
        .await
        .map_err(|error| AppError {
            code: "dictation_transcription_task_failed".to_string(),
            message: "The dictation transcription task did not finish.".to_string(),
            details: Some(error.to_string()),
        })?
}

/// Audible dictation feedback: a tick when listening starts and a chime when the
/// text is inserted. Paired with the menu-bar indicator in case sound is off.
const DICTATION_START_SOUND: &str = "/System/Library/Sounds/Tink.aiff";
const DICTATION_DONE_SOUND: &str = "/System/Library/Sounds/Glass.aiff";

/// Id of the menu-bar tray icon, used both to build it and to flip its title to a
/// recording indicator during dictation.
const TRAY_ICON_ID: &str = "scribe";

/// Plays a short macOS system sound as fire-and-forget dictation feedback.
fn play_cue(sound_file: &str) {
    let _ = Command::new("afplay").arg(sound_file).spawn();
}

/// Event the dictation pill listens to so it can reflect the current state
/// (`idle` → `listening` → `transcribing` → `idle`).
const DICTATION_STATE_EVENT: &str = "scribe://dictation-state";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DictationStateEvent {
    state: &'static str,
}

/// Broadcasts the current dictation state to the pill window. Fire-and-forget:
/// a failed emit must never disrupt the dictation flow itself.
fn emit_dictation_state(app: &AppHandle, state: &'static str) {
    let _ = app.emit(DICTATION_STATE_EVENT, DictationStateEvent { state });
}

/// Event the pill listens to while dictation records: the live microphone
/// input level (RMS, 0..=1) driving its waveform.
const DICTATION_LEVEL_EVENT: &str = "scribe://dictation-level";

/// How often the live input level is sampled and emitted to the pill.
const DICTATION_LEVEL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DictationLevelEvent {
    level: f32,
}

/// Streams the microphone input level to the pill for the duration of the
/// in-flight dictation. The thread holds only a weak view onto the capture's
/// level meter, so it exits on its own as soon as the capture is stopped and
/// dropped — no stop signal to coordinate, and a stale thread can never read
/// a later dictation's levels.
fn spawn_dictation_level_emitter(app: &AppHandle) {
    let observer = match app.state::<AppState>().dictation.lock() {
        Ok(recorder) => recorder.level_observer(),
        Err(_) => None,
    };
    let Some(observer) = observer else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        while let Some(level) = observer.read() {
            let _ = app.emit(DICTATION_LEVEL_EVENT, DictationLevelEvent { level });
            std::thread::sleep(DICTATION_LEVEL_INTERVAL);
        }
    });
}

/// Event the pill listens to for cursor hover, driving its idle-sliver →
/// capsule bloom. Detected by polling the global cursor position against the
/// pill's known frame (see `spawn_pill_hover_watcher`), NOT by DOM mouse
/// events: WebKit installs its mouse tracking scoped to the key window, and
/// the pill's non-activating panel is essentially never key, so mouseenter
/// never fires inside it.
#[cfg(target_os = "macos")]
const DICTATION_PILL_HOVER_EVENT: &str = "scribe://dictation-pill-hover";

#[cfg(target_os = "macos")]
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DictationPillHoverEvent {
    hovering: bool,
}

/// How often the hover watcher samples the cursor. 10 Hz keeps worst-case
/// hover latency at ~100ms, which reads as instant for a reveal affordance,
/// at a per-tick cost of one CoreGraphics call and a rect compare.
#[cfg(target_os = "macos")]
const PILL_HOVER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Watches the global cursor and tells the pill when it enters or leaves the
/// pill window's frame. Runs for the app's whole lifetime.
#[cfg(target_os = "macos")]
fn spawn_pill_hover_watcher(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut was_hovering = false;
        loop {
            std::thread::sleep(PILL_HOVER_POLL_INTERVAL);
            let Some((cursor_x, cursor_y)) = macos_cursor::location() else {
                continue;
            };
            let hovering = PILL_RECT.lock().ok().and_then(|rect| *rect).is_some_and(
                |(x, y, width, height)| {
                    cursor_x >= x && cursor_x < x + width && cursor_y >= y && cursor_y < y + height
                },
            );
            if hovering != was_hovering {
                was_hovering = hovering;
                let _ = app.emit(
                    DICTATION_PILL_HOVER_EVENT,
                    DictationPillHoverEvent { hovering },
                );
            }
        }
    });
}

/// Minimal CoreGraphics bindings for the global cursor position, which needs
/// no permissions (unlike synthesising events). Same no-#[link] approach as
/// `audio::capture::macos_transport`: CoreGraphics and CoreFoundation are
/// already linked by wry/tauri, so the symbols resolve without re-declaring
/// them on the link line.
#[cfg(target_os = "macos")]
mod macos_cursor {
    use std::ffi::c_void;

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CFRelease(object: *const c_void);
    }

    /// Cursor position in global top-left-origin screen points — the same
    /// space `position_window` computes window frames in. `None` if the
    /// event could not be created (never observed in practice).
    pub fn location() -> Option<(f64, f64)> {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let point = CGEventGetLocation(event);
            CFRelease(event);
            Some((point.x, point.y))
        }
    }
}

/// Event the pill listens to for brief polish-selection feedback: "nothing
/// selected" nudges and the "couldn't paste, saved to clipboard" fallback.
const POLISH_SELECTION_NOTICE_EVENT: &str = "scribe://polish-selection-notice";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolishSelectionNoticeEvent {
    message: String,
}

/// Event the meeting popup listens to: a live Teams call was just detected
/// and it should show itself with the given (not-yet-started) meeting id.
const MEETING_DETECTED_EVENT: &str = "scribe://meeting-detected";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingDetectedEvent {
    meeting_id: String,
}

/// Broadcasts a freshly detected call to the meeting popup. Fire-and-forget,
/// same as `emit_dictation_state`.
fn emit_meeting_detected(app: &AppHandle, meeting_id: &str) {
    let _ = app.emit(
        MEETING_DETECTED_EVENT,
        MeetingDetectedEvent {
            meeting_id: meeting_id.to_string(),
        },
    );
}

/// Event the meeting popup listens to: the call it was showing itself for
/// ended before the user chose Record or Dismiss, so it should hide itself.
const MEETING_CALL_ENDED_EVENT: &str = "scribe://meeting-call-ended";

/// Broadcasts that the current call ended. Fire-and-forget, same as
/// `emit_dictation_state`.
fn emit_meeting_call_ended(app: &AppHandle) {
    let _ = app.emit(MEETING_CALL_ENDED_EVENT, ());
}

/// Event broadcast to every window when a recording starts, regardless of
/// which window's button triggered it (the main window's Start button, or
/// the meeting-detection popup's Record button) — both the main window and
/// the floating recording indicator need to stay in sync.
const RECORDING_STARTED_EVENT: &str = "scribe://recording-started";

/// Broadcasts that a recording started. Fire-and-forget, same as
/// `emit_dictation_state`.
fn emit_recording_started(app: &AppHandle, started: &RecordingStarted) {
    let _ = app.emit(RECORDING_STARTED_EVENT, started);
}

/// Event broadcast to every window when a recording stops, regardless of
/// which window's stop button triggered it.
const RECORDING_STOPPED_EVENT: &str = "scribe://recording-stopped";

/// Broadcasts that a recording stopped. Fire-and-forget, same as
/// `emit_dictation_state`.
fn emit_recording_stopped(app: &AppHandle, metadata: &RecordingMetadata) {
    let _ = app.emit(RECORDING_STOPPED_EVENT, metadata);
}

/// Broadcasts a short-lived polish-selection notice to the pill. Fire-and-forget,
/// same as `emit_dictation_state`.
fn emit_polish_selection_notice(app: &AppHandle, message: impl Into<String>) {
    let _ = app.emit(
        POLISH_SELECTION_NOTICE_EVENT,
        PolishSelectionNoticeEvent {
            message: message.into(),
        },
    );
}

/// Shows or hides a floating panel window (the dictation pill or the meeting
/// popup) on the main thread (AppKit calls must run there). Hiding it hands
/// key focus back to the user's previously-active window so e.g. a
/// synthesised paste lands in their field; `order_front_regardless` brings it
/// back without making it the key window again.
#[cfg(target_os = "macos")]
fn set_panel_visible(app: &AppHandle, label: &'static str, visible: bool) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = app.get_webview_panel(label) {
            if visible {
                panel.order_front_regardless();
            } else {
                panel.order_out(None);
            }
        }
    });
}

/// Shows or hides the dictation pill. Thin wrapper over [`set_panel_visible`]
/// kept as a named function since most call sites only care about the pill.
#[cfg(target_os = "macos")]
fn set_pill_visible(app: &AppHandle, visible: bool) {
    set_panel_visible(app, DICTATION_PILL_WINDOW, visible);
}

/// A coordinate far outside any real display, used to "hide" the meeting
/// popup and recording indicator by moving them off-screen rather than
/// ordering them out (see `set_positioned_panel_visible`'s doc comment).
#[cfg(target_os = "macos")]
const OFFSCREEN_POSITION: (f64, f64) = (-10_000.0, -10_000.0);

/// Shows or hides a floating panel by **moving it** plus toggling its alpha
/// and click-through state, not by ordering it in/out like
/// [`set_panel_visible`] does for the pill. The pill toggles visibility for
/// only ~100ms at a time (mid-paste); the meeting popup and recording
/// indicator can sit hidden for hours waiting for a call or recording.
/// Empirically (screenshotting the window's own screen region), a WKWebView
/// left ordered-out that long comes back showing a stale opaque black
/// backing buffer instead of its actual transparent content — macOS appears
/// to reclaim/invalidate the backing store of windows ordered out for a
/// long time. Keeping the window permanently ordered front (so its backing
/// store is never reclaimed) and moving it off any display sidesteps that.
///
/// The off-screen move alone is not a reliable hide, though: `set_position`
/// applies asynchronously and was observed (via `debug_log` plus a
/// CGWindowList dump) to return `Ok` yet never move the window, leaving a
/// phantom recording indicator on screen; macOS can also move a parked
/// off-screen window back onto a display when the display configuration
/// changes (sleep/wake, monitor plug/unplug). So hiding *also* sets the
/// panel's alpha to 0 and makes it click-through — direct, synchronous
/// AppKit calls that cannot be dropped — and showing restores them. Even if
/// a move is lost, a "hidden" panel can never be seen or intercept clicks.
#[cfg(target_os = "macos")]
fn set_positioned_panel_visible(
    app: &AppHandle,
    label: &'static str,
    anchor: WindowAnchor,
    width: f64,
    height: f64,
    margin: f64,
    visible: bool,
) {
    let app = app.clone();
    debug_log(&format!(
        "set_positioned_panel_visible label={label} visible={visible} caller_thread={:?}",
        std::thread::current().id()
    ));
    let dispatch = app.clone().run_on_main_thread(move || {
        use tauri_nspanel::ManagerExt;
        let Some(window) = app.get_webview_window(label) else {
            debug_log(&format!(
                "set_positioned_panel_visible label={label}: window not found"
            ));
            return;
        };
        // Windows are converted to panels in setup() before their first
        // hide; a missing panel here means that conversion failed, so fall
        // back to position-only toggling rather than doing nothing.
        let panel = app.get_webview_panel(label).ok();
        if panel.is_none() {
            debug_log(&format!(
                "set_positioned_panel_visible label={label}: panel not found, position-only fallback"
            ));
        }
        if visible {
            position_window(&window, anchor, width, height, margin);
            if let Some(panel) = &panel {
                panel.set_ignore_mouse_events(false);
                panel.set_alpha_value(1.0);
            }
            debug_log(&format!(
                "set_positioned_panel_visible label={label}: shown, readback={:?}",
                window.outer_position()
            ));
        } else {
            if let Some(panel) = &panel {
                panel.set_alpha_value(0.0);
                panel.set_ignore_mouse_events(true);
            }
            let (x, y) = OFFSCREEN_POSITION;
            let result = window.set_position(tauri::LogicalPosition::new(x, y));
            debug_log(&format!(
                "set_positioned_panel_visible label={label}: hide result={result:?} readback={:?}",
                window.outer_position()
            ));
        }
    });
    debug_log(&format!(
        "set_positioned_panel_visible label={label} visible={visible} dispatch_ok={}",
        dispatch.is_ok()
    ));
}

/// Diagnostic logging for floating-panel visibility: appends to
/// ~/Library/Logs/Scribe/app-debug.log (eprintln is invisible for a
/// Finder-launched .app). This trail is what proved a hide's `set_position`
/// can return `Ok` yet never apply (see `set_positioned_panel_visible`);
/// kept because show/hide failures are environment-dependent (multi-monitor,
/// sleep/wake) and unreproducible without a record of what was dispatched.
#[cfg(target_os = "macos")]
fn debug_log(message: &str) {
    use std::io::Write;
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = std::path::Path::new(&home).join("Library/Logs/Scribe");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("app-debug.log"))
    {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(file, "{timestamp} {message}");
    }
}

/// Shows or hides the "Record this meeting?" popup.
#[cfg(target_os = "macos")]
fn set_meeting_popup_visible(app: &AppHandle, visible: bool) {
    set_positioned_panel_visible(
        app,
        MEETING_POPUP_WINDOW,
        WindowAnchor::TopCenter,
        MEETING_POPUP_WIDTH,
        MEETING_POPUP_HEIGHT,
        MEETING_POPUP_TOP_MARGIN,
        visible,
    );
}

/// Shows or hides the recording-in-progress indicator.
#[cfg(target_os = "macos")]
fn set_recording_indicator_visible(app: &AppHandle, visible: bool) {
    set_positioned_panel_visible(
        app,
        RECORDING_INDICATOR_WINDOW,
        WindowAnchor::RightCenter,
        RECORDING_INDICATOR_WIDTH,
        RECORDING_INDICATOR_HEIGHT,
        RECORDING_INDICATOR_RIGHT_MARGIN,
        visible,
    );
}

/// Shows or clears a "listening" indicator in the menu bar by setting the tray
/// title, so the user can see when dictation is recording even with the main
/// window hidden.
fn set_recording_indicator(app: &AppHandle, recording: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        // Clear with an empty string rather than None: on macOS set_title(None)
        // can leave the previous title in place, so the indicator would hang.
        let title = if recording { "● Rec" } else { "" };
        let _ = tray.set_title(Some(title));
    }
}

/// Label of the always-on dictation pill window (the small floating bar pinned to
/// the bottom-center of the screen).
#[cfg(desktop)]
const DICTATION_PILL_WINDOW: &str = "pill";

/// Logical window sizes of the dictation pill per visual layout, plus the gap
/// kept above the Dock. The window is resized to hug each state's painted
/// content (see `set_pill_layout`): the window is transparent, so any area
/// beyond the visuals is an invisible click-trap sitting over whatever the
/// user has at the bottom of the screen. Idle stays a small sliver; hover
/// needs headroom for the tooltip; listening/transcribing fit the waveform
/// capsule; notices fit a short line of text.
#[cfg(desktop)]
const PILL_IDLE_SIZE: (f64, f64) = (64.0, 18.0);
#[cfg(desktop)]
const PILL_HOVER_SIZE: (f64, f64) = (210.0, 64.0);
#[cfg(desktop)]
const PILL_ACTIVE_SIZE: (f64, f64) = (160.0, 40.0);
#[cfg(desktop)]
const PILL_NOTICE_SIZE: (f64, f64) = (300.0, 40.0);
#[cfg(desktop)]
const PILL_BOTTOM_MARGIN: f64 = 8.0;

/// The pill window's current frame in global top-left-origin screen points,
/// kept in sync by `create_dictation_pill` and `set_pill_layout`. The hover
/// watcher polls the cursor against this instead of relying on WebKit mouse
/// tracking, which is unreliable inside a non-activating panel that is never
/// the key window (mouseenter simply never fires there).
#[cfg(desktop)]
static PILL_RECT: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// Records the pill frame most recently applied by `position_window`.
#[cfg(desktop)]
fn remember_pill_rect(position: Option<(f64, f64)>, width: f64, height: f64) {
    if let (Some((x, y)), Ok(mut rect)) = (position, PILL_RECT.lock()) {
        *rect = Some((x, y, width, height));
    }
}

/// Label of the meeting-detected prompt window (the small bar pinned to the
/// top-center of the screen, like the dictation pill but anchored opposite).
#[cfg(desktop)]
const MEETING_POPUP_WINDOW: &str = "meeting-popup";

/// Logical size of the meeting popup and the gap kept below the menu bar.
#[cfg(desktop)]
const MEETING_POPUP_WIDTH: f64 = 360.0;
#[cfg(desktop)]
const MEETING_POPUP_HEIGHT: f64 = 64.0;
#[cfg(desktop)]
const MEETING_POPUP_TOP_MARGIN: f64 = 8.0;

/// Label of the recording-in-progress indicator window (the small vertical
/// capsule pinned to the right-center of the screen, with a stop button).
#[cfg(desktop)]
const RECORDING_INDICATOR_WINDOW: &str = "recording-indicator";

/// Logical size of the recording indicator and the gap kept clear of the
/// screen's right edge.
#[cfg(desktop)]
const RECORDING_INDICATOR_WIDTH: f64 = 56.0;
#[cfg(desktop)]
const RECORDING_INDICATOR_HEIGHT: f64 = 132.0;
#[cfg(desktop)]
const RECORDING_INDICATOR_RIGHT_MARGIN: f64 = 10.0;

/// Which edge of the primary monitor's work area a floating window is pinned to.
/// The shared `Center` postfix is deliberate (each window sits at the center
/// of its edge, not a corner), so the variant-name lint doesn't apply.
#[cfg(desktop)]
#[derive(Clone, Copy)]
#[allow(clippy::enum_variant_names)]
enum WindowAnchor {
    BottomCenter,
    TopCenter,
    RightCenter,
}

/// Creates the floating dictation pill: a small, transparent, always-on-top bar
/// pinned to the bottom-center of the primary screen. On macOS it is converted to
/// a non-activating panel (see `make_window_non_activating`) so clicking it never
/// activates Scribe; `accept_first_mouse` lets the mic button fire on the first
/// click even though the panel isn't the key window (otherwise the first click is
/// swallowed just to focus the webview).
#[cfg(desktop)]
fn create_dictation_pill(app: &AppHandle) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let pill = WebviewWindowBuilder::new(
        app,
        DICTATION_PILL_WINDOW,
        WebviewUrl::App("index.html?view=pill".into()),
    )
    .title("Scribe Dictation")
    .inner_size(PILL_IDLE_SIZE.0, PILL_IDLE_SIZE.1)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focusable(false)
    .focused(false)
    .accept_first_mouse(true)
    .visible(true)
    .build()?;

    let position = position_window(
        &pill,
        WindowAnchor::BottomCenter,
        PILL_IDLE_SIZE.0,
        PILL_IDLE_SIZE.1,
        PILL_BOTTOM_MARGIN,
    );
    remember_pill_rect(position, PILL_IDLE_SIZE.0, PILL_IDLE_SIZE.1);
    Ok(())
}

/// Window size for one of the pill's visual layouts. `listening` and
/// `transcribing` share a size so that transition never moves the window.
#[cfg(desktop)]
fn pill_layout_size(layout: &str) -> Result<(f64, f64), AppError> {
    match layout {
        "idle" => Ok(PILL_IDLE_SIZE),
        "hover" => Ok(PILL_HOVER_SIZE),
        "listening" | "transcribing" => Ok(PILL_ACTIVE_SIZE),
        "notice" => Ok(PILL_NOTICE_SIZE),
        other => Err(AppError {
            code: "pill_layout_unknown".to_string(),
            message: format!("Unknown dictation pill layout: {other}"),
            details: None,
        }),
    }
}

/// Resizes the pill window to hug the given visual layout and re-anchors it to
/// the bottom-center of the screen. Invoked by the pill webview whenever its
/// visual state changes (idle sliver, hover capsule, listening waveform,
/// transcribing sweep, notice text) so the transparent window never covers —
/// and never swallows clicks meant for — more of the screen than it paints.
#[tauri::command]
fn set_pill_layout(app: AppHandle, layout: String) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let (width, height) = pill_layout_size(&layout)?;
        if let Some(window) = app.get_webview_window(DICTATION_PILL_WINDOW) {
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
            let position = position_window(
                &window,
                WindowAnchor::BottomCenter,
                width,
                height,
                PILL_BOTTOM_MARGIN,
            );
            remember_pill_rect(position, width, height);
        }
    }
    #[cfg(not(desktop))]
    let _ = (app, layout);
    Ok(())
}

/// Creates the floating "Record this meeting?" prompt: a small, transparent,
/// always-on-top bar pinned to the top-center of the primary screen (matching
/// where meeting apps' own notification bars typically sit) — hidden until a
/// live Teams call is detected. Same non-activating-panel treatment as the
/// dictation pill, for the same reason: a click must not steal focus from the
/// meeting app the user is currently in.
///
/// Built `.visible(true)` (like the pill), not `.visible(false)`: a WKWebView
/// built hidden never gets an initial render pass, so ordering it front later
/// showed a stale opaque black backing buffer instead of the actual
/// transparent page (confirmed by screenshotting the window's screen region
/// during a real detected call). It's hidden immediately after being
/// converted to a panel in `setup()` instead, via the same `order_out`
/// mechanism used for all later show/hide toggling.
#[cfg(desktop)]
fn create_meeting_popup(app: &AppHandle) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let popup = WebviewWindowBuilder::new(
        app,
        MEETING_POPUP_WINDOW,
        WebviewUrl::App("index.html?view=meeting-popup".into()),
    )
    .title("Scribe Meeting Prompt")
    .inner_size(MEETING_POPUP_WIDTH, MEETING_POPUP_HEIGHT)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focusable(false)
    .focused(false)
    .accept_first_mouse(true)
    .visible(true)
    .build()?;

    position_window(
        &popup,
        WindowAnchor::TopCenter,
        MEETING_POPUP_WIDTH,
        MEETING_POPUP_HEIGHT,
        MEETING_POPUP_TOP_MARGIN,
    );
    Ok(())
}

/// Creates the floating recording-in-progress indicator: a small, transparent,
/// always-on-top vertical capsule pinned to the right-center of the primary
/// screen — hidden until a recording is active (see `emit_recording_started`/
/// `emit_recording_stopped`), and shown for *any* active recording, not just
/// ones started from the meeting-detection popup. Same non-activating-panel
/// treatment as the pill and the meeting popup.
///
/// Built `.visible(true)`, then hidden right after in `setup()` — see
/// `create_meeting_popup`'s doc comment for why (a hidden-at-creation
/// WKWebView never gets an initial render pass).
#[cfg(desktop)]
fn create_recording_indicator(app: &AppHandle) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let indicator = WebviewWindowBuilder::new(
        app,
        RECORDING_INDICATOR_WINDOW,
        WebviewUrl::App("index.html?view=recording-indicator".into()),
    )
    .title("Scribe Recording Indicator")
    .inner_size(RECORDING_INDICATOR_WIDTH, RECORDING_INDICATOR_HEIGHT)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focusable(false)
    .focused(false)
    .accept_first_mouse(true)
    .visible(true)
    .build()?;

    position_window(
        &indicator,
        WindowAnchor::RightCenter,
        RECORDING_INDICATOR_WIDTH,
        RECORDING_INDICATOR_HEIGHT,
        RECORDING_INDICATOR_RIGHT_MARGIN,
    );
    Ok(())
}

/// Pins `window` to the given edge of the primary monitor's work area — the
/// visible region that already excludes the Dock and menu bar — offset by
/// `margin` so it floats clear of that edge rather than being clipped behind
/// it. `window_width`/`window_height` are the window's known logical size
/// (the same values passed to `.inner_size()`), not queried via
/// `outer_size()`, which can return a stale/default size immediately after
/// `.build()`.
///
/// Works entirely in logical units (points) and positions via
/// `LogicalPosition`, rather than converting to physical pixels by hand:
/// mixing a hand-computed physical-pixel offset with `PhysicalPosition` on a
/// multi-monitor, mixed-DPI setup landed a window at coordinates that
/// matched neither display's logical bounds (confirmed against a real
/// two-monitor Retina + non-Retina setup) — `LogicalPosition` lets
/// tao/Tauri handle the physical/logical and any macOS coordinate-space
/// conversion internally, which is exactly what it's designed to abstract.
/// Best-effort: leaves the window at its default position if monitor info is
/// unavailable.
#[cfg(desktop)]
fn position_window(
    window: &tauri::WebviewWindow,
    anchor: WindowAnchor,
    window_width: f64,
    window_height: f64,
    margin: f64,
) -> Option<(f64, f64)> {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return None;
    };
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    let work_x = work_area.position.x as f64 / scale;
    let work_y = work_area.position.y as f64 / scale;
    let work_width = work_area.size.width as f64 / scale;
    let work_height = work_area.size.height as f64 / scale;

    let (x, y) = match anchor {
        WindowAnchor::BottomCenter | WindowAnchor::TopCenter => {
            let x = work_x + (work_width - window_width) / 2.0;
            let y = if matches!(anchor, WindowAnchor::BottomCenter) {
                work_y + work_height - window_height - margin
            } else {
                work_y + margin
            };
            (x, y)
        }
        WindowAnchor::RightCenter => {
            let x = work_x + work_width - window_width - margin;
            let y = work_y + (work_height - window_height) / 2.0;
            (x, y)
        }
    };
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    Some((x, y))
}

/// Converts a floating window (the dictation pill or the meeting popup) into a
/// non-activating NSPanel. A normal macOS window activates its owning app when
/// clicked, which would raise Scribe's main window over the floating bar and
/// steal focus from whatever app the user is currently in. The non-activating
/// panel style mask lets it receive clicks without ever activating the app.
/// Best-effort: logs and returns if the window is missing or the swizzle fails.
#[cfg(target_os = "macos")]
// tauri-nspanel's public API is built on the older `cocoa` crate, which is now
// deprecated in favour of objc2-app-kit; the plugin still requires these types.
#[allow(deprecated)]
fn make_window_non_activating(app: &AppHandle, label: &str) {
    use tauri_nspanel::cocoa::appkit::NSWindowCollectionBehavior;
    use tauri_nspanel::WebviewWindowExt;

    let Some(window) = app.get_webview_window(label) else {
        eprintln!("{label}: window missing, cannot convert to panel");
        return;
    };
    let panel = match window.to_panel() {
        Ok(panel) => panel,
        Err(error) => {
            eprintln!("{label}: to_panel failed: {error}");
            return;
        }
    };

    // Float above ordinary windows.
    const NS_FLOATING_WINDOW_LEVEL: i32 = 4;
    panel.set_level(NS_FLOATING_WINDOW_LEVEL);

    // NSWindowStyleMaskNonactivatingPanel — a click must not activate Scribe (the
    // app stays in the background; only its key window changes). Both floating
    // windows are borderless, so this is the only style bit either needs. The
    // dictation pill still briefly takes *key* focus on click, so the dictation
    // flow re-activates the user's previous app before pasting (see
    // `start_dictation_session` / inject).
    const NS_NONACTIVATING_PANEL: i32 = 1 << 7;
    panel.set_style_mask(NS_NONACTIVATING_PANEL);

    // Keep the window visible across spaces and alongside other apps' full-screen
    // windows, matching its always-on-top intent.
    panel.set_collection_behaviour(
        NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
    );
}

/// Handles one line of output from the meeting-detector sidecar: advances the
/// call-prompt state machine (`meeting_detection::advance`) and shows/hides
/// the popup accordingly. Runs on the sidecar's own reader thread, so all
/// AppKit-touching work goes through `set_meeting_popup_visible`, which hops
/// to the main thread itself.
fn on_meeting_detector_line(app: &AppHandle, line: &str) {
    let Some(event) = DetectorEvent::from_sidecar_line(line) else {
        return;
    };
    let state = app.state::<AppState>();
    let recording_already_active = state
        .recordings
        .lock()
        .map(|recordings| recordings.is_recording())
        .unwrap_or(false);
    let action = {
        let Ok(mut call_state) = state.meeting_call_state.lock() else {
            eprintln!("meeting detector: call state lock poisoned");
            return;
        };
        let (next_state, action) = advance(*call_state, event, recording_already_active);
        *call_state = next_state;
        action
    };

    match action {
        PromptAction::ShowPrompt => {
            let meeting_id = match current_time_ms() {
                Ok(now_ms) => format!("meeting-{now_ms}"),
                Err(error) => {
                    eprintln!("meeting detector: could not id meeting: {}", error.message);
                    return;
                }
            };
            emit_meeting_detected(app, &meeting_id);
            #[cfg(target_os = "macos")]
            set_meeting_popup_visible(app, true);
        }
        PromptAction::HidePrompt => {
            emit_meeting_call_ended(app);
            #[cfg(target_os = "macos")]
            set_meeting_popup_visible(app, false);
        }
        PromptAction::None => {}
    }
}

/// Starts the meeting-detector sidecar if `promptOnTeamsMeeting` is enabled
/// and it isn't already running. Best-effort: a failed start is logged, not
/// propagated, since it must never block app startup or a settings save.
fn start_meeting_detection_if_enabled(app: &AppHandle, settings: &ScribeSettings) {
    if !settings.prompt_on_teams_meeting {
        return;
    }
    let state = app.state::<AppState>();
    let Ok(mut detector) = state.meeting_detector.lock() else {
        eprintln!("meeting detector: lock poisoned, not starting");
        return;
    };
    let app_for_thread = app.clone();
    if let Err(error) = detector.start(move |line| on_meeting_detector_line(&app_for_thread, &line))
    {
        eprintln!(
            "meeting_detector_start_failed: {} ({})",
            error.message, error.code
        );
    }
}

/// Stops the meeting-detector sidecar (if running) and resets the prompt
/// state, hiding the popup if it happened to be showing.
fn stop_meeting_detection(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(mut detector) = state.meeting_detector.lock() {
        detector.stop();
    }
    if let Ok(mut call_state) = state.meeting_call_state.lock() {
        *call_state = CallPromptState::default();
    }
    #[cfg(target_os = "macos")]
    set_meeting_popup_visible(app, false);
}

/// Maps a persisted hotkey token (dictation or polish-selection) to a
/// registrable global shortcut. The set is deliberately small and vetted to
/// avoid the bare F-keys (media keys on Mac laptops) and F5 (Apple Dictation).
/// Returns None for an unknown token.
#[cfg(desktop)]
fn hotkey_shortcut_for(token: &str) -> Option<tauri_plugin_global_shortcut::Shortcut> {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    let (modifiers, code) = match token {
        "cmd+shift+d" => (Modifiers::SUPER | Modifiers::SHIFT, Code::KeyD),
        "ctrl+option+d" => (Modifiers::CONTROL | Modifiers::ALT, Code::KeyD),
        "cmd+shift+space" => (Modifiers::SUPER | Modifiers::SHIFT, Code::Space),
        "ctrl+option+p" => (Modifiers::CONTROL | Modifiers::ALT, Code::KeyP),
        _ => return None,
    };
    Some(Shortcut::new(Some(modifiers), code))
}

/// Registers `new_token` as a global shortcut, first unregistering
/// `previous_token`'s binding if it differs. Targets only its own shortcut
/// (never `unregister_all`) so the dictation and polish-selection hotkeys —
/// each registered independently — never clobber one another.
#[cfg(desktop)]
fn register_hotkey<F>(
    app: &AppHandle,
    previous_token: Option<&str>,
    new_token: &str,
    error_code: &str,
    on_press: F,
) -> Result<(), AppError>
where
    F: Fn(&AppHandle) + Send + Sync + 'static,
{
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let shortcut = hotkey_shortcut_for(new_token).ok_or_else(|| AppError {
        code: format!("{error_code}_unsupported"),
        message: format!("Unsupported hotkey: {new_token}"),
        details: None,
    })?;
    let global_shortcut = app.global_shortcut();
    if let Some(previous) = previous_token {
        if previous != new_token {
            if let Some(previous_shortcut) = hotkey_shortcut_for(previous) {
                let _ = global_shortcut.unregister(previous_shortcut);
            }
        }
    }
    global_shortcut
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                on_press(app);
            }
        })
        .map_err(|error| AppError {
            code: format!("{error_code}_register_failed"),
            message: "Could not register the hotkey.".to_string(),
            details: Some(error.to_string()),
        })
}

/// Registers (or re-registers) the dictation hotkey.
#[cfg(desktop)]
fn register_dictation_hotkey(
    app: &AppHandle,
    previous_token: Option<&str>,
    token: &str,
) -> Result<(), AppError> {
    register_hotkey(app, previous_token, token, "dictation_hotkey", |app| {
        on_dictation_hotkey_press(app);
    })
}

/// Registers the polish-selection hotkey: polishes whatever text is selected
/// in the focused app and pastes the result back in place.
#[cfg(desktop)]
fn register_polish_selection_hotkey(app: &AppHandle, token: &str) -> Result<(), AppError> {
    register_hotkey(app, None, token, "polish_selection_hotkey", |app| {
        on_polish_selection_hotkey_press(app);
    })
}

/// Handles the polish-selection hotkey: copies whatever is selected in the
/// focused app, polishes it, and pastes the result back in place. Runs off
/// the main thread since it shells out to `osascript`/`pbpaste` and the
/// Apple Intelligence sidecar. No pill focus-handback is needed here — unlike
/// dictation, the trigger is a hotkey, not a click on Scribe's own UI, so
/// focus never leaves the app the user is selecting text in.
fn on_polish_selection_hotkey_press(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        match dictation::polish_selection() {
            Ok(dictation::SelectionPolishOutcome::Applied) => {
                let mut count = state
                    .polish_selection_notice_count
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *count = 0;
            }
            Ok(dictation::SelectionPolishOutcome::NoSelection) => {
                let mut count = state
                    .polish_selection_notice_count
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *count += 1;
                let message = if *count <= 3 {
                    "Select some text first, then try again."
                } else {
                    "Select text to polish it."
                };
                drop(count);
                emit_polish_selection_notice(&app, message);
            }
            Ok(dictation::SelectionPolishOutcome::PasteFailed) => {
                emit_polish_selection_notice(
                    &app,
                    "Couldn't paste right now — the polished text is on your clipboard.",
                );
            }
            Err(error) => {
                eprintln!(
                    "polish selection failed: {} ({})",
                    error.message, error.code
                );
                emit_polish_selection_notice(&app, "Couldn't polish the selection. Try again.");
            }
        }
    });
}

/// Starts a dictation session: begins microphone capture and signals the
/// listening state to the UI and menu bar. Shared by the hotkey handler and the
/// pill's toggle command. On failure it resets the hotkey press tracker so a
/// failed start can never leave it stuck thinking a dictation is in flight.
fn start_dictation_session(app: &AppHandle) {
    let state = app.state::<AppState>();
    match begin_dictation(&state) {
        Ok(()) => {
            eprintln!("dictation: listening…");
            set_recording_indicator(app, true);
            emit_dictation_state(app, "listening");
            spawn_dictation_level_emitter(app);
            play_cue(DICTATION_START_SOUND);
        }
        Err(error) => {
            eprintln!("dictation start failed: {} ({})", error.message, error.code);
            if let Ok(mut tracker) = state.dictation_hotkey.lock() {
                *tracker = DictationHotkey::new();
            }
            emit_dictation_state(app, "idle");
        }
    }
}

/// Stops the in-flight dictation and runs transcribe → optional polish → inject
/// off the main thread, signalling the transcribing state up front and idle when
/// done. Shared by the hotkey handler and the pill's toggle command.
fn stop_and_process_dictation(app: &AppHandle) {
    eprintln!("dictation: transcribing…");
    set_recording_indicator(app, false);
    emit_dictation_state(app, "transcribing");
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let outcome =
            stop_dictation_capture(&state).and_then(|(wav_path, settings, started_at_ms)| {
                let mut text = transcribe_dictation_wav(&wav_path, &settings)?;
                if settings.dictation_polish_enabled && !text.trim().is_empty() {
                    // Polish with Apple Intelligence, but fall back to the raw
                    // transcript if it is unavailable or returns nothing.
                    match dictation::polish_text(&text) {
                        Ok(polished) if !polished.trim().is_empty() => text = polished,
                        Ok(_) => {}
                        Err(error) => eprintln!(
                            "dictation: polish failed ({}), inserting raw text",
                            error.code
                        ),
                    }
                }
                record_dictation_session(&state, started_at_ms, &text);
                // Before pasting, hand key focus back to the user's app: clicking the
                // pill can make it the key window (dropping the user's field as first
                // responder), so a synthesised Cmd+V would land nowhere. Hiding the
                // always-on-top pill returns key to the previously-active window;
                // after the paste, bring the pill back without re-keying it. Skipped
                // for empty transcripts (inject is a no-op then).
                #[cfg(target_os = "macos")]
                let restore_pill = !text.trim().is_empty();
                #[cfg(target_os = "macos")]
                if restore_pill {
                    set_pill_visible(&app, false);
                    // Brief: just long enough for key focus to return to the user's
                    // window before the synthesised paste fires.
                    std::thread::sleep(std::time::Duration::from_millis(70));
                }
                let inject_result = dictation::inject_text(&text);
                #[cfg(target_os = "macos")]
                if restore_pill {
                    // inject_text already waited for the paste keystroke, so the pill
                    // can float back immediately.
                    set_pill_visible(&app, true);
                }
                inject_result?;
                Ok(text)
            });
        match outcome {
            Ok(text) if text.trim().is_empty() => {
                eprintln!("dictation: no speech detected, nothing inserted");
            }
            Ok(text) => {
                // Log only the length, not the dictated text itself.
                eprintln!("dictation: inserted {} characters", text.chars().count());
                play_cue(DICTATION_DONE_SOUND);
            }
            Err(error) => {
                eprintln!("dictation stop failed: {} ({})", error.message, error.code);
            }
        }
        emit_dictation_state(&app, "idle");
    });
}

/// Handles one dictation hotkey press: advances the Wispr-style press tracker and
/// starts or stops dictation via the shared session helpers.
fn on_dictation_hotkey_press(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now_ms = match current_time_ms() {
        Ok(now) => now,
        Err(error) => {
            eprintln!("dictation hotkey: clock error: {}", error.message);
            return;
        }
    };
    let action = match state.dictation_hotkey.lock() {
        Ok(mut tracker) => tracker.on_press(now_ms),
        Err(_) => {
            eprintln!("dictation hotkey: press tracker lock poisoned");
            return;
        }
    };

    match action {
        HotkeyAction::None => {}
        HotkeyAction::StartRecording => start_dictation_session(app),
        HotkeyAction::StopRecording => stop_and_process_dictation(app),
    }
}

/// Toggles dictation from the pill's mic button: starts if idle, otherwise stops
/// and processes. Keeps the hotkey press tracker in sync so a later hotkey press
/// behaves correctly no matter which control started the dictation.
#[tauri::command]
fn toggle_dictation(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let is_recording = {
        let recorder = state.dictation.lock().map_err(map_lock_error)?;
        recorder.is_recording()
    };
    if is_recording {
        if let Ok(mut tracker) = state.dictation_hotkey.lock() {
            tracker.mark_recording_stopped();
        }
        stop_and_process_dictation(&app);
    } else {
        let now_ms = current_time_ms()?;
        if let Ok(mut tracker) = state.dictation_hotkey.lock() {
            tracker.mark_recording_started(now_ms);
        }
        start_dictation_session(&app);
    }
    Ok(())
}

/// Injects dictated text into the focused app via clipboard paste. Runs off the
/// main thread because it spawns `pbcopy` and `osascript`.
#[tauri::command]
async fn inject_dictation_text(text: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || dictation::inject_text(&text))
        .await
        .map_err(|error| AppError {
            code: "dictation_inject_task_failed".to_string(),
            message: "The dictation injection task did not finish.".to_string(),
            details: Some(error.to_string()),
        })?
}

#[tauri::command]
async fn polish_dictation(text: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || dictation::polish_text(&text))
        .await
        .map_err(|error| AppError {
            code: "dictation_polish_task_failed".to_string(),
            message: "The dictation polish task did not finish.".to_string(),
            details: Some(error.to_string()),
        })?
}

/// Persists the dictation hotkey + polish preference and re-registers the hotkey
/// so the change takes effect immediately.
#[tauri::command]
fn update_dictation_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    dictation_hotkey: String,
    dictation_polish_enabled: bool,
) -> Result<ScribeSettings, AppError> {
    #[cfg(desktop)]
    if hotkey_shortcut_for(&dictation_hotkey).is_none() {
        return Err(AppError {
            code: "dictation_hotkey_unsupported".to_string(),
            message: format!("Unsupported dictation hotkey: {dictation_hotkey}"),
            details: None,
        });
    }

    let (previous_hotkey, updated) = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let mut settings = repository.get_settings()?;
        let previous_hotkey = settings.dictation_hotkey.clone();
        settings.dictation_hotkey = dictation_hotkey;
        settings.dictation_polish_enabled = dictation_polish_enabled;
        repository.upsert_settings(&settings, current_time_ms()?)?;
        (
            previous_hotkey,
            hydrate_settings_with_local_defaults(settings),
        )
    };

    #[cfg(desktop)]
    register_dictation_hotkey(&app, Some(&previous_hotkey), &updated.dictation_hotkey)?;

    Ok(updated)
}

/// Runs meeting summarization against whichever local model server the user
/// configured. LM Studio gets the full lifecycle treatment (start the server,
/// load the model, summarize, unload to free RAM) via its `lms` CLI; Ollama and
/// Custom endpoints have no equivalent local start/load step, so they're
/// expected to already be reachable and just get a direct chat-completion call.
fn run_summary(
    provider: SummarizerProvider,
    host: &str,
    port: u16,
    segments: Vec<AnalysisTranscriptSegment>,
    model: String,
) -> Result<MeetingSummary, AppError> {
    match provider {
        SummarizerProvider::LmStudio => {
            let lifecycle = LmStudioLifecycle::resolve(None)?;
            lifecycle.ensure_running()?;
            lifecycle.load(&model)?;
            let summarizer = LmStudioSummarizer::new(
                LmStudioClient {
                    host: host.to_string(),
                    port,
                },
                model,
            );
            let result = summarizer.summarize(&segments, false);
            lifecycle.unload_all();
            result
        }
        SummarizerProvider::Ollama | SummarizerProvider::Custom => {
            let summarizer = LmStudioSummarizer::new(
                OpenAiCompatibleClient {
                    host: host.to_string(),
                    port,
                },
                model,
            );
            summarizer.summarize(&segments, false)
        }
    }
}

/// Persists which local model server to summarize meetings with.
#[tauri::command]
fn update_summarizer_settings(
    state: State<'_, AppState>,
    summarizer_provider: SummarizerProvider,
    summarizer_host: String,
    summarizer_port: u16,
    summarizer_model: Option<String>,
) -> Result<ScribeSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.summarizer_provider = summarizer_provider;
    settings.summarizer_host = summarizer_host;
    settings.summarizer_port = summarizer_port;
    settings.summarizer_model = summarizer_model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

/// Lists model ids/names available on the given local model server, so
/// Settings can offer a picker instead of asking the user to type a model
/// name from memory. Errors (server unreachable, endpoint unsupported) are
/// returned as-is for the frontend to surface inline.
#[tauri::command]
async fn list_summarizer_models(
    summarizer_provider: SummarizerProvider,
    summarizer_host: String,
    summarizer_port: u16,
) -> Result<Vec<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        list_summarizer_models_impl(summarizer_provider, &summarizer_host, summarizer_port)
    })
    .await
    .map_err(|error| AppError {
        code: "summarizer_models_task_failed".to_string(),
        message: "The model-listing task did not finish.".to_string(),
        details: Some(error.to_string()),
    })?
}

/// Runs all three onboarding permission probes off the main thread — each
/// starts (and immediately discards) a short real capture or a read-only
/// Accessibility query, so this genuinely takes a moment.
#[tauri::command]
async fn check_permissions() -> Result<permissions::PermissionsSnapshot, AppError> {
    tauri::async_runtime::spawn_blocking(permissions::check_permissions)
        .await
        .map_err(|error| AppError {
            code: "permissions_check_task_failed".to_string(),
            message: "The permissions check did not finish.".to_string(),
            details: Some(error.to_string()),
        })
}

/// Deep-links to the given pane in System Settings > Privacy & Security, for
/// permissions macOS won't re-prompt for after an explicit deny.
#[tauri::command]
fn open_permission_settings(pane: String) -> Result<(), AppError> {
    permissions::open_system_settings_pane(&pane)
}

/// Longest accepted vocabulary text. Whisper truncates carried prompts to
/// half its text context anyway, so anything longer would silently lose terms.
const MAX_TRANSCRIBER_VOCABULARY_CHARS: usize = 600;

#[tauri::command]
fn update_transcriber_settings(
    state: State<'_, AppState>,
    transcriber_bin_path: Option<String>,
    transcriber_model_path: Option<String>,
    transcriber_vocabulary: Option<String>,
    speaker_embedding_model_path: Option<String>,
    speaker_segmentation_model_path: Option<String>,
) -> Result<ScribeSettings, AppError> {
    let transcriber_vocabulary = transcriber_vocabulary
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(vocabulary) = &transcriber_vocabulary {
        if vocabulary.chars().count() > MAX_TRANSCRIBER_VOCABULARY_CHARS {
            return Err(AppError {
                code: "invalid_transcriber_vocabulary".to_string(),
                message: format!(
                    "Vocabulary is too long; keep it under {MAX_TRANSCRIBER_VOCABULARY_CHARS} characters."
                ),
                details: Some(format!("length={}", vocabulary.chars().count())),
            });
        }
    }
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.transcriber_bin_path = normalize_optional_path(transcriber_bin_path);
    settings.transcriber_model_path = normalize_optional_path(transcriber_model_path);
    settings.transcriber_vocabulary = transcriber_vocabulary;
    settings.speaker_embedding_model_path = normalize_optional_path(speaker_embedding_model_path);
    settings.speaker_segmentation_model_path =
        normalize_optional_path(speaker_segmentation_model_path);
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

#[tauri::command]
fn update_audio_processing_settings(
    state: State<'_, AppState>,
    enable_system_audio: bool,
    enable_echo_cancellation: bool,
) -> Result<ScribeSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.enable_system_audio = enable_system_audio;
    settings.enable_echo_cancellation = enable_echo_cancellation;
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

#[tauri::command]
fn update_theme_preference(
    state: State<'_, AppState>,
    theme_preference: ThemePreference,
) -> Result<ScribeSettings, AppError> {
    let repository = state.repository.lock().map_err(map_lock_error)?;
    let mut settings = repository.get_settings()?;
    settings.theme_preference = theme_preference;
    repository.upsert_settings(&settings, current_time_ms()?)?;
    Ok(hydrate_settings_with_local_defaults(settings))
}

/// Persists whether a live Teams call should prompt to record, and starts or
/// stops the detector sidecar immediately so the change takes effect without
/// an app restart.
#[tauri::command]
fn update_meeting_detection_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt_on_teams_meeting: bool,
) -> Result<ScribeSettings, AppError> {
    let updated = {
        let repository = state.repository.lock().map_err(map_lock_error)?;
        let mut settings = repository.get_settings()?;
        settings.prompt_on_teams_meeting = prompt_on_teams_meeting;
        repository.upsert_settings(&settings, current_time_ms()?)?;
        hydrate_settings_with_local_defaults(settings)
    };

    if prompt_on_teams_meeting {
        start_meeting_detection_if_enabled(&app, &updated);
    } else {
        stop_meeting_detection(&app);
    }

    Ok(updated)
}

/// Dismisses the currently showing "record this meeting?" popup: the call
/// keeps running, but the prompt won't reappear until it ends and a new one
/// starts.
#[tauri::command]
fn dismiss_meeting_prompt(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    {
        let mut call_state = state.meeting_call_state.lock().map_err(map_lock_error)?;
        let (next_state, _action) = advance(*call_state, DetectorEvent::Dismissed, false);
        *call_state = next_state;
    }
    #[cfg(target_os = "macos")]
    set_meeting_popup_visible(&app, false);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
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
            removed_audio_metadata_count: 0,
            skipped_audio_file_count: 0,
        }
    } else {
        let cutoff_ms = retention_cutoff_ms(retention_days, current_time_ms()?);
        let expired_metadata = {
            let repository = state.repository.lock().map_err(map_lock_error)?;
            repository.list_audio_metadata_before(cutoff_ms)?
        };
        let mut cleanup = delete_retained_audio_files(&expired_metadata, &app_data_dir(&app)?)?;
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
            .replace(['\n', '\r'], " ")
    )
}

#[tauri::command]
fn stop_recording(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingMetadata, AppError> {
    let stopped_at_ms = current_time_ms()?;
    let stop_result = state
        .recordings
        .lock()
        .map_err(map_lock_error)?
        .stop_recording(stopped_at_ms);
    let metadata = match stop_result {
        Ok(metadata) => metadata,
        Err(error) => {
            // Whatever went wrong, no recording is running after a stop
            // attempt — so the indicator must not stay on screen. Without
            // this, an indicator that is visible while no recording is
            // active (e.g. after a missed hide) has a Stop button that can
            // never dismiss it: the command errors out with
            // "recording_not_active" before ever reaching the hide below.
            #[cfg(target_os = "macos")]
            set_recording_indicator_visible(&app, false);
            return Err(error);
        }
    };
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

    // Broadcast to every window (see `start_recording`'s matching comment) —
    // whichever window the user clicked Stop from (the main window or the
    // floating recording indicator), all windows need to learn the recording
    // ended so their UI stays in sync.
    emit_recording_stopped(&app, &metadata);
    #[cfg(target_os = "macos")]
    set_recording_indicator_visible(&app, false);

    Ok(metadata)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_global_shortcut::Builder::new().build());
    // The NSPanel plugin is macOS-only; it backs the non-activating dictation pill.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            list_meeting_history,
            get_meeting_history_detail,
            delete_meeting,
            update_meeting_title,
            update_meeting_user_notes,
            list_meeting_trends,
            list_audio_devices,
            start_recording,
            stop_recording,
            transcribe_meeting,
            calculate_metrics,
            summarize_meeting,
            start_dictation,
            stop_dictation,
            list_dictation_sessions,
            delete_dictation_session,
            get_dictation_stats_summary,
            toggle_dictation,
            set_pill_layout,
            inject_dictation_text,
            polish_dictation,
            update_dictation_settings,
            update_summarizer_settings,
            list_summarizer_models,
            check_permissions,
            open_permission_settings,
            update_transcriber_settings,
            update_audio_processing_settings,
            update_theme_preference,
            update_privacy_settings,
            update_meeting_detection_settings,
            dismiss_meeting_prompt,
            send_completion_notification
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = app_data_dir.join(SCRIBE_DATABASE_FILE_NAME);
            let repository = SqliteRepository::open(&database_path)
                .map_err(|error| std::io::Error::other(error.message))?;
            let saved_settings = repository
                .get_settings()
                .map_err(|error| std::io::Error::other(error.message))?;
            let hydrated_settings = hydrate_settings_with_local_defaults(saved_settings.clone());
            if hydrated_settings != saved_settings {
                repository
                    .upsert_settings(
                        &hydrated_settings,
                        current_time_ms().map_err(|error| std::io::Error::other(error.message))?,
                    )
                    .map_err(|error| std::io::Error::other(error.message))?;
            }
            app.manage(AppState {
                repository: Mutex::new(repository),
                recordings: Mutex::new(RecordingManager::new(
                    CpalCaptureBackend::new(),
                    ScreenCaptureKitSystemAudioBackend::new(),
                )),
                dictation: Mutex::new(DictationRecorder::new(CpalCaptureBackend::new())),
                dictation_hotkey: Mutex::new(DictationHotkey::new()),
                polish_selection_notice_count: Mutex::new(0),
                meeting_detector: Mutex::new(TeamsCallDetector::new()),
                meeting_call_state: Mutex::new(CallPromptState::default()),
            });
            spawn_audio_retention_cleanup(database_path, app_data_dir.clone());

            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show = MenuItem::with_id(app, "show", "Show Scribe", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &quit])?;
                let tray_icon =
                    tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))
                        .expect("embedded tray icon should be a valid PNG");

                TrayIconBuilder::with_id(TRAY_ICON_ID)
                    .icon(tray_icon)
                    .icon_as_template(true)
                    .menu(&menu)
                    .tooltip("Scribe")
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

            #[cfg(desktop)]
            {
                // Floating dictation pill, pinned bottom-center. Built non-focusable
                // so it can be clicked without stealing focus from the user's app.
                if let Err(error) = create_dictation_pill(app.handle()) {
                    eprintln!("dictation_pill_create_failed: {error}");
                }
                // On macOS, convert it to a non-activating NSPanel so a click never
                // activates Scribe (which would raise the main window over the pill
                // and steal focus). focusable(false) alone does not prevent that.
                #[cfg(target_os = "macos")]
                {
                    make_window_non_activating(app.handle(), DICTATION_PILL_WINDOW);
                    spawn_pill_hover_watcher(app.handle());
                }
            }

            #[cfg(desktop)]
            {
                // Floating "record this meeting?" popup, pinned top-center and
                // hidden until a live Teams call is detected below.
                if let Err(error) = create_meeting_popup(app.handle()) {
                    eprintln!("meeting_popup_create_failed: {error}");
                }
                #[cfg(target_os = "macos")]
                {
                    make_window_non_activating(app.handle(), MEETING_POPUP_WINDOW);
                    // Built visible (see create_meeting_popup's doc comment) and
                    // stays ordered front forever -- "hidden" moves it off-screen
                    // instead (see set_positioned_panel_visible's doc comment).
                    set_meeting_popup_visible(app.handle(), false);
                }
            }

            #[cfg(desktop)]
            {
                // Floating recording-in-progress indicator, pinned right-center
                // and hidden until any recording is active (see
                // `emit_recording_started`/`emit_recording_stopped`).
                if let Err(error) = create_recording_indicator(app.handle()) {
                    eprintln!("recording_indicator_create_failed: {error}");
                }
                #[cfg(target_os = "macos")]
                {
                    make_window_non_activating(app.handle(), RECORDING_INDICATOR_WINDOW);
                    set_recording_indicator_visible(app.handle(), false);
                }
            }

            #[cfg(desktop)]
            {
                // Register the saved dictation hotkey (double-press to start, single
                // press to stop). Configurable from Settings.
                if let Err(error) = register_dictation_hotkey(
                    app.handle(),
                    None,
                    &hydrated_settings.dictation_hotkey,
                ) {
                    eprintln!("{}: {}", error.code, error.message);
                }
                // Register the polish-selection hotkey (single press): polishes
                // whatever text is selected in the focused app and pastes the
                // result back in place.
                if let Err(error) = register_polish_selection_hotkey(
                    app.handle(),
                    &hydrated_settings.polish_selection_hotkey,
                ) {
                    eprintln!("{}: {}", error.code, error.message);
                }
            }

            // Starts the Teams-call-detector sidecar if the setting is on. Best
            // effort: logged, not fatal, since detection is a nice-to-have on
            // top of manual recording, never a prerequisite for it.
            start_meeting_detection_if_enabled(app.handle(), &hydrated_settings);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Scribe")
        .run(|app_handle, event| {
            // The meeting-detector sidecar loops forever; without an explicit
            // stop here it would leak as an orphaned process after Scribe
            // quits (unlike the system-audio-capture sidecar, which only runs
            // for the bounded duration of an active recording).
            if let tauri::RunEvent::Exit = event {
                stop_meeting_detection(app_handle);
            }
        });
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

#[cfg(test)]
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

/// Transcribes the microphone track (echo-cancelled when possible) and, when
/// a system-audio track was recorded, the remote participants' track as well,
/// merging both into one speaker-labeled timeline. The system track is the
/// only place remote voices exist when the user wears headphones, so skipping
/// it would silently drop everyone else from the transcript. A system-track
/// failure degrades to the microphone-only transcript instead of failing the
/// whole meeting, mirroring how recording falls back when system capture
/// cannot start.
fn transcribe_meeting_tracks(
    settings: &ScribeSettings,
    metadata: &AudioMetadata,
    transcriber: &impl transcription::Transcriber,
    echo_cancellation: &impl EchoCancellationBackend,
) -> Result<TranscriptionOutput, AppError> {
    let microphone_audio_path =
        select_transcription_audio_path(settings, metadata, echo_cancellation);
    let microphone_output =
        transcribe_audio_with_retry(transcriber, std::path::Path::new(&microphone_audio_path))?;

    let Some(system_audio_file_path) = metadata.system_audio_file_path.as_deref() else {
        return Ok(microphone_output);
    };
    match transcribe_audio_with_retry(transcriber, std::path::Path::new(system_audio_file_path)) {
        Ok(system_output) => {
            transcription::merge_dual_track_outputs(microphone_output, system_output)
        }
        Err(error) => {
            eprintln!(
                "System audio transcription failed for meeting {}: code={}, message={}; keeping the microphone-only transcript",
                metadata.meeting_id.as_str(),
                error.code,
                error.message
            );
            Ok(microphone_output)
        }
    }
}

fn select_transcription_audio_path(
    settings: &ScribeSettings,
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

    // Metrics coach the user's own speaking (talk time, pace, filler words),
    // so segments attributed to remote participants must not count.
    let segments = repository
        .list_transcript_segments(meeting_id)?
        .into_iter()
        .filter(|segment| transcription::is_user_segment(segment.speaker_label.as_deref()))
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
        removed_audio_metadata_count: 0,
        skipped_audio_file_count: 0,
    };

    for metadata in expired_metadata {
        for file_path in retention_file_paths(metadata) {
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
        let settings = ScribeSettings::default();
        let metadata = audio_metadata_with_system_reference("meeting-aec-enabled");
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/scribe/meeting-aec-enabled.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, "/tmp/scribe/meeting-aec-enabled.aec.wav");
    }

    #[test]
    fn select_transcription_audio_path_falls_back_to_raw_mic_when_aec_fails() {
        let settings = ScribeSettings::default();
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
        let settings = ScribeSettings {
            enable_echo_cancellation: false,
            ..ScribeSettings::default()
        };
        let metadata = audio_metadata_with_system_reference("meeting-aec-disabled");
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/scribe/meeting-aec-disabled.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, metadata.file_path);
    }

    struct TrackAwareTranscriber {
        system_result: Result<TranscriptionOutput, AppError>,
    }

    impl transcription::Transcriber for TrackAwareTranscriber {
        fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
            if audio_path.to_string_lossy().contains(".system.") {
                return self.system_result.clone();
            }
            Ok(TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Mic point.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                }],
            })
        }
    }

    fn settings_without_echo_cancellation() -> ScribeSettings {
        ScribeSettings {
            enable_echo_cancellation: false,
            ..ScribeSettings::default()
        }
    }

    fn unused_echo_cancellation() -> StubEchoCancellation {
        StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/scribe/unused.aec.wav")),
        }
    }

    #[test]
    fn transcribe_meeting_tracks_merges_system_track_with_speaker_labels() {
        let metadata = audio_metadata_with_system_reference("meeting-dual-track");
        let transcriber = TrackAwareTranscriber {
            system_result: Ok(TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Remote point.".to_string(),
                    started_at_ms: 500,
                    ended_at_ms: 900,
                }],
            }),
        };

        let output = transcribe_meeting_tracks(
            &settings_without_echo_cancellation(),
            &metadata,
            &transcriber,
            &unused_echo_cancellation(),
        )
        .expect("dual-track transcription succeeds");

        assert_eq!(
            output
                .segments
                .iter()
                .map(|segment| {
                    (
                        segment.sequence_number,
                        segment.speaker_label.as_deref(),
                        segment.text.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (1, Some(transcription::USER_SPEAKER_LABEL), "Mic point."),
                (
                    2,
                    Some(transcription::OTHERS_SPEAKER_LABEL),
                    "Remote point."
                ),
            ]
        );
    }

    #[test]
    fn transcribe_meeting_tracks_keeps_mic_transcript_when_system_track_fails() {
        let metadata = audio_metadata_with_system_reference("meeting-dual-track-fallback");
        let transcriber = TrackAwareTranscriber {
            system_result: Err(AppError {
                code: "transcription_audio_conversion_failed".to_string(),
                message: "ffmpeg could not convert the audio file for transcription.".to_string(),
                details: None,
            }),
        };

        let output = transcribe_meeting_tracks(
            &settings_without_echo_cancellation(),
            &metadata,
            &transcriber,
            &unused_echo_cancellation(),
        )
        .expect("microphone transcript survives a system-track failure");

        assert_eq!(output.segments.len(), 1);
        assert_eq!(output.segments[0].text, "Mic point.");
        assert_eq!(output.segments[0].speaker_label, None);
    }

    #[test]
    fn transcribe_meeting_tracks_stays_mic_only_without_system_audio() {
        let metadata = AudioMetadata {
            system_audio_file_path: None,
            ..audio_metadata_with_system_reference("meeting-mic-only")
        };
        let transcriber = TrackAwareTranscriber {
            system_result: Err(AppError {
                code: "system_track_must_not_be_transcribed".to_string(),
                message: "The system track must not be requested without a recording.".to_string(),
                details: None,
            }),
        };

        let output = transcribe_meeting_tracks(
            &settings_without_echo_cancellation(),
            &metadata,
            &transcriber,
            &unused_echo_cancellation(),
        )
        .expect("microphone-only transcription succeeds");

        assert_eq!(output.segments.len(), 1);
        assert_eq!(output.segments[0].speaker_label, None);
    }

    #[test]
    fn select_transcription_audio_path_attempts_aec_for_m4a_system_audio() {
        let settings = ScribeSettings::default();
        let metadata = AudioMetadata {
            system_audio_file_path: Some("/tmp/scribe/meeting-aec-m4a.system.m4a".to_string()),
            ..audio_metadata_with_system_reference("meeting-aec-m4a")
        };
        let echo_cancellation = StubEchoCancellation {
            calls: AtomicU8::new(0),
            result: Ok(PathBuf::from("/tmp/scribe/meeting-aec-m4a.aec.wav")),
        };

        let selected_path =
            select_transcription_audio_path(&settings, &metadata, &echo_cancellation);

        assert_eq!(selected_path, "/tmp/scribe/meeting-aec-m4a.aec.wav");
        assert_eq!(echo_cancellation.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn apple_script_string_literal_escapes_notification_text() {
        assert_eq!(
            apple_script_string_literal("Score \"81\"\\100\nready"),
            "\"Score \\\"81\\\"\\\\100 ready\""
        );
    }

    #[cfg(desktop)]
    #[test]
    fn hotkey_shortcut_mapping_accepts_known_tokens_only() {
        assert!(super::hotkey_shortcut_for("cmd+shift+d").is_some());
        assert!(super::hotkey_shortcut_for("ctrl+option+d").is_some());
        assert!(super::hotkey_shortcut_for("cmd+shift+space").is_some());
        assert!(super::hotkey_shortcut_for("ctrl+option+p").is_some());
        // Unknown / unsafe tokens are rejected so the update command can validate.
        assert!(super::hotkey_shortcut_for("cmd+space").is_none());
        assert!(super::hotkey_shortcut_for("f5").is_none());
        assert!(super::hotkey_shortcut_for("").is_none());
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
            file_path: format!("/tmp/scribe/{meeting_id}.wav"),
            system_audio_file_path: Some(format!("/tmp/scribe/{meeting_id}.system.wav")),
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
}
