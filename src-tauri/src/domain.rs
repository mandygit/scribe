use serde::{Deserialize, Serialize};

macro_rules! new_string_id {
    ($name:ident) => {
        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MeetingId(String);

new_string_id!(MeetingId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SegmentId(String);

new_string_id!(SegmentId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MetricId(String);

new_string_id!(MetricId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ReportId(String);

new_string_id!(ReportId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SummaryId(String);

new_string_id!(SummaryId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DictationSessionId(String);

new_string_id!(DictationSessionId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PracticeRecordingId(String);

new_string_id!(PracticeRecordingId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PracticeReviewReportId(String);

new_string_id!(PracticeReviewReportId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PracticeAnnotationId(String);

new_string_id!(PracticeAnnotationId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MeetingLifecycleState {
    Idle,
    Recording {
        meeting_id: MeetingId,
        started_at_ms: u64,
    },
    Stopping {
        meeting_id: MeetingId,
        started_at_ms: u64,
        stopped_at_ms: u64,
    },
    Transcribing {
        meeting_id: MeetingId,
        started_at_ms: u64,
        stopped_at_ms: u64,
        audio_path: String,
    },
    Analyzing {
        meeting_id: MeetingId,
        started_at_ms: u64,
        stopped_at_ms: u64,
        audio_path: String,
        segment_count: u32,
    },
    Complete {
        meeting_id: MeetingId,
        started_at_ms: u64,
        stopped_at_ms: u64,
        audio_path: String,
        segment_count: u32,
        report_id: ReportId,
    },
    FailedPartial {
        meeting_id: Option<MeetingId>,
        failed_stage: ProcessingStage,
        error: AppError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingLifecycleEvent {
    StartRequested {
        meeting_id: MeetingId,
        started_at_ms: u64,
    },
    StopRequested {
        stopped_at_ms: u64,
    },
    AudioSaved {
        audio_path: String,
    },
    TranscriptionCompleted {
        segment_count: u32,
    },
    AnalysisCompleted {
        report_id: ReportId,
    },
    Failed {
        failed_stage: ProcessingStage,
        error: AppError,
    },
    ResetRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessingStage {
    Recording,
    Transcribing,
    Metrics,
    Analyzing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalyzerProvider {
    LocalOllama,
    CloudOpenAi,
    CloudClaude,
}

/// Which local model server backs meeting-notes summarization. Distinct from
/// [`AnalyzerProvider`], which governs an unrelated cloud-vs-local toggle for
/// the practice-recording coaching feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SummarizerProvider {
    LmStudio,
    Ollama,
    Custom,
}

/// The user's UI appearance preference. `System` follows the OS light/dark
/// setting; `Light`/`Dark` pin the app regardless of the OS setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResonanceSettings {
    pub microphone_device_id: Option<String>,
    pub enable_system_audio: bool,
    pub enable_echo_cancellation: bool,
    pub enable_realtime_nudges: bool,
    pub raw_audio_retention_days: u16,
    pub analyzer_provider: AnalyzerProvider,
    pub cloud_analysis_enabled: bool,
    pub cloud_video_review_enabled: bool,
    pub transcriber_bin_path: Option<String>,
    pub transcriber_model_path: Option<String>,
    pub speaker_embedding_model_path: Option<String>,
    pub speaker_segmentation_model_path: Option<String>,
    /// Canonical token for the system-wide dictation hotkey (e.g. `cmd+shift+d`).
    pub dictation_hotkey: String,
    /// When true, dictation runs Apple Intelligence polish before inserting;
    /// otherwise the raw transcript is inserted.
    pub dictation_polish_enabled: bool,
    /// Which local model server to summarize meetings with.
    pub summarizer_provider: SummarizerProvider,
    pub summarizer_host: String,
    pub summarizer_port: u16,
    /// Model name/id to request. When unset, falls back to
    /// [`crate::summarizer::DEFAULT_SUMMARIZER_MODEL`] for the LM Studio provider.
    pub summarizer_model: Option<String>,
    /// UI appearance preference; defaults to following the OS setting.
    pub theme_preference: ThemePreference,
}

/// Default token for the dictation hotkey: double-press Cmd+Shift+D.
pub const DEFAULT_DICTATION_HOTKEY: &str = "cmd+shift+d";

/// Default LM Studio host/port, matching `summarizer::LmStudioClient`'s default.
pub const DEFAULT_SUMMARIZER_HOST: &str = "127.0.0.1";
pub const DEFAULT_SUMMARIZER_PORT: u16 = 1234;

impl Default for ResonanceSettings {
    fn default() -> Self {
        Self {
            microphone_device_id: None,
            enable_system_audio: true,
            enable_echo_cancellation: true,
            enable_realtime_nudges: true,
            raw_audio_retention_days: 7,
            analyzer_provider: AnalyzerProvider::LocalOllama,
            cloud_analysis_enabled: false,
            cloud_video_review_enabled: false,
            transcriber_bin_path: None,
            transcriber_model_path: None,
            speaker_embedding_model_path: None,
            speaker_segmentation_model_path: None,
            dictation_hotkey: DEFAULT_DICTATION_HOTKEY.to_string(),
            dictation_polish_enabled: false,
            summarizer_provider: SummarizerProvider::LmStudio,
            summarizer_host: DEFAULT_SUMMARIZER_HOST.to_string(),
            summarizer_port: DEFAULT_SUMMARIZER_PORT,
            summarizer_model: None,
            theme_preference: ThemePreference::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Score(u8);

impl Score {
    pub fn new(value: u8) -> Result<Self, AppError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(AppError {
                code: "invalid_score".to_string(),
                message: "Scores must be between 0 and 100.".to_string(),
                details: Some(format!("value={value}")),
            })
        }
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingReportSummary {
    pub report_id: ReportId,
    pub meeting_id: MeetingId,
    pub overall_score: Score,
    pub generated_at_ms: u64,
}

impl MeetingLifecycleState {
    pub fn transition(self, event: MeetingLifecycleEvent) -> Result<Self, AppError> {
        match (self, event) {
            (
                Self::Idle,
                MeetingLifecycleEvent::StartRequested {
                    meeting_id,
                    started_at_ms,
                },
            ) => Ok(Self::Recording {
                meeting_id,
                started_at_ms,
            }),
            (
                Self::Recording {
                    meeting_id,
                    started_at_ms,
                },
                MeetingLifecycleEvent::StopRequested { stopped_at_ms },
            ) => {
                if stopped_at_ms < started_at_ms {
                    return Err(AppError {
                        code: "invalid_lifecycle_timestamp".to_string(),
                        message: "Meeting cannot stop before it starts.".to_string(),
                        details: Some(format!(
                            "started_at_ms={started_at_ms}, stopped_at_ms={stopped_at_ms}"
                        )),
                    });
                }

                Ok(Self::Stopping {
                    meeting_id,
                    started_at_ms,
                    stopped_at_ms,
                })
            }
            (
                Self::Stopping {
                    meeting_id,
                    started_at_ms,
                    stopped_at_ms,
                },
                MeetingLifecycleEvent::AudioSaved { audio_path },
            ) => Ok(Self::Transcribing {
                meeting_id,
                started_at_ms,
                stopped_at_ms,
                audio_path,
            }),
            (
                Self::Transcribing {
                    meeting_id,
                    started_at_ms,
                    stopped_at_ms,
                    audio_path,
                },
                MeetingLifecycleEvent::TranscriptionCompleted { segment_count },
            ) => Ok(Self::Analyzing {
                meeting_id,
                started_at_ms,
                stopped_at_ms,
                audio_path,
                segment_count,
            }),
            (
                Self::Analyzing {
                    meeting_id,
                    started_at_ms,
                    stopped_at_ms,
                    audio_path,
                    segment_count,
                },
                MeetingLifecycleEvent::AnalysisCompleted { report_id },
            ) => Ok(Self::Complete {
                meeting_id,
                started_at_ms,
                stopped_at_ms,
                audio_path,
                segment_count,
                report_id,
            }),
            (
                Self::Complete { .. } | Self::FailedPartial { .. },
                MeetingLifecycleEvent::ResetRequested,
            ) => Ok(Self::Idle),
            (
                state @ (Self::Recording { .. }
                | Self::Stopping { .. }
                | Self::Transcribing { .. }
                | Self::Analyzing { .. }),
                MeetingLifecycleEvent::Failed {
                    failed_stage,
                    error,
                },
            ) => Ok(Self::FailedPartial {
                meeting_id: state.meeting_id(),
                failed_stage,
                error,
            }),
            (state, event) => Err(AppError {
                code: "invalid_lifecycle_transition".to_string(),
                message: "Meeting lifecycle event is not valid for the current state.".to_string(),
                details: Some(format!("state={state:?}, event={event:?}")),
            }),
        }
    }

    fn meeting_id(&self) -> Option<MeetingId> {
        match self {
            Self::Idle => None,
            Self::Recording { meeting_id, .. }
            | Self::Stopping { meeting_id, .. }
            | Self::Transcribing { meeting_id, .. }
            | Self::Analyzing { meeting_id, .. }
            | Self::Complete { meeting_id, .. } => Some(meeting_id.clone()),
            Self::FailedPartial { meeting_id, .. } => meeting_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_can_transition_to_stopping() {
        let state = MeetingLifecycleState::Recording {
            meeting_id: MeetingId::new("meeting-1"),
            started_at_ms: 1_000,
        };

        let next = state
            .transition(MeetingLifecycleEvent::StopRequested {
                stopped_at_ms: 5_000,
            })
            .expect("recording can stop");

        assert_eq!(
            next,
            MeetingLifecycleState::Stopping {
                meeting_id: MeetingId::new("meeting-1"),
                started_at_ms: 1_000,
                stopped_at_ms: 5_000,
            }
        );
    }

    #[test]
    fn idle_rejects_stop_request() {
        let error = MeetingLifecycleState::Idle
            .transition(MeetingLifecycleEvent::StopRequested {
                stopped_at_ms: 5_000,
            })
            .expect_err("idle cannot stop");

        assert_eq!(error.code, "invalid_lifecycle_transition");
    }

    #[test]
    fn recording_rejects_stop_before_start() {
        let error = MeetingLifecycleState::Recording {
            meeting_id: MeetingId::new("meeting-early-stop"),
            started_at_ms: 5_000,
        }
        .transition(MeetingLifecycleEvent::StopRequested {
            stopped_at_ms: 1_000,
        })
        .expect_err("meeting cannot stop before it starts");

        assert_eq!(error.code, "invalid_lifecycle_timestamp");
    }

    #[test]
    fn active_lifecycle_can_fail_into_partial_state() {
        let source_error = AppError {
            code: "audio_write_failed".to_string(),
            message: "Could not save meeting audio.".to_string(),
            details: Some("path=/tmp/meeting.m4a".to_string()),
        };

        let state = MeetingLifecycleState::Transcribing {
            meeting_id: MeetingId::new("meeting-failed"),
            started_at_ms: 1_000,
            stopped_at_ms: 5_000,
            audio_path: "/tmp/meeting.m4a".to_string(),
        }
        .transition(MeetingLifecycleEvent::Failed {
            failed_stage: ProcessingStage::Transcribing,
            error: source_error.clone(),
        })
        .expect("active lifecycle can fail into a partial report");

        assert_eq!(
            state,
            MeetingLifecycleState::FailedPartial {
                meeting_id: Some(MeetingId::new("meeting-failed")),
                failed_stage: ProcessingStage::Transcribing,
                error: source_error,
            }
        );
    }

    #[test]
    fn idle_rejects_failure_without_active_meeting() {
        let error = MeetingLifecycleState::Idle
            .transition(MeetingLifecycleEvent::Failed {
                failed_stage: ProcessingStage::Recording,
                error: AppError {
                    code: "recording_failed".to_string(),
                    message: "Recording failed without an active meeting.".to_string(),
                    details: None,
                },
            })
            .expect_err("idle should not create a failed partial meeting");

        assert_eq!(error.code, "invalid_lifecycle_transition");
    }

    #[test]
    fn terminal_lifecycle_rejects_late_failure() {
        let error = MeetingLifecycleState::Complete {
            meeting_id: MeetingId::new("meeting-complete"),
            started_at_ms: 1_000,
            stopped_at_ms: 5_000,
            audio_path: "/tmp/meeting.m4a".to_string(),
            segment_count: 12,
            report_id: ReportId::new("report-complete"),
        }
        .transition(MeetingLifecycleEvent::Failed {
            failed_stage: ProcessingStage::Analyzing,
            error: AppError {
                code: "late_failure".to_string(),
                message: "Late failure should not overwrite completion.".to_string(),
                details: None,
            },
        })
        .expect_err("complete lifecycle should not become failed partial");

        assert_eq!(error.code, "invalid_lifecycle_transition");
    }

    #[test]
    fn terminal_lifecycle_can_reset_to_idle() {
        let state = MeetingLifecycleState::FailedPartial {
            meeting_id: Some(MeetingId::new("meeting-reset")),
            failed_stage: ProcessingStage::Analyzing,
            error: AppError {
                code: "analysis_failed".to_string(),
                message: "Analyzer failed.".to_string(),
                details: None,
            },
        }
        .transition(MeetingLifecycleEvent::ResetRequested)
        .expect("terminal lifecycle can reset");

        assert_eq!(state, MeetingLifecycleState::Idle);
    }

    #[test]
    fn lifecycle_reaches_complete_through_valid_events() {
        let state = MeetingLifecycleState::Idle
            .transition(MeetingLifecycleEvent::StartRequested {
                meeting_id: MeetingId::new("meeting-2"),
                started_at_ms: 1_000,
            })
            .expect("idle can start")
            .transition(MeetingLifecycleEvent::StopRequested {
                stopped_at_ms: 5_000,
            })
            .expect("recording can stop")
            .transition(MeetingLifecycleEvent::AudioSaved {
                audio_path: "/tmp/meeting-2.m4a".to_string(),
            })
            .expect("stopped meeting can transcribe")
            .transition(MeetingLifecycleEvent::TranscriptionCompleted { segment_count: 12 })
            .expect("transcribed meeting can analyze")
            .transition(MeetingLifecycleEvent::AnalysisCompleted {
                report_id: ReportId::new("report-1"),
            })
            .expect("analyzed meeting can complete");

        assert_eq!(
            state,
            MeetingLifecycleState::Complete {
                meeting_id: MeetingId::new("meeting-2"),
                started_at_ms: 1_000,
                stopped_at_ms: 5_000,
                audio_path: "/tmp/meeting-2.m4a".to_string(),
                segment_count: 12,
                report_id: ReportId::new("report-1"),
            }
        );
    }

    #[test]
    fn default_settings_are_local_first_and_private() {
        let settings = ResonanceSettings::default();

        assert_eq!(settings.raw_audio_retention_days, 7);
        assert!(settings.enable_system_audio);
        assert!(settings.enable_echo_cancellation);
        assert!(settings.enable_realtime_nudges);
        assert_eq!(settings.analyzer_provider, AnalyzerProvider::LocalOllama);
        assert!(!settings.cloud_analysis_enabled);
        assert!(!settings.cloud_video_review_enabled);
        assert_eq!(settings.transcriber_bin_path, None);
        assert_eq!(settings.transcriber_model_path, None);
        assert_eq!(settings.speaker_embedding_model_path, None);
        assert_eq!(settings.speaker_segmentation_model_path, None);
    }

    #[test]
    fn score_rejects_values_outside_zero_to_one_hundred() {
        assert!(Score::new(0).is_ok());
        assert!(Score::new(100).is_ok());
        assert_eq!(
            Score::new(101)
                .expect_err("score above 100 is invalid")
                .code,
            "invalid_score"
        );
    }
}
