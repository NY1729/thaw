#!/usr/bin/env bash
# Node-vs-AOT API surface matrix. Usage: tools/parity-aot/surface/run.sh [module ...]   (e.g. http stream globals; none = all)
#   THAW         thaw CLI (default /home/fedora/work/thaw-tmp/r126-target/release/thaw if present, else <repo>/target/release/thaw)
#   OUT          scratch dir (default /home/fedora/work/thaw-tmp/surface-out); results: $OUT/surface.{md,json}
#   JOBS         programs built in parallel (default 2); builds run under the heavytests slice (WRAP="" to disable)
#   PARITY_AOT_CACHE  build+run cache dir (default $OUT/cache; key = thaw binary hash + program text)
#   INV          reuse an existing inventory dir instead of running node (default: regenerate under $OUT/inventory)
# Non-gating. Exit 2 = harness error.
set -u
HERE=$(cd "$(dirname "$0")" && pwd); PA=$(dirname "$HERE"); REPO=$(cd "$PA/../.." && pwd)
OUT=${OUT:-/home/fedora/work/thaw-tmp/surface-out}
THAW=${THAW:-$( [ -x /home/fedora/work/thaw-tmp/r126-target/release/thaw ] && echo /home/fedora/work/thaw-tmp/r126-target/release/thaw || echo "$REPO/target/release/thaw")}
export PARITY_AOT_CACHE=${PARITY_AOT_CACHE:-$OUT/cache}
export PARITY_AOT_BUILD_WRAP=${WRAP-systemd-run --user --scope --quiet --slice=app-heavytests.slice -p OOMPolicy=kill -p MemoryMax=10G}
MODS=()
for m in "$@"; do case $m in globals|node:*) MODS+=("$m");; *) MODS+=("node:$m");; esac; done
INVD=${INV:-$OUT/inventory}
mkdir -p "$OUT"
[ -n "${INV:-}" ] || node "$HERE/inv.js" "$INVD" "${MODS[@]}" || exit 2
rm -rf "$OUT/programs" "$OUT/thaw-js" "$OUT/node"
python3 "$HERE/gen_surface.py" "$INVD" "$OUT/programs" "$@" || exit 2
python3 "$PA/aot_run.py" --programs "$OUT/programs" --out "$OUT" --thaw "$THAW" --jobs "${JOBS:-2}" \
  --baseline "$HERE/known-gaps.json" > "$OUT/aot_run.log" 2>&1
rc=$?; [ $rc -ne 2 ] || { tail -5 "$OUT/aot_run.log" >&2; exit 2; }
python3 "$HERE/report.py" "$OUT" "$HERE/baseline-counts.json" ${WRITE_BASELINE:+--write-baseline}
echo "report: $OUT/surface.md"
