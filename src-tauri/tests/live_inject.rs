//! Live text-injection check: `inject_text` must land the dictation in whatever
//! app has focus. Drives a throwaway TextEdit document as the focused target,
//! injects a unique marker, and reads it back.
//!
//! Ignored by default because it needs the Accessibility permission and drives
//! the GUI. Run it with:
//!
//! ```sh
//! cargo test --test live_inject -- --ignored --nocapture
//! ```

use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use scribe_lib::dictation::inject_text;

fn osascript(script: &str) -> String {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .expect("osascript runs");
    assert!(
        output.status.success(),
        "osascript failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
#[ignore = "requires Accessibility permission and drives TextEdit"]
fn live_inject_into_textedit() {
    let marker = format!("scribe-inject-{}", std::process::id());

    // Focus a fresh TextEdit document as the injection target.
    osascript("tell application \"TextEdit\" to activate");
    osascript("tell application \"TextEdit\" to make new document");
    sleep(Duration::from_millis(800));

    inject_text(&marker).expect("inject_text succeeds with Accessibility granted");
    sleep(Duration::from_millis(500));

    let contents = osascript("tell application \"TextEdit\" to get text of front document");
    println!("\n===== TEXTEDIT CONTENTS =====\n{contents}\n=============================");

    // Clean up before asserting so a failure doesn't leave a stray document.
    osascript("tell application \"TextEdit\" to close front document saving no");

    assert!(
        contents.contains(&marker),
        "expected injected marker {marker:?} in TextEdit, got {contents:?}"
    );
}
