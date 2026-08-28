use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use crate::domain::ScribeSettings;

static DETECTED_LOCAL_PATHS: OnceLock<DetectedLocalPaths> = OnceLock::new();

const WHISPER_BINARY_CANDIDATES: [&str; 2] = [
    "/opt/homebrew/bin/whisper-cli",
    "/usr/local/bin/whisper-cli",
];
const WHISPER_MODEL_QUERY: &str =
    "kMDItemFSName == \"ggml-*\"c && (kMDItemFSName == \"*.bin\"c || kMDItemFSName == \"*.gguf\"c)";
const SPEAKER_EMBEDDING_QUERY: &str =
    "kMDItemFSName == \"*.onnx\"c && (kMDItemFSName == \"*embedding*\"c || kMDItemFSName == \"*speaker*\"c)";
const SPEAKER_SEGMENTATION_QUERY: &str =
    "kMDItemFSName == \"*.onnx\"c && (kMDItemFSName == \"*segmentation*\"c || kMDItemFSName == \"*pyannote*\"c || kMDItemFSName == \"*speaker*\"c)";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectedLocalPaths {
    transcriber_bin_path: Option<String>,
    transcriber_model_path: Option<String>,
    speaker_embedding_model_path: Option<String>,
    speaker_segmentation_model_path: Option<String>,
}

enum ModelKind {
    Whisper,
    SpeakerEmbedding,
    SpeakerSegmentation,
}

pub fn hydrate_settings_with_local_defaults(mut settings: ScribeSettings) -> ScribeSettings {
    let detected = detect_local_paths();
    hydrate_settings_with_detected_paths(&mut settings, &detected);
    settings
}

fn hydrate_settings_with_detected_paths(
    settings: &mut ScribeSettings,
    detected: &DetectedLocalPaths,
) {
    if missing_path(&settings.transcriber_bin_path) {
        settings.transcriber_bin_path = detected.transcriber_bin_path.clone();
    }
    if missing_path(&settings.transcriber_model_path) {
        settings.transcriber_model_path = detected.transcriber_model_path.clone();
    }
    if missing_path(&settings.speaker_embedding_model_path) {
        settings.speaker_embedding_model_path = detected.speaker_embedding_model_path.clone();
    }
    if missing_path(&settings.speaker_segmentation_model_path) {
        settings.speaker_segmentation_model_path = detected.speaker_segmentation_model_path.clone();
    }
}

fn missing_path(path: &Option<String>) -> bool {
    path.as_deref().map(str::trim).map_or(true, str::is_empty)
}

fn detect_local_paths() -> DetectedLocalPaths {
    DETECTED_LOCAL_PATHS
        .get_or_init(|| DetectedLocalPaths {
            transcriber_bin_path: detect_whisper_binary_path(),
            transcriber_model_path: detect_model_path(ModelKind::Whisper),
            speaker_embedding_model_path: detect_model_path(ModelKind::SpeakerEmbedding),
            speaker_segmentation_model_path: detect_model_path(ModelKind::SpeakerSegmentation),
        })
        .clone()
}

/// Root of the resources that ship inside the app bundle: the source-tree
/// `src-tauri/resources` in development, `Scribe.app/Contents/Resources` when
/// running the packaged app. Assembled by `scripts/bundle-whisper-cli.sh` and
/// `scripts/fetch-whisper-model.sh`, so it may be absent in a fresh checkout.
pub(crate) fn bundled_resources_dir() -> Option<PathBuf> {
    let development_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    if development_dir.is_dir() {
        return Some(development_dir);
    }

    let executable_path = env::current_exe().ok()?;
    let resources_dir = executable_path.parent()?.parent()?.join("Resources");
    resources_dir.is_dir().then_some(resources_dir)
}

fn bundled_whisper_binary_path() -> Option<String> {
    let path = bundled_resources_dir()?.join("whisper").join("whisper-cli");
    is_file(&path).then(|| path.to_string_lossy().into_owned())
}

fn bundled_whisper_model_path() -> Option<String> {
    let models_dir = bundled_resources_dir()?.join("models");
    let mut candidates: Vec<PathBuf> = fs::read_dir(models_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_candidate_for_kind(path, &ModelKind::Whisper))
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|path| path.to_string_lossy().into_owned())
}

fn detect_whisper_binary_path() -> Option<String> {
    if let Some(bundled) = bundled_whisper_binary_path() {
        return Some(bundled);
    }

    for candidate in WHISPER_BINARY_CANDIDATES {
        let path = Path::new(candidate);
        if is_file(path) {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    find_binary_on_path("whisper-cli").map(|path| path.to_string_lossy().into_owned())
}

fn find_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_value = env::var_os("PATH")?;
    env::split_paths(&path_value)
        .map(|directory| directory.join(binary_name))
        .find(|candidate| is_file(candidate))
}

fn detect_model_path(kind: ModelKind) -> Option<String> {
    if matches!(kind, ModelKind::Whisper) {
        if let Some(bundled) = bundled_whisper_model_path() {
            return Some(bundled);
        }
    }

    let roots = search_roots();
    let mut candidates = spotlight_search(
        match kind {
            ModelKind::Whisper => WHISPER_MODEL_QUERY,
            ModelKind::SpeakerEmbedding => SPEAKER_EMBEDDING_QUERY,
            ModelKind::SpeakerSegmentation => SPEAKER_SEGMENTATION_QUERY,
        },
        &roots,
    );

    collect_candidates_from_common_dirs(&mut candidates, &kind, &roots);
    dedupe_and_select_best(candidates, kind)
}

/// Directories model auto-detection is allowed to look in: the current
/// working directory plus a handful of common download/model locations under
/// the user's home. Shared between [`spotlight_search`] (as `-onlyin` scopes)
/// and the manual directory walk so both stay in sync and neither searches
/// the whole disk — an unscoped `mdfind` query hits every Spotlight-indexed
/// location, including Photos and Music libraries, which is why macOS was
/// prompting for those permissions despite Scribe never touching them.
fn search_roots() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut roots = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        if is_searchable_root(&current_dir, home.as_deref()) {
            roots.push(current_dir);
        }
    }
    if let Some(home) = home {
        for suffix in ["models", "Downloads", "Documents", "Desktop", ".cache"] {
            let root = home.join(suffix);
            if root.is_dir() {
                roots.push(root);
            }
        }
    }
    roots
}

/// Whether `root` is narrow enough to search. Anything at or above the user's
/// home is not.
///
/// The working directory is in the list so a model sitting next to the project
/// can be found during development. That is harmless from a project directory
/// and catastrophic from `/`: an app launched from Finder inherits `/` as its
/// working directory, so the "scoped" search became `mdfind -onlyin /`, the
/// very whole-disk query this scoping exists to prevent, and the fallback walk
/// recursed five levels from the filesystem root. Observed 2026-08-28: a
/// freshly installed build hung in `tauri::setup` on the main thread, before
/// any window existed, with `mdfind -onlyin /` running for minutes. It only
/// ever looked fine in development, where the working directory is the repo.
fn is_searchable_root(root: &Path, home: Option<&Path>) -> bool {
    if root.parent().is_none() {
        return false;
    }
    // `/Users`, and anything else the home directory sits under, are as bad as
    // `/` for this purpose.
    match home {
        Some(home) => !home.starts_with(root) || home == root,
        None => true,
    }
}

/// How long Spotlight gets to answer before its results are done without.
///
/// This runs inside `tauri::setup` on the main thread, so an unbounded wait is
/// a hung app with no window and no way to quit it from the UI - which is
/// exactly what an unscoped query produced. Detection is a convenience: a
/// missing answer costs the user a manual path in Settings, where hanging
/// costs them the app.
const SPOTLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `command` to completion, killing it if it outlives `timeout`.
/// `None` if it could not be started, was killed, or its output was unreadable.
fn run_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spotlight_search(query: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    if roots.is_empty() {
        return Vec::new();
    }

    let mut command = Command::new("mdfind");
    for root in roots {
        command.arg("-onlyin").arg(root);
    }
    command.arg(query);

    let Some(output) = run_with_timeout(command, SPOTLIGHT_TIMEOUT) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| is_file(path))
        .collect()
}

fn collect_candidates_from_common_dirs(
    candidates: &mut Vec<PathBuf>,
    kind: &ModelKind,
    roots: &[PathBuf],
) {
    for root in roots {
        collect_matching_files(root, 5, kind, candidates);
    }
}

fn collect_matching_files(
    root: &Path,
    depth: usize,
    kind: &ModelKind,
    candidates: &mut Vec<PathBuf>,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_matching_files(&path, depth - 1, kind, candidates);
            continue;
        }

        if is_candidate_for_kind(&path, kind) {
            candidates.push(path);
        }
    }
}

fn is_candidate_for_kind(path: &Path, kind: &ModelKind) -> bool {
    if !is_file(path) {
        return false;
    }

    let name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    match kind {
        ModelKind::Whisper => {
            (name.ends_with(".bin") || name.ends_with(".gguf")) && name.starts_with("ggml-")
        }
        ModelKind::SpeakerEmbedding => {
            name.ends_with(".onnx") && (name.contains("embedding") || name.contains("speaker"))
        }
        ModelKind::SpeakerSegmentation => {
            name.ends_with(".onnx")
                && (name.contains("segmentation")
                    || name.contains("pyannote")
                    || name.contains("speaker"))
        }
    }
}

fn dedupe_and_select_best(mut candidates: Vec<PathBuf>, kind: ModelKind) -> Option<String> {
    candidates.retain(|path| is_file(path));
    candidates.sort();
    candidates.dedup();
    candidates.sort_by(|left, right| {
        let left_score = score_candidate(left, &kind);
        let right_score = score_candidate(right, &kind);
        right_score
            .cmp(&left_score)
            .then_with(|| left.as_os_str().len().cmp(&right.as_os_str().len()))
            .then_with(|| left.cmp(right))
    });

    candidates
        .into_iter()
        .next()
        .map(|path| path.to_string_lossy().into_owned())
}

fn score_candidate(path: &Path, kind: &ModelKind) -> i32 {
    let normalized_path = path.to_string_lossy().to_ascii_lowercase();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let mut score = match kind {
        ModelKind::Whisper => score_whisper_model(&file_name),
        ModelKind::SpeakerEmbedding => score_speaker_embedding_model(&file_name),
        ModelKind::SpeakerSegmentation => score_speaker_segmentation_model(&file_name),
    };

    if normalized_path.contains("/target/") || normalized_path.contains("/node_modules/") {
        score -= 80;
    }
    if normalized_path.contains("/downloads/") {
        score -= 10;
    }

    score
}

fn score_whisper_model(file_name: &str) -> i32 {
    let mut score = 10;
    if file_name.starts_with("ggml-") {
        score += 20;
    }
    if file_name.contains("base-q5") {
        score += 120;
    } else if file_name.contains("base") {
        score += 110;
    } else if file_name.contains("small-q5") {
        score += 100;
    } else if file_name.contains("small") {
        score += 90;
    } else if file_name.contains("medium") {
        score += 60;
    } else if file_name.contains("tiny") {
        score += 40;
    }
    if file_name.contains("large") {
        score -= 40;
    }
    score
}

fn score_speaker_embedding_model(file_name: &str) -> i32 {
    let mut score = 10;
    if file_name.contains("speaker") {
        score += 50;
    }
    if file_name.contains("embedding") {
        score += 80;
    }
    if file_name.contains("segmentation") {
        score -= 60;
    }
    score
}

fn score_speaker_segmentation_model(file_name: &str) -> i32 {
    let mut score = 10;
    if file_name.contains("segmentation") {
        score += 90;
    }
    if file_name.contains("pyannote") {
        score += 70;
    }
    if file_name.contains("speaker") {
        score += 20;
    }
    if file_name.contains("embedding") {
        score -= 60;
    }
    score
}

fn is_file(path: &Path) -> bool {
    path.exists() && path.is_file()
}

#[cfg(test)]
mod root_scope_tests {
    use super::{is_searchable_root, run_with_timeout};
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn the_filesystem_root_is_never_searched() {
        // The bug: an app launched from Finder inherits `/` as its working
        // directory, so this list turned `mdfind -onlyin <scope>` into a
        // whole-disk query and pointed a 5-deep directory walk at `/`. Startup
        // hung on the main thread inside tauri::setup, with no window yet and
        // no way to quit from the UI.
        let home = Path::new("/Users/someone");
        assert!(!is_searchable_root(Path::new("/"), Some(home)));
    }

    #[test]
    fn directories_above_home_are_never_searched() {
        // `/Users` is as bad as `/` here - every account on the machine, five
        // levels deep, and someone else's home is not ours to walk.
        let home = Path::new("/Users/someone");
        assert!(!is_searchable_root(Path::new("/Users"), Some(home)));
    }

    #[test]
    fn ordinary_directories_are_searched() {
        let home = Path::new("/Users/someone");
        assert!(is_searchable_root(
            Path::new("/Users/someone/models"),
            Some(home)
        ));
        assert!(is_searchable_root(Path::new("/Users/someone"), Some(home)));
        assert!(is_searchable_root(Path::new("/opt/models"), Some(home)));
    }

    #[test]
    fn a_command_that_never_finishes_is_killed() {
        // Detection runs on the main thread during setup, so "wait forever" is
        // a hung app. Better to lose the answer than the window.
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let started = Instant::now();
        assert!(run_with_timeout(command, Duration::from_millis(300)).is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "run_with_timeout must not outlive its deadline"
        );
    }

    #[test]
    fn a_command_that_finishes_returns_its_output() {
        let mut command = Command::new("/bin/echo");
        command.arg("hello");
        let output = run_with_timeout(command, Duration::from_secs(5)).expect("echo completes");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_model_ranking_prefers_base_q5_variants() {
        let candidates = [
            PathBuf::from("/Users/example/models/ggml-small.bin"),
            PathBuf::from("/Users/example/models/ggml-base-q5_1.bin"),
            PathBuf::from("/Users/example/models/ggml-medium.bin"),
        ];

        let best = candidates
            .iter()
            .max_by_key(|path| score_candidate(path, &ModelKind::Whisper))
            .expect("candidate exists");

        assert_eq!(
            best,
            &PathBuf::from("/Users/example/models/ggml-base-q5_1.bin")
        );
    }

    #[test]
    fn speaker_segmentation_ranking_avoids_embedding_models() {
        let candidates = [
            PathBuf::from("/Users/example/models/speaker-embedding.onnx"),
            PathBuf::from("/Users/example/models/pyannote-segmentation-3.0.onnx"),
        ];

        let best = candidates
            .iter()
            .max_by_key(|path| score_candidate(path, &ModelKind::SpeakerSegmentation))
            .expect("candidate exists");

        assert_eq!(
            best,
            &PathBuf::from("/Users/example/models/pyannote-segmentation-3.0.onnx")
        );
    }

    #[test]
    fn hydrate_settings_only_fills_missing_paths() {
        let detected = DetectedLocalPaths {
            transcriber_bin_path: Some("/opt/homebrew/bin/whisper-cli".to_string()),
            transcriber_model_path: Some("/Users/example/models/ggml-base-q5_1.bin".to_string()),
            speaker_embedding_model_path: Some(
                "/Users/example/models/speaker-embedding.onnx".to_string(),
            ),
            speaker_segmentation_model_path: Some(
                "/Users/example/models/speaker-segmentation.onnx".to_string(),
            ),
        };
        let mut hydrated = ScribeSettings {
            transcriber_bin_path: None,
            transcriber_model_path: Some("/Users/custom/model.bin".to_string()),
            ..ScribeSettings::default()
        };
        hydrate_settings_with_detected_paths(&mut hydrated, &detected);

        assert_eq!(
            hydrated.transcriber_bin_path.as_deref(),
            Some("/opt/homebrew/bin/whisper-cli")
        );
        assert_eq!(
            hydrated.transcriber_model_path.as_deref(),
            Some("/Users/custom/model.bin")
        );
        assert_eq!(
            hydrated.speaker_embedding_model_path.as_deref(),
            Some("/Users/example/models/speaker-embedding.onnx")
        );
        assert_eq!(
            hydrated.speaker_segmentation_model_path.as_deref(),
            Some("/Users/example/models/speaker-segmentation.onnx")
        );
    }
}
