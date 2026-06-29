use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;
#[cfg(feature = "speaker-matching-sherpa")]
use sherpa_onnx::{
    FastClusteringConfig, LinearResampler, OfflineSpeakerDiarization,
    OfflineSpeakerDiarizationConfig, OfflineSpeakerSegmentationModelConfig,
    OfflineSpeakerSegmentationPyannoteModelConfig, SpeakerEmbeddingExtractor,
    SpeakerEmbeddingExtractorConfig,
};

use crate::domain::{AppError, ResonanceSettings};

#[cfg(any(test, feature = "speaker-matching-sherpa"))]
const MAX_DIARIZATION_TIMESTAMP_SECONDS: f32 = 86_400.0 * 365.0;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMatcherStatus {
    pub model_configured: bool,
    pub model_path: Option<String>,
    pub extractor_ready: bool,
    pub embedding_dimension: Option<i32>,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiarizationStatus {
    pub model_configured: bool,
    pub model_path: Option<String>,
    pub diarization_ready: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMatchResult {
    pub is_match: bool,
    pub similarity_score: f32,
    pub threshold: f32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizedSpeakerSegment {
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub speaker: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiarizationResult {
    pub speaker_count: u32,
    pub segment_count: u32,
    pub segments: Vec<DiarizedSpeakerSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizedSpeakerMatch {
    pub speaker: i32,
    pub is_match: bool,
    pub similarity_score: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizedVoiceMatchWindow {
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub speaker: i32,
    pub similarity_score: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiarizationMatchResult {
    pub speaker_count: u32,
    pub segment_count: u32,
    pub matched_window_count: u32,
    pub speaker_matches: Vec<DiarizedSpeakerMatch>,
    pub matched_windows: Vec<DiarizedVoiceMatchWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedVoiceEmbedding {
    pub embedding_json: String,
    pub embedding_dimension: u32,
    pub embedding_model_path: String,
}

pub fn voice_matcher_status(settings: &ResonanceSettings) -> VoiceMatcherStatus {
    let Some(model_path) = settings
        .speaker_embedding_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    else {
        return VoiceMatcherStatus {
            model_configured: false,
            model_path: None,
            extractor_ready: false,
            embedding_dimension: None,
            message: "Speaker matching is not configured. Add a local speaker embedding ONNX model to enable readiness checks.".to_string(),
        };
    };

    let model_path_text = model_path.to_string_lossy().into_owned();
    if !model_path.is_absolute() {
        return not_ready(
            model_path_text,
            "Speaker embedding model path must be absolute.",
        );
    }

    if !model_path.is_file() {
        return not_ready(
            model_path_text,
            "Speaker embedding model file was not found.",
        );
    }

    create_extractor_status(&model_path, model_path_text)
}

pub fn voice_diarization_status(settings: &ResonanceSettings) -> VoiceDiarizationStatus {
    let Some(model_path) = settings
        .speaker_segmentation_model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    else {
        return VoiceDiarizationStatus {
            model_configured: false,
            model_path: None,
            diarization_ready: false,
            message: "Speaker diarization is not configured. Add a local speaker segmentation ONNX model to prepare segment-level matching.".to_string(),
        };
    };

    let model_path_text = model_path.to_string_lossy().into_owned();
    if !model_path.is_absolute() {
        return diarization_not_ready(
            model_path_text,
            "Speaker segmentation model path must be absolute.",
        );
    }

    if !model_path.is_file() {
        return diarization_not_ready(
            model_path_text,
            "Speaker segmentation model file was not found.",
        );
    }

    VoiceDiarizationStatus {
        model_configured: true,
        model_path: Some(model_path_text),
        diarization_ready: true,
        message: "Speaker segmentation model is configured. Segment-level diarization will be wired in the next slice.".to_string(),
    }
}

fn create_extractor_status(model_path: &Path, model_path_text: String) -> VoiceMatcherStatus {
    create_extractor_status_with_sherpa(model_path, model_path_text)
}

#[cfg(feature = "speaker-matching-sherpa")]
fn create_extractor_status_with_sherpa(
    model_path: &Path,
    model_path_text: String,
) -> VoiceMatcherStatus {
    let config = SpeakerEmbeddingExtractorConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        num_threads: 1,
        debug: false,
        provider: Some("cpu".to_string()),
    };

    match SpeakerEmbeddingExtractor::create(&config) {
        Some(extractor) => VoiceMatcherStatus {
            model_configured: true,
            model_path: Some(model_path_text),
            extractor_ready: true,
            embedding_dimension: Some(extractor.dim()),
            message: "Speaker embedding extractor is ready. Full voice matching will be wired in a later slice.".to_string(),
        },
        None => not_ready(
            model_path_text,
            "Speaker embedding extractor could not be constructed from this model.",
        ),
    }
}

#[cfg(not(feature = "speaker-matching-sherpa"))]
fn create_extractor_status_with_sherpa(
    _model_path: &Path,
    model_path_text: String,
) -> VoiceMatcherStatus {
    not_ready(
        model_path_text,
        "Speaker matching dependency is present but not enabled in this build.",
    )
}

fn not_ready(model_path: String, message: &str) -> VoiceMatcherStatus {
    VoiceMatcherStatus {
        model_configured: true,
        model_path: Some(model_path),
        extractor_ready: false,
        embedding_dimension: None,
        message: message.to_string(),
    }
}

fn diarization_not_ready(model_path: String, message: &str) -> VoiceDiarizationStatus {
    VoiceDiarizationStatus {
        model_configured: true,
        model_path: Some(model_path),
        diarization_ready: false,
        message: message.to_string(),
    }
}

pub fn prepare_voice_embedding(
    sample_audio_path: &Path,
    model_path: &Path,
) -> Result<PreparedVoiceEmbedding, AppError> {
    prepare_voice_embedding_with_sherpa(sample_audio_path, model_path)
}

pub fn diarize_speakers(
    audio_path: &Path,
    segmentation_model_path: &Path,
    embedding_model_path: &Path,
) -> Result<VoiceDiarizationResult, AppError> {
    validate_voice_matching_file_path(
        segmentation_model_path,
        "invalid_speaker_segmentation_model_path",
        "Speaker segmentation model path",
    )?;
    validate_voice_matching_file_path(
        embedding_model_path,
        "invalid_speaker_embedding_model_path",
        "Speaker embedding model path",
    )?;
    validate_voice_matching_file_path(
        audio_path,
        "invalid_speaker_diarization_audio_path",
        "Diarization audio path",
    )?;

    diarize_speakers_with_sherpa(audio_path, segmentation_model_path, embedding_model_path)
}

pub fn match_diarized_speakers(
    audio_path: &Path,
    segmentation_model_path: &Path,
    embedding_model_path: &Path,
    enrolled_embedding: &[f32],
    threshold: f32,
) -> Result<VoiceDiarizationMatchResult, AppError> {
    let diarization = diarize_speakers(audio_path, segmentation_model_path, embedding_model_path)?;
    let speaker_matches = match_diarized_speakers_with_sherpa(
        audio_path,
        embedding_model_path,
        enrolled_embedding,
        &diarization,
        threshold,
    )?;

    match_diarized_speaker_windows(&diarization, speaker_matches, threshold)
}

pub fn match_diarized_speaker_windows(
    diarization: &VoiceDiarizationResult,
    mut speaker_matches: Vec<DiarizedSpeakerMatch>,
    threshold: f32,
) -> Result<VoiceDiarizationMatchResult, AppError> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err(voice_matching_error(
            "speaker_match_threshold_invalid",
            "Speaker match threshold must be between 0.0 and 1.0.",
            Some(format!("threshold={threshold}")),
        ));
    }

    speaker_matches.sort_by_key(|speaker_match| speaker_match.speaker);
    let matched_speakers = speaker_matches
        .iter()
        .filter(|speaker_match| speaker_match.similarity_score >= threshold)
        .map(|speaker_match| (speaker_match.speaker, speaker_match))
        .collect::<BTreeMap<_, _>>();
    let matched_windows = diarization
        .segments
        .iter()
        .filter_map(|segment| {
            matched_speakers
                .get(&segment.speaker)
                .map(|speaker_match| DiarizedVoiceMatchWindow {
                    started_at_ms: segment.started_at_ms,
                    ended_at_ms: segment.ended_at_ms,
                    speaker: segment.speaker,
                    similarity_score: speaker_match.similarity_score,
                    threshold: speaker_match.threshold,
                })
        })
        .collect::<Vec<_>>();
    let matched_window_count = u32::try_from(matched_windows.len()).map_err(|error| {
        voice_matching_error(
            "speaker_match_window_count_invalid",
            "Matched speaker window count is too large.",
            Some(error.to_string()),
        )
    })?;

    Ok(VoiceDiarizationMatchResult {
        speaker_count: diarization.speaker_count,
        segment_count: diarization.segment_count,
        matched_window_count,
        speaker_matches,
        matched_windows,
    })
}

pub fn compare_voice_embeddings(
    enrolled_embedding: &[f32],
    candidate_embedding: &[f32],
    threshold: f32,
) -> Result<VoiceMatchResult, AppError> {
    if enrolled_embedding.is_empty() || candidate_embedding.is_empty() {
        return Err(voice_matching_error(
            "speaker_embedding_empty",
            "Speaker embeddings must not be empty.",
            None,
        ));
    }
    if enrolled_embedding.len() != candidate_embedding.len() {
        return Err(voice_matching_error(
            "speaker_embedding_dimension_mismatch",
            "Speaker embeddings must have the same dimension.",
            Some(format!(
                "enrolled_dimension={}, candidate_dimension={}",
                enrolled_embedding.len(),
                candidate_embedding.len()
            )),
        ));
    }
    if !(0.0..=1.0).contains(&threshold) {
        return Err(voice_matching_error(
            "speaker_match_threshold_invalid",
            "Speaker match threshold must be between 0.0 and 1.0.",
            Some(format!("threshold={threshold}")),
        ));
    }

    let similarity_score = cosine_similarity(enrolled_embedding, candidate_embedding)?;
    let is_match = similarity_score >= threshold;
    Ok(VoiceMatchResult {
        is_match,
        similarity_score,
        threshold,
        message: if is_match {
            "Candidate audio matches the prepared local voice profile.".to_string()
        } else {
            "Candidate audio did not match the prepared local voice profile.".to_string()
        },
    })
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32, AppError> {
    let (dot_product, left_norm_squared, right_norm_squared) = left.iter().zip(right.iter()).fold(
        (0.0_f32, 0.0_f32, 0.0_f32),
        |accumulator, (left, right)| {
            (
                accumulator.0 + left * right,
                accumulator.1 + left * left,
                accumulator.2 + right * right,
            )
        },
    );
    if left_norm_squared == 0.0 || right_norm_squared == 0.0 {
        return Err(voice_matching_error(
            "speaker_embedding_zero_norm",
            "Speaker embeddings must have non-zero magnitude.",
            None,
        ));
    }

    Ok(dot_product / (left_norm_squared.sqrt() * right_norm_squared.sqrt()))
}

#[cfg(feature = "speaker-matching-sherpa")]
fn prepare_voice_embedding_with_sherpa(
    sample_audio_path: &Path,
    model_path: &Path,
) -> Result<PreparedVoiceEmbedding, AppError> {
    if !model_path.is_absolute() {
        return Err(voice_matching_error(
            "invalid_speaker_embedding_model_path",
            "Speaker embedding model path must be absolute.",
            None,
        ));
    }
    if !model_path.is_file() {
        return Err(voice_matching_error(
            "speaker_embedding_model_not_found",
            "Speaker embedding model file was not found.",
            None,
        ));
    }

    let wave = sherpa_onnx::Wave::read(&sample_audio_path.to_string_lossy()).ok_or_else(|| {
        voice_matching_error(
            "voice_profile_sample_read_failed",
            "Could not read the local voice enrollment sample.",
            None,
        )
    })?;
    let embedding = compute_voice_embedding_values(
        wave.samples(),
        wave.sample_rate(),
        model_path,
        "voice_profile_sample_too_short",
        "Voice enrollment sample is too short to compute a speaker embedding.",
        "Could not compute a speaker embedding from the local voice profile.",
    )?;
    let embedding_dimension = u32::try_from(embedding.len()).map_err(|error| {
        voice_matching_error(
            "speaker_embedding_dimension_invalid",
            "Speaker embedding dimension is too large to persist.",
            Some(error.to_string()),
        )
    })?;
    let embedding_json = serde_json::to_string(&embedding).map_err(|error| {
        voice_matching_error(
            "speaker_embedding_serialization_failed",
            "Could not serialize the speaker embedding.",
            Some(error.to_string()),
        )
    })?;

    Ok(PreparedVoiceEmbedding {
        embedding_json,
        embedding_dimension,
        embedding_model_path: model_path.to_string_lossy().into_owned(),
    })
}

#[cfg(feature = "speaker-matching-sherpa")]
fn compute_voice_embedding_values(
    samples: &[f32],
    sample_rate: i32,
    model_path: &Path,
    sample_too_short_code: &str,
    sample_too_short_message: &str,
    compute_failed_message: &str,
) -> Result<Vec<f32>, AppError> {
    let config = SpeakerEmbeddingExtractorConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        num_threads: 1,
        debug: false,
        provider: Some("cpu".to_string()),
    };
    let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or_else(|| {
        voice_matching_error(
            "speaker_embedding_extractor_unavailable",
            "Speaker embedding extractor could not be constructed from this model.",
            None,
        )
    })?;
    let stream = extractor.create_stream().ok_or_else(|| {
        voice_matching_error(
            "speaker_embedding_stream_unavailable",
            "Speaker embedding extractor could not allocate an audio stream.",
            None,
        )
    })?;
    stream.accept_waveform(sample_rate, samples);
    stream.input_finished();
    if !extractor.is_ready(&stream) {
        return Err(voice_matching_error(
            sample_too_short_code,
            sample_too_short_message,
            None,
        ));
    }

    extractor.compute(&stream).ok_or_else(|| {
        voice_matching_error(
            "speaker_embedding_compute_failed",
            compute_failed_message,
            None,
        )
    })
}

#[cfg(feature = "speaker-matching-sherpa")]
fn diarize_speakers_with_sherpa(
    audio_path: &Path,
    segmentation_model_path: &Path,
    embedding_model_path: &Path,
) -> Result<VoiceDiarizationResult, AppError> {
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(segmentation_model_path.to_string_lossy().into_owned()),
            },
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(embedding_model_path.to_string_lossy().into_owned()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
        clustering: FastClusteringConfig::default(),
        min_duration_on: 0.3,
        min_duration_off: 0.5,
    };
    let diarizer = OfflineSpeakerDiarization::create(&config).ok_or_else(|| {
        voice_matching_error(
            "speaker_diarization_unavailable",
            "Speaker diarization could not be constructed from the configured models.",
            None,
        )
    })?;
    let wave = sherpa_onnx::Wave::read(&audio_path.to_string_lossy()).ok_or_else(|| {
        voice_matching_error(
            "speaker_diarization_audio_read_failed",
            "Could not read audio for speaker diarization.",
            None,
        )
    })?;
    let expected_sample_rate = diarizer.sample_rate();
    let samples = if wave.sample_rate() == expected_sample_rate {
        wave.samples().to_vec()
    } else {
        let resampler = LinearResampler::create(wave.sample_rate(), expected_sample_rate)
            .ok_or_else(|| {
                voice_matching_error(
                    "speaker_diarization_resampler_unavailable",
                    "Could not create a local resampler for speaker diarization.",
                    None,
                )
            })?;
        resampler.resample(wave.samples(), true)
    };
    if samples.is_empty() {
        return Err(voice_matching_error(
            "speaker_diarization_audio_empty",
            "Diarization audio must not be empty.",
            None,
        ));
    }

    let result = diarizer.process(&samples).ok_or_else(|| {
        voice_matching_error(
            "speaker_diarization_failed",
            "Could not diarize the selected recording.",
            None,
        )
    })?;
    let segments = result
        .sort_by_start_time()
        .into_iter()
        .map(|segment| {
            Ok(DiarizedSpeakerSegment {
                started_at_ms: seconds_to_milliseconds(segment.start)?,
                ended_at_ms: seconds_to_milliseconds(segment.end)?,
                speaker: segment.speaker,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let segment_count = u32::try_from(segments.len()).map_err(|error| {
        voice_matching_error(
            "speaker_diarization_segment_count_invalid",
            "Diarization produced too many speaker segments.",
            Some(error.to_string()),
        )
    })?;
    let speaker_count = u32::try_from(result.num_speakers().max(0)).map_err(|error| {
        voice_matching_error(
            "speaker_diarization_speaker_count_invalid",
            "Diarization produced an invalid speaker count.",
            Some(error.to_string()),
        )
    })?;

    Ok(VoiceDiarizationResult {
        speaker_count,
        segment_count,
        segments,
    })
}

#[cfg(feature = "speaker-matching-sherpa")]
fn match_diarized_speakers_with_sherpa(
    audio_path: &Path,
    embedding_model_path: &Path,
    enrolled_embedding: &[f32],
    diarization: &VoiceDiarizationResult,
    threshold: f32,
) -> Result<Vec<DiarizedSpeakerMatch>, AppError> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err(voice_matching_error(
            "speaker_match_threshold_invalid",
            "Speaker match threshold must be between 0.0 and 1.0.",
            Some(format!("threshold={threshold}")),
        ));
    }

    let wave = sherpa_onnx::Wave::read(&audio_path.to_string_lossy()).ok_or_else(|| {
        voice_matching_error(
            "speaker_diarization_audio_read_failed",
            "Could not read audio for speaker matching.",
            None,
        )
    })?;
    let speaker_samples = collect_diarized_speaker_samples(
        wave.samples(),
        wave.sample_rate(),
        &diarization.segments,
    )?;
    speaker_samples
        .into_iter()
        .map(|(speaker, samples)| {
            let speaker_embedding = compute_voice_embedding_values(
                &samples,
                wave.sample_rate(),
                embedding_model_path,
                "speaker_diarization_speaker_audio_too_short",
                "A diarized speaker segment is too short to compare with the local voice profile.",
                "Could not compute a speaker embedding from a diarized speaker segment.",
            )?;
            let match_result =
                compare_voice_embeddings(enrolled_embedding, &speaker_embedding, threshold)?;
            Ok(DiarizedSpeakerMatch {
                speaker,
                is_match: match_result.is_match,
                similarity_score: match_result.similarity_score,
                threshold: match_result.threshold,
            })
        })
        .collect()
}

#[cfg(feature = "speaker-matching-sherpa")]
fn collect_diarized_speaker_samples(
    samples: &[f32],
    sample_rate: i32,
    segments: &[DiarizedSpeakerSegment],
) -> Result<BTreeMap<i32, Vec<f32>>, AppError> {
    if sample_rate <= 0 {
        return Err(voice_matching_error(
            "speaker_diarization_sample_rate_invalid",
            "Diarization audio sample rate must be positive.",
            Some(format!("sample_rate={sample_rate}")),
        ));
    }

    let mut speaker_samples = BTreeMap::<i32, Vec<f32>>::new();
    for segment in segments {
        let start_index =
            milliseconds_to_sample_index(segment.started_at_ms, sample_rate, samples.len());
        let end_index =
            milliseconds_to_sample_index(segment.ended_at_ms, sample_rate, samples.len());
        if end_index > start_index {
            speaker_samples
                .entry(segment.speaker)
                .or_default()
                .extend_from_slice(&samples[start_index..end_index]);
        }
    }
    Ok(speaker_samples)
}

#[cfg(not(feature = "speaker-matching-sherpa"))]
fn prepare_voice_embedding_with_sherpa(
    _sample_audio_path: &Path,
    _model_path: &Path,
) -> Result<PreparedVoiceEmbedding, AppError> {
    Err(voice_matching_error(
        "speaker_matching_dependency_disabled",
        "Speaker matching is not enabled in this build.",
        None,
    ))
}

#[cfg(not(feature = "speaker-matching-sherpa"))]
fn diarize_speakers_with_sherpa(
    _audio_path: &Path,
    _segmentation_model_path: &Path,
    _embedding_model_path: &Path,
) -> Result<VoiceDiarizationResult, AppError> {
    Err(voice_matching_error(
        "speaker_diarization_dependency_disabled",
        "Speaker diarization is not enabled in this build.",
        None,
    ))
}

#[cfg(not(feature = "speaker-matching-sherpa"))]
fn match_diarized_speakers_with_sherpa(
    _audio_path: &Path,
    _embedding_model_path: &Path,
    _enrolled_embedding: &[f32],
    _diarization: &VoiceDiarizationResult,
    _threshold: f32,
) -> Result<Vec<DiarizedSpeakerMatch>, AppError> {
    Err(voice_matching_error(
        "speaker_diarization_matching_dependency_disabled",
        "Speaker diarization matching is not enabled in this build.",
        None,
    ))
}

fn validate_voice_matching_file_path(path: &Path, code: &str, label: &str) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(voice_matching_error(
            code,
            &format!("{label} must be absolute."),
            Some(format!("path={}", path.display())),
        ));
    }
    if !path.is_file() {
        return Err(voice_matching_error(
            code,
            &format!("{label} file was not found."),
            Some(format!("path={}", path.display())),
        ));
    }
    Ok(())
}

#[cfg(any(test, feature = "speaker-matching-sherpa"))]
fn seconds_to_milliseconds(seconds: f32) -> Result<u64, AppError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(voice_matching_error(
            "speaker_diarization_segment_timestamp_invalid",
            "Diarization produced an invalid speaker segment timestamp.",
            Some(format!("seconds={seconds}")),
        ));
    }
    if seconds > MAX_DIARIZATION_TIMESTAMP_SECONDS {
        return Err(voice_matching_error(
            "speaker_diarization_segment_timestamp_out_of_range",
            "Diarization produced an unreasonably large speaker segment timestamp.",
            Some(format!("seconds={seconds}")),
        ));
    }
    Ok((seconds * 1_000.0).round() as u64)
}

#[cfg(feature = "speaker-matching-sherpa")]
fn milliseconds_to_sample_index(milliseconds: u64, sample_rate: i32, sample_count: usize) -> usize {
    let index =
        (u128::from(milliseconds) * u128::try_from(sample_rate).unwrap_or_default()) / 1_000;
    usize::try_from(index)
        .unwrap_or(usize::MAX)
        .min(sample_count)
}

fn voice_matching_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_not_configured_without_model_path() {
        let status = voice_matcher_status(&ResonanceSettings::default());

        assert!(!status.model_configured);
        assert_eq!(status.model_path, None);
        assert!(!status.extractor_ready);
        assert_eq!(status.embedding_dimension, None);
    }

    #[test]
    fn status_rejects_relative_model_path() {
        let settings = ResonanceSettings {
            speaker_embedding_model_path: Some("models/speaker.onnx".to_string()),
            ..ResonanceSettings::default()
        };

        let status = voice_matcher_status(&settings);

        assert!(status.model_configured);
        assert_eq!(status.model_path, Some("models/speaker.onnx".to_string()));
        assert!(!status.extractor_ready);
        assert_eq!(
            status.message,
            "Speaker embedding model path must be absolute."
        );
    }

    #[test]
    fn status_reports_missing_absolute_model_path() {
        let settings = ResonanceSettings {
            speaker_embedding_model_path: Some("/tmp/resonance-missing-speaker.onnx".to_string()),
            ..ResonanceSettings::default()
        };

        let status = voice_matcher_status(&settings);

        assert!(status.model_configured);
        assert_eq!(
            status.model_path,
            Some("/tmp/resonance-missing-speaker.onnx".to_string())
        );
        assert!(!status.extractor_ready);
        assert_eq!(
            status.message,
            "Speaker embedding model file was not found."
        );
    }

    #[test]
    fn compare_voice_embeddings_scores_cosine_similarity_against_threshold() {
        let result = compare_voice_embeddings(&[1.0, 0.0, 0.0], &[0.9, 0.1, 0.0], 0.8)
            .expect("same-dimension embeddings can be compared");

        assert!(result.is_match);
        assert!(result.similarity_score >= 0.99);
        assert_eq!(result.threshold, 0.8);
    }

    #[test]
    fn compare_voice_embeddings_rejects_dimension_mismatch() {
        let error = compare_voice_embeddings(&[1.0, 0.0], &[1.0], 0.8)
            .expect_err("embedding dimensions must match");

        assert_eq!(error.code, "speaker_embedding_dimension_mismatch");
    }

    #[test]
    fn diarization_status_is_not_configured_without_segmentation_model_path() {
        let status = voice_diarization_status(&ResonanceSettings::default());

        assert!(!status.model_configured);
        assert!(!status.diarization_ready);
        assert_eq!(status.model_path, None);
    }

    #[test]
    fn diarization_status_rejects_relative_segmentation_model_path() {
        let settings = ResonanceSettings {
            speaker_segmentation_model_path: Some("models/segmentation.onnx".to_string()),
            ..ResonanceSettings::default()
        };

        let status = voice_diarization_status(&settings);

        assert!(status.model_configured);
        assert!(!status.diarization_ready);
        assert_eq!(
            status.message,
            "Speaker segmentation model path must be absolute."
        );
    }

    #[test]
    fn diarize_speakers_rejects_relative_segmentation_model_path() {
        let error = diarize_speakers(
            Path::new("/tmp/resonance-imported-recording.wav"),
            Path::new("models/segmentation.onnx"),
            Path::new("/tmp/resonance-speaker.onnx"),
        )
        .expect_err("relative segmentation path rejected");

        assert_eq!(error.code, "invalid_speaker_segmentation_model_path");
    }

    #[cfg(not(feature = "speaker-matching-sherpa"))]
    #[test]
    fn diarize_speakers_default_build_reports_disabled_dependency() {
        let temp_dir = std::env::temp_dir();
        let audio_path = temp_dir.join("resonance-diarization-audio.wav");
        let segmentation_path = temp_dir.join("resonance-diarization-segmentation.onnx");
        let embedding_path = temp_dir.join("resonance-diarization-embedding.onnx");
        std::fs::write(&audio_path, b"placeholder").expect("audio placeholder can be written");
        std::fs::write(&segmentation_path, b"placeholder")
            .expect("segmentation placeholder can be written");
        std::fs::write(&embedding_path, b"placeholder")
            .expect("embedding placeholder can be written");

        let error = diarize_speakers(&audio_path, &segmentation_path, &embedding_path)
            .expect_err("default build does not run diarization");

        assert_eq!(error.code, "speaker_diarization_dependency_disabled");

        std::fs::remove_file(audio_path).expect("audio placeholder can be removed");
        std::fs::remove_file(segmentation_path).expect("segmentation placeholder can be removed");
        std::fs::remove_file(embedding_path).expect("embedding placeholder can be removed");
    }

    #[test]
    fn diarization_timestamp_conversion_rejects_unreasonable_values() {
        let error = seconds_to_milliseconds(f32::MAX)
            .expect_err("corrupt oversized timestamps are rejected");

        assert_eq!(
            error.code,
            "speaker_diarization_segment_timestamp_out_of_range"
        );
    }

    #[test]
    fn matched_diarized_speaker_windows_include_only_matching_speaker_turns() {
        let diarization = VoiceDiarizationResult {
            speaker_count: 2,
            segment_count: 3,
            segments: vec![
                DiarizedSpeakerSegment {
                    started_at_ms: 1_000,
                    ended_at_ms: 2_000,
                    speaker: 0,
                },
                DiarizedSpeakerSegment {
                    started_at_ms: 2_200,
                    ended_at_ms: 3_000,
                    speaker: 1,
                },
                DiarizedSpeakerSegment {
                    started_at_ms: 3_200,
                    ended_at_ms: 4_000,
                    speaker: 0,
                },
            ],
        };
        let speaker_matches = vec![
            DiarizedSpeakerMatch {
                speaker: 0,
                is_match: true,
                similarity_score: 0.86,
                threshold: 0.75,
            },
            DiarizedSpeakerMatch {
                speaker: 1,
                is_match: false,
                similarity_score: 0.42,
                threshold: 0.75,
            },
        ];

        let result = match_diarized_speaker_windows(&diarization, speaker_matches, 0.75)
            .expect("windows map");

        assert_eq!(result.matched_window_count, 2);
        assert_eq!(
            result
                .matched_windows
                .iter()
                .map(|window| (window.started_at_ms, window.ended_at_ms, window.speaker))
                .collect::<Vec<_>>(),
            vec![(1_000, 2_000, 0), (3_200, 4_000, 0)]
        );
    }

    #[test]
    fn matched_diarized_speaker_windows_can_be_evaluated_with_lower_thresholds() {
        let diarization = VoiceDiarizationResult {
            speaker_count: 1,
            segment_count: 1,
            segments: vec![DiarizedSpeakerSegment {
                started_at_ms: 1_000,
                ended_at_ms: 2_000,
                speaker: 0,
            }],
        };
        let speaker_matches = vec![DiarizedSpeakerMatch {
            speaker: 0,
            is_match: false,
            similarity_score: 0.80,
            threshold: 0.85,
        }];

        let result = match_diarized_speaker_windows(&diarization, speaker_matches, 0.75)
            .expect("windows map");

        assert_eq!(result.matched_window_count, 1);
        assert_eq!(result.matched_windows[0].speaker, 0);
    }
}
