#!/usr/bin/env bash
# Self-test for dts_parse.py: inline fixtures (every construct the parser claims to handle, the ASI and
# recovery paths, and the unparsed-fragment report), then, when a dump directory is given (argument 1 or
# PARITY_AOT_DTS_DIR), a parse of all builtin d.ts files with the unparsed count printed. No cargo needed for
# the fixtures; the dump directory comes from crates/thaw-registry/tests/dump_builtin_dts.rs.
# Exit 0 = all fixtures pass (and the dump parses), 1 = a fixture failed, 2 = bad dump directory.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
DTS_DIR=${1:-${PARITY_AOT_DTS_DIR:-}}
python3 - "$HERE" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import dts_parse as d

fails = []
cases = 0

def case(name, fn):
    global cases
    cases += 1
    try:
        fn()
        print(f"ok   parse: {name}")
    except AssertionError as e:
        fails.append(name)
        print(f"FAIL parse: {name}: {e}")
    except Exception as e:  # a crash in the parser is a failure too
        fails.append(name)
        print(f"FAIL parse: {name}: raised {type(e).__name__}: {e}")

def one(src):
    decls, un = d.parse_text(src)
    return decls, un

def c1():  # optional, rest, return type
    decls, un = one("export declare function f(a: string, b?: number, ...rest: any[]): boolean;")
    f = decls[0]
    assert f["kind"] == "function" and f["exported"] and not un
    ps = f["sigs"][0]["params"]
    assert [(p["name"], p["type"], p["optional"], p["rest"]) for p in ps] == [
        ("a", "string", False, False), ("b", "number", True, False), ("rest", "any[]", False, True)], ps
    assert f["sigs"][0]["ret"] == "boolean"

def c2():  # overloads are grouped by the reader into one entry per signature
    decls, un = one("export declare function g(x: string): string;\nexport declare function g(x: number, y: number): number;")
    assert len(decls) == 1 and not un
    assert [len(x["sigs"]) for x in decls] == [2]
    rows = d.export_rows("m", decls)
    assert rows[0]["export"] == "g" and rows[0]["overloads"] == 2 and rows[0]["arity_min"] == 1 and rows[0]["arity_max"] == 2, rows[0]

def c3():  # generics on the function and a default parameter
    decls, un = one("export declare function h<T extends object = {}>(a: T, b = 1): T;")
    sig = decls[0]["sigs"][0]
    assert sig["generics"] == "<T extends object = {}>", sig["generics"]
    assert [p["name"] for p in sig["params"]] == ["a", "b"] and sig["params"][1]["optional"]
    assert not un

def c4():  # const with a type, and a literal initialiser
    decls, un = one("export declare const sep: string;\nexport declare const version = \"1\";")
    assert [(x["kind"], x["name"]) for x in decls] == [("const", "sep"), ("const", "version")], decls
    assert decls[0]["type"] == "string" and not un

def c5():  # class: constructor, instance and static methods, optional property
    src = ("export declare class Box { constructor(size?: number); area(): number; static of(n: number): Box; "
           "label?: string; }")
    decls, un = one(src)
    cls = decls[0]
    assert cls["kind"] == "class" and not un
    kinds = [(m["kind"], m["name"]) for m in cls["members"]]
    assert kinds == [("constructor", "constructor"), ("method", "area"), ("method", "of"), ("property", "label")], kinds
    assert cls["members"][2]["static"] and cls["members"][3]["optional"]

def c6():  # interface with ',' separators, an index signature and a method
    decls, un = one("export interface Opts { a: string, b?: number; [key: string]: any; run(x: number): void; }")
    it = decls[0]
    assert it["kind"] == "interface" and not un
    assert [(m["kind"], m["name"]) for m in it["members"]] == [
        ("property", "a"), ("property", "b"), ("index", "[index]"), ("method", "run")], it["members"]

def c7():  # namespace with functions and consts (recursive)
    decls, un = one("export declare namespace ns { function a(x: number): void; const k: symbol; namespace inner { function z(): void; } }")
    ns = decls[0]
    assert ns["kind"] == "namespace" and not un
    assert [c["kind"] for c in ns["children"]] == ["function", "const", "namespace"], ns["children"]
    rows = d.export_rows("m", decls)
    assert [r["export"] for r in rows] == ["ns", "ns.a", "ns.k", "ns.inner", "ns.inner.z"], [r["export"] for r in rows]

def c8():  # ambient module: its body is the module's own export list
    decls, un = one('declare module "node:x" { export function f(): void; export const C: number; }')
    assert [x["name"] for x in decls] == ["f", "C"] and not un

def c9():  # type alias whose object body contains ';'; the ';' inside braces does not end the statement
    decls, un = one("export type Opts = { a: string; b: number; };\nexport declare const after: string;")
    assert [(x["kind"], x["name"]) for x in decls] == [("type", "Opts"), ("const", "after")] and not un

def c10():  # re-exports
    decls, un = one("export { a as b, c };\nexport * from './x';")
    assert [x["kind"] for x in decls] == ["reexport", "reexport"] and not un

def c11():  # missing ';' before a new line that starts a declaration (ASI): both parse, nothing is swallowed
    decls, un = one("export declare function p(cb: (e: any, v: Buffer) => void): void\nexport declare function q(): number;")
    assert [x["name"] for x in decls] == ["p", "q"] and not un, [x["name"] for x in decls]
    assert decls[0]["sigs"][0]["ret"] == "void"

def c12():  # unsupported `export =` is reported, and the next statement still parses
    decls, un = one("export = Foo;\nexport declare const ok: number;")
    assert [x["name"] for x in decls] == ["ok"], decls
    assert len(un) == 1 and "export =" in un[0][0] and "export assignment" in un[0][1], un

def c13():  # a function-type parameter keeps its full type text
    decls, un = one("export declare function on(event: string, cb: (err: Error, v?: string) => void): Emitter;")
    ps = decls[0]["sigs"][0]["params"]
    assert ps[1]["type"] == "(err: Error, v?: string) => void", ps[1]["type"]
    assert not un

def c14():  # destructured parameter is named <pattern>
    decls, un = one("export declare function opt({ a, b }: Opts, c: number): void;")
    assert [p["name"] for p in decls[0]["sigs"][0]["params"]] == ["<pattern>", "c"] and not un

def c15():  # string-literal union return type and quoted names
    decls, un = one('export declare function enc(e: string): "utf8" | "hex";')
    assert decls[0]["sigs"][0]["ret"] == '"utf8" | "hex"' and not un

def c16():  # enum is recognised by name only
    decls, un = one("export declare enum Color { Red, Green }")
    assert decls[0]["kind"] == "enum" and decls[0]["name"] == "Color" and not un

def c17():  # export default <identifier> is a 'default' export that names its target
    decls, un = one("export declare class EE {}\nexport default EE;")
    rows = d.export_rows("m", decls)
    assert [(r["export"], r["kind"], r["ret"]) for r in rows] == [("EE", "class", "void"), ("default", "default", "EE")] or \
        [(r["export"], r["kind"], r["ret"]) for r in rows][1] == ("default", "default", "EE"), rows
    assert not un

def c18():  # a statement the parser does not know is reported with its text, not dropped
    decls, un = one("foo bar baz;\nexport declare const z: number;")
    assert [x["name"] for x in decls] == ["z"] and len(un) == 1 and un[0][0].startswith("foo"), un

def c19():  # unbalanced parameter list: the broken statement is reported; the next one still parses
    decls, un = one("export declare function bad(a: string;\nexport declare const good: number;")
    assert "good" in [x["name"] for x in decls], [x["name"] for x in decls]
    assert len(un) >= 1, un

def c20():  # a class member fragment that fails is reported and the rest of the class is kept
    decls, un = one("export declare class C { ok(): void; ) weird; fine(): number; }")
    names = [m["name"] for m in decls[0]["members"]]
    assert names == ["ok", "fine"] and len(un) == 1, (names, un)

def c21():  # overload count and arity bounds through the flattening
    decls, un = one("export declare function ov(a: string): string;\nexport declare function ov(a: string, b: number, ...c: any[]): any;")
    r = d.export_rows("m", decls)[0]
    assert r["overloads"] == 2 and r["arity_min"] == 1 and r["arity_max"] == 999, r

for fn in [c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, c19, c20, c21]:
    case(fn.__name__, fn)
print(f"{cases - len(fails)}/{cases} parser fixtures pass")
sys.exit(1 if fails else 0)
PY
rc=$?
[ "$rc" = 0 ] || { echo "selftest_parse: FAILED fixtures"; exit 1; }
if [ -z "$DTS_DIR" ]; then
  echo "selftest_parse: skipped the builtin d.ts pass (no dump directory; run dump_builtin_dts.rs and pass its dir)"
  exit 0
fi
[ -d "$DTS_DIR" ] || { echo "selftest_parse: no dump directory $DTS_DIR" >&2; exit 2; }
python3 "$HERE/dts_parse.py" "$DTS_DIR" || { echo "selftest_parse: dts_parse failed on $DTS_DIR" >&2; exit 2; }
echo "selftest_parse: passed"
