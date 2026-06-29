#!/usr/bin/env bash
set -euo pipefail

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHISPER_REPO_DIR="${SPIKE_DIR}/external/whisper.cpp"
WHISPER_BUILD_DIR="${WHISPER_REPO_DIR}/build"
MODELS_DIR="${SPIKE_DIR}/models"
SAMPLES_DIR="${SPIKE_DIR}/samples"
OUTPUTS_DIR="${SPIKE_DIR}/outputs"
DEFAULT_MODELS=(base base-q5_1 small small-q5_1)

fail() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

ensure_spike_dirs() {
  mkdir -p "$MODELS_DIR" "$SAMPLES_DIR" "$OUTPUTS_DIR"
}

absolute_path() {
  local path="$1"
  if [[ "$path" = /* ]]; then
    printf '%s\n' "$path"
  else
    printf '%s/%s\n' "$(pwd)" "$path"
  fi
}

ensure_output_inside_spike() {
  local path
  local python_bin="${VENV_BENCHMARK_PYTHON:-/Users/nama4008/Projects/meeting-coach/.venv-benchmark/bin/python}"
  local python_command=()
  if [[ -x "$python_bin" ]]; then
    python_command=("$python_bin")
  elif command -v python3 >/dev/null 2>&1; then
    python_command=(python3)
  else
    fail "python is required to validate output paths"
  fi

  path="$("${python_command[@]}" -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).expanduser().resolve(strict=False))' "$(absolute_path "$1")")"
  case "$path" in
    "$SPIKE_DIR"|"$SPIKE_DIR"/*) printf '%s\n' "$path" ;;
    *) fail "output path must be inside $SPIKE_DIR: $1" ;;
  esac
}

model_path() {
  local model="$1"
  printf '%s/ggml-%s.bin\n' "$MODELS_DIR" "$model"
}

whisper_binary() {
  local name="$1"
  local candidates=(
    "$WHISPER_BUILD_DIR/bin/$name"
    "$WHISPER_BUILD_DIR/examples/$name/$name"
    "$WHISPER_BUILD_DIR/examples/${name#whisper-}/${name#whisper-}"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  if command -v "$name" >/dev/null 2>&1; then
    command -v "$name"
    return 0
  fi

  fail "could not find built whisper.cpp binary '$name'; run scripts/build-whisper-cpp.sh first"
}

model_downloader() {
  local candidates=(
    "$WHISPER_REPO_DIR/models/download-ggml-model.sh"
    "/opt/homebrew/opt/whisper-cpp/share/whisper-cpp/models/download-ggml-model.sh"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  fail "model downloader not found; run scripts/build-whisper-cpp.sh or install Homebrew whisper-cpp"
}

join_by_comma() {
  local IFS=,
  echo "$*"
}
