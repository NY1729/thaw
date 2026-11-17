#!/usr/bin/env python3
"""Module-surface probes for tools/parity-aot, shapes 1-3: sync functions, constants/primitives/objects/accessors, classes.

Inputs: module_table.tsv (module_inv.js under Node 22: which exports exist and their runtime kind and vtype),
the ambient d.ts of each module (dumped from resolve_builtin by crates/thaw-registry/tests/dump_builtin_dts.rs and
parsed by dts_parse.py) and modules.txt. Types come ONLY from the d.ts, never from Node values, so a thaw REJECT of
a probe is a typing or lowering defect, not a probe artefact. Argument values come from the fixed POOL below, one
expression per TS type, and a probe only passes arguments whose types are in POOL.

Output (OUT_DIR): mod_<module>.<mode>.cNN.ts (thaw copy) and .mts (node copy), manifest.json (gen_probes schema plus
`imports` per program and `arg_types` per probe) and skips.json (every export that gets no probe, with a category
and a reason, plus member-level skips). Probe lines are `p("<name>", () => { <body> });` and use gen_probes.PRELUDE,
so show() and the @@ headers are the same as gen_probes output. The only difference is the `import * as M` line,
see README "Module surface" (aot_run.py must render `imports`).

Modes: `typed` (M.x through the d.ts type) and `any` (`const m: any = M`, functions only). Shapes:
  function  -> `return M.f(args)` (value) or the shape tail (nondeterministic exports)
  constant/primitive with number, string or boolean -> `return M.x`; objects, accessors -> shape tail
  class     -> `new M.C(args)` (construct) plus one probe per method (instance: `const o: M.C = new M.C(..)`, static)
               and per property
Shape tail: prints [typeof, length or key count] so the call still runs but its value is not compared.

Skips (never silent): a category from CATEGORIES and a reason. Calls with a callback parameter or a Promise return
are deferred to gen_async.py (shapes 4-6) and gen_inspect.py (console output), not probed here.

Usage: gen_modules.py OUT_DIR [--dts DIR] [--table FILE] [--modules FILE] [--per-program N]
       --dts defaults to $PARITY_AOT_DTS_DIR. Dump it with
       THAW_DUMP_DTS_DIR=<dir> cargo test -p thaw-registry --test dump_builtin_dts -- --ignored
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dts_parse  # noqa: E402  (tools/parity-aot/dts_parse.py)
import gen_probes  # noqa: E402  (PRELUDE, HEADER, PER_PROGRAM: the same show() and @@ headers as gen_probes)

MODES = ("typed", "any")
PER_PROGRAM = gen_probes.PER_PROGRAM
DEFAULT_TABLE = os.path.join(HERE, "module_table.tsv")
DEFAULT_MODULES = os.path.join(HERE, "modules.txt")

CATEGORIES = {
    "NEEDS-NET": "opens sockets or resolves names over the network; loopback cases need hand files",
    "NEEDS-PROC": "spawns processes, reads stdin, opens debugger or tracing, or prints to stdout as a harness",
    "NEEDS-FILE": "reads or writes the filesystem; the cwd differs between node and the thaw binary",
    "NEEDS-EVENTLOOP": "keeps the event loop alive or schedules work that prints after main() returns",
    "HANG": "never returns in a batch program (interactive)",
    "PROCESS-STATE": "changes process-wide state (exit, signals, uid, flags) that other probes depend on",
    "NODE-INTERNAL": "underscore-prefixed or non-identifier export: Node-internal, not public API",
    "DEFERRED-ASYNC": "returns a Promise; covered by shape 5 (gen_async.py)",
    "DEFERRED-CALLBACK": "takes a callback; covered by shapes 4 and 6 (gen_async.py)",
    "DEFERRED-STDOUT": "writes to stdout as its output; covered by the inspect family (gen_inspect.py)",
    "NO-DTS": "the export is not declared in the module's d.ts (a typing gap; the probe needs a type)",
    "NO-DTS-FILE": "the module has no d.ts file in the dump directory",
    "DTS-KIND": "the d.ts declares a different kind than the runtime (function, class, const)",
    "ARG-UNPOOLED": "a required parameter has a type with no pool value (generic, unknown name, literal type)",
    "CTOR-UNPOOLED": "the class constructor needs an argument with no pool value",
    "NOT-EXPORT": "table row that Object.keys(require(module)) does not list: outside the coverage universe",
    "NOT-IN-TABLE": "Object.keys lists it but module_table.tsv has no row: the table is stale (rerun module_inv.js)",
}

# Argument pool: TS type key -> JS expression. Every probe argument comes from here (arg_types records the keys).
# Object literals are only used for any/unknown/object/Json/JsValue/Record (no shape constraint, so no excess
# property errors): a typed object parameter with named members is ARG-UNPOOLED instead.
POOL = {
    "string": '"a/b"',
    "number": "1",
    "boolean": "true",
    "string[]": '["a", "b"]',
    "number[]": "[1, 2]",
    "Buffer": "Buffer.from([1, 2, 3, 250])",
    "Buffer[]": "[Buffer.from([1, 2]), Buffer.from([250])]",
    "Uint8Array": "new Uint8Array([1, 2, 3, 250])",
    "object": '{ a: 1 }',
    "array": '[1, "x"]',
}
POOL_TYPES = {
    "string": "string", "number": "number", "boolean": "boolean", "string[]": "string[]",
    "Array<string>": "string[]", "number[]": "number[]", "Array<number>": "number[]",
    "Buffer": "Buffer", "Buffer[]": "Buffer[]", "Array<Buffer>": "Buffer[]", "Uint8Array": "Uint8Array",
    "any": "object", "unknown": "object", "object": "object", "Json": "object", "JsValue": "object",
    "Record<string, any>": "object", "any[]": "array", "unknown[]": "array", "Json[]": "array", "JsValue[]": "array",
}

# Rules, first match wins. (action, module regex, export regex, member regex or None, kinds or None, category, reason)
#   action "skip": no probe; the export (or member) gets the category. action "shape": probed, output masked to the
#   shape tail (typeof, length or key count) because the value depends on the host, the clock, the pid or randomness.
RULES = [
    # ---- shape: environment, clock, randomness (the call still runs)
    ("shape", r"node:os", r"cpus|freemem|totalmem|loadavg|uptime|networkInterfaces|userInfo|hostname|"
     r"availableParallelism|getPriority|homedir|tmpdir", None, ("function",), None,
     "host state: varies with time and machine"),
    ("shape", r"node:path(/.*)?", r"resolve|relative", None, ("function",), None,
     "depends on process.cwd(), which differs between node and the binary"),
    ("shape", r"node:url", r"pathToFileURL", None, ("function",), None, "resolves against process.cwd()"),
    ("shape", r"node:url", r"URL", r"createObjectURL", ("class",), None, "random blob id"),
    ("shape", r"node:crypto", r"random.*|generate.*|pseudoRandom.*|getRandomValues|prng|rng", None,
     ("function", "accessor"), None, "random output"),
    ("shape", r"node:crypto", r"getCiphers|getHashes|getCurves", None, ("function",), None,
     "list depends on the OpenSSL build, not on the API"),
    ("shape", r"node:tls", r"getCiphers", None, ("function",), None, "list depends on the OpenSSL build"),
    ("shape", r"node:buffer", r"Buffer", r"allocUnsafe|allocUnsafeSlow", ("class",), None,
     "uninitialised memory: content is not defined"),
    ("shape", r"node:util", r"getCallSites?", None, ("function",), None, "depends on the call stack"),
    ("shape", r"node:async_hooks", r"executionAsyncId|triggerAsyncId|executionAsyncResource", None,
     ("function",), None, "async ids differ between runs"),
    ("shape", r"node:async_hooks", r"AsyncResource", r"asyncId|triggerAsyncId", ("class",), None,
     "async ids differ between runs"),
    ("shape", r"node:perf_hooks", r".*", None, None, None, "timing values"),
    ("shape", r"node:process", r"pid|ppid|argv|argv0|execPath|execArgv|cwd|title|env|version|versions|release|"
     r"config|hrtime|memoryUsage|cpuUsage|resourceUsage|uptime|getActiveResourcesInfo", None, None, None,
     "host or runtime identity (thaw is not node) or clock"),
    ("shape", r"node:tty", r"isatty", None, ("function",), None, "depends on the stdio of the harness"),
    ("shape", r"node:v8", r"getHeap.*|cachedDataVersionTag|getHeapSnapshot", None, ("function",), None,
     "heap and V8 build numbers"),
    ("shape", r"node:zlib", r".*", None, ("class", "function"), None,
     "stream handles (classes and create* factories) expose their internal buffers, which hold uninitialised memory"),

    # ---- skip: process-wide state, stdin, debugger, tracing
    ("skip", r"node:process", r"abort|chdir|exit|reallyExit|kill|setuid|seteuid|setgid|setegid|setgroups|"
     r"initgroups|dlopen|binding|loadEnvFile|setSourceMapsEnabled|setUncaughtExceptionCaptureCallback|"
     r"disconnect", None, ("function",), "PROCESS-STATE", "changes process state or exits the batch"),
    ("skip", r"node:process", r"stdin|stdout|stderr", None, ("accessor",), "NEEDS-PROC",
     "opens a stream on the harness stdio"),
    ("skip", r"node:os", r"setPriority", None, ("function",), "PROCESS-STATE", "renices another process"),
    ("skip", r"node:v8", r"writeHeapSnapshot|takeCoverage|stopCoverage|setHeapSnapshotNearHeapLimit",
     None, ("function",), "NEEDS-FILE", "writes files or changes heap limits"),
    ("skip", r"node:v8", r"setFlagsFromString|startupSnapshot", None, ("function",), "PROCESS-STATE",
     "changes V8 flags for the whole process"),
    ("skip", r"node:v8", r"GCProfiler", None, ("class",), "PROCESS-STATE", "profiler state for the process"),
    ("skip", r"node:module", r"register|syncBuiltinESMExports|enableCompileCache|flushCompileCache|runMain|"
     r"findPackageJSON|Module", None, ("function", "class"), "NEEDS-FILE",
     "loads hooks or reads and writes module caches on disk"),
    ("skip", r"node:timers", r"enroll|unenroll|active", None, ("function",), "NEEDS-EVENTLOOP",
     "schedules or inspects timers of the event loop"),
    ("skip", r"node:worker_threads", r"Worker|MessageChannel|MessagePort|BroadcastChannel", None, ("class",),
     "NEEDS-EVENTLOOP", "spawns a thread or keeps the loop alive until closed (needs a second file)"),
    ("skip", r"node:vm", r"measureMemory", None, ("function",), "DEFERRED-ASYNC", "returns a Promise"),
    ("skip", r"node:(timers/promises|stream/promises|stream/consumers)", r".*", None, ("function",),
     "DEFERRED-ASYNC", "returns a Promise at runtime (shape 5, gen_async.py); stream/consumers types its result as "
     "JsValue, so the d.ts cannot show it"),
    ("skip", r"node:stream", r"destroy", None, ("function",), "NEEDS-EVENTLOOP",
     "destroy() of a non-stream argument emits 'error' on a later tick, which crashes the batch (needs a stream)"),
    ("skip", r"node:stream", r"Readable|Writable|Duplex|Transform|PassThrough|Stream", None, ("class",),
     "NEEDS-EVENTLOOP", "stream methods emit events on a later tick; an unhandled 'error' crashes the batch "
     "(needs a hand case with an error listener)"),

    # ---- skip: whole modules that need the network, stdin, a debugger, child processes or the filesystem
    ("skip", r"node:child_process", r".*", None, ("function", "class"), "NEEDS-PROC",
     "spawns child processes (fixed argv cases in a hand file)"),
    ("skip", r"node:cluster", r".*", None, ("function", "class"), "NEEDS-PROC", "forks worker processes"),
    ("skip", r"node:dgram", r".*", None, ("function", "class"), "NEEDS-NET", "UDP sockets"),
    ("skip", r"node:dns(/promises)?", r".*", None, ("function", "class"), "NEEDS-NET",
     "name resolution goes to the resolver; loopback cases need a hand file"),
    ("skip", r"node:(http|https|http2|net|tls)", r".*", None, ("function", "class"), "NEEDS-NET",
     "sockets and servers (loopback only, ephemeral port masked, hand files)"),
    ("skip", r"node:(inspector|inspector/promises|trace_events)", r".*", None, ("function", "class"),
     "NEEDS-PROC", "opens the inspector or trace files"),
    ("skip", r"node:repl", r".*", None, ("function", "class"), "HANG", "interactive REPL reads stdin"),
    ("skip", r"node:(readline|readline/promises)", r".*", None, ("function", "class"), "NEEDS-PROC",
     "reads stdin"),
    ("skip", r"node:(test|test/reporters)", r".*", None, ("function", "class"), "NEEDS-PROC",
     "prints TAP to stdout and runs its own event loop"),
    ("skip", r"node:wasi", r".*", None, ("function", "class"), "NEEDS-FILE", "preopens directories"),
    ("skip", r"node:(fs|fs/promises)", r".*", None, ("function", "class"), "NEEDS-FILE",
     "filesystem functions (cwd differs between node and the binary; fixtures need hand/fs.ts)"),
    ("skip", r"node:console", r".*", None, ("function", "class"), "DEFERRED-STDOUT",
     "writes to stdout; the inspect family covers console output"),

]

IDENT_RE = re.compile(r"^[A-Za-z$][A-Za-z0-9_$]*$")


def enumerable_exports(modules, node):
    """{module: set of Object.keys(require(module))}: the coverage universe (coverage.py module_ids). A probe id must be
    one of these, or coverage.py exits 2 on an unknown api id."""
    script = ("const out = {};"
              "for (const m of process.argv.slice(1)) { out[m] = Object.keys(require(m)); }"
              "process.stdout.write(JSON.stringify(out));")
    res = subprocess.run([node, "-e", script, *modules], capture_output=True, text=True)
    if res.returncode != 0:
        raise SystemExit(f"gen_modules: node module enumeration failed: {res.stderr.strip()}")
    return {m: set(keys) for m, keys in json.loads(res.stdout).items()}
SHAPE_TAIL = ('return [typeof r, typeof r === "string" || Array.isArray(r) ? r.length : '
              '(r !== null && typeof r === "object" ? Object.keys(r).length : -1)];')


class Unpoolable(Exception):
    def __init__(self, category, reason):
        super().__init__(reason)
        self.category, self.reason = category, reason


def split_top(text, sep):
    """Split on sep at bracket depth 0 (arrow `=>` is not a bracket)."""
    out, depth, cur, i = [], 0, "", 0
    while i < len(text):
        two = text[i:i + 2]
        c = text[i]
        if two == "=>":
            cur += two
            i += 2
            continue
        if c in "([{<":
            depth += 1
        elif c in ")]}>":
            depth -= 1
        if c == sep and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += c
        i += 1
    out.append(cur)
    return [s.strip() for s in out]


def arg_for(ptype):
    """(pool key, JS expression) for a d.ts parameter type. Raises Unpoolable with the skip category."""
    t = ptype.strip()
    if "=>" in t and not split_top(t, "|")[1:]:
        raise Unpoolable("DEFERRED-CALLBACK", f"callback parameter `{t}`")
    if t.startswith("Promise<"):
        raise Unpoolable("DEFERRED-ASYNC", f"Promise parameter `{t}`")
    members = [m for m in split_top(t, "|") if m not in ("undefined", "null", "void", "")]
    if not members:
        raise Unpoolable("ARG-UNPOOLED", f"parameter type `{t}` has no value")
    for m in members:
        m = m.removeprefix("readonly ").strip()
        if m.startswith('"') or m.startswith("'"):
            key = "string"
        elif re.fullmatch(r"-?\d+(\.\d+)?", m):
            key = "number"
        elif m in POOL_TYPES:
            key = POOL_TYPES[m]
        else:
            continue
        return key, POOL[key]
    if any("=>" in m for m in members):
        raise Unpoolable("DEFERRED-CALLBACK", f"callback parameter `{t}`")
    raise Unpoolable("ARG-UNPOOLED", f"parameter type `{t}` has no pool value")


def call_args(sig):
    """(exprs, keys) for the required parameters of one signature; optional and rest parameters are omitted."""
    if sig.get("ret", "").startswith("Promise<"):
        raise Unpoolable("DEFERRED-ASYNC", f"returns {sig['ret']}")
    exprs, keys = [], []
    for p in sig["params"]:
        if p["optional"] or p["rest"]:
            continue
        key, expr = arg_for(p["type"])
        exprs.append(expr)
        keys.append(key)
    return exprs, keys


def pick_sig(sigs):
    """First signature whose required arguments all pool. Raises Unpoolable with the first failure."""
    first = None
    for s in sigs:
        try:
            return call_args(s)
        except Unpoolable as exc:
            first = first or exc
    if first is None:
        raise Unpoolable("NO-DTS", "no signature")
    raise first


class Rules:
    """RULES compiled. match() returns (action, category, reason) of the first rule that applies, or None."""

    def __init__(self):
        self.seen = set()
        self.compiled = []
        for i, (action, mrx, erx, mbr, kinds, cat, reason) in enumerate(RULES):
            self.compiled.append((i, action, re.compile(mrx), re.compile(erx),
                                  re.compile(mbr) if mbr else None, kinds, cat, reason))

    def match(self, module, export, kind, member=None):
        """member=None asks about the export as a whole (rules with a member regex do not apply to it)."""
        for i, action, mrx, erx, brx, kinds, cat, reason in self.compiled:
            if not mrx.fullmatch(module) or not erx.fullmatch(export):
                continue
            if kinds is not None and kind not in kinds:
                continue
            if brx is not None and (member is None or not brx.fullmatch(member)):
                continue
            self.seen.add(i)
            return action, cat, reason
        return None

    def unused(self):
        """Export-level rules that matched no table row (stale). Member rules are not checked: they need a d.ts."""
        return [{"module": RULES[i][1], "export": RULES[i][2], "action": RULES[i][0]}
                for i in range(len(RULES)) if i not in self.seen and RULES[i][3] is None]


def short(mod):
    """'node:path/posix' -> 'path_posix': the module part of probe names and of mod: ids (minus node:)."""
    return mod.removeprefix("node:").replace("/", "_")


def probe(name, body, mode, api, keys, shape=False):
    line = f'p({json.dumps(name)}, () => {{ {body} }});'
    return {"name": name, "api": [api], "mode": mode, "line": line, "arg_types": keys, "shape": shape}


def index_decls(decls):
    """name -> {kinds, sigs (functions), members (classes and interfaces merged), type (const)}."""
    idx = {}
    for d in decls:
        ent = idx.setdefault(d["name"], {"kinds": set(), "sigs": [], "members": [], "type": None})
        ent["kinds"].add(d["kind"])
        if d["kind"] == "function":
            ent["sigs"] += d.get("sigs", [])
        elif d["kind"] in ("class", "interface"):
            ent["members"] += d.get("members", [])
        elif d["kind"] == "const":
            ent["type"] = d.get("type")
    return idx


def class_probes(mod, export, ent, rules, member_skips):
    """(probes, None) for a class, or ([], (category, reason)) when the constructor cannot be called.
    Probes: construct, each method (instance `o.m`, static `M.C.m`), each property. Members that cannot be pooled
    go to member_skips with their reason."""
    api = f"mod:{mod.removeprefix('node:')}.{export}"
    base = f"{short(mod)}.{export}"
    members = ent["members"]
    ctors = [s for m in members if m["kind"] == "constructor" for s in m.get("sigs", [])]
    try:
        cargs, ckeys = pick_sig(ctors) if ctors else ([], [])
    except Unpoolable as exc:
        return [], (exc.category, exc.reason)
    ctor = f"new M.{export}({', '.join(cargs)})"
    obj = f"const o: M.{export} = {ctor}; "
    whole = rules.match(mod, export, "class")
    whole_shape = whole is not None and whole[0] == "shape"
    tail = (lambda expr: f"const r: any = {expr}; {SHAPE_TAIL}") if whole_shape else (lambda expr: f"return {expr};")
    probes = [probe(f"{base}.new.typed", tail(ctor), "typed", api, ckeys, whole_shape)]
    seen = set()
    for m in members:
        name = m["name"]
        if m["kind"] == "constructor" or name in seen:
            continue
        seen.add(name)
        static = bool(m.get("static"))
        rule = rules.match(mod, export, "class", name)
        if rule is not None and rule[0] == "skip":
            member_skips.append({"id": api, "member": name, "category": rule[1], "reason": rule[2]})
            continue
        shape = whole_shape or (rule is not None and rule[0] == "shape")
        if m["kind"] == "property":
            ref = f"M.{export}.{name}" if static else f"o.{name}"
            pre = "" if static else obj
            keys = [] if static else ckeys
            body = pre + (f"const r: any = {ref}; {SHAPE_TAIL}" if shape else f"return {ref};")
            probes.append(probe(f"{base}.{'static.' if static else ''}{name}.typed", body, "typed", api, keys, shape))
            continue
        sigs = [s for x in members if x["name"] == name and x["kind"] == "method" for s in x.get("sigs", [])]
        try:
            margs, mkeys = pick_sig(sigs)
        except Unpoolable as exc:
            member_skips.append({"id": api, "member": name, "category": exc.category, "reason": exc.reason})
            continue
        if static:
            call, pre, keys, label = f"M.{export}.{name}({', '.join(margs)})", "", mkeys, f"static.{name}"
        else:
            call, pre, keys, label = f"o.{name}({', '.join(margs)})", obj, ckeys + mkeys, name
        body = pre + (f"const r: any = {call}; {SHAPE_TAIL}" if shape else f"return {call};")
        probes.append(probe(f"{base}.{label}.typed", body, "typed", api, keys, shape))
    return probes, None


def export_probes(mod, row, ent, rules, member_skips, have_dts):
    """(probes, None) or ([], (category, reason)) for one table row. ent is the d.ts entry or None. Functions and
    classes need a signature; value exports need none (a typed access that the d.ts does not declare is a REJECT,
    which is the finding), so they are probed even without a d.ts entry."""
    export, kind, vtype = row["export"], row["kind"], row["vtype"]
    api = f"mod:{mod.removeprefix('node:')}.{export}"
    base = f"{short(mod)}.{export}"
    is_fn = kind == "function" or (kind in ("constant", "primitive", "object", "accessor") and vtype == "function")
    rule = rules.match(mod, export, "function" if is_fn else kind)
    if rule is not None and rule[0] == "skip":
        return [], (rule[1], rule[2])
    shape = rule is not None and rule[0] == "shape"
    if kind == "class" or is_fn:
        if not have_dts:
            return [], ("NO-DTS-FILE", "module has no d.ts in the dump directory")
        if ent is None:
            return [], ("NO-DTS", "export is not declared in the module's d.ts")
    if kind == "class":
        if "class" not in ent["kinds"]:
            return [], ("DTS-KIND", f"d.ts declares {sorted(ent['kinds'])}, runtime is a class")
        return class_probes(mod, export, ent, rules, member_skips)
    if is_fn:
        if "function" not in ent["kinds"]:
            return [], ("DTS-KIND", f"d.ts declares {sorted(ent['kinds'])}, runtime is a function")
        try:
            args, keys = pick_sig(ent["sigs"])
        except Unpoolable as exc:
            return [], (exc.category, exc.reason)
        call = f"M.{export}({', '.join(args)})"
        anyc = f"m.{export}({', '.join(args)})"
        if shape:
            typed_body = f"const r: any = {call}; {SHAPE_TAIL}"
            any_body = f"const m: any = M; const r: any = {anyc}; {SHAPE_TAIL}"
        else:
            typed_body = f"return {call};"
            any_body = f"const m: any = M; return {anyc};"
        return [probe(f"{base}.typed", typed_body, "typed", api, keys, shape),
                probe(f"{base}.any", any_body, "any", api, keys, shape)], None
    # value exports: constant, primitive, object, accessor
    dts = ent is not None
    if vtype in ("number", "string", "boolean") and kind in ("constant", "primitive") and not shape:
        p = probe(f"{base}.typed", f"return M.{export};", "typed", api, [])
    else:
        p = probe(f"{base}.typed", f"const r: any = M.{export}; {SHAPE_TAIL}", "typed", api, [], True)
    p["dts"] = dts
    return [p], None


def load_table(path):
    with open(path, encoding="utf-8") as fh:
        header = fh.readline().rstrip("\n").split("\t")
        return [dict(zip(header, line.rstrip("\n").split("\t"))) for line in fh]


def generate(dts_dir, table_path, modules_path, node="node", per_program=PER_PROGRAM):
    """Probes per module plus the skip records. Nothing is written here (node is run to list Object.keys)."""
    modules = [m.strip() for m in open(modules_path, encoding="utf-8") if m.strip()]
    rows = load_table(table_path)
    universe = enumerable_exports(modules, node)
    rules = Rules()
    by_mod = {}
    for r in rows:
        by_mod.setdefault(r["module"], []).append(r)
    res = {"rows": rows, "modules": modules, "per_program": per_program, "rules": rules,
           "probes_by_mod": {}, "skips": [], "member_skips": [], "dts_status": {}}
    for mod in modules:
        mangled = dts_parse.mangle(mod)
        path = os.path.join(dts_dir, mangled + ".d.ts")
        have = os.path.exists(path)
        res["dts_status"][mod] = "ok" if have else "missing"
        idx = index_decls(dts_parse.parse_text(open(path, encoding="utf-8").read())[0]) if have else {}
        out = []
        for r in sorted(by_mod.get(mod, []), key=lambda r: r["export"]):
            export = r["export"]
            mod_name = mod.removeprefix("node:")
            if export not in universe[mod]:
                res["skips"].append(skip_rec(mod, mod_name, export, "NOT-EXPORT", CATEGORIES["NOT-EXPORT"]))
                continue
            if export.startswith("_") or not IDENT_RE.match(export):
                rule = rules.match(mod, export, r["kind"])
                cat, reason = ("NODE-INTERNAL", "underscore-prefixed Node-internal export" if export.startswith("_")
                               else "export name is not an identifier")
                if rule is not None and rule[0] == "skip":
                    cat, reason = rule[1], rule[2]
                res["skips"].append(skip_rec(mod, mod_name, export, cat, reason))
                continue
            probes, skip = export_probes(mod, r, idx.get(export), rules, res["member_skips"], have)
            if skip is None:
                out.extend(probes)
            else:
                res["skips"].append(skip_rec(mod, mod_name, export, skip[0], skip[1]))
        for export in sorted(universe[mod] - {r["export"] for r in by_mod.get(mod, [])}):
            res["skips"].append(skip_rec(mod, mod_name_of(mod), export, "NOT-IN-TABLE", CATEGORIES["NOT-IN-TABLE"]))
        res["probes_by_mod"][mod] = out
    return res


def mod_name_of(mod):
    return mod.removeprefix("node:")


def skip_rec(mod, mod_name, export, cat, reason):
    assert cat in CATEGORIES, cat
    assert reason, export
    return {"id": f"mod:{mod_name}.{export}", "module": mod, "export": export, "category": cat, "reason": reason}


def emit(gen, out_dir):
    per = gen["per_program"]
    programs = []
    for mod in gen["modules"]:
        for mode in MODES:
            rows = sorted([p for p in gen["probes_by_mod"][mod] if p["mode"] == mode], key=lambda p: p["name"])
            for n, start in enumerate(range(0, len(rows), per)):
                programs.append({
                    "name": f"mod_{short(mod)}.{mode}.c{n:02d}",
                    "corpus": f"mod_{short(mod)}.{mode}",
                    "family": f"mod_{short(mod)}",
                    "mode": mode,
                    "module": False,
                    "imports": [f'import * as M from "{mod}";'],
                    "probes": rows[start:start + per],
                })
    names = [p["name"] for prog in programs for p in prog["probes"]]
    assert len(names) == len(set(names)), "duplicate probe names"
    for prog in programs:
        for p in prog["probes"]:
            assert set(p["arg_types"]) <= set(POOL), (p["name"], p["arg_types"])
    os.makedirs(out_dir, exist_ok=True)
    for prog in programs:
        lines = [p["line"] for p in prog["probes"]]
        for suffix, node in ((".ts", False), (".mts", True)):
            with open(os.path.join(out_dir, prog["name"] + suffix), "w", encoding="utf-8") as fh:
                fh.write(render(lines, prog["imports"][0], node))
    counts = {}
    for prog in programs:
        counts[prog["corpus"]] = counts.get(prog["corpus"], 0) + len(prog["probes"])
    manifest = {"per_program": per, "probes": len(names), "skipped": len(gen["skips"]) + len(gen["member_skips"]),
                "counts": counts, "findings": [], "programs": programs}
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=1, sort_keys=True)
        fh.write("\n")
    skips = {"exports": sorted(gen["skips"], key=lambda s: s["id"]),
             "members": sorted(gen["member_skips"], key=lambda s: (s["id"], s["member"])),
             "dts_status": gen["dts_status"], "unused_rules": gen["rules"].unused()}
    with open(os.path.join(out_dir, "skips.json"), "w", encoding="utf-8") as fh:
        json.dump(skips, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return manifest


HEADER = "// Generated by tools/parity-aot/gen_modules.py. Do not edit by hand.\n"


def render(lines, import_line, node):
    text = HEADER + import_line + "\n" + gen_probes.PRELUDE + "\nfunction main(): void {\n"
    text += "".join("  " + line + "\n" for line in lines)
    text += "}\n"
    if node:
        text += "main();\n"
    return text


def summary(gen, manifest):
    """Per module and kind: exports probed/skipped (an export counts once, probed if it has any probe)."""
    probed = {p["api"][0] for prog in manifest["programs"] for p in prog["probes"]}
    by_mod = {}
    for r in gen["rows"]:
        k = by_mod.setdefault(r["module"], {}).setdefault(r["kind"], [0, 0])
        k[0 if f"mod:{r['module'].removeprefix('node:')}.{r['export']}" in probed else 1] += 1
    lines = ["module | " + " | ".join(f"{k} p/s" for k in ("function", "class", "constant", "primitive",
                                                          "object", "accessor"))]
    for mod in gen["modules"]:
        per = by_mod.get(mod, {})
        cells = [f"{per[k][0]}/{per[k][1]}" if k in per else "-"
                 for k in ("function", "class", "constant", "primitive", "object", "accessor")]
        lines.append(f"{mod} | " + " | ".join(cells))
    return "\n".join(lines)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out_dir")
    ap.add_argument("--dts", default=os.environ.get("PARITY_AOT_DTS_DIR"),
                    help="directory of <module>.d.ts dumps (default $PARITY_AOT_DTS_DIR)")
    ap.add_argument("--table", default=DEFAULT_TABLE)
    ap.add_argument("--modules", default=DEFAULT_MODULES)
    ap.add_argument("--per-program", type=int, default=PER_PROGRAM)
    args = ap.parse_args(argv)
    if not args.dts or not os.path.isdir(args.dts):
        print("gen_modules: no d.ts dump directory (set PARITY_AOT_DTS_DIR or --dts; dump with "
              "THAW_DUMP_DTS_DIR=<dir> cargo test -p thaw-registry --test dump_builtin_dts -- --ignored)",
              file=sys.stderr)
        return 2
    if args.per_program < 1:
        ap.error("--per-program must be >= 1")
    gen = generate(args.dts, args.table, args.modules, node=os.environ.get("NODE", "node"),
                   per_program=args.per_program)
    manifest = emit(gen, args.out_dir)
    print(summary(gen, manifest))
    print(f"probes: {manifest['probes']} in {len(manifest['programs'])} programs; "
          f"skipped exports: {len(gen['skips'])}, member skips: {len(gen['member_skips'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
