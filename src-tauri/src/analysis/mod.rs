use serde::{Deserialize, Serialize};

use crate::domain::AppError;

/// Strategy interface for imported-recording summarizers.
pub trait MeetingSummarizer {
    fn summarize(
        &self,
        transcript_segments: &[AnalysisTranscriptSegment],
        include_speaking_improvements: bool,
    ) -> Result<MeetingSummary, AppError>;
}

/// One transcript segment available as evidence for analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisTranscriptSegment {
    pub sequence_number: u32,
    pub speaker_label: Option<String>,
    pub speaker_role: TranscriptSpeakerRole,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

/// Analyzer-facing speaker role derived locally from persisted speaker labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptSpeakerRole {
    User,
    Context,
}

/// Parsed summary for an imported meeting recording.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub executive_summary: String,
    pub action_items: Vec<MeetingActionItem>,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    pub speaking_improvements: Vec<SpeakingImprovement>,
    /// Short model-suggested meeting title. Only used to fill in a meeting's
    /// title when it doesn't already have one — see
    /// `SqliteRepository::set_meeting_title_if_absent`.
    #[serde(default)]
    pub meeting_title: Option<String>,
}

/// One action item extracted from a downloaded meeting recording.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingActionItem {
    pub owner: Option<String>,
    pub task: String,
    pub due: Option<String>,
}

/// One transcript-grounded speaking improvement for imported recordings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakingImprovement {
    pub category: String,
    pub quote: String,
    pub suggestion: String,
}
