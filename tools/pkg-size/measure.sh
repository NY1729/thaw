#!/usr/bin/env bash
# Per-package cost of an import-only program: exe size, build wall time, cold start (median) and max RSS.
# One row per corpus package plus a `_hello` row (no import) as the floor. Output: TSV (see README.md).
#
# usage: measure.sh [-o OUT.tsv] [--only NAME]
# env:   THAW  thaw binary (default $CARGO_TARGET_DIR|<repo>/target + /release/thaw)
#        SCRATCH scratch root (default /home/fedora/work/thaw-tmp/pkg-size)
#        VERSIONS pinned specs (default tools/corpus/corpus/versions.txt)
#        MEASURE_RUNS start-up rounds (default 10)
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
THAW="${THAW:-${CARGO_TARGET_DIR:-$REPO/target}/release/thaw}"
SCRATCH="${SCRATCH:-/home/fedora/work/thaw-tmp/pkg-size}"
VERSIONS="${VERSIONS:-$REPO/tools/corpus/corpus/versions.txt}"
RUNS="${MEASURE_RUNS:-10}"
OUT="$SCRATCH/pkg-size.tsv"
ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) OUT="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    *) echo "usage: $0 [-o OUT.tsv] [--only NAME]" >&2; exit 2 ;;
  esac
done
[ -x "$THAW" ] || { echo "missing thaw binary: $THAW" >&2; exit 1; }
mkdir -p "$SCRATCH/proj" "$SCRATCH/tmp" "$(dirname "$OUT")"
export TMPDIR="$SCRATCH/tmp" RUST_MIN_STACK=16777216
proj="$SCRATCH/proj"
cd "$proj" || exit 1
[ -f package.json ] || echo '{"name":"t","version":"1.0.0"}' > package.json

printf 'name\tbuild_ok\tbuild_first_ms\tbuild_warm_ms\tsize_bytes\tstart_ms\tmax_rss_kb\n' > "$OUT"

# measure NAME: builds NAME.ts in $proj, then times RUNS cold starts.
measure() {
  local name="$1" s e ok=1 size=0 bms i ms=0 rss times=() maxrss=0
  # first build: registry bundling and, for a new runtime feature set, the cargo build of its archives
  s=$(date +%s%N)
  timeout 900 "$THAW" build "$name.ts" -o "$name.bin" > "$name.build.log" 2>&1 || ok=0
  e=$(date +%s%N)
  fms=$(( (e - s) / 1000000 ))
  # second build: everything cached, the number to compare across commits
  s=$(date +%s%N)
  [ "$ok" = 1 ] && { timeout 900 "$THAW" build "$name.ts" -o "$name.bin" > "$name.build2.log" 2>&1 || ok=0; }
  e=$(date +%s%N)
  bms=$(( (e - s) / 1000000 ))
  if [ "$ok" = 1 ]; then
    size=$(stat -c %s "$name.bin")
    for (( i = 0; i < RUNS; i++ )); do
      s=$(date +%s%N)
      /usr/bin/time -f "%M" -o "$name.time" timeout 60 "./$name.bin" > /dev/null 2>&1
      e=$(date +%s%N)
      times+=( $(( (e - s) / 1000000 )) )
      rss=$(tail -n1 "$name.time" 2>/dev/null)
      case "$rss" in ''|*[!0-9]*) rss=0 ;; esac
      [ "$rss" -gt "$maxrss" ] && maxrss=$rss
    done
    ms=$(printf '%s\n' "${times[@]}" | sort -n | sed -n "$(( (RUNS + 1) / 2 ))p")
  else
    echo "[pkg-size] build failed: $name" >&2
    head -n 8 "$name.build.log" >&2
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$ok" "$fms" "$bms" "$size" "$ms" "$maxrss" >> "$OUT"
}

echo 'console.log("ok");' > _hello.ts
[ -z "$ONLY" ] && measure _hello

while read -r name specs; do
  case "$name" in ''|'#'*) continue ;; esac
  [ -n "$ONLY" ] && [ "$name" != "$ONLY" ] && continue
  # first spec is the package itself (strip the @version; scoped names keep their leading @)
  first="${specs%% *}"
  pkg="${first%@*}"
  # shellcheck disable=SC2086
  if ! timeout 300 "$THAW" add $specs --prefix . > "$name.add.log" 2>&1; then
    echo "[pkg-size] add failed: $name" >&2
    printf '%s\t0\t0\t0\t0\t0\t0\n' "$name" >> "$OUT"
    continue
  fi
  # a side-effect import: links and loads the package without needing a typed use of any export
  printf 'import "%s";\nconsole.log("ok");\n' "$pkg" > "$name.ts"
  measure "$name"
done < "$VERSIONS"
column -t -s$'\t' "$OUT" >&2
