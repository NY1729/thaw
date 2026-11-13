#!/usr/bin/env bash
# Self-test for diff.py: synthetic node/thaw outputs prove the gate fails on a missing member.
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

if [ $fail -eq 0 ]; then echo "selftest: PASS"; else echo "selftest: FAIL"; fi
exit $fail
