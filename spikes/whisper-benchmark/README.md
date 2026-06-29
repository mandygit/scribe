# Whisper CLI Benchmark Spike

This spike contains scripts for benchmarking `whisper.cpp` CLI performance on local sample audio. It does not include sample audio or transcripts.

## What you provide

Place your own non-copyrighted benchmark inputs under `spikes/whisper-benchmark/samples/`:

- `sample.wav`, `sample.m4a`, or another ffmpeg-readable audio file
- Optional `sample.reference.txt` containing the ground-truth transcript for WER

Keep samples short while validating the pipeline. Do not commit private or copyrighted audio.

## Setup

From the repository root:

```bash
cd spikes/whisper-benchmark
./scripts/build-whisper-cpp.sh
./scripts/download-models.sh
```

`download-models.sh` downloads these models by default:

- `base`
- `base-q5_1`
- `small`
- `small-q5_1`

The clone, build artifacts, models, sample files, and benchmark outputs stay inside this spike folder.

## Normalize audio only

```bash
./scripts/normalize-audio.sh \
  --input samples/sample.wav \
  --output outputs/audio/sample.16k-mono.wav
```

Normalization uses ffmpeg to create 16 kHz mono signed 16-bit PCM WAV, matching the usual `whisper.cpp` CLI input expectation.

## Run benchmark

```bash
./scripts/run-benchmark.sh \
  --audio samples/sample.wav \
  --reference samples/sample.reference.txt
```

Without `--reference`, the script still records timing and memory metrics, but leaves WER blank.

Useful options:

```bash
./scripts/run-benchmark.sh \
  --audio samples/sample.wav \
  --models base,base-q5_1,small,small-q5_1 \
  --threads 4 \
  --output-dir outputs/my-run
```

## Optional whisper-stream smoke check

`whisper-stream` uses live microphone input through SDL2. Keep runs short and pass version-appropriate stream flags after `--`:

```bash
./scripts/run-stream.sh --model base -- --threads 4 --step 500 --length 5000
```

If your `whisper.cpp` version uses different stream flags, run the built binary with `--help` and pass the supported flags after `--`.

## Outputs

Each `run-benchmark.sh` invocation writes a timestamped directory under `outputs/` unless `--output-dir` is supplied:

```text
outputs/run-YYYYMMDD-HHMMSS/
  audio.16k-mono.wav
  logs/
    <model>.whisper-bench.log
    <model>.whisper-cli.log
  transcripts/
    <model>.txt
  manifest.csv
  summary.csv
```

`logs/*.log` contain raw command output plus `/usr/bin/time -l` metrics. `summary.csv` extracts:

- `real_seconds` from the `real` line
- `max_rss_bytes` from `maximum resident set size`
- `wer` via `jiwer.wer(reference, transcript)` when `--reference` is provided
- transcript and log paths for auditability

## Safety notes

- Scripts use `set -euo pipefail`.
- Generated outputs must be inside `spikes/whisper-benchmark/`.
- Scripts do not delete or overwrite files outside this spike folder.
- Model downloads and benchmark execution are user-triggered; this spike does not run them automatically.
