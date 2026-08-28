//! System-wide dictation: record a push-to-talk clip, transcribe it, and clean
//! up the text with the on-device Apple Intelligence polish sidecar. Hotkey and
//! injection land in later slices.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::domain::AppError;

pub mod ax;
pub mod capture;
pub mod hotkey;
pub mod inject;
pub mod paste_target;
pub mod selection;

pub use capture::{
    new_dictation_session_id, new_dictation_wav_path, session_stats, transcribe_clip,
    DictationRecorder,
};
pub use hotkey::{DictationHotkey, HotkeyAction};
pub use inject::{
    capture_frontmost_app, copy_to_clipboard, inject_text, probe_accessibility, reactivate,
    spawn_frontmost_app_tracker, InjectOutcome, TargetAppPid,
};
pub use selection::{polish_selection, SelectionPolishOutcome};

fn polish_helper_unavailable() -> AppError {
    AppError {
        code: "dictation_polish_helper_unavailable".to_string(),
        message: "The dictation polish helper was not built for this platform.".to_string(),
        details: None,
    }
}

/// Resolves the bundled dictation-polish Swift sidecar, preferring the dev build
/// under `binaries/` and falling back to the path next to the app executable.
pub fn polish_helper_path() -> Result<PathBuf, AppError> {
    let helper_name =
        option_env!("SCRIBE_DICTATION_POLISH_HELPER_NAME").ok_or_else(polish_helper_unavailable)?;
    let target = option_env!("SCRIBE_DICTATION_POLISH_HELPER_TARGET")
        .ok_or_else(polish_helper_unavailable)?;

    let development_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("{helper_name}-{target}"));
    if development_path.is_file() {
        return Ok(development_path);
    }

    let executable_path = std::env::current_exe().map_err(|error| AppError {
        code: "dictation_polish_helper_unavailable".to_string(),
        message: "Could not locate the app executable to resolve the dictation polish helper."
            .to_string(),
        details: Some(error.to_string()),
    })?;
    let bundled_path = executable_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(helper_name);
    if bundled_path.is_file() {
        Ok(bundled_path)
    } else {
        Err(AppError {
            code: "dictation_polish_helper_unavailable".to_string(),
            message: "The dictation polish helper is not bundled for this platform.".to_string(),
            details: Some(format!(
                "checked={}, {}",
                development_path.display(),
                bundled_path.display()
            )),
        })
    }
}

/// Cleans dictated text with Apple Intelligence. Returns the raw text unchanged
/// if it is blank. Errors carry a stable `code` so callers can fall back to the
/// raw transcript when Apple Intelligence is unavailable.
pub fn polish_text(raw: &str) -> Result<String, AppError> {
    if raw.trim().is_empty() {
        return Ok(raw.trim().to_string());
    }
    let helper_path = polish_helper_path()?;
    run_polish_helper(&helper_path, raw)
}

fn run_polish_helper(helper_path: &Path, raw: &str) -> Result<String, AppError> {
    let mut child = Command::new(helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AppError {
            code: "dictation_polish_start_failed".to_string(),
            message: "Could not start the dictation polish helper.".to_string(),
            details: Some(error.to_string()),
        })?;

    child
        .stdin
        .take()
        .ok_or_else(|| AppError {
            code: "dictation_polish_stdin_failed".to_string(),
            message: "Could not write to the dictation polish helper.".to_string(),
            details: None,
        })?
        .write_all(raw.as_bytes())
        .map_err(|error| AppError {
            code: "dictation_polish_stdin_failed".to_string(),
            message: "Could not send text to the dictation polish helper.".to_string(),
            details: Some(error.to_string()),
        })?;

    let output = child.wait_with_output().map_err(|error| AppError {
        code: "dictation_polish_failed".to_string(),
        message: "The dictation polish helper did not finish.".to_string(),
        details: Some(error.to_string()),
    })?;

    if output.status.success() {
        let cleaned = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if cleaned.is_empty() || !is_plausible_cleanup(raw, &cleaned) {
            // The model answered/acted on the text instead of cleaning it up
            // (or produced nothing) — fall back to the raw transcript so
            // dictation always pastes what was actually said.
            return Ok(raw.trim().to_string());
        }
        return Ok(cleaned);
    }

    // Exit code 2 means Apple Intelligence is not available on this machine.
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let code = if output.status.code() == Some(2) {
        "apple_intelligence_unavailable"
    } else {
        "dictation_polish_failed"
    };
    Err(AppError {
        code: code.to_string(),
        message: "Apple Intelligence could not polish the dictation.".to_string(),
        details: if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        },
    })
}

/// True if `cleaned` looks like a cleanup of `raw` rather than a fabricated
/// reply. The polish instructions forbid the model from adding new ideas, so
/// a real cleanup only reuses `raw`'s words (minus filler) plus incidental
/// punctuation/number tokens; a model that answered the dictation instead
/// introduces a lot of vocabulary that was never spoken.
fn is_plausible_cleanup(raw: &str, cleaned: &str) -> bool {
    const MAX_NEW_WORD_RATIO: f64 = 0.25;

    let cleaned_words = normalized_words(cleaned);
    if cleaned_words.is_empty() {
        return true;
    }
    let raw_words = normalized_words(raw);

    let new_words = cleaned_words
        .iter()
        .filter(|word| !raw_words.contains(*word))
        .count();

    (new_words as f64) / (cleaned_words.len() as f64) <= MAX_NEW_WORD_RATIO
}

/// Lowercased, punctuation-stripped words, deduplicated for set comparison.
fn normalized_words(text: &str) -> std::collections::HashSet<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_input_is_returned_unchanged_without_spawning() {
        assert_eq!(polish_text("   \n").expect("blank ok"), "");
    }

    #[test]
    fn helper_path_is_configured_on_macos() {
        if cfg!(target_os = "macos") {
            let path = polish_helper_path().expect("macOS build resolves a helper path");
            assert!(path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.contains("scribe-dictation-polish"))
                .unwrap_or(false));
        }
    }

    #[test]
    fn cleanup_that_keeps_the_speakers_words_passes() {
        let raw = "so um first we need to uh fix the login bug";
        let cleaned = "First, we need to fix the login bug.";
        assert!(is_plausible_cleanup(raw, cleaned));
    }

    #[test]
    fn answer_instead_of_cleanup_is_rejected() {
        let raw = "what's the capital of france";
        let answer = "The capital of France is Paris.";
        assert!(!is_plausible_cleanup(raw, answer));
    }

    #[test]
    fn fabricated_reply_to_a_request_is_rejected() {
        let raw = "hey can you draft an email to john about the meeting";
        let answer =
            "Dear John, following up on our conversation, I wanted to confirm the meeting time.";
        assert!(!is_plausible_cleanup(raw, answer));
    }
}
