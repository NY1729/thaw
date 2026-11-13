#!/usr/bin/env bash
# Self-test for diff.py, threeway.py and the builtin table: synthetic outputs prove the gates fail on a missing member.
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

# 8. threeway.py: one probe per class (and one node-missing), checked row by row in the table
TW=(python3 "$HERE/threeway.py")
mkdir -p "$W/tw"
printf '@@ F3.a\n1\n@@ F3.b\n2\n@@ F5.c\n3\n@@ F6.d\n4\n@@ F5.e\n5\n' > "$W/tw/node.txt"
printf '@@ F3.a\n1\n@@ F3.b\n2\n@@ F5.c\n0\n@@ F6.d\n9\n@@ F5.e\n7\n' > "$W/tw/qjs.txt"
printf '@@ F3.a\n1\n@@ F3.b\n0\n@@ F5.c\n0\n@@ F6.d\n8\n@@ F5.e\n5\n@@ X.extra\n1\n' > "$W/tw/thaw.txt"
"${TW[@]}" --node "$W/tw/node.txt" --qjs "$W/tw/qjs.txt" --thaw "$W/tw/thaw.txt" --out "$W/tw/table.txt" >"$W/tw/out" 2>&1 ; check "threeway exits 0" 0 $?
for row in "F3.a | 1 | 1 | 1 | SAME-AS-NODE" "F3.b | 2 | 2 | 0 | THAWJS-REGRESSION" "F5.c | 3 | 0 | 0 | BOTH-DIFFER-SAME" \
           "F6.d | 4 | 9 | 8 | BOTH-DIFFER" "F5.e | 5 | 7 | 5 | QJS-ONLY-BAD" "X.extra | <no line> | <no line> | 1 | NODE-MISSING"; do
  grep -qxF "$row" "$W/tw/table.txt"; check "threeway row: $row" 0 $?
done
"${TW[@]}" --node "$W/tw/node.txt" --qjs "$W/tw/qjs.txt" --thaw "$W/tw/missing.txt" >/dev/null 2>&1 ; check "threeway missing input is harness error" 2 $?

# 9. builtin table: generator is deterministic; the staleness check passes on the checked-in table and fails on a mutated copy
GEN="$HERE/gen_builtin_table.js"; CHK="$HERE/check_builtin_table.sh"
TSV="$HERE/../../crates/thaw-js/src/builtin_table.tsv"
node "$GEN" "$W/bt1.tsv" && node "$GEN" "$W/bt2.tsv" && cmp -s "$W/bt1.tsv" "$W/bt2.tsv"; check "builtin table generator is deterministic" 0 $?
bash "$CHK" >/dev/null 2>&1; check "checked-in builtin_table.tsv is current" 0 $?
sed '10s/$/\tmutated/' "$TSV" > "$W/bt-mut-cell.tsv"; bash "$CHK" "$W/bt-mut-cell.tsv" >/dev/null 2>&1; check "check fails on a mutated cell" 1 $?
sed '10d' "$TSV" > "$W/bt-mut-row.tsv"; bash "$CHK" "$W/bt-mut-row.tsv" >/dev/null 2>&1; check "check fails on a deleted row" 1 $?
bash "$CHK" "$W/no-such-table.tsv" >/dev/null 2>&1; check "check on a missing table is harness error" 2 $?

if [ $fail -eq 0 ]; then echo "selftest: PASS"; else echo "selftest: FAIL"; fi
exit $fail
