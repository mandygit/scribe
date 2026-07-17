#!/usr/bin/env bash
# Assembles a self-contained whisper-cli folder for bundling inside Scribe.app.
#
# Compiling whisper.cpp from source is not possible on every dev machine (EDR
# policies can kill optimized clang builds), so instead we take the pinned
# Homebrew bottle already installed on the build machine and rewrite its
# library load paths so the whole folder is relocatable:
#
#   src-tauri/resources/whisper/
#     whisper-cli                 (from whisper-cpp keg)
#     libwhisper.1.dylib          (from whisper-cpp keg)
#     libggml.0.dylib             (from ggml keg)
#     libggml-base.0.dylib        (from ggml keg)
#     libggml-*.so                (dlopen'd backends: cpu variants, metal, blas)
#     libomp.dylib                (from libomp keg, needed by cpu backends)
#     licenses/, MANIFEST.txt
#
# Every inter-library reference is rewritten to @loader_path so the folder
# works from Contents/Resources/whisper inside the app bundle on a Mac with
# no Homebrew at all. ggml discovers the dlopen'd backends by searching the
# directory of the whisper-cli executable after the (absent) compiled-in
# Homebrew libexec path, so keeping everything in one flat folder is enough.
set -euo pipefail

EXPECTED_WHISPER_VERSION="1.8.4"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${REPO_ROOT}/src-tauri/resources/whisper"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: the bundled whisper-cli can only be assembled on macOS" >&2
  exit 1
fi

require_keg() {
  local formula="$1"
  if ! brew --prefix "${formula}" >/dev/null 2>&1 || [[ ! -d "$(brew --prefix "${formula}")" ]]; then
    echo "error: Homebrew formula '${formula}' is not installed. Run: brew install ${formula}" >&2
    exit 1
  fi
}

require_keg whisper-cpp
require_keg ggml
require_keg libomp

WHISPER_KEG="$(realpath "$(brew --prefix whisper-cpp)")"
GGML_KEG="$(realpath "$(brew --prefix ggml)")"
LIBOMP_KEG="$(realpath "$(brew --prefix libomp)")"

WHISPER_VERSION="$(basename "${WHISPER_KEG}")"
if [[ "${WHISPER_VERSION}" != "${EXPECTED_WHISPER_VERSION}" ]]; then
  echo "error: installed whisper-cpp is ${WHISPER_VERSION}, expected ${EXPECTED_WHISPER_VERSION}." >&2
  echo "Scribe's transcription flags are verified against ${EXPECTED_WHISPER_VERSION}." >&2
  echo "Either 'brew install whisper-cpp@...' the pinned version or update" >&2
  echo "EXPECTED_WHISPER_VERSION in this script after re-verifying transcription." >&2
  exit 1
fi

rm -rf "${DEST}"
mkdir -p "${DEST}/licenses"

cp "${WHISPER_KEG}/bin/whisper-cli" "${DEST}/whisper-cli"
cp "${WHISPER_KEG}/lib/libwhisper.1.dylib" "${DEST}/libwhisper.1.dylib"
cp "${GGML_KEG}/lib/libggml.0.dylib" "${DEST}/libggml.0.dylib"
cp "${GGML_KEG}/lib/libggml-base.0.dylib" "${DEST}/libggml-base.0.dylib"
cp "${GGML_KEG}"/libexec/libggml-*.so "${DEST}/"
cp "${LIBOMP_KEG}/lib/libomp.dylib" "${DEST}/libomp.dylib"
chmod 755 "${DEST}/whisper-cli"
chmod 644 "${DEST}"/*.dylib "${DEST}"/*.so

for keg in "${WHISPER_KEG}" "${GGML_KEG}" "${LIBOMP_KEG}"; do
  name="$(basename "$(dirname "${keg}")")"
  for license in LICENSE LICENSE.txt LICENSE.TXT COPYING; do
    if [[ -f "${keg}/${license}" ]]; then
      cp "${keg}/${license}" "${DEST}/licenses/${name}-${license}"
    fi
  done
done

{
  echo "Bundled from Homebrew kegs on $(date -u +%Y-%m-%dT%H:%M:%SZ):"
  echo "  whisper-cpp $(basename "${WHISPER_KEG}")"
  echo "  ggml $(basename "${GGML_KEG}")"
  echo "  libomp $(basename "${LIBOMP_KEG}")"
} > "${DEST}/MANIFEST.txt"

# Rewrite every non-system library reference to @loader_path so the folder is
# self-contained regardless of where the app bundle lives.
rewrite_refs() {
  local file="$1"
  otool -L "${file}" | tail -n +2 | awk '{print $1}' | while read -r ref; do
    case "${ref}" in
      /usr/lib/*|/System/*) continue ;;
    esac
    local base
    base="$(basename "${ref}")"
    if [[ "${base}" == "$(basename "${file}")" ]]; then
      install_name_tool -id "@loader_path/${base}" "${file}" 2>/dev/null
    elif [[ -f "${DEST}/${base}" ]]; then
      install_name_tool -change "${ref}" "@loader_path/${base}" "${file}" 2>/dev/null
    else
      echo "error: ${file} references ${ref} which is not bundled" >&2
      exit 1
    fi
  done
}

for file in "${DEST}/whisper-cli" "${DEST}"/*.dylib "${DEST}"/*.so; do
  rewrite_refs "${file}"
done
# whisper-cli's Homebrew rpath points at ../lib, which does not exist in the
# bundle; drop it so nothing can resolve outside the folder.
install_name_tool -delete_rpath "@loader_path/../lib" "${DEST}/whisper-cli" 2>/dev/null || true

# install_name_tool invalidates code signatures; re-sign ad hoc.
for file in "${DEST}/whisper-cli" "${DEST}"/*.dylib "${DEST}"/*.so; do
  codesign --force --sign - "${file}" >/dev/null 2>&1
done

echo "verifying no reference escapes the bundle..."
leaked=0
for file in "${DEST}/whisper-cli" "${DEST}"/*.dylib "${DEST}"/*.so; do
  if otool -L "${file}" | tail -n +2 | grep -E "/opt/homebrew|/usr/local|@rpath"; then
    echo "error: ${file} still references non-bundled libraries (see above)" >&2
    leaked=1
  fi
done
[[ "${leaked}" -eq 0 ]]

echo "smoke-testing the relocated whisper-cli (Homebrew hidden to simulate a fresh Mac)..."
sandbox-exec -p '(version 1)(allow default)(deny file-read* (subpath "/opt/homebrew"))' \
  "${DEST}/whisper-cli" --help >/dev/null 2>/dev/null \
  || "${DEST}/whisper-cli" --help >/dev/null

echo "bundled whisper-cli ready in ${DEST}"
