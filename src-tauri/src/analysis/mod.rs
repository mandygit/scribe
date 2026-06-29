use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::{AppError, Score};

/// Strategy interface for coaching analyzers.
pub trait Analyzer {
    fn analyze(&self, context: &AnalysisContext) -> Result<CoachingAnalysis, AppError>;
}

/// Strategy interface for imported-recording summarizers.
pub trait MeetingSummarizer {
    fn summarize(
        &self,
        transcript_segments: &[AnalysisTranscriptSegment],
        include_speaking_improvements: bool,
    ) -> Result<MeetingSummary, AppError>;
}

const OLLAMA_HOST: &str = "127.0.0.1";
const OLLAMA_PORT: u16 = 11434;
const OLLAMA_GENERATE_PATH: &str = "/api/generate";
const OLLAMA_VERSION_PATH: &str = "/api/version";
const DEFAULT_OLLAMA_MODEL: &str = "llama3.2";
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OLLAMA_RESPONSE_BODY_BYTES: usize = 1_048_576;
const MAX_OLLAMA_RESPONSE_HEADER_BYTES: usize = 16_384;
const MAX_SUMMARY_TRANSCRIPT_CHARS: usize = 48_000;
const MIN_SPEAKING_IMPROVEMENT_QUOTE_CHARS: usize = 20;

/// Transcript and deterministic metric context passed to an analyzer.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisContext {
    pub transcript_segments: Vec<AnalysisTranscriptSegment>,
    pub metrics: Vec<AnalysisMetricContext>,
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

/// Deterministic metric context available to the analyzer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisMetricContext {
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
}

/// Parsed coaching analysis with quote-backed observations.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachingAnalysis {
    pub overall_score: Score,
    pub observations: Vec<CoachingObservation>,
}

/// One coaching observation grounded in exact transcript evidence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachingObservation {
    pub category: String,
    pub score: Score,
    pub quote: String,
    pub speaker_label: Option<String>,
    pub context_quote: Option<String>,
    pub context_speaker_label: Option<String>,
    pub suggestion: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OllamaAvailability {
    Available,
    Unavailable { reason: String },
    InvalidResponse { reason: String },
}

pub trait OllamaTransport {
    fn send(&self, request: &[u8], timeout: Duration) -> Result<Vec<u8>, AppError>;
}

pub struct TcpOllamaTransport;

impl OllamaTransport for TcpOllamaTransport {
    fn send(&self, request: &[u8], timeout: Duration) -> Result<Vec<u8>, AppError> {
        let address = SocketAddr::from(([127, 0, 0, 1], OLLAMA_PORT));
        let mut stream =
            TcpStream::connect_timeout(&address, timeout).map_err(map_ollama_transport_error)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(map_ollama_transport_error)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(map_ollama_transport_error)?;
        stream
            .write_all(request)
            .map_err(map_ollama_transport_error)?;

        let mut response = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let bytes_read = stream
                .read(&mut buffer)
                .map_err(map_ollama_transport_error)?;
            if bytes_read == 0 {
                break;
            }
            response.extend_from_slice(&buffer[..bytes_read]);
            if response.len() > MAX_OLLAMA_RESPONSE_BODY_BYTES + MAX_OLLAMA_RESPONSE_HEADER_BYTES {
                return Err(ollama_error(
                    "ollama_response_too_large",
                    "Ollama response exceeded the safe size limit.",
                    None,
                ));
            }
        }

        Ok(response)
    }
}

pub struct OllamaAnalyzer<T: OllamaTransport = TcpOllamaTransport> {
    model: String,
    transport: T,
}

impl OllamaAnalyzer<TcpOllamaTransport> {
    pub fn default_local() -> Self {
        Self {
            model: DEFAULT_OLLAMA_MODEL.to_string(),
            transport: TcpOllamaTransport,
        }
    }
}

impl<T: OllamaTransport> OllamaAnalyzer<T> {
    pub fn new(model: impl Into<String>, transport: T) -> Self {
        Self {
            model: model.into(),
            transport,
        }
    }

    pub fn availability(&self) -> OllamaAvailability {
        let request = build_http_request("GET", OLLAMA_VERSION_PATH, &[]);
        match self.transport.send(&request, OLLAMA_TIMEOUT) {
            Ok(response) => match parse_http_response(&response) {
                Ok(parsed) if (200..300).contains(&parsed.status_code) => {
                    OllamaAvailability::Available
                }
                Ok(parsed) => OllamaAvailability::InvalidResponse {
                    reason: format!("status={}", parsed.status_code),
                },
                Err(error) => OllamaAvailability::InvalidResponse { reason: error.code },
            },
            Err(error) if error.code == "ollama_unavailable" => OllamaAvailability::Unavailable {
                reason: error.details.unwrap_or(error.message),
            },
            Err(error) => OllamaAvailability::InvalidResponse { reason: error.code },
        }
    }
}

impl<T: OllamaTransport> Analyzer for OllamaAnalyzer<T> {
    fn analyze(&self, context: &AnalysisContext) -> Result<CoachingAnalysis, AppError> {
        let prompt = build_analysis_prompt(context);
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false
        })
        .to_string();
        let request = build_http_request("POST", OLLAMA_GENERATE_PATH, body.as_bytes());
        let response_bytes = self.transport.send(&request, OLLAMA_TIMEOUT)?;
        let response = parse_http_response(&response_bytes)?;

        if !(200..300).contains(&response.status_code) {
            return Err(ollama_error(
                "ollama_http_error",
                "Ollama returned a non-success HTTP status.",
                Some(format!("status={}", response.status_code)),
            ));
        }

        let response_text = extract_ollama_response_text(&response.body)?;
        parse_coaching_analysis_response(&response_text, &context.transcript_segments)
    }
}

impl<T: OllamaTransport> MeetingSummarizer for OllamaAnalyzer<T> {
    fn summarize(
        &self,
        transcript_segments: &[AnalysisTranscriptSegment],
        include_speaking_improvements: bool,
    ) -> Result<MeetingSummary, AppError> {
        let prompt =
            build_meeting_summary_prompt(transcript_segments, include_speaking_improvements);
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false
        })
        .to_string();
        let request = build_http_request("POST", OLLAMA_GENERATE_PATH, body.as_bytes());
        let response_bytes = self.transport.send(&request, OLLAMA_TIMEOUT)?;
        let response = parse_http_response(&response_bytes)?;

        if !(200..300).contains(&response.status_code) {
            return Err(ollama_error(
                "ollama_http_error",
                "Ollama returned a non-success HTTP status.",
                Some(format!("status={}", response.status_code)),
            ));
        }

        let response_text = extract_ollama_response_text(&response.body)?;
        parse_meeting_summary_response(
            &response_text,
            transcript_segments,
            include_speaking_improvements,
        )
    }
}

/// Builds a strict local-summary prompt for imported recordings.
pub fn build_meeting_summary_prompt(
    transcript_segments: &[AnalysisTranscriptSegment],
    include_speaking_improvements: bool,
) -> String {
    let mut transcript = String::new();
    let mut truncated = false;
    for segment in transcript_segments {
        let line = format_transcript_segment(segment);
        if transcript.chars().count() + line.chars().count() + 1 > MAX_SUMMARY_TRANSCRIPT_CHARS {
            truncated = true;
            break;
        }
        transcript.push_str(&line);
        transcript.push('\n');
    }

    let truncation_notice = if truncated {
        "The transcript was truncated to fit the local model context. Summarize only the provided transcript.\n"
    } else {
        ""
    };

    let speaking_improvement_schema = if include_speaking_improvements {
        "[{\"category\":\"clarity|brevity|confidence|pace|structure|listening\",\"quote\":\"exact transcript quote\",\"suggestion\":\"specific improvement\"}]"
    } else {
        "[]"
    };
    let speaking_improvement_rules = if include_speaking_improvements {
        "- The user marked this recording as one where they are the main speaker; generate speakingImprovements from the visible transcript.\n\
- Every speaking improvement quote must be copied verbatim from the transcript and be at least one meaningful phrase.\n"
    } else {
        "- Do not generate speaking improvements for this imported recording because the user's voice has not been identified and they did not mark themselves as the main speaker.\n\
- Return speakingImprovements as an empty array.\n"
    };

    format!(
        "You summarize downloaded meeting recordings for someone who missed the meeting.\n\
Return exactly one strict JSON object and no Markdown.\n\
Schema:\n\
{{\"executiveSummary\":\"2-4 sentence summary\",\"actionItems\":[{{\"owner\":\"person or null\",\"task\":\"specific action\",\"due\":\"date or null\"}}],\"decisions\":[\"decision\"],\"openQuestions\":[\"question\"],\"speakingImprovements\":{speaking_improvement_schema}}}\n\
Rules:\n\
- Use only the transcript below.\n\
- Do not invent owners, dates, decisions, or questions.\n\
- Use null for unknown owner or due date.\n\
- Keep action item tasks concrete and concise.\n\
{speaking_improvement_rules}\
{truncation_notice}Transcript:\n{transcript}"
    )
}

/// Parses and validates untrusted imported-meeting summary JSON.
pub fn parse_meeting_summary_response(
    response: &str,
    transcript_segments: &[AnalysisTranscriptSegment],
    include_speaking_improvements: bool,
) -> Result<MeetingSummary, AppError> {
    let trimmed = response.trim();
    if trimmed.starts_with("```") || !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(analysis_error(
            "summary_response_not_json",
            "Summary response must be a single strict JSON object.",
            None,
        ));
    }

    let raw: RawMeetingSummary = serde_json::from_str(trimmed).map_err(|error| {
        analysis_error(
            "invalid_summary_json",
            "Summary response JSON did not match the expected schema.",
            Some(error.to_string()),
        )
    })?;

    raw.into_summary(transcript_segments, include_speaking_improvements)
}

/// Builds the deterministic local-analyzer prompt without making network calls.
pub fn build_analysis_prompt(context: &AnalysisContext) -> String {
    let transcript = context
        .transcript_segments
        .iter()
        .map(format_transcript_segment)
        .collect::<Vec<_>>()
        .join("\n");
    let metrics = context
        .metrics
        .iter()
        .map(format_metric)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are Resonance, a private local meeting coach.\n\
Use only the transcript text and metric context below.\n\
Do not invent facts, speakers, outcomes, intents, or details that are not present.\n\
Analyze the user's communication, not the other speakers' performance.\n\
Transcript rows marked role=user are the user's words. Rows marked role=context are surrounding meeting context from others.\n\
Every observation must include an exact user quote copied verbatim from a role=user transcript row.\n\
When relevant, include contextQuote as an exact quote from a role=context row that explains what the user was responding to; otherwise use null.\n\
Every observation must include concrete rewrite or suggestion text the user could use.\n\
Return strict JSON only: no markdown fences, no prose, no comments, no trailing text.\n\
JSON schema:\n\
{{\"overallScore\":0-100,\"observations\":[{{\"category\":\"string\",\"score\":0-100,\"quote\":\"exact user transcript quote\",\"contextQuote\":\"exact context transcript quote or null\",\"suggestion\":\"concrete rewrite or suggestion\"}}]}}\n\
\n\
Transcript:\n\
{transcript}\n\
\n\
Metrics:\n\
{metrics}"
    )
}

/// Parses and validates untrusted analyzer JSON against the transcript evidence.
pub fn parse_coaching_analysis_response(
    response: &str,
    transcript_segments: &[AnalysisTranscriptSegment],
) -> Result<CoachingAnalysis, AppError> {
    let trimmed = response.trim();
    if trimmed.starts_with("```") || !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(analysis_error(
            "analysis_response_not_json",
            "Analyzer response must be a single strict JSON object.",
            None,
        ));
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|error| {
        analysis_error(
            "invalid_analysis_json",
            "Analyzer response JSON did not match the expected schema.",
            Some(error.to_string()),
        )
    })?;
    let raw: RawCoachingAnalysis = serde_json::from_value(value).map_err(|error| {
        analysis_error(
            "invalid_analysis_json",
            "Analyzer response JSON did not match the expected schema.",
            Some(error.to_string()),
        )
    })?;

    raw.into_analysis(transcript_segments)
}

fn format_transcript_segment(segment: &AnalysisTranscriptSegment) -> String {
    let speaker = segment.speaker_label.as_deref().unwrap_or("unknown");
    let role = match segment.speaker_role {
        TranscriptSpeakerRole::User => "user",
        TranscriptSpeakerRole::Context => "context",
    };
    format!(
        "[{}] role={} speaker={} startMs={} endMs={} text={}",
        segment.sequence_number,
        role,
        speaker,
        segment.started_at_ms,
        segment.ended_at_ms,
        segment.text
    )
}

fn format_metric(metric: &AnalysisMetricContext) -> String {
    let unit = metric.unit.as_deref().unwrap_or("none");
    format!("{}={} unit={}", metric.name, metric.value, unit)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCoachingAnalysis {
    overall_score: u16,
    observations: Vec<RawCoachingObservation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCoachingObservation {
    category: String,
    score: u16,
    quote: String,
    #[serde(default)]
    context_quote: Option<String>,
    suggestion: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMeetingSummary {
    executive_summary: String,
    action_items: Vec<RawMeetingActionItem>,
    decisions: Vec<String>,
    open_questions: Vec<String>,
    #[serde(default)]
    speaking_improvements: Vec<RawSpeakingImprovement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMeetingActionItem {
    owner: Option<String>,
    task: String,
    due: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSpeakingImprovement {
    category: String,
    quote: String,
    suggestion: String,
}

impl RawCoachingAnalysis {
    fn into_analysis(
        self,
        transcript_segments: &[AnalysisTranscriptSegment],
    ) -> Result<CoachingAnalysis, AppError> {
        Ok(CoachingAnalysis {
            overall_score: score_from_u16(self.overall_score)?,
            observations: self
                .observations
                .into_iter()
                .map(|observation| observation.into_observation(transcript_segments))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl RawMeetingSummary {
    fn into_summary(
        self,
        transcript_segments: &[AnalysisTranscriptSegment],
        include_speaking_improvements: bool,
    ) -> Result<MeetingSummary, AppError> {
        Ok(MeetingSummary {
            executive_summary: require_non_empty("executiveSummary", self.executive_summary)?,
            action_items: self
                .action_items
                .into_iter()
                .map(RawMeetingActionItem::into_action_item)
                .collect::<Result<Vec<_>, _>>()?,
            decisions: self
                .decisions
                .into_iter()
                .map(|decision| require_non_empty("decisions", decision))
                .collect::<Result<Vec<_>, _>>()?,
            open_questions: self
                .open_questions
                .into_iter()
                .map(|question| require_non_empty("openQuestions", question))
                .collect::<Result<Vec<_>, _>>()?,
            speaking_improvements: if include_speaking_improvements {
                self.speaking_improvements
                    .into_iter()
                    .map(|improvement| improvement.into_speaking_improvement(transcript_segments))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            },
        })
    }
}

impl RawMeetingActionItem {
    fn into_action_item(self) -> Result<MeetingActionItem, AppError> {
        Ok(MeetingActionItem {
            owner: self
                .owner
                .map(|owner| require_non_empty("owner", owner))
                .transpose()?,
            task: require_non_empty("task", self.task)?,
            due: self
                .due
                .map(|due| require_non_empty("due", due))
                .transpose()?,
        })
    }
}

impl RawSpeakingImprovement {
    fn into_speaking_improvement(
        self,
        transcript_segments: &[AnalysisTranscriptSegment],
    ) -> Result<SpeakingImprovement, AppError> {
        let category = require_non_empty("category", self.category)?;
        let quote = require_non_empty("quote", self.quote)?;
        let suggestion = require_non_empty("suggestion", self.suggestion)?;
        if quote.chars().count() < MIN_SPEAKING_IMPROVEMENT_QUOTE_CHARS {
            return Err(analysis_error(
                "summary_improvement_quote_too_short",
                "Speaking improvement quotes must include enough transcript context to be meaningful.",
                Some(format!(
                    "quote_length={}, min={MIN_SPEAKING_IMPROVEMENT_QUOTE_CHARS}",
                    quote.chars().count()
                )),
            ));
        }

        if !transcript_segments
            .iter()
            .any(|segment| segment.text.contains(&quote))
        {
            return Err(analysis_error(
                "summary_improvement_quote_not_found",
                "Speaking improvement quotes must appear verbatim in the transcript.",
                Some(format!("quote_length={}", quote.chars().count())),
            ));
        }

        Ok(SpeakingImprovement {
            category,
            quote,
            suggestion,
        })
    }
}

impl RawCoachingObservation {
    fn into_observation(
        self,
        transcript_segments: &[AnalysisTranscriptSegment],
    ) -> Result<CoachingObservation, AppError> {
        let category = require_non_empty("category", self.category)?;
        let quote = require_non_empty("quote", self.quote)?;
        let context_quote = self
            .context_quote
            .map(|value| require_non_empty("contextQuote", value))
            .transpose()?;
        let suggestion = require_non_empty("suggestion", self.suggestion)?;
        let speaker_label =
            speaker_label_for_quote(transcript_segments, &quote, TranscriptSpeakerRole::User)
                .ok_or_else(|| {
                    analysis_error(
                        "analysis_user_quote_not_found",
                        "Analyzer quote must appear verbatim in a user transcript segment.",
                        Some(format!("quote_length={}", quote.chars().count())),
                    )
                })?;

        let context_speaker_label = context_quote
            .as_deref()
            .map(|context_quote| {
                speaker_label_for_quote(
                    transcript_segments,
                    context_quote,
                    TranscriptSpeakerRole::Context,
                )
                .ok_or_else(|| {
                    analysis_error(
                        "analysis_context_quote_not_found",
                        "Analyzer contextQuote must appear verbatim in a context transcript segment.",
                        Some(format!(
                            "context_quote_length={}",
                            context_quote.chars().count()
                        )),
                    )
                })
            })
            .transpose()?
            .flatten();

        Ok(CoachingObservation {
            category,
            score: score_from_u16(self.score)?,
            quote,
            speaker_label,
            context_quote,
            context_speaker_label,
            suggestion,
        })
    }
}

pub fn classify_transcript_speaker_role(speaker_label: Option<&str>) -> TranscriptSpeakerRole {
    let Some(label) = speaker_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
    else {
        return TranscriptSpeakerRole::User;
    };
    let normalized = label.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "user" | "you" | "me" | "self" | "speaker 1"
    ) {
        TranscriptSpeakerRole::User
    } else {
        TranscriptSpeakerRole::Context
    }
}

fn speaker_label_for_quote(
    transcript_segments: &[AnalysisTranscriptSegment],
    quote: &str,
    speaker_role: TranscriptSpeakerRole,
) -> Option<Option<String>> {
    transcript_segments
        .iter()
        .find(|segment| segment.speaker_role == speaker_role && segment.text.contains(quote))
        .map(|segment| segment.speaker_label.clone())
}

fn score_from_u16(value: u16) -> Result<Score, AppError> {
    let score = u8::try_from(value).map_err(|error| {
        analysis_error(
            "invalid_analysis_score",
            "Analyzer scores must be between 0 and 100.",
            Some(error.to_string()),
        )
    })?;
    Score::new(score).map_err(|error| {
        analysis_error(
            "invalid_analysis_score",
            "Analyzer scores must be between 0 and 100.",
            error.details,
        )
    })
}

fn require_non_empty(field_name: &str, value: String) -> Result<String, AppError> {
    if value.trim().is_empty() {
        return Err(analysis_error(
            "analysis_empty_field",
            "Analyzer response fields must not be empty.",
            Some(format!("field={field_name}")),
        ));
    }

    Ok(value)
}

struct ParsedHttpResponse {
    status_code: u16,
    body: Vec<u8>,
}

fn extract_ollama_response_text(body: &[u8]) -> Result<String, AppError> {
    let body_text = std::str::from_utf8(body).map_err(|error| {
        ollama_error(
            "ollama_invalid_json",
            "Ollama response body was not valid UTF-8.",
            Some(error.to_string()),
        )
    })?;
    let trimmed = body_text.trim();

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return response_text_from_value(&value)?.ok_or_else(|| {
            ollama_error(
                "ollama_invalid_json",
                "Ollama response JSON did not match the expected generate schema.",
                Some(truncated_response_details(trimmed)),
            )
        });
    }

    let mut combined = String::new();
    let mut saw_fragment = false;
    for line in trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            ollama_error(
                "ollama_invalid_json",
                "Ollama response JSON did not match the expected generate schema.",
                Some(error.to_string()),
            )
        })?;

        if let Some(fragment) = response_text_from_value(&value)? {
            combined.push_str(&fragment);
            saw_fragment = true;
        }
    }

    if saw_fragment {
        return Ok(combined);
    }

    Err(ollama_error(
        "ollama_invalid_json",
        "Ollama response JSON did not match the expected generate schema.",
        Some(truncated_response_details(trimmed)),
    ))
}

fn response_text_from_value(value: &Value) -> Result<Option<String>, AppError> {
    if let Some(error_message) = value
        .as_object()
        .and_then(|object| object.get("error"))
        .and_then(Value::as_str)
    {
        return Err(ollama_error(
            "ollama_generate_error",
            error_message,
            Some(serde_json::to_string(value).unwrap_or_else(|_| error_message.to_string())),
        ));
    }

    if let Some(response) = value
        .as_object()
        .and_then(|object| object.get("response"))
        .and_then(Value::as_str)
    {
        return Ok(Some(response.to_string()));
    }

    Ok(value.as_object().and_then(|object| {
        object
            .get("message")
            .and_then(Value::as_object)
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }))
}

fn truncated_response_details(response: &str) -> String {
    const MAX_RESPONSE_PREVIEW_CHARS: usize = 400;

    let mut preview = response.chars().take(MAX_RESPONSE_PREVIEW_CHARS).collect::<String>();
    if response.chars().count() > MAX_RESPONSE_PREVIEW_CHARS {
        preview.push_str("...");
    }

    format!("response_preview={preview}")
}

fn build_http_request(method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let request = format!(
        "{method} {path} HTTP/1.1\r\n\
        Host: {OLLAMA_HOST}:{OLLAMA_PORT}\r\n\
        Content-Type: application/json\r\n\
        Accept: application/json\r\n\
        Connection: close\r\n\
        Content-Length: {}\r\n\
        \r\n",
        body.len()
    );
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn parse_http_response(response: &[u8]) -> Result<ParsedHttpResponse, AppError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            ollama_error(
                "ollama_invalid_response",
                "Ollama HTTP response did not contain complete headers.",
                None,
            )
        })?;
    let header_bytes = &response[..header_end];
    if header_bytes.len() > MAX_OLLAMA_RESPONSE_HEADER_BYTES {
        return Err(ollama_error(
            "ollama_invalid_response",
            "Ollama HTTP headers exceeded the safe size limit.",
            Some(format!(
                "limit_bytes={MAX_OLLAMA_RESPONSE_HEADER_BYTES}, actual_bytes={}",
                header_bytes.len()
            )),
        ));
    }
    let body = response[header_end + 4..].to_vec();
    if body.len() > MAX_OLLAMA_RESPONSE_BODY_BYTES {
        return Err(ollama_error(
            "ollama_response_too_large",
            "Ollama response exceeded the safe size limit.",
            Some(format!(
                "limit_bytes={MAX_OLLAMA_RESPONSE_BODY_BYTES}, actual_bytes={}",
                body.len()
            )),
        ));
    }

    let header_text = std::str::from_utf8(header_bytes).map_err(|error| {
        ollama_error(
            "ollama_invalid_response",
            "Ollama HTTP headers were not valid UTF-8.",
            Some(error.to_string()),
        )
    })?;
    let status_line = header_text.lines().next().ok_or_else(|| {
        ollama_error(
            "ollama_invalid_response",
            "Ollama HTTP response did not include a status line.",
            None,
        )
    })?;
    let mut status_parts = status_line.split_whitespace();
    let http_version = status_parts.next().ok_or_else(|| {
        ollama_error(
            "ollama_invalid_response",
            "Ollama HTTP response did not include an HTTP version.",
            None,
        )
    })?;
    if !matches!(http_version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(ollama_error(
            "ollama_invalid_response",
            "Ollama HTTP response used an unsupported HTTP version.",
            Some(format!("version={http_version}")),
        ));
    }
    let status_code = status_parts
        .next()
        .ok_or_else(|| {
            ollama_error(
                "ollama_invalid_response",
                "Ollama HTTP status line did not include a status code.",
                None,
            )
        })?
        .parse::<u16>()
        .map_err(|error| {
            ollama_error(
                "ollama_invalid_response",
                "Ollama HTTP status code was invalid.",
                Some(error.to_string()),
            )
        })?;

    Ok(ParsedHttpResponse { status_code, body })
}

fn map_ollama_transport_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::TimedOut
        | std::io::ErrorKind::WouldBlock
        | std::io::ErrorKind::NotConnected => ollama_error(
            "ollama_unavailable",
            "Local Ollama is not reachable on 127.0.0.1:11434. Run `ollama serve` and `ollama pull llama3.2`, then retry analysis.",
            Some(error.to_string()),
        ),
        _ => ollama_error(
            "ollama_transport_error",
            "Local Ollama HTTP transport failed.",
            Some(error.to_string()),
        ),
    }
}

fn analysis_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

fn ollama_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeOllamaTransport {
        response: Result<Vec<u8>, AppError>,
    }

    impl OllamaTransport for FakeOllamaTransport {
        fn send(&self, _request: &[u8], _timeout: Duration) -> Result<Vec<u8>, AppError> {
            self.response.clone()
        }
    }

    #[test]
    fn prompt_requires_exact_quotes_strict_json_concrete_suggestions_and_no_invention() {
        let prompt = build_analysis_prompt(&AnalysisContext {
            transcript_segments: vec![AnalysisTranscriptSegment {
                sequence_number: 1,
                speaker_label: Some("User".to_string()),
                speaker_role: TranscriptSpeakerRole::User,
                text: "I think we should ship the small slice first.".to_string(),
                started_at_ms: 0,
                ended_at_ms: 2_000,
            }],
            metrics: vec![AnalysisMetricContext {
                name: "hedging_phrase_count".to_string(),
                value: 1.0,
                unit: Some("count".to_string()),
            }],
        });

        assert!(prompt.contains("Use only the transcript text"));
        assert!(prompt.contains("Analyze the user's communication"));
        assert!(prompt.contains("role=user"));
        assert!(prompt.contains("exact user quote copied verbatim"));
        assert!(prompt.contains("contextQuote"));
        assert!(prompt.contains("concrete rewrite or suggestion text the user could use"));
        assert!(prompt.contains("Return strict JSON only"));
        assert!(prompt.contains("Do not invent facts"));
        assert!(prompt.contains("I think we should ship the small slice first."));
        assert!(prompt.contains("hedging_phrase_count=1"));
    }

    #[test]
    fn valid_response_parses_into_typed_analysis() {
        let analysis = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we should ship the small slice first.",
                    "suggestion": "Say: We should ship the small slice first."
                }]
            }"#,
            &transcript_segments(),
        )
        .expect("valid quote-backed JSON parses");

        assert_eq!(analysis.overall_score.value(), 82);
        assert_eq!(analysis.observations.len(), 1);
        assert_eq!(analysis.observations[0].score.value(), 76);
        assert_eq!(
            analysis.observations[0].quote,
            "I think we should ship the small slice first."
        );
        assert_eq!(
            analysis.observations[0].speaker_label,
            Some("User".to_string())
        );
        assert_eq!(analysis.observations[0].context_quote, None);
    }

    #[test]
    fn meeting_summary_parses_transcript_grounded_speaking_improvements() {
        let summary = parse_meeting_summary_response(
            r#"{
                "executiveSummary": "The team aligned on a smaller launch slice.",
                "actionItems": [{"owner": "Sam", "task": "Send the rollout checklist.", "due": null}],
                "decisions": ["Ship the small slice first."],
                "openQuestions": ["Who owns support handoff?"],
                "speakingImprovements": [{
                    "category": "clarity",
                    "quote": "I think we should ship the small slice first.",
                    "suggestion": "Lead with the decision: We should ship the small slice first."
                }]
            }"#,
            &transcript_segments(),
            true,
        )
        .expect("valid imported summary parses");

        assert_eq!(
            summary.executive_summary,
            "The team aligned on a smaller launch slice."
        );
        assert_eq!(summary.action_items[0].owner.as_deref(), Some("Sam"));
        assert_eq!(summary.speaking_improvements.len(), 1);
        assert_eq!(summary.speaking_improvements[0].category, "clarity");
    }

    #[test]
    fn meeting_summary_rejects_invented_speaking_improvement_quotes() {
        let error = parse_meeting_summary_response(
            r#"{
                "executiveSummary": "The team aligned on a smaller launch slice.",
                "actionItems": [],
                "decisions": [],
                "openQuestions": [],
                "speakingImprovements": [{
                    "category": "clarity",
                    "quote": "We already launched the entire project.",
                    "suggestion": "Be more precise."
                }]
            }"#,
            &transcript_segments(),
            true,
        )
        .expect_err("invented speaking improvement quote is rejected");

        assert_eq!(error.code, "summary_improvement_quote_not_found");
        assert_eq!(error.details.as_deref(), Some("quote_length=39"));
    }

    #[test]
    fn meeting_summary_rejects_tiny_speaking_improvement_quote_fragments() {
        let error = parse_meeting_summary_response(
            r#"{
                "executiveSummary": "The team aligned on a smaller launch slice.",
                "actionItems": [],
                "decisions": [],
                "openQuestions": [],
                "speakingImprovements": [{
                    "category": "clarity",
                    "quote": "small slice",
                    "suggestion": "Be more precise."
                }]
            }"#,
            &transcript_segments(),
            true,
        )
        .expect_err("tiny quote fragments are rejected");

        assert_eq!(error.code, "summary_improvement_quote_too_short");
        assert_eq!(error.details.as_deref(), Some("quote_length=11, min=20"));
    }

    #[test]
    fn meeting_summary_omits_speaking_improvements_when_coaching_not_requested() {
        let summary = parse_meeting_summary_response(
            r#"{
                "executiveSummary": "The team aligned on a smaller launch slice.",
                "actionItems": [],
                "decisions": [],
                "openQuestions": [],
                "speakingImprovements": [{
                    "category": "clarity",
                    "quote": "I think we should ship the small slice first.",
                    "suggestion": "Lead with the decision."
                }]
            }"#,
            &transcript_segments(),
            false,
        )
        .expect("summary parses while disabled model coaching is ignored");

        assert!(summary.speaking_improvements.is_empty());
    }

    #[test]
    fn summary_prompt_disables_speaking_coaching_by_default() {
        let prompt = build_meeting_summary_prompt(&transcript_segments(), false);

        assert!(prompt.contains("\"speakingImprovements\":[]"));
        assert!(prompt.contains("Do not generate speaking improvements"));
        assert!(!prompt.contains("clarity|brevity|confidence"));
    }

    #[test]
    fn context_quote_must_come_from_context_segment() {
        let analysis = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we can maybe do that.",
                    "contextQuote": "Can we commit to the smaller launch today?",
                    "suggestion": "Say: Yes, we can commit to the smaller launch today."
                }]
            }"#,
            &contextual_transcript_segments(),
        )
        .expect("context-backed JSON parses");

        assert_eq!(
            analysis.observations[0].speaker_label,
            Some("User".to_string())
        );
        assert_eq!(
            analysis.observations[0].context_quote,
            Some("Can we commit to the smaller launch today?".to_string())
        );
        assert_eq!(
            analysis.observations[0].context_speaker_label,
            Some("Speaker 2".to_string())
        );
    }

    #[test]
    fn main_quote_from_context_segment_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "Can we commit to the smaller launch today?",
                    "contextQuote": null,
                    "suggestion": "Say: Yes, we can commit to the smaller launch today."
                }]
            }"#,
            &contextual_transcript_segments(),
        )
        .expect_err("main quote must come from user speech");

        assert_eq!(error.code, "analysis_user_quote_not_found");
        assert_eq!(error.details.as_deref(), Some("quote_length=42"));
        assert!(!error
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("Can we commit"));
    }

    #[test]
    fn invented_context_quote_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we can maybe do that.",
                    "contextQuote": "Can we ship everything tomorrow?",
                    "suggestion": "Say: Yes, we can commit to the smaller launch today."
                }]
            }"#,
            &contextual_transcript_segments(),
        )
        .expect_err("context quote must be grounded");

        assert_eq!(error.code, "analysis_context_quote_not_found");
        assert_eq!(error.details.as_deref(), Some("context_quote_length=32"));
        assert!(!error
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("ship everything"));
    }

    #[test]
    fn invalid_json_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{"overallScore":82,"observations":[}"#,
            &transcript_segments(),
        )
        .expect_err("invalid JSON is rejected");

        assert_eq!(error.code, "invalid_analysis_json");
    }

    #[test]
    fn duplicate_context_quote_field_uses_last_value_and_parses() {
        let analysis = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we can maybe do that.",
                    "contextQuote": null,
                    "contextQuote": "Can we commit to the smaller launch today?",
                    "suggestion": "Say: Yes, we can commit to the smaller launch today."
                }]
            }"#,
            &contextual_transcript_segments(),
        )
        .expect("duplicate field is normalized by serde_json::Value");

        assert_eq!(analysis.overall_score.value(), 82);
        assert_eq!(
            analysis.observations[0].context_quote,
            Some("Can we commit to the smaller launch today?".to_string())
        );
    }

    #[test]
    fn markdown_fenced_response_fails_safely() {
        let error = parse_coaching_analysis_response(
            "```json\n{\"overallScore\":82,\"observations\":[]}\n```",
            &transcript_segments(),
        )
        .expect_err("markdown fences are rejected");

        assert_eq!(error.code, "analysis_response_not_json");
    }

    #[test]
    fn out_of_range_score_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 101,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we should ship the small slice first.",
                    "suggestion": "Say: We should ship the small slice first."
                }]
            }"#,
            &transcript_segments(),
        )
        .expect_err("out-of-range score is rejected");

        assert_eq!(error.code, "invalid_analysis_score");
    }

    #[test]
    fn quote_not_present_in_transcript_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "We already launched the feature.",
                    "suggestion": "Say: We should ship the small slice first."
                }]
            }"#,
            &transcript_segments(),
        )
        .expect_err("invented quotes are rejected");

        assert_eq!(error.code, "analysis_user_quote_not_found");
    }

    #[test]
    fn empty_quote_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": " ",
                    "suggestion": "Say: We should ship the small slice first."
                }]
            }"#,
            &transcript_segments(),
        )
        .expect_err("empty quote is rejected");

        assert_eq!(error.code, "analysis_empty_field");
        assert_eq!(error.details, Some("field=quote".to_string()));
    }

    #[test]
    fn empty_suggestion_fails_safely() {
        let error = parse_coaching_analysis_response(
            r#"{
                "overallScore": 82,
                "observations": [{
                    "category": "clarity",
                    "score": 76,
                    "quote": "I think we should ship the small slice first.",
                    "suggestion": ""
                }]
            }"#,
            &transcript_segments(),
        )
        .expect_err("empty suggestion is rejected");

        assert_eq!(error.code, "analysis_empty_field");
        assert_eq!(error.details, Some("field=suggestion".to_string()));
    }

    #[test]
    fn ollama_analyzer_successful_generate_response_parses_to_coaching_analysis() {
        let ollama_body = json!({
            "response": "{\"overallScore\":82,\"observations\":[{\"category\":\"clarity\",\"score\":76,\"quote\":\"I think we should ship the small slice first.\",\"suggestion\":\"Say: We should ship the small slice first.\"}]}",
            "done": true
        })
        .to_string();
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(200, ollama_body.as_bytes())),
            },
        );

        let analysis = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: vec![AnalysisMetricContext {
                    name: "hedging_phrase_count".to_string(),
                    value: 1.0,
                    unit: Some("count".to_string()),
                }],
            })
            .expect("valid Ollama response parses");

        assert_eq!(analysis.overall_score.value(), 82);
        assert_eq!(analysis.observations[0].category, "clarity");
    }

    #[test]
    fn ollama_analyzer_streaming_generate_lines_parse_to_coaching_analysis() {
        let ollama_body = concat!(
            "{\"response\":\"{\\\"overallScore\\\":82,\\\"observations\\\":[\",\"done\":false}\n",
            "{\"response\":\"{\\\"category\\\":\\\"clarity\\\",\\\"score\\\":76,\\\"quote\\\":\\\"I think we should ship the small slice first.\\\",\\\"suggestion\\\":\\\"Say: We should ship the small slice first.\\\"}]}\",\"done\":true}"
        );
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(200, ollama_body.as_bytes())),
            },
        );

        let analysis = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: vec![AnalysisMetricContext {
                    name: "hedging_phrase_count".to_string(),
                    value: 1.0,
                    unit: Some("count".to_string()),
                }],
            })
            .expect("streamed Ollama response parses");

        assert_eq!(analysis.overall_score.value(), 82);
        assert_eq!(analysis.observations[0].category, "clarity");
    }

    #[test]
    fn ollama_analyzer_chat_style_response_parses_to_coaching_analysis() {
        let ollama_body = json!({
            "message": {
                "role": "assistant",
                "content": "{\"overallScore\":82,\"observations\":[{\"category\":\"clarity\",\"score\":76,\"quote\":\"I think we should ship the small slice first.\",\"suggestion\":\"Say: We should ship the small slice first.\"}]}"
            },
            "done": true
        })
        .to_string();
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(200, ollama_body.as_bytes())),
            },
        );

        let analysis = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: vec![AnalysisMetricContext {
                    name: "hedging_phrase_count".to_string(),
                    value: 1.0,
                    unit: Some("count".to_string()),
                }],
            })
            .expect("chat-style Ollama response parses");

        assert_eq!(analysis.overall_score.value(), 82);
        assert_eq!(analysis.observations[0].category, "clarity");
    }

    #[test]
    fn ollama_analyzer_error_payload_surfaces_real_message() {
        let ollama_body = json!({
            "error": "model requires more system memory"
        })
        .to_string();
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(200, ollama_body.as_bytes())),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("Ollama error payload is surfaced");

        assert_eq!(error.code, "ollama_generate_error");
        assert_eq!(error.message, "model requires more system memory");
        assert_eq!(error.details, Some(ollama_body));
    }

    #[test]
    fn ollama_analyzer_connection_error_maps_to_unavailable() {
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Err(ollama_error(
                    "ollama_unavailable",
                    "Local Ollama is not reachable on 127.0.0.1:11434. Run `ollama serve` and `ollama pull llama3.2`, then retry analysis.",
                    Some("connection refused".to_string()),
                )),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("transport error is returned safely");

        assert_eq!(error.code, "ollama_unavailable");
        assert!(error.message.contains("ollama serve"));
        assert!(error.message.contains("ollama pull llama3.2"));
    }

    #[test]
    fn ollama_analyzer_non_success_status_fails_safely() {
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(500, br#"{"error":"model failed"}"#)),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("non-2xx status is rejected");

        assert_eq!(error.code, "ollama_http_error");
        assert_eq!(error.details, Some("status=500".to_string()));
    }

    #[test]
    fn ollama_analyzer_oversized_response_body_fails_safely() {
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(http_response(
                    200,
                    &vec![b'a'; MAX_OLLAMA_RESPONSE_BODY_BYTES + 1],
                )),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("oversized body is rejected");

        assert_eq!(error.code, "ollama_response_too_large");
    }

    #[test]
    fn ollama_availability_distinguishes_invalid_response() {
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(b"not http".to_vec()),
            },
        );

        assert!(matches!(
            analyzer.availability(),
            OllamaAvailability::InvalidResponse { .. }
        ));
    }

    #[test]
    fn malformed_http_status_line_fails_safely() {
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(b"GARBAGE 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("malformed status line is rejected");

        assert_eq!(error.code, "ollama_invalid_response");
    }

    #[test]
    fn oversized_http_headers_fail_safely() {
        let mut response = b"HTTP/1.1 200 OK\r\n".to_vec();
        response.extend_from_slice(
            format!(
                "X-Resonance-Test: {}\r\n",
                "a".repeat(MAX_OLLAMA_RESPONSE_HEADER_BYTES)
            )
            .as_bytes(),
        );
        response.extend_from_slice(b"\r\n{}");
        let analyzer = OllamaAnalyzer::new(
            "test-model",
            FakeOllamaTransport {
                response: Ok(response),
            },
        );

        let error = analyzer
            .analyze(&AnalysisContext {
                transcript_segments: transcript_segments(),
                metrics: Vec::new(),
            })
            .expect_err("oversized headers are rejected");

        assert_eq!(error.code, "ollama_invalid_response");
    }

    fn transcript_segments() -> Vec<AnalysisTranscriptSegment> {
        vec![AnalysisTranscriptSegment {
            sequence_number: 1,
            speaker_label: Some("User".to_string()),
            speaker_role: TranscriptSpeakerRole::User,
            text: "I think we should ship the small slice first.".to_string(),
            started_at_ms: 0,
            ended_at_ms: 2_000,
        }]
    }

    fn contextual_transcript_segments() -> Vec<AnalysisTranscriptSegment> {
        vec![
            AnalysisTranscriptSegment {
                sequence_number: 1,
                speaker_label: Some("Speaker 2".to_string()),
                speaker_role: TranscriptSpeakerRole::Context,
                text: "Can we commit to the smaller launch today?".to_string(),
                started_at_ms: 0,
                ended_at_ms: 2_000,
            },
            AnalysisTranscriptSegment {
                sequence_number: 2,
                speaker_label: Some("User".to_string()),
                speaker_role: TranscriptSpeakerRole::User,
                text: "I think we can maybe do that.".to_string(),
                started_at_ms: 2_100,
                ended_at_ms: 4_000,
            },
        ]
    }

    fn http_response(status: u16, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }
}
