#!/usr/bin/env bash
# Staleness check for module_table.tsv: regenerate under Node 22 into scratch and compare.
# Usage: tools/parity-aot/check_module_table.sh [TABLE_FILE]
# Exit 0 = up to date (WARN lines allowed), 1 = format error or drift in a row present in both files,
# 2 = generator or harness error. Rows present in only one file are WARN (version-dependent exports).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TABLE=${1:-$HERE/module_table.tsv}
NODE=${NODE:-node}
BASE=${PARITY_AOT_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp/parity-aot-scratch}}
mkdir -p "$BASE" || exit 2
W=$(mktemp -d "$BASE/module-table.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
"$NODE" "$HERE/module_inv.js" "$W/gen.tsv" 2> "$W/gen.err" || { cat "$W/gen.err" >&2; echo "check_module_table: generator failed" >&2; exit 2; }
[ -f "$TABLE" ] || { echo "check_module_table: no table at $TABLE" >&2; exit 2; }
python3 - "$TABLE" "$W/gen.tsv" <<'PY'
import sys
HEADER = ["module", "export", "kind", "arity", "vtype", "alias"]
KINDS = {"function", "class", "constant", "primitive", "object", "accessor"}
VTYPES = {"function", "object", "string", "number", "boolean", "bigint", "symbol", "undefined", "null", "-"}

def load(path, label):
    lines = open(path, encoding="utf-8").read().split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    errs = []
    if not lines or lines[0].split("\t") != HEADER:
        errs.append(f"{label}: header is not {chr(9).join(HEADER)}")
        return {}, errs, []
    rows, keys = {}, []
    for n, line in enumerate(lines[1:], 2):
        c = line.split("\t")
        if len(c) != len(HEADER):
            errs.append(f"{label}:{n}: {len(c)} columns, want {len(HEADER)}")
            continue
        mod, exp, kind, arity, vtype, alias = c
        if kind not in KINDS:
            errs.append(f"{label}:{n}: unknown kind {kind!r}")
        if arity != "-" and not arity.isdigit():
            errs.append(f"{label}:{n}: arity {arity!r} is not a number or '-'")
        if vtype not in VTYPES:
            errs.append(f"{label}:{n}: unknown vtype {vtype!r}")
        if not mod or not exp:
            errs.append(f"{label}:{n}: empty module or export")
        key = (mod, exp)
        if key in rows:
            errs.append(f"{label}:{n}: duplicate row {mod} {exp}")
        rows[key] = line
        keys.append(key)
    if keys != sorted(keys):
        errs.append(f"{label}: rows are not in (module, export) order")
    return rows, errs, keys

committed, errs_c, _ = load(sys.argv[1], "committed")
generated, errs_g, _ = load(sys.argv[2], "generated")
errors = errs_c + errs_g
for e in errors:
    print(f"FORMAT {e}")
drift = [k for k in committed if k in generated and committed[k] != generated[k]]
for k in drift:
    print(f"DRIFT  {k[0]} {k[1]}\n  committed: {committed[k]}\n  generated: {generated[k]}")
for k in sorted(set(generated) - set(committed)):
    print(f"WARN   row not in committed table (version-dependent?): {k[0]} {k[1]}")
for k in sorted(set(committed) - set(generated)):
    print(f"WARN   committed row not generated (version-dependent?): {k[0]} {k[1]}")
if errors or drift:
    print(f"module_table: FAILED ({len(errors)} format errors, {len(drift)} drifted rows; regenerate: node tools/parity-aot/module_inv.js tools/parity-aot/module_table.tsv)")
    sys.exit(1)
print(f"module_table: up to date ({len(committed)} rows, {len(generated)} generated)")
PY
rc=$?
[ "$rc" = 0 ] || { echo "check_module_table: DRIFT or format error in $TABLE" >&2; exit 1; }
exit 0
