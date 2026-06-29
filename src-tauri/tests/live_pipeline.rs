//! Live end-to-end pipeline check: real audio -> whisper transcript -> Gemma notes.
//!
//! Ignored by default because it needs whisper-cli, a whisper model, and a
//! running LM Studio install. Run it with:
//!
//! ```sh
//! RESONANCE_LIVE_WAV=/abs/meeting.wav \
//! RESONANCE_WHISPER_MODEL=/abs/ggml-small.bin \
//! cargo test --test live_pipeline -- --ignored --nocapture
//! ```

use std::path::Path;

use resonance_lib::analysis::{AnalysisTranscriptSegment, MeetingSummarizer, TranscriptSpeakerRole};
use resonance_lib::domain::ResonanceSettings;
use resonance_lib::summarizer::{
    LmStudioClient, LmStudioLifecycle, LmStudioSummarizer, DEFAULT_SUMMARIZER_MODEL,
};
use resonance_lib::transcription::{Transcriber, WhisperShellTranscriber};

#[test]
#[ignore = "requires whisper-cli, a whisper model, and a running LM Studio install"]
fn live_audio_to_notes() {
    let wav = std::env::var("RESONANCE_LIVE_WAV").expect("set RESONANCE_LIVE_WAV to a 16 kHz wav");
    let whisper_model =
        std::env::var("RESONANCE_WHISPER_MODEL").expect("set RESONANCE_WHISPER_MODEL to a ggml model");
    let summarizer_model = std::env::var("RESONANCE_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| DEFAULT_SUMMARIZER_MODEL.to_string());

    // 1. Real transcription via the same whisper path the app uses.
    let mut settings = ResonanceSettings::default();
    settings.transcriber_model_path = Some(whisper_model);
    let transcriber = WhisperShellTranscriber::from_settings(&settings).expect("build transcriber");
    let output = transcriber
        .transcribe(Path::new(&wav))
        .expect("whisper transcription");
    assert!(!output.segments.is_empty(), "whisper produced no segments");

    println!("\n===== TRANSCRIPT ({} segments) =====", output.segments.len());
    for segment in &output.segments {
        println!("{}", segment.text.trim());
    }

    let segments: Vec<AnalysisTranscriptSegment> = output
        .segments
        .iter()
        .map(|segment| AnalysisTranscriptSegment {
            sequence_number: segment.sequence_number,
            speaker_label: segment.speaker_label.clone(),
            speaker_role: TranscriptSpeakerRole::User,
            text: segment.text.clone(),
            started_at_ms: segment.started_at_ms,
            ended_at_ms: segment.ended_at_ms,
        })
        .collect();

    // 2. Real summarization via the LM Studio lifecycle (start -> load -> unload).
    let lifecycle = LmStudioLifecycle::resolve(None).expect("resolve lms");
    lifecycle.ensure_running().expect("start LM Studio server");
    lifecycle.load(&summarizer_model).expect("load summarizer model");
    let summarizer = LmStudioSummarizer::new(LmStudioClient::default(), summarizer_model);
    let result = summarizer.summarize(&segments, false);
    lifecycle.unload_all();
    let notes = result.expect("generate notes");

    println!("\n===== NOTES =====");
    println!("TL;DR: {}", notes.executive_summary);
    println!("Decisions: {:?}", notes.decisions);
    println!("Open questions: {:?}", notes.open_questions);
    println!("Action items:");
    for item in &notes.action_items {
        println!(
            "  [{}] {} (due {})",
            item.owner.as_deref().unwrap_or("-"),
            item.task,
            item.due.as_deref().unwrap_or("-")
        );
    }

    assert!(!notes.executive_summary.trim().is_empty(), "notes had an empty TL;DR");
}
