#!/usr/bin/env bash
# Differential check of untyped-JS (implicit any) programs against Node. Usage: THAW=path tools/implicit-any/run.sh [WRAP...]
set -u
HERE=$(cd "$(dirname "$0")" && pwd); T=${THAW:-target/release/thaw}; fail=0
for f in "$HERE"/*.mjs; do
  b=$(mktemp -d)/p
  "$@" "$T" build "$f" -o "$b" >"$b.log" 2>&1 || { echo "BUILD-FAIL $(basename "$f"): $(grep -m1 error "$b.log")"; fail=1; continue; }
  if diff <(node "$f" 2>&1) <("$b" 2>&1) >/dev/null; then echo "MATCH $(basename "$f")"; else echo "DIFF $(basename "$f")"; diff <(node "$f" 2>&1) <("$b" 2>&1) | head -10; fail=1; fi
done
exit $fail
