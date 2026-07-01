use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::domain::{
    AnalyzerProvider, AppError, DictationSessionId, MeetingId, MetricId, PracticeAnnotationId,
    PracticeRecordingId, PracticeReviewReportId, ProcessingStage, ReportId, ResonanceSettings,
    Score, SegmentId, SummarizerProvider, SummaryId,
};

const CURRENT_SCHEMA_VERSION: i64 = 15;
const SETTINGS_ID: &str = "default";
const VOICE_PROFILE_ID: &str = "default";

/// SQLite-backed repository for local Resonance data.
pub struct SqliteRepository {
    connection: Connection,
}

/// Data needed to create a meeting row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMeeting {
    pub id: MeetingId,
    pub title: Option<String>,
    pub started_at_ms: u64,
    pub stopped_at_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Persisted meeting summary returned by repository reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRecord {
    pub id: MeetingId,
    pub title: Option<String>,
    pub started_at_ms: u64,
    pub stopped_at_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Data needed to create a dictation session summary row. Deliberately carries no
/// transcript text — only counts and timing, since dictation may include
/// sensitive content the user never asked to have logged.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateDictationSession {
    pub id: DictationSessionId,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    pub word_count: u32,
    pub words_per_minute: f64,
    pub created_at_ms: u64,
}

/// Persisted dictation session summary returned by repository reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSessionRecord {
    pub id: DictationSessionId,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    pub word_count: u32,
    pub words_per_minute: f64,
    pub created_at_ms: u64,
}

/// Aggregate dictation stats across all persisted sessions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationStatsSummary {
    pub total_sessions: u32,
    pub total_words: u64,
    pub average_words_per_minute: f64,
    pub total_duration_ms: u64,
}

/// Data needed to create a transcript segment row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTranscriptSegment {
    pub id: SegmentId,
    pub meeting_id: MeetingId,
    pub sequence_number: u32,
    pub speaker_label: Option<String>,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub created_at_ms: u64,
}

/// Persisted transcript segment returned by repository reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSegmentRecord {
    pub id: SegmentId,
    pub meeting_id: MeetingId,
    pub sequence_number: u32,
    pub speaker_label: Option<String>,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub created_at_ms: u64,
}

/// Data needed to create a metric row.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMetric {
    pub id: MetricId,
    pub meeting_id: MeetingId,
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub created_at_ms: u64,
}

/// Persisted metric returned by repository reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRecord {
    pub id: MetricId,
    pub meeting_id: MeetingId,
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub created_at_ms: u64,
}

/// Data needed to create a report row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReport {
    pub id: ReportId,
    pub meeting_id: MeetingId,
    pub overall_score: Score,
    pub body_json: String,
    pub generated_at_ms: u64,
}

/// Persisted report returned by repository reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRecord {
    pub id: ReportId,
    pub meeting_id: MeetingId,
    pub overall_score: Score,
    pub body_json: String,
    pub generated_at_ms: u64,
}

/// Data needed to store a summary for an imported recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateImportedMeetingSummary {
    pub id: SummaryId,
    pub meeting_id: MeetingId,
    pub source_file_path: String,
    pub extracted_audio_file_path: String,
    pub speaking_improvements_source: String,
    pub body_json: String,
    pub generated_at_ms: u64,
}

/// Persisted imported-recording summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedMeetingSummaryRecord {
    pub id: SummaryId,
    pub meeting_id: MeetingId,
    pub source_file_path: String,
    pub extracted_audio_file_path: String,
    pub speaking_improvements_source: String,
    pub body_json: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingSummaryRecord {
    pub body_json: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePracticeRecording {
    pub id: PracticeRecordingId,
    pub title: Option<String>,
    pub source_kind: String,
    pub video_file_path: String,
    pub extracted_audio_file_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub byte_size: Option<u64>,
    pub recorded_at_ms: u64,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub analysis_status: String,
    pub cloud_video_used: bool,
    pub pipeline_failure_code: Option<String>,
    pub pipeline_failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeRecordingRecord {
    pub id: PracticeRecordingId,
    pub title: Option<String>,
    pub source_kind: String,
    pub video_file_path: String,
    pub extracted_audio_file_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub byte_size: Option<u64>,
    pub recorded_at_ms: u64,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub analysis_status: String,
    pub cloud_video_used: bool,
    pub pipeline_failure_code: Option<String>,
    pub pipeline_failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePracticeReviewReport {
    pub id: PracticeReviewReportId,
    pub practice_recording_id: PracticeRecordingId,
    pub overall_score: Option<Score>,
    pub audio_score: Option<Score>,
    pub visual_score: Option<Score>,
    pub body_json: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeReviewReportRecord {
    pub id: PracticeReviewReportId,
    pub practice_recording_id: PracticeRecordingId,
    pub overall_score: Option<Score>,
    pub audio_score: Option<Score>,
    pub visual_score: Option<Score>,
    pub body_json: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePracticeTimelineAnnotation {
    pub id: PracticeAnnotationId,
    pub practice_recording_id: PracticeRecordingId,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub category: String,
    pub severity: String,
    pub evidence: String,
    pub suggestion: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeTimelineAnnotationRecord {
    pub id: PracticeAnnotationId,
    pub practice_recording_id: PracticeRecordingId,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub category: String,
    pub severity: String,
    pub evidence: String,
    pub suggestion: String,
    pub source: String,
}

/// Persisted local voice enrollment metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfileRecord {
    pub sample_audio_file_path: String,
    pub sample_duration_ms: Option<u64>,
    pub sample_byte_size: u64,
    pub enrolled_at_ms: u64,
    pub embedding_json: Option<String>,
    pub embedding_dimension: Option<u32>,
    pub embedding_model_path: Option<String>,
    pub embedding_computed_at_ms: Option<u64>,
}

/// Persisted audio metadata for a meeting recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMetadata {
    pub meeting_id: MeetingId,
    pub file_path: String,
    pub system_audio_file_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub sample_rate_hz: Option<u32>,
    pub byte_size: Option<u64>,
    pub system_audio_byte_size: Option<u64>,
    pub system_audio_stream_error: Option<String>,
    pub created_at_ms: u64,
}

/// Data needed to persist the latest pipeline failure for a meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePipelineFailure {
    pub meeting_id: MeetingId,
    pub failed_stage: ProcessingStage,
    pub error_code: String,
    pub error_message: String,
    pub error_details: Option<String>,
    pub failed_at_ms: u64,
}

/// Persisted latest pipeline failure for a meeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineFailureRecord {
    pub meeting_id: MeetingId,
    pub failed_stage: ProcessingStage,
    pub error_code: String,
    pub error_message: String,
    pub error_details: Option<String>,
    pub failed_at_ms: u64,
}

/// Summary row for paginated meeting history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingHistoryRecord {
    pub id: MeetingId,
    pub title: Option<String>,
    pub started_at_ms: u64,
    pub stopped_at_ms: Option<u64>,
    pub updated_at_ms: u64,
    pub duration_ms: Option<u64>,
    pub audio_file_path: Option<String>,
    pub report_id: Option<ReportId>,
    pub overall_score: Option<Score>,
    pub report_generated_at_ms: Option<u64>,
    pub transcript_segment_count: u32,
    pub pipeline_failure: Option<PipelineFailureRecord>,
}

/// One meeting-level datapoint for trend charts.
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingTrendRecord {
    pub id: MeetingId,
    pub title: Option<String>,
    pub started_at_ms: u64,
    pub filler_word_count: Option<f64>,
    pub words_per_minute: Option<f64>,
    pub overall_score: Option<Score>,
}

impl SqliteRepository {
    /// Opens a database file and applies all idempotent migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(map_db_error)?;
        configure_connection(&connection)?;
        run_migrations(&connection)?;

        Ok(Self { connection })
    }

    /// Returns the highest applied schema version.
    pub fn schema_version(&self) -> Result<i64, AppError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_versions",
                [],
                |row| row.get(0),
            )
            .map_err(map_db_error)
    }

    /// Inserts a meeting and returns the persisted row.
    pub fn create_meeting(&self, meeting: &CreateMeeting) -> Result<MeetingRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO meetings (
                    id,
                    title,
                    started_at_ms,
                    stopped_at_ms,
                    created_at_ms,
                    updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    meeting.id.as_str(),
                    meeting.title.as_deref(),
                    to_db_i64(meeting.started_at_ms)?,
                    optional_to_db_i64(meeting.stopped_at_ms)?,
                    to_db_i64(meeting.created_at_ms)?,
                    to_db_i64(meeting.updated_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        self.get_meeting(&meeting.id)?.ok_or_else(|| {
            persistence_error(
                "meeting_not_found",
                "Created meeting could not be read back.",
                None,
            )
        })
    }

    /// Returns a meeting by id, or `None` when it does not exist.
    pub fn get_meeting(&self, id: &MeetingId) -> Result<Option<MeetingRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT id, title, started_at_ms, stopped_at_ms, created_at_ms, updated_at_ms
                FROM meetings
                WHERE id = ?1",
                params![id.as_str()],
                read_meeting,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Inserts a dictation session summary and returns the persisted row.
    pub fn create_dictation_session(
        &self,
        session: &CreateDictationSession,
    ) -> Result<DictationSessionRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO dictation_sessions (
                    id,
                    started_at_ms,
                    ended_at_ms,
                    duration_ms,
                    word_count,
                    words_per_minute,
                    created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session.id.as_str(),
                    to_db_i64(session.started_at_ms)?,
                    to_db_i64(session.ended_at_ms)?,
                    to_db_i64(session.duration_ms)?,
                    session.word_count,
                    session.words_per_minute,
                    to_db_i64(session.created_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        Ok(DictationSessionRecord {
            id: session.id.clone(),
            started_at_ms: session.started_at_ms,
            ended_at_ms: session.ended_at_ms,
            duration_ms: session.duration_ms,
            word_count: session.word_count,
            words_per_minute: session.words_per_minute,
            created_at_ms: session.created_at_ms,
        })
    }

    /// Lists dictation session summaries newest-first with bounded pagination.
    pub fn list_dictation_sessions(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DictationSessionRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, started_at_ms, ended_at_ms, duration_ms, word_count, words_per_minute, created_at_ms
                FROM dictation_sessions
                ORDER BY started_at_ms DESC, id ASC
                LIMIT ?1 OFFSET ?2",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(
                params![i64::from(limit), i64::from(offset)],
                read_dictation_session,
            )
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Returns aggregate dictation stats across all persisted sessions.
    pub fn get_dictation_stats_summary(&self) -> Result<DictationStatsSummary, AppError> {
        self.connection
            .query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(word_count), 0),
                    COALESCE(AVG(words_per_minute), 0.0),
                    COALESCE(SUM(duration_ms), 0)
                FROM dictation_sessions",
                [],
                |row| {
                    Ok(DictationStatsSummary {
                        total_sessions: row.get::<_, i64>(0)? as u32,
                        total_words: row.get::<_, i64>(1)? as u64,
                        average_words_per_minute: row.get(2)?,
                        total_duration_ms: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .map_err(map_db_error)
    }

    /// Deletes a single dictation session summary row. No files on disk are
    /// tied to a dictation session — it's stats-only.
    pub fn delete_dictation_session(&self, id: &DictationSessionId) -> Result<bool, AppError> {
        self.connection
            .execute("DELETE FROM dictation_sessions WHERE id = ?1", params![id.as_str()])
            .map(|deleted_rows| deleted_rows > 0)
            .map_err(map_db_error)
    }

    /// Records the stop time for a meeting and returns the updated row.
    pub fn mark_meeting_stopped(
        &self,
        id: &MeetingId,
        stopped_at_ms: u64,
        updated_at_ms: u64,
    ) -> Result<MeetingRecord, AppError> {
        let changed = self
            .connection
            .execute(
                "UPDATE meetings
                SET stopped_at_ms = ?2, updated_at_ms = ?3
                WHERE id = ?1",
                params![
                    id.as_str(),
                    to_db_i64(stopped_at_ms)?,
                    to_db_i64(updated_at_ms)?,
                ],
            )
            .map_err(map_report_create_error)?;

        if changed == 0 {
            return Err(persistence_error(
                "meeting_not_found",
                "Meeting could not be marked as stopped because it does not exist.",
                Some(format!("meeting_id={}", id.as_str())),
            ));
        }

        self.get_meeting(id)?.ok_or_else(|| {
            persistence_error(
                "meeting_not_found",
                "Updated meeting could not be read back.",
                Some(format!("meeting_id={}", id.as_str())),
            )
        })
    }

    /// Overwrites a meeting's title (manual rename). Passing `None` or an
    /// empty title clears it, reverting the meeting to its date-based
    /// display name in the UI.
    pub fn update_meeting_title(
        &self,
        id: &MeetingId,
        title: Option<&str>,
        updated_at_ms: u64,
    ) -> Result<(), AppError> {
        let changed = self
            .connection
            .execute(
                "UPDATE meetings SET title = ?2, updated_at_ms = ?3 WHERE id = ?1",
                params![id.as_str(), title, to_db_i64(updated_at_ms)?],
            )
            .map_err(map_report_create_error)?;

        if changed == 0 {
            return Err(persistence_error(
                "meeting_not_found",
                "Meeting could not be renamed because it does not exist.",
                Some(format!("meeting_id={}", id.as_str())),
            ));
        }
        Ok(())
    }

    /// Fills in a meeting's title from a model-generated suggestion, but only
    /// when it doesn't already have one — so this never overwrites a manual
    /// rename or a title set by an earlier summarization.
    pub fn set_meeting_title_if_absent(
        &self,
        id: &MeetingId,
        title: &str,
        updated_at_ms: u64,
    ) -> Result<(), AppError> {
        self.connection
            .execute(
                "UPDATE meetings SET title = ?2, updated_at_ms = ?3
                WHERE id = ?1 AND (title IS NULL OR trim(title) = '')",
                params![id.as_str(), title, to_db_i64(updated_at_ms)?],
            )
            .map_err(map_report_create_error)?;
        Ok(())
    }

    /// Stores the latest failed pipeline stage for a meeting.
    pub fn upsert_pipeline_failure(
        &self,
        failure: &CreatePipelineFailure,
    ) -> Result<PipelineFailureRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO pipeline_failures (
                    meeting_id,
                    failed_stage,
                    error_code,
                    error_message,
                    error_details,
                    failed_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(meeting_id) DO UPDATE SET
                    failed_stage = excluded.failed_stage,
                    error_code = excluded.error_code,
                    error_message = excluded.error_message,
                    error_details = excluded.error_details,
                    failed_at_ms = excluded.failed_at_ms",
                params![
                    failure.meeting_id.as_str(),
                    processing_stage_to_db(failure.failed_stage),
                    failure.error_code.as_str(),
                    failure.error_message.as_str(),
                    failure.error_details.as_deref(),
                    to_db_i64(failure.failed_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        self.get_pipeline_failure(&failure.meeting_id)?
            .ok_or_else(|| {
                persistence_error(
                    "pipeline_failure_not_found",
                    "Stored pipeline failure could not be read back.",
                    Some(format!("meeting_id={}", failure.meeting_id.as_str())),
                )
            })
    }

    /// Clears the latest failed pipeline stage after a later retry succeeds.
    pub fn clear_pipeline_failure(&self, meeting_id: &MeetingId) -> Result<(), AppError> {
        self.connection
            .execute(
                "DELETE FROM pipeline_failures WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
            )
            .map_err(map_db_error)?;
        Ok(())
    }

    /// Returns the latest pipeline failure for a meeting, if one exists.
    pub fn get_pipeline_failure(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Option<PipelineFailureRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    meeting_id,
                    failed_stage,
                    error_code,
                    error_message,
                    error_details,
                    failed_at_ms
                FROM pipeline_failures
                WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
                read_pipeline_failure,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Lists meetings newest-first with deterministic tie-breakers.
    pub fn list_meetings(&self) -> Result<Vec<MeetingRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, title, started_at_ms, stopped_at_ms, created_at_ms, updated_at_ms
                FROM meetings
                ORDER BY updated_at_ms DESC, started_at_ms DESC, id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map([], read_meeting)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Lists meeting history rows newest-first with bounded pagination.
    pub fn list_meeting_history(
        &self,
        search_query: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MeetingHistoryRecord>, AppError> {
        let search_pattern = search_query.map(like_contains_pattern);
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    m.id,
                    m.title,
                    m.started_at_ms,
                    m.stopped_at_ms,
                    m.updated_at_ms,
                    a.duration_ms,
                    a.file_path,
                    r.id,
                    r.overall_score,
                    r.generated_at_ms,
                    COUNT(ts.id),
                    pf.failed_stage,
                    pf.error_code,
                    pf.error_message,
                    pf.error_details,
                    pf.failed_at_ms
                FROM meetings m
                LEFT JOIN audio_metadata a ON a.meeting_id = m.id
                LEFT JOIN reports r ON r.meeting_id = m.id
                LEFT JOIN transcript_segments ts ON ts.meeting_id = m.id
                LEFT JOIN pipeline_failures pf ON pf.meeting_id = m.id
                WHERE ?1 IS NULL
                    OR m.id LIKE ?1 ESCAPE '\\'
                    OR COALESCE(m.title, '') LIKE ?1 ESCAPE '\\'
                    OR EXISTS (
                        SELECT 1
                        FROM transcript_segments search_ts
                        WHERE search_ts.meeting_id = m.id
                            AND search_ts.text LIKE ?1 ESCAPE '\\'
                    )
                GROUP BY
                    m.id,
                    m.title,
                    m.started_at_ms,
                    m.stopped_at_ms,
                    m.updated_at_ms,
                    a.duration_ms,
                    a.file_path,
                    r.id,
                    r.overall_score,
                    r.generated_at_ms,
                    pf.failed_stage,
                    pf.error_code,
                    pf.error_message,
                    pf.error_details,
                    pf.failed_at_ms
                ORDER BY m.updated_at_ms DESC, m.started_at_ms DESC, m.id ASC
                LIMIT ?2 OFFSET ?3",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(
                params![
                    search_pattern.as_deref(),
                    i64::from(limit),
                    i64::from(offset)
                ],
                read_meeting_history,
            )
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Lists recent meeting-level trend datapoints newest-first.
    pub fn list_meeting_trends(&self, limit: u32) -> Result<Vec<MeetingTrendRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    m.id,
                    m.title,
                    m.started_at_ms,
                    MAX(CASE WHEN metrics.name = 'filler_word_count' THEN metrics.value END),
                    MAX(CASE WHEN metrics.name = 'words_per_minute' THEN metrics.value END),
                    latest_reports.overall_score
                FROM meetings m
                LEFT JOIN metrics ON metrics.meeting_id = m.id
                LEFT JOIN (
                    SELECT report_rows.meeting_id, report_rows.overall_score
                    FROM reports report_rows
                    WHERE report_rows.generated_at_ms = (
                        SELECT MAX(newest.generated_at_ms)
                        FROM reports newest
                        WHERE newest.meeting_id = report_rows.meeting_id
                    )
                ) latest_reports ON latest_reports.meeting_id = m.id
                GROUP BY
                    m.id,
                    m.title,
                    m.started_at_ms,
                    m.updated_at_ms,
                    latest_reports.overall_score
                ORDER BY m.updated_at_ms DESC, m.started_at_ms DESC, m.id ASC
                LIMIT ?1",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![i64::from(limit)], read_meeting_trend)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Inserts transcript segments in a single transaction and returns the persisted rows.
    pub fn create_transcript_segments(
        &self,
        segments: &[CreateTranscriptSegment],
    ) -> Result<Vec<TranscriptSegmentRecord>, AppError> {
        if segments.is_empty() {
            return Ok(Vec::new());
        }

        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(map_db_error)?;
        for segment in segments {
            transaction
                .execute(
                    "INSERT INTO transcript_segments (
                        id,
                        meeting_id,
                        sequence_number,
                        speaker_label,
                        text,
                        started_at_ms,
                        ended_at_ms,
                        created_at_ms
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        segment.id.as_str(),
                        segment.meeting_id.as_str(),
                        i64::from(segment.sequence_number),
                        segment.speaker_label.as_deref(),
                        segment.text.as_str(),
                        to_db_i64(segment.started_at_ms)?,
                        to_db_i64(segment.ended_at_ms)?,
                        to_db_i64(segment.created_at_ms)?,
                    ],
                )
                .map_err(map_db_error)?;
        }
        transaction.commit().map_err(map_db_error)?;

        Ok(segments.iter().map(TranscriptSegmentRecord::from).collect())
    }

    /// Lists transcript segments for a meeting in deterministic sequence order.
    pub fn list_transcript_segments(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Vec<TranscriptSegmentRecord>, AppError> {
        self.list_transcript_segments_page(meeting_id, u32::MAX, 0)
    }

    /// Lists a bounded page of transcript segments for a meeting in deterministic sequence order.
    pub fn list_transcript_segments_page(
        &self,
        meeting_id: &MeetingId,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TranscriptSegmentRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    id,
                    meeting_id,
                    sequence_number,
                    speaker_label,
                    text,
                    started_at_ms,
                    ended_at_ms,
                    created_at_ms
                FROM transcript_segments
                WHERE meeting_id = ?1
                ORDER BY sequence_number ASC, id ASC
                LIMIT ?2 OFFSET ?3",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(
                params![meeting_id.as_str(), i64::from(limit), i64::from(offset)],
                read_transcript_segment,
            )
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Counts transcript segments for a meeting without loading transcript text.
    pub fn count_transcript_segments(&self, meeting_id: &MeetingId) -> Result<u32, AppError> {
        self.connection
            .query_row(
                "SELECT COUNT(*)
                FROM transcript_segments
                WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
                |row| from_db_u32(row.get(0)?, 0),
            )
            .map_err(map_db_error)
    }

    /// Inserts a metric and returns the persisted row.
    pub fn create_metric(&self, metric: &CreateMetric) -> Result<MetricRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO metrics (
                    id,
                    meeting_id,
                    name,
                    value,
                    unit,
                    created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    metric.id.as_str(),
                    metric.meeting_id.as_str(),
                    metric.name.as_str(),
                    metric.value,
                    metric.unit.as_deref(),
                    to_db_i64(metric.created_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        Ok(MetricRecord::from(metric))
    }

    /// Lists metrics for a meeting oldest-first for trend/history consumers.
    pub fn list_metrics(&self, meeting_id: &MeetingId) -> Result<Vec<MetricRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, meeting_id, name, value, unit, created_at_ms
                FROM metrics
                WHERE meeting_id = ?1
                ORDER BY created_at_ms ASC, id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![meeting_id.as_str()], read_metric)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Inserts a report and returns the persisted row.
    pub fn create_report(&self, report: &CreateReport) -> Result<ReportRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO reports (
                    id,
                    meeting_id,
                    overall_score,
                    body_json,
                    generated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    report.id.as_str(),
                    report.meeting_id.as_str(),
                    i64::from(report.overall_score.value()),
                    report.body_json.as_str(),
                    to_db_i64(report.generated_at_ms)?,
                ],
            )
            .map_err(map_report_create_error)?;

        self.get_report(&report.id)?.ok_or_else(|| {
            persistence_error(
                "report_not_found",
                "Created report could not be read back.",
                None,
            )
        })
    }

    /// Returns a report by id, or `None` when it does not exist.
    pub fn get_report(&self, id: &ReportId) -> Result<Option<ReportRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT id, meeting_id, overall_score, body_json, generated_at_ms
                FROM reports
                WHERE id = ?1",
                params![id.as_str()],
                read_report,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Lists reports for a meeting.
    pub fn list_reports_for_meeting(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Vec<ReportRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, meeting_id, overall_score, body_json, generated_at_ms
                FROM reports
                WHERE meeting_id = ?1
                ORDER BY generated_at_ms DESC, id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![meeting_id.as_str()], read_report)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Lists recent reports newest-first.
    pub fn list_recent_reports(&self, limit: u32) -> Result<Vec<ReportRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, meeting_id, overall_score, body_json, generated_at_ms
                FROM reports
                ORDER BY generated_at_ms DESC, id ASC
                LIMIT ?1",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![i64::from(limit)], read_report)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Stores one imported-recording summary for a meeting.
    pub fn create_imported_meeting_summary(
        &self,
        summary: &CreateImportedMeetingSummary,
    ) -> Result<ImportedMeetingSummaryRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO imported_meeting_summaries (
                    id,
                    meeting_id,
                    source_file_path,
                    extracted_audio_file_path,
                    speaking_improvements_source,
                    body_json,
                    generated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    summary.id.as_str(),
                    summary.meeting_id.as_str(),
                    summary.source_file_path.as_str(),
                    summary.extracted_audio_file_path.as_str(),
                    summary.speaking_improvements_source.as_str(),
                    summary.body_json.as_str(),
                    to_db_i64(summary.generated_at_ms)?,
                ],
            )
            .map_err(map_report_create_error)?;

        self.get_imported_meeting_summary(&summary.id)?
            .ok_or_else(|| {
                persistence_error(
                    "imported_summary_not_found",
                    "Created imported recording summary could not be read back.",
                    None,
                )
            })
    }

    /// Returns an imported-recording summary by id.
    pub fn get_imported_meeting_summary(
        &self,
        id: &SummaryId,
    ) -> Result<Option<ImportedMeetingSummaryRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    id,
                    meeting_id,
                    source_file_path,
                    extracted_audio_file_path,
                    COALESCE(speaking_improvements_source, 'none'),
                    body_json,
                    generated_at_ms
                FROM imported_meeting_summaries
                WHERE id = ?1",
                params![id.as_str()],
                read_imported_meeting_summary,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Returns the imported-recording summary for a meeting, if one exists.
    pub fn get_imported_meeting_summary_for_meeting(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Option<ImportedMeetingSummaryRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    id,
                    meeting_id,
                    source_file_path,
                    extracted_audio_file_path,
                    COALESCE(speaking_improvements_source, 'none'),
                    body_json,
                    generated_at_ms
                FROM imported_meeting_summaries
                WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
                read_imported_meeting_summary,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Inserts or replaces the on-device notes for a recorded meeting.
    pub fn upsert_meeting_summary(
        &self,
        meeting_id: &MeetingId,
        body_json: &str,
        generated_at_ms: u64,
    ) -> Result<(), AppError> {
        self.connection
            .execute(
                "INSERT INTO meeting_summaries (meeting_id, body_json, generated_at_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(meeting_id) DO UPDATE SET
                    body_json = excluded.body_json,
                    generated_at_ms = excluded.generated_at_ms",
                params![meeting_id.as_str(), body_json, to_db_i64(generated_at_ms)?],
            )
            .map_err(map_db_error)?;
        Ok(())
    }

    /// Returns the stored notes for a recorded meeting, if any.
    pub fn get_meeting_summary(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Option<MeetingSummaryRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT body_json, generated_at_ms FROM meeting_summaries WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
                |row| {
                    Ok(MeetingSummaryRecord {
                        body_json: row.get(0)?,
                        generated_at_ms: from_db_u64(row.get(1)?)?,
                    })
                },
            )
            .optional()
            .map_err(map_db_error)
    }

    pub fn create_practice_recording(
        &self,
        recording: &CreatePracticeRecording,
    ) -> Result<PracticeRecordingRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO practice_recordings (
                    id,
                    title,
                    source_kind,
                    video_file_path,
                    extracted_audio_file_path,
                    duration_ms,
                    byte_size,
                    recorded_at_ms,
                    created_at_ms,
                    updated_at_ms,
                    analysis_status,
                    cloud_video_used,
                    pipeline_failure_code,
                    pipeline_failure_message
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    recording.id.as_str(),
                    recording.title.as_deref(),
                    recording.source_kind.as_str(),
                    recording.video_file_path.as_str(),
                    recording.extracted_audio_file_path.as_deref(),
                    optional_to_db_i64(recording.duration_ms)?,
                    optional_to_db_i64(recording.byte_size)?,
                    to_db_i64(recording.recorded_at_ms)?,
                    to_db_i64(recording.created_at_ms)?,
                    to_db_i64(recording.updated_at_ms)?,
                    recording.analysis_status.as_str(),
                    bool_to_db(recording.cloud_video_used),
                    recording.pipeline_failure_code.as_deref(),
                    recording.pipeline_failure_message.as_deref(),
                ],
            )
            .map_err(map_db_error)?;

        self.get_practice_recording(&recording.id)?.ok_or_else(|| {
            persistence_error(
                "practice_recording_not_found",
                "Created practice recording could not be read back.",
                Some(format!("practice_recording_id={}", recording.id.as_str())),
            )
        })
    }

    pub fn get_practice_recording(
        &self,
        id: &PracticeRecordingId,
    ) -> Result<Option<PracticeRecordingRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    id,
                    title,
                    source_kind,
                    video_file_path,
                    extracted_audio_file_path,
                    duration_ms,
                    byte_size,
                    recorded_at_ms,
                    created_at_ms,
                    updated_at_ms,
                    analysis_status,
                    cloud_video_used,
                    pipeline_failure_code,
                    pipeline_failure_message
                FROM practice_recordings
                WHERE id = ?1",
                params![id.as_str()],
                read_practice_recording,
            )
            .optional()
            .map_err(map_db_error)
    }

    pub fn list_practice_recordings(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<PracticeRecordingRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    id,
                    title,
                    source_kind,
                    video_file_path,
                    extracted_audio_file_path,
                    duration_ms,
                    byte_size,
                    recorded_at_ms,
                    created_at_ms,
                    updated_at_ms,
                    analysis_status,
                    cloud_video_used,
                    pipeline_failure_code,
                    pipeline_failure_message
                FROM practice_recordings
                ORDER BY updated_at_ms DESC, recorded_at_ms DESC, id ASC
                LIMIT ?1 OFFSET ?2",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(
                params![i64::from(limit), i64::from(offset)],
                read_practice_recording,
            )
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    pub fn list_practice_recordings_before(
        &self,
        cutoff_ms: u64,
    ) -> Result<Vec<PracticeRecordingRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    id,
                    title,
                    source_kind,
                    video_file_path,
                    extracted_audio_file_path,
                    duration_ms,
                    byte_size,
                    recorded_at_ms,
                    created_at_ms,
                    updated_at_ms,
                    analysis_status,
                    cloud_video_used,
                    pipeline_failure_code,
                    pipeline_failure_message
                FROM practice_recordings
                WHERE created_at_ms <= ?1
                ORDER BY created_at_ms ASC, id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![to_db_i64(cutoff_ms)?], read_practice_recording)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    pub fn update_practice_recording_analysis_state(
        &self,
        id: &PracticeRecordingId,
        extracted_audio_file_path: Option<&str>,
        analysis_status: &str,
        cloud_video_used: bool,
        failure: Option<(&str, &str)>,
        updated_at_ms: u64,
    ) -> Result<PracticeRecordingRecord, AppError> {
        let (failure_code, failure_message) = failure
            .map(|(code, message)| (Some(code), Some(message)))
            .unwrap_or((None, None));
        let changed = self
            .connection
            .execute(
                "UPDATE practice_recordings
                SET extracted_audio_file_path = COALESCE(?2, extracted_audio_file_path),
                    analysis_status = ?3,
                    cloud_video_used = ?4,
                    pipeline_failure_code = ?5,
                    pipeline_failure_message = ?6,
                    updated_at_ms = ?7
                WHERE id = ?1",
                params![
                    id.as_str(),
                    extracted_audio_file_path,
                    analysis_status,
                    bool_to_db(cloud_video_used),
                    failure_code,
                    failure_message,
                    to_db_i64(updated_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        if changed == 0 {
            return Err(persistence_error(
                "practice_recording_not_found",
                "Practice recording could not be updated because it does not exist.",
                Some(format!("practice_recording_id={}", id.as_str())),
            ));
        }

        self.get_practice_recording(id)?.ok_or_else(|| {
            persistence_error(
                "practice_recording_not_found",
                "Updated practice recording could not be read back.",
                Some(format!("practice_recording_id={}", id.as_str())),
            )
        })
    }

    pub fn create_practice_review_report(
        &self,
        report: &CreatePracticeReviewReport,
    ) -> Result<PracticeReviewReportRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO practice_review_reports (
                    id,
                    practice_recording_id,
                    overall_score,
                    audio_score,
                    visual_score,
                    body_json,
                    generated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(practice_recording_id) DO UPDATE SET
                    id = excluded.id,
                    overall_score = excluded.overall_score,
                    audio_score = excluded.audio_score,
                    visual_score = excluded.visual_score,
                    body_json = excluded.body_json,
                    generated_at_ms = excluded.generated_at_ms",
                params![
                    report.id.as_str(),
                    report.practice_recording_id.as_str(),
                    report.overall_score.map(|score| i64::from(score.value())),
                    report.audio_score.map(|score| i64::from(score.value())),
                    report.visual_score.map(|score| i64::from(score.value())),
                    report.body_json.as_str(),
                    to_db_i64(report.generated_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        self.get_practice_review_report_for_recording(&report.practice_recording_id)?
            .ok_or_else(|| {
                persistence_error(
                    "practice_review_report_not_found",
                    "Saved practice review report could not be read back.",
                    Some(format!(
                        "practice_recording_id={}",
                        report.practice_recording_id.as_str()
                    )),
                )
            })
    }

    pub fn get_practice_review_report_for_recording(
        &self,
        practice_recording_id: &PracticeRecordingId,
    ) -> Result<Option<PracticeReviewReportRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    id,
                    practice_recording_id,
                    overall_score,
                    audio_score,
                    visual_score,
                    body_json,
                    generated_at_ms
                FROM practice_review_reports
                WHERE practice_recording_id = ?1",
                params![practice_recording_id.as_str()],
                read_practice_review_report,
            )
            .optional()
            .map_err(map_db_error)
    }

    pub fn replace_practice_timeline_annotations(
        &self,
        practice_recording_id: &PracticeRecordingId,
        annotations: &[CreatePracticeTimelineAnnotation],
    ) -> Result<Vec<PracticeTimelineAnnotationRecord>, AppError> {
        self.connection
            .execute(
                "DELETE FROM practice_timeline_annotations WHERE practice_recording_id = ?1",
                params![practice_recording_id.as_str()],
            )
            .map_err(map_db_error)?;

        for annotation in annotations {
            self.connection
                .execute(
                    "INSERT INTO practice_timeline_annotations (
                        id,
                        practice_recording_id,
                        started_at_ms,
                        ended_at_ms,
                        category,
                        severity,
                        evidence,
                        suggestion,
                        source
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        annotation.id.as_str(),
                        annotation.practice_recording_id.as_str(),
                        to_db_i64(annotation.started_at_ms)?,
                        to_db_i64(annotation.ended_at_ms)?,
                        annotation.category.as_str(),
                        annotation.severity.as_str(),
                        annotation.evidence.as_str(),
                        annotation.suggestion.as_str(),
                        annotation.source.as_str(),
                    ],
                )
                .map_err(map_db_error)?;
        }

        self.list_practice_timeline_annotations(practice_recording_id)
    }

    pub fn list_practice_timeline_annotations(
        &self,
        practice_recording_id: &PracticeRecordingId,
    ) -> Result<Vec<PracticeTimelineAnnotationRecord>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    id,
                    practice_recording_id,
                    started_at_ms,
                    ended_at_ms,
                    category,
                    severity,
                    evidence,
                    suggestion,
                    source
                FROM practice_timeline_annotations
                WHERE practice_recording_id = ?1
                ORDER BY started_at_ms ASC, ended_at_ms ASC, id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(
                params![practice_recording_id.as_str()],
                read_practice_timeline_annotation,
            )
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Rewrites app-data-owned file paths after a product app-data directory migration.
    pub fn rewrite_app_data_file_paths(
        &self,
        old_app_data_dir: &str,
        new_app_data_dir: &str,
    ) -> Result<(), AppError> {
        let like_pattern = like_prefix_pattern(old_app_data_dir);
        for sql in [
            "UPDATE audio_metadata
             SET file_path = replace(file_path, ?1, ?2)
             WHERE file_path LIKE ?3 ESCAPE '\\'",
            "UPDATE audio_metadata
             SET system_audio_file_path = replace(system_audio_file_path, ?1, ?2)
             WHERE system_audio_file_path LIKE ?3 ESCAPE '\\'",
            "UPDATE voice_profiles
             SET sample_audio_file_path = replace(sample_audio_file_path, ?1, ?2)
             WHERE sample_audio_file_path LIKE ?3 ESCAPE '\\'",
            "UPDATE imported_meeting_summaries
             SET extracted_audio_file_path = replace(extracted_audio_file_path, ?1, ?2)
             WHERE extracted_audio_file_path LIKE ?3 ESCAPE '\\'",
        ] {
            self.connection
                .execute(
                    sql,
                    params![old_app_data_dir, new_app_data_dir, like_pattern],
                )
                .map_err(map_db_error)?;
        }
        Ok(())
    }

    /// Inserts or updates audio metadata for a meeting recording.
    pub fn upsert_audio_metadata(
        &self,
        metadata: &AudioMetadata,
    ) -> Result<AudioMetadata, AppError> {
        self.connection
            .execute(
                "INSERT INTO audio_metadata (
                    meeting_id,
                    file_path,
                    system_audio_file_path,
                    duration_ms,
                    sample_rate_hz,
                    byte_size,
                    system_audio_byte_size,
                    system_audio_stream_error,
                    created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(meeting_id) DO UPDATE SET
                    file_path = excluded.file_path,
                    system_audio_file_path = excluded.system_audio_file_path,
                    duration_ms = excluded.duration_ms,
                    sample_rate_hz = excluded.sample_rate_hz,
                    byte_size = excluded.byte_size,
                    system_audio_byte_size = excluded.system_audio_byte_size,
                    system_audio_stream_error = excluded.system_audio_stream_error,
                    created_at_ms = excluded.created_at_ms",
                params![
                    metadata.meeting_id.as_str(),
                    metadata.file_path.as_str(),
                    metadata.system_audio_file_path.as_deref(),
                    optional_to_db_i64(metadata.duration_ms)?,
                    metadata.sample_rate_hz.map(i64::from),
                    optional_to_db_i64(metadata.byte_size)?,
                    optional_to_db_i64(metadata.system_audio_byte_size)?,
                    metadata.system_audio_stream_error.as_deref(),
                    to_db_i64(metadata.created_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        self.get_audio_metadata(&metadata.meeting_id)?
            .ok_or_else(|| {
                persistence_error(
                    "audio_metadata_not_found",
                    "Upserted audio metadata could not be read back.",
                    None,
                )
            })
    }

    /// Returns audio metadata by meeting id, or `None` when it does not exist.
    pub fn get_audio_metadata(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Option<AudioMetadata>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    meeting_id,
                    file_path,
                    system_audio_file_path,
                    duration_ms,
                    sample_rate_hz,
                    byte_size,
                    system_audio_byte_size,
                    system_audio_stream_error,
                    created_at_ms
                FROM audio_metadata
                WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
                read_audio_metadata,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Lists audio metadata older than the provided retention cutoff.
    pub fn list_audio_metadata_before(
        &self,
        cutoff_ms: u64,
    ) -> Result<Vec<AudioMetadata>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    meeting_id,
                    file_path,
                    system_audio_file_path,
                    duration_ms,
                    sample_rate_hz,
                    byte_size,
                    system_audio_byte_size,
                    system_audio_stream_error,
                    created_at_ms
                FROM audio_metadata
                WHERE created_at_ms <= ?1
                ORDER BY created_at_ms ASC, meeting_id ASC",
            )
            .map_err(map_db_error)?;

        let rows = statement
            .query_map(params![to_db_i64(cutoff_ms)?], read_audio_metadata)
            .map_err(map_db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)
    }

    /// Removes audio metadata rows after their raw audio files have been deleted.
    pub fn delete_audio_metadata(&self, meeting_id: &MeetingId) -> Result<bool, AppError> {
        self.connection
            .execute(
                "DELETE FROM audio_metadata WHERE meeting_id = ?1",
                params![meeting_id.as_str()],
            )
            .map(|deleted_rows| deleted_rows > 0)
            .map_err(map_db_error)
    }

    /// Deletes a meeting and, via `ON DELETE CASCADE` (enabled in
    /// `configure_connection`), everything tied to it: transcript segments,
    /// metrics, reports, audio metadata, pipeline failures, and summaries.
    /// Does not touch audio files on disk — callers must delete those first,
    /// since this row is the only record of their paths.
    pub fn delete_meeting(&self, meeting_id: &MeetingId) -> Result<bool, AppError> {
        self.connection
            .execute("DELETE FROM meetings WHERE id = ?1", params![meeting_id.as_str()])
            .map(|deleted_rows| deleted_rows > 0)
            .map_err(map_db_error)
    }

    /// Inserts or replaces the singleton local voice profile.
    pub fn upsert_voice_profile(
        &self,
        profile: &VoiceProfileRecord,
    ) -> Result<VoiceProfileRecord, AppError> {
        self.connection
            .execute(
                "INSERT INTO voice_profiles (
                    id,
                    sample_audio_file_path,
                    sample_duration_ms,
                    sample_byte_size,
                    enrolled_at_ms,
                    embedding_json,
                    embedding_dimension,
                    embedding_model_path,
                    embedding_computed_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(id) DO UPDATE SET
                    sample_audio_file_path = excluded.sample_audio_file_path,
                    sample_duration_ms = excluded.sample_duration_ms,
                    sample_byte_size = excluded.sample_byte_size,
                    enrolled_at_ms = excluded.enrolled_at_ms,
                    embedding_json = excluded.embedding_json,
                    embedding_dimension = excluded.embedding_dimension,
                    embedding_model_path = excluded.embedding_model_path,
                    embedding_computed_at_ms = excluded.embedding_computed_at_ms",
                params![
                    VOICE_PROFILE_ID,
                    profile.sample_audio_file_path.as_str(),
                    optional_to_db_i64(profile.sample_duration_ms)?,
                    to_db_i64(profile.sample_byte_size)?,
                    to_db_i64(profile.enrolled_at_ms)?,
                    profile.embedding_json.as_deref(),
                    optional_to_db_i64(profile.embedding_dimension.map(u64::from))?,
                    profile.embedding_model_path.as_deref(),
                    optional_to_db_i64(profile.embedding_computed_at_ms)?,
                ],
            )
            .map_err(map_db_error)?;

        self.get_voice_profile()?.ok_or_else(|| {
            persistence_error(
                "voice_profile_not_found",
                "Saved voice profile could not be read back.",
                None,
            )
        })
    }

    /// Returns the singleton local voice profile, if enrolled.
    pub fn get_voice_profile(&self) -> Result<Option<VoiceProfileRecord>, AppError> {
        self.connection
            .query_row(
                "SELECT
                    sample_audio_file_path,
                    sample_duration_ms,
                    sample_byte_size,
                    enrolled_at_ms,
                    embedding_json,
                    embedding_dimension,
                    embedding_model_path,
                    embedding_computed_at_ms
                FROM voice_profiles
                WHERE id = ?1",
                params![VOICE_PROFILE_ID],
                read_voice_profile,
            )
            .optional()
            .map_err(map_db_error)
    }

    /// Deletes the singleton local voice profile metadata.
    pub fn delete_voice_profile(&self) -> Result<bool, AppError> {
        self.connection
            .execute(
                "DELETE FROM voice_profiles WHERE id = ?1",
                params![VOICE_PROFILE_ID],
            )
            .map(|deleted_rows| deleted_rows > 0)
            .map_err(map_db_error)
    }

    /// Returns persisted settings, or local-first defaults when none exist.
    pub fn get_settings(&self) -> Result<ResonanceSettings, AppError> {
        self.connection
            .query_row(
                "SELECT
                    microphone_device_id,
                    enable_system_audio,
                    enable_echo_cancellation,
                    enable_realtime_nudges,
                    raw_audio_retention_days,
                    analyzer_provider,
                    cloud_analysis_enabled,
                    cloud_video_review_enabled,
                    transcriber_bin_path,
                    transcriber_model_path,
                    speaker_embedding_model_path,
                    speaker_segmentation_model_path,
                    dictation_hotkey,
                    dictation_polish_enabled,
                    summarizer_provider,
                    summarizer_host,
                    summarizer_port,
                    summarizer_model
                FROM settings
                WHERE id = ?1",
                params![SETTINGS_ID],
                read_settings,
            )
            .optional()
            .map(|settings| settings.unwrap_or_default())
            .map_err(map_db_error)
    }

    /// Inserts or replaces the singleton settings row.
    pub fn upsert_settings(
        &self,
        settings: &ResonanceSettings,
        updated_at_ms: u64,
    ) -> Result<(), AppError> {
        self.connection
            .execute(
                "INSERT INTO settings (
                    id,
                    microphone_device_id,
                    enable_system_audio,
                    enable_echo_cancellation,
                    enable_realtime_nudges,
                    raw_audio_retention_days,
                    analyzer_provider,
                    cloud_analysis_enabled,
                    cloud_video_review_enabled,
                    transcriber_bin_path,
                    transcriber_model_path,
                    speaker_embedding_model_path,
                    speaker_segmentation_model_path,
                    dictation_hotkey,
                    dictation_polish_enabled,
                    summarizer_provider,
                    summarizer_host,
                    summarizer_port,
                    summarizer_model,
                    updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                ON CONFLICT(id) DO UPDATE SET
                    microphone_device_id = excluded.microphone_device_id,
                    enable_system_audio = excluded.enable_system_audio,
                    enable_echo_cancellation = excluded.enable_echo_cancellation,
                    enable_realtime_nudges = excluded.enable_realtime_nudges,
                    raw_audio_retention_days = excluded.raw_audio_retention_days,
                    analyzer_provider = excluded.analyzer_provider,
                    cloud_analysis_enabled = excluded.cloud_analysis_enabled,
                    cloud_video_review_enabled = excluded.cloud_video_review_enabled,
                    transcriber_bin_path = excluded.transcriber_bin_path,
                    transcriber_model_path = excluded.transcriber_model_path,
                    speaker_embedding_model_path = excluded.speaker_embedding_model_path,
                    speaker_segmentation_model_path = excluded.speaker_segmentation_model_path,
                    dictation_hotkey = excluded.dictation_hotkey,
                    dictation_polish_enabled = excluded.dictation_polish_enabled,
                    summarizer_provider = excluded.summarizer_provider,
                    summarizer_host = excluded.summarizer_host,
                    summarizer_port = excluded.summarizer_port,
                    summarizer_model = excluded.summarizer_model,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    SETTINGS_ID,
                    settings.microphone_device_id.as_deref(),
                    bool_to_db(settings.enable_system_audio),
                    bool_to_db(settings.enable_echo_cancellation),
                    bool_to_db(settings.enable_realtime_nudges),
                    i64::from(settings.raw_audio_retention_days),
                    analyzer_provider_to_db(settings.analyzer_provider),
                    bool_to_db(settings.cloud_analysis_enabled),
                    bool_to_db(settings.cloud_video_review_enabled),
                    settings.transcriber_bin_path.as_deref(),
                    settings.transcriber_model_path.as_deref(),
                    settings.speaker_embedding_model_path.as_deref(),
                    settings.speaker_segmentation_model_path.as_deref(),
                    settings.dictation_hotkey.as_str(),
                    bool_to_db(settings.dictation_polish_enabled),
                    summarizer_provider_to_db(settings.summarizer_provider),
                    settings.summarizer_host.as_str(),
                    i64::from(settings.summarizer_port),
                    settings.summarizer_model.as_deref(),
                    to_db_i64(updated_at_ms)?,
                ],
            )
            .map(|_| ())
            .map_err(map_db_error)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), AppError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(map_db_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(map_db_error)?;
    Ok(())
}

fn run_migrations(connection: &Connection) -> Result<(), AppError> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_versions (
                version INTEGER PRIMARY KEY,
                applied_at_ms INTEGER NOT NULL DEFAULT (unixepoch('now') * 1000)
            );

            CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                title TEXT,
                started_at_ms INTEGER NOT NULL,
                stopped_at_ms INTEGER,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                CHECK (stopped_at_ms IS NULL OR stopped_at_ms >= started_at_ms)
            );

            CREATE TABLE IF NOT EXISTS transcript_segments (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                sequence_number INTEGER NOT NULL,
                speaker_label TEXT,
                text TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                UNIQUE (meeting_id, sequence_number),
                CHECK (ended_at_ms >= started_at_ms),
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS metrics (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                name TEXT NOT NULL,
                value REAL NOT NULL,
                unit TEXT,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS reports (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL UNIQUE,
                overall_score INTEGER NOT NULL,
                body_json TEXT NOT NULL,
                generated_at_ms INTEGER NOT NULL,
                CHECK (overall_score BETWEEN 0 AND 100),
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS audio_metadata (
                meeting_id TEXT PRIMARY KEY,
                file_path TEXT NOT NULL,
                system_audio_file_path TEXT,
                duration_ms INTEGER,
                sample_rate_hz INTEGER,
                byte_size INTEGER,
                system_audio_byte_size INTEGER,
                system_audio_stream_error TEXT,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS pipeline_failures (
                meeting_id TEXT PRIMARY KEY,
                failed_stage TEXT NOT NULL,
                error_code TEXT NOT NULL,
                error_message TEXT NOT NULL,
                error_details TEXT,
                failed_at_ms INTEGER NOT NULL,
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imported_meeting_summaries (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL UNIQUE,
                source_file_path TEXT NOT NULL,
                extracted_audio_file_path TEXT NOT NULL,
                speaking_improvements_source TEXT NOT NULL DEFAULT 'none',
                body_json TEXT NOT NULL,
                generated_at_ms INTEGER NOT NULL,
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS meeting_summaries (
                meeting_id TEXT PRIMARY KEY,
                body_json TEXT NOT NULL,
                generated_at_ms INTEGER NOT NULL,
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS voice_profiles (
                id TEXT PRIMARY KEY CHECK (id = 'default'),
                sample_audio_file_path TEXT NOT NULL,
                sample_duration_ms INTEGER,
                sample_byte_size INTEGER NOT NULL,
                enrolled_at_ms INTEGER NOT NULL,
                embedding_json TEXT,
                embedding_dimension INTEGER,
                embedding_model_path TEXT,
                embedding_computed_at_ms INTEGER
            );

            CREATE TABLE IF NOT EXISTS settings (
                id TEXT PRIMARY KEY CHECK (id = 'default'),
                microphone_device_id TEXT,
                enable_system_audio INTEGER NOT NULL CHECK (enable_system_audio IN (0, 1)),
                enable_echo_cancellation INTEGER NOT NULL DEFAULT 1 CHECK (enable_echo_cancellation IN (0, 1)),
                enable_realtime_nudges INTEGER NOT NULL CHECK (enable_realtime_nudges IN (0, 1)),
                raw_audio_retention_days INTEGER NOT NULL CHECK (raw_audio_retention_days >= 0),
                analyzer_provider TEXT NOT NULL,
                cloud_analysis_enabled INTEGER NOT NULL CHECK (cloud_analysis_enabled IN (0, 1)),
                cloud_video_review_enabled INTEGER NOT NULL DEFAULT 0 CHECK (cloud_video_review_enabled IN (0, 1)),
                transcriber_bin_path TEXT,
                transcriber_model_path TEXT,
                speaker_embedding_model_path TEXT,
                speaker_segmentation_model_path TEXT,
                dictation_hotkey TEXT NOT NULL DEFAULT 'cmd+shift+d',
                dictation_polish_enabled INTEGER NOT NULL DEFAULT 0 CHECK (dictation_polish_enabled IN (0, 1)),
                summarizer_provider TEXT NOT NULL DEFAULT 'lm_studio',
                summarizer_host TEXT NOT NULL DEFAULT '127.0.0.1',
                summarizer_port INTEGER NOT NULL DEFAULT 1234,
                summarizer_model TEXT,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS practice_recordings (
                id TEXT PRIMARY KEY,
                title TEXT,
                source_kind TEXT NOT NULL CHECK (source_kind IN ('camera', 'imported')),
                video_file_path TEXT NOT NULL,
                extracted_audio_file_path TEXT,
                duration_ms INTEGER,
                byte_size INTEGER,
                recorded_at_ms INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                analysis_status TEXT NOT NULL CHECK (
                    analysis_status IN ('recorded', 'extracting', 'transcribing', 'reviewing', 'complete', 'failed_partial')
                ),
                cloud_video_used INTEGER NOT NULL CHECK (cloud_video_used IN (0, 1)),
                pipeline_failure_code TEXT,
                pipeline_failure_message TEXT
            );

            CREATE TABLE IF NOT EXISTS practice_review_reports (
                id TEXT PRIMARY KEY,
                practice_recording_id TEXT NOT NULL UNIQUE,
                overall_score INTEGER,
                audio_score INTEGER,
                visual_score INTEGER,
                body_json TEXT NOT NULL,
                generated_at_ms INTEGER NOT NULL,
                CHECK (overall_score IS NULL OR overall_score BETWEEN 0 AND 100),
                CHECK (audio_score IS NULL OR audio_score BETWEEN 0 AND 100),
                CHECK (visual_score IS NULL OR visual_score BETWEEN 0 AND 100),
                FOREIGN KEY (practice_recording_id) REFERENCES practice_recordings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS practice_timeline_annotations (
                id TEXT PRIMARY KEY,
                practice_recording_id TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                category TEXT NOT NULL,
                severity TEXT NOT NULL CHECK (severity IN ('info', 'caution', 'strong')),
                evidence TEXT NOT NULL,
                suggestion TEXT NOT NULL,
                source TEXT NOT NULL CHECK (source IN ('audioLocal', 'videoCloud', 'videoLocal')),
                CHECK (ended_at_ms >= started_at_ms),
                FOREIGN KEY (practice_recording_id) REFERENCES practice_recordings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS dictation_sessions (
                id TEXT PRIMARY KEY,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                word_count INTEGER NOT NULL,
                words_per_minute REAL NOT NULL,
                created_at_ms INTEGER NOT NULL,
                CHECK (ended_at_ms >= started_at_ms)
            );

            CREATE INDEX IF NOT EXISTS idx_meetings_recent
                ON meetings(updated_at_ms DESC, started_at_ms DESC, id ASC);
            CREATE INDEX IF NOT EXISTS idx_transcript_segments_meeting
                ON transcript_segments(meeting_id, sequence_number);
            CREATE INDEX IF NOT EXISTS idx_metrics_meeting
                ON metrics(meeting_id);
            CREATE INDEX IF NOT EXISTS idx_metrics_meeting_history
                ON metrics(meeting_id, created_at_ms ASC, id ASC);
            CREATE INDEX IF NOT EXISTS idx_reports_recent
                ON reports(generated_at_ms DESC, id ASC);
            CREATE INDEX IF NOT EXISTS idx_imported_summaries_recent
                ON imported_meeting_summaries(generated_at_ms DESC, id ASC);
            CREATE INDEX IF NOT EXISTS idx_practice_recordings_recent
                ON practice_recordings(updated_at_ms DESC, recorded_at_ms DESC, id ASC);
            CREATE INDEX IF NOT EXISTS idx_practice_annotations_recording
                ON practice_timeline_annotations(practice_recording_id, started_at_ms ASC);
            CREATE INDEX IF NOT EXISTS idx_dictation_sessions_recent
                ON dictation_sessions(started_at_ms DESC, id ASC);

            INSERT OR IGNORE INTO schema_versions(version) VALUES (1);
            ",
        )
        .map_err(map_db_error)?;

    ensure_settings_column(connection, "transcriber_bin_path", "TEXT")?;
    ensure_settings_column(connection, "transcriber_model_path", "TEXT")?;
    ensure_settings_column(connection, "speaker_embedding_model_path", "TEXT")?;
    ensure_settings_column(connection, "speaker_segmentation_model_path", "TEXT")?;
    ensure_settings_column(
        connection,
        "enable_system_audio",
        "INTEGER NOT NULL DEFAULT 1 CHECK (enable_system_audio IN (0, 1))",
    )?;
    ensure_settings_column(
        connection,
        "enable_echo_cancellation",
        "INTEGER NOT NULL DEFAULT 1 CHECK (enable_echo_cancellation IN (0, 1))",
    )?;
    ensure_settings_column(
        connection,
        "cloud_video_review_enabled",
        "INTEGER NOT NULL DEFAULT 0 CHECK (cloud_video_review_enabled IN (0, 1))",
    )?;
    ensure_column(
        connection,
        "audio_metadata",
        "system_audio_file_path",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "audio_metadata",
        "system_audio_byte_size",
        "INTEGER",
    )?;
    ensure_column(
        connection,
        "audio_metadata",
        "system_audio_stream_error",
        "TEXT",
    )?;
    ensure_column(connection, "voice_profiles", "embedding_json", "TEXT")?;
    ensure_column(
        connection,
        "voice_profiles",
        "embedding_dimension",
        "INTEGER",
    )?;
    ensure_column(connection, "voice_profiles", "embedding_model_path", "TEXT")?;
    ensure_column(
        connection,
        "voice_profiles",
        "embedding_computed_at_ms",
        "INTEGER",
    )?;
    ensure_column(
        connection,
        "imported_meeting_summaries",
        "speaking_improvements_source",
        "TEXT NOT NULL DEFAULT 'none'",
    )?;
    ensure_settings_column(
        connection,
        "dictation_hotkey",
        "TEXT NOT NULL DEFAULT 'cmd+shift+d'",
    )?;
    ensure_settings_column(
        connection,
        "dictation_polish_enabled",
        "INTEGER NOT NULL DEFAULT 0 CHECK (dictation_polish_enabled IN (0, 1))",
    )?;
    ensure_settings_column(
        connection,
        "summarizer_provider",
        "TEXT NOT NULL DEFAULT 'lm_studio'",
    )?;
    ensure_settings_column(
        connection,
        "summarizer_host",
        "TEXT NOT NULL DEFAULT '127.0.0.1'",
    )?;
    ensure_settings_column(
        connection,
        "summarizer_port",
        "INTEGER NOT NULL DEFAULT 1234",
    )?;
    ensure_settings_column(connection, "summarizer_model", "TEXT")?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (2)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (3)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (4)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (5)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (6)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (7)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (8)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (9)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (10)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (11)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (12)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (13)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (14)",
            [],
        )
        .map_err(map_db_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_versions(version) VALUES (15)",
            [],
        )
        .map_err(map_db_error)?;

    let version = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_versions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(map_db_error)?;

    if version < CURRENT_SCHEMA_VERSION {
        return Err(persistence_error(
            "migration_failed",
            "Database schema did not reach the current version.",
            Some(format!(
                "version={version}, expected={CURRENT_SCHEMA_VERSION}"
            )),
        ));
    }

    Ok(())
}

fn ensure_settings_column(
    connection: &Connection,
    column_name: &str,
    column_type: &str,
) -> Result<(), AppError> {
    ensure_column(connection, "settings", column_name, column_type)
}

fn ensure_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
    column_type: &str,
) -> Result<(), AppError> {
    validate_migration_identifier("table_name", table_name)?;
    validate_migration_identifier("column_name", column_name)?;
    validate_migration_column_type(column_type)?;

    let column_exists = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .and_then(|mut statement| {
            let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
            for row in rows {
                if row? == column_name {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .map_err(map_db_error)?;

    if column_exists {
        return Ok(());
    }

    let sql = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_type}");
    connection.execute(&sql, []).map_err(map_db_error)?;
    Ok(())
}

fn validate_migration_identifier(label: &str, value: &str) -> Result<(), AppError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(invalid_migration_sql(label));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(invalid_migration_sql(label));
    }
    if chars.any(|character| !(character.is_ascii_alphanumeric() || character == '_')) {
        return Err(invalid_migration_sql(label));
    }
    Ok(())
}

fn validate_migration_column_type(column_type: &str) -> Result<(), AppError> {
    match column_type {
        "TEXT"
        | "INTEGER"
        | "TEXT NOT NULL DEFAULT 'none'"
        | "TEXT NOT NULL DEFAULT 'cmd+shift+d'"
        | "INTEGER NOT NULL DEFAULT 1 CHECK (enable_system_audio IN (0, 1))"
        | "INTEGER NOT NULL DEFAULT 1 CHECK (enable_echo_cancellation IN (0, 1))"
        | "INTEGER NOT NULL DEFAULT 0 CHECK (cloud_video_review_enabled IN (0, 1))"
        | "INTEGER NOT NULL DEFAULT 0 CHECK (dictation_polish_enabled IN (0, 1))"
        | "TEXT NOT NULL DEFAULT 'lm_studio'"
        | "TEXT NOT NULL DEFAULT '127.0.0.1'"
        | "INTEGER NOT NULL DEFAULT 1234" => Ok(()),
        _ => Err(invalid_migration_sql("column_type")),
    }
}

fn invalid_migration_sql(label: &str) -> AppError {
    persistence_error(
        "invalid_migration_sql",
        "Migration SQL identifiers must be static safe schema values.",
        Some(format!("field={label}")),
    )
}

fn read_meeting(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingRecord> {
    Ok(MeetingRecord {
        id: MeetingId::new(row.get::<_, String>(0)?),
        title: row.get(1)?,
        started_at_ms: from_db_u64(row.get(2)?)?,
        stopped_at_ms: optional_from_db_u64(row.get(3)?)?,
        created_at_ms: from_db_u64(row.get(4)?)?,
        updated_at_ms: from_db_u64(row.get(5)?)?,
    })
}

fn read_dictation_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<DictationSessionRecord> {
    Ok(DictationSessionRecord {
        id: DictationSessionId::new(row.get::<_, String>(0)?),
        started_at_ms: from_db_u64(row.get(1)?)?,
        ended_at_ms: from_db_u64(row.get(2)?)?,
        duration_ms: from_db_u64(row.get(3)?)?,
        word_count: from_db_u32(row.get(4)?, 4)?,
        words_per_minute: row.get(5)?,
        created_at_ms: from_db_u64(row.get(6)?)?,
    })
}

fn read_meeting_history(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingHistoryRecord> {
    let overall_score = row
        .get::<_, Option<i64>>(8)?
        .map(|value| score_from_db(value, 8))
        .transpose()?;
    Ok(MeetingHistoryRecord {
        id: MeetingId::new(row.get::<_, String>(0)?),
        title: row.get(1)?,
        started_at_ms: from_db_u64(row.get(2)?)?,
        stopped_at_ms: optional_from_db_u64(row.get(3)?)?,
        updated_at_ms: from_db_u64(row.get(4)?)?,
        duration_ms: optional_from_db_u64(row.get(5)?)?,
        audio_file_path: row.get(6)?,
        report_id: row.get::<_, Option<String>>(7)?.map(ReportId::new),
        overall_score,
        report_generated_at_ms: optional_from_db_u64(row.get(9)?)?,
        transcript_segment_count: from_db_u32(row.get(10)?, 10)?,
        pipeline_failure: read_optional_pipeline_failure(row, 0, 11)?,
    })
}

fn read_pipeline_failure(row: &rusqlite::Row<'_>) -> rusqlite::Result<PipelineFailureRecord> {
    Ok(PipelineFailureRecord {
        meeting_id: MeetingId::new(row.get::<_, String>(0)?),
        failed_stage: processing_stage_from_db(row.get::<_, String>(1)?.as_str(), 1)?,
        error_code: row.get(2)?,
        error_message: row.get(3)?,
        error_details: row.get(4)?,
        failed_at_ms: from_db_u64(row.get(5)?)?,
    })
}

fn read_optional_pipeline_failure(
    row: &rusqlite::Row<'_>,
    meeting_id_index: usize,
    failed_stage_index: usize,
) -> rusqlite::Result<Option<PipelineFailureRecord>> {
    let failed_stage = row.get::<_, Option<String>>(failed_stage_index)?;
    failed_stage
        .map(|stage| {
            Ok(PipelineFailureRecord {
                meeting_id: MeetingId::new(row.get::<_, String>(meeting_id_index)?),
                failed_stage: processing_stage_from_db(&stage, failed_stage_index)?,
                error_code: row.get(failed_stage_index + 1)?,
                error_message: row.get(failed_stage_index + 2)?,
                error_details: row.get(failed_stage_index + 3)?,
                failed_at_ms: from_db_u64(row.get(failed_stage_index + 4)?)?,
            })
        })
        .transpose()
}

fn read_meeting_trend(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingTrendRecord> {
    let overall_score = row
        .get::<_, Option<i64>>(5)?
        .map(|value| score_from_db(value, 5))
        .transpose()?;
    Ok(MeetingTrendRecord {
        id: MeetingId::new(row.get::<_, String>(0)?),
        title: row.get(1)?,
        started_at_ms: from_db_u64(row.get(2)?)?,
        filler_word_count: row.get(3)?,
        words_per_minute: row.get(4)?,
        overall_score,
    })
}

fn processing_stage_to_db(stage: ProcessingStage) -> &'static str {
    match stage {
        ProcessingStage::Recording => "recording",
        ProcessingStage::Transcribing => "transcribing",
        ProcessingStage::Metrics => "metrics",
        ProcessingStage::Analyzing => "analyzing",
    }
}

fn processing_stage_from_db(value: &str, column_index: usize) -> rusqlite::Result<ProcessingStage> {
    match value {
        "recording" => Ok(ProcessingStage::Recording),
        "transcribing" => Ok(ProcessingStage::Transcribing),
        "metrics" => Ok(ProcessingStage::Metrics),
        "analyzing" => Ok(ProcessingStage::Analyzing),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            column_index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown processing stage: {value}"),
            )),
        )),
    }
}

impl From<&CreateTranscriptSegment> for TranscriptSegmentRecord {
    fn from(segment: &CreateTranscriptSegment) -> Self {
        Self {
            id: segment.id.clone(),
            meeting_id: segment.meeting_id.clone(),
            sequence_number: segment.sequence_number,
            speaker_label: segment.speaker_label.clone(),
            text: segment.text.clone(),
            started_at_ms: segment.started_at_ms,
            ended_at_ms: segment.ended_at_ms,
            created_at_ms: segment.created_at_ms,
        }
    }
}

fn read_transcript_segment(row: &rusqlite::Row<'_>) -> rusqlite::Result<TranscriptSegmentRecord> {
    Ok(TranscriptSegmentRecord {
        id: SegmentId::new(row.get::<_, String>(0)?),
        meeting_id: MeetingId::new(row.get::<_, String>(1)?),
        sequence_number: from_db_u32(row.get(2)?, 2)?,
        speaker_label: row.get(3)?,
        text: row.get(4)?,
        started_at_ms: from_db_u64(row.get(5)?)?,
        ended_at_ms: from_db_u64(row.get(6)?)?,
        created_at_ms: from_db_u64(row.get(7)?)?,
    })
}

impl From<&CreateMetric> for MetricRecord {
    fn from(metric: &CreateMetric) -> Self {
        Self {
            id: metric.id.clone(),
            meeting_id: metric.meeting_id.clone(),
            name: metric.name.clone(),
            value: metric.value,
            unit: metric.unit.clone(),
            created_at_ms: metric.created_at_ms,
        }
    }
}

fn read_metric(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricRecord> {
    Ok(MetricRecord {
        id: MetricId::new(row.get::<_, String>(0)?),
        meeting_id: MeetingId::new(row.get::<_, String>(1)?),
        name: row.get(2)?,
        value: row.get(3)?,
        unit: row.get(4)?,
        created_at_ms: from_db_u64(row.get(5)?)?,
    })
}

fn read_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportRecord> {
    Ok(ReportRecord {
        id: ReportId::new(row.get::<_, String>(0)?),
        meeting_id: MeetingId::new(row.get::<_, String>(1)?),
        overall_score: score_from_db(row.get(2)?, 2)?,
        body_json: row.get(3)?,
        generated_at_ms: from_db_u64(row.get(4)?)?,
    })
}

fn read_imported_meeting_summary(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ImportedMeetingSummaryRecord> {
    Ok(ImportedMeetingSummaryRecord {
        id: SummaryId::new(row.get::<_, String>(0)?),
        meeting_id: MeetingId::new(row.get::<_, String>(1)?),
        source_file_path: row.get(2)?,
        extracted_audio_file_path: row.get(3)?,
        speaking_improvements_source: row.get(4)?,
        body_json: row.get(5)?,
        generated_at_ms: from_db_u64(row.get(6)?)?,
    })
}

fn read_practice_recording(row: &rusqlite::Row<'_>) -> rusqlite::Result<PracticeRecordingRecord> {
    Ok(PracticeRecordingRecord {
        id: PracticeRecordingId::new(row.get::<_, String>(0)?),
        title: row.get(1)?,
        source_kind: row.get(2)?,
        video_file_path: row.get(3)?,
        extracted_audio_file_path: row.get(4)?,
        duration_ms: optional_from_db_u64(row.get(5)?)?,
        byte_size: optional_from_db_u64(row.get(6)?)?,
        recorded_at_ms: from_db_u64(row.get(7)?)?,
        created_at_ms: from_db_u64(row.get(8)?)?,
        updated_at_ms: from_db_u64(row.get(9)?)?,
        analysis_status: row.get(10)?,
        cloud_video_used: db_to_bool(row.get(11)?, 11)?,
        pipeline_failure_code: row.get(12)?,
        pipeline_failure_message: row.get(13)?,
    })
}

fn read_practice_review_report(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PracticeReviewReportRecord> {
    Ok(PracticeReviewReportRecord {
        id: PracticeReviewReportId::new(row.get::<_, String>(0)?),
        practice_recording_id: PracticeRecordingId::new(row.get::<_, String>(1)?),
        overall_score: row
            .get::<_, Option<i64>>(2)?
            .map(|value| score_from_db(value, 2))
            .transpose()?,
        audio_score: row
            .get::<_, Option<i64>>(3)?
            .map(|value| score_from_db(value, 3))
            .transpose()?,
        visual_score: row
            .get::<_, Option<i64>>(4)?
            .map(|value| score_from_db(value, 4))
            .transpose()?,
        body_json: row.get(5)?,
        generated_at_ms: from_db_u64(row.get(6)?)?,
    })
}

fn read_practice_timeline_annotation(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PracticeTimelineAnnotationRecord> {
    Ok(PracticeTimelineAnnotationRecord {
        id: PracticeAnnotationId::new(row.get::<_, String>(0)?),
        practice_recording_id: PracticeRecordingId::new(row.get::<_, String>(1)?),
        started_at_ms: from_db_u64(row.get(2)?)?,
        ended_at_ms: from_db_u64(row.get(3)?)?,
        category: row.get(4)?,
        severity: row.get(5)?,
        evidence: row.get(6)?,
        suggestion: row.get(7)?,
        source: row.get(8)?,
    })
}

fn read_audio_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudioMetadata> {
    Ok(AudioMetadata {
        meeting_id: MeetingId::new(row.get::<_, String>(0)?),
        file_path: row.get(1)?,
        system_audio_file_path: row.get(2)?,
        duration_ms: optional_from_db_u64(row.get(3)?)?,
        sample_rate_hz: optional_from_db_u32(row.get(4)?, 4)?,
        byte_size: optional_from_db_u64(row.get(5)?)?,
        system_audio_byte_size: optional_from_db_u64(row.get(6)?)?,
        system_audio_stream_error: row.get(7)?,
        created_at_ms: from_db_u64(row.get(8)?)?,
    })
}

fn read_voice_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<VoiceProfileRecord> {
    Ok(VoiceProfileRecord {
        sample_audio_file_path: row.get(0)?,
        sample_duration_ms: optional_from_db_u64(row.get(1)?)?,
        sample_byte_size: from_db_u64(row.get(2)?)?,
        enrolled_at_ms: from_db_u64(row.get(3)?)?,
        embedding_json: row.get(4)?,
        embedding_dimension: optional_from_db_u32(row.get(5)?, 5)?,
        embedding_model_path: row.get(6)?,
        embedding_computed_at_ms: optional_from_db_u64(row.get(7)?)?,
    })
}

fn read_settings(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResonanceSettings> {
    let retention_days = row.get::<_, i64>(4)?;
    let raw_audio_retention_days = u16::try_from(retention_days).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;

    Ok(ResonanceSettings {
        microphone_device_id: row.get(0)?,
        enable_system_audio: db_to_bool(row.get(1)?, 1)?,
        enable_echo_cancellation: db_to_bool(row.get(2)?, 2)?,
        enable_realtime_nudges: db_to_bool(row.get(3)?, 3)?,
        raw_audio_retention_days,
        analyzer_provider: analyzer_provider_from_db(row.get::<_, String>(5)?, 5)?,
        cloud_analysis_enabled: db_to_bool(row.get(6)?, 6)?,
        cloud_video_review_enabled: db_to_bool(row.get(7)?, 7)?,
        transcriber_bin_path: row.get(8)?,
        transcriber_model_path: row.get(9)?,
        speaker_embedding_model_path: row.get(10)?,
        speaker_segmentation_model_path: row.get(11)?,
        dictation_hotkey: row.get(12)?,
        dictation_polish_enabled: db_to_bool(row.get(13)?, 13)?,
        summarizer_provider: summarizer_provider_from_db(row.get::<_, String>(14)?, 14)?,
        summarizer_host: row.get(15)?,
        summarizer_port: from_db_u16(row.get(16)?, 16)?,
        summarizer_model: row.get(17)?,
    })
}

fn to_db_i64(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|error| {
        persistence_error(
            "invalid_persistence_value",
            "Value is too large to persist as a SQLite integer.",
            Some(error.to_string()),
        )
    })
}

fn optional_to_db_i64(value: Option<u64>) -> Result<Option<i64>, AppError> {
    value.map(to_db_i64).transpose()
}

fn like_contains_pattern(value: &str) -> String {
    format!("%{}%", escape_like_pattern(value))
}

fn like_prefix_pattern(value: &str) -> String {
    format!("{}%", escape_like_pattern(value))
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn from_db_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn optional_from_db_u64(value: Option<i64>) -> rusqlite::Result<Option<u64>> {
    value.map(from_db_u64).transpose()
}

fn from_db_u32(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn from_db_u16(value: i64, column: usize) -> rusqlite::Result<u16> {
    u16::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn optional_from_db_u32(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u32>> {
    value.map(|item| from_db_u32(item, column)).transpose()
}

fn score_from_db(value: i64, column: usize) -> rusqlite::Result<Score> {
    let score_value = u8::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;

    Score::new(score_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            format!("{}: {}", error.code, error.message).into(),
        )
    })
}

fn bool_to_db(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn db_to_bool(value: i64, column: usize) -> rusqlite::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            format!("expected 0 or 1, got {value}").into(),
        )),
    }
}

fn analyzer_provider_to_db(provider: AnalyzerProvider) -> &'static str {
    match provider {
        AnalyzerProvider::LocalOllama => "local_ollama",
        AnalyzerProvider::CloudOpenAi => "cloud_open_ai",
        AnalyzerProvider::CloudClaude => "cloud_claude",
    }
}

fn analyzer_provider_from_db(value: String, column: usize) -> rusqlite::Result<AnalyzerProvider> {
    match value.as_str() {
        "local_ollama" => Ok(AnalyzerProvider::LocalOllama),
        "cloud_open_ai" => Ok(AnalyzerProvider::CloudOpenAi),
        "cloud_claude" => Ok(AnalyzerProvider::CloudClaude),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            format!("unknown analyzer provider: {value}").into(),
        )),
    }
}

fn summarizer_provider_to_db(provider: SummarizerProvider) -> &'static str {
    match provider {
        SummarizerProvider::LmStudio => "lm_studio",
        SummarizerProvider::Ollama => "ollama",
        SummarizerProvider::Custom => "custom",
    }
}

fn summarizer_provider_from_db(value: String, column: usize) -> rusqlite::Result<SummarizerProvider> {
    match value.as_str() {
        "lm_studio" => Ok(SummarizerProvider::LmStudio),
        "ollama" => Ok(SummarizerProvider::Ollama),
        "custom" => Ok(SummarizerProvider::Custom),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            format!("unknown summarizer provider: {value}").into(),
        )),
    }
}

fn map_db_error(error: rusqlite::Error) -> AppError {
    persistence_error(
        "database_error",
        "SQLite persistence operation failed.",
        Some(error.to_string()),
    )
}

fn map_report_create_error(error: rusqlite::Error) -> AppError {
    if is_constraint_violation(&error) {
        return persistence_error(
            "report_already_exists",
            "Meeting already has a coaching report. Re-analysis is not supported yet.",
            Some(error.to_string()),
        );
    }

    map_db_error(error)
}

fn is_constraint_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if sqlite_error.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

fn persistence_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use super::*;

    struct TestRepository {
        _database: NamedTempFile,
        repository: SqliteRepository,
    }

    #[test]
    fn migration_creates_schema_from_scratch_and_is_idempotent() {
        let database = NamedTempFile::new().expect("temp database can be created");
        let repository = SqliteRepository::open(database.path()).expect("schema can be migrated");

        assert_eq!(
            repository
                .schema_version()
                .expect("schema version can be read"),
            CURRENT_SCHEMA_VERSION
        );

        let reopened = SqliteRepository::open(database.path()).expect("migration is idempotent");
        assert_eq!(
            reopened
                .schema_version()
                .expect("schema version can be read after reopening"),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn migration_column_helper_rejects_dynamic_sql_fragments() {
        let connection = Connection::open_in_memory().expect("in-memory database can open");
        connection
            .execute("CREATE TABLE settings (id TEXT PRIMARY KEY)", [])
            .expect("settings table can be created");

        let table_error = ensure_column(
            &connection,
            "settings; DROP TABLE settings",
            "safe_column",
            "TEXT",
        )
        .expect_err("table names must be safe static identifiers");
        assert_eq!(table_error.code, "invalid_migration_sql");
        assert_eq!(table_error.details.as_deref(), Some("field=table_name"));

        let type_error = ensure_column(
            &connection,
            "settings",
            "safe_column",
            "TEXT DEFAULT ''; DROP TABLE settings",
        )
        .expect_err("column types must be from the static migration allow-list");
        assert_eq!(type_error.code, "invalid_migration_sql");
        assert_eq!(type_error.details.as_deref(), Some("field=column_type"));
    }

    #[test]
    fn migration_adds_system_audio_setting_to_existing_settings_table() {
        let database = NamedTempFile::new().expect("temp database can be created");
        let connection = Connection::open(database.path()).expect("legacy database can open");
        connection
            .execute_batch(
                "
                CREATE TABLE schema_versions(version INTEGER PRIMARY KEY);
                INSERT INTO schema_versions(version) VALUES (1);
                CREATE TABLE settings (
                    id TEXT PRIMARY KEY CHECK (id = 'default'),
                    microphone_device_id TEXT,
                    enable_realtime_nudges INTEGER NOT NULL CHECK (enable_realtime_nudges IN (0, 1)),
                    raw_audio_retention_days INTEGER NOT NULL CHECK (raw_audio_retention_days >= 0),
                    analyzer_provider TEXT NOT NULL,
                    cloud_analysis_enabled INTEGER NOT NULL CHECK (cloud_analysis_enabled IN (0, 1)),
                    updated_at_ms INTEGER NOT NULL
                );
                INSERT INTO settings (
                    id,
                    microphone_device_id,
                    enable_realtime_nudges,
                    raw_audio_retention_days,
                    analyzer_provider,
                    cloud_analysis_enabled,
                    updated_at_ms
                ) VALUES ('default', NULL, 1, 7, 'local_ollama', 0, 1);
                ",
            )
            .expect("legacy settings table can be created");
        drop(connection);

        let repository =
            SqliteRepository::open(database.path()).expect("legacy schema can be migrated");

        assert!(
            repository
                .get_settings()
                .expect("settings can be read after migration")
                .enable_system_audio
        );
        assert!(
            repository
                .get_settings()
                .expect("settings can be read after migration")
                .enable_echo_cancellation
        );
        assert!(
            !repository
                .get_settings()
                .expect("settings can be read after migration")
                .cloud_video_review_enabled
        );
    }

    #[test]
    fn migration_adds_imported_summary_source_with_default_to_legacy_table() {
        let database = NamedTempFile::new().expect("temp database can be created");
        let connection = Connection::open(database.path()).expect("legacy database can open");
        connection
            .execute_batch(
                "
                CREATE TABLE schema_versions(version INTEGER PRIMARY KEY);
                INSERT INTO schema_versions(version) VALUES (10);
                CREATE TABLE imported_meeting_summaries (
                    id TEXT PRIMARY KEY,
                    meeting_id TEXT NOT NULL UNIQUE,
                    source_file_path TEXT NOT NULL,
                    extracted_audio_file_path TEXT NOT NULL,
                    body_json TEXT NOT NULL,
                    generated_at_ms INTEGER NOT NULL
                );
                INSERT INTO imported_meeting_summaries (
                    id,
                    meeting_id,
                    source_file_path,
                    extracted_audio_file_path,
                    body_json,
                    generated_at_ms
                ) VALUES (
                    'legacy-summary',
                    'legacy-meeting',
                    '/tmp/source.mp4',
                    '/tmp/extracted.wav',
                    '{}',
                    1000
                );
                ",
            )
            .expect("legacy imported summary table can be created");
        drop(connection);

        let repository =
            SqliteRepository::open(database.path()).expect("legacy schema can be migrated");

        let source = repository
            .connection
            .query_row(
                "SELECT speaking_improvements_source FROM imported_meeting_summaries WHERE id = 'legacy-summary'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("legacy imported summary source can be read after migration");
        assert_eq!(source, "none");

        let not_null = repository
            .connection
            .prepare("PRAGMA table_info(imported_meeting_summaries)")
            .and_then(|mut statement| {
                let rows = statement.query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
                })?;
                for row in rows {
                    let (name, not_null) = row?;
                    if name == "speaking_improvements_source" {
                        return Ok(not_null);
                    }
                }
                Ok(0)
            })
            .expect("imported summary schema can be inspected");
        assert_eq!(not_null, 1);
    }

    #[test]
    fn create_get_and_list_meetings_returns_recent_meetings_first() {
        let test_repository = repository();
        let older = CreateMeeting {
            id: MeetingId::new("meeting-older"),
            title: Some("Older sync".to_string()),
            started_at_ms: 1_000,
            stopped_at_ms: Some(2_000),
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
        };
        let newest = CreateMeeting {
            id: MeetingId::new("meeting-newest"),
            title: Some("Newest sync".to_string()),
            started_at_ms: 3_000,
            stopped_at_ms: None,
            created_at_ms: 3_000,
            updated_at_ms: 5_000,
        };
        let tied = CreateMeeting {
            id: MeetingId::new("meeting-tied"),
            title: None,
            started_at_ms: 4_000,
            stopped_at_ms: None,
            created_at_ms: 4_000,
            updated_at_ms: 5_000,
        };

        test_repository
            .repository
            .create_meeting(&older)
            .expect("older meeting can be created");
        test_repository
            .repository
            .create_meeting(&newest)
            .expect("newest meeting can be created");
        let persisted_tied = test_repository
            .repository
            .create_meeting(&tied)
            .expect("tied meeting can be created");

        assert_eq!(
            test_repository
                .repository
                .get_meeting(&tied.id)
                .expect("meeting can be read by id"),
            Some(persisted_tied)
        );

        let ids = test_repository
            .repository
            .list_meetings()
            .expect("meetings can be listed")
            .into_iter()
            .map(|meeting| meeting.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                MeetingId::new("meeting-tied"),
                MeetingId::new("meeting-newest"),
                MeetingId::new("meeting-older"),
            ]
        );
    }

    #[test]
    fn delete_meeting_cascades_to_related_rows() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-to-delete");

        test_repository
            .repository
            .create_transcript_segments(&[CreateTranscriptSegment {
                id: SegmentId::new("segment-1"),
                meeting_id: meeting.id.clone(),
                sequence_number: 0,
                speaker_label: None,
                text: "Hello".to_string(),
                started_at_ms: 0,
                ended_at_ms: 500,
                created_at_ms: 500,
            }])
            .expect("segment can be created");
        test_repository
            .repository
            .create_metric(&CreateMetric {
                id: MetricId::new("metric-1"),
                meeting_id: meeting.id.clone(),
                name: "word_count".to_string(),
                value: 10.0,
                unit: None,
                created_at_ms: 500,
            })
            .expect("metric can be created");

        let deleted = test_repository
            .repository
            .delete_meeting(&meeting.id)
            .expect("meeting can be deleted");
        assert!(deleted);

        assert_eq!(
            test_repository
                .repository
                .get_meeting(&meeting.id)
                .expect("meeting lookup succeeds"),
            None
        );
        assert!(test_repository
            .repository
            .list_transcript_segments(&meeting.id)
            .expect("segments lookup succeeds")
            .is_empty());
        assert!(test_repository
            .repository
            .list_metrics(&meeting.id)
            .expect("metrics lookup succeeds")
            .is_empty());

        let deleted_again = test_repository
            .repository
            .delete_meeting(&meeting.id)
            .expect("deleting a missing meeting does not error");
        assert!(!deleted_again);
    }

    #[test]
    fn meeting_history_supports_pagination_search_and_report_status_fields() {
        let test_repository = repository();
        let older = test_repository
            .repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new("history-older"),
                title: Some("Weekly design review".to_string()),
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("older history meeting can be created");
        let newer = test_repository
            .repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new("history-newer"),
                title: Some("Customer roadmap".to_string()),
                started_at_ms: 3_000,
                stopped_at_ms: Some(4_000),
                created_at_ms: 3_000,
                updated_at_ms: 4_000,
            })
            .expect("newer history meeting can be created");

        test_repository
            .repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: newer.id.clone(),
                file_path: "/tmp/customer-roadmap.wav".to_string(),
                system_audio_file_path: None,
                duration_ms: Some(65_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(1_024),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 4_000,
            })
            .expect("audio metadata can be created");
        test_repository
            .repository
            .create_transcript_segments(&[
                CreateTranscriptSegment {
                    id: SegmentId::new("history-segment-1"),
                    meeting_id: newer.id.clone(),
                    sequence_number: 0,
                    speaker_label: Some("User".to_string()),
                    text: "We should refine the launch plan".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                    created_at_ms: 4_000,
                },
                CreateTranscriptSegment {
                    id: SegmentId::new("history-segment-2"),
                    meeting_id: older.id.clone(),
                    sequence_number: 0,
                    speaker_label: Some("User".to_string()),
                    text: "Architecture review notes".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                    created_at_ms: 2_000,
                },
            ])
            .expect("history transcript segments can be created");
        test_repository
            .repository
            .create_report(&CreateReport {
                id: ReportId::new("history-report"),
                meeting_id: newer.id.clone(),
                overall_score: Score::new(84).expect("score is valid"),
                body_json: "{}".to_string(),
                generated_at_ms: 4_500,
            })
            .expect("history report can be created");

        let first_page = test_repository
            .repository
            .list_meeting_history(None, 1, 0)
            .expect("history first page can be listed");
        assert_eq!(first_page.len(), 1);
        assert_eq!(first_page[0].id, MeetingId::new("history-newer"));
        assert_eq!(first_page[0].duration_ms, Some(65_000));
        assert_eq!(
            first_page[0].report_id,
            Some(ReportId::new("history-report"))
        );
        assert_eq!(
            first_page[0].overall_score,
            Some(Score::new(84).expect("score is valid"))
        );
        assert_eq!(first_page[0].transcript_segment_count, 1);

        let second_page = test_repository
            .repository
            .list_meeting_history(None, 1, 1)
            .expect("history second page can be listed");
        assert_eq!(second_page[0].id, MeetingId::new("history-older"));

        let transcript_search = test_repository
            .repository
            .list_meeting_history(Some("launch plan"), 10, 0)
            .expect("history can be searched by transcript text");
        assert_eq!(
            transcript_search
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![MeetingId::new("history-newer")]
        );

        let literal_search = test_repository
            .repository
            .list_meeting_history(Some("100%"), 10, 0)
            .expect("history literal wildcard search is safe");
        assert!(literal_search.is_empty());
    }

    #[test]
    fn meeting_trends_list_recent_sparse_datapoints() {
        let test_repository = repository();
        let older = test_repository
            .repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new("trend-older"),
                title: Some("Older trend".to_string()),
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("older trend meeting can be created");
        let newer = test_repository
            .repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new("trend-newer"),
                title: Some("Newer trend".to_string()),
                started_at_ms: 3_000,
                stopped_at_ms: Some(4_000),
                created_at_ms: 3_000,
                updated_at_ms: 4_000,
            })
            .expect("newer trend meeting can be created");

        for (meeting_id, metric_name, value) in [
            (&older.id, "filler_word_count", 6.0),
            (&older.id, "words_per_minute", 108.0),
            (&newer.id, "words_per_minute", 142.0),
        ] {
            test_repository
                .repository
                .create_metric(&CreateMetric {
                    id: MetricId::new(format!("metric-{}-{metric_name}", meeting_id.as_str())),
                    meeting_id: meeting_id.clone(),
                    name: metric_name.to_string(),
                    value,
                    unit: None,
                    created_at_ms: 5_000,
                })
                .expect("trend metric can be created");
        }
        test_repository
            .repository
            .create_report(&CreateReport {
                id: ReportId::new("trend-older-report"),
                meeting_id: older.id.clone(),
                overall_score: Score::new(71).expect("score is valid"),
                body_json: "{}".to_string(),
                generated_at_ms: 6_000,
            })
            .expect("older trend report can be created");

        let trends = test_repository
            .repository
            .list_meeting_trends(10)
            .expect("meeting trends can be listed");

        assert_eq!(trends.len(), 2);
        assert_eq!(trends[0].id, newer.id);
        assert_eq!(trends[0].filler_word_count, None);
        assert_eq!(trends[0].words_per_minute, Some(142.0));
        assert_eq!(trends[0].overall_score, None);
        assert_eq!(trends[1].id, older.id);
        assert_eq!(trends[1].filler_word_count, Some(6.0));
        assert_eq!(trends[1].words_per_minute, Some(108.0));
        assert_eq!(
            trends[1].overall_score,
            Some(Score::new(71).expect("score is valid"))
        );

        assert_eq!(
            test_repository
                .repository
                .list_meeting_trends(1)
                .expect("trend limit is respected")
                .len(),
            1
        );
    }

    #[test]
    fn imported_meeting_summary_persists_speaking_improvements_source() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "imported-source");

        let created = test_repository
            .repository
            .create_imported_meeting_summary(&CreateImportedMeetingSummary {
                id: SummaryId::new("imported-source-summary"),
                meeting_id: meeting.id,
                source_file_path: "/tmp/source.mp4".to_string(),
                extracted_audio_file_path: "/tmp/extracted.wav".to_string(),
                speaking_improvements_source: "voice_match".to_string(),
                body_json: "{}".to_string(),
                generated_at_ms: 5_000,
            })
            .expect("imported summary can be created");

        assert_eq!(created.speaking_improvements_source, "voice_match");
        assert_eq!(
            test_repository
                .repository
                .get_imported_meeting_summary(&created.id)
                .expect("imported summary can be read")
                .expect("imported summary exists")
                .speaking_improvements_source,
            "voice_match"
        );
        assert_eq!(
            test_repository
                .repository
                .get_imported_meeting_summary_for_meeting(&created.meeting_id)
                .expect("imported summary can be read by meeting")
                .expect("imported summary exists for meeting")
                .id,
            created.id
        );
    }

    #[test]
    fn practice_recordings_reports_and_annotations_round_trip() {
        let test_repository = repository();
        let recording = test_repository
            .repository
            .create_practice_recording(&CreatePracticeRecording {
                id: PracticeRecordingId::new("practice-round-trip"),
                title: Some("Pitch rehearsal".to_string()),
                source_kind: "imported".to_string(),
                video_file_path: "/tmp/resonance/practice.mp4".to_string(),
                extracted_audio_file_path: None,
                duration_ms: Some(60_000),
                byte_size: Some(1024),
                recorded_at_ms: 1_000,
                created_at_ms: 1_000,
                updated_at_ms: 1_000,
                analysis_status: "recorded".to_string(),
                cloud_video_used: false,
                pipeline_failure_code: None,
                pipeline_failure_message: None,
            })
            .expect("practice recording can be created");

        let updated_recording = test_repository
            .repository
            .update_practice_recording_analysis_state(
                &recording.id,
                Some("/tmp/resonance/practice.audio.wav"),
                "complete",
                false,
                None,
                2_000,
            )
            .expect("practice recording state can be updated");
        assert_eq!(
            updated_recording.extracted_audio_file_path.as_deref(),
            Some("/tmp/resonance/practice.audio.wav")
        );
        assert_eq!(updated_recording.analysis_status, "complete");

        let report = test_repository
            .repository
            .create_practice_review_report(&CreatePracticeReviewReport {
                id: PracticeReviewReportId::new("practice-round-trip-review"),
                practice_recording_id: recording.id.clone(),
                overall_score: Some(Score::new(82).expect("score is valid")),
                audio_score: Some(Score::new(82).expect("score is valid")),
                visual_score: None,
                body_json: "{\"summary\":\"Audio review\"}".to_string(),
                generated_at_ms: 3_000,
            })
            .expect("practice report can be created");
        assert_eq!(
            report.audio_score,
            Some(Score::new(82).expect("score is valid"))
        );

        let annotations = test_repository
            .repository
            .replace_practice_timeline_annotations(
                &recording.id,
                &[CreatePracticeTimelineAnnotation {
                    id: PracticeAnnotationId::new("practice-round-trip-annotation"),
                    practice_recording_id: recording.id.clone(),
                    started_at_ms: 10_000,
                    ended_at_ms: 12_000,
                    category: "pace".to_string(),
                    severity: "caution".to_string(),
                    evidence: "182 words per minute".to_string(),
                    suggestion: "Slow down around key points.".to_string(),
                    source: "audioLocal".to_string(),
                }],
            )
            .expect("practice annotations can be replaced");
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].category, "pace");

        let listed = test_repository
            .repository
            .list_practice_recordings(10, 0)
            .expect("practice recordings can be listed");
        assert_eq!(listed, vec![updated_recording]);
        assert_eq!(
            test_repository
                .repository
                .get_practice_review_report_for_recording(&recording.id)
                .expect("practice report can be loaded")
                .expect("practice report exists")
                .id,
            PracticeReviewReportId::new("practice-round-trip-review")
        );
    }

    #[test]
    fn rewrite_app_data_file_paths_updates_owned_audio_voice_and_imported_paths() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "path-rewrite");
        let unrelated_meeting =
            create_test_meeting(&test_repository.repository, "path-rewrite-unrelated");
        let old_root = "/Users/john_doe/Library/Application Support/com.orator.meetingcoach";
        let new_root = "/Users/john_doe/Library/Application Support/com.resonance.meetingcoach";
        let wildcard_lookalike_root =
            "/Users/johnXdoe/Library/Application Support/com.orator.meetingcoach";

        test_repository
            .repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: meeting.id.clone(),
                file_path: format!("{old_root}/path-rewrite.wav"),
                system_audio_file_path: Some(format!("{old_root}/path-rewrite.system.m4a")),
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(64),
                system_audio_byte_size: Some(32),
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("audio metadata can be stored");
        test_repository
            .repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: unrelated_meeting.id.clone(),
                file_path: format!("{wildcard_lookalike_root}/unrelated.wav"),
                system_audio_file_path: None,
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(64),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("unrelated audio metadata can be stored");
        test_repository
            .repository
            .upsert_voice_profile(&VoiceProfileRecord {
                sample_audio_file_path: format!("{old_root}/voice-profile/enrollment-sample.wav"),
                sample_duration_ms: Some(2_000),
                sample_byte_size: 128,
                enrolled_at_ms: 1_000,
                embedding_json: None,
                embedding_dimension: None,
                embedding_model_path: None,
                embedding_computed_at_ms: None,
            })
            .expect("voice profile can be stored");
        test_repository
            .repository
            .create_imported_meeting_summary(&CreateImportedMeetingSummary {
                id: SummaryId::new("path-rewrite-summary"),
                meeting_id: meeting.id.clone(),
                source_file_path: "/Users/example/Downloads/source.mp4".to_string(),
                extracted_audio_file_path: format!(
                    "{old_root}/imported-recordings/path-rewrite.wav"
                ),
                speaking_improvements_source: "none".to_string(),
                body_json: "{}".to_string(),
                generated_at_ms: 2_000,
            })
            .expect("imported summary can be stored");

        test_repository
            .repository
            .rewrite_app_data_file_paths(old_root, new_root)
            .expect("app data paths can be rewritten");

        let audio_metadata = test_repository
            .repository
            .get_audio_metadata(&meeting.id)
            .expect("audio metadata can be read")
            .expect("audio metadata exists");
        assert_eq!(
            audio_metadata.file_path,
            format!("{new_root}/path-rewrite.wav")
        );
        assert_eq!(
            audio_metadata.system_audio_file_path,
            Some(format!("{new_root}/path-rewrite.system.m4a"))
        );
        assert_eq!(
            test_repository
                .repository
                .get_audio_metadata(&unrelated_meeting.id)
                .expect("unrelated audio metadata can be read")
                .expect("unrelated audio metadata exists")
                .file_path,
            format!("{wildcard_lookalike_root}/unrelated.wav")
        );
        assert_eq!(
            test_repository
                .repository
                .get_voice_profile()
                .expect("voice profile can be read")
                .expect("voice profile exists")
                .sample_audio_file_path,
            format!("{new_root}/voice-profile/enrollment-sample.wav")
        );
        let imported_summary = test_repository
            .repository
            .get_imported_meeting_summary(&SummaryId::new("path-rewrite-summary"))
            .expect("imported summary can be read")
            .expect("imported summary exists");
        assert_eq!(
            imported_summary.extracted_audio_file_path,
            format!("{new_root}/imported-recordings/path-rewrite.wav")
        );
        assert_eq!(
            imported_summary.source_file_path,
            "/Users/example/Downloads/source.mp4"
        );
    }

    #[test]
    fn default_settings_are_returned_when_no_row_exists() {
        let test_repository = repository();

        assert_eq!(
            test_repository
                .repository
                .get_settings()
                .expect("settings defaults can be read"),
            ResonanceSettings::default()
        );
    }

    #[test]
    fn upsert_settings_persists_and_reads_back_values() {
        let test_repository = repository();
        let initial_settings = ResonanceSettings {
            microphone_device_id: Some("microphone-1".to_string()),
            enable_system_audio: false,
            enable_echo_cancellation: false,
            enable_realtime_nudges: false,
            raw_audio_retention_days: 30,
            analyzer_provider: AnalyzerProvider::CloudClaude,
            cloud_analysis_enabled: true,
            cloud_video_review_enabled: true,
            transcriber_bin_path: Some("/opt/homebrew/bin/whisper-cli".to_string()),
            transcriber_model_path: Some("/models/base.bin".to_string()),
            speaker_embedding_model_path: Some("/models/speaker.onnx".to_string()),
            speaker_segmentation_model_path: Some("/models/segmentation.onnx".to_string()),
            dictation_hotkey: "ctrl+option+d".to_string(),
            dictation_polish_enabled: true,
            summarizer_provider: SummarizerProvider::Ollama,
            summarizer_host: "127.0.0.1".to_string(),
            summarizer_port: 11434,
            summarizer_model: Some("llama3.2".to_string()),
        };
        let updated_settings = ResonanceSettings {
            microphone_device_id: Some("microphone-2".to_string()),
            enable_system_audio: true,
            enable_echo_cancellation: true,
            enable_realtime_nudges: true,
            raw_audio_retention_days: 14,
            analyzer_provider: AnalyzerProvider::LocalOllama,
            cloud_analysis_enabled: false,
            cloud_video_review_enabled: false,
            transcriber_bin_path: Some("/usr/local/bin/whisper-cli".to_string()),
            transcriber_model_path: Some("/models/small-q5_1.bin".to_string()),
            speaker_embedding_model_path: Some("/models/speaker-v2.onnx".to_string()),
            speaker_segmentation_model_path: Some("/models/segmentation-v2.onnx".to_string()),
            dictation_hotkey: "cmd+shift+space".to_string(),
            dictation_polish_enabled: false,
            summarizer_provider: SummarizerProvider::Custom,
            summarizer_host: "192.168.1.50".to_string(),
            summarizer_port: 8080,
            summarizer_model: Some("custom-model".to_string()),
        };

        test_repository
            .repository
            .upsert_settings(&initial_settings, 10_000)
            .expect("settings can be inserted");
        test_repository
            .repository
            .upsert_settings(&updated_settings, 20_000)
            .expect("settings can be updated");

        assert_eq!(
            test_repository
                .repository
                .get_settings()
                .expect("persisted settings can be read"),
            updated_settings
        );
    }

    #[test]
    fn voice_profile_persists_replaces_and_deletes_singleton_profile() {
        let test_repository = repository();
        let initial_profile = VoiceProfileRecord {
            sample_audio_file_path: "/tmp/resonance/voice-sample-1.wav".to_string(),
            sample_duration_ms: Some(10_000),
            sample_byte_size: 320_000,
            enrolled_at_ms: 50_000,
            embedding_json: None,
            embedding_dimension: None,
            embedding_model_path: None,
            embedding_computed_at_ms: None,
        };
        let updated_profile = VoiceProfileRecord {
            sample_audio_file_path: "/tmp/resonance/voice-sample-2.wav".to_string(),
            sample_duration_ms: Some(12_000),
            sample_byte_size: 384_000,
            enrolled_at_ms: 60_000,
            embedding_json: Some("[0.1,0.2,0.3]".to_string()),
            embedding_dimension: Some(3),
            embedding_model_path: Some("/models/speaker.onnx".to_string()),
            embedding_computed_at_ms: Some(61_000),
        };

        test_repository
            .repository
            .upsert_voice_profile(&initial_profile)
            .expect("voice profile can be inserted");
        test_repository
            .repository
            .upsert_voice_profile(&updated_profile)
            .expect("voice profile can be replaced");

        assert_eq!(
            test_repository
                .repository
                .get_voice_profile()
                .expect("voice profile can be read"),
            Some(updated_profile)
        );
        assert!(test_repository
            .repository
            .delete_voice_profile()
            .expect("voice profile can be deleted"));
        assert_eq!(
            test_repository
                .repository
                .get_voice_profile()
                .expect("deleted voice profile can be queried"),
            None
        );
    }

    #[test]
    fn transcript_segments_list_in_sequence_order_and_enforce_meeting_fk() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-segments");
        let segments = vec![
            CreateTranscriptSegment {
                id: SegmentId::new("segment-2"),
                meeting_id: meeting.id.clone(),
                sequence_number: 2,
                speaker_label: Some("Speaker 2".to_string()),
                text: "Second segment".to_string(),
                started_at_ms: 2_000,
                ended_at_ms: 3_000,
                created_at_ms: 3_100,
            },
            CreateTranscriptSegment {
                id: SegmentId::new("segment-1"),
                meeting_id: meeting.id.clone(),
                sequence_number: 1,
                speaker_label: Some("Speaker 1".to_string()),
                text: "First segment".to_string(),
                started_at_ms: 1_000,
                ended_at_ms: 2_000,
                created_at_ms: 2_100,
            },
        ];

        test_repository
            .repository
            .create_transcript_segments(&segments)
            .expect("segments can be inserted");

        let ordered_ids = test_repository
            .repository
            .list_transcript_segments(&meeting.id)
            .expect("segments can be listed")
            .into_iter()
            .map(|segment| segment.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered_ids,
            vec![SegmentId::new("segment-1"), SegmentId::new("segment-2")]
        );

        let foreign_key_error = test_repository
            .repository
            .create_transcript_segments(&[CreateTranscriptSegment {
                id: SegmentId::new("segment-missing-meeting"),
                meeting_id: MeetingId::new("missing-meeting"),
                sequence_number: 1,
                speaker_label: None,
                text: "Orphan segment".to_string(),
                started_at_ms: 1_000,
                ended_at_ms: 2_000,
                created_at_ms: 2_100,
            }])
            .expect_err("missing meeting should surface a repository error");

        assert_eq!(foreign_key_error.code, "database_error");
    }

    #[test]
    fn metrics_create_and_list_for_meeting() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-metrics");

        let metric = test_repository
            .repository
            .create_metric(&CreateMetric {
                id: MetricId::new("metric-talk-ratio"),
                meeting_id: meeting.id.clone(),
                name: "talk_ratio".to_string(),
                value: 0.42,
                unit: Some("ratio".to_string()),
                created_at_ms: 4_000,
            })
            .expect("metric can be created");

        assert_eq!(
            test_repository
                .repository
                .list_metrics(&meeting.id)
                .expect("metrics can be listed"),
            vec![metric]
        );
    }

    #[test]
    fn reports_persist_bounded_score_and_can_be_read_and_listed() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-reports");
        let score = Score::new(87).expect("score is within bounds");

        let report = test_repository
            .repository
            .create_report(&CreateReport {
                id: ReportId::new("report-1"),
                meeting_id: meeting.id.clone(),
                overall_score: score,
                body_json: "{\"summary\":\"Clear next steps\"}".to_string(),
                generated_at_ms: 5_000,
            })
            .expect("report can be created");

        assert_eq!(report.overall_score, score);
        assert_eq!(
            test_repository
                .repository
                .get_report(&ReportId::new("report-1"))
                .expect("report can be read by id"),
            Some(report.clone())
        );
        assert_eq!(
            test_repository
                .repository
                .list_reports_for_meeting(&meeting.id)
                .expect("reports can be listed for meeting"),
            vec![report.clone()]
        );
        assert_eq!(
            test_repository
                .repository
                .list_recent_reports(10)
                .expect("recent reports can be listed"),
            vec![report]
        );
    }

    #[test]
    fn duplicate_report_for_meeting_returns_explicit_conflict() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-duplicate-report");
        let score = Score::new(87).expect("score is within bounds");
        test_repository
            .repository
            .create_report(&CreateReport {
                id: ReportId::new("report-duplicate-1"),
                meeting_id: meeting.id.clone(),
                overall_score: score,
                body_json: "{\"summary\":\"First report\"}".to_string(),
                generated_at_ms: 5_000,
            })
            .expect("first report can be created");

        let error = test_repository
            .repository
            .create_report(&CreateReport {
                id: ReportId::new("report-duplicate-2"),
                meeting_id: meeting.id.clone(),
                overall_score: score,
                body_json: "{\"summary\":\"Second report\"}".to_string(),
                generated_at_ms: 6_000,
            })
            .expect_err("duplicate report is mapped before surfacing raw DB details");

        assert_eq!(error.code, "report_already_exists");
    }

    #[test]
    fn audio_metadata_upsert_updates_existing_metadata() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-audio");
        let initial_metadata = AudioMetadata {
            meeting_id: meeting.id.clone(),
            file_path: "recordings/meeting-audio-initial.wav".to_string(),
            system_audio_file_path: None,
            duration_ms: Some(1_000),
            sample_rate_hz: Some(44_100),
            byte_size: Some(4_096),
            system_audio_byte_size: None,
            system_audio_stream_error: None,
            created_at_ms: 6_000,
        };
        let updated_metadata = AudioMetadata {
            meeting_id: meeting.id.clone(),
            file_path: "recordings/meeting-audio-updated.wav".to_string(),
            system_audio_file_path: Some("recordings/meeting-audio-updated.system.m4a".to_string()),
            duration_ms: Some(2_000),
            sample_rate_hz: Some(48_000),
            byte_size: Some(8_192),
            system_audio_byte_size: Some(16_384),
            system_audio_stream_error: None,
            created_at_ms: 7_000,
        };

        test_repository
            .repository
            .upsert_audio_metadata(&initial_metadata)
            .expect("audio metadata can be inserted");
        test_repository
            .repository
            .upsert_audio_metadata(&updated_metadata)
            .expect("audio metadata can be updated");

        assert_eq!(
            test_repository
                .repository
                .get_audio_metadata(&meeting.id)
                .expect("audio metadata can be read"),
            Some(updated_metadata)
        );
    }

    #[test]
    fn audio_metadata_retention_query_and_delete_preserve_meeting() {
        let test_repository = repository();
        let older_meeting = create_test_meeting(&test_repository.repository, "old-audio");
        let newer_meeting = create_test_meeting(&test_repository.repository, "new-audio");

        test_repository
            .repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: older_meeting.id.clone(),
                file_path: "/tmp/old.wav".to_string(),
                system_audio_file_path: Some("/tmp/old.system.m4a".to_string()),
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(128),
                system_audio_byte_size: Some(64),
                system_audio_stream_error: None,
                created_at_ms: 1_000,
            })
            .expect("old metadata can be created");
        test_repository
            .repository
            .upsert_audio_metadata(&AudioMetadata {
                meeting_id: newer_meeting.id.clone(),
                file_path: "/tmp/new.wav".to_string(),
                system_audio_file_path: None,
                duration_ms: Some(1_000),
                sample_rate_hz: Some(48_000),
                byte_size: Some(128),
                system_audio_byte_size: None,
                system_audio_stream_error: None,
                created_at_ms: 3_000,
            })
            .expect("new metadata can be created");

        let expired = test_repository
            .repository
            .list_audio_metadata_before(2_000)
            .expect("expired metadata can be listed");

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].meeting_id, older_meeting.id);
        assert!(test_repository
            .repository
            .delete_audio_metadata(&older_meeting.id)
            .expect("old metadata can be deleted"));
        assert!(test_repository
            .repository
            .get_audio_metadata(&older_meeting.id)
            .expect("old metadata lookup succeeds")
            .is_none());
        assert!(test_repository
            .repository
            .get_meeting(&older_meeting.id)
            .expect("meeting lookup succeeds")
            .is_some());
        assert!(test_repository
            .repository
            .get_audio_metadata(&newer_meeting.id)
            .expect("new metadata lookup succeeds")
            .is_some());
    }

    #[test]
    fn mark_meeting_stopped_persists_stop_time_and_rejects_missing_meeting() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-stop");

        let stopped = test_repository
            .repository
            .mark_meeting_stopped(&meeting.id, 4_000, 4_100)
            .expect("meeting can be marked stopped");

        assert_eq!(stopped.stopped_at_ms, Some(4_000));
        assert_eq!(stopped.updated_at_ms, 4_100);

        let missing = test_repository
            .repository
            .mark_meeting_stopped(&MeetingId::new("missing-meeting"), 4_000, 4_100)
            .expect_err("missing meeting cannot be marked stopped");

        assert_eq!(missing.code, "meeting_not_found");
    }

    #[test]
    fn update_meeting_title_overwrites_and_clears() {
        let test_repository = repository();
        let meeting = create_test_meeting(&test_repository.repository, "meeting-rename");

        test_repository
            .repository
            .update_meeting_title(&meeting.id, Some("Renamed by user"), 4_100)
            .expect("meeting can be renamed");
        assert_eq!(
            test_repository.repository.get_meeting(&meeting.id).unwrap().unwrap().title,
            Some("Renamed by user".to_string()),
        );

        test_repository
            .repository
            .update_meeting_title(&meeting.id, None, 4_200)
            .expect("meeting title can be cleared");
        assert_eq!(test_repository.repository.get_meeting(&meeting.id).unwrap().unwrap().title, None);

        let missing = test_repository
            .repository
            .update_meeting_title(&MeetingId::new("missing-meeting"), Some("x"), 4_200)
            .expect_err("missing meeting cannot be renamed");
        assert_eq!(missing.code, "meeting_not_found");
    }

    #[test]
    fn set_meeting_title_if_absent_fills_blank_titles_but_never_overwrites() {
        let test_repository = repository();
        let untitled = test_repository
            .repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new("meeting-untitled"),
                title: None,
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("untitled meeting can be created");

        test_repository
            .repository
            .set_meeting_title_if_absent(&untitled.id, "Model Suggested Title", 3_000)
            .expect("title fills in when absent");
        assert_eq!(
            test_repository.repository.get_meeting(&untitled.id).unwrap().unwrap().title,
            Some("Model Suggested Title".to_string()),
        );

        test_repository
            .repository
            .set_meeting_title_if_absent(&untitled.id, "Later Model Title", 4_000)
            .expect("re-summarizing does not error");
        assert_eq!(
            test_repository.repository.get_meeting(&untitled.id).unwrap().unwrap().title,
            Some("Model Suggested Title".to_string()),
            "a later auto-generated title must never overwrite the first one",
        );

        let titled = create_test_meeting(&test_repository.repository, "meeting-already-titled");
        test_repository
            .repository
            .set_meeting_title_if_absent(&titled.id, "Should Not Apply", 3_000)
            .expect("call succeeds even though title is already set");
        assert_eq!(
            test_repository.repository.get_meeting(&titled.id).unwrap().unwrap().title,
            titled.title,
            "a manually-set or pre-existing title must never be overwritten",
        );
    }

    #[test]
    fn dictation_session_can_be_created_and_listed() {
        let test_repository = repository();

        let first = test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-1"),
                started_at_ms: 1_000,
                ended_at_ms: 4_000,
                duration_ms: 3_000,
                word_count: 6,
                words_per_minute: 120.0,
                created_at_ms: 4_000,
            })
            .expect("first session can be created");
        assert_eq!(first.word_count, 6);

        let second = test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-2"),
                started_at_ms: 5_000,
                ended_at_ms: 6_000,
                duration_ms: 1_000,
                word_count: 2,
                words_per_minute: 120.0,
                created_at_ms: 6_000,
            })
            .expect("second session can be created");

        let sessions = test_repository
            .repository
            .list_dictation_sessions(10, 0)
            .expect("sessions can be listed");
        assert_eq!(sessions, vec![second, first]);
    }

    #[test]
    fn delete_dictation_session_removes_only_that_row() {
        let test_repository = repository();
        test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-keep"),
                started_at_ms: 1_000,
                ended_at_ms: 2_000,
                duration_ms: 1_000,
                word_count: 3,
                words_per_minute: 90.0,
                created_at_ms: 2_000,
            })
            .expect("first session can be created");
        test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-remove"),
                started_at_ms: 3_000,
                ended_at_ms: 4_000,
                duration_ms: 1_000,
                word_count: 5,
                words_per_minute: 150.0,
                created_at_ms: 4_000,
            })
            .expect("second session can be created");

        let deleted = test_repository
            .repository
            .delete_dictation_session(&DictationSessionId::new("dictation-remove"))
            .expect("session can be deleted");
        assert!(deleted);

        let remaining = test_repository
            .repository
            .list_dictation_sessions(10, 0)
            .expect("sessions can be listed");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, DictationSessionId::new("dictation-keep"));

        let deleted_again = test_repository
            .repository
            .delete_dictation_session(&DictationSessionId::new("dictation-remove"))
            .expect("deleting a missing session does not error");
        assert!(!deleted_again);
    }

    #[test]
    fn dictation_stats_summary_aggregates_sessions() {
        let test_repository = repository();

        let empty = test_repository
            .repository
            .get_dictation_stats_summary()
            .expect("empty summary can be read");
        assert_eq!(empty.total_sessions, 0);
        assert_eq!(empty.total_words, 0);
        assert_eq!(empty.average_words_per_minute, 0.0);
        assert_eq!(empty.total_duration_ms, 0);

        test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-1"),
                started_at_ms: 1_000,
                ended_at_ms: 4_000,
                duration_ms: 3_000,
                word_count: 6,
                words_per_minute: 100.0,
                created_at_ms: 4_000,
            })
            .expect("first session can be created");
        test_repository
            .repository
            .create_dictation_session(&CreateDictationSession {
                id: DictationSessionId::new("dictation-2"),
                started_at_ms: 5_000,
                ended_at_ms: 7_000,
                duration_ms: 2_000,
                word_count: 4,
                words_per_minute: 140.0,
                created_at_ms: 7_000,
            })
            .expect("second session can be created");

        let summary = test_repository
            .repository
            .get_dictation_stats_summary()
            .expect("summary can be read");
        assert_eq!(summary.total_sessions, 2);
        assert_eq!(summary.total_words, 10);
        assert_eq!(summary.average_words_per_minute, 120.0);
        assert_eq!(summary.total_duration_ms, 5_000);
    }

    fn repository() -> TestRepository {
        let database = NamedTempFile::new().expect("temp database can be created");
        let repository = SqliteRepository::open(database.path()).expect("repository can be opened");
        TestRepository {
            _database: database,
            repository,
        }
    }

    fn create_test_meeting(repository: &SqliteRepository, id: &str) -> MeetingRecord {
        repository
            .create_meeting(&CreateMeeting {
                id: MeetingId::new(id),
                title: Some(format!("Test meeting {id}")),
                started_at_ms: 1_000,
                stopped_at_ms: Some(2_000),
                created_at_ms: 1_000,
                updated_at_ms: 2_000,
            })
            .expect("test meeting can be created")
    }
}
