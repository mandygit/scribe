use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    build_system_audio_sidecar();
    tauri_build::build();
}

fn build_system_audio_sidecar() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown-target".to_string());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir is set"));
    let source = manifest_dir.join("native/system-audio-capture/main.swift");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rustc-env=RESONANCE_SYSTEM_AUDIO_HELPER_NAME=resonance-system-audio-capture");
    println!("cargo:rustc-env=RESONANCE_SYSTEM_AUDIO_HELPER_TARGET={target}");

    if target_os != "macos" {
        return;
    }

    let binary_dir = manifest_dir.join("binaries");
    std::fs::create_dir_all(&binary_dir).expect("system audio sidecar directory can be created");
    let output = binary_dir.join(format!("resonance-system-audio-capture-{target}"));

    if sidecar_binary_is_current(&source, &output) {
        return;
    }

    compile_swift_sidecar(&source, &output);
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

fn compile_swift_sidecar(source: &Path, output: &Path) {
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            source.to_str().expect("swift source path is valid UTF-8"),
            "-o",
            output.to_str().expect("sidecar output path is valid UTF-8"),
            "-framework",
            "AVFoundation",
            "-framework",
            "CoreGraphics",
            "-framework",
            "CoreMedia",
            "-framework",
            "ScreenCaptureKit",
        ])
        .status()
        .expect("xcrun swiftc is available to build the system audio sidecar");

    assert!(
        status.success(),
        "failed to build ScreenCaptureKit system audio sidecar"
    );
}
