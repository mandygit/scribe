#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="Scribe System Audio Spike"
BUNDLE_ID="dev.scribe.screencapturekit-audio-spike"
APP_DIR="$SCRIPT_DIR/.build/app/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
BINARY_PATH="$MACOS_DIR/SpikeAudioCapture"

echo "Building Swift package with Xcode Command Line Tools..."
xcrun swift build --package-path "$SCRIPT_DIR" -c debug

echo "Assembling app bundle: $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$SCRIPT_DIR/.build/debug/SpikeAudioCapture" "$BINARY_PATH"
cp "$SCRIPT_DIR/Info.plist" "$CONTENTS_DIR/Info.plist"

echo "Ad-hoc signing app bundle with entitlements..."
/usr/bin/codesign --force --deep --sign - \
  --entitlements "$SCRIPT_DIR/Entitlements.entitlements" \
  "$APP_DIR"

echo "Verifying signature..."
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_DIR"

cat <<EOF

Launching $APP_NAME ($BUNDLE_ID)...

If macOS prompts for Screen Recording, enable this app in:
  System Settings → Privacy & Security → Screen Recording

Because this script rebuilds and ad-hoc signs a local .app, TCC may treat
rebuilt bundles as changed and Screen Recording permission may reset between
rebuilds. If capture fails after a rebuild, remove/re-add the app in Screen
Recording settings and run this script again.

The spike writes:
  ~/Desktop/scribe-system-audio-spike.m4a

EOF

open -n "$APP_DIR"
