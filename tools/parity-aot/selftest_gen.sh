#!/usr/bin/env bash
# Generator self-test for tools/parity-aot (no thaw build, no cargo): gen_probes.py + gen_builtins.py are
# deterministic, the built-in volume stays <= 6000 probes, every program runs to completion under node twice with
# identical output and every header present, the skip list is honoured, every api id resolves, and the --families and
# --max-probes flags behave. Called from selftest.sh.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${PARITY_AOT_SCRATCH:-${RUNNER_TEMP:-/home/fedora/work/thaw-tmp/parity-aot-scratch}}
NODE=${NODE:-node}
mkdir -p "$BASE"
W=${PARITY_AOT_GEN_SELFTEST_DIR:-$(mktemp -d "$BASE/selftest-gen.XXXXXX")}
rm -rf "$W"; mkdir -p "$W"
fail=0
check() { # name expected_rc actual_rc
  if [ "$2" = "$3" ]; then echo "ok   $1 (rc=$3)"; else echo "FAIL $1 expected rc=$2 got rc=$3"; fail=1; fi
}

# 1. generation is deterministic (byte-identical across two runs, built-in families included)
python3 "$HERE/gen_probes.py" "$W/a" --max-probes 6000 > "$W/a.log"; check "generate with --max-probes 6000" 0 $?
python3 "$HERE/gen_probes.py" "$W/b" --max-probes 6000 > "$W/b.log"; check "generate (run 2)" 0 $?
diff -r "$W/a" "$W/b" > "$W/diff.txt"; check "output is byte-identical across runs" 0 $?

# 2. volume, shape, per-program cap, manifest schema (api list and mode on every probe)
python3 - "$W/a" <<'PY'
import json, os, re, sys
d = sys.argv[1]
m = json.load(open(os.path.join(d, "manifest.json")))
MODES = {"typed", "any", "union", "unknown", "generic", "callback", "json", "module"}
shape = re.compile(r'^p\("([^"]+)", \(\) => \{ .* \}\);$')
assert m["probes"] <= 6000, m["probes"]
seen = set()
for prog in m["programs"]:
    assert len(prog["probes"]) <= m["per_program"], prog["name"]
    for p in prog["probes"]:
        assert shape.match(p["line"]), p["line"]
        assert isinstance(p["api"], list) and p["api"], p["name"]
        assert p["mode"] in MODES, p["name"]
        assert p["name"] not in seen, p["name"]
        seen.add(p["name"])
    ts = open(os.path.join(d, prog["name"] + ".ts")).read()
    mts = open(os.path.join(d, prog["name"] + ".mts")).read()
    assert ts.count("\n  p(\"") == len(prog["probes"]), prog["name"]
    assert not re.search(r"^main\(\);", ts, re.M) and mts.rstrip().endswith("main();"), prog["name"]
    assert [p["api"][0] for p in prog["probes"]] == sorted(p["api"][0] for p in prog["probes"]), prog["name"]
print(f"ok   {m['probes']} probes in {len(m['programs'])} programs, <= 6000 total, api/mode on every row")
PY
check "volume and manifest schema" 0 $?

# 3. every program runs under node twice: rc 0, every header present, identical output both runs
python3 - "$W/a" "$NODE" <<'PY'
import concurrent.futures as cf, json, os, re, subprocess, sys
d, node = sys.argv[1], sys.argv[2]
ver = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
major, minor = (int(x) for x in re.match(r"v(\d+)\.(\d+)", ver).groups())
flags = [] if (major, minor) >= (22, 18) else ["--experimental-strip-types"]
m = json.load(open(os.path.join(d, "manifest.json")))
def run_once(path):
    p = subprocess.run([node, *flags, path], cwd=d, capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout, p.stderr
def check_prog(prog):
    path = os.path.join(d, prog["name"] + ".mts")
    rc1, out1, err1 = run_once(path)
    rc2, out2, _ = run_once(path)
    if rc1 != 0:
        return f"{prog['name']}: node rc {rc1}: {err1.strip()[:200]}"
    if out1 != out2:
        return f"{prog['name']}: output differs between two node runs"
    heads = re.findall(r"^@@ (.+)$", out1, flags=re.M)
    want = [p["name"] for p in prog["probes"]]
    if heads != want:
        return f"{prog['name']}: headers {len(heads)} != probes {len(want)}"
    return None
with cf.ThreadPoolExecutor(max_workers=6) as ex:
    errors = [e for e in ex.map(check_prog, m["programs"]) if e]
if errors:
    print("\n".join(errors[:20]))
    sys.exit(1)
print(f"ok   {len(m['programs'])} programs run to completion under {node} {ver} twice, identical, all headers present")
PY
check "node runs every program twice, identical, headers present" 0 $?

# 4. skip list honoured (api ids, probe names and bodies)
python3 - "$W/a" "$HERE" <<'PY'
import json, os, re, sys
d, here = sys.argv[1], sys.argv[2]
sys.path.insert(0, here)
import gen_builtins as g
m = json.load(open(os.path.join(d, "manifest.json")))
extra = re.compile(r"Math\.random|Date\.now|new Date\(\)|performance\.|setTimeout|\.stack\b")
bad = []
for prog in m["programs"]:
    for p in prog["probes"]:
        for api in p["api"]:
            if g.is_skipped(api):
                bad.append(api)
        if any(rx.search(p["name"]) for rx in g.SKIP_RE) or extra.search(p["line"]):
            bad.append(p["name"])
assert not bad, sorted(set(bad))[:10]
print("ok   no skipped api id, probe name or body")
PY
check "skip list honoured" 0 $?

# 5. every api id resolves: table row, documented hand id, or node_globals.json entry (global:<Name>)
python3 - "$W/a" "$HERE" <<'PY'
import json, os, sys
d, here = sys.argv[1], sys.argv[2]
sys.path.insert(0, here)
import gen_builtins as g
repo = os.path.abspath(os.path.join(here, "..", ".."))
table = g.table_ids(g.load_table(os.path.join(repo, "crates", "thaw-js", "src", "builtin_table.tsv")))
import gen_probes
globals_ = {name for name, _kind in json.load(open(os.path.join(repo, "tools", "parity", "node_globals.json")))}
# ctor_probes draws from the compiler's PLATFORM_GLOBALS Ctor list, which can be newer than node_globals.json
with open(os.path.join(repo, "crates", "thaw-hir", "src", "lower", "platform_globals.rs"), encoding="utf-8") as fh:
    globals_ |= {name for name, kind in gen_probes.parse_platform(fh.read())[0] if kind == "Ctor"}
m = json.load(open(os.path.join(d, "manifest.json")))
unresolved, used = set(), set()
for prog in m["programs"]:
    for p in prog["probes"]:
        for api in p["api"]:
            used.add(api)
            if api.startswith("hand:"):
                ok = api in g.HAND_IDS
            elif api.startswith("global:"):
                ok = api[len("global:"):] in globals_
            else:
                ok = api in table
            if not ok:
                unresolved.add(api)
assert not unresolved, sorted(unresolved)[:10]
print(f"ok   {len(used)} distinct api ids resolve (table, hand, node_globals or PLATFORM_GLOBALS)")
PY
check "every api id resolves" 0 $?

# 6. flags: --families restricts the programs, an unknown family and an over-budget run exit 2
python3 "$HERE/gen_probes.py" "$W/f" --families array,map > "$W/f.log"; check "--families array,map" 0 $?
python3 - "$W/f" <<'PY'
import json, os, sys
m = json.load(open(os.path.join(sys.argv[1], "manifest.json")))
fams = {p["family"] for p in m["programs"]}
assert fams == {"array", "map"}, fams
print(f"ok   --families emits only {sorted(fams)}")
PY
check "--families emits only the requested families" 0 $?
python3 "$HERE/gen_probes.py" "$W/x" --families nosuch > /dev/null 2>&1; check "unknown family exits 2" 2 $?
python3 "$HERE/gen_probes.py" "$W/y" --max-probes 10 > /dev/null 2>&1; check "--max-probes 10 exits 2" 2 $?

[ "$fail" = 0 ] && echo "selftest-gen: all checks passed" || echo "selftest-gen: FAILED"
exit "$fail"
