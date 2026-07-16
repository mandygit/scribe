use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{SyncSender, TrySendError},
};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::domain::{AppError, ScribeSettings};

const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &["wav", "m4a", "mp3", "flac", "ogg"];
const HOMEBREW_WHISPER_PATHS: &[&str] = &[
    "/opt/homebrew/bin/whisper-cli",
    "/usr/local/bin/whisper-cli",
];
const MAX_WHISPER_JSON_BYTES: u64 = 16 * 1024 * 1024;
const WHISPER_INPUT_SAMPLE_RATE_HZ: u32 = 16_000;
pub const TRANSCRIPT_SEGMENT_EVENT: &str = "scribe://transcript-segment";
pub const TRANSCRIPT_STREAM_COMPLETE_EVENT: &str = "scribe://transcript-stream-complete";

/// Speaker label for microphone-track segments in dual-track transcripts.
pub const USER_SPEAKER_LABEL: &str = "You";
/// Speaker label for system-audio-track segments (remote participants).
pub const OTHERS_SPEAKER_LABEL: &str = "Others";

/// Whether a segment belongs to the user for coaching purposes (metrics,
/// nudges). Unlabeled segments come from microphone-only transcripts and
/// count as the user's.
pub fn is_user_segment(speaker_label: Option<&str>) -> bool {
    match speaker_label {
        None => true,
        Some(label) => label == USER_SPEAKER_LABEL,
    }
}

/// Strategy interface for batch transcription providers.
pub trait Transcriber {
    fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionOutput, AppError>;
}

/// One timestamped transcript segment returned by a transcriber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub sequence_number: u32,
    pub speaker_label: Option<String>,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

/// Batch transcription output before persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionOutput {
    pub segments: Vec<TranscriptSegment>,
}

/// Incremental transcript segment event shared by streaming and batch adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStreamEvent {
    pub meeting_id: String,
    pub segment: TranscriptSegment,
    pub is_final: bool,
}

/// Summary emitted when a transcript stream finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStreamSummary {
    pub meeting_id: String,
    pub segment_count: u32,
    pub dropped_event_count: u64,
}

/// Event sink used by streaming transcription strategies.
pub trait TranscriptEventSink {
    fn emit_segment(&mut self, event: TranscriptStreamEvent) -> Result<(), AppError>;

    fn complete(&mut self, summary: TranscriptStreamSummary) -> Result<(), AppError>;

    fn dropped_event_count(&self) -> u64 {
        0
    }
}

/// Strategy interface for providers that can surface segments incrementally.
pub trait StreamingTranscriber {
    fn stream_transcribe(
        &self,
        meeting_id: &str,
        audio_path: &Path,
        sink: &mut dyn TranscriptEventSink,
    ) -> Result<TranscriptStreamSummary, AppError>;
}

/// Bounded sink for live transcript events. Overflow drops UI events instead of
/// allowing unbounded memory growth; final persistence remains source-of-truth.
pub struct BoundedTranscriptEventSink {
    sender: SyncSender<TranscriptStreamEvent>,
    dropped_event_count: u64,
}

impl BoundedTranscriptEventSink {
    pub fn new(sender: SyncSender<TranscriptStreamEvent>) -> Self {
        Self {
            sender,
            dropped_event_count: 0,
        }
    }
}

impl TranscriptEventSink for BoundedTranscriptEventSink {
    fn emit_segment(&mut self, event: TranscriptStreamEvent) -> Result<(), AppError> {
        match self.sender.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.dropped_event_count = self.dropped_event_count.saturating_add(1);
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(transcription_error(
                "transcript_stream_disconnected",
                "Transcript stream receiver disconnected before transcription completed.",
                None,
            )),
        }
    }

    fn complete(&mut self, _summary: TranscriptStreamSummary) -> Result<(), AppError> {
        Ok(())
    }

    fn dropped_event_count(&self) -> u64 {
        self.dropped_event_count
    }
}

/// Adapter that exposes any batch transcriber through the streaming contract.
pub struct BatchReplayStreamingTranscriber<T> {
    transcriber: T,
}

impl<T> BatchReplayStreamingTranscriber<T> {
    pub fn new(transcriber: T) -> Self {
        Self { transcriber }
    }
}

impl<T: Transcriber> StreamingTranscriber for BatchReplayStreamingTranscriber<T> {
    fn stream_transcribe(
        &self,
        meeting_id: &str,
        audio_path: &Path,
        sink: &mut dyn TranscriptEventSink,
    ) -> Result<TranscriptStreamSummary, AppError> {
        let output = self.transcriber.transcribe(audio_path)?;
        replay_transcription_output(meeting_id, &output, sink)
    }
}

/// Shell-based whisper.cpp batch transcriber using direct argv construction.
pub struct WhisperShellTranscriber {
    binary_path: PathBuf,
    model_path: PathBuf,
    vocabulary_prompt: Option<String>,
}

impl WhisperShellTranscriber {
    pub fn from_settings(settings: &ScribeSettings) -> Result<Self, AppError> {
        let binary_path = match &settings.transcriber_bin_path {
            Some(path) => validate_binary_path(Path::new(path))?,
            None => resolve_default_binary_path()?,
        };
        let vocabulary_prompt =
            normalized_vocabulary_prompt(settings.transcriber_vocabulary.as_deref());
        let model_path = settings
            .transcriber_model_path
            .as_deref()
            .ok_or_else(|| {
                transcription_error(
                    "transcriber_model_not_configured",
                    "Configure a whisper.cpp model path before transcribing meetings.",
                    Some(
                        "Download a whisper.cpp model and save its absolute path in settings."
                            .to_string(),
                    ),
                )
            })
            .and_then(|path| validate_model_path(Path::new(path)))?;

        Ok(Self {
            binary_path,
            model_path,
            vocabulary_prompt,
        })
    }

    pub fn new(
        binary_path: impl Into<PathBuf>,
        model_path: impl Into<PathBuf>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            binary_path: validate_binary_path(&binary_path.into())?,
            model_path: validate_model_path(&model_path.into())?,
            vocabulary_prompt: None,
        })
    }
}

impl Transcriber for WhisperShellTranscriber {
    fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
        let audio_path = validate_audio_path(audio_path)?;
        let output_dir = TempDir::new().map_err(|error| {
            transcription_error(
                "transcription_temp_dir_failed",
                "Could not create a temporary transcription output directory.",
                Some(error.to_string()),
            )
        })?;
        let whisper_input = prepare_whisper_input(&audio_path, output_dir.path())?;
        let output_stem = output_dir.path().join("transcript");
        let mut command = Command::new(&self.binary_path);
        command
            .arg("-m")
            .arg(&self.model_path)
            .arg("-f")
            .arg(&whisper_input);
        // Rolling text context must never survive between decode windows:
        // with carryover, a stretch of non-speech audio (music playing before
        // a meeting starts) locks the decoder into repeating "[Music]" for
        // the remainder of the file. Without a vocabulary that is "-mc 0".
        // With one, "-mc" is sized to the glossary prompt itself: whisper.cpp
        // fills the context budget with the carried initial prompt first and
        // only gives rolling context the leftover, so a budget the size of
        // the prompt keeps the bias while starving carryover to zero.
        match &self.vocabulary_prompt {
            Some(prompt) => {
                command
                    .arg("-mc")
                    .arg(vocabulary_max_context(prompt).to_string())
                    .arg("--prompt")
                    .arg(prompt)
                    .arg("--carry-initial-prompt");
            }
            None => {
                command.arg("-mc").arg("0");
            }
        }
        let output = command
            .arg("-ojf")
            .arg("-of")
            .arg(&output_stem)
            .output()
            .map_err(|error| {
                transcription_error(
                    "transcription_command_failed",
                    "Could not start whisper-cli.",
                    Some(error.to_string()),
                )
            })?;

        if !output.status.success() {
            return Err(transcription_error(
                "transcription_command_failed",
                "whisper-cli failed to transcribe the audio file.",
                Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            ));
        }

        let json_path = output_stem.with_extension("json");
        read_whisper_json_file(&json_path)
    }
}

/// Turns the user's raw vocabulary text (comma- or newline-separated terms)
/// into the whisper initial prompt, e.g. "Glossary: SymbioRAG, Jira.".
/// Returns `None` when there are no usable terms.
pub fn normalized_vocabulary_prompt(raw: Option<&str>) -> Option<String> {
    let terms: Vec<&str> = raw?
        .split(['\n', ','])
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(format!("Glossary: {}.", terms.join(", ")))
}

/// Text-context budget sized to the vocabulary prompt: a conservative token
/// estimate (BPE averages under four characters per token for product names)
/// so the carried prompt consumes the whole "-mc" budget and rolling context
/// gets nothing. Underestimating only trims the glossary's leading terms;
/// overestimating would leak rolling context back in.
fn vocabulary_max_context(prompt: &str) -> usize {
    prompt.chars().count() / 4 + 1
}

/// whisper-cli only decodes WAV input, so compressed sources (the system
/// audio m4a, imported mp3/ogg files) are converted to 16 kHz mono WAV first.
fn prepare_whisper_input(audio_path: &Path, temp_dir: &Path) -> Result<PathBuf, AppError> {
    if path_has_extension(audio_path, "wav") {
        return Ok(audio_path.to_path_buf());
    }

    let converted_path = temp_dir.join("whisper-input.wav");
    let ffmpeg_path = crate::media_import::resolve_ffmpeg_path(None)?;
    let output = Command::new(ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(audio_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg(WHISPER_INPUT_SAMPLE_RATE_HZ.to_string())
        .arg("-sample_fmt")
        .arg("s16")
        .arg("-f")
        .arg("wav")
        .arg(&converted_path)
        .output()
        .map_err(|error| {
            transcription_error(
                "transcription_audio_conversion_failed",
                "Could not start ffmpeg to convert audio for transcription.",
                Some(error.to_string()),
            )
        })?;

    if !output.status.success() {
        return Err(transcription_error(
            "transcription_audio_conversion_failed",
            "ffmpeg could not convert the audio file for transcription.",
            Some(format!(
                "path={}, stderr={}",
                audio_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        ));
    }

    Ok(converted_path)
}

fn path_has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

/// Merges the microphone and system-audio transcripts into one timeline.
/// Microphone segments are attributed to the user; system-audio segments to
/// the remote participants, since the system track is the output mix that the
/// user's own voice never enters.
pub fn merge_dual_track_outputs(
    microphone: TranscriptionOutput,
    system: TranscriptionOutput,
) -> Result<TranscriptionOutput, AppError> {
    let mut segments: Vec<TranscriptSegment> = microphone
        .segments
        .into_iter()
        .map(|segment| TranscriptSegment {
            speaker_label: Some(USER_SPEAKER_LABEL.to_string()),
            ..segment
        })
        .chain(
            system
                .segments
                .into_iter()
                .map(|segment| TranscriptSegment {
                    speaker_label: Some(OTHERS_SPEAKER_LABEL.to_string()),
                    ..segment
                }),
        )
        .collect();
    segments.sort_by_key(|segment| (segment.started_at_ms, segment.ended_at_ms));

    for (index, segment) in segments.iter_mut().enumerate() {
        segment.sequence_number = u32::try_from(index + 1).map_err(|error| {
            transcription_error(
                "too_many_transcript_segments",
                "Transcription produced too many transcript segments.",
                Some(error.to_string()),
            )
        })?;
    }

    Ok(TranscriptionOutput { segments })
}

pub fn replay_transcription_output(
    meeting_id: &str,
    output: &TranscriptionOutput,
    sink: &mut dyn TranscriptEventSink,
) -> Result<TranscriptStreamSummary, AppError> {
    let segment_count = u32::try_from(output.segments.len()).map_err(|error| {
        transcription_error(
            "too_many_transcript_segments",
            "Transcription produced too many transcript segments.",
            Some(error.to_string()),
        )
    })?;

    for segment in &output.segments {
        sink.emit_segment(TranscriptStreamEvent {
            meeting_id: meeting_id.to_string(),
            segment: segment.clone(),
            is_final: true,
        })?;
    }

    let summary = TranscriptStreamSummary {
        meeting_id: meeting_id.to_string(),
        segment_count,
        dropped_event_count: sink.dropped_event_count(),
    };
    sink.complete(summary.clone())?;
    Ok(summary)
}

pub fn validate_audio_path(path: &Path) -> Result<PathBuf, AppError> {
    validate_existing_file(path, "invalid_audio_path", "Audio file path")?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| unsupported_audio_error(path))?;

    if SUPPORTED_AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Ok(path.to_path_buf())
    } else {
        Err(unsupported_audio_error(path))
    }
}

pub fn parse_whisper_json(json: &str) -> Result<TranscriptionOutput, AppError> {
    let document: WhisperJsonDocument = serde_json::from_str(json).map_err(|error| {
        transcription_error(
            "invalid_transcription_json",
            "whisper-cli produced JSON that could not be parsed.",
            Some(error.to_string()),
        )
    })?;

    let mut previous_from_ms = 0;
    let mut previous_to_ms = 0;
    let mut segments = Vec::new();

    for raw_segment in document.transcription {
        let (started_at_ms, ended_at_ms) = raw_segment.offsets_millis()?;
        if ended_at_ms < started_at_ms
            || started_at_ms < previous_from_ms
            || ended_at_ms < previous_to_ms
        {
            return Err(transcription_error(
                "invalid_transcription_offsets",
                "whisper-cli produced transcript offsets that are not non-decreasing.",
                Some(format!(
                    "from={started_at_ms}, to={ended_at_ms}, previous_from={previous_from_ms}, previous_to={previous_to_ms}"
                )),
            ));
        }

        previous_from_ms = started_at_ms;
        previous_to_ms = ended_at_ms;
        let text = raw_segment.text.trim().to_string();
        if text.is_empty() {
            continue;
        }

        segments.push(TranscriptSegment {
            sequence_number: u32::try_from(segments.len() + 1).map_err(|error| {
                transcription_error(
                    "too_many_transcript_segments",
                    "Transcription produced too many transcript segments.",
                    Some(error.to_string()),
                )
            })?,
            speaker_label: None,
            text,
            started_at_ms,
            ended_at_ms,
        });
    }

    Ok(TranscriptionOutput { segments })
}

fn read_whisper_json_file(path: &Path) -> Result<TranscriptionOutput, AppError> {
    validate_existing_file(
        path,
        "transcription_output_missing",
        "Transcription JSON output",
    )?;
    let metadata = std::fs::metadata(path).map_err(|error| {
        transcription_error(
            "transcription_output_unreadable",
            "Could not inspect transcription JSON output.",
            Some(error.to_string()),
        )
    })?;
    if metadata.len() > MAX_WHISPER_JSON_BYTES {
        return Err(transcription_error(
            "transcription_output_too_large",
            "Transcription JSON output is too large to read safely.",
            Some(format!(
                "bytes={}, max={MAX_WHISPER_JSON_BYTES}",
                metadata.len()
            )),
        ));
    }

    let json = std::fs::read_to_string(path).map_err(|error| {
        transcription_error(
            "transcription_output_unreadable",
            "Could not read transcription JSON output.",
            Some(error.to_string()),
        )
    })?;
    parse_whisper_json(&json)
}

fn resolve_default_binary_path() -> Result<PathBuf, AppError> {
    for candidate in HOMEBREW_WHISPER_PATHS {
        let path = Path::new(candidate);
        if path.exists() && path.is_file() {
            return validate_binary_path(path);
        }
    }

    if let Some(path) = find_binary_on_path("whisper-cli") {
        return validate_binary_path(&path);
    }

    Err(transcription_error(
        "transcriber_binary_not_found",
        "Could not find whisper-cli. Configure the whisper-cli binary path in settings.",
        Some("Checked common Homebrew paths and PATH.".to_string()),
    ))
}

fn find_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_value = std::env::var_os("PATH")?;
    std::env::split_paths(&path_value)
        .map(|directory| directory.join(binary_name))
        .find(|candidate| candidate.exists() && candidate.is_file())
}

fn validate_binary_path(path: &Path) -> Result<PathBuf, AppError> {
    validate_existing_file(
        path,
        "invalid_transcriber_binary_path",
        "whisper-cli binary path",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|error| {
                transcription_error(
                    "invalid_transcriber_binary_path",
                    "Could not inspect whisper-cli binary permissions.",
                    Some(error.to_string()),
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(transcription_error(
                "invalid_transcriber_binary_path",
                "whisper-cli binary path is not executable.",
                Some(format!("path={}", path.display())),
            ));
        }
    }
    Ok(path.to_path_buf())
}

fn validate_model_path(path: &Path) -> Result<PathBuf, AppError> {
    validate_existing_file(
        path,
        "invalid_transcriber_model_path",
        "Transcriber model path",
    )?;
    Ok(path.to_path_buf())
}

fn validate_existing_file(path: &Path, code: &str, label: &str) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(transcription_error(
            code,
            &format!("{label} must be absolute."),
            Some(format!("path={}", path.display())),
        ));
    }
    if !path.exists() {
        return Err(transcription_error(
            code,
            &format!("{label} does not exist."),
            Some(format!("path={}", path.display())),
        ));
    }
    if !path.is_file() {
        return Err(transcription_error(
            code,
            &format!("{label} must point to a file."),
            Some(format!("path={}", path.display())),
        ));
    }
    Ok(())
}

fn unsupported_audio_error(path: &Path) -> AppError {
    transcription_error(
        "unsupported_audio_format",
        "Audio file extension is not supported for transcription.",
        Some(format!(
            "path={}, allowed={}",
            path.display(),
            SUPPORTED_AUDIO_EXTENSIONS.join(",")
        )),
    )
}

fn transcription_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[derive(Debug, Deserialize)]
struct WhisperJsonDocument {
    #[serde(default)]
    transcription: Vec<WhisperJsonSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperJsonSegment {
    text: String,
    offsets: Option<WhisperOffsets>,
    timestamps: Option<WhisperTimestamps>,
}

impl WhisperJsonSegment {
    fn offsets_millis(&self) -> Result<(u64, u64), AppError> {
        if let Some(offsets) = &self.offsets {
            return Ok((offsets.from.to_millis()?, offsets.to.to_millis()?));
        }
        if let Some(timestamps) = &self.timestamps {
            return Ok((
                parse_timestamp_millis(&timestamps.from)?,
                parse_timestamp_millis(&timestamps.to)?,
            ));
        }
        Err(transcription_error(
            "invalid_transcription_json",
            "Transcript segment is missing offsets or timestamps.",
            Some(format!("text={}", self.text)),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct WhisperOffsets {
    from: WhisperOffsetValue,
    to: WhisperOffsetValue,
}

#[derive(Debug, Deserialize)]
struct WhisperTimestamps {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WhisperOffsetValue {
    Integer(u64),
    Float(f64),
    Text(String),
}

impl WhisperOffsetValue {
    fn to_millis(&self) -> Result<u64, AppError> {
        match self {
            Self::Integer(value) => Ok(*value),
            Self::Float(value) if value.is_finite() && *value >= 0.0 => Ok(value.round() as u64),
            Self::Float(value) => Err(invalid_offset_value(format!("{value}"))),
            Self::Text(value) => value
                .parse::<u64>()
                .or_else(|_| parse_timestamp_millis(value))
                .map_err(|_| invalid_offset_value(value.clone())),
        }
    }
}

fn parse_timestamp_millis(value: &str) -> Result<u64, AppError> {
    let normalized = value.trim().replace(',', ".");
    let mut parts = normalized.split(':');
    let hours = parts
        .next()
        .ok_or_else(|| invalid_offset_value(value.to_string()))?
        .parse::<u64>()
        .map_err(|_| invalid_offset_value(value.to_string()))?;
    let minutes = parts
        .next()
        .ok_or_else(|| invalid_offset_value(value.to_string()))?
        .parse::<u64>()
        .map_err(|_| invalid_offset_value(value.to_string()))?;
    let seconds_part = parts
        .next()
        .ok_or_else(|| invalid_offset_value(value.to_string()))?;
    if parts.next().is_some() {
        return Err(invalid_offset_value(value.to_string()));
    }

    let mut second_parts = seconds_part.split('.');
    let seconds = second_parts
        .next()
        .ok_or_else(|| invalid_offset_value(value.to_string()))?
        .parse::<u64>()
        .map_err(|_| invalid_offset_value(value.to_string()))?;
    let millis = second_parts
        .next()
        .map(|raw_millis| {
            let mut millis = raw_millis.chars().take(3).collect::<String>();
            while millis.len() < 3 {
                millis.push('0');
            }
            millis
                .parse::<u64>()
                .map_err(|_| invalid_offset_value(value.to_string()))
        })
        .transpose()?
        .unwrap_or(0);

    Ok((((hours * 60) + minutes) * 60 + seconds) * 1_000 + millis)
}

fn invalid_offset_value(value: String) -> AppError {
    transcription_error(
        "invalid_transcription_offsets",
        "whisper-cli produced an invalid transcript offset value.",
        Some(format!("value={value}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CollectingSink {
        events: Vec<TranscriptStreamEvent>,
        summaries: Vec<TranscriptStreamSummary>,
    }

    impl CollectingSink {
        fn new() -> Self {
            Self {
                events: Vec::new(),
                summaries: Vec::new(),
            }
        }
    }

    impl TranscriptEventSink for CollectingSink {
        fn emit_segment(&mut self, event: TranscriptStreamEvent) -> Result<(), AppError> {
            self.events.push(event);
            Ok(())
        }

        fn complete(&mut self, summary: TranscriptStreamSummary) -> Result<(), AppError> {
            self.summaries.push(summary);
            Ok(())
        }
    }

    struct StubTranscriber {
        output: TranscriptionOutput,
    }

    impl Transcriber for StubTranscriber {
        fn transcribe(&self, _audio_path: &Path) -> Result<TranscriptionOutput, AppError> {
            Ok(self.output.clone())
        }
    }

    #[test]
    fn parser_returns_timestamped_segments_and_skips_empty_text() {
        let json = r#"{
            "transcription": [
                {"offsets": {"from": 0, "to": 1200}, "text": "  Hello there.  "},
                {"offsets": {"from": 1200, "to": 1500}, "text": "   "},
                {"timestamps": {"from": "00:00:01,500", "to": "00:00:03.250"}, "text": "Next point"}
            ]
        }"#;

        let output = parse_whisper_json(json).expect("valid whisper JSON parses");

        assert_eq!(
            output.segments,
            vec![
                TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Hello there.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_200,
                },
                TranscriptSegment {
                    sequence_number: 2,
                    speaker_label: None,
                    text: "Next point".to_string(),
                    started_at_ms: 1_500,
                    ended_at_ms: 3_250,
                },
            ]
        );
    }

    #[test]
    fn parser_rejects_decreasing_offsets() {
        let json = r#"{
            "transcription": [
                {"offsets": {"from": 1000, "to": 2000}, "text": "first"},
                {"offsets": {"from": 900, "to": 2100}, "text": "second"}
            ]
        }"#;

        let error = parse_whisper_json(json).expect_err("decreasing offsets are rejected");

        assert_eq!(error.code, "invalid_transcription_offsets");
    }

    #[test]
    fn audio_path_validation_rejects_relative_missing_and_unsupported_paths() {
        let relative_error =
            validate_audio_path(Path::new("meeting.wav")).expect_err("relative paths are rejected");
        assert_eq!(relative_error.code, "invalid_audio_path");

        let missing_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/transcription/missing.wav");
        let missing_error =
            validate_audio_path(&missing_path).expect_err("missing files are rejected");
        assert_eq!(missing_error.code, "invalid_audio_path");

        let unsupported_path = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/transcription/meeting.txt");
        std::fs::create_dir_all(unsupported_path.parent().expect("path has parent"))
            .expect("test directory can be created");
        std::fs::write(&unsupported_path, b"not audio").expect("test file can be written");

        let unsupported_error = validate_audio_path(&unsupported_path)
            .expect_err("unsupported extensions are rejected");
        assert_eq!(unsupported_error.code, "unsupported_audio_format");
    }

    #[test]
    fn merge_dual_track_outputs_interleaves_by_time_and_labels_speakers() {
        let microphone = TranscriptionOutput {
            segments: vec![
                TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "My first point.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                },
                TranscriptSegment {
                    sequence_number: 2,
                    speaker_label: None,
                    text: "My reply.".to_string(),
                    started_at_ms: 5_000,
                    ended_at_ms: 6_000,
                },
            ],
        };
        let system = TranscriptionOutput {
            segments: vec![TranscriptSegment {
                sequence_number: 1,
                speaker_label: None,
                text: "Their answer.".to_string(),
                started_at_ms: 2_000,
                ended_at_ms: 4_500,
            }],
        };

        let merged =
            merge_dual_track_outputs(microphone, system).expect("dual track outputs merge");

        assert_eq!(
            merged
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
                (1, Some(USER_SPEAKER_LABEL), "My first point."),
                (2, Some(OTHERS_SPEAKER_LABEL), "Their answer."),
                (3, Some(USER_SPEAKER_LABEL), "My reply."),
            ]
        );
    }

    #[test]
    fn merge_dual_track_outputs_keeps_microphone_first_on_timestamp_ties() {
        let segment = TranscriptSegment {
            sequence_number: 1,
            speaker_label: None,
            text: "Same start.".to_string(),
            started_at_ms: 1_000,
            ended_at_ms: 2_000,
        };
        let microphone = TranscriptionOutput {
            segments: vec![segment.clone()],
        };
        let system = TranscriptionOutput {
            segments: vec![segment],
        };

        let merged =
            merge_dual_track_outputs(microphone, system).expect("dual track outputs merge");

        assert_eq!(
            merged
                .segments
                .iter()
                .map(|segment| segment.speaker_label.as_deref())
                .collect::<Vec<_>>(),
            vec![Some(USER_SPEAKER_LABEL), Some(OTHERS_SPEAKER_LABEL)]
        );
    }

    #[test]
    fn vocabulary_prompt_normalizes_commas_newlines_and_blanks() {
        assert_eq!(
            normalized_vocabulary_prompt(Some("SymbioRAG, Jira\n Snyk ,,\n")),
            Some("Glossary: SymbioRAG, Jira, Snyk.".to_string())
        );
        assert_eq!(normalized_vocabulary_prompt(Some("  , \n ")), None);
        assert_eq!(normalized_vocabulary_prompt(None), None);
    }

    #[test]
    fn vocabulary_max_context_stays_at_or_below_prompt_token_count() {
        // Every whisper BPE token covers at least one character, and product
        // names average well under four characters per token, so chars/4 + 1
        // must stay a safe underestimate that leaves no rolling-context room.
        let prompt = normalized_vocabulary_prompt(Some("SymbioRAG, Jira, Snyk, Langflow"))
            .expect("prompt is built");
        let budget = vocabulary_max_context(&prompt);
        assert!(budget >= 1);
        assert!(budget <= prompt.split_whitespace().count() * 3);
    }

    #[test]
    fn user_segments_are_unlabeled_or_labeled_as_user() {
        assert!(is_user_segment(None));
        assert!(is_user_segment(Some(USER_SPEAKER_LABEL)));
        assert!(!is_user_segment(Some(OTHERS_SPEAKER_LABEL)));
    }

    #[test]
    fn prepare_whisper_input_passes_wav_files_through_untouched() {
        let temp_dir = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/transcription/whisper-input");
        std::fs::create_dir_all(&temp_dir).expect("test directory can be created");
        let wav_path = temp_dir.join("already.wav");
        std::fs::write(&wav_path, b"RIFF fake wav").expect("wav placeholder");

        let prepared =
            prepare_whisper_input(&wav_path, &temp_dir).expect("wav input needs no conversion");

        assert_eq!(prepared, wav_path);
    }

    #[test]
    fn replay_transcription_output_emits_segments_and_completion_summary() {
        let output = TranscriptionOutput {
            segments: vec![
                TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "First point.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 1_000,
                },
                TranscriptSegment {
                    sequence_number: 2,
                    speaker_label: Some("User".to_string()),
                    text: "Second point.".to_string(),
                    started_at_ms: 1_000,
                    ended_at_ms: 2_000,
                },
            ],
        };
        let mut sink = CollectingSink::new();

        let summary = replay_transcription_output("meeting-stream", &output, &mut sink)
            .expect("stream replay succeeds");

        assert_eq!(summary.segment_count, 2);
        assert_eq!(sink.events.len(), 2);
        assert_eq!(sink.events[0].meeting_id, "meeting-stream");
        assert_eq!(sink.events[0].segment.text, "First point.");
        assert!(sink.events[0].is_final);
        assert_eq!(sink.summaries, vec![summary]);
    }

    #[test]
    fn bounded_transcript_event_sink_drops_overflow_without_unbounded_growth() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut sink = BoundedTranscriptEventSink::new(sender);
        let first_event = TranscriptStreamEvent {
            meeting_id: "meeting-backpressure".to_string(),
            segment: TranscriptSegment {
                sequence_number: 1,
                speaker_label: None,
                text: "Retained.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 500,
            },
            is_final: true,
        };
        let dropped_event = TranscriptStreamEvent {
            meeting_id: "meeting-backpressure".to_string(),
            segment: TranscriptSegment {
                sequence_number: 2,
                speaker_label: None,
                text: "Dropped.".to_string(),
                started_at_ms: 500,
                ended_at_ms: 1_000,
            },
            is_final: true,
        };

        sink.emit_segment(first_event.clone())
            .expect("first event is enqueued");
        sink.emit_segment(dropped_event)
            .expect("overflow is tracked without growing memory");

        assert_eq!(sink.dropped_event_count(), 1);
        assert_eq!(
            receiver.try_recv().expect("one retained event"),
            first_event
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn batch_replay_streaming_transcriber_uses_shared_segment_contract() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/transcription/streaming-adapter");
        std::fs::create_dir_all(&base).expect("test directory can be created");
        let audio_path = base.join("meeting.wav");
        std::fs::write(&audio_path, b"RIFF test wav").expect("audio file can be written");
        let streaming_transcriber = BatchReplayStreamingTranscriber::new(StubTranscriber {
            output: TranscriptionOutput {
                segments: vec![TranscriptSegment {
                    sequence_number: 1,
                    speaker_label: None,
                    text: "Shared contract.".to_string(),
                    started_at_ms: 0,
                    ended_at_ms: 700,
                }],
            },
        });
        let mut sink = CollectingSink::new();

        let summary = streaming_transcriber
            .stream_transcribe("meeting-adapter", &audio_path, &mut sink)
            .expect("batch output can be streamed");

        assert_eq!(summary.segment_count, 1);
        assert_eq!(sink.events[0].segment.text, "Shared contract.");
        assert_eq!(sink.summaries[0], summary);
    }
}
