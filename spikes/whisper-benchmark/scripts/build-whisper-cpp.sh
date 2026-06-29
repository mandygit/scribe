#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  cat <<USAGE
Usage: $0 [--ref REF] [--repo-url URL] [--without-sdl2] [--jobs N]

Clones whisper.cpp into this spike folder and builds CLI benchmark tools.
Defaults:
  repo: https://github.com/ggml-org/whisper.cpp.git
  ref:  repository default branch
  sdl2: enabled, so whisper-stream can be built when SDL2 is installed
  jobs: 4
USAGE
}

repo_url="${WHISPER_CPP_REPO_URL:-https://github.com/ggml-org/whisper.cpp.git}"
repo_ref="${WHISPER_CPP_REF:-}"
build_sdl2=1
jobs=4

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ref) repo_ref="${2:-}"; shift 2 ;;
    --repo-url) repo_url="${2:-}"; shift 2 ;;
    --without-sdl2) build_sdl2=0; shift ;;
    --jobs) jobs="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[[ "$jobs" =~ ^[1-9][0-9]*$ ]] || fail "--jobs must be a positive integer"

require_command git
require_command cmake
ensure_spike_dirs
mkdir -p "$(dirname "$WHISPER_REPO_DIR")"

if [[ ! -d "$WHISPER_REPO_DIR/.git" ]]; then
  git clone --depth 1 "$repo_url" "$WHISPER_REPO_DIR"
else
  echo "Using existing clone: $WHISPER_REPO_DIR"
fi

if [[ -n "$repo_ref" ]]; then
  git -C "$WHISPER_REPO_DIR" fetch --depth 1 origin "$repo_ref"
  git -C "$WHISPER_REPO_DIR" checkout --detach FETCH_HEAD
fi

cmake_args=(-S "$WHISPER_REPO_DIR" -B "$WHISPER_BUILD_DIR" -DCMAKE_BUILD_TYPE=Release)
if [[ "$build_sdl2" -eq 1 ]]; then
  cmake_args+=(-DWHISPER_SDL2=ON)
fi

cmake "${cmake_args[@]}"
cmake --build "$WHISPER_BUILD_DIR" --config Release --parallel "$jobs"

cat <<DONE

Build complete.
Expected tools:
  $(whisper_binary whisper-bench 2>/dev/null || echo 'whisper-bench not found')
  $(whisper_binary whisper-cli 2>/dev/null || echo 'whisper-cli not found')
  $(whisper_binary whisper-stream 2>/dev/null || echo 'whisper-stream not found; rebuild with SDL2 enabled')
DONE
