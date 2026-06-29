use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::domain::AppError;

const IMPORTED_MEDIA_DIR_NAME: &str = "imported-recordings";
const PRACTICE_MEDIA_DIR_NAME: &str = "practice-recordings";
const SUPPORTED_MEDIA_EXTENSIONS: &[&str] = &["m4a", "mov", "mp3", "mp4", "wav"];
const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["mov", "mp4", "webm"];
const HOMEBREW_FFMPEG_PATHS: &[&str] = &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"];

/// Extracts normalized mono WAV audio from a downloaded recording using ffmpeg.
pub fn extract_recording_audio(
    source_path: &Path,
    ffmpeg_bin_path: Option<&str>,
    app_data_dir: &Path,
    meeting_id: &str,
) -> Result<PathBuf, AppError> {
    let source_path = validate_media_source_path(source_path)?;
    let ffmpeg_path = resolve_ffmpeg_path(ffmpeg_bin_path)?;
    let output_path = safe_imported_audio_path(app_data_dir, meeting_id)?;

    let output = Command::new(&ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(&source_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-f")
        .arg("wav")
        .arg(&output_path)
        .output()
        .map_err(|error| {
            media_import_error(
                "media_extract_start_failed",
                "Could not start ffmpeg.",
                Some(error.to_string()),
            )
        })?;

    if !output.status.success() {
        return Err(media_import_error(
            "media_extract_failed",
            "ffmpeg could not extract audio from the selected recording.",
            None,
        ));
    }

    Ok(output_path)
}

pub fn copy_practice_video(
    source_path: &Path,
    app_data_dir: &Path,
    practice_id: &str,
) -> Result<PathBuf, AppError> {
    let source_path = validate_video_source_path(source_path)?;
    let extension = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(unsupported_video_error)?;
    let output_path = safe_practice_video_path(app_data_dir, practice_id, &extension)?;
    fs::copy(&source_path, &output_path).map_err(|error| {
        media_import_error(
            "practice_video_copy_failed",
            "Could not copy the practice video into Resonance app data.",
            Some(error.to_string()),
        )
    })?;
    Ok(output_path)
}

pub fn write_practice_video_bytes(
    bytes: &[u8],
    extension: &str,
    app_data_dir: &Path,
    practice_id: &str,
) -> Result<PathBuf, AppError> {
    let normalized_extension = normalize_supported_video_extension(extension)?;
    let output_path = safe_practice_video_path(app_data_dir, practice_id, normalized_extension)?;
    fs::write(&output_path, bytes).map_err(|error| {
        media_import_error(
            "practice_video_write_failed",
            "Could not save the recorded practice video.",
            Some(error.to_string()),
        )
    })?;
    Ok(output_path)
}

pub fn extract_practice_video_audio(
    source_path: &Path,
    ffmpeg_bin_path: Option<&str>,
    app_data_dir: &Path,
    practice_id: &str,
) -> Result<PathBuf, AppError> {
    validate_video_source_path(source_path)?;
    let ffmpeg_path = resolve_ffmpeg_path(ffmpeg_bin_path)?;
    let output_path = safe_practice_audio_path(app_data_dir, practice_id)?;

    let output = Command::new(&ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(source_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-f")
        .arg("wav")
        .arg(&output_path)
        .output()
        .map_err(|error| {
            media_import_error(
                "practice_audio_extract_start_failed",
                "Could not start ffmpeg for practice video audio extraction.",
                Some(error.to_string()),
            )
        })?;

    if !output.status.success() {
        return Err(media_import_error(
            "practice_audio_extract_failed",
            "ffmpeg could not extract audio from the selected practice video.",
            None,
        ));
    }

    Ok(output_path)
}

pub fn validate_media_source_path(path: &Path) -> Result<PathBuf, AppError> {
    validate_existing_file(path, "invalid_media_source_path", "Recording file path")?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(unsupported_media_error)?;

    if SUPPORTED_MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        Ok(path.to_path_buf())
    } else {
        Err(unsupported_media_error())
    }
}

pub fn validate_video_source_path(path: &Path) -> Result<PathBuf, AppError> {
    validate_existing_file(path, "invalid_practice_video_path", "Practice video path")?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(unsupported_video_error)?;

    if SUPPORTED_VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Ok(path.to_path_buf())
    } else {
        Err(unsupported_video_error())
    }
}

fn safe_imported_audio_path(app_data_dir: &Path, meeting_id: &str) -> Result<PathBuf, AppError> {
    crate::audio::storage::validate_recording_file_stem(meeting_id)?;
    let directory = app_data_dir.join(IMPORTED_MEDIA_DIR_NAME);
    std::fs::create_dir_all(&directory).map_err(|error| {
        media_import_error(
            "import_directory_error",
            "Could not create the imported recordings directory.",
            Some(error.to_string()),
        )
    })?;
    Ok(directory.join(format!("{meeting_id}.wav")))
}

fn safe_practice_video_path(
    app_data_dir: &Path,
    practice_id: &str,
    extension: &str,
) -> Result<PathBuf, AppError> {
    crate::audio::storage::validate_recording_file_stem(practice_id)?;
    let directory = app_data_dir.join(PRACTICE_MEDIA_DIR_NAME);
    fs::create_dir_all(&directory).map_err(|error| {
        media_import_error(
            "practice_directory_error",
            "Could not create the practice recordings directory.",
            Some(error.to_string()),
        )
    })?;
    Ok(directory.join(format!("{practice_id}.{extension}")))
}

fn safe_practice_audio_path(app_data_dir: &Path, practice_id: &str) -> Result<PathBuf, AppError> {
    crate::audio::storage::validate_recording_file_stem(practice_id)?;
    let directory = app_data_dir.join(PRACTICE_MEDIA_DIR_NAME);
    fs::create_dir_all(&directory).map_err(|error| {
        media_import_error(
            "practice_directory_error",
            "Could not create the practice recordings directory.",
            Some(error.to_string()),
        )
    })?;
    Ok(directory.join(format!("{practice_id}.audio.wav")))
}

fn normalize_supported_video_extension(extension: &str) -> Result<&str, AppError> {
    let trimmed = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    SUPPORTED_VIDEO_EXTENSIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == trimmed)
        .ok_or_else(unsupported_video_error)
}

pub(crate) fn resolve_ffmpeg_path(configured_path: Option<&str>) -> Result<PathBuf, AppError> {
    if let Some(path) = configured_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return validate_executable_path(
            Path::new(path),
            "invalid_ffmpeg_path",
            "ffmpeg binary path",
        );
    }

    for candidate in HOMEBREW_FFMPEG_PATHS {
        let path = Path::new(candidate);
        if path.exists() && path.is_file() {
            return validate_executable_path(path, "invalid_ffmpeg_path", "ffmpeg binary path");
        }
    }

    if let Some(path) = find_binary_on_path("ffmpeg") {
        return validate_executable_path(&path, "invalid_ffmpeg_path", "ffmpeg binary path");
    }

    Err(media_import_error(
        "ffmpeg_not_found",
        "Could not find ffmpeg. Install ffmpeg or paste an absolute ffmpeg binary path.",
        Some("Checked common Homebrew paths and PATH.".to_string()),
    ))
}

fn find_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_value = std::env::var_os("PATH")?;
    std::env::split_paths(&path_value)
        .map(|directory| directory.join(binary_name))
        .find(|candidate| candidate.exists() && candidate.is_file())
}

fn validate_executable_path(path: &Path, code: &str, label: &str) -> Result<PathBuf, AppError> {
    validate_existing_file(path, code, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|error| {
                media_import_error(
                    code,
                    &format!("Could not inspect {label}."),
                    Some(error.to_string()),
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(media_import_error(
                code,
                &format!("{label} is not executable."),
                Some(format!("path={}", path.display())),
            ));
        }
    }
    Ok(path.to_path_buf())
}

fn validate_existing_file(path: &Path, code: &str, label: &str) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(media_import_error(
            code,
            &format!("{label} must be an absolute path."),
            Some(format!("path={}", path.display())),
        ));
    }

    if !path.exists() || !path.is_file() {
        return Err(media_import_error(
            code,
            &format!("{label} must point to an existing file."),
            Some(format!("path={}", path.display())),
        ));
    }

    Ok(())
}

fn unsupported_media_error() -> AppError {
    media_import_error(
        "unsupported_media_type",
        "Recording import supports .mp4, .mov, .m4a, .mp3, and .wav files.",
        None,
    )
}

fn unsupported_video_error() -> AppError {
    media_import_error(
        "unsupported_practice_video_type",
        "Practice video import supports .mp4, .mov, and .webm files.",
        None,
    )
}

fn media_import_error(code: &str, message: &str, details: Option<String>) -> AppError {
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
    fn media_source_validation_rejects_relative_paths() {
        let error = validate_media_source_path(Path::new("meeting.mp4"))
            .expect_err("relative path rejected");

        assert_eq!(error.code, "invalid_media_source_path");
    }

    #[test]
    fn media_source_validation_rejects_unsupported_extensions() {
        let path = std::env::temp_dir().join("resonance-import-test.txt");
        std::fs::write(&path, b"not media").expect("test file can be written");

        let error = validate_media_source_path(&path).expect_err("unsupported extension rejected");

        assert_eq!(error.code, "unsupported_media_type");
        std::fs::remove_file(path).expect("test file can be removed");
    }
}
