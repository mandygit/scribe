# ADR-003: Default local summarizer model, sizing, and request timeout

## Status

Accepted

## Date

2026-07-01

## Context

Scribe generates meeting notes by sending the transcript to a local
OpenAI-compatible chat server (`summarizer::LmStudioSummarizer` over
`ChatCompletion`), with LM Studio as the default provider. Transcripts over
`SINGLE_SHOT_CHAR_BUDGET` (36,000 chars) are map-reduced into
`CHUNK_CHAR_TARGET`-sized (12,000 char) windows, each requiring its own chat
completion call plus a final reduce call. Every HTTP call to the model server
has a fixed `REQUEST_TIMEOUT` of 600 seconds (`summarizer/mod.rs`).

The default model was `google/gemma-4-26b-a4b-qat` (26B parameters). On an
real 81-minute meeting (377 transcript segments, map-reduced into ~12
chunks), summarization was attempted three times, each restarting the LM
Studio server and reloading the model from scratch. The LM Studio server log
showed individual chat completions taking one to several minutes each, with
the third attempt's final reduce call still generating tokens *after* our
client had already given up. The recorded failure was:

```
summarizer_unavailable — Could not reach the local model server —
Resource temporarily unavailable (os error 35)
```

`os error 35` is `EAGAIN`, which is exactly what Rust's `TcpStream` read
times out as once `set_read_timeout` fires. In other words: the 26B model was
not broken, it was simply too slow for this hardware to finish a map/reduce
call inside the 600-second window, so the client-side timeout fired while
LM Studio kept computing in the background, unaware the caller had moved on.

Separately, this surfaced that `DEFAULT_SUMMARIZER_MODEL` names a specific
model string that must exactly match what the local server reports back for
`/v1/chat/completions` — for LM Studio, that's whatever id `lms ls` shows for
the downloaded model, which does not always match the Hugging Face repo name
verbatim (e.g. `lmstudio-community/Qwen3-14B-MLX-4bit` on disk is exposed as
`qwen3-14b-mlx`, not `qwen/qwen3-14b` or the repo slug).

## Decision

- Changed the default summarizer model from `google/gemma-4-26b-a4b-qat` (26B)
  to **`qwen3-14b-mlx`** (Qwen3-14B, MLX 4-bit quantization, ~8-9GB of
  weights) — a size the target hardware (Apple Silicon, 36GB unified memory)
  can run well within available headroom, instead of a model that appears to
  have been thrashing.
- Appended a `/no_think` directive to the end of every summarizer prompt
  (single-shot, map, reduce — see `NO_THINK_DIRECTIVE` in
  `summarizer/mod.rs`). Qwen3's chat template runs a hidden chain-of-thought
  pass by default; for a deterministic extraction task like meeting notes
  this is pure latency with no quality benefit, and it directly eats into the
  same 600-second timeout budget that caused the incident above. `/no_think`
  is Qwen3's own documented in-prompt switch and works regardless of whether
  the serving backend (LM Studio's MLX engine here, but also llama.cpp-backed
  servers) exposes an explicit `enable_thinking` API parameter.
- Left `REQUEST_TIMEOUT` at 600 seconds rather than raising it further. A
  right-sized model finishing chunks in tens of seconds has ample margin
  under 600s; raising the timeout would mask a slow-model problem instead of
  fixing it, and would make a genuinely hung request take even longer to
  surface as an error.
- Left the model configurable (`update_summarizer_settings`,
  `list_summarizer_models` against `/v1/models`) — this default is a
  starting point for the common case, not a hard requirement. Users with
  less RAM should size down further (see Consequences); users with more
  headroom (e.g. an M-series Max/Ultra) can reasonably go bigger.

## Alternatives Considered

### Raise `REQUEST_TIMEOUT` instead of changing the model

- Pros: Zero code change to the summarizer prompts/model default; lets any model "just eventually finish."
- Cons: Doesn't fix the actual problem (a model too large for the hardware to serve interactively), turns a config mistake into a multi-minute-per-chunk tax on every summary, and delays failure detection when something is *actually* hung (crashed model server, deadlocked request) rather than just slow.
- Rejected: treats a capacity problem as a patience problem.

### Keep the 26B model but reduce map-reduce chunk size to speed up each call

- Pros: No model swap needed.
- Cons: Smaller chunks mean more chunks, which means more round trips and more chances to load-then-unload the model between attempts (the 26B model's load lifecycle managed by `LmStudioLifecycle` was already a meaningful chunk of the wall-clock time in the failing runs). Doesn't address the fundamental issue that the model itself was slow per-token on this hardware.
- Rejected: shrinks the symptom, not the cause.

### Qwen2.5-7B-Instruct instead of Qwen3-14B

- Pros: Even smaller (~4.5GB Q4), even faster per chunk, no thinking-mode gotcha to work around.
- Cons: Noticeably lower quality on nuanced multi-topic meetings and structured JSON extraction than 14B, and the target hardware (36GB RAM, M3 Pro) has comfortable headroom for 14B specifically.
- Rejected as the *default* for this hardware profile, but remains a reasonable smaller fallback recommendation for lower-RAM machines (documented in the technical architecture doc's Summarization section).

## Consequences

- `DEFAULT_SUMMARIZER_MODEL` is a string match against the local server's
  reported model id, not a Hugging Face repo slug — whoever changes this
  constant again must verify the exact id via `lms ls` (LM Studio) or
  `ollama list` / `/v1/models` (Ollama/Custom) after downloading, not guess
  from the download page's name. Getting this wrong fails fast and loud
  (`summarizer_model_not_configured` / a 404 from the server) rather than
  silently, so it's a cheap mistake to catch — but still worth writing down.
- Model sizing is a hardware-dependent choice, not a universal constant. This
  ADR's 14B pick assumes ~36GB+ unified memory. Anyone packaging Scribe for a
  16GB or 24GB Mac should size down (Qwen3-8B or similar Q4 builds) rather
  than reuse this default blindly.
- `/no_think` is specific to Qwen-family models. If the default model is ever
  changed to a different family, revisit whether `NO_THINK_DIRECTIVE` still
  makes sense (harmless no-op text for a model that doesn't recognize the
  token, but worth confirming) or whether that family has its own
  thinking-mode switch to reach for instead.
