#!/usr/bin/env bash
# Node-vs-thaw failure-behaviour harness (non-gating). Usage: tools/failure-conformance/run.sh [--write-baseline] [OUT_DIR] [run.py args]
#   THAW           thaw CLI (default <repo>/target/release/thaw)     NODE  oracle (default node, 22.x)
#   FC_BUILD_WRAP  command prefix for each `thaw build` (e.g. a systemd-run slice with MemoryMax)
#   FC_JOBS        parallel builds/runs (default 3)    FC_BUILD_BUDGET max isolation builds per program (0 = unlimited)    FC_RUN_TIMEOUT / FC_BUILD_TIMEOUT  seconds (20 / 900)
#   FC_RLIMIT_AS   address-space cap for thaw binaries (default 8 GiB)
# Exit 0: harness ran (gaps are findings, not failures). Exit 2: harness error.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WRITE=()
if [ "${1:-}" = "--write-baseline" ]; then WRITE=(--write-baseline); shift; fi
OUT=${FC_OUT:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp}/failure-conformance-out}
case "${1:-}" in ""|-*) ;; *) OUT=$1; shift ;; esac
exec python3 "$HERE/run.py" --out "$OUT" --thaw "${THAW:-$REPO/target/release/thaw}" --node "${NODE:-node}" "${WRITE[@]}" "$@"
