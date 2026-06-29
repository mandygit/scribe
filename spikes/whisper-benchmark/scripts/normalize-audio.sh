#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  cat <<USAGE
Usage: $0 --input AUDIO_FILE [--output OUTPUT_WAV]

Normalizes input audio to 16 kHz mono signed 16-bit PCM WAV with ffmpeg.
Output defaults to:
  $OUTPUTS_DIR/audio/<input-name>.16k-mono.wav
USAGE
}

input=""
output=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input|-i) input="${2:-}"; shift 2 ;;
    --output|-o) output="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[[ -n "$input" ]] || fail "--input is required"
[[ -f "$input" ]] || fail "input audio not found: $input"
require_command ffmpeg
ensure_spike_dirs

if [[ -z "$output" ]]; then
  stem="$(basename "$input")"
  stem="${stem%.*}"
  output="$OUTPUTS_DIR/audio/${stem}.16k-mono.wav"
fi
output="$(ensure_output_inside_spike "$output")"
mkdir -p "$(dirname "$output")"

ffmpeg -nostdin -hide_banner -loglevel warning -y \
  -i "$input" \
  -ar 16000 -ac 1 -c:a pcm_s16le \
  "$output"

printf '%s\n' "$output"
