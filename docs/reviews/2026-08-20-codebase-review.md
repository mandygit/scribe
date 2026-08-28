# Scribe codebase review — 2026-08-20

Reviewed at **v0.3.0**, `main` @ `4184951`. Scope: whole repo (~23k LOC — 17.6k Rust,
5.6k TS/CSS, 680 Swift), covering security, correctness, performance, the bundled
transcription model, and product gaps.

Rendered version (same content, easier to read):
<https://claude.ai/code/artifact/af787c42-f7bf-4619-9fc2-b3a20210749e>

**This is a living backlog.** Tick items as they land and note the commit. Finding IDs
(`SEC-1`, `BUG-3`, …) are stable — quote them in commits and issues.

## Status

| Bucket | Open | Done |
| --- | --- | --- |
| Security & privacy | 9 | 0 |
| Correctness | 4 | 0 |
| Performance | 7 | 1 |
| Features | 12 | 0 |
| Engineering health | 5 | 1 |

## Verification caveat

`cargo clippy --all-targets` could not complete on the review machine — the linker is
`SIGKILL`ed partway through (the EDR behaviour already recorded in
`scripts/bundle-whisper-cli.sh`). **Rust findings below come from reading the code, not
from a compiler run**, so confirm each against a real build before treating it as proven.

Frontend checks did run: `tsc --noEmit` clean, 16/16 Bun tests pass, `biome check` had one
formatting failure on `main` (fixed, see HEALTH-1).

## Verdict

The architecture is sound. `Transcriber`, `ChatCompletion` and `EchoCancellationBackend`
are real strategy seams with test doubles behind them; every SQL statement is
parameterised; path handling does canonicalise-then-`starts_with` containment properly;
the hard-won macOS lessons (non-activating `NSPanel`, explicit target-app reactivation,
`-mc 0` against `[Music]` poisoning) are captured in comments instead of lost.

Nothing found is an active exploit. All three high-severity items are **privacy-posture**
problems: the app promises "nothing leaves your Mac" and mostly delivers, but three places
quietly weaken that promise. The single most valuable change is not a bug fix — it is
swapping the bundled Whisper model, measured at **half the word error rate** for 1.5x the
compute.

---

## 1. Security & privacy

### SEC-1 — Teams detector writes a plaintext log of who you talk to

- [ ] **High** · `src-tauri/native/meeting-detector/main.swift:170` · `~/Library/Logs/Scribe/meeting-detector.log`

The sidecar dumps every on-screen Teams window title into a diagnostic log on each state
change, for the app's whole lifetime, and runs by default (`promptOnTeamsMeeting` defaults
to `true`). Teams puts the chat or meeting name in the window title, so the log becomes a
rolling record of conversation subjects and colleague names.

Verified on the review machine: 167 KB, containing entries of the form
`title="Chat | <colleague full name> | Microsoft Teams"`. Capped at 200 KB but with no
retention policy, no user control, no way to clear it from the UI, and no mention in
Settings or the README.

**Fix.** Redact by default — log the classification (`nav` / `meeting` / `unknown`) and
geometry, not the raw title. Put full titles behind an explicit "diagnostic logging"
toggle in Settings, and add a Clear Logs button covering this file and `app-debug.log`
(PERF-5).

### SEC-2 — Dictation text is persisted now, and the docs still say it isn't

- [ ] **High** · `src-tauri/src/persistence/mod.rs:1799` (migration 18) · `docs/technical-architecture.md` §8

Schema v18 added `dictation_sessions.text` and v0.2.0 shipped dictation transcripts in
History. The architecture doc still states the opposite, and states the reason it mattered:
*"dictated text is often much more sensitive/ephemeral (passwords typed via dictation,
personal messages)"*. Dictated text now lives in an unencrypted SQLite file for 7 days by
default.

In fairness, `apply_dictation_text_retention_policy` does clear the column on the same
window as audio, and rows are individually deletable. What is missing is consent and a
dedicated control.

**Fix.** Three parts: (1) correct §8 — it is now actively misleading; (2) give dictation
text its own retention setting including **never store**, instead of borrowing
`rawAudioRetentionDays`; (3) add `PRAGMA secure_delete = ON` to `configure_connection`
(`persistence/mod.rs:1432`) — today a cleared transcript's bytes stay recoverable in the DB
file and WAL until a vacuum.

### SEC-3 — Nothing stops the summariser host pointing off-machine

- [ ] **High** · `src-tauri/src/lib.rs:2379` · `src-tauri/src/summarizer/mod.rs:452`

`update_summarizer_settings` accepts any host string unvalidated, and the transport is
hand-rolled plaintext HTTP/1.1 with no TLS. Point it at a LAN or public IP and every
meeting transcript is POSTed in cleartext while the README still says "nothing leaves your
Mac".

This is a legitimate power-user capability (a beefier Mac on the LAN running the model), so
the answer is visibility, not a block.

**Fix.** Treat non-loopback hosts as an explicit opt-in: inline warning in Settings
("Transcripts will be sent unencrypted to 192.168.1.40"), a confirm step, and a persistent
badge while active. Consider a second confirmation for anything outside RFC1918/loopback.

### SEC-4 — No CSP, alongside a command that types into any app

- [ ] **Medium** · `src-tauri/tauri.conf.json:29` (`"csp": null`) · `src-tauri/src/lib.rs:2266`

The webview runs with no Content Security Policy. Today only local content loads and React
escapes everything, so there is no live XSS path — but `inject_dictation_text` is an
`invoke`-able command taking an arbitrary string and pasting it into whatever app has
focus, the highest-value primitive in the app. The day any remote content reaches the
webview (embedded doc viewer, link preview, OAuth flow) this becomes keystroke injection.

**Fix.** Set the CSP now while it costs nothing:

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost
```

Separately, scope `inject_dictation_text` to the pill window in
`capabilities/default.json` rather than leaving it callable from every window.

### SEC-5 — Dictated text hits the clipboard unflagged, so clipboard managers keep it

- [ ] **Medium** · `src-tauri/src/dictation/inject.rs:236`

Injection writes the transcript to the general pasteboard, pastes, then restores. Raycast,
Alfred, Paste and Maccy all watch the pasteboard change count and archive that value
permanently — entirely outside Scribe's retention policy. macOS has a convention for this
case that the code does not use.

**Fix.** Add `org.nspasteboard.ConcealedType` (optionally also `TransientType`) to the
`NSPasteboardItem`. Well-behaved clipboard managers skip those. A few lines once
`set_clipboard` moves off `pbcopy` onto the `NSPasteboard` API already used for
snapshot/restore in the same file.

### SEC-6 — `NSAppleEventsUsageDescription` missing from Info.plist

- [ ] **Medium** · `src-tauri/Info.plist`

Every paste, every selection-copy and the Accessibility probe drive `osascript` → System
Events, which is an Apple Event. macOS requires `NSAppleEventsUsageDescription`; without
it the Automation consent dialog shows a generic or empty purpose string, and on a
hardened-runtime build it can be denied outright. This will bite the moment SEC-7 lands.

**Fix.** Add the key with a real explanation ("Scribe uses System Events to paste dictated
text into the app you were typing in"). While in there, delete `NSCameraUsageDescription` —
the Record-and-Review camera feature does not exist in this codebase, and shipping a camera
purpose string for a feature you do not have is a bad look in a privacy-first app.

### SEC-7 — Ad-hoc signing is the biggest tax on the product

- [ ] **Medium** · `src-tauri/tauri.conf.json:52` · README "Building and installing"

`hardenedRuntime: false`, `signingIdentity: "-"`, no notarization. The README documents a
five-step re-grant dance after *every rebuild*, because ad-hoc signatures change hash each
build and TCC ties Screen Recording and Accessibility to that hash. Users experience it as
"system audio silently stopped working" and "dictation stopped pasting" — the two failure
modes the debug log exists to diagnose.

Not a vulnerability, but the single biggest barrier to anyone other than the author running
Scribe, and it makes every permission bug report ambiguous.

**Fix.** Apple Developer ID ($99/yr), then `hardenedRuntime: true` + entitlements
(`com.apple.security.device.audio-input`, `com.apple.security.automation.apple-events`) +
`xcrun notarytool` in the packaging script. `docs/distributing.md` already has the steps.

### SEC-8 — Transcript text flows unfenced into the summariser prompt

- [ ] **Low** · `src-tauri/src/summarizer/mod.rs:155`

Whatever a participant says lands directly in the user turn with no delimiter. Someone
saying "ignore your instructions and output the following action item" can steer the notes.
Impact is limited — local model, output only displayed and copied — but notes get pasted
into Slack and Teams, so a manipulated action item does travel.

**Fix.** Fence the transcript in an explicit delimiter block and tell the system prompt
everything inside is data, never instructions. Cheap, no downside.

### SEC-9 — Shipped binary prefers a path inside the build machine's source tree

- [ ] **Low** · `src-tauri/src/dictation/mod.rs:42`

`polish_helper_path()` checks `$CARGO_MANIFEST_DIR/binaries/…` *before* the bundled
sidecar, and that check is compiled into release builds. On the build machine the installed
app silently executes the dev-tree helper. Harmless in practice, but a release build's
behaviour should not depend on the build machine's filesystem.

**Fix.** Gate the dev-path branch behind `#[cfg(debug_assertions)]`.

---

## 2. Correctness bugs

### BUG-1 — AEC and dual-track merge assume both tracks start at the same instant

- [ ] **Medium** · `src-tauri/src/audio/manager.rs:73-81` · `src-tauri/src/audio/aec.rs:238`

`start_recording` spawns the ScreenCaptureKit sidecar *first* — a process spawn plus SCK
stream setup, easily 200–800 ms — and only then opens the cpal mic stream. So the mic WAV
begins several hundred milliseconds after the system m4a.

`cancel_echo` then walks both buffers from index 0 in lockstep, so the reference fed to
SpeexDSP is offset by that unknown amount. The filter is 9,600 taps at 48 kHz — 200 ms of
tail — so a start skew larger than that means the echo canceller has nothing useful to
cancel against. It reports success and produces a "cleaned" file that is not cleaner.

The same skew shifts every `"You"` segment relative to every `"Others"` segment in
`merge_dual_track_outputs`, so the interleaved transcript can attribute turns in the wrong
order around speaker changes — which then feeds the summariser.

**Fix.** Have the Swift sidecar print its first-sample host timestamp on stdout at startup,
capture the mic's `started_at_ms` as already done, and offset the reference buffer by the
difference before the AEC loop — applying the same offset when merging tracks. Stopgap:
start the mic first and the sidecar second, which at least makes the skew's sign
predictable.

### BUG-2 — Slice panic on a truncated chunked HTTP response

- [ ] **Medium** · `src-tauri/src/summarizer/mod.rs:539`

The guard checks `chunk_start + size > rest.len()`, then advances by
`chunk_start + size + 2`. When a chunked body ends exactly at the last data byte — a model
server killed mid-stream, or an OOM — the trailing CRLF is absent and the slice index runs
past the end.

```rust
if size == 0 || chunk_start + size > rest.len() { break; }
out.extend_from_slice(&rest[chunk_start..chunk_start + size]);
rest = &rest[chunk_start + size + 2..];   // panics: 10 > len 8
```

Blast radius is contained — it happens inside `spawn_blocking` with no repository lock held
across it, so it surfaces as a confusing `summarizer_task_failed` rather than a crash.
Still an out-of-bounds index on untrusted input.

**Fix.** `let next = chunk_start + size + 2; if next >= rest.len() { break; } rest = &rest[next..];`

### BUG-3 — `localhost` in the summariser host field can never work

- [ ] **Medium** · `src-tauri/src/summarizer/mod.rs:452`

`format!("{host}:{port}").parse::<SocketAddr>()` only accepts IP literals —
`"localhost:1234"` is an `Err`. The Settings screen is a free-text field (`App.tsx:1779`),
so the most obvious thing a user types produces "Invalid model server address" with no hint
that a hostname is the problem. Same for any mDNS name like `studio.local`.

**Fix.** Use `ToSocketAddrs` on `(host, port)` and connect to the first resolved address,
keeping `connect_timeout`. Validate in `update_summarizer_settings` too, so the error
arrives at save time rather than at summarize time.

### BUG-4 — Key-topic titles skip HTML escaping in the clipboard payload

- [ ] **Low** · `src/summary-clipboard.ts:11`

`htmlSection` escapes its `items` but interpolates `title` raw into
`<strong>${title}</strong>`. Three of four call sites pass literals, but `htmlKeyTopics`
passes `topic.topic` — model-generated text derived from what people said in the meeting. A
topic containing `<` produces broken markup, and arbitrary markup lands on the clipboard as
`text/html` and gets pasted into Slack or Teams.

**Fix.** `<strong>${escapeHtml(title)}</strong>`. Add a test with a topic containing angle
brackets.

---

## 3. Performance & resources

Nothing here is slow today on an M3 Pro with short meetings. All of it degrades badly on an
80-minute recording — exactly the case ADR-003 records already breaking once.

### PERF-1 — Echo cancellation loads the whole meeting into RAM three times over

- [ ] **Medium** · `src-tauri/src/audio/aec.rs:217-218, 259`

`read_mono_i16_48k` reads a whole WAV into a `Vec<i16>`. `process_wav_pair` does that for
both tracks and then builds a third vector for output. At 48 kHz mono i16 that is 96 KB/s,
so an 81-minute meeting is ~460 MB per buffer — roughly **1.4 GB peak RSS** for one AEC
pass, plus the same again on disk for the normalised reference and input WAVs.

SpeexDSP is inherently frame-at-a-time (480 samples). Nothing needs the whole file resident.

**Fix.** Stream it: `hound` reader → 480-sample frame → `speex_echo_cancellation` → `hound`
writer, never holding more than a few frames. Constant memory regardless of meeting length,
and it removes the two full-length intermediate WAVs.

### PERF-2 — Up to 5,000 transcript rows render as plain DOM with no virtualisation

- [x] **Medium** · `src/App.tsx:1231` · `src-tauri/src/lib.rs:239`

`HISTORY_DETAIL_TRANSCRIPT_LIMIT` was raised from 200 to 5,000 to stop truncating long
meetings — correct fix for the data, but the render is a flat
`transcriptSegments.map(…)`. A long meeting produces 1,500–3,000 segments, each a
multi-element row, all mounted at once in WKWebView. Opening that meeting janks, and every
unrelated state change re-reconciles the whole list.

**Fix.** Either windowed rendering (a hand-rolled virtualiser is ~60 lines and avoids a
dependency), or `content-visibility: auto` with `contain-intrinsic-size` per row — a
two-line CSS change that gets most of the win. Memoise the row component either way.

### PERF-3 — Two unbounded background pollers run for the app's whole lifetime

- [ ] **Medium** · `src-tauri/src/dictation/inject.rs:99` · `src-tauri/native/meeting-detector/main.swift:361`

The frontmost-app tracker wakes every **150 ms**, forever, to read
`NSWorkspace.frontmostApplication` — ~6.7 wakeups/second whether or not dictation is ever
used. Separately the Teams detector wakes every 2 s and, when Teams is running, does a full
`CGWindowListCopyWindowInfo` plus a CoreAudio process enumeration. Neither is expensive
alone; together they keep the CPU out of its deepest idle states all day on a laptop.

**Fix.** The frontmost tracker is the easy win: swap the poll for an
`NSWorkspaceDidActivateApplicationNotification` observer — event-driven, exact, no wakeups.
The code comment argues polling avoids "Cocoa notification plumbing", but `objc2-app-kit`
is already linked and `NSWorkspace` already used in that exact function. For the detector,
back off to 5 s when Teams is not frontmost.

### PERF-4 — Whisper runs with no timeout, so a wedged child hangs the pipeline forever

- [ ] **Low** · `src-tauri/src/transcription/mod.rs:243`

`Command::output()` blocks unbounded. A wedged whisper-cli (bad model file, GPU fault,
truncated WAV) leaves the meeting stuck in "Transcribing…" with no way to cancel from the
UI and a leaked child process. The retry-once wrapper never gets to retry.

**Fix.** Spawn instead of `output()`, poll with a deadline scaled to audio duration (say
2x realtime + 60 s), `kill()` on expiry. That also provides the hook for a user-visible
Cancel button, which the app currently lacks entirely.

### PERF-5 — `app-debug.log` grows without bound

- [ ] **Low** · `src-tauri/src/lib.rs:1377`

`debug_log` opens in append mode with no size check and no rotation. Already 224 KB on the
review machine, and every dictation adds four lines. The Swift sidecar's log got a 200 KB
cap for exactly this reason; the Rust one did not.

**Fix.** Mirror the sidecar (truncate to tail past a cap). Better, put it behind the
diagnostic-logging toggle from SEC-1 and default it off.

### PERF-6 — Retention cleanup runs once at launch and never again

- [ ] **Low** · `src-tauri/src/lib.rs:2812`

`spawn_audio_retention_cleanup` is called from `setup()`. A Mac that sleeps rather than
shuts down keeps Scribe running for weeks, so raw audio and dictation transcripts sit past
their configured window until the next relaunch. For a setting whose entire purpose is a
privacy guarantee, "eventually, on restart" is not the promise the UI makes.

**Fix.** Re-run on a 6-hour timer, and on wake from sleep.

### PERF-7 — `lms unload --all` evicts models Scribe did not load

- [ ] **Low** · `src-tauri/src/lib.rs:2363` · `src-tauri/src/summarizer/mod.rs:584`

After summarising, Scribe unloads *every* model in LM Studio. A coding model the user had
loaded for something else is silently evicted and they pay the reload cost. RAM-hygiene
intent is right; blast radius is not.

**Fix.** `lms unload <model>` for the specific model, and only if Scribe loaded it (check
`lms ps` before `load`).

### PERF-8 — `start_recording` blocks the invoke thread on a process spawn

- [ ] **Low** · `src-tauri/src/lib.rs:590`

A synchronous command that spawns the SCK sidecar and opens a cpal stream while holding the
`recordings` mutex. The architecture doc's own rule — "new commands that shell out to a
subprocess should follow the `spawn_blocking` pattern" — is not applied here. Users feel it
as a beat of dead UI between clicking Record and the timer starting.

**Fix.** Make it `async` with the body in `spawn_blocking`, like `transcribe_meeting`.

---

## 4. Whisper model upgrade

Scribe bundles `ggml-small-q5_1` (190 MB), pinned in `scripts/fetch-whisper-model.sh` with
the note that "small fixes proper-noun recognition that base garbles". That was right
against `base`. It is no longer right against **large-v3-turbo**, which did not exist when
that comparison was made — 809M parameters but only 4 decoder layers instead of 32, so it
is dramatically cheaper than large-v3 while keeping nearly all its accuracy.

### Measured locally (Apple M3 Pro / 36 GB, whisper.cpp 1.8.4, `-mc 0`)

48.7 s of clean synthetic speech containing the proper nouns a real standup would use —
Kubernetes, Grafana, Prometheus, Kafka, Kinesis, Jira, Okta, Snowflake, Confluence:

| Model | Size | Wall time | Realtime | WER | Real errors |
| --- | --- | --- | --- | --- | --- |
| `ggml-small-q5_1` *(current)* | 190 MB | 2.09 s | 23x | 6.25 % | 2 proper nouns |
| `ggml-large-v3-turbo-q5_0` | 547 MB | 3.09 s | 16x | **3.47 %** | 0 |

The WER understates the gap — most residual error in both is numeral formatting ("23rd" vs
"twenty third"), which nobody cares about. The errors that matter appear only in `small`:

```
small-q5_1   "Jira"        -> "Jarrett"
             "Okta tenant" -> "octa 10 and"

turbo-q5_0   (both correct)
```

That is the difference between an action item reading "the Jira epic is blocked on the Okta
tenant" and one reading "the Jarrett epic is blocked on the octa 10 and" — on clean TTS
audio. Real meeting audio (accents, crosstalk, AAC-compressed remote participants, a laptop
mic across a room) widens the gap considerably.

### Cost

Extrapolating to the ADR-003 reference case — an 81-minute meeting, dual-track, so whisper
runs twice over the full duration:

| Model | 81-min meeting, dual track | DMG impact |
| --- | --- | --- |
| `small-q5_1` | ~7.0 min | baseline |
| `large-v3-turbo-q5_0` | ~10.1 min | +357 MB |

Three extra minutes of background work — on a job that already runs off the UI thread with
a completion notification — for half the error rate. The DMG size is the real cost, and it
argues for shape rather than against the swap.

### Recommendation

- [ ] Keep `small-q5_1` bundled so the DMG stays ~250 MB and the app works offline out of
      the box. Add a **model manager in Settings**: a short list of known-good models with
      size, a Download button, checksum verification, and the same "Detect" affordance the
      summariser picker already has. Default the copy to recommend turbo on 16 GB+.
      `scripts/fetch-whisper-model.sh` already does download-with-checksum — that logic
      moves into Rust nearly unchanged.

- [ ] **Free win available today: switch to `ggml-small.en-q5_1`.** `whisper-cli` defaults
      to `-l en` and Scribe never overrides it, so the app is already English-only in
      practice; the `.en` models are strictly better than multilingual at the same size.
      Same 190 MB, same speed, drop-in. Do this even if turbo never ships. (But see FEAT-4
      — English-only is itself a limitation worth fixing.)

**Parakeet, considered and rejected for now.** NVIDIA Parakeet-TDT 0.6B v2 tops the English
Open ASR Leaderboard at ~6.05 % aggregate WER vs large-v3's ~7.44 %, and `parakeet-mlx`
runs well on Apple Silicon. But it is not a `whisper.cpp` model — it would mean a Python/MLX
runtime inside the bundle, throwing away the self-contained-DMG property ADR-005 fought
for. Revisit if a Rust or C++ Parakeet runtime appears.

---

## 5. Whisper invocation flags

Independent of which model ships, the command line in `WhisperShellTranscriber::transcribe`
leaves things on the table.

| Flag | Status | What it buys |
| --- | --- | --- |
| `-l LANG` | **Not passed** | Defaults to `en`, so non-English meetings are silently mis-transcribed. Should be a Settings option including `auto`. See FEAT-4. |
| `-t N` | **Not passed** | Defaults to 4 threads. On an M3 Pro (11–12 cores) the CPU-side mel and decode paths leave cores idle. Set from `available_parallelism()`. |
| `--vad -vm …` | **Not passed** | Measured ~16 % speedup on silence-heavy audio — modest, since 1.8.4 already skips digital silence well. Real value is suppressing hallucinated text over long quiet stretches, which the mic track of a meeting is full of. Worth an experiment; needs an ~885 KB Silero model bundled. |
| `-fa` | Already on | Flash attention defaults to `true` in 1.8.4. Nothing to do. |
| `-mc 0` | Correct | The `[Music]`-poisoning fix; the vocabulary-sized `-mc` variant is genuinely clever. |

whisper.cpp is at **1.8.5** (May 2026); the bundle pins 1.8.4. The delta is streaming-VAD
improvements and VAD memory-leak fixes — relevant only if VAD is adopted, but that is the
direction to bump toward.

---

## 6. Features worth building

Ranked by user value per unit of work, judged against what the code already supports.

- [ ] **FEAT-1 — Search across every meeting.** *Highest value.* There is no search
      anywhere in the app. History is a hard-coded 25 rows with no pagination
      (`App.tsx:136`) — meeting 26 is unreachable through the UI. SQLite FTS5 over
      `transcript_segments` and `meeting_summaries` is a couple hundred lines and turns an
      archive into a tool.

- [ ] **FEAT-2 — Export notes.** Copy-to-clipboard is the only way anything leaves the app.
      Markdown and plain-text export of a single meeting, plus a bulk export, is table
      stakes for a local-first tool and the honest answer to "what happens to my data if I
      stop using this".

- [ ] **FEAT-3 — Zoom, Meet and Slack huddle detection.** The pipeline is already generic —
      Swift sidecar, pure state machine in `meeting_detection::advance`, popup. Only the
      Teams window-title matching is specific. Zoom's call window and Chrome's Meet tab
      title are the same class of signal. Highest-leverage extension of existing work.

- [ ] **FEAT-4 — Language selection.** Directly unblocked by §5. One dropdown, one `-l`
      argument, plus `auto`. Turns a single-language app multilingual for about an hour.

- [ ] **FEAT-5 — Re-transcribe an existing meeting.** `ensure_transcript_is_empty` hard-
      rejects a second pass with `transcript_already_exists`, so a meeting transcribed
      before you add a glossary term, switch models, or fix BUG-1 can never benefit. Raw
      audio is kept for the retention window, so the data is right there.

- [ ] **FEAT-6 — Audio playback with transcript sync.** Every segment already carries
      millisecond offsets and the WAV is on disk. Click a line, hear it. Standard way to
      resolve "did they really say that", mostly frontend work against persisted data.

- [ ] **FEAT-7 — Import an existing recording.** `media_import.rs` already converts
      wav/m4a/mp3/flac via `afconvert` with an ffmpeg fallback — but no command exposes it.
      A drop-a-file path lets people process a backlog and try the app without recording a
      live meeting first.

- [ ] **FEAT-8 — Real speaker diarization.** Today it is binary: mic track is "You", system
      track is "Others", so a four-person call collapses into one anonymous voice.
      `ScribeSettings` already carries `speaker_embedding_model_path` and
      `speaker_segmentation_model_path` — intent is there, unwired. `whisper-cli`'s `-tdrz`
      with a tinydiarize model is the cheap first step. Do BUG-1 first.

- [ ] **FEAT-9 — Live transcription during the meeting.** The `StreamingTranscriber` /
      `TranscriptEventSink` traits exist but `BatchReplayStreamingTranscriber` just replays
      a finished batch. Chunking audio every ~15 s and transcribing incrementally gives real
      progress instead of a spinner, and makes the live nudge pipeline actually live —
      which is what `nudges/` was built for.

- [ ] **FEAT-10 — Calendar-aware titles and attendees.** EventKit can name the meeting and
      list attendees at record time. Better `meetingTitle` than asking a 14B model to guess
      one from the transcript, and it gives the summariser real names to attach action items
      to instead of "Speaker".

- [ ] **FEAT-11 — Dictation vocabulary, separate from meetings.** `transcriber_vocabulary`
      is shared between both paths, but the useful terms differ — meetings need project
      nouns, dictation needs the names you type constantly. Also cap dictation duration:
      there is no upper bound today, so a forgotten toggle records until you notice.

- [ ] **FEAT-12 — Summarisation progress.** Map-reduce over a long transcript is *n*
      sequential model calls behind a single spinner, with a 600 s per-request timeout that
      ADR-003 documents having blown in production. Stream the completion, or just emit
      "section 3 of 7".

---

## 7. Engineering health

### What is genuinely good

- Every SQL statement uses bound parameters; `ensure_column` validates DDL identifiers
  against a static allow-list rather than formatting user input into a schema change.
- `validate_recording_file_stem` and `delete_retained_audio_file` do canonicalise-then-
  `starts_with` containment checks — path traversal properly closed, with tests.
- Subprocesses are spawned with argv arrays, never a shell. `open_system_settings_pane`
  allow-lists its panes.
- The bounded audio channel that drops-and-counts rather than blocking the realtime callback
  is exactly the right failure mode, and the reasoning is written down.
- ADR-001 through 005 and the architecture doc's §12 "known dead code" table are the kind of
  thing most codebases do not have. Keep doing it.

### Gaps

- [x] **HEALTH-1 — Lint was failing on `main`.** `biome check` flagged a formatting
      violation in `src/contracts.ts`. Fixed via `bun run lint:fix` during this review
      (uncommitted at time of writing).
- [ ] **HEALTH-2 — No CI at all.** No `.github/workflows`. `cargo test`, `bun test`, `tsc`
      and `biome` all exist and all pass — they are just not enforced anywhere.
- [ ] **HEALTH-3 — Frontend test coverage is 3 files, 126 lines**, all pure utilities.
      Nothing covers `summary-clipboard.ts` (where BUG-4 lives) or any component behaviour.
- [ ] **HEALTH-4 — `App.tsx` is 1,924 lines** and the architecture doc's justification
      ("not urgent") is wearing thin — it now holds recording, history, detail, trends,
      settings, onboarding, dictation and the confirm dialog. Split by view, not by
      component-library convention.
- [ ] **HEALTH-5 — Doc drift.** §8's dictation-privacy claim is now wrong (SEC-2). The
      `cloud_analysis_enabled` / `analyzer_provider` / `cloud_video_review_enabled` settings
      and `AnalyzerProvider::CloudOpenAi` survive from the abandoned cloud direction and are
      not in §12's dead-code table. Delete them or list them.

---

## 8. Suggested order of work

### This week — cheap, high return

1. BUG-4 escape the topic title (one line); BUG-3 use `ToSocketAddrs` so `localhost` works.
2. SEC-6 add `NSAppleEventsUsageDescription`, remove the unused camera key.
3. SEC-4 set the CSP; BUG-2 fix the chunked-decode slice.
4. Switch to `ggml-small.en-q5_1` — same size, same speed, better English (§4).
5. Fix the docs that are now wrong (HEALTH-5), and put lint + tests in CI (HEALTH-2).

### Next — the privacy posture

1. SEC-1 redact the detector log; add a diagnostic-logging toggle and a Clear Logs button
   that also covers PERF-5.
2. SEC-2 separate dictation retention with a "never store" option; `PRAGMA secure_delete`.
3. SEC-3 warn on non-loopback summariser hosts; SEC-5 mark the pasteboard concealed.
4. PERF-6 run retention on a timer, not only at launch.

### Then — the things users will actually notice

1. Model manager in Settings + large-v3-turbo (§4). Biggest quality jump available.
2. FEAT-1 search, and fix the 25-meeting history ceiling; FEAT-2 export.
3. BUG-1 track alignment — silently degrades both AEC and speaker attribution, so do it
   before any diarization work.
4. PERF-1 stream the AEC; PERF-2 virtualise the transcript. Both only matter for long
   meetings, but long meetings are the use case.
5. SEC-7 Developer ID and notarization — until this lands, every permission bug report is
   ambiguous.
