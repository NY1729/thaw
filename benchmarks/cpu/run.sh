#!/usr/bin/env bash
set -euo pipefail

thaw=${1:-target/release/thaw}
runs=${THAW_BENCH_RUNS:-10}
warmups=${THAW_BENCH_WARMUPS:-3}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

measure() {
  local name=$1 expected=$2 executable="$work/$1"
  "$thaw" build "$root/benchmarks/cpu/$name.ts" -o "$executable" >/dev/null
  for ((run = 0; run < warmups; run++)); do "$executable" >/dev/null; done
  : >"$work/$name.times"
  for ((run = 0; run < runs; run++)); do
    output=$(/usr/bin/time -f '%e %M' -o "$work/time" "$executable")
    [[ $output == "$expected" ]] || { echo "$name: checksum $output, expected $expected" >&2; exit 1; }
    cat "$work/time" >>"$work/$name.times"
  done
  awk -v name="$name" '{ seconds += $1; rss += $2 } END { printf "%s\t%.3f s\t%.1f MiB\n", name, seconds / NR, rss / NR / 1024 }' "$work/$name.times"
}

printf 'benchmark\tmean elapsed\tmean max RSS\n'
measure fibonacci 102334155
measure numeric-loop 14985000000
