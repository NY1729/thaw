#!/usr/bin/env bash
# Self-test for tools/parity-aot (no thaw build): the generator is deterministic, every generated program runs
# to completion under node with every probe header present (no probe escapes its try/catch), and the
# classification and diagnostic-mapping helpers behave on synthetic inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${PARITY_AOT_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp/parity-aot-scratch}}
mkdir -p "$BASE"
W=${PARITY_AOT_SELFTEST_DIR:-$(mktemp -d "$BASE/selftest.XXXXXX")}
rm -rf "$W"; mkdir -p "$W"
fail=0
check() { # name expected_rc actual_rc
  if [ "$2" = "$3" ]; then echo "ok   $1 (rc=$3)"; else echo "FAIL $1 expected rc=$2 got rc=$3"; fail=1; fi
}

# 1. generation is deterministic
python3 "$HERE/gen_probes.py" "$W/a" > "$W/a.log"; check "generate (run 1)" 0 $?
python3 "$HERE/gen_probes.py" "$W/b" > "$W/b.log"; check "generate (run 2)" 0 $?
diff -r "$W/a" "$W/b" > "$W/diff.txt"; check "output is byte-identical across runs" 0 $?

# 2. every program: <= 40 probes, one probe per line, every line matches the probe shape, .mts ends in main();
python3 - "$W/a" <<'PY'
import json, os, re, sys
d = sys.argv[1]
m = json.load(open(os.path.join(d, "manifest.json")))
shape = re.compile(r'^p\("([^"]+)", \(\) => \{ .* \}\);$')
for prog in m["programs"]:
    assert len(prog["probes"]) <= m["per_program"], prog["name"]
    for p in prog["probes"]:
        assert shape.match(p["line"]), p["line"]
    ts = open(os.path.join(d, prog["name"] + ".ts")).read()
    mts = open(os.path.join(d, prog["name"] + ".mts")).read()
    assert ts.count("\n  p(\"") == len(prog["probes"]), prog["name"]
    assert "main();" not in ts and mts.rstrip().endswith("main();"), prog["name"]
print(f"ok   {len(m['programs'])} programs, {m['probes']} probes, shape and line mapping")
PY
check "program shape and probe count" 0 $?

# 3. every program runs to completion under node: rc 0, every header present (aot_run.py --node-only)
python3 "$HERE/aot_run.py" --programs "$W/a" --out "$W/node-out" --node-only > "$W/node.log" 2>&1
check "node runs every program, all headers present" 0 $?
cat "$W/node.log"

# 4. drift findings are emitted (the three ctor lists are compared, the op names are checked)
python3 - "$W/a" <<'PY'
import json, sys, os
m = json.load(open(os.path.join(sys.argv[1], "manifest.json")))
kinds = {f["class"] for f in m["findings"]}
assert "LIST-DRIFT" in kinds, kinds
assert any("DYNAMIC_NEW_CTORS" in f["detail"] for f in m["findings"] if f["class"] == "LIST-DRIFT")
print(f"ok   {len(m['findings'])} findings, classes {sorted(kinds)}")
PY
check "list drift emitted" 0 $?

# 5. classification, diagnostic mapping and reject/crash handling on synthetic inputs
python3 - "$HERE" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import aot_run as a
assert a.classify_ran("x", 'THROW TypeError: a', 'THROW TypeError: a') == ("MATCH", "")
assert a.classify_ran("x", "THROW TypeError: a", "THROW TypeError: b")[0] == "MSG"
assert a.classify_ran("x", "THROW TypeError: a", "THROW RangeError: a")[0] == "WRONG-TYPE"
assert a.classify_ran("x", "number 1", "number 2")[0] == "WRONG-VALUE"
assert a.classify_ran("x", "object {}", "object {__thaw_js_handle_id__:3}")[0] == "LEAK"
assert a.classify_ran("x", "number 1", None)[0] == "MISSING"
class P(a.Program):
    def __init__(self):
        self.prog = {"module": False}
texts = ['p("a.b", () => { return 1; });', 'p("c.d", () => { return 2; });']
p = P()
assert p.name_from_diag("error: x\n  --> g1.ts:2:5", "g1", texts, ["a.b", "c.d"]) == "c.d"
assert p.name_from_diag("error: x\n  --> other.ts:2:5", "g1", texts, ["a.b", "c.d"]) is None
assert p.name_from_diag("error: no location", "g1", texts, ["a.b", "c.d"]) is None
print("ok   classify, diagnostic-to-probe mapping")
PY
check "classification and diagnostic mapping" 0 $?

[ "$fail" = 0 ] && echo "selftest: all checks passed" || echo "selftest: FAILED"
exit "$fail"
