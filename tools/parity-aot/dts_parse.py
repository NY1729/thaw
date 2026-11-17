#!/usr/bin/env python3
"""Parses the ambient .d.ts text of each builtin (dumped by crates/thaw-registry/tests/dump_builtin_dts.rs)
into per-module export signatures.

Usage: dts_parse.py DTS_DIR [--modules modules.txt] [--json OUT] [--tsv OUT] [--all-unparsed]

File <DIR>/<module with ':' and '/' replaced by '_'>.d.ts maps back to a module through modules.txt.
Recognised: `export [declare] function` (overloads kept, generics, optional and rest params, defaults),
`const|let|var`, `class` and `interface` (constructor, methods, properties, static, index and call signatures,
get/set), `namespace` (recursive), `module "x" { }` (its body is the module's own exports), `type`, `enum`
(name only), `export { }`, `export * from`, `import`. Anything else, or a statement that fails to parse, is
an UNPARSED fragment: counted per module and listed (never silently dropped).
Exit 0 on success (unparsed fragments are data, not failure), 2 on a missing dir or unmappable file.
"""
import json
import os
import re
import sys

# ---------------------------------------------------------------- tokens

TOKEN = re.compile(
    r"""(?P<ws>\s+)|(?P<lc>//[^\n]*)|(?P<bc>/\*[\s\S]*?\*/)"""
    r"""|(?P<str>"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*')"""
    r"""|(?P<num>\d+(?:\.\d+)?)|(?P<id>[A-Za-z_$][\w$]*)|(?P<op>\.\.\.|=>)|(?P<p>\S)"""
)
OPEN = {"(": ")", "[": "]", "{": "}", "<": ">"}
CLOSE = set(")]}>")
# tokens after which a type continues on the next line (a new line after these is not a terminator)
TYPE_CONTINUES = {"|", "&", "=>", ":", ",", "(", "<", "[", "{", "?", "=", ".", "keyof", "extends", "is"}
STATEMENT_KEYWORDS = {"export", "declare", "function", "class", "interface", "const", "let", "var", "type",
                      "namespace", "module", "enum", "import"}
MODIFIERS = {"export", "declare", "abstract", "readonly", "public", "private", "protected", "async", "static"}


class Tok:
    __slots__ = ("kind", "text", "start", "end")

    def __init__(self, kind, text, start, end):
        self.kind, self.text, self.start, self.end = kind, text, start, end

    def __repr__(self):
        return f"Tok({self.kind},{self.text!r})"


def tokenize(src):
    out = []
    for m in TOKEN.finditer(src):
        kind = m.lastgroup
        if kind in ("ws", "lc", "bc"):
            continue
        if kind == "p" and m.group() == "`":
            raise ValueError("template literal type is not supported")
        out.append(Tok("str" if kind == "str" else kind, m.group(), m.start(), m.end()))
    return out


class Unparsed(Exception):
    pass


class Parser:
    def __init__(self, src):
        self.src = src
        self.t = tokenize(src)
        self.i = 0
        self.unparsed = []  # (raw text of the fragment, reason)

    # -- token helpers
    def peek(self, k=0):
        j = self.i + k
        return self.t[j] if j < len(self.t) else None

    def at(self, text, k=0):
        tok = self.peek(k)
        return tok is not None and tok.text == text and tok.kind in ("p", "op", "id")

    def take(self, text=None):
        tok = self.peek()
        if tok is None:
            raise Unparsed("unexpected end of input")
        if text is not None and tok.text != text:
            raise Unparsed(f"expected {text!r}, got {tok.text!r}")
        self.i += 1
        return tok

    def name(self):
        tok = self.peek()
        if tok is None or tok.kind not in ("id", "str", "num"):
            raise Unparsed(f"expected a name, got {tok.text if tok else 'end'!r}")
        self.i += 1
        return tok.text.strip("\"'")

    def raw(self, start_i, end_i):
        if start_i >= end_i:
            return ""
        return " ".join(self.src[self.t[start_i].start:self.t[end_i - 1].end].split())

    def skip_balanced(self):
        """At an opener: consumes through its matching closer."""
        depth = 0
        while True:
            tok = self.peek()
            if tok is None:
                raise Unparsed("unbalanced brackets")
            if tok.kind == "p" and tok.text in OPEN:
                depth += 1
            elif tok.kind == "p" and tok.text in CLOSE:
                depth -= 1
            self.i += 1
            if depth == 0:
                return

    def read_type(self, stops):
        """Raw text of a type up to a stop token at bracket depth 0 (not consumed)."""
        start = self.i
        depth = 0
        while True:
            tok = self.peek()
            if tok is None:
                if depth:
                    raise Unparsed("unterminated type")
                break
            if depth == 0 and tok.text in stops and tok.kind in ("p", "op"):
                break
            # a missing ';' before a new line that starts with an identifier ends the type (ASI); the next
            # declaration is then parsed on its own instead of being swallowed into this type
            if depth == 0 and tok.kind == "id" and self.i > start and self.newline_before(self.i) \
                    and self.t[self.i - 1].text not in TYPE_CONTINUES:
                break
            if tok.kind == "p" and tok.text in OPEN:
                depth += 1
            elif tok.kind == "p" and tok.text in CLOSE:
                depth -= 1
                if depth < 0:
                    break
            self.i += 1
        text = self.raw(start, self.i)
        if not text:
            raise Unparsed("empty type")
        return text

    def newline_before(self, i):
        return "\n" in self.src[self.t[i - 1].end:self.t[i].start]

    def generics(self):
        if not self.at("<"):
            return ""
        start = self.i
        self.skip_balanced()
        return self.raw(start, self.i)

    # -- signatures
    def params(self):
        self.take("(")
        out = []
        while not self.at(")"):
            rest = False
            if self.peek() and self.peek().kind == "op" and self.peek().text == "...":
                rest = True
                self.take()
            tok = self.peek()
            if tok is None:
                raise Unparsed("unterminated parameter list")
            if tok.text in ("{", "["):
                pname = "<pattern>"
                self.skip_balanced()
            else:
                pname = self.name()
            optional = False
            if self.at("?"):
                optional = True
                self.take()
            ptype = "any"
            if self.at(":"):
                self.take()
                ptype = self.read_type({",", ")", "="})
            default = False
            if self.at("="):
                default = True
                self.take()
                self.read_type({",", ")"})
            out.append({"name": pname, "type": ptype, "optional": optional or default, "rest": rest})
            if self.at(","):
                self.take()
            elif not self.at(")"):
                raise Unparsed(f"expected ',' or ')' in parameters, got {self.peek().text!r}")
        self.take(")")
        return out

    def signature(self, stops):
        """generics? ( params ) : ret? -- ret ends at a stop token."""
        gen = self.generics()
        params = self.params()
        ret = "void"
        if self.at(":"):
            self.take()
            ret = self.read_type(stops)
        return {"generics": gen, "params": params, "ret": ret}

    # -- statements (module / namespace level)
    def unparsed_fragment(self, start, reason):
        """Skips to the end of the statement at `start` and records it as unparsed. Recovery stops at a ';' at
        bracket depth 0, after a closing '}' that returns to depth 0, or before a declaration keyword that starts
        a new line (a broken bracket must not swallow the statements after it)."""
        self.i = start
        depth = 0
        while self.peek() is not None:
            tok = self.peek()
            if self.i > start and tok.kind == "id" and tok.text in STATEMENT_KEYWORDS and self.newline_before(self.i):
                break
            if tok.kind == "p" and tok.text in OPEN:
                depth += 1
            elif tok.kind == "p" and tok.text in CLOSE:
                depth -= 1
            elif tok.text == ";" and depth <= 0:
                self.i += 1
                break
            self.i += 1
            if depth == 0 and self.i > start and self.t[self.i - 1].text == "}":
                break
        text = self.raw(start, self.i)
        self.unparsed.append((text, reason))

    def statements(self, closer):
        """Parses declarations until `closer` ('}' or None for end of input)."""
        decls = []
        while True:
            while self.at(";"):
                self.take()
            tok = self.peek()
            if tok is None:
                if closer is not None:
                    raise Unparsed("missing '}'")
                break
            if closer is not None and tok.text == "}" and tok.kind == "p":
                self.take()
                break
            start = self.i
            try:
                decl = self.statement()
                if decl is not None:
                    prev = decls[-1] if decls else None
                    if decl["kind"] == "function" and prev is not None and prev["kind"] == "function" \
                            and prev["name"] == decl["name"]:
                        prev["sigs"].extend(decl["sigs"])  # overloads of one function
                    else:
                        decls.append(decl)
            except Unparsed as e:
                self.unparsed_fragment(start, str(e))
        return decls

    def statement(self):
        exported = False
        while self.peek() and self.peek().kind == "id" and self.peek().text in ("export", "declare"):
            if self.peek().text == "export":
                exported = True
                if self.at("=", 1) or self.at("as", 1):
                    raise Unparsed("export assignment (export =) is not supported")
                if self.at("{", 1) or self.at("*", 1):
                    self.take()
                    start = self.i
                    if self.at("{"):
                        self.skip_balanced()
                    else:
                        self.take("*")
                    if self.at("from"):
                        self.take()
                        self.take()
                    self.take(";") if self.at(";") else None
                    return {"kind": "reexport", "name": self.raw(start, self.i), "exported": True}
            self.take()
        tok = self.peek()
        if tok is None:
            return None
        if tok.kind == "id" and tok.text == "import":
            start = self.i
            while self.peek() is not None and not self.at(";"):
                self.take()
            self.take(";")
            return {"kind": "import", "name": self.raw(start, self.i), "exported": exported}
        if tok.kind != "id":
            raise Unparsed(f"unexpected token {tok.text!r}")
        kw = tok.text
        if kw == "default" and exported:
            self.take()
            target = self.name()
            self.take(";") if self.at(";") else None
            return {"kind": "default", "name": "default", "exported": True, "target": target, "sigs": [], "members": []}
        if kw == "function":
            self.take()
            name = self.name()
            sig = self.signature({";"})
            self.take(";") if self.at(";") else None
            return {"kind": "function", "name": name, "exported": exported, "sigs": [sig], "members": []}
        if kw in ("const", "let", "var"):
            self.take()
            name = self.name()
            ctype = "any"
            if self.at(":"):
                self.take()
                ctype = self.read_type({";", "="})
            if self.at("="):
                self.take()
                self.read_type({";"})
            self.take(";") if self.at(";") else None
            return {"kind": "const", "name": name, "exported": exported, "type": ctype}
        if kw in ("class", "interface"):
            self.take()
            name = self.name()
            gen = self.generics()
            heritage_start = self.i
            while not self.at("{"):
                if self.peek() is None:
                    raise Unparsed(f"{kw} without body")
                if self.peek().kind == "p" and self.peek().text in OPEN and self.peek().text != "{":
                    self.skip_balanced()
                else:
                    self.take()
            heritage = self.raw(heritage_start, self.i)
            self.take("{")
            members = self.members()
            return {"kind": kw, "name": name, "exported": exported, "generics": gen, "heritage": heritage,
                    "members": members, "sigs": []}
        if kw in ("namespace", "module"):
            self.take()
            name_tok = self.peek()
            name = self.name()
            if not self.at("{"):
                raise Unparsed(f"{kw} {name} without body")
            self.take("{")
            children = self.statements("}")
            if kw == "module" and name_tok.kind == "str":
                return {"kind": "ambient-module", "name": name, "exported": True, "children": children,
                        "members": [], "sigs": []}
            return {"kind": "namespace", "name": name, "exported": exported, "children": children,
                    "members": [], "sigs": []}
        if kw == "type":
            self.take()
            name = self.name()
            self.generics()
            self.take("=")
            self.read_type({";"})
            self.take(";") if self.at(";") else None
            return {"kind": "type", "name": name, "exported": exported, "members": [], "sigs": []}
        if kw == "enum":
            self.take()
            name = self.name()
            if not self.at("{"):
                raise Unparsed("enum without body")
            self.skip_balanced()
            return {"kind": "enum", "name": name, "exported": exported, "members": [], "sigs": []}
        raise Unparsed(f"unsupported statement starting {kw!r}")

    # -- class / interface members
    def members(self):
        out = []
        while True:
            while self.at(";") or self.at(","):
                self.take()
            tok = self.peek()
            if tok is None:
                raise Unparsed("missing '}' in class/interface body")
            if tok.kind == "p" and tok.text == "}":
                self.take()
                return out
            start = self.i
            try:
                m = self.member()
                out.append(m)
            except Unparsed as e:
                self.unparsed_fragment(start, str(e))
        return out

    def member(self):
        static = False
        while self.peek() and self.peek().kind == "id" and self.peek().text in MODIFIERS:
            nxt = self.peek(1)
            if nxt is None or nxt.text in ("(", ":", "?", "<", ";", ","):
                break  # the modifier word is itself the member name
            if self.peek().text == "static":
                static = True
            self.take()
        tok = self.peek()
        if tok is None:
            raise Unparsed("unexpected end in member")
        if tok.kind == "p" and tok.text == "[":
            self.skip_balanced()
            kind = "index"
            name = "[index]"
            if self.at(":"):
                self.take()
                self.read_type({";", ","})
            self.take() if self.at(";") or self.at(",") else None
            return {"kind": kind, "name": name, "static": static, "sigs": []}
        if tok.kind == "p" and tok.text in ("(", "<"):
            sig = self.signature({";", ","})
            self.take() if self.at(";") or self.at(",") else None
            return {"kind": "call", "name": "()", "static": static, "sigs": [sig]}
        if tok.kind == "id" and tok.text == "new" and (self.at("(", 1) or self.at("<", 1)):
            self.take()
            sig = self.signature({";", ","})
            self.take() if self.at(";") or self.at(",") else None
            return {"kind": "construct", "name": "new", "static": static, "sigs": [sig]}
        if tok.kind == "id" and tok.text == "constructor" and self.at("(", 1):
            self.take()
            sig = self.signature({";", ","})
            self.take() if self.at(";") or self.at(",") else None
            return {"kind": "constructor", "name": "constructor", "static": static, "sigs": [sig]}
        if tok.kind == "id" and tok.text in ("get", "set") and self.peek(1) and self.peek(1).kind in ("id", "str"):
            self.take()
        name = self.name()
        optional = False
        if self.at("?"):
            optional = True
            self.take()
        if self.at("(") or self.at("<"):
            sig = self.signature({";", ","})
            self.take() if self.at(";") or self.at(",") else None
            return {"kind": "method", "name": name, "static": static, "optional": optional, "sigs": [sig]}
        ptype = "any"
        if self.at(":"):
            self.take()
            ptype = self.read_type({";", ","})
        self.take() if self.at(";") or self.at(",") else None
        return {"kind": "property", "name": name, "static": static, "optional": optional, "type": ptype, "sigs": []}


def parse_text(src):
    """Returns (decls, unparsed). A decl is a dict with kind, name, exported, sigs, members, children."""
    p = Parser(src)
    decls = p.statements(None)
    flat = []
    for d in decls:
        if d["kind"] == "ambient-module":
            flat.extend(d["children"])
        else:
            flat.append(d)
    return flat, p.unparsed


# ---------------------------------------------------------------- flattening

def export_rows(module, decls, prefix=""):
    """One row per export: module, export, kind, overloads, arity_min, arity_max, params, ret, members, unparsed."""
    rows = []
    for d in decls:
        name = prefix + d["name"]
        if d["kind"] in ("reexport", "import"):
            rows.append(row(module, name, d["kind"], [], 0))
            continue
        if d["kind"] == "namespace":
            rows.append(row(module, name, "namespace", [], len(d["children"])))
            rows.extend(export_rows(module, d["children"], name + "."))
            continue
        r = row(module, name, d["kind"], d.get("sigs", []), len(d.get("members", [])))
        if "target" in d:
            r["ret"] = d["target"]
        rows.append(r)
    return rows


def row(module, name, kind, sigs, members):
    amin = [sum(1 for p in s["params"] if not p["optional"] and not p["rest"]) for s in sigs]
    amax = [999 if any(p["rest"] for p in s["params"]) else len(s["params"]) for s in sigs]
    params = ["; ".join(f"{p['name']}{'?' if p['optional'] else ''}{'...' if p['rest'] else ''}: {p['type']}"
                        for p in s["params"]) for s in sigs]
    rets = [s["ret"] for s in sigs]
    return {
        "module": module, "export": name, "kind": kind, "overloads": len(sigs),
        "arity_min": min(amin) if amin else "-", "arity_max": max(amax) if amax else "-",
        "params": " | ".join(params) if params else "-", "ret": " | ".join(rets) if rets else "-",
        "members": members,
    }


# ---------------------------------------------------------------- driver

def mangle(module):
    return module.replace(":", "_").replace("/", "_")


def main(argv):
    args = list(argv)
    show_all = "--all-unparsed" in args
    if show_all:
        args.remove("--all-unparsed")
    opts = {}
    for flag in ("--modules", "--json", "--tsv"):
        if flag in args:
            k = args.index(flag)
            opts[flag] = args[k + 1]
            del args[k:k + 2]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    dts_dir = args[0]
    modules_file = opts.get("--modules", os.path.join(os.path.dirname(os.path.abspath(__file__)), "modules.txt"))
    if not os.path.isdir(dts_dir):
        print(f"dts_parse: no such directory {dts_dir}", file=sys.stderr)
        return 2
    specs = [s.strip() for s in open(modules_file, encoding="utf-8") if s.strip()]
    by_file = {mangle(s) + ".d.ts": s for s in specs}
    files = sorted(f for f in os.listdir(dts_dir) if f.endswith(".d.ts"))
    unknown = [f for f in files if f not in by_file]
    if unknown:
        print(f"dts_parse: files not mapped to a module in {modules_file}: {unknown}", file=sys.stderr)
        return 2
    missing = [by_file[f] for f in by_file if f not in files]
    all_rows, unparsed = [], []
    per_module = {}
    for f in files:
        mod = by_file[f]
        decls, bad = parse_text(open(os.path.join(dts_dir, f), encoding="utf-8").read())
        rows = export_rows(mod, decls)
        all_rows.extend(rows)
        unparsed.extend((mod, text, reason) for text, reason in bad)
        kinds = {}
        for r in rows:
            kinds[r["kind"]] = kinds.get(r["kind"], 0) + 1
        per_module[mod] = {"exports": len(rows), "unparsed": len(bad), "kinds": kinds}
    all_rows.sort(key=lambda r: (r["module"], r["export"]))
    print(f"dts_parse: {len(files)} d.ts files, {len(all_rows)} export rows, "
          f"{len(unparsed)} unparsed fragments in {sum(1 for m in per_module.values() if m['unparsed'])} modules")
    if missing:
        print(f"dts_parse: {len(missing)} modules have no d.ts file: {missing}")
    kind_totals = {}
    for r in all_rows:
        kind_totals[r["kind"]] = kind_totals.get(r["kind"], 0) + 1
    print("kinds: " + " ".join(f"{k}={v}" for k, v in sorted(kind_totals.items())))
    shown = unparsed if show_all else unparsed[:5]
    if unparsed:
        print(f"unparsed fragments (showing {len(shown)} of {len(unparsed)}):")
        for mod, text, reason in shown:
            print(f"  [{mod}] {text[:160]!r}  -- {reason}")
    if "--json" in opts:
        with open(opts["--json"], "w", encoding="utf-8") as fh:
            json.dump({"rows": all_rows, "per_module": per_module,
                       "unparsed": [{"module": m, "text": t, "reason": r} for m, t, r in unparsed]},
                      fh, indent=1, sort_keys=True)
    if "--tsv" in opts:
        cols = ["module", "export", "kind", "overloads", "arity_min", "arity_max", "params", "ret", "members"]
        with open(opts["--tsv"], "w", encoding="utf-8") as fh:
            fh.write("\t".join(cols) + "\n")
            for r in all_rows:
                fh.write("\t".join(str(r[c]).replace("\t", " ") for c in cols) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
