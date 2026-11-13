#!/usr/bin/env python3
"""Generate the routed parity matrix from ONE probe table, split into independent programs.

For every chunk i (zero-padded), OUT_DIR gets:
  matrix_cNN.ts       compiled by thaw with THAW_JS_ROUTE_ALL=1 (its own main())
  matrix_cNN.node.js  the same probes as plain CommonJS, the node oracle
  probes_cNN.txt      the expected probe names of that chunk
plus OUT_DIR/probes.txt (all expected names, emission order).

Why chunks: thaw is a typed compiler, so one compile error stops a whole program (a bare reference
to an undeclared global is `unknown variable`, a compile error by design). With N probes per program,
a bad probe only hides its own chunk, not the whole matrix.

Every probe prints one line `name = result` (a throw prints `name = THROW <Name>: <message>`),
so diff.py compares the concatenated stdouts directly (node/matrix.txt vs thaw/matrix.txt).

Families (plan-q9-systematic-parity.md sections 2 and 3, H2):
  F3  handle 0 / absent globals: each name x {typeof, === undefined, == null, String(), truthiness}
      (NopeNope is not declared in the prelude, so it only gets the typeof probe)
  F5  printing: value x container (bare console.log, array, object, template, String, JSON, concat, inspect)
  F6  identity/aliasing: imported or global object x identity operations (through any, arrays, Map, Set, Object.is)

Usage: gen_routed.py OUT_DIR [--chunk N]     (N probes per program, default 10)
Self-check (asserts before writing): names unique, no " = " inside a name, per-family counts as expected.
"""
import argparse
import os
import re
import sys

ABSENT_GLOBALS = [
    "navigator", "NopeNope", "fetch", "structuredClone", "queueMicrotask", "WebSocket", "Blob",
    "performance", "BroadcastChannel", "MessageChannel", "AbortController", "EventTarget", "Event",
    "TextEncoder", "TextDecoder", "URL", "URLSearchParams", "crypto", "localStorage", "WebAssembly",
    "atob", "btoa", "File", "FormData", "Headers", "Request", "Response", "DOMException",
    "CustomEvent", "require",
]
# Not declared in the TS prelude: a bare reference is a compile error in thaw by design (node gives a
# runtime ReferenceError), so these only keep the typeof probe, the one form that is legal on an undeclared name.
UNDECLARED_GLOBALS = {"NopeNope"}
ABSENT_OPS = [
    ("typeof", "return typeof {x};"),
    ("eqUndef", "return {x} === undefined;"),
    ("eqNull", "return {x} == null;"),
    ("string", "return String({x}).slice(0, 60).split('\\n')[0];"),
    ("truthy", "return {x} ? 1 : 0;"),
]

# (id, TS expression for the value). K5, fnP, makeTimeout are top-level helpers in the header.
VALUES = [
    ("math", "Math"),
    ("json", "JSON"),
    ("reflect", "Reflect"),
    ("intl", "Intl"),
    ("procver", "(globalThis as any).process.versions"),
    ("class", "K5"),
    ("function", "fnP"),
    ("symbol", "Symbol('s')"),
    ("bigint", "1n"),
    ("map", "new Map([[1, 2]])"),
    ("set", "new Set([1])"),
    ("buffer", "Buffer.from('a')"),
    ("error", "new Error('e')"),
    ("timeout", "makeTimeout()"),
]
# (id, template over v). console containers go through console.log's own formatter (native printer under
# test); string containers are one line by construction. A throwing container is a probe result.
# process.versions and Timeout print over several lines in node under console.log, which would break
# one-line-per-probe, so they only get the string containers.
WIDE = "{ breakLength: Infinity }"
CONSOLE_CONTAINERS = [
    ("bare", "v"),
    ("array", "[v]"),
    ("object", "{ k: v }"),
]
STRING_CONTAINERS = [
    ("template", "`${v}`"),
    ("string", "String(v)"),
    ("json", "JSON.stringify(v)"),
    ("concat", '"a" + v'),
    ("inspect", "inspect(v, " + WIDE + ")"),
]
MULTILINE_VALUES = {"procver", "timeout"}

OBJECTS = [
    ("eventemitter", "EventEmitter"),
    ("buffer", "Buffer"),
    ("process", "(globalThis as any).process"),
    ("map", "Map"),
]
IDENT_OPS = [
    ("same", "const v: any = {X}; return v === v;"),
    ("twoPaths", "const a: any = {X}; const b: any = {X}; return a === b;"),
    ("anyVar", "const a: any = {X}; return a === {X};"),
    ("arrayRoundTrip", "const arr: any[] = [{X}]; return arr[0] === {X};"),
    ("arrayIncludes", "const arr: any[] = [{X}]; return arr.includes({X});"),
    ("arrayIndexOf", "const arr: any[] = [{X}]; return arr.indexOf({X});"),
    ("mapKey", "const m: any = new Map(); m.set({X}, 1); return m.get({X});"),
    ("mapKeyViaAny", "const m: any = new Map(); const k: any = {X}; m.set(k, 1); return m.get({X});"),
    ("setHas", "const s: any = new Set([{X}]); return s.has({X});"),
    ("objectIs", "const a: any = {X}; return Object.is(a, {X});"),
    ("typeofAny", "const a: any = {X}; return typeof a;"),
    ("ctorEq", "const a: any = {X}; return a.constructor === {X}.constructor;"),
    ("instOfObject", "const a: any = {X}; return a instanceof Object;"),
    ("instOfFunction", "const a: any = {X}; return a instanceof Function;"),
    ("hasOwnName", "const a: any = {X}; return Object.prototype.hasOwnProperty.call(a, 'name');"),
]
# Instance-level identity checks (the instance is a fresh object, so every answer is a real projection check).
EXTRA_F6 = [
    ("ee_instance", "instOfEventEmitter", "return new EventEmitter() instanceof EventEmitter;"),
    ("ee_instance", "ctorIsEventEmitter", "return new EventEmitter().constructor === EventEmitter;"),
    ("buffer_instance", "instOfUint8Array", "return Buffer.from('a') instanceof Uint8Array;"),
]

TS_HEADER = """\
// Generated by tools/parity/gen_routed.py. Do not edit by hand.
import { EventEmitter } from "node:events";
import { inspect } from "node:util";
declare var require: any;
"""

TS_HELPERS = """\
class K5 {}
function fnP(a: number): number { return a; }
function makeTimeout(): any {
  const t: any = setTimeout(() => {}, 1000);
  clearTimeout(t);
  return t;
}
"""

IMPORT_RE = re.compile(r'^import \{ (.+) \} from "(.+)";$')


def iife(body):
    return "(() => { " + body + " })()"


def probe(name, body):
    return (f'try {{ console.log("{name} =", {iife(body)}); }} '
            f'catch (e: any) {{ console.log("{name} =", "THROW", e.name + ": " + e.message); }}')


def build_probes():
    """Return [(family, name, body)] in emission order."""
    out = []
    for g in ABSENT_GLOBALS:
        ops = [op for op in ABSENT_OPS if op[0] == "typeof"] if g in UNDECLARED_GLOBALS else ABSENT_OPS
        for op_id, tpl in ops:
            out.append(("F3", f"F3.{g}.{op_id}", tpl.format(x=g)))
    for vid, vexpr in VALUES:
        cont = STRING_CONTAINERS if vid in MULTILINE_VALUES else CONSOLE_CONTAINERS + STRING_CONTAINERS
        for cid, ctpl in cont:
            body = f"const v: any = {vexpr}; return {ctpl};"
            out.append(("F5", f"F5.{vid}.{cid}", body))
    for oid, oexpr in OBJECTS:
        for op_id, tpl in IDENT_OPS:
            out.append(("F6", f"F6.{oid}.{op_id}", tpl.replace("{X}", oexpr)))
    for oid, op_id, body in EXTRA_F6:
        out.append(("F6", f"F6.{oid}.{op_id}", body))
    return out


def selfcheck(probes):
    names = [p[1] for p in probes]
    assert len(names) == len(set(names)), "duplicate probe names"
    assert all(" = " not in n for n in names), "probe name contains ' = '"
    counts = {}
    for fam, _, _ in probes:
        counts[fam] = counts.get(fam, 0) + 1
    n3 = sum(1 if g in UNDECLARED_GLOBALS else len(ABSENT_OPS) for g in ABSENT_GLOBALS)
    assert counts["F3"] == n3
    n5 = sum(len(STRING_CONTAINERS) if v in MULTILINE_VALUES else len(CONSOLE_CONTAINERS) + len(STRING_CONTAINERS)
             for v, _ in VALUES)
    assert counts["F5"] == n5
    assert counts["F6"] == len(OBJECTS) * len(IDENT_OPS) + len(EXTRA_F6)
    return counts


def to_js(ts):
    lines = []
    for line in ts.split("\n"):
        if line.startswith("declare "):
            continue
        m = IMPORT_RE.match(line)
        if m:
            lines.append(f'const {{ {m.group(1)} }} = require("{m.group(2)}");')
            continue
        lines.append(line)
    s = "\n".join(lines)
    s = re.sub(r"\s+as any\b", "", s)
    s = re.sub(r":\s*(any\[\]|any|void|number)(?!\w)", "", s)
    return s


def render_chunk(chunk):
    """Return (ts, js) text for one program: the header, the helpers, `probes()` and its own main()."""
    body = "\n".join("  " + probe(name, pbody) for _, name, pbody in chunk)
    ts = (TS_HEADER + TS_HELPERS
          + "function probes(): void {\n" + body + "\n}\n"
          + "function main(): void {\n"
          + "  (Error as any).stackTraceLimit = 0;\n"
          + "  probes();\n"
          + "}\n")
    js = to_js(ts) + "\nmain();\n"
    assert ts.count("try {") == len(chunk) and js.count("try {") == len(chunk)
    return ts, js


def write(path, text):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def main(argv):
    ap = argparse.ArgumentParser(description="Generate the routed parity matrix as independent programs.")
    ap.add_argument("out_dir")
    ap.add_argument("--chunk", type=int, default=10, help="probes per program (default 10)")
    args = ap.parse_args(argv)
    if args.chunk < 1:
        ap.error("--chunk must be >= 1")
    probes = build_probes()
    counts = selfcheck(probes)
    os.makedirs(args.out_dir, exist_ok=True)
    chunks = [probes[i:i + args.chunk] for i in range(0, len(probes), args.chunk)]
    for i, chunk in enumerate(chunks):
        ts, js = render_chunk(chunk)
        stem = os.path.join(args.out_dir, f"matrix_c{i:02d}")
        write(stem + ".ts", ts)
        write(stem + ".node.js", js)
        write(os.path.join(args.out_dir, f"probes_c{i:02d}.txt"), "\n".join(p[1] for p in chunk) + "\n")
    write(os.path.join(args.out_dir, "probes.txt"), "\n".join(p[1] for p in probes) + "\n")
    print(f"probes: {len(probes)} (F3 {counts['F3']}, F5 {counts['F5']}, F6 {counts['F6']}) "
          f"in {len(chunks)} programs of <= {args.chunk}")
    print(f"wrote {args.out_dir}/matrix_c00..matrix_c{len(chunks) - 1:02d} (.ts, .node.js), probes_cNN.txt, probes.txt")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
