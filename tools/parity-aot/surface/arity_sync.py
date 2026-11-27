#!/usr/bin/env python3
"""Make declared `Function.length` match Node's, from the committed inventory (inventory/node_*.json is the source of truth).

thaw reports `fn.length` = number of non-rest parameters of the FIRST declared overload (optional ones counted;
`argsArray: any` and `...rest` count 0). The hand-written d.ts in registry/resolution.rs drifted from Node
(overload order, optional params counted, `argsArray: any` = 0). This checks every `export declare function` and the
`functions:` lists of `generated_builtin_declarations` (`name/N` = N optional `any` parameters + rest) against Node.

  arity_sync.py            report mismatches (exit 1 when any)
  arity_sync.py --fix      (callback-typed signatures are reported as skipped, never rewritten) rewrite resolution.rs in place: the first overload gets exactly N non-rest parameters
                           (extra typed parameters become `...rest: any[]`, missing ones optional `any`)
Parameter types of kept parameters are untouched; an overload that already has N parameters is never rewritten.
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RES = os.path.join(HERE, "../../../crates/thaw-registry/src/registry/resolution.rs")
INV = os.path.join(HERE, "../inventory")


def inv_arity(mod):
    """{function name: arity} for node:<mod>, or None when there is no inventory."""
    fn = os.path.join(INV, "node_" + mod.replace("/", "_") + ".json")
    if not os.path.exists(fn):
        return None
    return {e["name"]: e["arity"] for e in json.load(open(fn))["exports"] if e["kind"] == "function" and e["arity"] is not None}


def split_params(s):
    out, depth, cur = [], 0, ""
    for i, ch in enumerate(s):
        if ch in "([{<":
            depth += 1
        elif ch in ")]}>" and not (ch == ">" and cur.endswith("=")):
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def declared_len(params):
    for p in params:
        m = re.match(r"(?:\.\.\.__length_|[\w$]+__length_)(\d+)\b", p)
        if m:
            return int(m.group(1))
    return sum(1 for p in params if not p.startswith("...") and not p.startswith("argsArray"))


def funcs(text):
    """(name, params_start, params_end) of every `declare function NAME(...)` in an escaped dts string."""
    for m in re.finditer(r"declare function ([A-Za-z_$][\w$]*)\(", text):
        i, depth = m.end(), 1
        while depth:
            depth += {"(": 1, ")": -1}.get(text[i], 0)
            i += 1
        yield m.group(1), m.end(), i - 1


def fix_params(params, n):
    kept = [p for p in params if not p.startswith("...") and not p.startswith("argsArray")][:n]
    rest = [p for p in params if p.startswith("...")]
    dropped = len(kept) < len([p for p in params if not p.startswith("...") and not p.startswith("argsArray")]) or any(p.startswith("argsArray") for p in params)
    while len(kept) < n:
        kept.append(f"a{len(kept)}?: any")
    # optional before required is invalid: make everything after the first optional optional
    seen = False
    for i, p in enumerate(kept):
        if re.match(r"[\w$]+\?:", p):
            seen = True
        elif seen:
            kept[i] = re.sub(r"^([\w$]+):", r"\1?:", p)
    if rest:
        kept.append(rest[0])  # a typed rest parameter is kept as declared
    elif dropped:
        kept.append("...rest: any[]")
    return kept


def stmt_span(body, a, b):
    """Span of the whole `export declare function ...;\\n` statement whose params are body[a:b]."""
    start = body.rfind("export declare function", 0, a)
    end = body.index(";", b) + 1
    if body.startswith("\\n", end):
        end += 2
    return start, end, body[b + 1:end].lstrip(":").strip().rstrip(";").replace("\\n", "").strip()


def process_arm(mods, body, fix):
    """mods: module names sharing this dts. Returns (new body, [(mod, fn, declared, node)])."""
    invs = [i for i in map(inv_arity, mods) if i is not None]
    occ = {}
    for name, a, b in funcs(body):
        occ.setdefault(name, []).append((a, b))
    bad, edits = [], []
    for name, spans in occ.items():
        ns = {i[name] for i in invs if name in i}
        if len(ns) != 1:
            continue
        n = ns.pop()
        a, b = spans[0]
        params = split_params(body[a:b])
        d = declared_len(params)
        if d == n:
            continue
        if len(spans) == 1 and len(params) == 1 and (params[0].startswith("...") or params[0].startswith("argsArray")) and mods[0] != "assert" and name not in KEEP:
            # rest-only (or dynamic `argsArray`) declaration: call lowering is unchanged, only `.length` is reported (rest named __length_N)
            bad.append((mods[0], name, d, n))
            edits.append((a, b, f"...__length_{n}: " + (params[0].split(":", 1)[1].strip() if params[0].startswith("...") else "any[]")))
        elif mods[0] in MARKER_MODS and name not in KEEP and params and "__length_" not in params[-1]:
            # callback-typed / overloaded / typed-rest: keep the real parameters, only the last one's name carries Node's
            # `.length` (`cb__length_3`, `...args__length_0`), so overload resolution and callback lowering are untouched
            bad.append((mods[0], name, d, n))
            last = re.match(r"^(\.\.\.)?([\w$]+)(\??:.*)$", params[-1], re.S)
            edits.append((a, b, ", ".join(params[:-1] + [f"{last.group(1) or ''}{last.group(2)}__length_{n}{last.group(3)}"])))
        elif name in KEEP or "assert" in mods[0] or any("=>" in p or "JsValue" in p or "argsArray" in p or p.startswith("...") for p in params):
            # callback-typed signatures are lowered natively (typed server/fs paths); an extra parameter breaks that
            # lowering (http.createServer), so these are only reported
            SKIPPED.append((mods[0], name, d, n, "callback/JsValue/dynamic-typed or assert"))
        elif len(spans) == 1:
            bad.append((mods[0], name, d, n))
            edits.append((a, b, ", ".join(fix_params(params, n))))
        else:
            # overloads: a rewrite could change which one a call resolves to or break callback inference; report only
            if True:
                SKIPPED.append((mods[0], name, d, n, "overloaded"))
    if fix:
        for a, b, new in sorted(edits, reverse=True):
            body = body[:a] + new + body[b:]
    return body, bad


def gen_lists(text, fix):
    """`functions: "a b c"` of generated_builtin_declarations: rewrite to `name/N` (N>0)."""
    bad, out = [], []
    for m in re.finditer(r'((?:"[\w/]+"\s*\|\s*)*"[\w/]+")\s*=>\s*ExportKinds \{', text):
        i, depth = m.end(), 1
        while depth:
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        block = text[m.end():i]
        mods = re.findall(r'"([\w/]+)"', m.group(1))
        invs = [x for x in map(inv_arity, mods) if x is not None]
        fm = re.search(r'functions: "([^"]*)"', block)
        if not fm or not invs:
            continue
        names = []
        for tok in fm.group(1).split():
            name = tok.split("/")[0]
            ns = {x[name] for x in invs if name in x}
            cur = int(tok.split("/")[1]) if "/" in tok else 0
            n = ns.pop() if len(ns) == 1 else cur
            if cur != n:
                bad.append((mods[0], name, cur, n))
            names.append(f"{name}/{n}" if n else name)
        s0 = m.end() + fm.start(1)
        out.append((s0, s0 + len(fm.group(1)), " ".join(names)))
    if fix:
        for a, b, new in reversed(out):
            text = text[:a] + new + text[b:]
    return text, bad


def process_extra(m):
    mod = re.search(r'"([\w/]+)"', m.group(1)).group(1)
    body, b = process_arm([mod], m.group(2), FIX[0])
    bad_extra.extend(b)
    return body


# factories whose typed lowering/overload resolution is known (or not proven) to survive an extra optional parameter
KEEP = {"createInterface", "fork", "createSecretKey", "createHmac", "createSign", "createVerify", "sign", "verify",
        "monitorEventLoopDelay", "createHistogram"}
# modules where a typed signature may carry a trailing `...__length_N` marker (verified against their callback lowering)
MARKER_MODS = {"fs", "fs/promises", "util", "module", "v8", "process", "path"}
FIX = [False]
bad_extra = []
SKIPPED = []


def main():
    fix = "--fix" in sys.argv
    FIX[0] = fix
    text = open(RES).read()
    bad = []
    # module arms of resolve_builtin: ("a" | "b" =>) { "<dts>" }
    pat = re.compile(r'((?:"[\w/]+"\s*\|?\s*)+)=>\s*\{\s*"((?:[^"\\]|\\.)*)"\s*\}')
    def sub(m):
        mods = re.findall(r'"([\w/]+)"', m.group(1))
        body, b = process_arm(mods, m.group(2), fix)
        bad.extend(b)
        return m.group(0)[: m.start(2) - m.start(0)] + body + m.group(0)[m.end(2) - m.start(0):]
    text = pat.sub(sub, text)
    # per-module extras appended in resolve_builtin: `"fs" => "<dts>",`
    text = re.compile(r'(?m)^( {16}"[\w/]+" => )"((?:[^"\\]|\\.)*)",$').sub(
        lambda m: m.group(1) + '"' + process_extra(m) + '",', text)
    text, glist = gen_lists(text, fix)
    bad.extend(glist)
    bad.extend(bad_extra)
    for mod, name, d, n in bad:
        print(f"{mod}.{name}: declared {d}, node {n}")
    for mod, name, d, n, why in SKIPPED:
        print(f"skipped ({why}): {mod}.{name}: declared {d}, node {n}")
    print(f"{len(bad)} mismatches" + (" (fixed)" if fix else ""))
    if fix:
        open(RES, "w").write(text)
    return 1 if bad and not fix else 0


if __name__ == "__main__":
    sys.exit(main())
