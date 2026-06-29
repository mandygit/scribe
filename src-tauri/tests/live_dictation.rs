//! Live dictation transcribe check: real audio clip -> whisper -> one line of
//! insert-ready text via the dictation `transcribe_clip` path.
//!
//! Ignored by default because it needs whisper-cli and a whisper model. Run it
//! with:
//!
//! ```sh
//! RESONANCE_LIVE_WAV=/abs/dictation.wav \
//! RESONANCE_WHISPER_MODEL=/abs/ggml-small.bin \
//! cargo test --test live_dictation -- --ignored --nocapture
//! ```

use std::path::Path;

use resonance_lib::dictation::transcribe_clip;
use resonance_lib::domain::ResonanceSettings;
use resonance_lib::transcription::WhisperShellTranscriber;

#[test]
#[ignore = "requires whisper-cli and a whisper model"]
fn live_clip_to_text() {
    let wav = std::env::var("RESONANCE_LIVE_WAV").expect("set RESONANCE_LIVE_WAV to a wav clip");
    let whisper_model = std::env::var("RESONANCE_WHISPER_MODEL")
        .expect("set RESONANCE_WHISPER_MODEL to a ggml model");

    let mut settings = ResonanceSettings::default();
    settings.transcriber_model_path = Some(whisper_model);
    let transcriber = WhisperShellTranscriber::from_settings(&settings).expect("build transcriber");

    let text = transcribe_clip(&transcriber, Path::new(&wav)).expect("dictation transcription");

    println!("\n===== DICTATION TEXT =====\n{text}\n==========================");
    assert!(!text.trim().is_empty(), "dictation produced no text");
    assert!(!text.contains('\n'), "dictation text should be a single line");
}
