#!/usr/bin/env bash
# Self-test for tools/failure-conformance (no thaw): generation is deterministic, case ids are unique and each is in
# exactly one program, every case runs under Node without timing out and with a stable result across two runs,
# and the normaliser / classifier behave on synthetic inputs (an `Uncaught opaque exception` must classify as a DIFF).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${FC_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp}/failure-conformance-selftest}
rm -rf "$BASE"; mkdir -p "$BASE"
fail=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected $2 got $3)"; fail=1; fi; }

python3 "$HERE/gen.py" "$BASE/a" > /dev/null; check "generate" 0 $?
python3 "$HERE/gen.py" "$BASE/b" > /dev/null
diff -r "$BASE/a" "$BASE/b" > /dev/null; check "generation is byte-identical" 0 $?
python3 - "$BASE/a/manifest.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
ids = [c["id"] for p in m["programs"] for c in p["cases"]]
assert len(ids) == len(set(ids)) == m["cases"], "duplicate or miscounted case ids"
assert all(len(p["cases"]) <= 40 for p in m["programs"])
print(f"{len(ids)} cases in {len(m['programs'])} programs")
PY
check "unique ids, program sizes" 0 $?

python3 "$HERE/run.py" --out "$BASE/n1" --node-only > "$BASE/n1.log" 2>&1; check "node runs every case (no timeout)" 0 $?
python3 "$HERE/run.py" --out "$BASE/n2" --node-only > "$BASE/n2.log" 2>&1
diff "$BASE/n1/node-only.tsv" "$BASE/n2/node-only.tsv" > "$BASE/stable.diff"; check "node results are stable across two runs" 0 $?
head -20 "$BASE/stable.diff"
if grep -P '\tSyntaxError|\tReferenceError|unknown case|ERR_UNKNOWN_FILE_EXTENSION' "$BASE/n1/node-only.tsv" | grep -v -P '^(syserr|messages|limits|worker)\.' > "$BASE/bad.txt"; then
  echo "FAIL a program itself is broken under node:"; head "$BASE/bad.txt"; fail=1
else echo "ok   no harness-level node errors"; fi

python3 -I - "$HERE" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import run
P = ["/w/prog.mts", "/w"]
node_err = ("file:///w/prog.mts:19\nthrow new Error(\"boom\");\n      ^\n\nError: boom\n    at file:///w/prog.mts:19:7\n"
            "    at ModuleJob.run (node:internal/modules/esm/module_job:343:25)\n\nNode.js v22.23.1\n")
assert run.norm(node_err, P) == "Error: boom", repr(run.norm(node_err, P))
s = "Error: x\n    at f (/w/prog.mts:1:2) {\n  code: 'E'\n}\n"
assert run.norm(s, P) == "Error: x {\n  code: 'E'\n}"
assert run.norm("[Error: x] {\n  code: 'E'\n}", P) == "Error: x {\n  code: 'E'\n}"
assert run.norm("{ e: [TypeError: y] }", P) == "{ e: TypeError: y }"
assert run.norm("listen EADDRINUSE :::39179 (node:123)", P) == "listen EADDRINUSE :::PORT (node:PID)"
assert run.norm("throw 1\n(Use `node --trace-uncaught ...` to show where the exception was thrown)", P) == "throw 1"
big = run.norm("x" * 20000, P)
assert len(big) < 400 and "sha256" in big
ok = {"rc": 0, "out": "a\n", "err": "", "timeout": False, "paths": P}
def rr(**kw): return dict(ok, **kw)
assert run.classify(rr(), rr())[0] == "MATCH"
assert run.classify(rr(rc=1, err="Error: boom\n"), rr(rc=1, err="Uncaught: Uncaught opaque exception\n")) == (
    "DIFF", "stderr", "opaque-exception")
assert run.classify(rr(rc=1, err="Error: boom\n"), rr(rc=1))[2] == "no-error-output"
assert run.classify(rr(rc=3), rr(rc=0))[2] == "exit-status"
assert run.classify(rr(), rr(rc=-11))[0] == "RUN-FAIL"
assert run.classify(rr(), rr(timeout=True, rc=-9))[0] == "RUN-FAIL"
assert run.classify(rr(), {"build_fail": "error: x", "paths": []})[0] == "BUILD-FAIL"
assert run.classify(rr(rc=-15), rr(rc=-15))[0] == "MATCH"
assert run.diag_line("Running as unit: run-p1.scope; invocation ID: 9\nnote: x\nerror: unsupported on thaw-js: `a` bad" "Running as unit: y") == "error: unsupported on thaw-js: `a` bad"
PY
check "normaliser and classifier" 0 $?
exit $fail
