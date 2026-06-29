use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tempfile::{tempdir, NamedTempFile, TempDir};

use crate::domain::{AppError, Score};

const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const DEFAULT_OPENAI_MAX_FRAMES: usize = 12;
const OPENAI_MAX_FRAME_CAP: usize = 16;
const DEFAULT_OPENAI_FRAME_INTERVAL_SECONDS: u64 = 10;
const OPENAI_MIN_FRAME_INTERVAL_SECONDS: u64 = 5;
const OPENAI_MAX_FRAME_INTERVAL_SECONDS: u64 = 60;
const MAX_OPENAI_RESPONSE_BYTES: usize = 4 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PracticeVideoReviewRequest {
    pub practice_recording_id: String,
    pub video_file_path: String,
    pub ffmpeg_bin_path: Option<String>,
    pub matched_speech_windows: Vec<VideoReviewWindow>,
    pub allow_cloud_video_for_this_review: bool,
    pub cloud_video_review_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoReviewWindow {
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeVideoReview {
    pub visual_score: Option<Score>,
    pub summary: String,
    pub annotations: Vec<PracticeVideoAnnotation>,
    pub cloud_video_used: bool,
    pub user_visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeVideoAnnotation {
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub category: String,
    pub severity: String,
    pub evidence: String,
    pub suggestion: String,
    pub source: String,
}

pub trait VideoReviewAnalyzer {
    fn analyze_practice_video(
        &self,
        request: &PracticeVideoReviewRequest,
    ) -> Result<PracticeVideoReview, AppError>;
}

pub struct DisabledCloudVideoReviewer;

impl VideoReviewAnalyzer for DisabledCloudVideoReviewer {
    fn analyze_practice_video(
        &self,
        request: &PracticeVideoReviewRequest,
    ) -> Result<PracticeVideoReview, AppError> {
        ensure_video_review_consent(request)?;
        Err(video_review_error(
            "cloud_video_reviewer_not_configured",
            "Cloud video review has consent but no provider adapter is configured yet.",
            None,
        ))
    }
}

pub struct OpenAiVideoReviewer<T: OpenAiVideoTransport = CurlOpenAiVideoTransport> {
    transport: T,
}

impl OpenAiVideoReviewer<CurlOpenAiVideoTransport> {
    pub fn from_environment() -> Self {
        Self {
            transport: CurlOpenAiVideoTransport,
        }
    }
}

impl<T: OpenAiVideoTransport> OpenAiVideoReviewer<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: OpenAiVideoTransport> VideoReviewAnalyzer for OpenAiVideoReviewer<T> {
    fn analyze_practice_video(
        &self,
        request: &PracticeVideoReviewRequest,
    ) -> Result<PracticeVideoReview, AppError> {
        ensure_video_review_consent(request)?;
        let config = OpenAiVideoReviewConfig::from_environment()?;
        let frame_set = extract_review_frames(request, &config)?;
        let body = build_openai_video_review_request(&config, request, &frame_set.frames)?;
        let response = self.transport.send_review_request(&config.api_key, &body)?;
        parse_openai_video_review_response(&response)
    }
}

pub trait OpenAiVideoTransport {
    fn send_review_request(&self, api_key: &str, body: &Value) -> Result<Vec<u8>, AppError>;
}

pub struct CurlOpenAiVideoTransport;

impl OpenAiVideoTransport for CurlOpenAiVideoTransport {
    fn send_review_request(&self, api_key: &str, body: &Value) -> Result<Vec<u8>, AppError> {
        let mut body_file = NamedTempFile::new().map_err(|error| {
            video_review_error(
                "openai_request_body_tempfile_failed",
                "Could not prepare the OpenAI video review request.",
                Some(error.to_string()),
            )
        })?;
        serde_json::to_writer(&mut body_file, body).map_err(|error| {
            video_review_error(
                "openai_request_serialization_failed",
                "Could not serialize the OpenAI video review request.",
                Some(error.to_string()),
            )
        })?;
        body_file.flush().map_err(|error| {
            video_review_error(
                "openai_request_body_write_failed",
                "Could not write the OpenAI video review request.",
                Some(error.to_string()),
            )
        })?;

        let config = build_curl_config(api_key, body_file.path(), OPENAI_REQUEST_TIMEOUT);
        let mut child = Command::new("curl")
            .arg("--config")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                video_review_error(
                    "openai_curl_start_failed",
                    "Could not start curl for OpenAI video review.",
                    Some(error.to_string()),
                )
            })?;

        {
            let stdin = child.stdin.as_mut().ok_or_else(|| {
                video_review_error(
                    "openai_curl_stdin_unavailable",
                    "Could not pass request configuration to curl.",
                    None,
                )
            })?;
            stdin.write_all(config.as_bytes()).map_err(|error| {
                video_review_error(
                    "openai_curl_config_write_failed",
                    "Could not configure the OpenAI video review request.",
                    Some(error.to_string()),
                )
            })?;
        }

        let output = child.wait_with_output().map_err(|error| {
            video_review_error(
                "openai_curl_wait_failed",
                "Could not read the OpenAI video review response.",
                Some(error.to_string()),
            )
        })?;
        if output.stdout.len() > MAX_OPENAI_RESPONSE_BYTES {
            return Err(video_review_error(
                "openai_response_too_large",
                "OpenAI video review response exceeded the safe size limit.",
                None,
            ));
        }
        if !output.status.success() {
            return Err(openai_status_error(&output.stdout, &output.stderr));
        }
        Ok(output.stdout)
    }
}

#[cfg(test)]
pub struct FixtureVideoReviewer;

#[cfg(test)]
impl VideoReviewAnalyzer for FixtureVideoReviewer {
    fn analyze_practice_video(
        &self,
        request: &PracticeVideoReviewRequest,
    ) -> Result<PracticeVideoReview, AppError> {
        ensure_video_review_consent(request)?;
        Ok(PracticeVideoReview {
            visual_score: Some(Score::new(78)?),
            summary: "Fixture visual review found steady framing with one eye-contact drift."
                .to_string(),
            cloud_video_used: true,
            user_visible: Some(true),
            annotations: vec![PracticeVideoAnnotation {
                started_at_ms: 10_000,
                ended_at_ms: 18_000,
                category: "eyeContact".to_string(),
                severity: "caution".to_string(),
                evidence: "Gaze moved away from the camera during the key point.".to_string(),
                suggestion: "Return to the lens before delivering the next sentence.".to_string(),
                source: "videoCloud".to_string(),
            }],
        })
    }
}

pub fn ensure_video_review_consent(request: &PracticeVideoReviewRequest) -> Result<(), AppError> {
    if !request.cloud_video_review_enabled {
        return Err(video_review_error(
            "cloud_video_review_disabled",
            "Enable cloud video review before sending a full practice video to a provider.",
            None,
        ));
    }
    if !request.allow_cloud_video_for_this_review {
        return Err(video_review_error(
            "cloud_video_review_confirmation_required",
            "Confirm this specific review before sending the full practice video to a cloud provider.",
            None,
        ));
    }
    if !Path::new(&request.video_file_path).is_file() {
        return Err(video_review_error(
            "practice_video_not_found",
            "The practice video file could not be found for visual review.",
            None,
        ));
    }
    Ok(())
}

fn video_review_error(code: &str, message: &str, details: Option<String>) -> AppError {
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

    #[test]
    fn disabled_reviewer_requires_saved_and_per_review_video_consent() {
        let video = NamedTempFile::new().expect("video fixture can be created");
        let reviewer = DisabledCloudVideoReviewer;

        let disabled_error = reviewer
            .analyze_practice_video(&PracticeVideoReviewRequest {
                practice_recording_id: "practice-1".to_string(),
                video_file_path: video.path().to_string_lossy().into_owned(),
                ffmpeg_bin_path: None,
                matched_speech_windows: Vec::new(),
                allow_cloud_video_for_this_review: true,
                cloud_video_review_enabled: false,
            })
            .expect_err("saved setting is required");
        assert_eq!(disabled_error.code, "cloud_video_review_disabled");

        let confirmation_error = reviewer
            .analyze_practice_video(&PracticeVideoReviewRequest {
                practice_recording_id: "practice-1".to_string(),
                video_file_path: video.path().to_string_lossy().into_owned(),
                ffmpeg_bin_path: None,
                matched_speech_windows: Vec::new(),
                allow_cloud_video_for_this_review: false,
                cloud_video_review_enabled: true,
            })
            .expect_err("per-review confirmation is required");
        assert_eq!(
            confirmation_error.code,
            "cloud_video_review_confirmation_required"
        );
    }

    #[test]
    fn fixture_reviewer_returns_deterministic_visual_annotations() {
        let video = NamedTempFile::new().expect("video fixture can be created");
        let review = FixtureVideoReviewer
            .analyze_practice_video(&PracticeVideoReviewRequest {
                practice_recording_id: "practice-1".to_string(),
                video_file_path: video.path().to_string_lossy().into_owned(),
                ffmpeg_bin_path: None,
                matched_speech_windows: Vec::new(),
                allow_cloud_video_for_this_review: true,
                cloud_video_review_enabled: true,
            })
            .expect("fixture review succeeds with consent");

        assert_eq!(
            review.visual_score,
            Some(Score::new(78).expect("score is valid"))
        );
        assert_eq!(review.annotations[0].category, "eyeContact");
        assert!(review.cloud_video_used);
    }

    #[test]
    fn openai_response_parser_accepts_strict_review_json() {
        let response = br#"{
            "output": [{
                "content": [{
                    "type": "output_text",
                    "text": "{\"userVisible\":true,\"visualScore\":82,\"summary\":\"Framing and posture were steady in sampled frames.\",\"annotations\":[{\"startedAtMs\":0,\"endedAtMs\":10000,\"category\":\"framing\",\"severity\":\"info\",\"evidence\":\"The speaker remained centered in the sampled opening frame.\",\"suggestion\":\"Keep this centered framing for the rest of the talk.\"}]}"
                }]
            }]
        }"#;

        let review = parse_openai_video_review_response(response)
            .expect("valid OpenAI review response can be parsed");

        assert_eq!(
            review.visual_score,
            Some(Score::new(82).expect("score is valid"))
        );
        assert_eq!(review.annotations[0].source, "videoCloud");
        assert!(review.cloud_video_used);
        assert_eq!(review.user_visible, Some(true));
    }

    #[test]
    fn openai_response_parser_rejects_invalid_annotation_ranges() {
        let response = br#"{
            "output": [{
                "content": [{
                    "type": "output_text",
                    "text": "{\"userVisible\":true,\"visualScore\":82,\"summary\":\"Sampled frame review.\",\"annotations\":[{\"startedAtMs\":20000,\"endedAtMs\":10000,\"category\":\"movement\",\"severity\":\"caution\",\"evidence\":\"Movement changed between sampled frames.\",\"suggestion\":\"Reduce shifting during key points.\"}]}"
                }]
            }]
        }"#;

        let error = parse_openai_video_review_response(response)
            .expect_err("invalid timestamp ranges are rejected");

        assert_eq!(error.code, "openai_review_timestamp_invalid");
    }

    #[test]
    fn base64_encoder_handles_padding() {
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
        assert_eq!(encode_base64(b"hi"), "aGk=");
        assert_eq!(encode_base64(b"?"), "Pw==");
    }

    #[test]
    fn openai_api_key_rejects_control_characters() {
        let error = validate_openai_api_key("sk-test\nurl = \"https://example.com\"".to_string())
            .expect_err("control characters are rejected");

        assert_eq!(error.code, "openai_api_key_invalid");
    }

    #[test]
    fn curl_config_escaping_escapes_line_breaks() {
        let escaped = escape_curl_config_value("sk-test\nnext\r\"quote\"");

        assert_eq!(escaped, "sk-test\\nnext\\r\\\"quote\\\"");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenAiVideoReviewConfig {
    api_key: String,
    model: String,
    max_frames: usize,
    frame_interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewFrame {
    timestamp_ms: u64,
    path: PathBuf,
}

#[derive(Debug)]
struct ReviewFrameSet {
    _temp_dir: TempDir,
    frames: Vec<ReviewFrame>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAiPracticeVideoReview {
    user_visible: bool,
    visual_score: Option<u8>,
    summary: String,
    annotations: Vec<OpenAiPracticeVideoAnnotation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAiPracticeVideoAnnotation {
    started_at_ms: u64,
    ended_at_ms: u64,
    category: String,
    severity: String,
    evidence: String,
    suggestion: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesApiResponse {
    output: Vec<OpenAiOutputItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputItem {
    content: Option<Vec<OpenAiOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorBody {
    error: Option<OpenAiErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorDetail {
    message: Option<String>,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
}

impl OpenAiVideoReviewConfig {
    fn from_environment() -> Result<Self, AppError> {
        let api_key = std::env::var("RESONANCE_OPENAI_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                video_review_error(
                    "openai_api_key_missing",
                    "Set RESONANCE_OPENAI_API_KEY before running cloud video review.",
                    Some(
                        "The key is read at runtime and is never stored by Resonance.".to_string(),
                    ),
                )
            })
            .and_then(validate_openai_api_key)?;
        let model = std::env::var("RESONANCE_OPENAI_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string());
        let max_frames = read_usize_env("RESONANCE_OPENAI_MAX_FRAMES", DEFAULT_OPENAI_MAX_FRAMES)
            .clamp(1, OPENAI_MAX_FRAME_CAP);
        let frame_interval_seconds = read_u64_env(
            "RESONANCE_OPENAI_FRAME_INTERVAL_SECONDS",
            DEFAULT_OPENAI_FRAME_INTERVAL_SECONDS,
        )
        .clamp(
            OPENAI_MIN_FRAME_INTERVAL_SECONDS,
            OPENAI_MAX_FRAME_INTERVAL_SECONDS,
        );

        Ok(Self {
            api_key,
            model,
            max_frames,
            frame_interval_seconds,
        })
    }
}

fn extract_review_frames(
    request: &PracticeVideoReviewRequest,
    config: &OpenAiVideoReviewConfig,
) -> Result<ReviewFrameSet, AppError> {
    let ffmpeg_path = crate::media_import::resolve_ffmpeg_path(request.ffmpeg_bin_path.as_deref())?;
    let frame_dir = tempdir().map_err(|error| {
        video_review_error(
            "practice_frame_tempdir_failed",
            "Could not prepare temporary frame extraction storage.",
            Some(error.to_string()),
        )
    })?;
    let frame_pattern = frame_dir.path().join("frame-%03d.jpg");
    let fps_filter = format!(
        "fps=1/{},scale=640:-1:flags=lanczos",
        config.frame_interval_seconds
    );
    let output = Command::new(ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(&request.video_file_path)
        .arg("-vf")
        .arg(fps_filter)
        .arg("-frames:v")
        .arg(config.max_frames.to_string())
        .arg("-q:v")
        .arg("4")
        .arg(&frame_pattern)
        .output()
        .map_err(|error| {
            video_review_error(
                "practice_frame_extract_start_failed",
                "Could not start ffmpeg for practice video frame extraction.",
                Some(error.to_string()),
            )
        })?;

    if !output.status.success() {
        return Err(video_review_error(
            "practice_frame_extract_failed",
            "ffmpeg could not extract sample frames from the practice video.",
            Some(
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(500)
                    .collect(),
            ),
        ));
    }

    let mut frames = fs::read_dir(frame_dir.path())
        .map_err(|error| {
            video_review_error(
                "practice_frame_read_failed",
                "Could not read extracted practice video frames.",
                Some(error.to_string()),
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jpg"))
        .collect::<Vec<_>>();
    frames.sort();
    if frames.is_empty() {
        return Err(video_review_error(
            "practice_frame_extract_empty",
            "No sample frames could be extracted from the practice video.",
            None,
        ));
    }

    let review_frames = frames
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            Ok::<ReviewFrame, AppError>(ReviewFrame {
                timestamp_ms: index as u64 * config.frame_interval_seconds * 1_000,
                path,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ReviewFrameSet {
        _temp_dir: frame_dir,
        frames: review_frames,
    })
}

fn build_openai_video_review_request(
    config: &OpenAiVideoReviewConfig,
    request: &PracticeVideoReviewRequest,
    frames: &[ReviewFrame],
) -> Result<Value, AppError> {
    let mut content = vec![json!({
        "type": "input_text",
        "text": build_openai_video_review_prompt(request, frames, config),
    })];
    for frame in frames {
        let bytes = fs::read(&frame.path).map_err(|error| {
            video_review_error(
                "practice_frame_read_failed",
                "Could not read an extracted practice video frame.",
                Some(error.to_string()),
            )
        })?;
        content.push(json!({
            "type": "input_image",
            "image_url": format!("data:image/jpeg;base64,{}", encode_base64(&bytes)),
            "detail": "low",
        }));
    }

    Ok(json!({
        "model": config.model,
        "input": [{
            "role": "user",
            "content": content,
        }],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "practice_video_review",
                "strict": true,
                "schema": openai_video_review_json_schema(),
            }
        },
        "max_output_tokens": 1200,
    }))
}

fn build_openai_video_review_prompt(
    request: &PracticeVideoReviewRequest,
    frames: &[ReviewFrame],
    config: &OpenAiVideoReviewConfig,
) -> String {
    let frame_timestamps = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| format!("frame {}: {} ms", index + 1, frame.timestamp_ms))
        .collect::<Vec<_>>()
        .join(", ");
    let matched_window_context = if request.matched_speech_windows.is_empty() {
        "This is a self-practice video; assume the visible presenter is the user unless the sampled frames clearly show no presenter.".to_string()
    } else {
        let windows = request
            .matched_speech_windows
            .iter()
            .map(|window| format!("{}-{} ms", window.started_at_ms, window.ended_at_ms))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "This is a meeting recording. The user's voice was locally matched in these timestamp windows: {windows}. Only provide visual feedback if the sampled frames show the likely active speaker/user on camera around those windows. If the user's camera appears off, hidden, or not identifiable in sampled frames, set userVisible=false, visualScore=null, and return no annotations."
        )
    };
    format!(
        "You are reviewing a video for Resonance. The user explicitly opted in to cloud video review for recording {}. {} Analyze only visible presentation delivery: eye contact with camera, posture, hand gesture usefulness, framing, lighting/background distractions, facial expressiveness, and movement stability. Do not infer sensitive traits or identity. Return concise JSON only. Use these frame timestamps for annotation ranges: {}. The frames were sampled every {} seconds and capped at {} frames, so phrase evidence as visual cues from sampled frames rather than full-video certainty.",
        request.practice_recording_id,
        matched_window_context,
        frame_timestamps,
        config.frame_interval_seconds,
        config.max_frames,
    )
}

fn openai_video_review_json_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["userVisible", "visualScore", "summary", "annotations"],
        "properties": {
            "userVisible": {
                "type": "boolean"
            },
            "visualScore": {
                "type": ["integer", "null"],
                "minimum": 0,
                "maximum": 100
            },
            "summary": {
                "type": "string",
                "minLength": 1,
                "maxLength": 600
            },
            "annotations": {
                "type": "array",
                "maxItems": 12,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "startedAtMs",
                        "endedAtMs",
                        "category",
                        "severity",
                        "evidence",
                        "suggestion"
                    ],
                    "properties": {
                        "startedAtMs": { "type": "integer", "minimum": 0 },
                        "endedAtMs": { "type": "integer", "minimum": 0 },
                        "category": {
                            "type": "string",
                            "enum": [
                                "eyeContact",
                                "posture",
                                "gesture",
                                "framing",
                                "lighting",
                                "facialExpression",
                                "movement"
                            ]
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["info", "caution", "strong"]
                        },
                        "evidence": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 500
                        },
                        "suggestion": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 500
                        }
                    }
                }
            }
        }
    })
}

fn parse_openai_video_review_response(response: &[u8]) -> Result<PracticeVideoReview, AppError> {
    let api_response =
        serde_json::from_slice::<OpenAiResponsesApiResponse>(response).map_err(|error| {
            video_review_error(
                "openai_response_parse_failed",
                "OpenAI video review returned an invalid response envelope.",
                Some(error.to_string()),
            )
        })?;
    let output_text = api_response
        .output
        .iter()
        .filter_map(|item| item.content.as_ref())
        .flat_map(|content| content.iter())
        .find_map(|content| {
            (content.content_type == "output_text")
                .then(|| content.text.as_ref())
                .flatten()
        })
        .ok_or_else(|| {
            video_review_error(
                "openai_response_text_missing",
                "OpenAI video review did not return review JSON.",
                None,
            )
        })?;
    let review =
        serde_json::from_str::<OpenAiPracticeVideoReview>(output_text).map_err(|error| {
            video_review_error(
                "openai_review_json_invalid",
                "OpenAI video review JSON did not match the expected schema.",
                Some(error.to_string()),
            )
        })?;
    convert_openai_review(review)
}

fn convert_openai_review(
    review: OpenAiPracticeVideoReview,
) -> Result<PracticeVideoReview, AppError> {
    if review.summary.trim().is_empty() {
        return Err(video_review_error(
            "openai_review_summary_empty",
            "OpenAI video review returned an empty summary.",
            None,
        ));
    }
    let visual_score = review
        .visual_score
        .map(Score::new)
        .transpose()
        .map_err(|error| {
            video_review_error(
                "openai_review_score_invalid",
                "OpenAI video review returned an invalid score.",
                Some(error.message),
            )
        })?;
    let annotations = review
        .annotations
        .into_iter()
        .take(12)
        .map(convert_openai_annotation)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PracticeVideoReview {
        visual_score,
        summary: review.summary,
        annotations,
        cloud_video_used: true,
        user_visible: Some(review.user_visible),
    })
}

fn convert_openai_annotation(
    annotation: OpenAiPracticeVideoAnnotation,
) -> Result<PracticeVideoAnnotation, AppError> {
    if annotation.ended_at_ms < annotation.started_at_ms {
        return Err(video_review_error(
            "openai_review_timestamp_invalid",
            "OpenAI video review returned an annotation with an invalid timestamp range.",
            None,
        ));
    }
    if !matches!(annotation.severity.as_str(), "info" | "caution" | "strong") {
        return Err(video_review_error(
            "openai_review_severity_invalid",
            "OpenAI video review returned an unsupported annotation severity.",
            Some(annotation.severity),
        ));
    }
    Ok(PracticeVideoAnnotation {
        started_at_ms: annotation.started_at_ms,
        ended_at_ms: annotation.ended_at_ms,
        category: annotation.category,
        severity: annotation.severity,
        evidence: annotation.evidence,
        suggestion: annotation.suggestion,
        source: "videoCloud".to_string(),
    })
}

fn openai_status_error(stdout: &[u8], stderr: &[u8]) -> AppError {
    if let Ok(body) = serde_json::from_slice::<OpenAiErrorBody>(stdout) {
        if let Some(error) = body.error {
            let code = error
                .code
                .or(error.error_type)
                .unwrap_or_else(|| "openai_request_failed".to_string());
            return video_review_error(
                "openai_request_failed",
                "OpenAI video review request failed.",
                Some(format!(
                    "{}: {}",
                    code,
                    error
                        .message
                        .unwrap_or_else(|| "No message returned.".to_string())
                )),
            );
        }
    }
    video_review_error(
        "openai_request_failed",
        "OpenAI video review request failed.",
        Some(String::from_utf8_lossy(stderr).chars().take(500).collect()),
    )
}

fn build_curl_config(api_key: &str, body_path: &Path, timeout: Duration) -> String {
    format!(
        "url = \"{}\"\nrequest = \"POST\"\nsilent\nshow-error\nfail-with-body\nmax-time = \"{}\"\nheader = \"Authorization: Bearer {}\"\nheader = \"Content-Type: application/json\"\ndata-binary = \"@{}\"\n",
        OPENAI_RESPONSES_URL,
        timeout.as_secs(),
        escape_curl_config_value(api_key),
        escape_curl_config_value(&body_path.to_string_lossy()),
    )
}

fn escape_curl_config_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn validate_openai_api_key(api_key: String) -> Result<String, AppError> {
    if api_key.chars().any(char::is_control) {
        return Err(video_review_error(
            "openai_api_key_invalid",
            "OpenAI API key contains invalid control characters.",
            None,
        ));
    }
    Ok(api_key)
}

fn read_usize_env(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn read_u64_env(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        let combined = ((first as u32) << 16) | ((second as u32) << 8) | third as u32;
        output.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[((combined >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(combined & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}
