#!/bin/sh
# usage: mem-bench.sh /path/to/thaw-js-node   (or `node` for the reference column)
# bytes per element = (peak RSS of the case - peak RSS of the empty program) / 1M elements.
# Run under systemd-run with a MemoryMax cap (each case holds ~0.5 GB).
BIN=${1:?runner}; D=$(dirname "$0")
rss() { /usr/bin/time -f %M "$BIN" "$D/$1" 2>&1 >/dev/null | tail -1; }
base=$(rss base.js)
for c in obj_ab arr4 closures strs class3; do
  r=$(rss $c.js)
  echo "$c: peak_rss_kb=$r  bytes_per_element=$(( (r - base) * 1024 / 1000000 ))"
done
