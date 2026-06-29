#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  cat <<USAGE
Usage: $0 --audio AUDIO_FILE [--reference TRANSCRIPT.txt] [--models MODEL[,MODEL...]] [--output-dir DIR] [--threads N]

Runs whisper-bench and whisper-cli for each selected model, captures /usr/bin/time -l
logs, and writes a CSV summary. Default models: $(join_by_comma "${DEFAULT_MODELS[@]}")
USAGE
}

audio=""
reference=""
models_csv="$(join_by_comma "${DEFAULT_MODELS[@]}")"
run_output_dir="$OUTPUTS_DIR/run-$(date +%Y%m%d-%H%M%S)"
threads="$(sysctl -n hw.perflevel0.physicalcpu 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
python_bin="${VENV_BENCHMARK_PYTHON:-/Users/nama4008/Projects/meeting-coach/.venv-benchmark/bin/python}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --audio|-a) audio="${2:-}"; shift 2 ;;
    --reference|-r) reference="${2:-}"; shift 2 ;;
    --models|-m) models_csv="${2:-}"; shift 2 ;;
    --output-dir|-o) run_output_dir="${2:-}"; shift 2 ;;
    --threads|-t) threads="${2:-}"; shift 2 ;;
    --python) python_bin="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[[ -n "$audio" ]] || fail "--audio is required"
[[ -f "$audio" ]] || fail "audio not found: $audio"
[[ -x "$python_bin" ]] || fail "python not found or not executable: $python_bin"
if [[ -n "$reference" && ! -f "$reference" ]]; then
  fail "reference transcript not found: $reference"
fi

require_command /usr/bin/time
run_output_dir="$(ensure_output_inside_spike "$run_output_dir")"
mkdir -p "$run_output_dir/logs" "$run_output_dir/transcripts"

bench_bin="$(whisper_binary whisper-bench)"
cli_bin="$(whisper_binary whisper-cli)"
normalized_audio="$run_output_dir/audio.16k-mono.wav"
"$SCRIPT_DIR/normalize-audio.sh" --input "$audio" --output "$normalized_audio" >/dev/null

manifest="$run_output_dir/manifest.csv"
printf 'model,command,status,transcript_path,log_path\n' > "$manifest"

IFS=',' read -r -a models <<< "$models_csv"

run_timed() {
  local log_path="$1"
  shift
  set +e
  {
    printf 'command:'
    printf ' %q' "$@"
    printf '\n\n'
    /usr/bin/time -l "$@"
  } > "$log_path" 2>&1
  local status=$?
  set -e
  return "$status"
}

csv_field() {
  local value="$1"
  value="${value//\"/\"\"}"
  printf '"%s"' "$value"
}

record_row() {
  local model="$1"
  local command_name="$2"
  local status="$3"
  local transcript_path="$4"
  local log_path="$5"
  {
    csv_field "$model"; printf ','
    csv_field "$command_name"; printf ','
    csv_field "$status"; printf ','
    csv_field "$transcript_path"; printf ','
    csv_field "$log_path"; printf '\n'
  } >> "$manifest"
}

for model in "${models[@]}"; do
  model_file="$(model_path "$model")"
  [[ -s "$model_file" ]] || fail "missing model $model_file; run scripts/download-models.sh first"

  bench_log="$run_output_dir/logs/${model}.whisper-bench.log"
  if run_timed "$bench_log" "$bench_bin" -m "$model_file" -t "$threads"; then
    record_row "$model" whisper-bench 0 "" "$bench_log"
  else
    record_row "$model" whisper-bench "$?" "" "$bench_log"
  fi

  transcript_base="$run_output_dir/transcripts/${model}"
  cli_log="$run_output_dir/logs/${model}.whisper-cli.log"
  if run_timed "$cli_log" "$cli_bin" -m "$model_file" -f "$normalized_audio" -t "$threads" -otxt -of "$transcript_base"; then
    record_row "$model" whisper-cli 0 "${transcript_base}.txt" "$cli_log"
  else
    record_row "$model" whisper-cli "$?" "${transcript_base}.txt" "$cli_log"
  fi
done

summary="$run_output_dir/summary.csv"
summary_args=(--manifest "$manifest" --output "$summary")
if [[ -n "$reference" ]]; then
  summary_args+=(--reference "$reference")
fi
"$python_bin" "$SCRIPT_DIR/analyze-results.py" "${summary_args[@]}" >/dev/null

cat <<DONE
Benchmark run complete.
Output directory: $run_output_dir
Summary: $summary
Raw /usr/bin/time -l logs: $run_output_dir/logs
Transcripts: $run_output_dir/transcripts
DONE
