#!/usr/bin/env bash
# JS-engine microbenchmarks: times every benchmarks/jsengine/*.js under thaw-js.
#
# usage: run.sh [-n RUNS] [-o OUT.tsv] [-h]
# env:   THAW_JS_RUN       prebuilt plain-script runner, invoked as `$THAW_JS_RUN file.js`
#                          (default: build thaw-js-node in release and use it)
#        CARGO_TARGET_DIR  honoured by the default build
#
# Output: TSV `benchmark  thaw_js_ms`. Each script prints one checksum line, shown on stderr.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RUNS=3
OUT=""

usage() { sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }
while getopts "n:o:h" opt; do
  case $opt in
    n) RUNS=$OPTARG ;;
    o) OUT=$OPTARG ;;
    h) usage 0 ;;
    *) usage 2 ;;
  esac
done
if ! [[ $RUNS =~ ^[1-9][0-9]*$ ]]; then
  echo "run.sh: -n must be a positive integer" >&2
  exit 2
fi

if [[ -z "${THAW_JS_RUN:-}" ]]; then
  echo "building thaw-js-node (release)..." >&2
  (cd "$REPO" && cargo build --release -p thaw-js --bin thaw-js-node >&2)
  THAW_JS_RUN="${CARGO_TARGET_DIR:-$REPO/target}/release/thaw-js-node"
fi
[[ -x "$THAW_JS_RUN" ]] || { echo "run.sh: not executable: $THAW_JS_RUN" >&2; exit 1; }

# time_best BIN FILE: best-of-RUNS wall ms into BEST_MS, last stdout into LAST_OUT.
time_best() {
  local bin=$1 file=$2 i s e ms best=""
  for ((i = 0; i < RUNS; i++)); do
    s=${EPOCHREALTIME/[.,]/}
    if ! LAST_OUT=$("$bin" "$file"); then
      echo "run.sh: $bin failed on $file" >&2
      return 1
    fi
    e=${EPOCHREALTIME/[.,]/}
    ms=$(( (e - s) / 1000 ))
    if [[ -z $best || $ms -lt $best ]]; then best=$ms; fi
  done
  BEST_MS=$best
}

body="benchmark"$'\t'"thaw_js_ms"$'\n'
for f in "$HERE"/*.js; do
  name=$(basename "$f" .js)
  time_best "$THAW_JS_RUN" "$f" || exit 1
  echo "$name: thaw-js ${BEST_MS} ms  [$LAST_OUT]" >&2
  body+="$name"$'\t'"$BEST_MS"$'\n'
done

if [[ -n "$OUT" ]]; then
  printf '%s' "$body" > "$OUT"
  echo "wrote $OUT" >&2
else
  printf '%s' "$body"
fi
