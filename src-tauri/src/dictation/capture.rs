//! Push-to-talk dictation capture: record the microphone to a temporary WAV
//! between the hotkey press that starts dictation and the one that ends it, then
//! transcribe the clip with the same whisper backend the meeting recorder uses.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::audio::capture::{current_time_ms, ActiveCapture, AudioCaptureBackend};
use crate::domain::AppError;
use crate::transcription::{Transcriber, TranscriptionOutput};

/// Monotonic suffix so back-to-back dictations never collide on a temp path even
/// within the same millisecond.
static DICTATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Monotonic suffix so back-to-back dictation session summary rows never collide
/// on id even within the same millisecond.
static DICTATION_SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Builds a unique temporary WAV path for a single dictation clip. The clip is
/// short-lived: it is transcribed and then deleted, so the system temp dir is the
/// right home for it.
pub fn new_dictation_wav_path() -> Result<PathBuf, AppError> {
    let sequence = DICTATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = current_time_ms()?;
    Ok(std::env::temp_dir().join(format!("scribe-dictation-{stamp}-{sequence}.wav")))
}

/// Builds a unique id for a persisted dictation session summary row.
pub fn new_dictation_session_id() -> Result<String, AppError> {
    let sequence = DICTATION_SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = current_time_ms()?;
    Ok(format!("dictation-session-{stamp}-{sequence}"))
}

/// Holds an in-flight push-to-talk microphone capture. Only one dictation can be
/// recording at a time, so the hotkey toggles between [`start`](Self::start) and
/// [`finish`](Self::finish).
pub struct DictationRecorder<B: AudioCaptureBackend> {
    backend: B,
    active: Option<ActiveCapture>,
}

impl<B: AudioCaptureBackend> DictationRecorder<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            active: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// Starts recording the microphone to `file_path`. Errors if a dictation is
    /// already in flight so a double hotkey press can't start two captures.
    pub fn start(
        &mut self,
        file_path: PathBuf,
        device_id: Option<String>,
    ) -> Result<(), AppError> {
        if self.active.is_some() {
            return Err(dictation_already_recording());
        }
        let capture = self.backend.start_recording(file_path, device_id)?;
        self.active = Some(capture);
        Ok(())
    }

    /// Stops the in-flight capture and returns the path to the recorded WAV
    /// along with when the capture started, so callers can measure session
    /// duration. Errors if no dictation is in flight.
    pub fn finish(&mut self) -> Result<(PathBuf, u64), AppError> {
        let capture = self.active.take().ok_or_else(dictation_not_recording)?;
        let ActiveCapture {
            file_path,
            started_at_ms,
            handle,
            ..
        } = capture;
        handle.stop()?;
        Ok((file_path, started_at_ms))
    }
}

/// Computes word count and words-per-minute for a finished dictation, given the
/// final (possibly polished) text and how long the capture ran.
pub fn session_stats(text: &str, duration_ms: u64) -> (u32, f64) {
    let word_count = text.split_whitespace().count() as u32;
    let minutes = duration_ms as f64 / 60_000.0;
    let words_per_minute = if minutes > 0.0 {
        word_count as f64 / minutes
    } else {
        0.0
    };
    (word_count, words_per_minute)
}

/// Transcribes a recorded dictation clip and flattens it into one insert-ready
/// line of text.
pub fn transcribe_clip<T: Transcriber>(
    transcriber: &T,
    wav_path: &Path,
) -> Result<String, AppError> {
    let output = transcriber.transcribe(wav_path)?;
    Ok(transcript_to_text(&output))
}

/// Collapses whisper's per-segment output into a single dictation string,
/// trimming each segment and dropping both blanks and whisper's non-speech
/// annotations so silence never gets typed into the focused app.
pub fn transcript_to_text(output: &TranscriptionOutput) -> String {
    output
        .segments
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty() && !is_non_speech_marker(text))
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when a segment is one of whisper's non-speech annotations rather than
/// dictated words. Whisper wraps these wholly in brackets or parentheses, e.g.
/// `[BLANK_AUDIO]`, `[ Silence ]`, `[Music]`, `(wind blowing)`.
fn is_non_speech_marker(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'))
}

fn dictation_already_recording() -> AppError {
    AppError {
        code: "dictation_already_recording".to_string(),
        message: "A dictation is already recording.".to_string(),
        details: None,
    }
}

fn dictation_not_recording() -> AppError {
    AppError {
        code: "dictation_not_recording".to_string(),
        message: "No dictation is currently recording.".to_string(),
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::audio::capture::{CaptureSessionHandle, CaptureSummary};
    use crate::audio::domain::AudioDevice;
    use crate::transcription::{TranscriptSegment, TranscriptionOutput};

    struct StubBackend;

    struct StubSession;

    impl CaptureSessionHandle for StubSession {
        fn stop(self: Box<Self>) -> Result<CaptureSummary, AppError> {
            Ok(CaptureSummary {
                sample_count: 16_000,
                dropped_sample_count: 0,
                stream_error: None,
            })
        }
    }

    impl AudioCaptureBackend for StubBackend {
        fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
            Ok(Vec::new())
        }

        fn start_recording(
            &self,
            file_path: PathBuf,
            _device_id: Option<String>,
        ) -> Result<ActiveCapture, AppError> {
            Ok(ActiveCapture {
                file_path,
                sample_rate_hz: 16_000,
                started_at_ms: 0,
                handle: Box::new(StubSession),
            })
        }
    }

    fn segment(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            sequence_number: 0,
            speaker_label: None,
            text: text.to_string(),
            started_at_ms: 0,
            ended_at_ms: 0,
        }
    }

    #[test]
    fn start_then_finish_returns_the_capture_path_and_start_time() {
        let mut recorder = DictationRecorder::new(StubBackend);
        assert!(!recorder.is_recording());

        recorder
            .start(PathBuf::from("/tmp/scribe-dictation-test.wav"), None)
            .expect("start succeeds");
        assert!(recorder.is_recording());

        let (path, started_at_ms) = recorder.finish().expect("finish succeeds");
        assert_eq!(path, PathBuf::from("/tmp/scribe-dictation-test.wav"));
        assert_eq!(started_at_ms, 0);
        assert!(!recorder.is_recording());
    }

    #[test]
    fn starting_twice_is_rejected() {
        let mut recorder = DictationRecorder::new(StubBackend);
        recorder
            .start(PathBuf::from("/tmp/a.wav"), None)
            .expect("first start succeeds");

        let error = recorder
            .start(PathBuf::from("/tmp/b.wav"), None)
            .expect_err("second start is rejected");
        assert_eq!(error.code, "dictation_already_recording");
    }

    #[test]
    fn finishing_without_a_capture_is_rejected() {
        let mut recorder = DictationRecorder::new(StubBackend);
        let error = recorder.finish().expect_err("finish without start is rejected");
        assert_eq!(error.code, "dictation_not_recording");
    }

    #[test]
    fn transcript_text_joins_and_trims_segments() {
        let output = TranscriptionOutput {
            segments: vec![
                segment("  Hello there  "),
                segment("   "),
                segment("general kinobi"),
            ],
        };
        assert_eq!(transcript_to_text(&output), "Hello there general kinobi");
    }

    #[test]
    fn transcript_text_drops_whisper_non_speech_markers() {
        let output = TranscriptionOutput {
            segments: vec![
                segment("[BLANK_AUDIO]"),
                segment("Send the deck"),
                segment("[ Silence ]"),
                segment("(wind blowing)"),
            ],
        };
        assert_eq!(transcript_to_text(&output), "Send the deck");
    }

    #[test]
    fn transcript_text_is_empty_for_pure_silence() {
        let output = TranscriptionOutput {
            segments: vec![segment("[BLANK_AUDIO]")],
        };
        assert_eq!(transcript_to_text(&output), "");
    }

    #[test]
    fn distinct_temp_paths_do_not_collide() {
        let first = new_dictation_wav_path().expect("first path");
        let second = new_dictation_wav_path().expect("second path");
        assert_ne!(first, second);
    }

    #[test]
    fn distinct_session_ids_do_not_collide() {
        let first = new_dictation_session_id().expect("first id");
        let second = new_dictation_session_id().expect("second id");
        assert_ne!(first, second);
    }

    #[test]
    fn session_stats_counts_words_and_computes_wpm() {
        let (word_count, wpm) = session_stats("Send the deck to the team", 30_000);
        assert_eq!(word_count, 6);
        assert!((wpm - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn session_stats_handles_empty_text() {
        let (word_count, wpm) = session_stats("   ", 5_000);
        assert_eq!(word_count, 0);
        assert_eq!(wpm, 0.0);
    }

    #[test]
    fn session_stats_guards_zero_duration() {
        let (word_count, wpm) = session_stats("hello world", 0);
        assert_eq!(word_count, 2);
        assert_eq!(wpm, 0.0);
    }
}
