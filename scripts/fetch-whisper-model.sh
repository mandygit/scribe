#!/usr/bin/env bash
# Places the whisper model that ships inside Scribe.app at
# src-tauri/resources/models/ggml-small-q5_1.bin.
#
# ggml-small-q5_1 (~190 MB) is the verified default: small fixes proper-noun
# recognition that base garbles, and the q5_1 quantization keeps the installer
# size reasonable. Reuses a local copy when one exists, otherwise downloads
# from the official whisper.cpp Hugging Face repo, and always verifies the
# checksum.
set -euo pipefail

MODEL_NAME="ggml-small-q5_1.bin"
MODEL_SHA256="ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL_NAME}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="${REPO_ROOT}/src-tauri/resources/models"
DEST="${DEST_DIR}/${MODEL_NAME}"
LOCAL_COPY="${REPO_ROOT}/spikes/whisper-benchmark/models/${MODEL_NAME}"

checksum_ok() {
  echo "${MODEL_SHA256}  $1" | shasum -a 256 --check --status
}

if [[ -f "${DEST}" ]] && checksum_ok "${DEST}"; then
  echo "bundled whisper model already in place: ${DEST}"
  exit 0
fi

mkdir -p "${DEST_DIR}"

if [[ -f "${LOCAL_COPY}" ]] && checksum_ok "${LOCAL_COPY}"; then
  echo "copying local model from ${LOCAL_COPY}"
  cp "${LOCAL_COPY}" "${DEST}"
else
  echo "downloading ${MODEL_NAME} (~190 MB) from Hugging Face..."
  curl -fL --retry 3 -o "${DEST}.partial" "${MODEL_URL}"
  mv "${DEST}.partial" "${DEST}"
fi

if ! checksum_ok "${DEST}"; then
  echo "error: ${DEST} failed checksum verification" >&2
  rm -f "${DEST}"
  exit 1
fi

echo "bundled whisper model ready: ${DEST}"
