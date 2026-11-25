#!/usr/bin/env bash
# AOT-vs-Node probe harness (non-gating). Node is the oracle; the thaw CLI builds each generated program.
# Usage: tools/parity-aot/run.sh [--write-baseline] [OUT_DIR]
#   OUT_DIR              default $PARITY_AOT_OUT, else $PARITY_AOT_SCRATCH/out (base: $RUNNER_TEMP, $TMPDIR, thaw-tmp)
#   THAW                 thaw CLI (default <repo>/target/release/thaw); built as: thaw build <file.ts> -o <bin>
#   NODE                 oracle (default node, 22.18+ strips types; older node gets --experimental-strip-types)
#   PARITY_AOT_ENGINES   engines to run, default "thaw-js"
#   PARITY_AOT_JOBS      programs built in parallel (default min(4, cores/2))
#   PARITY_AOT_BUILD_WRAP  optional command prefix for each build, e.g. a systemd-run slice with MemoryMax
#   PARITY_AOT_RUN_TIMEOUT / PARITY_AOT_BUILD_TIMEOUT  seconds (defaults 20 / 900)
#   PARITY_AOT_FAMILIES  optional comma list: generate only these families (CI shards); coverage then skips its ratchet
#   PARITY_AOT_SUITE     ecmascript (default: gen_probes.py), boundary (gen_boundary.py) or modules (gen_modules.py: node:* exports, no coverage run)
#   PARITY_AOT_DTS_DIR   modules only: dump of the module d.ts files (THAW_DUMP_DTS_DIR, see README "Module surface")
#   PARITY_AOT_MODULES   modules only: module list file (default tools/parity-aot/modules.txt; a CI shard passes its subset)
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
ENGINES=${PARITY_AOT_ENGINES:-thaw-js}
JOBS=${PARITY_AOT_JOBS:-$(n=$(nproc 2>/dev/null || echo 2); n=$((n / 2)); [ "$n" -lt 1 ] && n=1; [ "$n" -gt 4 ] && n=4; echo "$n")}

mkdir -p "$OUT" || exit 2
# Only the directories this harness writes are cleared; anything else under OUT is left alone.
rm -rf "$OUT/programs" "$OUT/node" "$OUT/thaw-js" "$OUT/coverage.json"
SUITE=${PARITY_AOT_SUITE:-ecmascript}

if [ "$SUITE" = modules ]; then
  # Module surface: the d.ts dump is required. Coverage is not run here (CI computes it over the merged shards).
  [ -n "${PARITY_AOT_DTS_DIR:-}" ] || { echo "harness error: PARITY_AOT_DTS_DIR not set (module d.ts dump)" >&2; exit 2; }
  python3 "$HERE/gen_modules.py" "$OUT/programs" --dts "$PARITY_AOT_DTS_DIR" \
    --modules "${PARITY_AOT_MODULES:-$HERE/modules.txt}" > "$OUT/gen_modules.log" 2>&1 \
    || { echo "harness error: gen_modules.py failed (see $OUT/gen_modules.log)" >&2; exit 2; }
  python3 "$HERE/aot_run.py" --programs "$OUT/programs" --out "$OUT" --thaw "$THAW" --node "$NODE" \
    --engines "$ENGINES" --jobs "$JOBS" "${WRITE[@]}"
  rc=$?
  echo "summary: $OUT/summary.md"
  exit "$rc"
elif [ "$SUITE" = boundary ]; then
  # Boundary-value matrix (gen_boundary.py): node:* functions x argument shape x value origin. PARITY_AOT_MODULES is a
  # comma list of module keys here (default all), PARITY_AOT_BOUNDARY_PROFILE full|ci. Own known-gaps file.
  python3 "$HERE/gen_boundary.py" "$OUT/programs" ${PARITY_AOT_MODULES:+--modules "$PARITY_AOT_MODULES"} \
    --profile "${PARITY_AOT_BOUNDARY_PROFILE:-full}" > "$OUT/gen_boundary.log" 2>&1 \
    || { echo "harness error: gen_boundary.py failed (see $OUT/gen_boundary.log)" >&2; exit 2; }
  PARITY_AOT_RLIMIT_AS_MB=${PARITY_AOT_RLIMIT_AS_MB:-8192} python3 "$HERE/aot_run.py" --programs "$OUT/programs" \
    --out "$OUT" --thaw "$THAW" --node "$NODE" --engines "$ENGINES" --jobs "$JOBS" \
    --baseline "$HERE/known-gaps-boundary.json" "${WRITE[@]}"
  rc=$?
  python3 "$HERE/boundary_report.py" "$OUT" > "$OUT/boundary_report.log" 2>&1 || echo "boundary_report: exit $?" >&2
  echo "summary: $OUT/boundary.md"
  exit "$rc"
elif [ "$SUITE" != ecmascript ]; then
  echo "harness error: PARITY_AOT_SUITE must be ecmascript, modules or boundary" >&2; exit 2
fi

python3 "$HERE/gen_probes.py" "$OUT/programs" ${PARITY_AOT_FAMILIES:+--families "$PARITY_AOT_FAMILIES"} || { echo "harness error: gen_probes.py failed" >&2; exit 2; }
python3 "$HERE/aot_run.py" --programs "$OUT/programs" --out "$OUT" --thaw "$THAW" --node "$NODE" \
  --engines "$ENGINES" --jobs "$JOBS" "${WRITE[@]}"
rc=$?
# API coverage of the manifest (no thaw). Appends its table to summary.md; never changes the exit code. A subset
# run (PARITY_AOT_FAMILIES) skips the ratchet, since untouched ids outside the subset are expected.
COV=(--manifest "$OUT/programs/manifest.json" --out "$OUT" --summary "$OUT/summary.md")
if [ -n "${PARITY_AOT_FAMILIES:-}" ]; then COV+=(--no-ratchet); fi
python3 "$HERE/coverage.py" "${COV[@]}" || echo "coverage: exit $? (non-gating, see log)" >&2
echo "summary: $OUT/summary.md"
exit "$rc"
