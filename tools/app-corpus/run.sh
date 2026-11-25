#!/usr/bin/env bash
# Application-style program corpus vs Node (non-gating). Usage: tools/app-corpus/run.sh [run.py options, e.g. --only a,b --shard 1/2]
#   THAW                  thaw CLI (default <repo>/target/release/thaw)
#   NODE                  oracle (default node; Node 22)
#   APP_CORPUS_OUT        output dir (default $RUNNER_TEMP or thaw-tmp)/app-corpus-out: report.tsv, summary.txt, logs/
#   APP_CORPUS_BUILD_WRAP command prefix for `thaw build` / `thaw add`, e.g. "flock L systemd-run --user --scope -p MemoryMax=10G"
#   APP_CORPUS_BUILD_TIMEOUT / APP_CORPUS_RUN_TIMEOUT / APP_CORPUS_RUN_AS_MB   seconds / seconds / RLIMIT_AS MiB for the thaw run
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
OUT=${APP_CORPUS_OUT:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp}/app-corpus-out}
exec python3 -I "$HERE/run.py" --thaw "${THAW:-$REPO/target/release/thaw}" --node "${NODE:-node}" --out "$OUT" "$@"
