use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    build_system_audio_sidecar();
    build_dictation_polish_sidecar();
    tauri_build::build();
}

fn build_system_audio_sidecar() {
    build_swift_sidecar(
        "native/system-audio-capture/main.swift",
        "scribe-system-audio-capture",
        "SCRIBE_SYSTEM_AUDIO_HELPER",
        &["AVFoundation", "CoreGraphics", "CoreMedia", "ScreenCaptureKit"],
    );
}

fn build_dictation_polish_sidecar() {
    build_swift_sidecar(
        "native/dictation-polish/main.swift",
        "scribe-dictation-polish",
        "SCRIBE_DICTATION_POLISH_HELPER",
        &["FoundationModels"],
    );
}

/// Compiles a Swift sidecar into `binaries/<name>-<target>` on macOS and exports
/// `<env_prefix>_NAME` / `<env_prefix>_TARGET` so the crate can resolve it.
fn build_swift_sidecar(source_rel: &str, name: &str, env_prefix: &str, frameworks: &[&str]) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown-target".to_string());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir is set"));
    let source = manifest_dir.join(source_rel);

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rustc-env={env_prefix}_NAME={name}");
    println!("cargo:rustc-env={env_prefix}_TARGET={target}");

    if target_os != "macos" {
        return;
    }

    let binary_dir = manifest_dir.join("binaries");
    std::fs::create_dir_all(&binary_dir).expect("sidecar directory can be created");
    let output = binary_dir.join(format!("{name}-{target}"));

    if sidecar_binary_is_current(&source, &output) {
        return;
    }

    compile_swift_sidecar(&source, &output, frameworks);
}

fn sidecar_binary_is_current(source: &Path, output: &Path) -> bool {
    let Ok(source_modified_at) = std::fs::metadata(source).and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    let Ok(output_modified_at) = std::fs::metadata(output).and_then(|metadata| metadata.modified())
    else {
        return false;
    };

    output_modified_at >= source_modified_at
}

fn compile_swift_sidecar(source: &Path, output: &Path, frameworks: &[&str]) {
    let mut command = Command::new("xcrun");
    command.args([
        "swiftc",
        "-parse-as-library",
        "-O",
        source.to_str().expect("swift source path is valid UTF-8"),
        "-o",
        output.to_str().expect("sidecar output path is valid UTF-8"),
    ]);
    for framework in frameworks {
        command.arg("-framework").arg(framework);
    }

    let status = command
        .status()
        .expect("xcrun swiftc is available to build the Swift sidecar");

    assert!(
        status.success(),
        "failed to build Swift sidecar: {}",
        source.display()
    );
}
