// Remaining Array/Map/Set subclass limits: expected output from node 22.

#[test]
fn collim_contract_species_required_non_numeric_ctor() {
    let source = r##"class Tagged extends Array<number> {
  tag: any;
  constructor(tag: string) { super(); this.tag = tag; console.log("ctor", typeof tag, tag); }
}
function main(): void {
  const t = new Tagged("x"); t.push(1); t.push(2); t.push(3);
  const m = t.map(x => x * 2);
  console.log(m, m instanceof Tagged, m.length);
  const f = t.filter(x => x > 1);
  console.log(f, f instanceof Tagged);
  console.log(t.slice(1), t.concat([9]));
  const sp = t.splice(0, 1);
  console.log(sp, sp instanceof Tagged);
  console.log(Tagged.from([5, 6]), Tagged.of(7, 8));
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_species_required_non_numeric_ctor"),
        "ctor string x\nctor number 3\nTagged(3) [ 2, 4, 6, tag: 3 ] true 3\nctor number 0\nTagged(2) [ 2, 3, tag: 0 ] true\nctor number 2\nctor number 0\nTagged(2) [ 2, 3, tag: 2 ] Tagged(4) [ 1, 2, 3, 9, tag: 0 ]\nctor number 1\nTagged(1) [ 1, tag: 1 ] true\nctor undefined undefined\nctor number 2\nTagged(2) [ 5, 6, tag: undefined ] Tagged(2) [ 7, 8, tag: 2 ]\n"
    );
}

#[test]
fn collim_contract_species_multi_param_optional_and_chains() {
    let source = r##"class Pair extends Array<number> {
  label: any;
  note: any;
  constructor(label: string, note: string) { super(); this.label = label; this.note = note; console.log("pair", typeof label, label, typeof note, note); }
}
class OptTag extends Array<number> {
  tag: string;
  constructor(tag?: string) { super(); this.tag = tag === undefined ? "none" : String(tag); }
}
class Base extends Array<number> {
  who: any;
  constructor(who: string) { super(); this.who = who; }
}
class Leaf extends Base {}
function main(): void {
  const p = new Pair("a", "b"); p.push(1, 2, 3);
  console.log(p.filter(x => x > 1));
  console.log(p.flat());
  const o = new OptTag("t"); o.push(4, 5);
  const om = o.map(x => x + 1);
  console.log(om, om instanceof OptTag);
  const l = new Leaf("leaf"); l.push(9);
  const lf = l.slice(0);
  console.log(lf, lf instanceof Leaf, lf instanceof Base);
  console.log(Leaf.from([1, 2]), Leaf.of(3));
  console.log(l.flatMap(x => [x, x]));
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_species_multi_param_optional_and_chains"),
        "pair string a string b\npair number 0 undefined undefined\nPair(2) [ 2, 3, label: 0, note: undefined ]\npair number 0 undefined undefined\nPair(3) [ 1, 2, 3, label: 0, note: undefined ]\nOptTag(2) [ 5, 6, tag: '2' ] true\nLeaf(1) [ 9, who: 1 ] true true\nLeaf(2) [ 1, 2, who: undefined ] Leaf(1) [ 3, who: 1 ]\nLeaf(2) [ 9, 9, who: 0 ]\n"
    );
}

#[test]
fn collim_contract_native_view_of_subclass_instance() {
    let source = r##"class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
class Reg extends Map<string, number> { n = 1; }
class Bag extends Set<number> { }
function main(): void {
  const s = new Stack(); s.push(1); s.push(2);
  const a: number[] = s;
  console.log(a, a instanceof Stack, a.length, a.map(x => x + 1));
  const r = new Reg(); r.set("a", 1);
  const m: Map<string, number> = r;
  console.log(m, m instanceof Reg, m.get("a"), m.size);
  const b = new Bag(); b.add(3);
  const st: Set<number> = b;
  console.log(st, st instanceof Bag, st.has(3));
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_native_view_of_subclass_instance"),
        "Stack(2) [ 1, 2 ] true 2 Stack(2) [ 2, 3 ]\nReg(1) [Map] { 'a' => 1, n: 1 } true 1 1\nBag(1) [Set] { 3 } true true\n"
    );
}

#[test]
fn collim_contract_native_view_optional_chain() {
    let source = r##"class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
class Reg extends Map<string, number> { n = 1; }
function main(): void {
  const s = new Stack(); s.push(1, 2);
  const arr: (number[] | null) = s;
  console.log(arr?.length, arr?.[1], arr?.map(x => x * 2));
  const r = new Reg(); r.set("a", 1);
  const m: Map<string, number> | undefined = r;
  console.log(m?.size, m?.get("a"), m?.has("z"));
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_native_view_optional_chain"),
        "2 2 Stack(2) [ 2, 4 ]\n1 1 false\n"
    );
}

#[test]
fn collim_contract_union_with_map_and_set_basic() {
    let source = r##"function f(x: boolean): Map<string, number> | string {
  if (x) { const m = new Map<string, number>(); m.set("a", 1); return m; }
  return "no";
}
function g(x: boolean): Set<number> | number {
  return x ? new Set([1, 2]) : 5;
}
function h(x: Map<string, number> | undefined): Map<string, number> {
  return x ?? new Map<string, number>([["z", 26]]);
}
function main(): void {
  const a = f(true), b = f(false);
  console.log(a, b);
  if (typeof a === "string") console.log("str", a); else console.log("map", a.size, a.get("a"));
  if (a instanceof Map) console.log("is map", a.size);
  if (typeof b === "string") console.log("str", b.length);
  console.log(g(true), g(false));
  const s = g(true);
  if (s instanceof Set) console.log(s.has(1), s.size);
  console.log(h(undefined), h(new Map([["q", 1]])));
  const u: Map<string, number> | string = Math.random() > 2 ? "x" : new Map([["k", 2]]);
  console.log(u, typeof u);
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_union_with_map_and_set_basic"),
        "Map(1) { 'a' => 1 } no\nmap 1 1\nis map 1\nstr 2\nSet(2) { 1, 2 } 5\ntrue 2\nMap(1) { 'z' => 26 } Map(1) { 'q' => 1 }\nMap(1) { 'k' => 2 } object\n"
    );
}

#[test]
fn collim_contract_union_collections_narrowing() {
    let source = r##"class Reg extends Map<string, number> { n = 1; }
function pick(x: number): Map<string, number> | number[] | string {
  if (x === 0) return new Map<string, number>([["a", 1]]);
  if (x === 1) return [1, 2];
  return "s";
}
function opt(x: Set<number> | null): number { return x ? x.size : -1; }
function main(): void {
  for (const i of [0, 1, 2]) {
    const v = pick(i);
    if (v instanceof Map) console.log("map", v.size, v.get("a"), v.has("b"));
    else if (Array.isArray(v)) console.log("arr", v.length, v.join("-"));
    else console.log("str", v.toUpperCase());
    console.log(v, typeof v);
  }
  const s = new Set([1, 2, 3]);
  const w: Set<number> | string = s;
  console.log(w, opt(s), opt(null));
  const r = new Reg(); r.set("k", 1);
  const m: Map<string, number> | undefined = r;
  console.log(m, m instanceof Reg);
  const n = Math.random() > 5 ? "no" : (new Map([[1, "x"]]) as Map<number, string> | string);
  if (typeof n !== "string") { n.set(2, "y"); console.log(n.size, [...n.keys()]); }
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_union_collections_narrowing"),
        "map 1 1 false\nMap(1) { 'a' => 1 } object\narr 2 1-2\n[ 1, 2 ] object\nstr S\ns string\nSet(3) { 1, 2, 3 } 3 -1\nReg(1) [Map] { 'k' => 1, n: 1 } true\n2 [ 1, 2 ]\n"
    );
}

#[test]
fn collim_contract_object_values_entries_precise() {
    let source = r##"class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
class Counted extends Array<number> { hits = 3; }
class Labeled extends Array<number> { label = "L"; }
function main(): void {
  const s = new Stack(); s.push(1); s.push(2);
  const v = Object.values(s);
  const sum: number = v.reduce((a, b) => a + b, 0);
  console.log(v, sum, v.map(x => x * 2));
  const e = Object.entries(s);
  console.log(e, e.map(([k, x]) => k + x));
  const c = new Counted(); c.push(5);
  const cv = Object.values(c);
  console.log(cv, cv.map(x => x + 1), Object.entries(c));
  const l = new Labeled(); l.push(7);
  const lv = Object.values(l);
  console.log(lv, lv.map(x => x + 1), lv.length, Object.entries(l).map(([k, x]) => k + ":" + x));
  const num: number[] = Object.values(c);
  console.log(num);
  more();
}
function more(): void {
  const c = new Counted(); c.push(5); c.push(6);
  const t: [string, number][] = Object.entries(c);
  const total = t.reduce((a, [, n]) => a + n, 0);
  const lbl = new Labeled(); lbl.push(1);
  const lv = Object.values(lbl);
  const strs = lv.filter(x => typeof x === "string");
  console.log(total, t.length, strs, lv.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_object_values_entries_precise"),
        "[ 1, 2 ] 3 [ 2, 4 ]\n[ [ '0', 1 ], [ '1', 2 ] ] [ '01', '12' ]\n[ 5, 3 ] [ 6, 4 ] [ [ '0', 5 ], [ 'hits', 3 ] ]\n[ 7, 'L' ] [ 8, 'L1' ] 2 [ '0:7', 'label:L' ]\n[ 5, 3 ]\n14 3 [ 'L' ] 2\n"
    );
}

#[test]
fn collim_contract_object_values_entries_mixed_own_fields() {
    let source = r##"class Stack extends Array<number> { label = "L"; }
function main(): void {
  const s = new Stack(); s.push(1); s.push(2);
  const v = Object.values(s);
  console.log(v);
  const e = Object.entries(s);
  console.log(e);
  const x: number = v[0] as number;
  console.log(Object.keys(s), x);
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_object_values_entries_mixed_own_fields"),
        "[ 1, 2, 'L' ]\n[ [ '0', 1 ], [ '1', 2 ], [ 'label', 'L' ] ]\n[ '0', '1', 'label' ] 1\n"
    );
}

#[test]
fn collim_contract_union_array_and_string() {
    let source = r##"function f(x: boolean): number[] | string {
  if (x) { return [1, 2]; }
  return "no";
}
function main(): void {
  const a = f(true);
  console.log(a, f(false));
  if (typeof a === "string") console.log("str", a); else console.log("arr", a.length, a.indexOf(2), a.map(x => x + 1));
  if (Array.isArray(a)) console.log("is arr", a.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "collim_contract_union_array_and_string"),
        "[ 1, 2 ] no\narr 2 1 [ 2, 3 ]\nis arr 2\n"
    );
}
