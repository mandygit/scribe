#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  cat <<USAGE
Usage: $0 --model MODEL [--output-dir DIR] -- [whisper-stream args]

Runs whisper-stream with /usr/bin/time -l capture. This is optional and intended
for short live microphone checks, not long benchmarks.
Example:
  $0 --model base -- --threads 4 --step 500 --length 5000
USAGE
}

model="base"
run_output_dir="$OUTPUTS_DIR/stream-$(date +%Y%m%d-%H%M%S)"
extra_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model|-m) model="${2:-}"; shift 2 ;;
    --output-dir|-o) run_output_dir="${2:-}"; shift 2 ;;
    --) shift; extra_args=("$@"); break ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument before --: $1" ;;
  esac
done

require_command /usr/bin/time
stream_bin="$(whisper_binary whisper-stream)"
model_file="$(model_path "$model")"
[[ -s "$model_file" ]] || fail "missing model $model_file; run scripts/download-models.sh $model first"
run_output_dir="$(ensure_output_inside_spike "$run_output_dir")"
mkdir -p "$run_output_dir"

log_path="$run_output_dir/whisper-stream.log"
set +e
/usr/bin/time -l "$stream_bin" -m "$model_file" "${extra_args[@]}" > "$log_path" 2>&1
status=$?
set -e

echo "status=$status"
echo "log=$log_path"
exit "$status"
