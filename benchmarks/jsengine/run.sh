#!/usr/bin/env bash
# JS-engine microbenchmarks: times every benchmarks/jsengine/*.js under thaw-js and, when QJS is set, under QuickJS.
#
# usage: run.sh [-n RUNS] [-o OUT.tsv] [-h]
# env:   THAW_JS_RUN       prebuilt plain-script runner, invoked as `$THAW_JS_RUN file.js`
#                          (default: build thaw-js-node in release and use it)
#        QJS               QuickJS-style binary, invoked as `$QJS file.js` (default: unset, reference column is "-")
#        CARGO_TARGET_DIR  honoured by the default build
#
# Output: TSV `benchmark  thaw_js_ms  quickjs_ms  ratio` (ratio = thaw_js_ms / quickjs_ms), then a `geomean` line.
# Each script prints one checksum line; a thaw-js vs QuickJS output mismatch is reported on stderr.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RUNS=3
OUT=""

usage() { sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }
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
for bin in "$THAW_JS_RUN" ${QJS:+"$QJS"}; do
  [[ -x "$bin" ]] || { echo "run.sh: not executable: $bin" >&2; exit 1; }
done

# time_best BIN FILE: best-of-RUNS wall ms into BEST_MS, last stdout into LAST_OUT.
time_best() {
  local bin=$1 file=$2 i s e ms best=""
  for ((i = 0; i < RUNS; i++)); do
    s=${EPOCHREALTIME/[.,]/}
    if ! LAST_OUT=$("$bin" "$file"); then
      echo "run.sh: $bin failed on $file" >&2
      exit 1
    fi
    e=${EPOCHREALTIME/[.,]/}
    ms=$(( (e - s) / 1000 ))
    if [[ -z $best || $ms -lt $best ]]; then best=$ms; fi
  done
  BEST_MS=$best
}

body="benchmark"$'\t'"thaw_js_ms"$'\t'"quickjs_ms"$'\t'"ratio"$'\n'
ratios=()
for f in "$HERE"/*.js; do
  name=$(basename "$f" .js)
  time_best "$THAW_JS_RUN" "$f"
  t_ms=$BEST_MS
  t_out=$LAST_OUT
  q_ms="-"
  ratio="-"
  if [[ -n "${QJS:-}" ]]; then
    time_best "$QJS" "$f"
    q_ms=$BEST_MS
    q_out=$LAST_OUT
    if [[ "$q_out" != "$t_out" ]]; then
      echo "run.sh: checksum mismatch for $name: thaw-js=[$t_out] quickjs=[$q_out]" >&2
    fi
    if (( q_ms > 0 )); then
      ratio=$(awk -v a="$t_ms" -v b="$q_ms" 'BEGIN { printf "%.3f", a / b }')
      ratios+=("$ratio")
    fi
  fi
  echo "$name: thaw-js ${t_ms} ms, quickjs ${q_ms} ms  [$t_out]" >&2
  body+="$name"$'\t'"$t_ms"$'\t'"$q_ms"$'\t'"$ratio"$'\n'
done

geo=$(printf '%s\n' ${ratios[@]+"${ratios[@]}"} | awk 'NF { s += log($1); n++ } END { if (n) printf "%.3f", exp(s / n); else print "-" }')
body+="geomean"$'\t'""$'\t'""$'\t'"$geo"$'\n'

if [[ -n "$OUT" ]]; then
  printf '%s' "$body" > "$OUT"
  echo "wrote $OUT" >&2
else
  printf '%s' "$body"
fi
