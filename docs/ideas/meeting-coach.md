# Resonance

## Problem Statement

**How might we** give professionals private, real-time and post-meeting feedback on their communication skills — without any audio ever leaving their machine?

Meetings are where careers are made or stalled. How you speak — clarity, confidence, filler words, hedging, directness — shapes how people perceive your competence. Yet most professionals get zero feedback on their meeting communication unless they hire an executive coach at $200–500/hr. The tools that exist (Poised, Yoodli) are cloud-based, which means your sensitive business audio leaves your machine. For professionals in regulated industries, consulting, legal, or enterprise — that's a non-starter.

## Recommended Direction

A **privacy-first, local-only macOS desktop application** (Tauri + React) that acts as a personal AI communication coach. It captures your microphone and system audio during meetings, provides **real-time lightweight nudges** (filler words, pace, hedging) via a rule engine during the call, then runs deep **post-meeting LLM analysis** for context-aware coaching — with longitudinal tracking to show improvement over time.

**Why this wins:**

- **Privacy as a feature, not a limitation.** No audio, transcripts, or analysis ever leaves the user's machine. This is the #1 differentiator against every competitor in the space, and it's a hard requirement for enterprise/regulated buyers. Opt-in cloud LLM API (transcript text only, never raw audio) available for users who want higher quality analysis.
- **Two-layer feedback model.** Live nudges during the meeting (rule-based, no LLM) keep you aware in the moment. Deep LLM coaching after the meeting gives you specific, context-aware improvement suggestions. No other tool does both locally.
- **Context-aware coaching beats generic advice.** By capturing system audio (what others said), the tool can give feedback like *"When asked about the deadline, you hedged with 'probably sometime next month.' Try: 'I'll deliver by March 15th.'"* — not just *"avoid filler words."*
- **Longitudinal tracking creates a moat.** The longer you use it, the more valuable your data becomes. Weekly/monthly trend reports on filler word frequency, speaking pace, hedging patterns, and confidence indicators.

**Target user:** Knowledge workers in meetings daily — engineers, PMs, consultants, lawyers, executives — who want to improve how they communicate but don't get feedback today.

## Design Decisions (Resolved)

These decisions were stress-tested and locked in during design review:

| Dimension | Decision | Rationale |
|-----------|----------|-----------|
| **Product name** | Resonance | Capture what happened. Improve what you said. |
| **Platform (V1)** | macOS only | Ship faster, validate on own machine. Windows in V2+ |
| **Audio capture** | Mic + system audio, two-channel separation | Mic = "your voice," system audio = "others" for context |
| **Echo cancellation** | AEC via SpeexDSP or WebRTC AEC3, Rust FFI bindings | Existing C/C++ library — don't build from scratch. Needed because users may not always use headphones |
| **Transcription** | Streaming Whisper during meeting | Enables real-time nudges. Architected with Strategy pattern for easy swap between streaming/batch modes |
| **Real-time nudges** | Lightweight rule engine on live transcript (no LLM) | Filler word count, pace (WPM), hedging phrase detection, talk-time ratio — all deterministic code. Fits in 16GB RAM |
| **Post-meeting analysis** | Sequential: unload Whisper → load Ollama → LLM coaching | Deep qualitative analysis (tone, sentiment, specific suggestions with quotes). Sequential avoids memory contention |
| **Memory budget** | Fits in 16GB RAM | Streaming Whisper (~1-2GB) during meeting, then Ollama (~5-6GB) after. Never concurrent |
| **Analysis approach** | Hybrid | Quantitative metrics (filler count, WPM, talk-time) via deterministic code. Qualitative (hedging, tone, suggestions) via LLM |
| **Coaching dimensions** | Filler words, hedging language, speaking pace, talk-time ratio, sentiment/tone, specific improvement suggestions with quotes | Comprehensive from V1 |
| **Report format** | Numerical score card with expandable sections | Overall score + per-dimension scores. Each section expands to exact quotes + suggestions. Consumable in <30 seconds |
| **Audio retention** | Raw audio kept 7 days (user-configurable), transcripts + reports kept forever | Balances disk usage with ability to re-analyze |
| **Privacy model** | Default fully local. Opt-in cloud LLM sends transcript text only, never raw audio | Clear boundary: audio never leaves the machine under any circumstance |
| **Ollama dependency** | External prerequisite | App detects if Ollama is running, prompts user to install/start with clear instructions if not found |
| **Error handling** | Save raw audio always → retry transcription once → fall back to partial report with warning | Resilient pipeline, never lose meeting data |
| **App model** | macOS menubar app | Always accessible, never in the way. Small recording indicator during meetings |
| **Notification** | macOS native notification when analysis completes → click opens full report | Low-friction path to reviewing feedback |

## Key Assumptions to Validate

### Must Be True (Dealbreakers)

- [ ] **LLM coaching quality is genuinely useful, not generic.** The tool must reference specific words, specific moments, specific context from YOUR meeting — not produce advice anyone could Google. **Validation:** Build the analysis pipeline first, run it on 5 real meeting recordings, show the output to 3 colleagues. If they say "this is obvious," redesign the prompt engineering.
- [ ] **Whisper streaming on CPU is ≥90% accurate on VoIP meeting audio.** Compressed Teams/Meet audio with accents, crosstalk, and background noise is harder than clean recordings. **Validation:** Record 3 actual meetings, transcribe with `whisper-small` on CPU in streaming mode, measure WER (word error rate) against manual transcript.
- [ ] **System audio capture works reliably on macOS without admin privileges.** ScreenCaptureKit (macOS 13+) requires a one-time permission grant but no admin install. If corporate Macs block this permission, the "context" feature breaks. **Validation:** Test on a managed corporate Mac with standard restrictions.
- [ ] **AEC via FFI produces clean speaker separation.** If echo cancellation doesn't work well, the mic channel contains bleed from system audio, corrupting the "your voice" transcript. **Validation:** Record test meetings with speakers (no headphones), apply AEC, compare transcript quality before/after.

### Should Be True (Important)

- [ ] **Users actually review their feedback after meetings.** The tool is useless if summaries go unread. **Validation:** Design the post-meeting report to be consumable in <30 seconds. Track "report opened" metrics in V1 beta.
- [ ] **Privacy-first is a genuine buying factor, not just a nice-to-have.** Enterprise and regulated industries will pay for this. Individual consumers might not care enough. **Validation:** Talk to 5 professionals in regulated industries (legal, healthcare, finance) about whether they'd use a cloud-based speaking coach. If they say "sure, why not," the privacy angle is weaker than assumed.
- [ ] **Real-time nudges are helpful, not distracting.** Some users may find live feedback during a meeting anxiety-inducing. **Validation:** Test with 3 users, ask if nudges helped or distracted.

### Might Be True (Nice-to-have)

- [ ] **Rehearsal mode (practicing before meetings) is valuable.** This expands the tool beyond live meetings but may be a different user behavior. Validate after core meeting coaching is proven.

## MVP Scope

The MVP includes mic capture, system audio capture with AEC, streaming transcription, real-time nudges, and post-meeting LLM coaching analysis — all local.

### What's In (MVP)

| Feature | Description |
|---------|-------------|
| **Menubar app** | Tauri macOS menubar app. One-click start/stop recording. Recording indicator visible during meetings. |
| **Mic capture** | Record your voice via default microphone input. |
| **System audio capture** | Record meeting audio (what others are saying) via ScreenCaptureKit. |
| **Echo cancellation** | AEC via SpeexDSP/WebRTC AEC3 FFI — remove system audio bleed from mic channel. |
| **Streaming transcription** | Whisper.cpp streaming on CPU. Live transcript with timestamps during the meeting. |
| **Real-time nudges** | Rule-based during meeting: filler word counter, speaking pace (WPM), hedging phrase alerts, talk-time ratio. No LLM required. |
| **Post-meeting LLM analysis** | After meeting: unload Whisper, load Ollama, run deep coaching analysis. Sentiment, tone, specific improvement suggestions with exact quotes. |
| **Score card report** | Numerical overall score + per-dimension scores (filler, clarity, pace, talk-time, tone). Each section expandable to show quotes + suggestions. |
| **Meeting history** | SQLite database of past meetings, transcripts, scores, and coaching reports. |
| **Basic trends** | Filler word count, pace, and overall score over last N meetings. Charts in the app. |
| **Audio retention** | Raw audio kept 7 days (configurable). Transcripts and reports kept indefinitely. |

### Architecture (MVP)

```
┌──────────────────────────────────────────────────┐
│              Tauri 2.x Desktop App               │
│                                                  │
│  ┌──────────────────┐  ┌──────────────────────┐  │
│  │   React Frontend  │  │    Rust Backend       │  │
│  │   (Webview)       │  │                       │  │
│  │                   │  │  ┌─────────────────┐  │  │
│  │  - Menubar UI     │  │  │ Audio Pipeline  │  │  │
│  │  - Live nudges    │◄─►  │  - cpal (mic)   │  │  │
│  │  - Score card     │  │  │  - SCKit (sys)  │  │  │
│  │  - Trends charts  │  │  │  - AEC (FFI)    │  │  │
│  │  - Meeting history│  │  └────────┬────────┘  │  │
│  │  - Settings       │  │           │           │  │
│  │                   │  │  ┌────────▼────────┐  │  │
│  │  shadcn/ui        │  │  │  Transcriber    │  │  │
│  │  Framer Motion    │  │  │  (Strategy)     │  │  │
│  │  Recharts         │  │  │  - Streaming    │  │  │
│  │  Tailwind CSS     │  │  │  - Batch        │  │  │
│  └──────────────────┘  │  └────────┬────────┘  │  │
│                         │           │           │  │
│                         │  ┌────────▼────────┐  │  │
│                         │  │  Rule Engine    │  │  │
│                         │  │  (Real-time)    │  │  │
│                         │  │  - Filler count │  │  │
│                         │  │  - WPM calc     │  │  │
│                         │  │  - Hedge detect │  │  │
│                         │  └────────┬────────┘  │  │
│                         │           │           │  │
│                         │  ┌────────▼────────┐  │  │
│                         │  │  LLM Analyzer   │  │  │
│                         │  │  (Post-meeting) │  │  │
│                         │  │  - Ollama       │  │  │
│                         │  │  - Cloud API    │  │  │
│                         │  └────────┬────────┘  │  │
│                         │           │           │  │
│                         │  ┌────────▼────────┐  │  │
│                         │  │  SQLite (local) │  │  │
│                         │  └─────────────────┘  │  │
│                         └──────────────────────┘  │
└──────────────────────────────────────────────────┘
```

### Tech Stack

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| Desktop shell | **Tauri 2.x** | Rust backend for performance + low-level audio access, web frontend, ~10MB binary |
| Frontend | **React 18 + TypeScript (strict)** | Familiar stack, type-safe |
| UI components | **shadcn/ui + Tailwind CSS** | Accessible, customizable primitives. Full control over styling |
| Design system | **Dark mode + glassmorphism** | Linear/Raycast aesthetic. Premium, modern feel |
| Animations | **Framer Motion** | Smooth transitions, micro-interactions, spring physics. Makes the app feel polished |
| Charts | **Recharts** | React-native charting for trend visualizations |
| Audio capture | **cpal** (Rust) for mic, **ScreenCaptureKit** for macOS system audio | Platform-native, reliable |
| Echo cancellation | **SpeexDSP or WebRTC AEC3** via Rust FFI | Battle-tested C/C++ AEC, not built from scratch |
| Transcription | **whisper.cpp** (via whisper-rs bindings) | Runs on CPU, streaming mode, no Python dependency |
| AI analysis | **Ollama** (Llama 3.1 8B / Mistral 7B) locally, optional OpenAI/Claude API | Local-first, API as opt-in premium option |
| Storage | **SQLite** (via rusqlite) | Zero-config, local, fast. Meetings, transcripts, scores, reports |
| Build/bundle | **Tauri CLI** | macOS builds for V1 |

### UI Design Direction

| Aspect | Specification |
|--------|--------------|
| **Theme** | Dark mode with glassmorphism (frosted glass panels, blur effects, subtle transparency) |
| **Inspiration** | Linear, Raycast — clean, premium, developer-tool aesthetic |
| **Animations** | Smooth transitions + micro-animations via Framer Motion. Score counters animate up, sections expand with spring physics, page transitions feel fluid |
| **Typography** | Clean sans-serif (Inter or system font). Clear hierarchy: scores large, details readable |
| **Colors** | Dark background (#0a0a0a range), glass panels with blur, accent colors for scores (green/amber/red spectrum) |
| **Key screens** | Menubar dropdown (recording controls), Score card report, Meeting history list, Trends dashboard, Settings |
| **Interaction** | Minimal clicks to value. One-click start, auto-notification on completion, click to open report. Expandable sections for detail on demand |

## Phased Roadmap

| Phase | Scope | Key Deliverable |
|-------|-------|-----------------|
| **V1 (MVP)** | Full audio pipeline (mic + system + AEC) + streaming transcription + real-time nudges + post-meeting LLM coaching + score card + basic trends | Working macOS menubar app with live + post-meeting feedback |
| **V2** | Windows support (WASAPI audio backend) | Cross-platform |
| **V3** | Advanced longitudinal tracking + weekly/monthly improvement reports | Growth trajectory dashboard |
| **V4** | Rehearsal mode — practice presentations before meetings | Pre-meeting coaching |
| **V5** | Product hardening (auto-update, onboarding, licensing, polish) | Commercial-ready product |

## Not Doing (and Why)

- **Full speaker diarization (identifying individual remote speakers by name)** — Separating "you" from "everyone else" is easy (mic vs. system audio). Identifying that Speaker A is Sarah and Speaker B is Mike from a mixed audio stream requires voice enrollment or meeting integration. Not worth the complexity for personal coaching.
- **Cloud sync / team features** — This is a personal tool first. Team/enterprise features (manager dashboards, team communication analytics) are a V5+ concern and a different product entirely.
- **Mobile app** — Desktop-first. Meetings happen on laptops. Mobile adds platform complexity with no clear value.
- **Video analysis (body language, facial expressions)** — Video capture adds massive complexity, privacy concerns, and compute requirements. Audio-only coaching is the 80/20.
- **Calendar integration / auto-start** — Nice UX polish but not core value. Manual start/stop is fine for MVP. Add calendar integration in V3+.
- **Linux support** — macOS + Windows cover >95% of target market. Linux adds a third audio backend to maintain. Revisit based on demand.
- **Bundling Ollama inside the app** — Ollama is an external dependency. Bundling adds binary size and maintenance burden. Better to prompt users to install it.

## Open Questions

- **Licensing model (if commercializing):** One-time purchase (like Raycast) vs. subscription (for LLM API costs)? If fully local, one-time makes more sense. If offering cloud LLM as premium tier, subscription fits.
- **Whisper model size default:** `whisper-small` (461MB, ~93% accuracy) vs. `whisper-base` (142MB, ~88% accuracy) for streaming mode. Need to benchmark on real meeting audio with CPU streaming performance.
- **Ollama model recommendation:** Llama 3.1 8B runs well on most machines. Mistral 7B is faster. Need to test coaching prompt quality on both to pick a recommended default.
- **Real-time nudge UI:** Small floating widget overlay? Notification-style toasts? Subtle menubar badge updates? Need to prototype and test which is least distracting during meetings.
- **Scoring algorithm:** How to calculate the overall communication score. Weighted average of sub-scores? What weights? Need to define the formula and validate it feels fair and motivating, not punitive.
