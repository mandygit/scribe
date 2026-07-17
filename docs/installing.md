# Installing Scribe

You received a file like `Scribe_0.1.0_aarch64.dmg`. Everything Scribe needs
for recording and transcription is inside it - there is nothing to install
first, no Homebrew, no whisper, no ffmpeg.

## Prerequisites

- A Mac with Apple Silicon (M1 or newer)
- macOS 13 (Ventura) or newer
- Optional, only for AI summaries: a local LLM server (LM Studio or Ollama,
  see below). Recording, transcription, and notes work without it.

## Install

1. Open the DMG and drag **Scribe** into **Applications**.
2. Scribe is an internal build and not notarized by Apple, so the very first
   launch is blocked by Gatekeeper. Do ONE of the following once:
   - **Right-click** (or Control-click) `Scribe.app` in Applications, choose
     **Open**, then confirm **Open** in the dialog. If macOS only offers
     "Move to Trash / Cancel", cancel and go to **System Settings → Privacy &
     Security**, scroll down, and click **Open Anyway**.
   - Or run this in Terminal and launch normally afterwards:

     ```bash
     xattr -cr /Applications/Scribe.app
     ```

3. After that one-time step, Scribe opens like any other app.

If macOS claims the app "is damaged and can't be opened", that is the same
Gatekeeper block in different words - the `xattr` command above fixes it.

## First launch

Scribe walks you through the permissions it can use. All of them are
optional; each one just unlocks a feature:

| Permission | What it unlocks |
| --- | --- |
| Microphone | Recording meetings and dictation |
| Screen Recording | Capturing other participants' audio in meetings (otherwise meetings record mic-only) |
| Accessibility | Letting dictation type into other apps |

You can skip any of this and revisit it later in **Settings → Permissions**.

## Optional: AI summaries with a local model

Summaries run against a local LLM server on your own machine (nothing leaves
your Mac). Either works:

- **LM Studio**: install from lmstudio.ai, download a model (e.g. a small
  Llama or Gemma variant), and start the local server.
- **Ollama**: install from ollama.com, then e.g. `ollama pull llama3.2` and
  leave it running.

Then open **Scribe → Settings → Local model**, pick your provider, and select
the model from the list. Transcripts and notes work fine without this - you
just won't get generated summaries.

## Troubleshooting

- **No system audio in transcripts**: grant Screen Recording in System
  Settings → Privacy & Security, then quit and reopen Scribe.
- **Meeting transcripts sound telephone-quality with Bluetooth headphones**:
  that is the headset's hands-free microphone; pick the built-in mic in
  Scribe's audio settings or let "System default" handle it.
- **Summaries button does nothing / errors**: make sure LM Studio or Ollama
  is running and a model is loaded, then check Settings → Local model.
