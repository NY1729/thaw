#!/bin/bash
# usage: run-shard.sh SHARD|all OUT.jsonl [jobs]   (shard names from shards.json)
# env: NODE_TESTS_DIR (fetch.sh checkout), NODE_TESTS_WORK (scratch dir), THAW (thaw binary)
# Heavy: run under  systemd-run --user --scope --slice=app-heavytests.slice -p OOMPolicy=kill -p MemoryMax=20G
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
names=$1; [ "$1" = all ] && names=$(python3 -I -c "import json;print(' '.join(json.load(open('$here/shards.json'))))")
for s in $names; do
  p=$(python3 -I -c "import json;print(json.load(open('$here/shards.json'))['$s']['prefix'])")
  python3 -I "$here/run.py" run --prefix "$p" --out "$2" --jobs "${3:-3}"
done
