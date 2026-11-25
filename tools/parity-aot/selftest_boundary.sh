#!/usr/bin/env bash
# Self-test for gen_boundary.py (no thaw build): generation is deterministic, every probe id is
# <module>.<fn>.<slot>.<shape>.<origin> and unique, every fs path slot has the URL shape in every origin that applies,
# and every program (ci profile) runs under node twice with identical output, every header present, and a TREE
# snapshot after each probe. Called from selftest.sh.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${PARITY_AOT_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp/parity-aot-scratch}}
NODE=${NODE:-node}
mkdir -p "$BASE"
W=$(mktemp -d "$BASE/selftest-boundary.XXXXXX")
fail=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 (rc=$3)"; else echo "FAIL $1 expected rc=$2 got rc=$3"; fail=1; fi; }

python3 "$HERE/gen_boundary.py" "$W/a" --profile ci > "$W/a.log"; check "boundary generate (run 1)" 0 $?
python3 "$HERE/gen_boundary.py" "$W/b" --profile ci > "$W/b.log"; check "boundary generate (run 2)" 0 $?
diff -r "$W/a" "$W/b" > "$W/diff.txt"; check "boundary output is byte-identical across runs" 0 $?

python3 - "$W/a" "$NODE" <<'PY'
import concurrent.futures as cf, json, os, re, subprocess, sys
d, node = sys.argv[1], sys.argv[2]
m = json.load(open(os.path.join(d, "manifest.json")))
ids = [p["name"] for prog in m["programs"] for p in prog["probes"]]
assert len(ids) == len(set(ids)) and all(len(i.split(".")) >= 5 for i in ids)
for fn in ("fs.rmSync_file", "fs.copyFileSync", "fsp.cp", "fsp.mkdir", "fsp.copyFile"):
    for origin in ("any", "direct", "box", "eval"):
        assert f"{fn}.0.url.{origin}" in ids, (fn, origin)
print(f"ok   {len(ids)} ids unique, URL shape present for the path functions in every origin")
flags = [] if tuple(int(x) for x in re.match(r"v(\d+)\.(\d+)", subprocess.run([node, "--version"], capture_output=True, text=True).stdout).groups()) >= (22, 18) else ["--experimental-strip-types"]
def run_once(prog, tag):
    tmp = os.path.join(d, "pa", prog["name"])  # same dir both runs, as aot_run.py does for both sides
    os.makedirs(tmp, exist_ok=True)
    p = subprocess.run([node, *flags, os.path.join(d, prog["name"] + ".mts")], cwd=tmp, capture_output=True, text=True,
                       timeout=180, env=dict(os.environ, PA_TMP=tmp, TZ="UTC", LANG="C"))
    return p.returncode, p.stdout, p.stderr
def check_prog(prog):
    rc1, o1, e1 = run_once(prog, 1)
    rc2, o2, _ = run_once(prog, 2)
    if rc1 != 0:
        return f"{prog['name']}: node rc {rc1}: {e1.strip()[:200]}"
    if o1 != o2:
        return f"{prog['name']}: output differs between two node runs"
    if re.findall(r"^@@ (.+)$", o1, flags=re.M) != [p["name"] for p in prog["probes"]]:
        return f"{prog['name']}: headers differ from probes"
    if o1.count("\nTREE\n") != len(prog["probes"]):
        return f"{prog['name']}: a probe lacks its TREE snapshot"
    return None
with cf.ThreadPoolExecutor(max_workers=6) as ex:
    errors = [e for e in ex.map(check_prog, m["programs"]) if e]
if errors:
    print("\n".join(errors[:20])); sys.exit(1)
print(f"ok   {len(m['programs'])} boundary programs run under node twice, identical, headers and TREE present")
PY
check "boundary programs under node" 0 $?
python3 - "$HERE" <<'PY'
import os, re, sys
sys.path.insert(0, sys.argv[1])
import gen_boundary
wf = open(os.path.join(sys.argv[1], "..", "..", ".github", "workflows", "parity-aot.yml"), encoding="utf-8").read()
keys = [k for m in re.findall(r'\{ shard: b\d, modules: "([^"]+)" \}', wf) for k in m.split(",")]
assert sorted(keys) == sorted(gen_boundary.DEFAULT_MODULES), (sorted(keys), sorted(gen_boundary.DEFAULT_MODULES))
assert "parity-aot-boundary" in wf and "boundary-shards" in wf
print(f"ok   workflow boundary shards cover each of the {len(keys)} module keys once")
PY
check "workflow boundary shards" 0 $?
rm -rf "$W"
exit $fail
