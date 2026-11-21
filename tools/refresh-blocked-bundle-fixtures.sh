#!/bin/sh
# Refresh the blocked-bundle gate fixtures (designs/blocked-bundles-gate.md, section 2).
#
# usage: tools/refresh-blocked-bundle-fixtures.sh <dir>
#
# <dir> holds plain bundles named <name>.js, as written by `thaw registry add` (one per fixture name).
# For every row of tools/blocked-bundles-manifest.tsv this gzips <dir>/<name>.js (gzip -9n: deterministic,
# no name or mtime) into crates/thaw-cli/tests/fixtures/bundles/<name>.js.gz and rewrites the sha256 column.
# The installed= and addon= columns are kept. Afterwards run the crate test
#   cargo test -p thaw-cli --test blocked_bundles
# and update tools/blocked-bundles.txt for any row the scan now gives differently (the test says which).
set -eu

if [ $# -ne 1 ]; then
    echo "usage: $0 <dir with <name>.js bundles>" >&2
    exit 2
fi

root=$(cd "$(dirname "$0")/.." && pwd)
src=$1
manifest=$root/tools/blocked-bundles-manifest.tsv
out=$root/crates/thaw-cli/tests/fixtures/bundles
mkdir -p "$out"

awk -F '\t' -v OFS='\t' -v src="$src" -v out="$out" '
    /^#/ || NF == 0 { print; next }
    {
        if (system("gzip -9nc \"" src "/" $1 ".js\" > \"" out "/" $1 ".js.gz\"") != 0) exit 1
        cmd = "sha256sum \"" out "/" $1 ".js.gz\" | cut -d \" \" -f1"
        cmd | getline sha
        close(cmd)
        $2 = sha
        print
    }
' "$manifest" > "$manifest.new"
mv "$manifest.new" "$manifest"
echo "refreshed $(grep -vc '^#' "$manifest") fixtures in $out"
