#!/usr/bin/env bash
# Differential parity run: node is the oracle, thaw-js-node is the subject.
# Usage: tools/parity/run.sh [--write-baseline] [OUT_DIR]
#   OUT_DIR defaults to $PARITY_OUT or /home/fedora/work/thaw-tmp/parity-scratch/out
#   NODE          oracle binary (default: node)
#   THAW_JS_NODE  thaw-js runner binary (default: the ra8 prebuilt, see README)
#   PARITY_TIMEOUT  seconds per thaw corpus run (default 60)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
WRITE=()
if [ "${1:-}" = "--write-baseline" ]; then WRITE=(--write-baseline); shift; fi
OUT=${1:-${PARITY_OUT:-/home/fedora/work/thaw-tmp/parity-scratch/out}}
NODE=${NODE:-node}
THAW_JS_NODE=${THAW_JS_NODE:-/home/fedora/work/thaw-tmp/ra8/target/release/thaw-js-node}
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
