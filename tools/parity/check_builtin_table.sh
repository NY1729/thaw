#!/usr/bin/env bash
# Staleness check: regenerate the builtin table under Node 22 into scratch and diff it against the checked-in copy.
# Usage: tools/parity/check_builtin_table.sh [TABLE_FILE]
# Exit 0 = up to date, 1 = drift (regenerate, see tools/parity/README.md), 2 = generator or harness error.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TABLE=${1:-$REPO/crates/thaw-js/src/builtin_table.tsv}
NODE=${NODE:-node}
BASE=${PARITY_SCRATCH:-${RUNNER_TEMP:-${TMPDIR:-/home/fedora/work/thaw-tmp/parity-scratch}}}
mkdir -p "$BASE" || exit 2
W=$(mktemp -d "$BASE/builtin-table.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
"$NODE" "$HERE/gen_builtin_table.js" "$W/gen.tsv" || { echo "check_builtin_table: generator failed" >&2; exit 2; }
[ -f "$TABLE" ] || { echo "check_builtin_table: no table at $TABLE" >&2; exit 2; }
if diff -u "$TABLE" "$W/gen.tsv" > "$W/diff"; then
  echo "builtin_table: up to date ($TABLE)"
  exit 0
fi
head -n 60 "$W/diff"
echo "builtin_table: DRIFT in $TABLE (regenerate: node tools/parity/gen_builtin_table.js crates/thaw-js/src/builtin_table.tsv)" >&2
exit 1
