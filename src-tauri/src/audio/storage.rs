use std::path::{Path, PathBuf};

use crate::domain::AppError;

use super::audio_error;

const RECORDINGS_DIR_NAME: &str = "recordings";
const SYSTEM_AUDIO_EXTENSION: &str = "m4a";
const WAV_EXTENSION: &str = "wav";
const MAX_MEETING_ID_FILENAME_LEN: usize = 128;

pub fn validate_recording_file_stem(meeting_id: &str) -> Result<(), AppError> {
    if meeting_id.is_empty() {
        return Err(audio_error(
            "invalid_meeting_id",
            "Meeting id cannot be empty.",
            None,
        ));
    }

    if meeting_id.len() > MAX_MEETING_ID_FILENAME_LEN {
        return Err(audio_error(
            "invalid_meeting_id",
            "Meeting id is too long to use as a recording filename.",
            Some(format!(
                "length={}, max={MAX_MEETING_ID_FILENAME_LEN}",
                meeting_id.len()
            )),
        ));
    }

    if meeting_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        Ok(())
    } else {
        Err(audio_error(
            "invalid_meeting_id",
            "Meeting id can only contain ASCII letters, numbers, dashes, and underscores.",
            Some(format!("meeting_id={meeting_id}")),
        ))
    }
}

pub fn recordings_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(RECORDINGS_DIR_NAME)
}

pub fn safe_wav_path(app_data_dir: &Path, meeting_id: &str) -> Result<PathBuf, AppError> {
    validate_recording_file_stem(meeting_id)?;
    let directory = ensure_recordings_dir(app_data_dir)?;

    Ok(directory.join(format!("{meeting_id}.{WAV_EXTENSION}")))
}

pub fn safe_system_audio_path(app_data_dir: &Path, meeting_id: &str) -> Result<PathBuf, AppError> {
    validate_recording_file_stem(meeting_id)?;
    let directory = ensure_recordings_dir(app_data_dir)?;

    Ok(directory.join(format!("{meeting_id}.system.{SYSTEM_AUDIO_EXTENSION}")))
}

fn ensure_recordings_dir(app_data_dir: &Path) -> Result<PathBuf, AppError> {
    let directory = recordings_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| {
        audio_error(
            "recording_directory_error",
            "Could not create the recordings directory.",
            Some(error.to_string()),
        )
    })?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_id_filename_validation_allows_safe_ascii_stems() {
        assert!(validate_recording_file_stem("meeting-123_OK").is_ok());
    }

    #[test]
    fn meeting_id_filename_validation_rejects_path_traversal_and_separators() {
        for id in [
            "",
            "../meeting",
            "nested/meeting",
            "nested\\meeting",
            "meeting.wav",
        ] {
            let error = validate_recording_file_stem(id).expect_err("unsafe id is rejected");
            assert_eq!(error.code, "invalid_meeting_id");
        }
    }

    #[test]
    fn safe_wav_path_stays_under_recordings_directory() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/audio-storage");
        let path = safe_wav_path(&base, "meeting-safe").expect("path can be created");

        assert_eq!(path, base.join("recordings").join("meeting-safe.wav"));
    }

    #[test]
    fn safe_system_audio_path_uses_separate_channel_suffix() {
        let base = std::env::current_dir()
            .expect("current dir exists")
            .join("target/test-data/audio-storage-system");
        let path = safe_system_audio_path(&base, "meeting-safe").expect("path can be created");

        assert_eq!(
            path,
            base.join("recordings").join("meeting-safe.system.m4a")
        );
    }
}
