use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use analysis::{AnalysisTranscriptSegment, MeetingSummarizer, MeetingSummary, OllamaAnalyzer};
use audio::{
    aec::{EchoCancellationBackend, SpeexEchoCancellationBackend},
    storage::{safe_system_audio_path, safe_wav_path, validate_recording_file_stem},
    AudioDevice, CpalCaptureBackend, RecordingManager, RecordingMetadata, RecordingStarted,
    ScreenCaptureKitSystemAudioBackend,
};
use domain::{
    AnalyzerProvider, AppError, MeetingId, MeetingLifecycleState, ProcessingStage, ReportId,
    ResonanceSettings,
};
use nudges::{
    LiveNudgeEvent, LiveNudgePipeline, NudgeEventSink, NudgeTranscriptEventSink, LIVE_NUDGE_EVENT,
};
use path_detection::hydrate_settings_with_local_defaults;
use persistence::{
    AudioMetadata, CreateMeeting, CreateMetric, CreatePipelineFailure, CreateTranscriptSegment,
    MeetingHistoryRecord, MeetingTrendRecord, MetricRecord, PipelineFailureRecord, SqliteRepository,
};
use rules::{MetricsSummary, RuleTranscriptSegment};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use transcription::{
    TranscriptEventSink, TranscriptSegment, TranscriptStreamEvent, TranscriptStreamSummary,
    TranscriptionOutput, WhisperShellTranscriber, TRANSCRIPT_SEGMENT_EVENT,
    TRANSCRIPT_STREAM_COMPLETE_EVENT,
};

pub mod analysis;
pub mod audio;
pub mod domain;
pub mod media_import;
pub mod nudges;
pub mod path_detection;
pub mod persistence;
pub mod rules;
pub mod transcription;

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
    settings: ResonanceSettings,
    cleanup: RetentionCleanupSummary,
}

const DEFAULT_HISTORY_LIMIT: u32 = 10;
const MAX_HISTORY_LIMIT: u32 = 50;
const HISTORY_DETAIL_TRANSCRIPT_LIMIT: u32 = 200;
const DEFAULT_TRENDS_LIMIT: u32 = 12;
const MAX_TRENDS_LIMIT: u32 = 50;
const MAX_RAW_AUDIO_RETENTION_DAYS: u16 = 365;
const MILLIS_PER_DAY: u64 = 86_400_000;
const RESONANCE_DATABASE_FILE_NAME: &str = "resonance.sqlite3";
const LEGACY_APP_IDENTIFIER: &str = "com.orator.meetingcoach";
const LEGACY_APP_NAME: &str = "Orator";
const LEGACY_DATABASE_FILE_NAME: &str = "orator.sqlite3";

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
            start_recording,
            stop_recording,
            transcribe_meeting,
            calculate_metrics,
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

// Retained for the upcoming meeting-summary command (Phase 1 build).
#[allow(dead_code)]
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

// Retained for the upcoming meeting-summary command (Phase 1 build).
#[allow(dead_code)]
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
                .join("enrollment-sample.wav"),
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
                    .join("enrollment-sample.wav")
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
}
