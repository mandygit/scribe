#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  cat <<USAGE
Usage: $0 [MODEL ...]

Downloads whisper.cpp models into:
  $MODELS_DIR

Default models: $(join_by_comma "${DEFAULT_MODELS[@]}")
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

require_command bash
require_command curl
ensure_spike_dirs

model_min_bytes() {
  case "$1" in
    base) echo 140000000 ;;
    base-q5_1) echo 50000000 ;;
    small) echo 450000000 ;;
    small-q5_1) echo 180000000 ;;
    *) echo 1 ;;
  esac
}

download_model() {
  local model="$1"
  local destination="$2"
  local part="${destination}.part"
  local url="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-${model}.bin"

  echo "Downloading $model from $url"
  curl --fail --location --retry 5 --retry-delay 3 --continue-at - \
    --output "$part" "$url"
  mv "$part" "$destination"
}

models=("$@")
if [[ ${#models[@]} -eq 0 ]]; then
  models=("${DEFAULT_MODELS[@]}")
fi

for model in "${models[@]}"; do
  destination="$(model_path "$model")"
  min_bytes="$(model_min_bytes "$model")"
  if [[ -s "$destination" ]] && [[ "$(stat -f '%z' "$destination")" -ge "$min_bytes" ]]; then
    echo "Skipping existing model: $destination"
    continue
  fi

  if [[ -s "$destination" ]]; then
    echo "Existing $destination is smaller than expected; resuming download."
    mv "$destination" "${destination}.part"
  fi

  download_model "$model" "$destination"

  if [[ ! -s "$destination" ]] || [[ "$(stat -f '%z' "$destination")" -lt "$min_bytes" ]]; then
    fail "downloaded model is smaller than expected: $destination"
  fi
done
