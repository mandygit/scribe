#!/usr/bin/env bash
# Places libspeexdsp (echo cancellation) at src-tauri/resources/lib/ so the
# packaged app can run AEC without Homebrew. The library only depends on
# libSystem, so a copy with a neutral install name is fully relocatable.
# aec.rs dlopens the bundled copy first and falls back to Homebrew paths.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="${REPO_ROOT}/src-tauri/resources/lib"
DEST="${DEST_DIR}/libspeexdsp.dylib"

if ! brew --prefix speexdsp >/dev/null 2>&1 || [[ ! -d "$(brew --prefix speexdsp)" ]]; then
  echo "error: Homebrew formula 'speexdsp' is not installed. Run: brew install speexdsp" >&2
  exit 1
fi

KEG="$(realpath "$(brew --prefix speexdsp)")"
mkdir -p "${DEST_DIR}"
cp "${KEG}/lib/libspeexdsp.1.dylib" "${DEST}"
chmod 644 "${DEST}"
install_name_tool -id "@loader_path/libspeexdsp.dylib" "${DEST}" 2>/dev/null

if otool -L "${DEST}" | tail -n +2 | grep -vE "/usr/lib|/System/|@loader_path" | grep .; then
  echo "error: bundled libspeexdsp references non-system libraries (see above)" >&2
  exit 1
fi

codesign --force --sign - "${DEST}" >/dev/null 2>&1

for license in COPYING LICENSE; do
  [[ -f "${KEG}/${license}" ]] && mkdir -p "${DEST_DIR}/licenses" \
    && cp "${KEG}/${license}" "${DEST_DIR}/licenses/speexdsp-${license}"
done

echo "bundled libspeexdsp ready: ${DEST} ($(basename "${KEG}"))"
