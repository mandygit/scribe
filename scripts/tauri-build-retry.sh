#!/usr/bin/env bash
# Runs `tauri build` and retries when the toolchain is killed rather than
# failing.
#
# Microsoft Defender for Endpoint (wdav) on this build machine SIGKILLs
# optimized clang/rustc work non-deterministically - the same invocation that
# dies will usually succeed moments later. Observed 2026-08-28 across one
# release link, one rustc ICE whose backtrace ended in __pthread_cond_wait,
# several swiftc links, and a debug link, all while identical retries passed.
#
# This is a workaround, not the fix. The fix is a Defender exclusion for the
# build directories, which needs admin rights on the machine. Without it a
# release build is a coin flip, and a coin flip is not a release process.
#
# Deliberately narrow: only kills and compiler panics are retried. A real
# compile error fails immediately, because retrying one wastes minutes and
# buries the message that matters.
set -uo pipefail

MAX_ATTEMPTS="${TAURI_BUILD_ATTEMPTS:-5}"
LOG="$(mktemp -t scribe-tauri-build)"
trap 'rm -f "${LOG}"' EXIT

for attempt in $(seq 1 "${MAX_ATTEMPTS}"); do
  if [[ "${attempt}" -gt 1 ]]; then
    echo "==> retrying tauri build (attempt ${attempt}/${MAX_ATTEMPTS}) after a toolchain kill" >&2
  fi
  if bunx tauri build "$@" 2>&1 | tee "${LOG}"; then
    exit 0
  fi
  if ! grep -qE "signal: 9 \(SIGKILL\)|the compiler unexpectedly panicked|Killed: 9" "${LOG}"; then
    echo "==> build failed for a real reason, not a kill - not retrying" >&2
    exit 1
  fi
done

echo "==> gave up after ${MAX_ATTEMPTS} attempts, all killed by the endpoint agent." >&2
echo "==> Add a Defender exclusion for target/ and src-tauri/target/ to fix this properly." >&2
exit 1
