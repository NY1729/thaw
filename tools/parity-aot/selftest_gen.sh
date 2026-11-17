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
shape = re.compile(r'^pt?\("([^"]+)", \(\) => \{ .* \}\);$')
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
    assert len(re.findall(r'\n  pt?\("', ts)) == len(prog["probes"]), prog["name"]
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
skip_txt = [re.compile(l.strip()) for l in open(os.path.join(here, "skip.txt"), encoding="utf-8")
            if l.strip() and not l.lstrip().startswith("#")]
names = [p["name"] for prog in m["programs"] for p in prog["probes"]]
dropped = [n for n in names if any(rx.search(n) for rx in skip_txt)]
assert not dropped, dropped[:5]
assert m["skipped"] > 0, "skip.txt drops nothing: the documented skips are not applied"
print(f"ok   no skipped api id, probe name or body; skip.txt drops {m['skipped']} probe(s) at generation")
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

# 7. gen_modules.py (module surface, shapes 1-3): deterministic on the committed d.ts fixture (testdata/dts, seven
#    builtins; other modules are NO-DTS-FILE skips). No thaw.
FIX="$HERE/testdata/dts"
MOD_CONF=(--dts "$FIX")
python3 "$HERE/gen_modules.py" "$W/m1" "${MOD_CONF[@]}" > "$W/m1.log" 2> "$W/m1.err"; check "gen_modules (run 1)" 0 $?
python3 "$HERE/gen_modules.py" "$W/m2" "${MOD_CONF[@]}" > "$W/m2.log" 2> "$W/m2.err"; check "gen_modules (run 2)" 0 $?
diff -r "$W/m1" "$W/m2" > "$W/mdiff.txt"; check "gen_modules output is byte-identical across runs" 0 $?

# 8. gen_modules invariants: every probe is one p() line with a single mod: id; every argument type is a POOL key and
#    its pool expression appears in the line; probe names unique; every universe id (Object.keys of each module, the
#    coverage.py universe) is probed or skipped; every probed id is in the universe; every skip has a known category
#    and a reason; no export-level skip rule matched nothing; a second check of the .ts and .mts files.
python3 - "$W/m1" "$HERE" <<'PY'
import json, os, re, subprocess, sys
d, here = sys.argv[1], sys.argv[2]
sys.path.insert(0, here)
import gen_modules as g
m = json.load(open(os.path.join(d, "manifest.json")))
sk = json.load(open(os.path.join(d, "skips.json")))
mods = [l.strip() for l in open(os.path.join(here, "modules.txt")) if l.strip()]
out = subprocess.run([os.environ.get("NODE", "node"), "-e",
                      "const o={};for(const m of process.argv.slice(1)){o[m]=Object.keys(require(m));}"
                      "process.stdout.write(JSON.stringify(o));", *mods], capture_output=True, text=True, check=True)
universe = {f"mod:{k.removeprefix('node:')}.{e}" for k, es in json.loads(out.stdout).items() for e in es}
probed, names, pool_bad, line_bad = set(), [], [], []
shape = re.compile(r'^p\("([^"]+)", \(\) => \{ .* \}\);$')
for prog in m["programs"]:
    for p in prog["probes"]:
        names.append(p["name"])
        if not shape.match(p["line"]) or len(p["api"]) != 1 or not p["api"][0].startswith("mod:"):
            line_bad.append(p["name"])
        if p["mode"] not in ("typed", "any"):
            line_bad.append(p["name"])
        for key in p["arg_types"]:
            if key not in g.POOL or g.POOL[key] not in p["line"]:
                pool_bad.append((p["name"], key))
        probed.add(p["api"][0])
assert len(names) == len(set(names)), "duplicate probe names"
assert not line_bad, line_bad[:5]
assert not pool_bad, pool_bad[:5]
assert probed <= universe, sorted(probed - universe)[:5]
skipped = {e["id"] for e in sk["exports"]}
assert universe <= probed | skipped, sorted(universe - probed - skipped)[:5]
assert all(e["category"] in g.CATEGORIES and e["reason"] for e in sk["exports"] + sk["members"]), "skip without reason"
assert not sk["unused_rules"], sk["unused_rules"]
print(f"ok   {len(names)} probes, {len(probed)} probed ids, {len(skipped)} skipped ids; universe {len(universe)} "
      f"covered, every skip has a category and reason, no stale rule")
PY
check "gen_modules invariants (ids, pool, completeness, skips)" 0 $?

# 9. gen_modules programs are syntactically valid TypeScript (node --check strips types) and run to completion under
#    node twice with identical output and every header present (no thaw). Type errors are not checked: no tsc here.
python3 - "$W/m1" "$NODE" <<'PY'
import concurrent.futures as cf, json, os, re, subprocess, sys
d, node = sys.argv[1], sys.argv[2]
m = json.load(open(os.path.join(d, "manifest.json")))
def syntax(name):
    for suffix in (".ts", ".mts"):
        r = subprocess.run([node, "--check", os.path.join(d, name + suffix)], capture_output=True, text=True)
        if r.returncode != 0:
            return f"{name}{suffix}: {r.stderr.strip()[:200]}"
    return None
def run(name):
    path = os.path.join(d, name + ".mts")
    a = subprocess.run([node, path], cwd=d, capture_output=True, text=True, timeout=120)
    b = subprocess.run([node, path], cwd=d, capture_output=True, text=True, timeout=120)
    if a.returncode != 0:
        return f"{name}: node rc {a.returncode}"
    if a.stdout != b.stdout:
        return f"{name}: output differs between two node runs"
    heads = re.findall(r"^@@ (.+)$", a.stdout, flags=re.M)
    want = [p["name"] for p in next(q for q in m["programs"] if q["name"] == name)["probes"]]
    return None if heads == want else f"{name}: headers {len(heads)} != probes {len(want)}"
names = [p["name"] for p in m["programs"]]
with cf.ThreadPoolExecutor(max_workers=6) as ex:
    errors = [e for e in ex.map(syntax, names) if e] + [e for e in ex.map(run, names) if e]
if errors:
    print("\n".join(errors[:10]))
    sys.exit(1)
print(f"ok   {len(names)} programs: .ts and .mts pass node --check; run twice, identical, headers present")
PY
check "gen_modules programs: valid TypeScript, run to completion" 0 $?

# 10. coverage scores the gen_modules manifest: every probed id is touched (not in untouched_any_mode)
CMV="$W/cov-mod"; mkdir -p "$CMV"
python3 "$HERE/coverage.py" --manifest "$W/m1/manifest.json" --out "$CMV" --no-ratchet > "$CMV.log" 2>&1
check "coverage.py accepts the gen_modules manifest" 0 $?
python3 - "$W/m1/manifest.json" "$CMV/coverage.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
cov = json.load(open(sys.argv[2]))
probed = {p["api"][0] for prog in m["programs"] for p in prog["probes"]}
untouched = set(cov["untouched_any_mode"])
bad = sorted(probed & untouched)
assert not bad, bad[:5]
assert cov["unknown_api_ids"] == [], cov["unknown_api_ids"][:5]
print(f"ok   {len(probed)} probed module ids are covered (not untouched in any mode), no unknown api ids")
PY
check "module ids probed by gen_modules count as covered" 0 $?

# 11. two manifests scored together: gen_probes + gen_modules, probes add up
python3 "$HERE/gen_probes.py" "$W/p" --families array > /dev/null 2>&1
python3 "$HERE/coverage.py" --manifest "$W/p/manifest.json" --manifest "$W/m1/manifest.json" --out "$W/cov-both" \
  --no-ratchet > /dev/null 2>&1
check "coverage.py scores two manifests together" 0 $?

[ "$fail" = 0 ] && echo "selftest-gen: all checks passed" || echo "selftest-gen: FAILED"
exit "$fail"
