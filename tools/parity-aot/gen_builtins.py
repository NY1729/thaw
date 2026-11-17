#!/usr/bin/env python3
"""Built-in family probes for tools/parity-aot (PR2), generated from crates/thaw-js/src/builtin_table.tsv.

build(rows) -> [(family, mode, name, body, api)]: body is `<setup> return <expr>;` (one line, no newline), api is a
list of API ids. Node is the oracle; nothing here encodes an expected value.

Receivers: each family has variants (receiver factories). Statics (`Owner.key(...)`) have no receiver to vary, so they
use typed, any, union and callback only. Receiver modes: typed, any, union, unknown, generic, callback, json (only
receivers with a JSON text) and module (a `mod_<family>_<variant>()` factory in lib.ts).

Argument sets per method: T1 = benign values per parameter kind; T2 = edge values wrapped `as any`. Methods with no
parameters get one set (a0). Properties (accessors and Error.prototype strings) get one set (v).

Quota: each family emits at most QUOTA probes. Candidates are ranked in tiers (T1 with typed/any/union/callback; T2
with typed/any; T1 with unknown/generic/json/module; T2 with the rest). Inside a tier the API ids take turns, so every
id with any candidate is touched before any id gets a second probe in that tier.
"""
import json
import re

QUOTA = 600
ERRORS = ["Error", "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError"]
TYPED = [
    ("Float64Array", "[1.5, 2, NaN]", ("1", "2")),
    ("Int8Array", "[1, -2, 3]", ("1", "2")),
    ("Int16Array", "[1, -2, 3]", ("1", "2")),
    ("Uint16Array", "[1, 2, 3]", ("1", "2")),
    ("Int32Array", "[1, -2, 3]", ("1", "2")),
    ("Uint32Array", "[1, 2, 3]", ("1", "2")),
    ("Float32Array", "[1.5, 2, 3]", ("1", "2")),
    ("Uint8ClampedArray", "[1, 300, -4]", ("1", "2")),
    ("BigInt64Array", "[1n, 2n]", ("1n", "2n")),
]

# Every probe name, api id and factory is deterministic; SKIP is matched against api ids and bare probe names.
SKIP = [
    r"^Math\.random$",
    r"^Date\.now$",
    r"toLocale",  # toLocaleString/DateString/TimeString everywhere (Intl slice)
    r"localeCompare$",
    r"^Date\.prototype\.(toString|toDateString|toTimeString|getYear|setYear|getTimezoneOffset"
    r"|get(Date|Day|FullYear|Hours|Milliseconds|Minutes|Month|Seconds)"
    r"|set(Date|Day|FullYear|Hours|Milliseconds|Minutes|Month|Seconds))$",  # local-zone dependent
    r"^Error\.(captureStackTrace|prepareStackTrace|stackTraceLimit)$",
    r"\.stack$",
    r"^Function\.prototype\.toString$",
    r"__(proto|defineGetter|defineSetter|lookupGetter|lookupSetter)__$",
    r"^(performance|process|console|Atomics|WebAssembly|Intl)\.",
    r"^(setTimeout|setInterval|setImmediate|clearTimeout|clearInterval|queueMicrotask|fetch)$",
    r"^(WeakRef|FinalizationRegistry)\.",
    r"\.deref$",
    r"^Promise\.reject$",  # unhandled rejection would end the program
    r"^Array\.fromAsync$",  # async (PR3): a rejection would be unhandled
    r"^(eval|with)$",
]

# Hand ids: API surfaces that are not table rows. Each one is documented here; selftest_gen checks they are used.
HAND_IDS = {
    "hand:index-get": "element read x[i] (in range, out of range, negative)",
    "hand:index-set": "element write x[i] = v",
    "hand:iterate": "iteration protocol: spread, for-of, array destructuring",
    "hand:in": "`in` operator on an indexed receiver",
    "hand:delete-index": "delete operator on an element",
    "hand:typeof": "typeof operator on a receiver",
    "hand:instanceof": "instanceof against a constructor",
    "hand:string-conversion": "String(x) of a receiver",
    "hand:loose-equality": "== between a receiver and a string",
    "hand:array-length": "length of a plain list argument",
    "hand:Proxy.trap.get": "Proxy get trap",
    "hand:Proxy.trap.set": "Proxy set trap",
    "hand:Proxy.trap.has": "Proxy has trap (in operator)",
    "hand:Proxy.trap.deleteProperty": "Proxy deleteProperty trap",
    "hand:Proxy.trap.ownKeys": "Proxy ownKeys trap (Reflect.ownKeys)",
    "hand:Proxy.trap.apply": "Proxy apply trap (function target)",
    "hand:Proxy.trap.construct": "Proxy construct trap (class target)",
    "hand:Proxy.revoked-read": "property read through a revoked proxy",
    "hand:Error.prototype.message": "Error.prototype.message (data string row, read as a property)",
    "hand:Error.prototype.name": "Error.prototype.name (data string row, read as a property)",
    "hand:global.DisposableStack": "thaw-only global (PLATFORM_GLOBALS Ctor), absent from Node 22: expected ReferenceError vs thaw value",
    "hand:global.AsyncDisposableStack": "thaw-only global (PLATFORM_GLOBALS Ctor), absent from Node 22: expected ReferenceError vs thaw value",
}

# (family, key) -> parameter kinds, overriding the family default. Keys are API method names.
FAMILY_KINDS = {
    "array": ["elem", "idx", "idx"], "string": ["str", "idx", "idx"], "map": ["elem", "str"], "set": ["elem"],
    "weak": ["wkey", "elem"], "date": ["num"], "regexp": ["str"], "promise": ["pl", "elem"], "json": ["str"],
    "math": ["num"], "number": ["num"], "bigint": ["num", "bigint"], "symbol": ["str"], "error": [],
    "typedarray": ["elem", "idx", "idx"], "object": ["obj", "str", "desc"], "reflect": ["obj", "str", "elem"],
    "proxy": ["obj", "obj"],
}
OVR = {
    ("array", "join"): ["str"], ("array", "with"): ["idx", "elem"], ("array", "splice"): ["idx", "idx", "elem"],
    ("array", "toSpliced"): ["idx", "idx", "elem"], ("array", "copyWithin"): ["idx", "idx", "idx"],
    ("array", "concat"): ["arr"], ("array", "reduce"): ["fn", "elem"], ("array", "reduceRight"): ["fn", "elem"],
    ("array", "at"): ["idx"], ("array", "from"): ["arr"], ("array", "isArray"): ["obj"], ("array", "flat"): ["num"],
    ("string", "at"): ["idx"], ("string", "charAt"): ["idx"], ("string", "charCodeAt"): ["idx"],
    ("string", "codePointAt"): ["idx"], ("string", "concat"): ["str"], ("string", "endsWith"): ["str", "idx"],
    ("string", "startsWith"): ["str", "idx"], ("string", "includes"): ["str", "idx"],
    ("string", "indexOf"): ["str", "idx"], ("string", "lastIndexOf"): ["str", "idx"],
    ("string", "padEnd"): ["num", "str"], ("string", "padStart"): ["num", "str"], ("string", "repeat"): ["num"],
    ("string", "replace"): ["regex"], ("string", "replaceAll"): ["str"], ("string", "match"): ["regex"],
    ("string", "matchAll"): ["regex"], ("string", "search"): ["regex"], ("string", "split"): ["str", "num"],
    ("string", "slice"): ["idx"], ("string", "substring"): ["idx"], ("string", "substr"): ["idx"],
    ("string", "normalize"): ["str"], ("string", "raw"): ["tpl"], ("string", "fromCharCode"): ["num"],
    ("string", "fromCodePoint"): ["num"],
    ("map", "get"): ["elem"], ("map", "has"): ["elem"], ("map", "delete"): ["elem"], ("map", "groupBy"): ["arr"],
    ("set", "add"): ["elem"], ("set", "union"): ["set"], ("set", "intersection"): ["set"],
    ("set", "difference"): ["set"], ("set", "symmetricDifference"): ["set"], ("set", "isSubsetOf"): ["set"],
    ("set", "isSupersetOf"): ["set"], ("set", "isDisjointFrom"): ["set"],
    ("date", "parse"): ["datestr"],
    ("regexp", "exec"): ["str"], ("regexp", "test"): ["str"], ("regexp", "compile"): ["str", "flags"],
    ("number", "parseFloat"): ["str"], ("number", "parseInt"): ["str", "num"],
    ("bigint", "asIntN"): ["num", "bigint"], ("bigint", "asUintN"): ["num", "bigint"],
    ("symbol", "for"): ["str"], ("symbol", "keyFor"): ["sym"],
    ("json", "parse"): ["json"], ("json", "stringify"): ["obj", "nullable", "num"],
    ("json", "rawJSON"): ["jsonstr"], ("json", "isRawJSON"): ["obj"],
    ("object", "assign"): ["obj", "obj"], ("object", "create"): ["obj", "props"],
    ("object", "defineProperties"): ["obj", "props"], ("object", "fromEntries"): ["pairs"],
    ("object", "groupBy"): ["arr"], ("object", "is"): ["elem", "elem"],
    ("object", "setPrototypeOf"): ["obj", "nullable"],
    ("reflect", "apply"): ["fn", "any", "arr"], ("reflect", "construct"): ["ctor", "arr"],
    ("reflect", "defineProperty"): ["obj", "str", "desc"], ("reflect", "setPrototypeOf"): ["obj", "nullable"],
    ("reflect", "getPrototypeOf"): ["obj"], ("reflect", "isExtensible"): ["obj"],
    ("reflect", "ownKeys"): ["obj"], ("reflect", "preventExtensions"): ["obj"],
    ("reflect", "deleteProperty"): ["obj", "str"], ("reflect", "get"): ["obj", "str"],
    ("reflect", "getOwnPropertyDescriptor"): ["obj", "str"], ("reflect", "has"): ["obj", "str"],
    ("promise", "resolve"): ["elem"], ("promise", "withResolvers"): [],
    ("proxy", "revocable"): ["obj", "obj"],
    ("typedarray", "join"): ["str"], ("typedarray", "set"): ["arr", "idx"], ("typedarray", "with"): ["idx", "elem"],
    ("typedarray", "subarray"): ["idx", "idx"], ("typedarray", "slice"): ["idx", "idx"],
    ("typedarray", "copyWithin"): ["idx", "idx", "idx"], ("typedarray", "at"): ["idx"],
    ("typedarray", "reduce"): ["fn", "elem"], ("typedarray", "reduceRight"): ["fn", "elem"],
}
# Parameter index that is a callback (named shapes below). Keyed by method name.
CB_POS = {
    "map": 0, "filter": 0, "forEach": 0, "some": 0, "every": 0, "find": 0, "findIndex": 0, "findLast": 0,
    "findLastIndex": 0, "flatMap": 0, "reduce": 0, "reduceRight": 0, "sort": 0, "toSorted": 0,
    "from": 1, "groupBy": 1, "replace": 1, "replaceAll": 1,
}
CB_T1 = {
    "reduce": "(a: any, x: any) => a", "reduceRight": "(a: any, x: any) => a",
    "sort": "(a: any, b: any) => a - b", "toSorted": "(a: any, b: any) => a - b",
    "replace": "(m: any) => String(m)", "replaceAll": "(m: any) => String(m)",
}
# kind -> (T1 values, T2 values). "elem" and "arr" take the variant's element values instead.
POOLS = {
    "num": (["2", "10"], ["-1", "NaN"]), "idx": (["0", "1"], ["-1", "10"]),
    "str": (['"a"', '","'], ['""', "undefined"]), "datestr": (['"2020-01-31T12:34:56.789Z"'], ['"x"']),
    "regex": (["/a/g", "/b/"], ["undefined"]), "obj": (["{ a: 1 }"], ["null"]),
    "bigint": (["1n"], ["1"]), "sym": (['Symbol.for("s")'], ["1"]),
    "pl": (["[Promise.resolve(1), 2]"], ["[Promise.resolve(3)]"]),
    "desc": (["{ value: 3, writable: true, enumerable: true, configurable: true }"], ["1"]),
    "nullable": (["null"], ["1"]), "json": (['"{\\"a\\":[1,2]}"'], ['"{"']), "ctor": (["Array"], ["1"]),
    "pairs": (['[["a", 1]]'], ["1"]), "props": (["{ c: { value: 1 } }"], ["1"]), "any": (["1"], ["undefined"]),
    "wkey": (["k"], ["1"]), "fn": (["(x: any) => x"], ["1"]),
    "elem": (None, ["undefined", "null"]), "arr": (None, ["undefined"]),
    "set": (["new Set([1, 2])"], ["1"]), "flags": (['"g"'], ['"z"']), "tpl": (['{ raw: ["a", "b"] }'], ["null"]), "jsonstr": (['"2"'], ['"{"']),
}

SKIP_RE = [re.compile(p) for p in SKIP]
EXCLUDED_KEYS = {"constructor", "length", "prototype"}

MODE_SETUP = {
    "typed": "const b: {T} = {MK};",
    "any": "const b: any = {MK};",
    "union": "const b: {T} | null = flip() ? {MK} : null;",
    "unknown": "const u: unknown = {MK};",
    "generic": "const b = gen({MK});",
    "callback": "const b: any = viaCb({MK});",
    "json": "const b: any = JSON.parse({DOC});",
    "module": "const b: {T} = {MOD};",
}
MODE_RECV = {"union": "(b as {T})", "unknown": "(u as {T})"}
# Namespaces take no union mode (`typeof X | null` is a probe artefact) and typed calls the namespace directly.
STATIC_T1 = {1: ["typed", "any", "callback"]}
STATIC_T2 = {2: ["typed", "any"]}
RECV_T1 = {1: ["typed", "any", "union", "callback"], 3: ["unknown", "generic", "json", "module"]}
RECV_T2 = {2: ["typed", "any"], 4: ["union", "callback", "unknown", "generic", "json", "module"]}


def V(vname, owner, T, MK, json=None, E=("1", "2"), prefix=""):
    return {"vname": vname, "owner": owner, "T": T, "MK": MK, "json": json, "E": E, "prefix": prefix}


KPRE = "const k: object = {a: 1}; "
FAMILIES = [
    {"family": "array", "static": "Array", "variants": [
        V("recv", "Array.prototype", "number[]", "[3, 1, 2]", json="[3,1,2]")]},
    {"family": "string", "static": "String", "variants": [
        V("recv", "String.prototype", "string", '"héllo,wörld"', json='"héllo,wörld"')]},
    {"family": "map", "static": "Map", "variants": [
        V("recv", "Map.prototype", "Map<number, string>", 'new Map<number, string>([[1, "a"], [2, "b"]])')]},
    {"family": "set", "static": None, "variants": [
        V("recv", "Set.prototype", "Set<number>", "new Set<number>([1, 2, 2, 3])")]},
    {"family": "weak", "static": None, "variants": [
        V("WeakMap", "WeakMap.prototype", "WeakMap<object, number>", "new WeakMap<object, number>([[k, 1]])",
          prefix=KPRE),
        V("WeakSet", "WeakSet.prototype", "WeakSet<object>", "new WeakSet<object>([k])", prefix=KPRE)]},
    {"family": "date", "static": "Date", "variants": [
        V("recv", "Date.prototype", "Date", "new Date(Date.UTC(2020, 0, 31, 12, 34, 56, 789))")]},
    {"family": "regexp", "static": "RegExp", "variants": [
        V("recv", "RegExp.prototype", "RegExp", "/a(b)?/g")]},
    {"family": "promise", "static": "Promise", "variants": []},
    {"family": "json", "static": "JSON", "variants": []},
    {"family": "math", "static": "Math", "variants": []},
    {"family": "number", "static": "Number", "variants": [
        V("recv", "Number.prototype", "number", "1.5", json="1.5")]},
    {"family": "bigint", "static": "BigInt", "variants": [
        V("recv", "BigInt.prototype", "bigint", "5n")]},
    {"family": "symbol", "static": "Symbol", "variants": [
        V("recv", "Symbol.prototype", "symbol", 'Symbol.for("s")')]},
    {"family": "error", "static": None, "variants": [
        V(c, "Error.prototype", c, f'new {c}("m")') for c in ERRORS]},
    {"family": "typedarray", "static": None, "variants": [
        V(k, "%TypedArray%.prototype", k, f"new {k}({init})", E=E) for k, init, E in TYPED]},
    {"family": "object", "static": "Object", "variants": []},
    {"family": "reflect", "static": "Reflect", "variants": []},
    {"family": "proxy", "static": "Proxy", "variants": []},
]
FAMILY_NAMES = [f["family"] for f in FAMILIES]

# Proxy trap probes are hand-written (no table rows for traps). Each runs in typed and any receivers.
PROXY_DECL = {"typed": "const px = ", "any": "const px: any = "}
PROXY_OPS = [
    ("get", 'new Proxy({ a: 1 }, { get: (t: any, k: any) => "g:" + String(k) }); return px.a;',
     ["hand:Proxy.trap.get"]),
    ("set", "new Proxy({ a: 1 }, { set: (t: any, k: any, v: any) => { t[k] = v * 2; return true; } }); "
            "px.a = 3; return px.a;", ["hand:Proxy.trap.set"]),
    ("has", 'new Proxy({ a: 1 }, { has: (t: any, k: any) => k === "z" }); return ["z" in px, "a" in px];',
     ["hand:Proxy.trap.has"]),
    ("deleteProperty", 'new Proxy({ a: 1 }, { deleteProperty: (t: any, k: any) => false }); '
                       'return Reflect.deleteProperty(px, "a");', ["hand:Proxy.trap.deleteProperty"]),
    ("ownKeys", 'new Proxy({ a: 1 }, { ownKeys: (t: any) => ["a"] }); return Reflect.ownKeys(px);',
     ["hand:Proxy.trap.ownKeys"]),
    ("apply", "new Proxy((x: any, ...r: any[]) => x, { apply: (t: any, th: any, args: any[]) => args.length }); "
              "return px(1, 2);", ["hand:Proxy.trap.apply"]),
    ("construct", "new Proxy(class K { constructor(...r: any[]) { } }, "
                  "{ construct: (t: any, args: any[]) => ({ c: args.length }) }); return new px(1);",
     ["hand:Proxy.trap.construct"]),
    ("revokedRead", "Proxy.revocable({ a: 1 }, {}); return 0;", ["Proxy.revocable", "hand:Proxy.revoked-read"]),
]


def num(s):
    return 0 if s in ("-", "") else int(s)


def load_table(path):
    """Rows of builtin_table.tsv as dicts (owner, key, kind, vtype, length)."""
    rows = []
    with open(path, encoding="utf-8") as fh:
        next(fh)
        for line in fh:
            c = line.rstrip("\n").split("\t")
            rows.append({"owner": c[0], "key": c[1], "kind": c[2], "vtype": c[4], "length": num(c[5])})
    return rows


def table_ids(rows):
    return {f"{r['owner']}.{r['key']}" for r in rows}


def is_skipped(api):
    return any(p.search(api) for p in SKIP_RE)


def owner_entries(rows, owner, static):
    """Method and property entries for one owner: dicts with kind call/prop, key, length and api."""
    out = []
    for r in rows:
        key = r["key"]
        if r["owner"] != owner or key.startswith("@@") or key in EXCLUDED_KEYS:
            continue
        if key == "name" and owner != "Error.prototype":
            continue
        api = f"{owner}.{key}"
        if is_skipped(api):
            continue
        if r["kind"] == "data" and r["vtype"] == "function":
            out.append({"kind": "call", "key": key, "length": r["length"], "api": api})
        elif r["kind"] == "accessor" and not static:
            out.append({"kind": "prop", "key": key, "length": 0, "api": api})
        elif r["kind"] == "data" and r["vtype"] == "string" and owner == "Error.prototype":
            # data rows, not accessors or functions: a documented hand id keeps them out of the coverage universe
            out.append({"kind": "prop", "key": key, "length": 0, "api": f"hand:{api}"})
    return out


def arg_sets(family, key, length, E, is_call):
    """[(tag, [arg text]) ] for one method: t1 benign, t2 edge (as any); a0 when no parameters; v for properties."""
    if not is_call:
        return [("v", [])]
    cb = CB_POS.get(key)
    n = max(length, cb + 1 if cb is not None else 0)
    n = min(n, 3)
    if n == 0:
        return [("a0", [])]
    base = OVR.get((family, key)) or FAMILY_KINDS[family] or ["any"]
    kinds = [base[min(i, len(base) - 1)] for i in range(n)]
    if cb is not None:
        kinds[cb] = "fn"
    t1, t2 = [], []
    for i, kind in enumerate(kinds):
        if kind == "elem":
            t1.append(E[i % 2])
            t2.append(f"({POOLS['elem'][1][i % 2]} as any)")
        elif kind == "arr":
            t1.append(f"[{E[0]}, {E[1]}]")
            t2.append(f"({POOLS['arr'][1][0]} as any)")
        elif kind == "fn":
            t1.append(CB_T1.get(key, POOLS["fn"][0][0]))
            t2.append(f"({POOLS['fn'][1][0]} as any)")
        else:
            vals1, vals2 = POOLS[kind]
            t1.append(vals1[i % len(vals1)])
            t2.append(f"({vals2[i % len(vals2)]} as any)")
    return [("t1", t1), ("t2", t2)]


def body_for(fam, variant, mode, entry, args):
    """One probe body: `<setup> return <call>;`. Statics use the owner as the receiver."""
    if variant is None:
        fields = {"T": f"typeof {fam['static']}", "MK": fam["static"], "DOC": None, "MOD": None}
        prefix = ""
    else:
        fields = {"T": variant["T"], "MK": variant["MK"],
                  "DOC": json.dumps(variant["json"]) if variant["json"] else None,
                  "MOD": f"mod_{fam['family']}_{variant['vname']}()"}
        prefix = variant["prefix"]
    if variant is None and mode == "typed":
        # A namespace is called directly. The `typeof X` annotation form has its own family (typeofann).
        setup, recv = "", fam["static"]
    else:
        setup = prefix + MODE_SETUP[mode].format(**fields)
        recv = MODE_RECV.get(mode, "b").format(**fields)
    if entry["kind"] == "call":
        call = f"{recv}.{entry['key']}({', '.join(args)})"
    else:
        call = f"{recv}.{entry['key']}"
    return f"{setup} return {call};"


def add_candidates(cands, entry, family, label, side, tag, body_fn, tiers_t1, tiers_t2):
    """Register the candidates of one (entry, argument set) in the tier they belong to. side: s (static) or i."""
    tiers = tiers_t1 if tag in ("t1", "a0", "v") else tiers_t2
    for tier, modes in tiers.items():
        for mode in modes:
            name = f"{family}.{label}.{mode}.{side}.{entry['key']}.{tag}"
            body = body_fn(mode)
            if body is None:
                continue
            cands.setdefault(entry["api"], {}).setdefault(tier, []).append((name, body, [entry["api"]], mode))


def family_candidates(fam, rows):
    """{api: {tier: [(name, body, [api], mode)]}} for one family, in emission order."""
    family = fam["family"]
    cands = {}
    if fam["static"]:
        for e in owner_entries(rows, fam["static"], True):
            for tag, args in arg_sets(family, e["key"], e["length"], ("1", "2"), e["kind"] == "call"):
                add_candidates(cands, e, family, "static", "s", tag,
                               lambda m, e=e, args=args: body_for(fam, None, m, e, args),
                               STATIC_T1, STATIC_T2)
    for variant in fam["variants"]:
        for e in owner_entries(rows, variant["owner"], False):
            for tag, args in arg_sets(family, e["key"], e["length"], variant["E"], e["kind"] == "call"):
                def body_fn(mode, e=e, args=args, variant=variant):
                    if mode == "json" and not variant["json"]:
                        return None
                    return body_for(fam, variant, mode, e, args)
                add_candidates(cands, e, family, variant["vname"], "i", tag, body_fn, RECV_T1, RECV_T2)
    return cands


def select(cands, quota):
    """Round-robin inside each tier over api ids (sorted), so each id gets a probe before any id gets a second."""
    ranked = []
    for api in sorted(cands):
        for tier in sorted(cands[api]):
            for r, cand in enumerate(cands[api][tier]):
                ranked.append((tier, r, api, cand))
    ranked.sort(key=lambda t: (t[0], t[1], t[2]))
    return [t[3] for t in ranked[:quota]]


def proxy_probes():
    """Hand-written trap probes: (family, mode, name, body, api)."""
    out = []
    for op, body, api in PROXY_OPS:
        for mode, decl in PROXY_DECL.items():
            if op == "revokedRead":
                text = "const r = Proxy.revocable({ a: 1 }, {}); r.revoke(); return (r.proxy as any).a;"
            else:
                text = f"{decl}{body}"
            out.append(("proxy", mode, f"proxy.hand.{mode}.{op}.v", text, list(api)))
    return out


def typeof_probes(rows):
    """typeofann: one `const b: typeof <ns> = <ns>` probe per namespace, the only probes that keep the annotation form.
    Its compile rejects measure the TsTypeQuery gap (unsupported `typeof <builtin>` annotation) on their own."""
    out = []
    for fam in FAMILIES:
        ns = fam["static"]
        if not ns:
            continue
        calls = sorted((e for e in owner_entries(rows, ns, True) if e["kind"] == "call"), key=lambda e: e["key"])
        if not calls:
            continue
        e = calls[0]
        args = arg_sets(fam["family"], e["key"], e["length"], ("1", "2"), True)[0][1]
        body = f"const b: typeof {ns} = {ns}; return b.{e['key']}({', '.join(args)});"
        out.append(("typeofann", "typed", f"typeofann.typed.{ns}.{e['key']}", body, [e["api"]]))
    return out


# Tag family: Object.prototype.toString on each native kind, through the tag printer (pt probes). Measures the tag
# gap alone; the value families use show(), which prints the constructor name instead.
TAG_RECV = [
    ("Map", "Map<number, string>", 'new Map<number, string>([[1, "a"]])', ""),
    ("Set", "Set<number>", "new Set<number>([1])", ""),
    ("WeakMap", "WeakMap<object, number>", "new WeakMap<object, number>([[k, 1]])", KPRE),
    ("WeakSet", "WeakSet<object>", "new WeakSet<object>([k])", KPRE),
    ("Date", "Date", "new Date(0)", ""),
    ("RegExp", "RegExp", "/a/g", ""),
    ("Promise", "Promise<number>", "Promise.resolve(1)", ""),
    ("Error", "Error", 'new Error("m")', ""),
    ("ArrayBuffer", "ArrayBuffer", "new ArrayBuffer(2)", ""),
    ("Float64Array", "Float64Array", "new Float64Array([1.5])", ""),
    ("Int8Array", "Int8Array", "new Int8Array([1])", ""),
    ("Uint8Array", "Uint8Array", "new Uint8Array([1, 2])", ""),
    ("Buffer", "Buffer", "Buffer.from([1, 2])", ""),
    ("Math", None, "Math", ""),
    ("JSON", None, "JSON", ""),
]


def tag_probes():
    out = []
    for label, typ, value, prefix in TAG_RECV:
        modes = (["typed"] if typ else []) + ["any"]
        for mode in modes:
            decl = f"const b: {typ} = {value};" if mode == "typed" else f"const b: any = {value};"
            out.append(("tag", mode, f"tag.{mode}.{label}", f"{prefix}{decl} return b;",
                        ["Object.prototype.toString"]))
    return out


def build(rows, quota=QUOTA):
    """All built-in probes: [(family, mode, name, body, api)]; each family emits at most `quota` generated probes."""
    out = []
    for fam in FAMILIES:
        for cand in select(family_candidates(fam, rows), quota):
            name, body, api, mode = cand
            out.append((fam["family"], mode, name, body, api))
    return out + proxy_probes() + typeof_probes(rows) + tag_probes()


def lib_text():
    """lib.ts factories for module mode: one per receiver variant (same text serves lib.mts)."""
    lines = []
    for fam in FAMILIES:
        for v in fam["variants"]:
            lines.append(f"export function mod_{fam['family']}_{v['vname']}(): {v['T']} {{ "
                         f"{v['prefix']}return {v['MK']}; }}")
    return "\n".join(lines) + "\n"


def api_ids_used(probes):
    return {api for _, _, _, _, apis in probes for api in apis}
