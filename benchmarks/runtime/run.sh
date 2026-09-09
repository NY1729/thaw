#!/usr/bin/env bash
set -euo pipefail

thaw=${1:-target/release/thaw}
output=${2:-runtime-metrics.json}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

printf 'function main(): void { console.log("hello"); }\n' > "$work/hello.ts"
"$thaw" build "$work/hello.ts" -o "$work/hello" >/dev/null

measure() {
  name=$1
  shift
  started=$(date +%s%N)
  /usr/bin/time -f '%M' -o "$work/$name.rss" "$@" >/dev/null
  printf '%s %s\n' "$((($(date +%s%N) - started) / 1000000))" "$(cat "$work/$name.rss")" > "$work/$name.time"
}

measure thaw "$work/hello"
measure node node -e 'console.log("hello")'
measure bun bun -e 'console.log("hello")'
read -r thaw_ms thaw_rss_kb < "$work/thaw.time"
read -r node_ms node_rss_kb < "$work/node.time"
read -r bun_ms bun_rss_kb < "$work/bun.time"

printf '{"architecture":"%s","thaw":{"startup_ms":%s,"max_rss_kb":%s,"executable_bytes":%s},"node":{"startup_ms":%s,"max_rss_kb":%s,"executable_bytes":%s},"bun":{"startup_ms":%s,"max_rss_kb":%s,"executable_bytes":%s}}\n' \
  "$(uname -m)" \
  "$thaw_ms" "$thaw_rss_kb" "$(stat -c %s "$work/hello")" \
  "$node_ms" "$node_rss_kb" "$(stat -Lc %s "$(command -v node)")" \
  "$bun_ms" "$bun_rss_kb" "$(stat -Lc %s "$(command -v bun)")" \
  | tee "$output"
