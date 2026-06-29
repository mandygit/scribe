# AEC Offline Quality Spike

Small Rust CLI for offline acoustic echo cancellation validation before live integration. It uses a small safe Rust wrapper around the SpeexDSP echo-cancellation API and `hound` for WAV I/O. The `aec-rs` crate was evaluated first, but its vendored SpeexDSP build was unsuitable for this local spike environment.

## Expected inputs

- `--reference`: system/playback audio WAV
- `--recorded`: microphone WAV containing playback echo
- `--output`: cleaned microphone WAV to write
- WAV format for this spike: 48 kHz, mono, signed 16-bit PCM

Default processing uses 10 ms frames (`--frame-size 480`) and a 200 ms Speex echo filter (`--filter-length 9600`).

## Prerequisites

The spike links against Homebrew SpeexDSP on macOS:

```sh
brew install speexdsp
```

The local Cargo config adds `/opt/homebrew/lib` to the linker search path. If SpeexDSP is installed elsewhere, update `.cargo/config.toml` inside this spike directory.

## Build and test

```sh
cd spikes/aec-offline
$HOME/.cargo/bin/cargo test
$HOME/.cargo/bin/cargo build
```

## Generate synthetic samples

```sh
cd spikes/aec-offline
mkdir -p samples
$HOME/.cargo/bin/cargo run -- \
  --generate-synthetic \
  --reference samples/reference.wav \
  --recorded samples/recorded.wav
```

The generated `recorded.wav` is a delayed multi-tap echo of `reference.wav`, useful for repeatable offline validation.

## Run cancellation

```sh
$HOME/.cargo/bin/cargo run -- \
  --reference samples/reference.wav \
  --recorded samples/recorded.wav \
  --output samples/clean.wav
```

You can also generate and process in one command:

```sh
$HOME/.cargo/bin/cargo run -- \
  --generate-synthetic \
  --reference samples/reference.wav \
  --recorded samples/recorded.wav \
  --output samples/clean.wav
```

## Metric and pass/fail

The CLI reports an ERLE-like reduction metric:

```text
estimated ERLE = 10 * log10(recorded_power / output_power)
```

For far-end-only validation clips, this approximates Echo Return Loss Enhancement. With near-end speech present it is only a comparable reduction metric, so use controlled clips when deciding pass/fail.

Pass criterion for this spike: **> 10 dB estimated ERLE**.
