#!/usr/bin/env bash
# Self-test for tools/parity-aot (no thaw build): the generator is deterministic, every generated program runs
# to completion under node with every probe header present (no probe escapes its try/catch), the
# classification and diagnostic-mapping helpers behave on synthetic inputs, and coverage.py counts exactly
# on a synthetic manifest and its ratchet fails only on untouched pairs missing from the baseline.
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
shape = re.compile(r'^pt?\("([^"]+)", \(\) => \{ .* \}\);$')
for prog in m["programs"]:
    assert len(prog["probes"]) <= m["per_program"], prog["name"]
    for p in prog["probes"]:
        assert shape.match(p["line"]), p["line"]
    ts = open(os.path.join(d, prog["name"] + ".ts")).read()
    mts = open(os.path.join(d, prog["name"] + ".mts")).read()
    assert len(re.findall(r'\n  pt?\("', ts)) == len(prog["probes"]), prog["name"]
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
assert a.classify_ran("x", "THROW TypeError: a", "THROW TypeError: b")[1] == "node THROW TypeError: a | thaw THROW TypeError: b"
assert a.expected_reject("THROW TypeError: 1 is not a function", "error: array predicate/reducer must be an arrow or function value")
assert not a.expected_reject("number 1", "error: array predicate/reducer must be an arrow or function value")
assert not a.expected_reject("THROW TypeError: a", "error: unsupported type annotation TsTypeQuery")
class P(a.Program):
    def __init__(self):
        self.prog = {"module": False}
texts = ['p("a.b", () => { return 1; });', 'p("c.d", () => { return 2; });']
p = P()
assert p.name_from_diag("error: x\n  --> g1.ts:2:5", "g1", texts, ["a.b", "c.d"]) == "c.d"
assert p.name_from_diag("error: x\n  --> other.ts:2:5", "g1", texts, ["a.b", "c.d"]) is None
assert p.name_from_diag("error: no location", "g1", texts, ["a.b", "c.d"]) is None
two = "error: x\n  --> g1.ts:1:5\nerror: y\n  --> g1.ts:2:5\nnote: see\n  --> g1.ts:1:5"
assert p.sites_from_diag(two, "g1", texts, ["a.b", "c.d"]) == ["a.b", "c.d"]  # error blocks only; the note is ignored
assert p.sites_from_diag("warning: w\n  --> g1.ts:2:5", "g1", texts, ["a.b", "c.d"]) == ["c.d"]  # no error block: first match
assert p.sites_from_diag("error: x\n  --> g1.ts:2:5", "g1", texts, ["a.b"]) == []  # named probe not in the group
print("ok   classify, diagnostic-to-probe mapping")
PY
check "classification and diagnostic mapping" 0 $?

# 5b. isolation on a fake compiler (no thaw): every error site of one failing build is dropped in one pass, so
#     the build count is one per failing build plus the clean rebuild, and a 150-reject program stays inside EXTRA_CAP
python3 - "$HERE" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import aot_run as a
class Fake(a.Program):
    def __init__(self, bad):
        self.prog = {"module": False}
        self.bad, self.builds, self.n, self.rejects, self.unisolated = set(bad), 0, 0, {}, []
    def build(self, names):
        self.builds += 1
        texts = [f'p("{n}", () => {{ return 1; }});' for n in names]
        hit = [n for n in names if n in self.bad]
        if not hit:
            return True, "", "g", texts
        diag = "".join(f"error: bad probe\n  --> g.ts:{names.index(n) + 1}:3\n" for n in hit)
        return False, diag, "g", texts
names = [f"p{i:03d}" for i in range(160)]
f = Fake(names[:150])                 # 150 bad probes, 10 good
groups = f.solve(names)
assert [n for g, _ in groups for n in g] == names[150:], "good probes not compiled"
assert len(f.rejects) == 150 and f.builds == 2, (len(f.rejects), f.builds)
assert f.unisolated == []
g = Fake(["p001"])
assert g.solve(["p000", "p001", "p002"]) and g.builds == 2 and list(g.rejects) == ["p001"]
print("ok   isolation: 150 rejects dropped in 2 builds (one pass), 10 probes compiled, nothing unisolated")
PY
check "isolation drops all named error sites in one pass" 0 $?

# C0. modules.txt is the THAW_JS_BUILTIN_MODULES allowlist, copied exactly (source present in CI and the repo)
python3 - "$HERE" <<'PY'
import os, re, sys
here = sys.argv[1]
src = os.path.join(here, "..", "..", "crates", "thaw-cli", "src", "registry_integration", "thaw_js_routing.rs")
text = open(src, encoding="utf-8").read()
body = text[text.index("const THAW_JS_BUILTIN_MODULES"):]
body = body[:body.index("];")]
want = re.findall(r'^\s*"([^"]+)",$', body, re.M)
have = [l.strip() for l in open(os.path.join(here, "modules.txt"), encoding="utf-8") if l.strip()]
assert want and have == want, (len(want), len(have))
print(f"ok   modules.txt matches the allowlist ({len(have)} modules)")
PY
check "modules.txt equals the thaw-js builtin allowlist" 0 $?

# C1. coverage on a synthetic 3-probe manifest with a tiny table: exact counts (hand-computed)
C="$HERE/coverage.py"; CV="$W/cov"; mkdir -p "$CV"
python3 - "$CV" <<'PY'
import json, os, sys
d = sys.argv[1]
hdr = "owner key kind flags vtype length name hasPrototype alias tag".split()
rows = [("Math", "max", "data", "w-c", "function", "2", "max", "0", "-", "-"),
        ("Math", "min", "data", "w-c", "function", "2", "min", "0", "-", "-"),
        ("Math", "LN2", "data", "---", "number", "-", "-", "-", "-", "-"),
        ("Map.prototype", "size", "accessor", "--c", "-", "-", "get size|-", "-", "-", "-"),
        ("Array.prototype", "map", "data", "w-c", "function", "1", "map", "0", "-", "-"),
        ("%ArrayIteratorPrototype%", "next", "data", "w-c", "function", "0", "next", "0", "-", "-"),
        ("Array.prototype", "constructor", "data", "w-c", "function", "1", "Array", "0", "-", "-")]
open(os.path.join(d, "table.tsv"), "w").write("\t".join(hdr) + "\n" + "".join("\t".join(r) + "\n" for r in rows))
open(os.path.join(d, "globals.json"), "w").write('[["Foo","function"]]')
open(os.path.join(d, "modules.txt"), "w").write("")
prog = {"name": "math.typed.c00", "corpus": "math.typed", "family": "math", "mode": "typed", "module": False,
        "probes": [{"name": "a", "line": "", "api": ["Math.max"], "mode": "typed"},
                   {"name": "b", "line": "", "api": ["Math.max", "global:Foo"], "mode": "any"},
                   {"name": "c", "line": "", "api": ["Map.prototype.size"], "mode": "module"}]}
json.dump({"probes": 3, "programs": [prog]}, open(os.path.join(d, "manifest.json"), "w"))
PY
syn() { python3 "$C" --manifest "$CV/manifest.json" --table "$CV/table.tsv" --globals "$CV/globals.json" \
  --modules "$CV/modules.txt" --no-hand --baseline "$1" --out "$CV" "${@:2}"; }
syn "$CV/base.json" --write-baseline > /dev/null; check "synthetic: baseline written" 0 $?
python3 - "$CV/coverage.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["universe"]["total"] == 5 and d["universe"]["u1"] == 4 and d["universe"]["u2"] == 1 and d["universe"]["u3"] == 0
assert d["overall"]["touched"] == 3 and d["overall"]["total"] == 5
pm = {m: (c["touched"], c["total"]) for m, c in d["per_mode"].items()}
assert pm == {"typed": (1, 5), "any": (2, 5), "union": (0, 5), "unknown": (0, 5), "generic": (0, 5),
              "callback": (0, 5), "json": (0, 3), "module": (1, 5)}, pm
assert d["per_family"]["math"]["typed"] == {"touched": 1, "total": 2, "pct": 50.0}
assert d["per_family"]["globals"]["any"]["touched"] == 1
assert d["per_family"]["map-set-weak"]["module"]["touched"] == 1
assert sum(len(v) for v in d["untouched_by_mode"].values()) == 34
print("ok   synthetic counts: universe 5, overall 3/5, per-mode and per-family cells, 34 untouched pairs")
PY
check "synthetic: exact touched/total counts" 0 $?

# C2. ratchet: equal baseline passes; a removed baseline pair fails (rc 1); a stale extra pair passes; a new
#     table row fails (rc 1); a subset run with --no-ratchet never fails on the ratchet.
syn "$CV/base.json" > /dev/null; check "ratchet: equal baseline" 0 $?
python3 - "$CV/base.json" "$CV/base-minus.json" "$CV/base-stale.json" <<'PY'
import json, sys
b = json.load(open(sys.argv[1]))
json.dump(dict(b, untouched=[p for p in b["untouched"] if p != "Math.min#typed"]), open(sys.argv[2], "w"))
json.dump(dict(b, untouched=sorted(b["untouched"] + ["Gone.x#typed"])), open(sys.argv[3], "w"))
PY
syn "$CV/base-minus.json" > /dev/null; check "ratchet: pair missing from baseline fails" 1 $?
syn "$CV/base-stale.json" > /dev/null; check "ratchet: stale extra pair passes" 0 $?
cp "$CV/table.tsv" "$CV/table.orig"; printf 'Math\tabs\tdata\tw-c\tfunction\t1\tabs\t0\t-\t-\n' >> "$CV/table.tsv"
syn "$CV/base.json" > /dev/null; check "ratchet: new table row fails" 1 $?
cp "$CV/table.orig" "$CV/table.tsv"
syn "$CV/base-minus.json" --no-ratchet > /dev/null; check "ratchet: --no-ratchet ignores it" 0 $?

# C3. real manifest: every api id is in the universe (exit 2 otherwise), every probe carries api, and the
#     committed baseline holds (node major 22 only; other majors print a skip and pass)
python3 "$C" --manifest "$W/a/manifest.json" --out "$W/cov-real" --summary "$W/cov-real/summary.md" > "$W/cov-real.log" 2>&1
check "real manifest: ids in universe, ratchet vs coverage-baseline.json" 0 $?
tail -n 3 "$W/cov-real.log"
python3 - "$W/cov-real/coverage.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["probes_with_api"] == d["probes"], (d["probes_with_api"], d["probes"])
print(f"ok   every probe carries api ({d['probes']} probes, universe {d['universe']['total']})")
PY
check "real manifest: every probe has api" 0 $?

# C4. skip list: no probe calls a clock, random or a no-arg Date (their values differ between runs)
python3 - "$W/a/manifest.json" <<'PY'
import json, re, sys
bad = re.compile(r"Math\.random\(|Date\.now\(|new Date\(\)")
m = json.load(open(sys.argv[1]))
hits = [p["line"] for prog in m["programs"] for p in prog["probes"] if bad.search(p["line"])]
assert not hits, hits[:3]
print("ok   no probe calls Math.random, Date.now or new Date()")
PY
check "skip list: no clock or random probes" 0 $?

# C5. PRELUDE (compiled into every program) calls no probe-target API except the declared HELPER_DEPS: a reject of
#     an undeclared one masks every probe.
python3 - "$W/a/manifest.json" "$HERE" <<'PY'
import json, re, sys
manifest, here = sys.argv[1], sys.argv[2]
sys.path.insert(0, here)
import gen_probes as g
DOTTED = re.compile(r"\b[A-Z]\w*(?:\.prototype)?\.\w+")
assert DOTTED.findall("x = ArrayBuffer.isView(y); z = Buffer.isBuffer(w);") == ["ArrayBuffer.isView", "Buffer.isBuffer"]
m = json.load(open(manifest))
targets = {api for prog in m["programs"] for p in prog["probes"] for api in p["api"] if "." in api and ":" not in api}
assert "ArrayBuffer.isView" in targets, "ArrayBuffer.isView probe missing"
mentions = set(DOTTED.findall(g.PRELUDE))
bad = sorted((mentions & targets) - set(g.HELPER_DEPS))
stale = sorted(set(g.HELPER_DEPS) - mentions)
assert not bad, f"PRELUDE calls undeclared probe-target APIs (add to HELPER_DEPS or replace): {bad}"
assert not stale, f"HELPER_DEPS lists APIs PRELUDE no longer mentions: {stale}"
print(f"ok   PRELUDE probe-target dependencies are exactly HELPER_DEPS {sorted(mentions & targets)}")
PY
check "PRELUDE has no undeclared probe-target API" 0 $?

# C6. the workflow shard matrix (regex, no PyYAML needed on the runner): each generated family in exactly one shard
python3 - "$HERE/../../.github/workflows/parity-aot.yml" "$W/a/manifest.json" <<'PY'
import json, re, sys
wf, manifest = sys.argv[1], sys.argv[2]
text = open(wf, encoding="utf-8").read()
shards = re.findall(r'- shard: (\w+)\s*\n\s*families: "([^"]*)"', text)
assert [s for s, _ in shards] == ["s1", "s2", "s3", "s4"], shards
listed = [f for _, fams in shards for f in fams.split(",")]
dup = sorted({f for f in listed if listed.count(f) > 1})
assert not dup, f"family in more than one shard: {dup}"
gen = {p["family"] for p in json.load(open(manifest))["programs"]}
assert set(listed) == gen, (sorted(set(listed) - gen), sorted(gen - set(listed)))
print(f"ok   shard matrix: 4 shards, {len(listed)} families, each generated family in exactly one shard")
PY
check "shard matrix covers every generated family exactly once" 0 $?

# D. generator-side checks (volume, skip list, api resolution, --families and --max-probes): selftest_gen.sh
bash "$HERE/selftest_gen.sh"; check "generator self-test (selftest_gen.sh)" 0 $?

# M. module surface (PR1): the d.ts parser (21 inline fixtures; plus all builtin d.ts when a dump dir is given)
#    and the committed module_table.tsv against a regenerated one (format errors and drift fail, version rows warn)
bash "$HERE/selftest_parse.sh"; check "module surface: d.ts parser fixtures (selftest_parse.sh)" 0 $?
bash "$HERE/check_module_table.sh"; check "module surface: module_table.tsv is up to date (check_module_table.sh)" 0 $?

[ "$fail" = 0 ] && echo "selftest: all checks passed" || echo "selftest: FAILED"
exit "$fail"
