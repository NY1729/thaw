#!/usr/bin/env bash
# QuickJS removal stage Q-1: measurement harness.
#
# Builds the 24-program npm corpus under both JS engines and records, per program and engine:
#   build_ok, build_s, F (QuickJS-routed entries from `thaw inspect`), links_quickjs,
#   stdout_eq_node, run_ms, rss_kb, binary size.
# Also records: thaw-js-scan eligibility over the bundle corpus, and `thaw node-compat` pass/fail
# for tests/node-compat.json and tests/builtins-compat.json under both engines, plus the number of
# cases whose classification differs between the two engines ("both ways" diff).
#
# Output: one long-format TSV (section, subject, engine, metric, value). Scratch lives under $SCRATCH.
#
# usage: measure.sh [-o OUT.tsv] [--only NAME] [--skip-node-compat]
# env:   THAW        thaw binary            (default: $CARGO_TARGET_DIR/debug/thaw or <repo>/target/debug/thaw)
#        THAW_SCAN   thaw-js-scan binary    (default: same dir as THAW)
#        SCRATCH     scratch root           (default: /home/fedora/work/thaw-tmp/qjsq1-measure)
#        CORPUS      corpus root            (default: <repo>/tools/quickjs-removal/corpus)
#        VERSIONS    pinned specs           (default: $CORPUS/versions.txt; lines `name spec@ver ...`)
#        PK_DIR      corpus programs        (default: $CORPUS/pk)
#        JS_DIR      scan bundles           (default: $CORPUS/corpus; scan skipped if absent)
#        Each program's run_ms is the median of 5 runs (date-based ms, %e kept for RSS).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
THAW="${THAW:-$TARGET_DIR/debug/thaw}"
THAW_SCAN="${THAW_SCAN:-$(dirname "$THAW")/thaw-js-scan}"
SCRATCH="${SCRATCH:-/home/fedora/work/thaw-tmp/qjsq1-measure}"
CORPUS="${CORPUS:-$REPO/tools/quickjs-removal/corpus}"
VERSIONS="${VERSIONS:-$CORPUS/versions.txt}"
PK_DIR="${PK_DIR:-$CORPUS/pk}"
JS_DIR="${JS_DIR:-$CORPUS/corpus}"
OUT="$SCRATCH/baseline.tsv"
ONLY=""
SKIP_NODE_COMPAT=0

while [ $# -gt 0 ]; do
  case "$1" in
    -o) OUT="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --skip-node-compat) SKIP_NODE_COMPAT=1; shift ;;
    *) echo "usage: $0 [-o OUT.tsv] [--only NAME] [--skip-node-compat]" >&2; exit 2 ;;
  esac
done

# program name and pinned npm specs: one line per program, `name spec@ver ...`
[ -f "$VERSIONS" ] || { echo "missing versions file: $VERSIONS" >&2; exit 1; }
mapfile -t PROGRAMS < <(grep -vE '^[[:space:]]*(#|$)' "$VERSIONS")

ENGINES=(quickjs thaw-js)
export TMPDIR="$SCRATCH/tmp"
export RUST_MIN_STACK=16777216
mkdir -p "$SCRATCH/proj" "$SCRATCH/bin" "$SCRATCH/out" "$TMPDIR" "$(dirname "$OUT")"

[ -x "$THAW" ] || { echo "missing thaw binary: $THAW" >&2; exit 1; }
MEASURE_ROWS="$SCRATCH/out/rows.tsv"
: > "$MEASURE_ROWS"
row() { printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >> "$MEASURE_ROWS"; }

# Engine selection: THAW_JS_ENGINE=quickjs|thaw-js; the legacy THAW_JS_RUNTIME is always unset.
engine_env() {
  echo "-u THAW_JS_RUNTIME THAW_JS_ENGINE=$1"
}

measure_program() {
  local name="$1"; shift
  local specs="$*"
  local proj="$SCRATCH/proj/$name"
  mkdir -p "$proj"
  cd "$proj" || return
  [ -f package.json ] || echo '{"name":"t","version":"1.0.0"}' > package.json
  # a cached thaw_modules bundle from an older thaw is "up to date" to `add` and silently stale
  rm -rf thaw_modules
  local attempt add_ok=0
  for attempt in 1 2 3; do
    # shellcheck disable=SC2086
    if timeout 300 "$THAW" add $specs --prefix . > "$SCRATCH/out/$name.add.log" 2>&1; then
      add_ok=1
      break
    fi
    echo "[measure] thaw add for $name failed (attempt $attempt/3)" >&2
    sleep 5
  done
  row program "$name" node add_ok "$add_ok"
  if [ "$add_ok" = 0 ] || [ ! -f "$PK_DIR/$name.ts" ]; then
    # Missing bundle or program source: fail the program under both engines, do not skip silently.
    local f
    for f in "${ENGINES[@]}"; do
      row program "$name" "$f" build_ok 0
      row program "$name" "$f" stdout_eq_node 0
    done
    return
  fi
  cp "$PK_DIR/$name.ts" .

  # Node reference output (the program does not auto-invoke main() under node).
  cp "$name.ts" nodecmp.mts
  printf '\nawait main();\n' >> nodecmp.mts
  if ! timeout 120 node --experimental-strip-types nodecmp.mts > "$SCRATCH/out/$name.node" 2>/dev/null; then
    # A CommonJS package (lodash) has no ESM named exports under Node: retry with
    # `import { a, b } from "p"` rewritten to a default import destructured at the top.
    sed -E 's/^import \{([^}]*)\} from ("[^"]+");/import __p from \2; const {\1} = __p as any;/' \
      nodecmp.mts > nodecmp_default.mts
    cp nodecmp_default.mts nodecmp.mts
    rm -f nodecmp_default.mts
  fi
  if timeout 120 node --experimental-strip-types nodecmp.mts > "$SCRATCH/out/$name.node" 2>/dev/null; then
    row program "$name" node stdout_ok 1
  else
    row program "$name" node stdout_ok 0
  fi

  local engine
  for engine in "${ENGINES[@]}"; do
    local -a envargs
    read -r -a envargs <<< "$(engine_env "$engine")"
    local bin="$SCRATCH/bin/$name.$engine"
    local log="$SCRATCH/out/$name.$engine"
    local s e build_ok=1
    s=$(date +%s%N)
    env "${envargs[@]}" timeout 900 "$THAW" build "$name.ts" -o "$bin" > "$log.build" 2>&1 || build_ok=0
    e=$(date +%s%N)
    row program "$name" "$engine" build_ok "$build_ok"
    row program "$name" "$engine" build_ms "$(( (e - s) / 1000000 ))"
    if [ "$build_ok" = 0 ]; then
      row program "$name" "$engine" F ""
      row program "$name" "$engine" links_quickjs ""
      row program "$name" "$engine" stdout_eq_node 0
      continue
    fi
    "$THAW" inspect "$bin" > "$log.inspect" 2>&1
    local links F
    links=$(awk -F': ' '/^quickjs: /{print ($2=="true")?1:0}' "$log.inspect")
    F=$(grep -c '^quickjs-reason:' "$log.inspect")
    row program "$name" "$engine" links_quickjs "${links:-0}"
    row program "$name" "$engine" F "$F"
    row program "$name" "$engine" size_bytes "$(stat -c %s "$bin")"

    # Five runs: run_ms is the median (startup-dominated runs are noisy); stdout must match Node on every run.
    local -a runs=()
    local r status run_status=0 rss=0 rrss ok=1
    for r in 1 2 3 4 5; do
      s=$(date +%s%N)
      /usr/bin/time -f "%e %M" -o "$log.time" timeout 120 "$bin" > "$log.stdout" 2> "$log.stderr"
      status=$?
      e=$(date +%s%N)
      runs+=( $(( (e - s) / 1000000 )) )
      [ "$status" = 0 ] || run_status=$status
      read -r _ rrss < "$log.time"
      [ "${rrss:-0}" -gt "$rss" ] && rss=$rrss
      cmp -s "$log.stdout" "$SCRATCH/out/$name.node" || ok=0
    done
    local median
    median=$(printf '%s\n' "${runs[@]}" | sort -n | sed -n '3p')
    row program "$name" "$engine" run_exit "$run_status"
    row program "$name" "$engine" run_ms "$median"
    row program "$name" "$engine" rss_kb "$rss"
    row program "$name" "$engine" stdout_eq_node "$ok"
  done
}

for entry in "${PROGRAMS[@]}"; do
  read -r -a parts <<< "$entry"
  name="${parts[0]}"
  [ -n "$ONLY" ] && [ "$name" != "$ONLY" ] && continue
  echo "[measure] program $name" >&2
  measure_program "${parts[@]}"
done

# thaw-js-scan over the bundle corpus: eligibility and blocker counts (engine-independent).
# Skipped when the bundle directory is absent (the CI ratio gate does not need it).
if [ -z "$ONLY" ] && [ -x "$THAW_SCAN" ] && [ -d "$JS_DIR" ]; then
  echo "[measure] thaw-js-scan" >&2
  "$THAW_SCAN" "$JS_DIR"/*.js > "$SCRATCH/out/scan.tsv" 2> "$SCRATCH/out/scan.err"
  row scan corpus thaw-js bundles "$(awk -F'\t' '$2=="modules"' "$SCRATCH/out/scan.tsv" | wc -l)"
  row scan corpus thaw-js eligible "$(awk -F'\t' '$2=="modules" && $4=="CLEAN"' "$SCRATCH/out/scan.tsv" | wc -l)"
  row scan corpus thaw-js blockers "$(awk -F'\t' '$2=="blocker"' "$SCRATCH/out/scan.tsv" | wc -l)"
  row scan corpus thaw-js scan_errors "$(awk -F'\t' '$2=="error"' "$SCRATCH/out/scan.tsv" | wc -l)"
fi

# node-compat manifests under both engines, and per-case agreement between the engines.
if [ "$SKIP_NODE_COMPAT" = 0 ] && [ -z "$ONLY" ]; then
  cd "$REPO" || exit 1
  for manifest in tests/node-compat.json tests/builtins-compat.json; do
    label=$(basename "$manifest" .json)
    for engine in "${ENGINES[@]}"; do
      local_env=()
      read -r -a local_env <<< "$(engine_env "$engine")"
      echo "[measure] $label under $engine" >&2
      env "${local_env[@]}" timeout 14400 "$THAW" node-compat "$manifest" \
        > "$SCRATCH/out/$label.$engine.json" 2> "$SCRATCH/out/$label.$engine.err"
      python3 - "$SCRATCH/out/$label.$engine.json" "$label" "$engine" >> "$MEASURE_ROWS" <<'PY'
import json, sys
path, label, engine = sys.argv[1:4]
try:
    report = json.load(open(path))
except Exception:
    print(f"compat\t{label}\t{engine}\treport\tunparsable")
    sys.exit(0)
total, bugs, ref = report["total"], report["bugs"], report["referenceErrors"]
for metric, value in [("total", total), ("pass", total - bugs - ref), ("bug", bugs), ("reference_error", ref)]:
    print(f"compat\t{label}\t{engine}\t{metric}\t{value}")
PY
    done
    python3 - "$SCRATCH/out/$label.quickjs.json" "$SCRATCH/out/$label.thaw-js.json" "$label" >> "$MEASURE_ROWS" <<'PY'
import json, sys
a, b, label = sys.argv[1:4]
try:
    ra = {r["name"]: r["classification"] for r in json.load(open(a))["results"]}
    rb = {r["name"]: r["classification"] for r in json.load(open(b))["results"]}
    diff = sum(1 for k in set(ra) | set(rb) if ra.get(k) != rb.get(k))
    print(f"compat\t{label}\tboth\tcase_diffs\t{diff}")
except Exception as e:
    print(f"compat\t{label}\tboth\tcase_diffs\tunavailable")
PY
  done
  cd "$REPO" || exit 1
fi

# Final TSV: header + rows, sorted by section/subject/engine/metric.
{ printf 'section\tsubject\tengine\tmetric\tvalue\n'; sort "$MEASURE_ROWS"; } > "$OUT"
echo "[measure] wrote $OUT ($(wc -l < "$OUT") lines)" >&2
