use serde::Serialize;

/// Input audio device exposed to the Tauri command layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default_input: bool,
}

/// Metadata returned immediately after microphone recording starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStarted {
    pub meeting_id: String,
    pub file_path: String,
    pub system_audio_file_path: Option<String>,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
}

/// Finalized recording metadata returned after microphone recording stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingMetadata {
    pub meeting_id: String,
    pub file_path: String,
    pub system_audio_file_path: Option<String>,
    pub duration_ms: u64,
    pub sample_rate_hz: u32,
    pub byte_size: u64,
    pub system_audio_byte_size: Option<u64>,
    pub started_at_ms: u64,
    pub stopped_at_ms: u64,
    pub dropped_sample_count: u64,
    pub stream_error: Option<String>,
    pub system_audio_stream_error: Option<String>,
}
