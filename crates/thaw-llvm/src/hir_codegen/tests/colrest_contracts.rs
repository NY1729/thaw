// `extends Array/Map/Set` remaining gaps: expected output from node 22.

#[test]
fn colrest_contract_species_element_type_changes() {
    let source = r##"class Stack extends Array<number> {
  top(): number { return this[this.length - 1]; }
}
class GStack<T> extends Array<T> {
  last(): T { return this[this.length - 1]; }
}
class Sized extends Array<number> {
  tag: string;
  constructor(tag: string) { super(); this.tag = tag; }
}
function main(): void {
  const s = new Stack(); s.push(1); s.push(2); s.push(3);
  const m = s.map(x => x + "!");
  console.log(m, m instanceof Stack, m.length, Array.isArray(m));
  const m2 = s.map(x => x * 2);
  console.log(m2, m2 instanceof Stack, m2.top());
  const f = s.filter(x => x > 1);
  console.log(f, f instanceof Stack);
  console.log(s.slice(1), s.concat([9]), s.slice(1) instanceof Stack);
  const sp = s.splice(0, 1);
  console.log(sp, sp instanceof Stack, s);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_species_element_type_changes"),
        "Stack(3) [ '1!', '2!', '3!' ] true 3 true\nStack(3) [ 2, 4, 6 ] true 6\nStack(2) [ 2, 3 ] true\nStack(2) [ 2, 3 ] Stack(4) [ 1, 2, 3, 9 ] true\nStack(1) [ 1 ] true Stack(2) [ 2, 3 ]\n"
    );
}

#[test]
fn colrest_contract_species_generic_subclass_and_flat() {
    let source = r##"class Stack extends Array<number> {
  top(): number { return this[this.length - 1]; }
}
class GStack<T> extends Array<T> {
  last(): T { return this[this.length - 1]; }
}
class Sized extends Array<number> {
  tag: string;
  constructor(tag: string) { super(); this.tag = tag; }
}
function main(): void {
  const g = new GStack<number>(); g.push(1); g.push(2);
  const gm = g.map(x => "v" + x);
  console.log(gm, gm.length);
  const gf = g.filter(x => x > 1);
  console.log(gf, gf.last());
  const fl = new GStack<number[]>(); fl.push([1, 2]); fl.push([3]);
  console.log(fl.flat(), fl.flatMap(x => x));
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_species_generic_subclass_and_flat"),
        "GStack(2) [ 'v1', 'v2' ] 2\nGStack(1) [ 2 ] 2\nGStack(3) [ 1, 2, 3 ] GStack(3) [ 1, 2, 3 ]\n"
    );
}

#[test]
fn colrest_contract_species_numeric_constructor_chains_and_statics() {
    let source = r##"class Sized extends Array<number> {
  tag: number;
  constructor(tag: number) { super(); this.tag = tag; }
}
class Opt extends Array<number> {
  tag: number;
  extra: string;
  constructor(tag?: number, extra?: string) { super(); this.tag = tag ?? -1; this.extra = extra ?? "none"; }
}
class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
class Wide extends Stack {}
class Deep extends Wide { extra = 1; }
function main(): void {
  const z = new Sized(9); z.push(5); z.push(6);
  const zm = z.map(x => x + 1);
  console.log(zm, zm instanceof Sized, zm.length);
  const zf = z.filter(x => x > 5);
  console.log(zf, zf instanceof Sized, z.slice(1), z.concat([1]));
  const o = new Opt(3); o.push(1);
  console.log(o.map(x => "s" + x), o.filter(x => true));
  const w = new Wide(); w.push(4); w.push(5);
  console.log(w.map(x => x * 3), w.map(x => x * 3) instanceof Wide, w.map(x => x * 3) instanceof Stack);
  const d = new Deep(); d.push(1);
  console.log(d.map(x => String(x)), d.filter(x => true));
  const st = Stack.from([1, 2, 3]); const so = Stack.of(7, 8);
  console.log(st, so, st instanceof Stack, so.top());
  const mm: string[] = w.map(x => "q" + x);
  console.log(mm.length, Array.isArray(mm));
  console.log(w.map(x => "a" + x).map(x => x.length), w.filter(x => x > 4).map(x => x * 2) instanceof Wide);
  console.log(Stack.from([3, 4]).map(x => x + 1), w.reverse().map(x => x));
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_species_numeric_constructor_chains_and_statics"),
        "Sized(2) [ 6, 7, tag: 2 ] true 2\nSized(1) [ 6, tag: 0 ] true Sized(1) [ 6, tag: 1 ] Sized(3) [ 5, 6, 1, tag: 0 ]\nOpt(1) [ 's1', tag: 1, extra: 'none' ] Opt(1) [ 1, tag: 0, extra: 'none' ]\nWide(2) [ 12, 15 ] true true\nDeep(1) [ '1', extra: 1 ] Deep(1) [ 1, extra: 1 ]\nStack(3) [ 1, 2, 3 ] Stack(2) [ 7, 8 ] true 8\n2 true\nWide(2) [ 2, 2 ] true\nStack(2) [ 4, 5 ] Wide(2) [ 5, 4 ]\n"
    );
}

#[test]
fn colrest_contract_array_subclass_own_keys_values_entries() {
    let source = r##"class Rows extends Array<number> {
  label = "rows";
  note: string;
  constructor() { super(); this.note = "n"; }
  sum(): number { let t = 0; for (const x of this) t += x; return t; }
}
class Plain extends Array<number> {}
function main(): void {
  const r = new Rows(); r.push(3); r.push(4);
  console.log(Object.keys(r), Object.values(r), Object.entries(r));
  console.log(Object.getOwnPropertyNames(r), Reflect.ownKeys(r));
  for (const k in r) console.log(k);
  console.log(JSON.stringify(r), r.sum());
  const p = new Plain(); p.push(1); p.push(2);
  console.log(Object.keys(p), Object.values(p), Object.entries(p), Object.getOwnPropertyNames(p));
  for (const k in p) console.log("p", k);
  for (const [k, v] of Object.entries(r)) console.log(k, v);
  console.log(Object.values(r).length, Object.keys(r).length);
  console.log(r instanceof Object, p instanceof Object, Object.prototype.hasOwnProperty.call(r, "label"));
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_array_subclass_own_keys_values_entries"),
        "[ '0', '1', 'label', 'note' ] [ 3, 4, 'rows', 'n' ] [ [ '0', 3 ], [ '1', 4 ], [ 'label', 'rows' ], [ 'note', 'n' ] ]\n[ '0', '1', 'length', 'label', 'note' ] [ '0', '1', 'length', 'label', 'note' ]\n0\n1\nlabel\nnote\n[3,4] 7\n[ '0', '1' ] [ 1, 2 ] [ [ '0', 1 ], [ '1', 2 ] ] [ '0', '1', 'length' ]\np 0\np 1\n0 3\n1 4\nlabel rows\nnote n\n4 4\ntrue true true\n"
    );
}

#[test]
fn colrest_contract_array_subclass_keys_for_in_json() {
    let source = r##"class Stack extends Array<number> {
  label = "L";
  top(): number { return this[this.length - 1]; }
}
function main(): void {
  const s = new Stack(); s.push(1); s.push(2);
  s.label = "hi";
  console.log(Object.keys(s), Object.values(s), Object.entries(s), Object.getOwnPropertyNames(s));
  console.log(JSON.stringify(s));
  for (const k in s) console.log("k", k);
  console.log(Object.keys(s).length);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_array_subclass_keys_for_in_json"),
        "[ '0', '1', 'label' ] [ 1, 2, 'hi' ] [ [ '0', 1 ], [ '1', 2 ], [ 'label', 'hi' ] ] [ '0', '1', 'length', 'label' ]\n[1,2]\nk 0\nk 1\nk label\n3\n"
    );
}

#[test]
fn colrest_contract_inherited_statics_through_subclasses() {
    let source = r##"class Counter extends Map<string, number> { static made = 0; static mk(): Counter { Counter.made++; return new Counter(); } }
class Wide extends Counter { static other = 2; }
class Deeper extends Wide {}
class Arr extends Array<string> { static tag = "A"; static build(): Arr { return Arr.of("a"); } }
class Arr2 extends Arr {}
function main(): void {
  console.log(Deeper.made, Deeper.other, Deeper.mk().size, Counter.made);
  const g = Deeper.groupBy(["a", "bb", "cc"], (s: string) => s.length);
  console.log(g, g instanceof Map, g instanceof Deeper);
  console.log(Arr2.tag, Arr2.build(), Arr2.isArray([1]), Arr2.isArray(Arr2.of("z")), Arr2.of("q", "r"));
  const f = Arr2.from(["u", "v"]);
  console.log(f, f instanceof Arr2, f instanceof Arr, f.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_inherited_statics_through_subclasses"),
        "0 2 0 1\nMap(2) { 1 => [ 'a' ], 2 => [ 'bb', 'cc' ] } true false\nA Arr(1) [ 'a' ] true true Arr2(2) [ 'q', 'r' ]\nArr2(2) [ 'u', 'v' ] true true 2\n"
    );
}

#[test]
fn colrest_contract_inherited_is_array_and_group_by() {
    let source = r##"class Counter extends Array<number> {
  static made = 0;
  static mk(): Counter { Counter.made++; return new Counter(); }
}
class Sub extends Counter { }
class M2 extends Map<number, number[]> { }
class M3 extends M2 { static k = 3; }
function main(): void {
  console.log(Array.isArray(new Counter()), Counter.isArray([1]), Counter.isArray(new Counter()));
  console.log(Sub.made, Sub.mk() instanceof Counter, Sub.mk() instanceof Sub);
  const mg = M2.groupBy([1, 2, 3], (x: number) => x % 2);
  console.log(mg.size, mg, M3.groupBy([1, 2, 3, 4], (x: number) => x % 3), M3.k, mg instanceof M2);
  const f = Sub.from([1, 2, 3]);
  console.log(f, f instanceof Sub, Sub.of(7, 8) instanceof Sub);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_inherited_is_array_and_group_by"),
        "true true true\n0 true false\n2 Map(2) { 1 => [ 1, 3 ], 0 => [ 2 ] } Map(3) { 1 => [ 1, 4 ], 2 => [ 2 ], 0 => [ 3 ] } 3 false\nSub(3) [ 1, 2, 3 ] true true\n"
    );
}

#[test]
fn colrest_contract_map_set_iterable_calls_overridden_set_add() {
    let source = r##"const LOG: string[] = [];
const SEEN: number[] = [];
class Reg extends Map<string, number> {
  
  set(k: string, v: number): this { LOG.push(k); return super.set(k, v * 2); }
}
class Bag extends Set<number> {
  
  add(v: number): this { SEEN.push(v); return super.add(v + 100); }
}
class Plain extends Map<string, number> {}
class PlainS extends Set<number> {}
function main(): void {
  const r = new Reg([["a", 1], ["b", 2]]);
  console.log(r, LOG);
  const b = new Bag([1, 2, 2]);
  console.log(b, SEEN);
  const other = new Map<string, number>([["x", 1]]);
  const p = new Plain(other);
  console.log(p, p.size, p instanceof Plain);
  const ps = new PlainS(new Set([1, 2]));
  console.log(ps, ps.size);
  const p2 = new Plain(new Plain(other));
  console.log(p2);
  const ps2 = new PlainS([3, 4]);
  console.log(ps2);
  const pl = new Plain(r);
  console.log(pl);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_map_set_iterable_calls_overridden_set_add"),
        "Reg(2) [Map] { 'a' => 2, 'b' => 4 } [ 'a', 'b' ]\nBag(2) [Set] { 101, 102 } [ 1, 2, 2 ]\nPlain(1) [Map] { 'x' => 1 } 1 true\nPlainS(2) [Set] { 1, 2 } 2\nPlain(1) [Map] { 'x' => 1 }\nPlainS(2) [Set] { 3, 4 }\nPlain(2) [Map] { 'a' => 2, 'b' => 4 }\n"
    );
}

#[test]
fn colrest_contract_map_set_super_override_through_user_base() {
    let source = r##"const LOG: string[] = [];
class Counter extends Map<string, number> {
  inc(k: string): void { this.set(k, (this.get(k) ?? 0) + 1); }
}
class Logged extends Counter {
  set(k: string, v: number): this { LOG.push(k + "=" + v); return super.set(k, v + 1); }
}
class Explicit extends Map<string, number> {
  constructor(e?: [string, number][]) { super(e); }
  set(k: string, v: number): this { LOG.push("E" + k); return super.set(k, v); }
}
class Tags extends Set<string> {
  add(v: string): this { LOG.push("add " + v); return super.add(v.toUpperCase()); }
}
class SubTags extends Tags {}
function main(): void {
  const l = new Logged([["a", 1], ["b", 2]]);
  console.log(l, LOG);
  const c: Logged = new Logged();
  c.set("z", 10);
  c.inc("z");
  console.log(c, LOG.length);
  const e = new Explicit([["q", 1]]);
  console.log(e, LOG.length);
  const e2 = new Explicit();
  console.log(e2.size);
  const t = new SubTags(["x", "y", "x"]);
  console.log(t, LOG.slice(-3));
  const t2 = new Tags(new Set(["m"]));
  console.log(t2);
  const f = new Logged(new Map([["k", 5]]));
  console.log(f);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_map_set_super_override_through_user_base"),
        "Logged(2) [Map] { 'a' => 2, 'b' => 3 } [ 'a=1', 'b=2' ]\nLogged(1) [Map] { 'z' => 13 } 4\nExplicit(1) [Map] { 'q' => 1 } 5\n0\nSubTags(2) [Set] { 'X', 'Y' } [ 'add x', 'add y', 'add x' ]\nTags(1) [Set] { 'M' }\nLogged(1) [Map] { 'k' => 6 }\n"
    );
}

#[test]
fn colrest_contract_subclass_identity_with_native_collection() {
    let source = r##"class Stack extends Array<number> {}
class Reg extends Map<string, number> {}
function takes(a: number[]): number { a.push(9); return a.length; }
function main(): void {
  const s = new Stack(); s.push(1);
  const a: number[] = s;
  console.log(a === s, a !== s, s === a);
  console.log(takes(s), s.length, s);
  const r = new Reg();
  const m: Map<string, number> = r;
  console.log(m === r, m !== r);
  const t = new Stack();
  console.log(a === t, s === s);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_subclass_identity_with_native_collection"),
        "true false true\n2 2 Stack(2) [ 1, 9 ]\ntrue false\nfalse true\n"
    );
}

#[test]
fn colrest_contract_instanceof_object_and_generic_class() {
    let source = r##"class Base<K, V> { m: Map<K, V> = new Map<K, V>(); }
class Child extends Base<string, number> {}
function main(): void {
  const m = new Map<string, number>([["a", 1]]); const st = new Set<number>([1]);
  console.log(m instanceof Object, st instanceof Object, [1] instanceof Object);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_instanceof_object_and_generic_class"),
        "true true true\n"
    );
}

#[test]
fn colrest_contract_instanceof_generic_user_class() {
    let source = r##"class Base<K, V> { m: Map<K, V> = new Map<K, V>(); }
class Child extends Base<string, number> {}
function main(): void {
  const l = new Child();
  console.log(l instanceof Base, l instanceof Child);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_instanceof_generic_user_class"),
        "true true\n"
    );
}

#[test]
fn colrest_contract_map_set_destructuring() {
    let source = r##"class Base<K, V> { m: Map<K, V> = new Map<K, V>(); }
class Child extends Base<string, number> {}
function main(): void {
  const m = new Map<string, number>([["a", 1], ["b", 2]]);
  const [first] = m;
  console.log(first);
  const st = new Set<number>([1, 2]);
  const [x, y] = st;
  console.log(x, y);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_map_set_destructuring"),
        "[ 'a', 1 ]\n1 2\n"
    );
}

#[test]
fn colrest_contract_map_set_copy_constructor() {
    let source = r##"class Base<K, V> { m: Map<K, V> = new Map<K, V>(); }
class Child extends Base<string, number> {}
function main(): void {
  const m = new Map<string, number>([["a", 1], ["b", 2]]);
  const st = new Set<number>([1, 2]);
  const c = new Map(m); c.set("z", 3);
  console.log(c, m.size, c.size);
  const d = new Set(st); d.add(7);
  console.log(d, st.size);
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_map_set_copy_constructor"),
        "Map(3) { 'a' => 1, 'b' => 2, 'z' => 3 } 2 3\nSet(3) { 1, 2, 7 } 2\n"
    );
}

#[test]
fn colrest_contract_map_set_truthiness() {
    let source = r##"class Base<K, V> { m: Map<K, V> = new Map<K, V>(); }
class Child extends Base<string, number> {}
function main(): void {
  const m = new Map<string, number>(); const st = new Set<number>();
  if (m) console.log("truthy map");
  if (st) console.log("truthy set");
  if (!m) console.log("bad");
  console.log(m ? "y" : "n", st ? 1 : 2, m && "and");
  const arr: number[] = [];
  if (arr) console.log("arr");
  console.log(!!m, !st, Boolean(m), m || "or");
}
"##;
    assert_eq!(
        compile_and_run(source, "colrest_contract_map_set_truthiness"),
        "truthy map\ntruthy set\ny 1 and\narr\ntrue false true Map(0) {}\n"
    );
}
