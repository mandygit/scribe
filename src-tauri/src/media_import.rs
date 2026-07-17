use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::domain::AppError;

const HOMEBREW_FFMPEG_PATHS: &[&str] = &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"];
const AFCONVERT_PATH: &str = "/usr/bin/afconvert";

/// Converts an audio file to mono 16-bit PCM WAV at the requested sample
/// rate. Prefers macOS's built-in `afconvert` (handles wav, m4a/aac, mp3,
/// alac, flac, aiff) so a packaged Scribe works without any external tools;
/// falls back to ffmpeg (settings/Homebrew/PATH) for formats CoreAudio cannot
/// decode, such as ogg. `error_code` becomes the `AppError` code so each
/// caller keeps its established error identity.
pub(crate) fn convert_to_mono_s16_wav(
    source_path: &Path,
    output_path: &Path,
    sample_rate_hz: u32,
    error_code: &str,
) -> Result<(), AppError> {
    let afconvert_failure = match run_afconvert(source_path, output_path, sample_rate_hz) {
        Ok(()) => return Ok(()),
        Err(failure) => failure,
    };

    let ffmpeg_path = resolve_ffmpeg_path(None).map_err(|error| {
        media_import_error(
            error_code,
            "Could not convert the audio file, and no ffmpeg fallback is available.",
            Some(format!(
                "path={}, afconvert: {afconvert_failure}, ffmpeg: {}",
                source_path.display(),
                error.details.unwrap_or(error.message)
            )),
        )
    })?;
    run_ffmpeg(
        &ffmpeg_path,
        source_path,
        output_path,
        sample_rate_hz,
        error_code,
        &afconvert_failure,
    )
}

fn run_afconvert(
    source_path: &Path,
    output_path: &Path,
    sample_rate_hz: u32,
) -> Result<(), String> {
    if !Path::new(AFCONVERT_PATH).is_file() {
        return Err("afconvert is not available".to_string());
    }

    let output = Command::new(AFCONVERT_PATH)
        .arg("-f")
        .arg("WAVE")
        .arg("-d")
        .arg(format!("LEI16@{sample_rate_hz}"))
        .arg("-c")
        .arg("1")
        .arg(source_path)
        .arg(output_path)
        .output()
        .map_err(|error| format!("could not start afconvert: {error}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "afconvert failed ({})",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn run_ffmpeg(
    ffmpeg_path: &Path,
    source_path: &Path,
    output_path: &Path,
    sample_rate_hz: u32,
    error_code: &str,
    afconvert_failure: &str,
) -> Result<(), AppError> {
    let output = Command::new(ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(source_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg(sample_rate_hz.to_string())
        .arg("-sample_fmt")
        .arg("s16")
        .arg("-f")
        .arg("wav")
        .arg(output_path)
        .output()
        .map_err(|error| {
            media_import_error(
                error_code,
                "Could not start ffmpeg to convert the audio file.",
                Some(error.to_string()),
            )
        })?;

    if !output.status.success() {
        return Err(media_import_error(
            error_code,
            "The audio file could not be converted for processing.",
            Some(format!(
                "path={}, afconvert: {afconvert_failure}, ffmpeg stderr: {}",
                source_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        ));
    }

    Ok(())
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

fn media_import_error(code: &str, message: &str, details: Option<String>) -> AppError {
    AppError {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}
