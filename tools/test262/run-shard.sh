#!/bin/bash
# usage: run-shard.sh SHARD OUT.jsonl [jobs]   (shard names from shards.json; "all" runs every shard in turn)
# env: TEST262_DIR, THAW_JS_NODE
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
names=$1; [ "$1" = all ] && names=$(python3 -I -c "import json,sys;print(' '.join(json.load(open('$here/shards.json'))))")
for s in $names; do
  args=$(python3 -I -c "
import json,sys,shlex
c=json.load(open('$here/shards.json'))['$s']
print('--dirs',shlex.quote(c['dirs']),'--exclude',shlex.quote(c.get('exclude','')))")
  start=$(date +%s)
  eval python3 -I "$here/run.py" --out "$2" $args --jobs "${3:-3}"
  echo "shard $s: $(( $(date +%s) - start )) s" >&2
done
