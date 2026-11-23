#!/usr/bin/env bash
# Self-test for diff.py and the builtin table: synthetic outputs prove the gates fail on a missing member.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${PARITY_SCRATCH:-${RUNNER_TEMP:-${TMPDIR:-/home/fedora/work/thaw-tmp/parity-scratch}}}
mkdir -p "$BASE"
W=${PARITY_SELFTEST_DIR:-$(mktemp -d "$BASE/selftest.XXXXXX")}
rm -rf "$W"; mkdir -p "$W/node" "$W/thaw"
fail=0
check() { # name expected_rc actual_rc
  if [ "$2" = "$3" ]; then echo "ok   $1 (rc=$3)"; else echo "FAIL $1 expected rc=$2 got rc=$3"; fail=1; fi
}
DIFF=(python3 "$HERE/diff.py" --node-dir "$W/node" --thaw-dir "$W/thaw")

# 1. synthetic missing member must fail the gate
printf 'a = 1\nb = 2\n' > "$W/node/c.txt"; printf 'a = 1\n' > "$W/thaw/c.txt"
"${DIFF[@]}" --baseline "$W/empty.json" --gaps "$W/g.json" >"$W/o1" 2>&1 ; check "missing member fails gate" 1 $?
grep -q 'MISSING' "$W/o1" && grep -q 'c::b' "$W/o1" || { echo "FAIL report does not name c::c.b"; fail=1; }
python3 -c "import json,sys; g=json.load(open('$W/g.json'))['gaps']; assert [x['id'] for x in g]==['c::b'] and g[0]['kind']=='MISSING'" && echo "ok   gaps.json records c::b MISSING" || { echo "FAIL gaps.json content"; fail=1; }

# 2. the same gap, recorded in a baseline, passes
printf '{"gaps":[{"id":"c::b","kind":"MISSING"}]}\n' > "$W/base.json"
"${DIFF[@]}" --baseline "$W/base.json" >/dev/null 2>&1 ; check "known gap in baseline passes" 0 $?

# 3. identical outputs pass with an empty baseline
printf 'a = 1\n' > "$W/node/d.txt"; printf 'a = 1\n' > "$W/thaw/d.txt"
rm -f "$W/node/c.txt" "$W/thaw/c.txt"
"${DIFF[@]}" --baseline "$W/empty.json" >/dev/null 2>&1 ; check "identical outputs pass" 0 $?

# 4. a DIFFERENT value fails; THROW messages are normalised away but error names are not
printf 'x = THROW TypeError: foo\n' > "$W/node/e.txt"; printf 'x = THROW TypeError: bar\n' > "$W/thaw/e.txt"
"${DIFF[@]}" --baseline "$W/empty.json" >/dev/null 2>&1 ; check "THROW message text ignored" 0 $?
printf 'x = THROW RangeError: bar\n' > "$W/thaw/e.txt"
"${DIFF[@]}" --baseline "$W/empty.json" >/dev/null 2>&1 ; check "THROW error name differs fails" 1 $?
printf 'x = 2\n' > "$W/thaw/e.txt"; printf 'x = 1\n' > "$W/node/e.txt"
"${DIFF[@]}" --baseline "$W/empty.json" >/dev/null 2>&1 ; check "different value fails" 1 $?

# 5. duplicate probe names are a harness error (exit 2)
printf 'x = 1\nx = 1\n' > "$W/node/e.txt"; printf 'x = 1\n' > "$W/thaw/e.txt"
"${DIFF[@]}" --baseline "$W/empty.json" >/dev/null 2>&1 ; check "duplicate probe is harness error" 2 $?
printf 'x = 1\n' > "$W/node/e.txt"

# 6. gaps may only shrink: recording a baseline then fixing the gap passes, re-adding it fails
rm -f "$W/node/e.txt" "$W/thaw/e.txt"; printf 'a = 1\nb = 2\n' > "$W/node/c.txt"; printf 'a = 1\n' > "$W/thaw/c.txt"
"${DIFF[@]}" --baseline "$W/rec.json" --write-baseline >/dev/null 2>&1 ; check "write-baseline records" 0 $?
"${DIFF[@]}" --baseline "$W/rec.json" >/dev/null 2>&1 ; check "recorded baseline passes" 0 $?
printf 'a = 1\nb = 2\n' > "$W/thaw/c.txt"
"${DIFF[@]}" --baseline "$W/rec.json" >/dev/null 2>&1 ; check "fixed gap still passes (shrink)" 0 $?
printf 'a = 1\n' > "$W/thaw/c.txt"
printf '{"gaps":[]}\n' > "$W/rec2.json"
"${DIFF[@]}" --baseline "$W/rec2.json" >/dev/null 2>&1 ; check "gap not in baseline fails again" 1 $?

# 7. --blocks: `@@ name` delimited blocks, values may span lines
BDIFF=("${DIFF[@]}" --blocks --baseline "$W/empty.json")
rm -f "$W"/node/*.txt "$W"/thaw/*.txt "$W/rec.json" "$W/rec2.json"
printf '@@ c.a\nclass K {\n  constructor(...a){\n  }\n}\n@@ c.b\n[ 1,\n  2 ]\n@@ c.c\nplain\n' > "$W/node/c.txt"
printf '@@ c.a\nclass K {\n  constructor(...a){\n  }\n}   \n@@ c.b\n[ 1,\n  2 ]\n\n@@ c.c\nplain\n' > "$W/thaw/c.txt"
"${BDIFF[@]}" >/dev/null 2>&1 ; check "multi-line identical blocks pass (trailing ws ignored)" 0 $?
printf '@@ c.a\nclass K {\n  constructor(...a){\n  }\n}\n@@ c.b\n[ 1,\n  3 ]\n@@ c.c\nplain\n' > "$W/thaw/c.txt"
"${BDIFF[@]}" >/dev/null 2>&1 ; check "multi-line different block fails" 1 $?
"${BDIFF[@]}" >"$W/ob" 2>&1; grep -q 'DIFFERENT' "$W/ob" && grep -q 'c::c.b' "$W/ob" || { echo "FAIL block report does not name c::c.b DIFFERENT"; fail=1; }
printf '@@ c.a\nclass K {\n  constructor(...a){\n  }\n}\n@@ c.c\nplain\n' > "$W/thaw/c.txt"
"${BDIFF[@]}" >"$W/ob" 2>&1 ; check "missing block fails" 1 $?
grep -q 'MISSING' "$W/ob" && grep -q 'c::c.b' "$W/ob" || { echo "FAIL block report does not name c::c.b MISSING"; fail=1; }
printf '@@ c.a\nx\n@@ c.a\nx\n' > "$W/thaw/c.txt"
"${BDIFF[@]}" >/dev/null 2>&1 ; check "duplicate block is harness error" 2 $?
printf 'THROW TypeError: multi\nline message\n' > "$W/node/t.txt"
rm -f "$W/node/c.txt" "$W/thaw/c.txt"; printf '@@ t.x\nTHROW TypeError: a\nmore\n' > "$W/node/t.txt"; printf '@@ t.x\nTHROW TypeError: b\n' > "$W/thaw/t.txt"
"${BDIFF[@]}" >/dev/null 2>&1 ; check "block THROW message text ignored" 0 $?
rm -f "$W/node/t.txt" "$W/thaw/t.txt"

# 9. builtin table: generator is deterministic; the staleness check passes on the checked-in table and fails on a mutated copy
GEN="$HERE/gen_builtin_table.js"; CHK="$HERE/check_builtin_table.sh"
TSV="$HERE/../../crates/thaw-js/src/builtin_table.tsv"
node "$GEN" "$W/bt1.tsv" && node "$GEN" "$W/bt2.tsv" && cmp -s "$W/bt1.tsv" "$W/bt2.tsv"; check "builtin table generator is deterministic" 0 $?
bash "$CHK" >/dev/null 2>&1; check "checked-in builtin_table.tsv is current" 0 $?
sed '10s/$/\tmutated/' "$TSV" > "$W/bt-mut-cell.tsv"; bash "$CHK" "$W/bt-mut-cell.tsv" >/dev/null 2>&1; check "check fails on a mutated cell" 1 $?
sed '10d' "$TSV" > "$W/bt-mut-row.tsv"; bash "$CHK" "$W/bt-mut-row.tsv" >/dev/null 2>&1; check "check fails on a deleted row" 1 $?
bash "$CHK" "$W/no-such-table.tsv" >/dev/null 2>&1; check "check on a missing table is harness error" 2 $?

# 10. waivers: classify gaps (synthetic ids). The gate is unchanged; waived gaps stay baseline entries
WV="$W/waivers.json"
cat > "$WV" <<'EOF'
{"waivers":[
 {"id":"W1","reason":"synthetic internal","match":["c::k_\\w+","c::.*kBar"]},
 {"id":"W2","reason":"synthetic ws","match":["c::WS"]},
 {"id":"W3","reason":"synthetic overlap","match":["c::Foo.*"]}
]}
EOF
rm -f "$W"/node/*.txt "$W"/thaw/*.txt
printf 'p1 = 1\nk_internal = 1\nFoo kBar = 1\np2 = 1\nWS = 1\n' > "$W/node/c.txt"; printf 'p3 = 1\n' > "$W/thaw/c.txt"
WDIFF=("${DIFF[@]}" --waivers "$WV")
printf '{"gaps":[{"id":"c::k_internal","kind":"MISSING"},{"id":"c::Foo kBar","kind":"MISSING"},{"id":"c::WS","kind":"MISSING"},{"id":"c::p2","kind":"MISSING"}]}\n' > "$W/wb.json"
"${WDIFF[@]}" --baseline "$W/wb.json" >/dev/null 2>&1 ; check "waivers: real gap missing from baseline still fails" 1 $?
printf '{"gaps":[{"id":"c::p1","kind":"MISSING"},{"id":"c::p2","kind":"MISSING"},{"id":"c::k_internal","kind":"MISSING"},{"id":"c::Foo kBar","kind":"MISSING"},{"id":"c::WS","kind":"MISSING"}]}\n' > "$W/wb2.json"
"${WDIFF[@]}" --baseline "$W/wb2.json" --gaps "$W/wg.json" >"$W/ow" 2>&1 ; check "waivers: all gaps baselined passes" 0 $?
python3 -c "
import json
g={x['id']:x for x in json.load(open('$W/wg.json'))['gaps']}
exp={'c::p1':None,'c::p2':None,'c::k_internal':'W1','c::Foo kBar':'W1','c::WS':'W2'}
assert set(g)==set(exp), sorted(g)
for k,w in exp.items(): assert g[k]['waiver']==w and g[k]['waived']==(w is not None), (k, g[k])
" && echo "ok   gaps.json waived/waiver fields" || { echo "FAIL gaps.json waiver fields"; fail=1; }
grep -qE '^c +5 +5 +0 +1 +2 +3$' "$W/ow" || { echo "FAIL corpus row real/waived 2/3"; fail=1; }
grep -qE '^W1 +2 ' "$W/ow" && grep -qE '^W2 +1 ' "$W/ow" && grep -qE '^W3 +0 ' "$W/ow" || { echo "FAIL per-waiver counts W1=2 W2=1 W3=0"; fail=1; }
grep -q 'matched 2 waivers (W1, W3), counted as W1: c::Foo kBar' "$W/ow" || { echo "FAIL multi-match not reported"; fail=1; }
"${DIFF[@]}" --baseline "$W/wb2.json" --gaps "$W/ng.json" >/dev/null 2>&1 ; check "no --waivers: gate unchanged" 0 $?
python3 -c "
import json
assert all(x['waived'] is False and x['waiver'] is None for x in json.load(open('$W/ng.json'))['gaps'])
" && echo "ok   no --waivers: every gap real" || { echo "FAIL no-waivers gaps.json"; fail=1; }
# the checked-in waivers.json: a real ES symbol stays real, the approved Node-internal symbol is W1
rm -f "$W"/node/*.txt "$W"/thaw/*.txt
printf 'globalThis.console.[Symbol(kIsConsole)] = 1\nconsole.Console.[Symbol(Symbol.hasInstance)] = 1\nWebAssembly.Tag = 1\nglobalThis.process.[Symbol(kCapture)] = 1\nEventTarget.[Symbol(nodejs.event_target)] = 1\n' > "$W/node/surface.txt"; : > "$W/thaw/surface.txt"
"${DIFF[@]}" --waivers "$HERE/waivers.json" --baseline "$W/empty.json" --gaps "$W/rw.json" >/dev/null 2>&1
python3 -c "
import json
g={x['id']:x['waiver'] for x in json.load(open('$W/rw.json'))['gaps']}
assert g['surface::globalThis.console.[Symbol(kIsConsole)]']=='W1', g
assert g['surface::console.Console.[Symbol(Symbol.hasInstance)]'] is None, g
assert g['surface::WebAssembly.Tag'] is None, g
assert g['surface::globalThis.process.[Symbol(kCapture)]'] is None, g
assert g['surface::EventTarget.[Symbol(nodejs.event_target)]'] is None, g
" && echo "ok   checked-in waivers: ES symbol real, kIsConsole W1, WebAssembly.Tag/kCapture/event_target real" || { echo "FAIL checked-in waivers classification"; fail=1; }
# malformed waivers files are harness errors (exit 2)
printf '{"waivers":[{"id":"W1","reason":"","match":["c::x"]}]}\n' > "$W/bw1.json"
printf '{"waivers":[{"id":"W1","reason":"a","match":["c::x"]},{"id":"W1","reason":"b","match":["c::y"]}]}\n' > "$W/bw2.json"
printf '{"waivers":[{"id":"X1","reason":"a","match":["c::x"]}]}\n' > "$W/bw3.json"
for b in bw1 bw2 bw3; do "${DIFF[@]}" --waivers "$W/$b.json" --baseline "$W/empty.json" >/dev/null 2>&1; check "bad waivers file $b is harness error" 2 $?; done
rm -f "$W"/node/*.txt "$W"/thaw/*.txt

if [ $fail -eq 0 ]; then echo "selftest: PASS"; else echo "selftest: FAIL"; fi
exit $fail
