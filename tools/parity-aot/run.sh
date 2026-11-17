#!/usr/bin/env bash
# AOT-vs-Node probe harness (non-gating). Node is the oracle; the thaw CLI builds each generated program.
# Usage: tools/parity-aot/run.sh [--write-baseline] [OUT_DIR]
#   OUT_DIR              default $PARITY_AOT_OUT, else $PARITY_AOT_SCRATCH/out (base: $RUNNER_TEMP, $TMPDIR, thaw-tmp)
#   THAW                 thaw CLI (default <repo>/target/release/thaw); built as: thaw build <file.ts> -o <bin>
#   NODE                 oracle (default node, 22.18+ strips types; older node gets --experimental-strip-types)
#   PARITY_AOT_ENGINES   engines to run, default "thaw-js,quickjs" (each is THAW_JS_ENGINE for the build and run)
#   PARITY_AOT_JOBS      programs built in parallel (default min(4, cores/2))
#   PARITY_AOT_BUILD_WRAP  optional command prefix for each build, e.g. a systemd-run slice with MemoryMax
#   PARITY_AOT_RUN_TIMEOUT / PARITY_AOT_BUILD_TIMEOUT  seconds (defaults 20 / 900)
# Exit 0: harness ran (gaps and rejects are reported, not gated). Exit 2: harness error.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WRITE=()
if [ "${1:-}" = "--write-baseline" ]; then WRITE=(--write-baseline); shift; fi
BASE=${PARITY_AOT_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp/parity-aot-scratch}}
OUT=${1:-${PARITY_AOT_OUT:-$BASE/out}}
THAW=${THAW:-$REPO/target/release/thaw}
NODE=${NODE:-node}
ENGINES=${PARITY_AOT_ENGINES:-thaw-js,quickjs}
JOBS=${PARITY_AOT_JOBS:-$(n=$(nproc 2>/dev/null || echo 2); n=$((n / 2)); [ "$n" -lt 1 ] && n=1; [ "$n" -gt 4 ] && n=4; echo "$n")}

mkdir -p "$OUT" || exit 2
# Only the directories this harness writes are cleared; anything else under OUT is left alone.
rm -rf "$OUT/programs" "$OUT/node" "$OUT/thaw-js" "$OUT/quickjs"

python3 "$HERE/gen_probes.py" "$OUT/programs" || { echo "harness error: gen_probes.py failed" >&2; exit 2; }
python3 "$HERE/aot_run.py" --programs "$OUT/programs" --out "$OUT" --thaw "$THAW" --node "$NODE" \
  --engines "$ENGINES" --jobs "$JOBS" "${WRITE[@]}"
rc=$?
echo "summary: $OUT/summary.md"
exit "$rc"
