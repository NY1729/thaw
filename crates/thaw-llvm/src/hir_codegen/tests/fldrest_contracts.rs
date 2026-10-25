// Remaining class-field gaps (constructor-less bare fields, Map/Set/Bytes/function/any/nullable
// fields, callback/generic reads, live structural views, untyped `new Map()`). Expected output
// recorded from node 22 (--experimental-strip-types).

#[test]
fn fldrest_bare_fields_of_constructorless_classes_read_undefined() {
    let source = r##"class A { x: number; s: string; b: boolean; }
class B extends A { y: number[]; }
function main(): void {
  const a = new A();
  console.log(a.x, a.s, a.b, typeof a.x, a.x === undefined);
  console.log(a);
  const b = new B();
  console.log(b);
  a.x = 3; a.s = "q";
  console.log(a, a.x + 1);
  console.log(JSON.stringify(new A()));
}
"##;
    assert_eq!(
        compile_and_run(source, "fldrest_bare_fields_of_constructorless_classes_read_undefined"),
        "undefined undefined undefined undefined true\nA { x: undefined, s: undefined, b: undefined }\nB { x: undefined, s: undefined, b: undefined, y: undefined }\nA { x: 3, s: 'q', b: undefined } 4\n{}\n"
    );
}

#[test]
fn fldrest_map_set_bytes_function_any_and_nullable_fields_read_undefined() {
    let source = r##"class H {
  m!: Map<string, number>; st!: Set<number>; by!: Uint8Array; f!: () => number; j!: any; nl!: number | null; ns!: string | null;
  fill(): void { this.m = new Map(); this.m.set("a", 1); this.st = new Set([1, 2]); this.by = new Uint8Array(2); this.f = () => 7; this.j = { k: 1 }; this.nl = 4; this.ns = "z"; }
}
class K {
  m: Map<string, number>; j: any; nl: number | null;
  constructor(c: boolean) { if (c) { this.m = new Map(); this.j = 5; this.nl = null; } }
}
function main(): void {
  const h = new H();
  console.log(h.m, h.st, h.by, h.f, h.j, h.nl, h.ns);
  console.log(h);
  console.log(h.m === undefined, h.st === undefined, h.by === undefined, h.f === undefined, h.j === undefined, h.nl === undefined, h.nl === null);
  console.log(typeof h.m, typeof h.f, typeof h.j, typeof h.nl);
  console.log(JSON.stringify(h), Object.keys(h));
  h.fill();
  console.log(h.m, h.st, h.by.length, h.f(), h.j, h.nl, h.ns);
  console.log(h.m.get("a"), h.st.has(2), h.m.size, h.j.k);
  const k = new K(false); console.log(k);
  const k2 = new K(true); console.log(k2);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldrest_map_set_bytes_function_any_and_nullable_fields_read_undefined"),
        "undefined undefined undefined undefined undefined undefined undefined\nH {\n  m: undefined,\n  st: undefined,\n  by: undefined,\n  f: undefined,\n  j: undefined,\n  nl: undefined,\n  ns: undefined\n}\ntrue true true true true true false\nundefined undefined undefined undefined\n{} [\n  'm',  'st', 'by',\n  'f',  'j',  'nl',\n  'ns'\n]\nMap(1) { 'a' => 1 } Set(2) { 1, 2 } 2 7 { k: 1 } 4 z\n1 true 1 1\nK { m: undefined, j: undefined, nl: undefined }\nK { m: Map(0) {}, j: 5, nl: null }\n"
    );
}

#[test]
fn fldrest_value_context_reads_keep_undefined_through_callbacks_generics_and_casts() {
    let source = r##"class P { n!: number; s!: string; tags!: string[]; set(n: number, s: string): void { this.n = n; this.s = s; this.tags = [s]; } }
function main(): void {
  more();
  const a = [new P(), new P()];
  a[0].set(3, "x"); a[1].set(1, "yy");
  console.log(a.map(x => x.n).reduce((s, v) => s + v, 0));
  console.log(Math.max(...a.map(x => x.n)));
  console.log(a.map(x => x.n).join(","));
  console.log(a.map(x => x.n).filter(v => v > 1));
  console.log(a.map(x => x.n * 2));
  console.log(a.map(x => x.s.length));
  console.log(a.map(x => x.s).sort());
  console.log(a.map(x => x.n).sort((p, q) => p - q));
  console.log(a.map(x => x.tags).flat());
  console.log(a.map(x => x.s).includes("x"), a.map(x => x.n).indexOf(1));
  const total = a.reduce((acc, x) => acc + x.n, 0);
  console.log(total);
  const names = a.map(x => { return x.s; });
  console.log(names.join("-"), a.map(x => x.s.toUpperCase()));
  const e = [new P()];
  console.log(e.map(x => x.n), e.map(x => x.s), e.map(x => x.tags));
  console.log(a.map(x => x.n).map(v => v + 1));
}
class Q { n!: number; s!: string; o?: number; set(): void { this.n = 5; } }
function id<T>(x: T): T { return x; }
function show(v: number | undefined): void { console.log(v); }
function twice(x: number): number { return x * 2; }
function more(): void {
  const q = [new Q(), new Q()];
  console.log(q.map(x => x.n), q.map(x => x.s), q.map(x => x.o));
  q[0].set();
  console.log(q.map(x => x.n), q.filter(x => x.n === undefined).length);
  const p = new Q();
  console.log(id(p.n), id(p.s), (p.n as number), [p.n, p.s]);
  show(p.n);
  console.log(twice(p.n), Math.max(p.n, 1), String(p.n), `${p.s}`);
}

"##;
    assert_eq!(
        compile_and_run(source, "fldrest_value_context_reads_keep_undefined_through_callbacks_generics_and_casts"),
        "[ undefined, undefined ] [ undefined, undefined ] [ undefined, undefined ]\n[ 5, undefined ] 1\nundefined undefined undefined [ undefined, undefined ]\nundefined\nNaN NaN undefined undefined\n4\n3\n3,1\n[ 3 ]\n[ 6, 2 ]\n[ 1, 2 ]\n[ 'x', 'yy' ]\n[ 1, 3 ]\n[ 'x', 'yy' ]\ntrue 1\n4\nx-yy [ 'X', 'YY' ]\n[ undefined ] [ undefined ] [ undefined ]\n[ 4, 2 ]\n"
    );
}

#[test]
fn fldrest_structural_parameter_is_a_live_view_of_a_widened_class_instance() {
    let source = r##"class P { n!: number; t: string = "t"; set(): void { this.n = 5; } }
function bump(v: { n: number }): void { v.n = v.n + 1; v.n += 10; }
function put(v: { n: number }): void { v.n = 42; }
function rd(v: { n: number }): number { return v.n; }
function main(): void {
  const p = new P();
  console.log(typeof p.n);
  put(p);
  console.log(p.n, p);
  const q = new P(); q.set();
  bump(q);
  console.log(q.n, q);
  const r = new P();
  bump(r);
  console.log(r.n);
  const lit: { n: number } = p;
  lit.n = 9; console.log(p.n);
  p.n = 11; console.log(lit.n);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldrest_structural_parameter_is_a_live_view_of_a_widened_class_instance"),
        "undefined\n42 P { n: 42, t: 't' }\n16 P { n: 16, t: 't' }\nNaN\n9\n11\n"
    );
}

#[test]
fn fldrest_untyped_new_map_takes_the_annotated_type_and_abstract_fields_keep_node_order() {
    let source = r##"abstract class Base { abstract a: number; abstract b: string; c: number = 1; }
class Impl extends Base { a: number = 2; b: string = "x"; d: number = 3; }
function main(): void {
  console.log(new Impl());
  const m: Map<string, number> = new Map();
  let m2: Map<string, number>;
  m2 = new Map();
  m2.set("a", 1);
  let s2: Set<number>;
  s2 = new Set();
  s2.add(1);
  console.log(m, m2, s2);
  class Z { m: Map<string, number> = new Map(); w!: Map<string, number>; init(): void { this.w = new Map(); this.w.set("k", 2); } }
  const z = new Z(); z.init(); console.log(z);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldrest_untyped_new_map_takes_the_annotated_type_and_abstract_fields_keep_node_order"),
        "Impl { c: 1, a: 2, b: 'x', d: 3 }\nMap(0) {} Map(1) { 'a' => 1 } Set(1) { 1 }\nZ { m: Map(0) {}, w: Map(1) { 'k' => 2 } }\n"
    );
}

#[test]
fn fldrest_uninitialised_function_and_any_fields_match_node() {
    let source = r##"class A { j: any; f: () => number; nl: string | null; constructor(c: boolean) { if (c) { this.j = { a: 1 }; this.f = () => 2; this.nl = null; } } }
function main(): void {
  const a = new A(false);
  console.log(a.j, a.j === undefined, typeof a.f, a.nl, a.nl === null);
  console.log(a.j ?? "d", a.nl ?? "n", a.j?.a);
  try { a.f(); } catch (e) { console.log((e as Error).message, e instanceof TypeError); }
  const b = new A(true);
  console.log(b.j, b.f(), b.nl, b.j.a, b.nl === null);
  console.log(a);
  console.log(JSON.stringify(a));
  b.nl = "set"; console.log(b.nl.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldrest_uninitialised_function_and_any_fields_match_node"),
        "undefined true undefined undefined false\nd n undefined\na.f is not a function true\n{ a: 1 } 2 null 1 true\nA { j: undefined, f: undefined, nl: undefined }\n{}\n3\n"
    );
}
