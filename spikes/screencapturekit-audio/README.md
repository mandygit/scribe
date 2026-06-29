# ScreenCaptureKit system-audio spike

Standalone Resonance spike for capturing system/app audio with `SCStream`.

## What it does

- Builds a Swift Package executable with Xcode Command Line Tools only.
- Assembles a minimal `.app` bundle with bundle id `dev.resonance.screencapturekit-audio-spike`.
- Ad-hoc signs the bundle.
- Requests/checks Screen Recording permission.
- Captures about 10 seconds of system/app audio only, with no microphone input.
- Writes `~/Desktop/resonance-system-audio-spike.m4a`.

The implementation uses the ScreenCaptureKit audio path with a tiny 2×2 screen output workaround, adds `.screen` output before `.audio`, sets `excludesCurrentProcessAudio = true`, and requests shareable content with `onScreenWindowsOnly: false`.

The `.m4a` output is only a temporary validation artifact for this macOS spike. The production app should keep audio capture format-agnostic by normalizing native capture buffers into PCM `AudioFrame`s for AEC/transcription, then writing platform-appropriate storage formats through separate writer adapters.

## Run

```bash
cd /Users/nama4008/Projects/meeting-coach/spikes/screencapturekit-audio
chmod +x build_and_run.sh
./build_and_run.sh
```

When prompted, allow Screen Recording for **Resonance System Audio Spike**, then run the script again if macOS requires a relaunch.

Because the app is rebuilt and ad-hoc signed locally, TCC may treat rebuilds as a changed app and Screen Recording permission may reset between rebuilds. If that happens, remove/re-add the app in System Settings → Privacy & Security → Screen Recording.

If the Screen Recording control is disabled on a managed Mac, ask the MDM administrator to allow Screen Recording for bundle id `dev.resonance.screencapturekit-audio-spike`.
