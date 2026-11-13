#!/usr/bin/env bash
# Differential parity run: node is the oracle, thaw-js-node is the subject.
# Usage: tools/parity/run.sh [--write-baseline] [OUT_DIR]
#   OUT_DIR defaults to $PARITY_OUT, else $PARITY_SCRATCH/out (PARITY_SCRATCH
#   defaults to $RUNNER_TEMP, then $TMPDIR, then the thaw-tmp dev path)
#   NODE          oracle binary (default: node)
#   THAW_JS_NODE  thaw-js runner binary (default: repo target/release, else the ra8 prebuilt)
#   PARITY_TIMEOUT  seconds per thaw corpus run (default 60)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WRITE=()
if [ "${1:-}" = "--write-baseline" ]; then WRITE=(--write-baseline); shift; fi
BASE=${PARITY_SCRATCH:-${RUNNER_TEMP:-${TMPDIR:-/home/fedora/work/thaw-tmp/parity-scratch}}}
OUT=${1:-${PARITY_OUT:-$BASE/out}}
NODE=${NODE:-node}
default_thaw="$REPO/target/release/thaw-js-node"
[ -x "$default_thaw" ] || default_thaw=/home/fedora/work/thaw-tmp/ra8/target/release/thaw-js-node
THAW_JS_NODE=${THAW_JS_NODE:-$default_thaw}
TIMEOUT=${PARITY_TIMEOUT:-60}

mkdir -p "$OUT/node" "$OUT/thaw"
rm -f "$OUT"/node/*.txt "$OUT"/node/*.err "$OUT"/thaw/*.txt "$OUT"/thaw/*.err "$OUT"/thaw/*.rc

corpora=("$HERE/surface.js" "$HERE"/corpus/*.js)
status=0
for f in "${corpora[@]}"; do
  n=$(basename "$f" .js)
  if ! "$NODE" "$f" > "$OUT/node/$n.txt" 2> "$OUT/node/$n.err"; then
    echo "harness error: oracle (node) failed on $n, see $OUT/node/$n.err" >&2
    exit 2
  fi
  timeout "$TIMEOUT" "$THAW_JS_NODE" "$f" > "$OUT/thaw/$n.txt" 2> "$OUT/thaw/$n.err"
  echo $? > "$OUT/thaw/$n.rc"
done

python3 "$HERE/diff.py" \
  --node-dir "$OUT/node" --thaw-dir "$OUT/thaw" \
  --baseline "$HERE/known-gaps.json" \
  --skip "$HERE/skip.txt" \
  --gaps "$OUT/gaps.json" \
  "${WRITE[@]}"
