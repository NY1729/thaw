#!/usr/bin/env bash
# Unrun regression: malformed or unavailable measurements must fail the gate.
set -euo pipefail
check=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/check_regression.sh
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
printf '%s\n' '{"thaw":{"startup_ms":1,"max_rss_kb":100}}' > "$scratch/valid.json"
bash "$check" "$scratch/valid.json" "$scratch/valid.json" >/dev/null
for value in '{' '{"thaw":{"startup_ms":"bad","max_rss_kb":100}}' '{"thaw":{"startup_ms":1}}' '{"thaw":{"startup_ms":1,"max_rss_kb":-1}}'; do
  printf '%s\n' "$value" > "$scratch/invalid.json"
  for role in current baseline; do
    current="$scratch/valid.json"
    baseline="$scratch/valid.json"
    if [[ "$role" == current ]]; then current="$scratch/invalid.json"; else baseline="$scratch/invalid.json"; fi
    if bash "$check" "$current" "$baseline" >/dev/null 2>&1; then
      echo "accepted invalid $role measurement: $value" >&2
      exit 1
    fi
  done
done
if bash "$check" "$scratch/missing.json" "$scratch/valid.json" >/dev/null 2>&1; then
  echo 'accepted missing current measurement' >&2
  exit 1
fi
