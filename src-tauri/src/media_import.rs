use std::path::{Path, PathBuf};

use crate::domain::AppError;

const HOMEBREW_FFMPEG_PATHS: &[&str] = &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"];

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
