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
#        PK_DIR      corpus programs        (default: /home/fedora/work/thaw-tmp/tmp-run/jsr/pk)
#        JS_DIR      scan bundles           (default: /home/fedora/work/thaw-tmp/tmp-run/jsr/corpus)
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
THAW="${THAW:-$TARGET_DIR/debug/thaw}"
THAW_SCAN="${THAW_SCAN:-$(dirname "$THAW")/thaw-js-scan}"
SCRATCH="${SCRATCH:-/home/fedora/work/thaw-tmp/qjsq1-measure}"
PK_DIR="${PK_DIR:-/home/fedora/work/thaw-tmp/tmp-run/jsr/pk}"
JS_DIR="${JS_DIR:-/home/fedora/work/thaw-tmp/tmp-run/jsr/corpus}"
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

# program name and npm package specs (same mapping as the jsr run1.sh set)
PROGRAMS=(
  "ms ms @types/ms"
  "lodash lodash @types/lodash"
  "zod zod"
  "dayjs dayjs"
  "semver semver @types/semver"
  "uuid uuid"
  "qs qs @types/qs"
  "marked marked"
  "jsyaml js-yaml @types/js-yaml"
  "validator validator @types/validator"
  "nanoid nanoid"
  "chalk chalk"
  "minimist minimist @types/minimist"
  "bcryptjs bcryptjs"
  "luxon luxon @types/luxon"
  "datefns date-fns"
  "hono hono"
  "handlebars handlebars"
  "ajv ajv"
  "commander commander"
  "cheerio cheerio"
  "axios axios"
  "ejs ejs @types/ejs"
  "yup yup"
)

ENGINES=(quickjs thaw-js)
export TMPDIR="$SCRATCH/tmp"
export RUST_MIN_STACK=16777216
mkdir -p "$SCRATCH/proj" "$SCRATCH/bin" "$SCRATCH/out" "$TMPDIR" "$(dirname "$OUT")"

[ -x "$THAW" ] || { echo "missing thaw binary: $THAW" >&2; exit 1; }
MEASURE_ROWS="$SCRATCH/out/rows.tsv"
: > "$MEASURE_ROWS"
row() { printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >> "$MEASURE_ROWS"; }

# Engine selection: quickjs = routing flag unset (default); thaw-js = THAW_JS_RUNTIME=1.
engine_env() {
  if [ "$1" = thaw-js ]; then echo "THAW_JS_RUNTIME=1"; else echo "-u THAW_JS_RUNTIME"; fi
}

measure_program() {
  local name="$1"; shift
  local specs="$*"
  local proj="$SCRATCH/proj/$name"
  mkdir -p "$proj"
  cd "$proj" || return
  [ -f package.json ] || echo '{"name":"t","version":"1.0.0"}' > package.json
  # shellcheck disable=SC2086
  timeout 300 "$THAW" add $specs --prefix . > "$SCRATCH/out/$name.add.log" 2>&1 || true
  cp "$PK_DIR/$name.ts" .

  # Node reference output (the program does not auto-invoke main() under node).
  cp "$name.ts" nodecmp.mts
  printf '\nawait main();\n' >> nodecmp.mts
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

    /usr/bin/time -f "%e %M" -o "$log.time" timeout 120 "$bin" > "$log.stdout" 2> "$log.stderr"
    local run_status=$?
    local wall rss
    read -r wall rss < "$log.time"
    row program "$name" "$engine" run_exit "$run_status"
    row program "$name" "$engine" run_ms "$(awk -v w="$wall" 'BEGIN{printf "%d", w*1000}')"
    row program "$name" "$engine" rss_kb "$rss"
    if cmp -s "$log.stdout" "$SCRATCH/out/$name.node"; then
      row program "$name" "$engine" stdout_eq_node 1
    else
      row program "$name" "$engine" stdout_eq_node 0
    fi
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
if [ -z "$ONLY" ] && [ -x "$THAW_SCAN" ]; then
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
