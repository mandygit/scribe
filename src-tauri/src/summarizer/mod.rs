//! On-device meeting summarizer backed by LM Studio's OpenAI-compatible API.
//!
//! The engine is swappable behind [`MeetingSummarizer`]: today it talks to a
//! locally running LM Studio server (default model Gemma), but any
//! [`ChatCompletion`] backend can be dropped in without touching callers.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::analysis::{AnalysisTranscriptSegment, KeyTopic, MeetingActionItem, MeetingSummary, MeetingSummarizer};
use crate::domain::{AppError, SummarizerProvider};

pub const DEFAULT_SUMMARIZER_MODEL: &str = "qwen3-14b-mlx";
const LM_STUDIO_HOST: &str = "127.0.0.1";
const LM_STUDIO_PORT: u16 = 1234;
const CHAT_PATH: &str = "/v1/chat/completions";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Single-shot when the rendered transcript fits this budget (~9k tokens),
/// otherwise map-reduce over windows of [`CHUNK_CHAR_TARGET`].
const SINGLE_SHOT_CHAR_BUDGET: usize = 36_000;
const CHUNK_CHAR_TARGET: usize = 12_000;

const SUMMARY_SYSTEM: &str =
    "You write thorough, accurate meeting notes as strict JSON, grounded only in the provided text. \
Prefer specific, granular detail (names, numbers, tool/system names, concrete reasoning) over vague \
generalities, and do not stop at the first few points, decisions, or action items when more were \
stated. Never invent owners, dates, decisions, or facts. Output a single JSON object and nothing else.";
const MAP_SYSTEM: &str =
    "You condense one portion of a meeting transcript into thorough, factual bullet notes. \
Capture every distinct point, decision, and action item with owners and due dates when stated, \
preserving specific names, numbers, and tool/system names rather than generalizing them away. \
Plain text only, no preamble.";

const SUMMARY_SCHEMA: &str = "{\"meetingTitle\":\"3-6 word descriptive title, no quotes or punctuation\",\
\"executiveSummary\":\"2-4 sentence overview of what the meeting was and why it happened\",\
\"keyTopics\":[{\"topic\":\"short label for one thread of discussion\",\
\"points\":[\"specific, granular point from that thread, grounded in the transcript\"]}],\
\"actionItems\":[{\"owner\":\"person or null\",\"task\":\"specific action\",\"due\":\"date or null\"}],\
\"decisions\":[\"decision\"],\"openQuestions\":[\"question\"],\"speakingImprovements\":[]}";

/// A chat backend that takes a system and user message and returns assistant text.
pub trait ChatCompletion {
    fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, AppError>;
}

/// Summarizer that drives any [`ChatCompletion`] backend, applying map-reduce
/// for transcripts that exceed the single-shot budget.
pub struct LmStudioSummarizer<C: ChatCompletion> {
    pub client: C,
    pub model: String,
}

impl<C: ChatCompletion> LmStudioSummarizer<C> {
    pub fn new(client: C, model: impl Into<String>) -> Self {
        Self { client, model: model.into() }
    }
}

impl<C: ChatCompletion> MeetingSummarizer for LmStudioSummarizer<C> {
    fn summarize(
        &self,
        transcript_segments: &[AnalysisTranscriptSegment],
        _include_speaking_improvements: bool,
    ) -> Result<MeetingSummary, AppError> {
        if transcript_segments.is_empty() {
            return Err(summarizer_error(
                "summary_empty_transcript",
                "Cannot summarize a meeting without any transcript segments.",
                None,
            ));
        }

        let transcript = render_transcript(transcript_segments);
        if transcript.chars().count() <= SINGLE_SHOT_CHAR_BUDGET {
            let reply = self.client.complete(&self.model, SUMMARY_SYSTEM, &single_shot_prompt(&transcript))?;
            return parse_summary(&reply);
        }

        let mut digests = String::new();
        for (index, chunk) in chunk_transcript(transcript_segments, CHUNK_CHAR_TARGET).iter().enumerate() {
            let digest = self.client.complete(&self.model, MAP_SYSTEM, &map_prompt(chunk))?;
            digests.push_str(&format!("Section {}:\n{}\n\n", index + 1, digest.trim()));
        }
        let reply = self.client.complete(&self.model, SUMMARY_SYSTEM, &reduce_prompt(&digests))?;
        parse_summary(&reply)
    }
}

fn render_transcript(segments: &[AnalysisTranscriptSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        let speaker = segment.speaker_label.as_deref().unwrap_or("Speaker");
        out.push_str(speaker);
        out.push_str(": ");
        out.push_str(segment.text.trim());
        out.push('\n');
    }
    out
}

/// Groups consecutive segments into windows of roughly `target_chars`.
fn chunk_transcript(segments: &[AnalysisTranscriptSegment], target_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for segment in segments {
        let speaker = segment.speaker_label.as_deref().unwrap_or("Speaker");
        let line = format!("{speaker}: {}\n", segment.text.trim());
        if !current.is_empty() && current.chars().count() + line.chars().count() > target_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(&line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

// Qwen3's chat template enables a "thinking" pass by default, which adds
// pure latency for this deterministic extraction task (and eats into the
// per-request timeout). "/no_think" is Qwen3's documented in-prompt switch
// to skip it, independent of whether the serving backend exposes an
// enable_thinking API flag.
const NO_THINK_DIRECTIVE: &str = "\n\n/no_think";

fn single_shot_prompt(transcript: &str) -> String {
    format!(
        "Summarize this meeting for someone who missed it, thoroughly enough that they don't need \
to read the transcript.\n\
Return exactly one strict JSON object matching this schema and no Markdown:\n{SUMMARY_SCHEMA}\n\
Break keyTopics into as many distinct threads of discussion as the transcript actually covers \
(do not compress everything into one or two topics), and give each thread every specific, \
concrete point made about it, not just a headline takeaway. Do the same for actionItems, \
decisions, and openQuestions: include every one that was actually stated, not only the most \
obvious few. Use null for an unknown owner or due date. Keep action item tasks concrete.\n\
meetingTitle should identify the meeting's actual subject (e.g. \"Q3 Budget Review\"), not a generic label like \"Meeting Notes\".\n\n\
Transcript:\n{transcript}{NO_THINK_DIRECTIVE}"
    )
}

fn map_prompt(chunk: &str) -> String {
    format!(
        "Condense this portion of a meeting into terse bullet notes.\n\nTranscript:\n{chunk}{NO_THINK_DIRECTIVE}"
    )
}

fn reduce_prompt(digests: &str) -> String {
    format!(
        "Combine these section notes from one meeting into a single, thorough set of meeting notes.\n\
Return exactly one strict JSON object matching this schema and no Markdown:\n{SUMMARY_SCHEMA}\n\
Break keyTopics into as many distinct threads of discussion as the notes actually cover, each with \
every specific point from that thread, not a one-line generalization. Carry forward every action \
item, decision, and open question mentioned across all sections, not just the most obvious ones. \
Merge only true duplicates, use null for an unknown owner or due date, and keep tasks concrete.\n\
meetingTitle should identify the meeting's actual subject (e.g. \"Q3 Budget Review\"), not a generic label like \"Meeting Notes\".\n\n\
Section notes:\n{digests}{NO_THINK_DIRECTIVE}"
    )
}

/// Parses possibly-noisy assistant text into a [`MeetingSummary`], tolerating
/// code fences, preamble, and missing optional arrays.
pub fn parse_summary(reply: &str) -> Result<MeetingSummary, AppError> {
    let json = extract_json_object(reply).ok_or_else(|| {
        summarizer_error(
            "summary_response_not_json",
            "The summarizer did not return a JSON object.",
            Some(truncate_for_details(reply)),
        )
    })?;
    let value: Value = serde_json::from_str(&json).map_err(|error| {
        summarizer_error(
            "summary_invalid_json",
            "The summarizer returned malformed JSON.",
            Some(error.to_string()),
        )
    })?;
    Ok(meeting_summary_from_value(&value))
}

fn meeting_summary_from_value(value: &Value) -> MeetingSummary {
    MeetingSummary {
        meeting_title: non_empty_string(value.get("meetingTitle")),
        executive_summary: value
            .get("executiveSummary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        key_topics: value
            .get("keyTopics")
            .and_then(Value::as_array)
            .map(|topics| topics.iter().filter_map(key_topic_from_value).collect())
            .unwrap_or_default(),
        action_items: value
            .get("actionItems")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(action_item_from_value).collect())
            .unwrap_or_default(),
        decisions: string_list(value.get("decisions")),
        open_questions: string_list(value.get("openQuestions")),
        speaking_improvements: Vec::new(),
    }
}

fn key_topic_from_value(value: &Value) -> Option<KeyTopic> {
    let topic = value.get("topic").and_then(Value::as_str)?.trim().to_string();
    if topic.is_empty() {
        return None;
    }
    Some(KeyTopic { topic, points: string_list(value.get("points")) })
}

fn action_item_from_value(value: &Value) -> Option<MeetingActionItem> {
    let task = value.get("task").and_then(Value::as_str)?.trim().to_string();
    if task.is_empty() {
        return None;
    }
    Some(MeetingActionItem {
        owner: non_empty_string(value.get("owner")),
        task,
        due: non_empty_string(value.get("due")),
    })
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty() && !item.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(text[start..=end].to_string())
}

fn truncate_for_details(text: &str) -> String {
    text.chars().take(200).collect()
}

/// HTTP client for a locally running LM Studio server.
pub struct LmStudioClient {
    pub host: String,
    pub port: u16,
}

impl Default for LmStudioClient {
    fn default() -> Self {
        Self { host: LM_STUDIO_HOST.to_string(), port: LM_STUDIO_PORT }
    }
}

impl ChatCompletion for LmStudioClient {
    fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, AppError> {
        let response = post(&self.host, self.port, CHAT_PATH, &chat_request_body(model, system, user))?;
        parse_chat_content(&response)
    }
}

/// Generic OpenAI-compatible chat client for a local model server other than
/// LM Studio (Ollama's `/v1/chat/completions`, or any other OpenAI-compatible
/// endpoint a dev points Scribe at). Shares the wire format with
/// [`LmStudioClient`] but skips LM Studio's `lms` CLI-driven server/model
/// lifecycle, since other servers don't have an equivalent local start/load
/// step — the model is expected to already be reachable.
pub struct OpenAiCompatibleClient {
    pub host: String,
    pub port: u16,
}

impl ChatCompletion for OpenAiCompatibleClient {
    fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, AppError> {
        let response = post(&self.host, self.port, CHAT_PATH, &chat_request_body(model, system, user))?;
        parse_chat_content(&response)
    }
}

fn chat_request_body(model: &str, system: &str, user: &str) -> Vec<u8> {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "temperature": 0.2,
        "stream": false,
    })
    .to_string()
    .into_bytes()
}

fn parse_chat_content(response: &[u8]) -> Result<String, AppError> {
    let value: Value = serde_json::from_slice(response).map_err(|error| {
        summarizer_error(
            "summarizer_invalid_json",
            "The model server returned a response that could not be parsed.",
            Some(error.to_string()),
        )
    })?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            summarizer_error(
                "summarizer_missing_content",
                "The model server's response did not contain assistant content.",
                None,
            )
        })
}

/// Lists model ids/names available on a local summarizer server, for
/// populating a model picker in Settings. LM Studio and Custom endpoints speak
/// the OpenAI-compatible `/v1/models` list endpoint; Ollama has its own
/// `/api/tags` shape.
pub fn list_models(provider: SummarizerProvider, host: &str, port: u16) -> Result<Vec<String>, AppError> {
    let response = match provider {
        SummarizerProvider::Ollama => get(host, port, "/api/tags")?,
        SummarizerProvider::LmStudio | SummarizerProvider::Custom => get(host, port, "/v1/models")?,
    };
    let value = parse_models_json(&response)?;
    Ok(extract_model_names(provider, &value))
}

fn parse_models_json(response: &[u8]) -> Result<Value, AppError> {
    serde_json::from_slice(response).map_err(|error| {
        summarizer_error(
            "summarizer_invalid_json",
            "The model server returned a response that could not be parsed.",
            Some(error.to_string()),
        )
    })
}

/// Pulls model ids/names out of a parsed models-list response. Split out from
/// [`list_models`] so the two response shapes (Ollama's `/api/tags` vs the
/// OpenAI-compatible `/v1/models`) can be unit tested without a real server.
fn extract_model_names(provider: SummarizerProvider, value: &Value) -> Vec<String> {
    let (array_key, name_key) = match provider {
        SummarizerProvider::Ollama => ("models", "name"),
        SummarizerProvider::LmStudio | SummarizerProvider::Custom => ("data", "id"),
    };
    value
        .get(array_key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get(name_key).and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn post(host: &str, port: u16, path: &str, body: &[u8]) -> Result<Vec<u8>, AppError> {
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut framed = request.into_bytes();
    framed.extend_from_slice(body);
    send_request(host, port, &framed)
}

fn get(host: &str, port: u16, path: &str) -> Result<Vec<u8>, AppError> {
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    send_request(host, port, request.as_bytes())
}

/// Sends a fully-framed HTTP/1.1 request and returns the parsed response body,
/// shared by [`post`] and [`get`].
fn send_request(host: &str, port: u16, framed_request: &[u8]) -> Result<Vec<u8>, AppError> {
    let address = format!("{host}:{port}")
        .parse::<std::net::SocketAddr>()
        .map_err(|error| summarizer_error("summarizer_bad_address", "Invalid model server address.", Some(error.to_string())))?;
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(map_transport_error)?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).map_err(map_transport_error)?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).map_err(map_transport_error)?;

    stream.write_all(framed_request).map_err(map_transport_error)?;
    stream.flush().map_err(map_transport_error)?;

    let mut raw = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = stream.read(&mut buffer).map_err(map_transport_error)?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.len() > MAX_RESPONSE_BYTES {
            return Err(summarizer_error(
                "summarizer_response_too_large",
                "The model server's response exceeded the size limit.",
                None,
            ));
        }
    }

    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| summarizer_error("summarizer_no_headers", "The model server's response had no header terminator.", None))?;
    let header_text = String::from_utf8_lossy(&raw[..separator]);
    let status_ok = header_text
        .lines()
        .next()
        .map(|line| line.contains(" 200"))
        .unwrap_or(false);
    let body_bytes = raw[separator + 4..].to_vec();
    let body_bytes = if header_text.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        decode_chunked(&body_bytes)
    } else {
        body_bytes
    };

    if !status_ok {
        return Err(summarizer_error(
            "summarizer_http_error",
            "The model server returned a non-success status. Is it running and the model loaded?",
            Some(header_text.lines().next().unwrap_or("").to_string()),
        ));
    }
    Ok(body_bytes)
}

fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let Some(line_end) = rest.windows(2).position(|window| window == b"\r\n") else {
            break;
        };
        let size_line = String::from_utf8_lossy(&rest[..line_end]);
        let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
        let chunk_start = line_end + 2;
        if size == 0 || chunk_start + size > rest.len() {
            break;
        }
        out.extend_from_slice(&rest[chunk_start..chunk_start + size]);
        rest = &rest[chunk_start + size + 2..];
    }
    out
}

/// Manages the LM Studio server + model lifecycle via the `lms` CLI so models
/// only occupy RAM while a summary is being produced.
pub struct LmStudioLifecycle {
    bin: PathBuf,
}

impl LmStudioLifecycle {
    pub fn resolve(bin_override: Option<&str>) -> Result<Self, AppError> {
        if let Some(path) = bin_override.filter(|value| !value.trim().is_empty()) {
            return Ok(Self { bin: PathBuf::from(path) });
        }
        if let Some(home) = std::env::var_os("HOME") {
            let candidate = PathBuf::from(home).join(".lmstudio/bin/lms");
            if candidate.exists() {
                return Ok(Self { bin: candidate });
            }
        }
        Ok(Self { bin: PathBuf::from("lms") })
    }

    pub fn ensure_running(&self) -> Result<(), AppError> {
        self.run(&["server", "start"], "lm_studio_server_start_failed", "Could not start the LM Studio server.")
    }

    pub fn load(&self, model: &str) -> Result<(), AppError> {
        self.run(&["load", model, "-y"], "lm_studio_model_load_failed", "Could not load the summarizer model in LM Studio.")
    }

    pub fn unload_all(&self) {
        let _ = Command::new(&self.bin).args(["unload", "--all"]).output();
    }

    fn run(&self, args: &[&str], code: &str, message: &str) -> Result<(), AppError> {
        let output = Command::new(&self.bin).args(args).output().map_err(|error| {
            summarizer_error(
                code,
                "Could not run the LM Studio CLI. Install LM Studio and the `lms` command.",
                Some(error.to_string()),
            )
        })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(summarizer_error(code, message, Some(String::from_utf8_lossy(&output.stderr).trim().to_string())))
        }
    }
}

fn map_transport_error(error: std::io::Error) -> AppError {
    summarizer_error(
        "summarizer_unavailable",
        "Could not reach the local model server. Make sure it's running and try again.",
        Some(error.to_string()),
    )
}

fn summarizer_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError { code: code.to_string(), message: message.to_string(), details }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::analysis::TranscriptSpeakerRole;

    struct ScriptedChat {
        replies: RefCell<Vec<String>>,
        calls: RefCell<usize>,
    }

    impl ScriptedChat {
        fn new(replies: Vec<&str>) -> Self {
            Self {
                replies: RefCell::new(replies.into_iter().map(str::to_string).collect()),
                calls: RefCell::new(0),
            }
        }
    }

    impl ChatCompletion for ScriptedChat {
        fn complete(&self, _model: &str, _system: &str, _user: &str) -> Result<String, AppError> {
            *self.calls.borrow_mut() += 1;
            Ok(self.replies.borrow_mut().remove(0))
        }
    }

    fn segment(seq: u32, speaker: &str, text: &str) -> AnalysisTranscriptSegment {
        AnalysisTranscriptSegment {
            sequence_number: seq,
            speaker_label: Some(speaker.to_string()),
            speaker_role: TranscriptSpeakerRole::User,
            text: text.to_string(),
            started_at_ms: u64::from(seq) * 1000,
            ended_at_ms: u64::from(seq) * 1000 + 900,
        }
    }

    #[test]
    fn parses_strict_json_into_notes() {
        let summary = parse_summary(
            "{\"executiveSummary\":\"We cut scope.\",\"actionItems\":[{\"owner\":\"Dana\",\"task\":\"Buy devices\",\"due\":\"Fri\"}],\"decisions\":[\"Cut saved cards\"],\"openQuestions\":[],\"speakingImprovements\":[]}",
        )
        .expect("valid json parses");
        assert_eq!(summary.executive_summary, "We cut scope.");
        assert_eq!(summary.action_items.len(), 1);
        assert_eq!(summary.action_items[0].owner.as_deref(), Some("Dana"));
        assert_eq!(summary.decisions, vec!["Cut saved cards".to_string()]);
    }

    #[test]
    fn extracts_key_topics_when_present() {
        let summary = parse_summary(
            "{\"executiveSummary\":\"Overview.\",\"keyTopics\":[{\"topic\":\"Budget\",\"points\":[\"Cut cloud spend by 10%\",\"Delayed the Q3 hire\"]},{\"topic\":\"Empty\",\"points\":[]}],\"actionItems\":[],\"decisions\":[],\"openQuestions\":[]}",
        )
        .expect("valid json parses");
        assert_eq!(summary.key_topics.len(), 2);
        assert_eq!(summary.key_topics[0].topic, "Budget");
        assert_eq!(summary.key_topics[0].points, vec!["Cut cloud spend by 10%", "Delayed the Q3 hire"]);
        assert!(summary.key_topics[1].points.is_empty());
    }

    #[test]
    fn missing_key_topics_defaults_to_empty() {
        let summary = parse_summary(
            "{\"executiveSummary\":\"Overview.\",\"actionItems\":[],\"decisions\":[],\"openQuestions\":[]}",
        )
        .expect("valid json parses");
        assert!(summary.key_topics.is_empty());
    }

    #[test]
    fn extracts_meeting_title_when_present() {
        let summary = parse_summary(
            "{\"meetingTitle\":\"Q3 Budget Review\",\"executiveSummary\":\"We cut scope.\",\"actionItems\":[],\"decisions\":[],\"openQuestions\":[]}",
        )
        .expect("valid json parses");
        assert_eq!(summary.meeting_title.as_deref(), Some("Q3 Budget Review"));
    }

    #[test]
    fn tolerates_code_fences_preamble_and_missing_fields() {
        let summary = parse_summary(
            "Sure, here are the notes:\n```json\n{\"executiveSummary\":\"Done.\",\"actionItems\":[{\"task\":\"Ship it\",\"owner\":\"null\"}]}\n```",
        )
        .expect("noisy json still parses");
        assert_eq!(summary.executive_summary, "Done.");
        assert_eq!(summary.action_items[0].owner, None);
        assert!(summary.decisions.is_empty());
        assert!(summary.open_questions.is_empty());
    }

    #[test]
    fn single_shot_makes_one_call_for_short_transcripts() {
        let chat = ScriptedChat::new(vec![
            "{\"executiveSummary\":\"Short sync.\",\"actionItems\":[],\"decisions\":[],\"openQuestions\":[]}",
        ]);
        let summarizer = LmStudioSummarizer::new(chat, "test-model");
        let segments = vec![segment(1, "Priya", "Quick standup, nothing blocking.")];
        let summary = summarizer.summarize(&segments, false).expect("summary");
        assert_eq!(summary.executive_summary, "Short sync.");
        assert_eq!(*summarizer.client.calls.borrow(), 1);
    }

    #[test]
    fn map_reduce_chunks_long_transcripts_and_makes_extra_calls() {
        let long_text = "word ".repeat(4_000);
        let segments: Vec<AnalysisTranscriptSegment> =
            (0..4).map(|i| segment(i, "Speaker", &long_text)).collect();
        let chunks = chunk_transcript(&segments, CHUNK_CHAR_TARGET);
        assert!(chunks.len() >= 2, "long transcript should split into multiple chunks");

        let mut replies: Vec<&str> = vec!["- point a"; chunks.len()];
        replies.push("{\"executiveSummary\":\"Reduced.\",\"actionItems\":[],\"decisions\":[],\"openQuestions\":[]}");
        let chat = ScriptedChat::new(replies);
        let summarizer = LmStudioSummarizer::new(chat, "test-model");
        let summary = summarizer.summarize(&segments, false).expect("summary");
        assert_eq!(summary.executive_summary, "Reduced.");
        assert_eq!(*summarizer.client.calls.borrow(), chunks.len() + 1);
    }

    #[test]
    fn empty_transcript_is_rejected() {
        let chat = ScriptedChat::new(vec![]);
        let summarizer = LmStudioSummarizer::new(chat, "test-model");
        let error = summarizer.summarize(&[], false).expect_err("empty transcript errors");
        assert_eq!(error.code, "summary_empty_transcript");
    }

    #[test]
    fn extracts_model_ids_from_openai_compatible_models_list() {
        let value: Value = serde_json::from_str(
            r#"{"data":[{"id":"google/gemma-4-26b-a4b-qat"},{"id":"llama-3.2-3b"}]}"#,
        )
        .expect("valid json");
        let names = extract_model_names(SummarizerProvider::LmStudio, &value);
        assert_eq!(names, vec!["google/gemma-4-26b-a4b-qat", "llama-3.2-3b"]);

        let names = extract_model_names(SummarizerProvider::Custom, &value);
        assert_eq!(names, vec!["google/gemma-4-26b-a4b-qat", "llama-3.2-3b"]);
    }

    #[test]
    fn extracts_model_names_from_ollama_tags_list() {
        let value: Value = serde_json::from_str(r#"{"models":[{"name":"llama3.2:latest"},{"name":"mistral:7b"}]}"#)
            .expect("valid json");
        let names = extract_model_names(SummarizerProvider::Ollama, &value);
        assert_eq!(names, vec!["llama3.2:latest", "mistral:7b"]);
    }

    #[test]
    fn extract_model_names_tolerates_missing_or_malformed_fields() {
        let value: Value = serde_json::from_str(r#"{"unexpected":"shape"}"#).expect("valid json");
        assert!(extract_model_names(SummarizerProvider::LmStudio, &value).is_empty());
        assert!(extract_model_names(SummarizerProvider::Ollama, &value).is_empty());
    }
}
