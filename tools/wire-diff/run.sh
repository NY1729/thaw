#!/usr/bin/env bash
# Usage: tools/wire-diff/run.sh [--scenes N] [--seed S] [--write-baseline] [--label L]
# Env: THAW (thaw binary, default thaw), CARGO_TARGET_DIR, WIRE_WORK (cache/work dir).
# Heavy: `thaw build` runs under the app-heavytests slice with a 10G cap.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
export WIRE_BUILD_WRAP="${WIRE_BUILD_WRAP:-systemd-run --user --scope --slice=app-heavytests.slice -p OOMPolicy=kill -p MemoryMax=10G -p ManagedOOMPreference=omit}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
exec python3 "$here/wire_diff.py" "$@"
