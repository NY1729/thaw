#!/usr/bin/env bash
# JIT coverage, stage 0 (PR 1): per corpus program, builds the program twice with the thaw CLI
#   default: as shipped (THAW_JS_INFER unset, thaw-js routing is the default engine)
#   infer:   THAW_JS_INFER=1 at build time (AOT clones from the JS->HIR inference)
# then times each binary (MEASURE_RUNS rounds, no profiling) and runs it once more with
# THAW_JS_PROFILE=1 THAW_JIT_PROFILE=1 to capture the interpreter-entry and JIT-call counters.
# The fetch, Node reference and stdout comparison are the steps of tools/quickjs-removal/measure.sh.
#
# Writes per program and variant into OUT_DIR (read by run.py):
#   <prog>.meta                 add_ok, node_ok
#   <prog>.node                 Node reference stdout
#   <prog>.<variant>.meta       build_ok, build_ms, rounds, wall_ms (median), stdout_ok, profile_stdout_ok
#   <prog>.<variant>.build      thaw build log (contains the js-infer lines when variant=infer)
#   <prog>.<variant>.inspect    thaw inspect output of the built binary
#   <prog>.<variant>.shim.ts    THAW_DUMP_SHIM registry shim (typed __thaw_typed_jit_ declarations)
#   <prog>.<variant>.profile    stderr of the profiled run (thaw-js-profile / thaw-jit-profile rows)
#
# usage: collect.sh OUT_DIR
# env:   THAW          thaw binary            (default: <repo>/target/release/thaw)
#        SCRATCH       scratch root           (default: /home/fedora/work/thaw-tmp/jit-coverage)
#        CORPUS        corpus root            (default: <repo>/tools/quickjs-removal/corpus)
#        VERSIONS      pinned specs           (default: $CORPUS/versions.txt)
#        PK_DIR        corpus programs        (default: $CORPUS/pk)
#        ONLY          run one program only
#        MEASURE_RUNS  timed rounds per binary (default 5)
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
THAW="${THAW:-$TARGET_DIR/release/thaw}"
SCRATCH="${SCRATCH:-/home/fedora/work/thaw-tmp/jit-coverage}"
CORPUS="${CORPUS:-$REPO/tools/quickjs-removal/corpus}"
VERSIONS="${VERSIONS:-$CORPUS/versions.txt}"
PK_DIR="${PK_DIR:-$CORPUS/pk}"
ONLY="${ONLY:-}"
RUNS="${MEASURE_RUNS:-5}"
case "$RUNS" in ''|*[!0-9]*|0) RUNS=5 ;; esac

OUT="${1:?usage: collect.sh OUT_DIR}"
[ -x "$THAW" ] || { echo "missing thaw binary: $THAW" >&2; exit 1; }
[ -f "$VERSIONS" ] || { echo "missing versions file: $VERSIONS" >&2; exit 1; }
mkdir -p "$OUT" "$SCRATCH/proj" "$SCRATCH/bin" "$SCRATCH/tmp"
export TMPDIR="$SCRATCH/tmp"
export RUST_MIN_STACK=16777216

# program name and pinned npm specs: one line per program, `name spec@ver ...`
mapfile -t PROGRAMS < <(grep -vE '^[[:space:]]*(#|$)' "$VERSIONS")
VARIANTS=(default infer)

# Writes one key=value line to a meta file. Args: file key value.
meta() { printf '%s=%s\n' "$2" "$3" >> "$1"; }

# Times one run of a binary. Args: binary stdout stderr. Prints "<ms> <exit status>".
timed_run() {
  local bin="$1" out="$2" err="$3" s e status
  s=$(date +%s%N)
  timeout 120 "$bin" > "$out" 2> "$err"
  status=$?
  e=$(date +%s%N)
  echo "$(( (e - s) / 1000000 )) $status"
}

measure_program() {
  local name="$1"; shift
  local specs="$*"
  local proj="$SCRATCH/proj/$name" attempt add_ok=0 node_ok=0
  mkdir -p "$proj"
  cd "$proj" || return
  [ -f package.json ] || echo '{"name":"t","version":"1.0.0"}' > package.json
  # a cached thaw_modules bundle from an older thaw is "up to date" to `add` and silently stale
  rm -rf thaw_modules
  for attempt in 1 2 3; do
    # shellcheck disable=SC2086
    if timeout 300 "$THAW" add $specs --prefix . > "$OUT/$name.add.log" 2>&1; then
      add_ok=1
      break
    fi
    echo "[jit-coverage] thaw add for $name failed (attempt $attempt/3)" >&2
    sleep 5
  done
  : > "$OUT/$name.meta"
  meta "$OUT/$name.meta" add_ok "$add_ok"
  if [ "$add_ok" = 0 ] || [ ! -f "$PK_DIR/$name.ts" ]; then
    echo "[jit-coverage] SKIP $name: thaw add or program source missing" >&2
    return
  fi
  cp "$PK_DIR/$name.ts" .

  # Node reference output (the program does not auto-invoke main() under node). Same fallback as measure.sh.
  cp "$name.ts" nodecmp.mts
  printf '\nawait main();\n' >> nodecmp.mts
  if ! timeout 120 node --experimental-strip-types nodecmp.mts > "$OUT/$name.node" 2>/dev/null; then
    sed -E 's/^import \{([^}]*)\} from ("[^"]+");/import __p from \2; const {\1} = __p as any;/' \
      nodecmp.mts > nodecmp_default.mts
    cp nodecmp_default.mts nodecmp.mts
    rm -f nodecmp_default.mts
  fi
  if timeout 120 node --experimental-strip-types nodecmp.mts > "$OUT/$name.node" 2>/dev/null; then
    node_ok=1
  fi
  meta "$OUT/$name.meta" node_ok "$node_ok"

  local variant
  for variant in "${VARIANTS[@]}"; do
    measure_variant "$name" "$variant"
  done
}

# Builds, times and profiles one variant of a program. Args: name variant.
measure_variant() {
  local name="$1" variant="$2"
  local bin="$SCRATCH/bin/$name.$variant"
  local log="$OUT/$name.$variant"
  local meta_file="$log.meta"
  local s e build_ok=1 r rounds="" status=0 rc=0 stdout_ok=1 timed median
  local -a extra=()
  [ "$variant" = infer ] && extra=(THAW_JS_INFER=1)

  : > "$meta_file"
  s=$(date +%s%N)
  env -u THAW_JS_RUNTIME -u THAW_JS_ENGINE -u THAW_JS_INFER "${extra[@]}" \
    THAW_DUMP_SHIM="$log.shim.ts" timeout 900 "$THAW" build "$name.ts" -o "$bin" > "$log.build" 2>&1 || build_ok=0
  e=$(date +%s%N)
  meta "$meta_file" build_ok "$build_ok"
  meta "$meta_file" build_ms "$(( (e - s) / 1000000 ))"
  if [ "$build_ok" = 0 ]; then
    echo "[jit-coverage] SKIP $name/$variant: build failed" >&2
    return
  fi
  "$THAW" inspect "$bin" > "$log.inspect" 2>&1

  # timed rounds: the stdout of every round must equal the Node reference
  for (( r = 1; r <= RUNS; r++ )); do
    timed=$(timed_run "$bin" "$log.stdout" "$log.stderr")
    rounds+=" ${timed% *}"
    [ "${timed#* }" = 0 ] || rc="${timed#* }"
    cmp -s "$log.stdout" "$OUT/$name.node" || stdout_ok=0
  done
  median=$(printf '%s\n' $rounds | sort -n | sed -n "$(( (RUNS + 1) / 2 ))p")
  meta "$meta_file" rounds "$RUNS"
  meta "$meta_file" wall_ms "$median"
  meta "$meta_file" exit "$rc"
  meta "$meta_file" stdout_ok "$stdout_ok"

  # one profiled run: counters on, stdout checked again (the counters must not change the output)
  env THAW_JS_PROFILE=1 THAW_JIT_PROFILE=1 timeout 120 "$bin" > "$log.profile.stdout" 2> "$log.profile"
  status=$?
  local profile_ok=1
  cmp -s "$log.profile.stdout" "$OUT/$name.node" || profile_ok=0
  meta "$meta_file" profile_exit "$status"
  meta "$meta_file" profile_stdout_ok "$profile_ok"
  rm -f "$log.profile.stdout"
}

for entry in "${PROGRAMS[@]}"; do
  read -r -a parts <<< "$entry"
  name="${parts[0]}"
  [ -n "$ONLY" ] && [ "$name" != "$ONLY" ] && continue
  echo "[jit-coverage] program $name" >&2
  measure_program "${parts[@]}"
done
echo "[jit-coverage] wrote logs to $OUT" >&2
