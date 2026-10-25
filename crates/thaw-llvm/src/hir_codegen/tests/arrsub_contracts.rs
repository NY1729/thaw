// `extends Map/Set` stage 2 and `extends Array`: expected output from node 22.

#[test]
fn arrsub_contract_map_tag_and_own_fields() {
    let source = r#"class Counter extends Map<string, number> {
  total = 0;
  inc(k: string): void { this.set(k, (this.get(k) ?? 0) + 1); this.total++; }
}
class Tags extends Set<string> { label = "x"; }
class Reg extends Map<string, number> {}
class Bag extends Set<number> {}
function main(): void {
  const c = new Counter(); c.inc("a"); c.inc("a");
  console.log(String(c), Object.prototype.toString.call(c), `${c}`);
  console.log(c);
  const t = new Tags(); t.add("q");
  console.log(String(t), Object.prototype.toString.call(t));
  console.log(t);
  console.log(Object.keys(c), JSON.stringify(c), JSON.stringify(t));
  const r = new Reg([["a", 1], ["b", 2]]);
  console.log(r.size, r);
  const b = new Bag([1, 2, 2, 3]);
  console.log(b.size, b);
  console.log(c.total);
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_map_tag_and_own_fields"),
        "[object Map] [object Map] [object Map]\nCounter(1) [Map] { 'a' => 2, total: 2 }\n[object Set] [object Set]\nTags(1) [Set] { 'q', label: 'x' }\n[ 'total' ] {\"total\":2} {\"label\":\"x\"}\n2 Reg(2) [Map] { 'a' => 1, 'b' => 2 }\n3 Bag(3) [Set] { 1, 2, 3 }\n2\n"
    );
}

#[test]
fn arrsub_contract_map_ctor_entries_and_statics() {
    let source = r#"class Counter extends Map<string, number> {
  static made = 0;
  static make(): Counter { const c = new Counter(); c.set("z", 1); Counter.made++; return c; }
  static count = 5;
}
class Tags extends Set<string> { static of2(a: string, b: string): Tags { const t = new Tags(); t.add(a); t.add(b); return t; } }
class Wide extends Counter { static make2(): Wide { return new Wide([["q", 2]]); } }
function main(): void {
  console.log(Counter.make().size, Counter.count, Tags.of2("a", "b").size, Counter.made);
  console.log(Wide.make2().size, Wide.count, Wide.make().size);
  const g = Map.groupBy([1, 2, 3], (x: number) => x % 2);
  console.log(g.size);
  const e = new Counter([["k", 1]]);
  console.log(e.size, e instanceof Map);
  const s = new Tags(["a", "b", "a"]);
  console.log(s.size, [...s].join(""));
  const none = new Tags();
  console.log(none.size, none);
  const e2 = new Counter(undefined);
  console.log(e2.size);
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_map_ctor_entries_and_statics"),
        "1 5 2 1\n1 5 1\n2\n1 true\n2 ab\n0 Tags(0) [Set] {}\n0\n"
    );
}

#[test]
fn arrsub_contract_array_basic() {
    let source = r#"class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
function main(): void {
  const s = new Stack(); s.push(1); s.push(2);
  console.log(s.length, s.top(), s instanceof Array, s instanceof Stack, Array.isArray(s));
  console.log(s);
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_basic"),
        "2 2 true true true\nStack(2) [ 1, 2 ]\n"
    );
}

#[test]
fn arrsub_contract_array_override_super_length() {
    let source = r#"class Logged extends Array<number> {
  pushed = 0;
  push(...xs: number[]): number { this.pushed += xs.length; return super.push(...xs); }
  last(): number | undefined { return this.at(-1); }
}
function main(): void {
  const a = new Logged(); a.push(1, 2); a.push(3);
  console.log(a.length, a.pushed, a.last(), a.indexOf(2), a.join("-"));
  a.length = 1; console.log(a.length, a[0], a[1]);
  a[1] = 9; console.log(a.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_override_super_length"),
        "3 3 3 1 1-2-3\n1 1 undefined\n2\n"
    );
}

#[test]
fn arrsub_contract_array_iteration_coercion() {
    let source = r#"class Bag extends Array<string> {}
function addOne(xs: string[]): void { xs.push("z"); }
function main(): void {
  const b = new Bag(); b.push("a", "b");
  for (const x of b) console.log(x);
  const [h, ...rest] = b; console.log(h, rest);
  console.log([...b, "c"], b.concat(["q"]).length, Array.from(b).length);
  addOne(b); console.log(b.length, `${b}`, String(b));
  console.log(JSON.stringify(b), Object.keys(b));
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_iteration_coercion"),
        "a\nb\na [ 'b' ]\n[ 'a', 'b', 'c' ] 3 2\n3 a,b,z a,b,z\n[\"a\",\"b\",\"z\"] [ '0', '1', '2' ]\n"
    );
}

#[test]
fn arrsub_contract_array_generic_isarray_items() {
    let source = r#"class Q<T> extends Array<T> { second(): T | undefined { return this[1]; } }
function kind(x: number[] | string): string { return Array.isArray(x) ? "arr" : "str"; }
class Num extends Array<number> {}
function main(): void {
  const q = new Q<string>(); q.push("a", "b");
  const n = new Num(); n.push(1);
  console.log(q.second(), kind(n), kind("x"), n instanceof Array);
  const m = new Num(4, 5, 6);
  console.log(m.length, m, Object.prototype.toString.call(m));
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_generic_isarray_items"),
        "b arr str true\n3 Num(3) [ 4, 5, 6 ] [object Array]\n"
    );
}

#[test]
fn arrsub_contract_array_ctor_rest_methods() {
    let source = r#"class Stack extends Array<number> {
  label: string;
  constructor(label: string, ...items: number[]) { super(...items); this.label = label; }
  sum(): number { return this.reduce((a, b) => a + b, 0); }
  peek(): number | undefined { return this[this.length - 1]; }
  bump(i: number): void { this[i]++; this[i] += 10; }
  clear(): void { this.length = 0; }
}
class Wide extends Stack { extra = 1; double(): number[] { return Array.from(this.map((x) => x * 2)); } }
function total(xs: number[]): number { let t = 0; for (const x of xs) t += x; return t; }
function sumAll(...xs: number[]): number { return xs.length; }
function main(): void {
  const s = new Stack("s", 1, 2, 3);
  console.log(s.sum(), s.peek(), s.label, s.length);
  s.bump(0); console.log(s[0], s);
  console.log(total(s), sumAll(...s), Math.max(...s));
  console.log(s.includes(2), s.indexOf(3), s.slice(1).length, s.filter((x) => x > 5).length);
  console.log(s.join(","), s.reverse().join(","), s.sort((a, b) => a - b).join(","));
  const w = new Wide("w", 4, 5);
  console.log(w.double(), w.extra, w instanceof Stack, w instanceof Wide, w instanceof Array);
  console.log(w);
  s.clear(); console.log(s.length, s);
  console.log(new Stack("n").length);
  s.push(7, 8);
  console.log(s.pop(), s.shift(), s.length);
  s.unshift(1, 2); s.splice(1, 1); console.log(s.join("|"));
  s.forEach((x, i) => console.log(i, x));
  for (const [i, x] of s.entries()) console.log(i, x);
  console.log(Array.from(s).length);
  console.log(s.at(-1), s.find((x) => x === 1), s.some((x) => x > 1), s.every((x) => x > 0));
  console.log(Array.isArray(w), typeof s, Object.prototype.toString.call(s), String(s));
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_ctor_rest_methods"),
        "6 3 s 3\n12 Stack(3) [ 12, 2, 3, label: 's' ]\n17 3 12\ntrue 2 2 1\n12,2,3 3,2,12 2,3,12\n[ 8, 10 ] 1 true true true\nWide(2) [ 4, 5, label: 'w', extra: 1 ]\n0 Stack(0) [ label: 's' ]\n0\n8 7 0\n1\n0 1\n0 1\n1\n1 1 false true\ntrue object [object Array] 1\n"
    );
}

#[test]
fn arrsub_contract_array_statics_and_aliasing() {
    let source = r#"class Vec extends Array<number> {
  static zero(n: number): Vec { const v = new Vec(); for (let i = 0; i < n; i++) v.push(0); return v; }
  static count = 3;
  add(o: Vec): Vec { const r = new Vec(); for (let i = 0; i < this.length; i++) r.push(this[i] + o[i]); return r; }
  get first(): number { return this[0]; }
  self(): this { return this; }
}
function mk(): Vec { const v = new Vec(); v.push(5); return v; }
function main(): void {
  const a = Vec.zero(2); a[0] = 1; a[1] = 2;
  const b = a.add(a);
  console.log(b, b.first, Vec.count, a.self().length);
  console.log(new Vec().push(9));
  console.log(mk().length, mk()[0]);
  const list: Vec[] = [a, b];
  console.log(list.length, list[1].length, list.map((v) => v.length));
  const arr: number[] = a;
  arr.push(99);
  console.log(a.length, a);
  const o = { v: a };
  console.log(o.v.length);
  const j = structuredClone(a);
  console.log(j instanceof Vec, j);
  const empty = new Vec();
  console.log(empty, empty.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_statics_and_aliasing"),
        "Vec(2) [ 2, 4 ] 2 3 2\n1\n1 5\n2 2 [ 2, 2 ]\n3 Vec(3) [ 1, 2, 99 ]\n3\nfalse [ 1, 2, 99 ]\nVec(0) [] 0\n"
    );
}

#[test]
fn arrsub_contract_array_nested_promise_strings() {
    let source = r#"class Matrix extends Array<number[]> {
  rows(): number { return this.length; }
  cell(r: number, c: number): number { return this[r][c]; }
}
class Words extends Array<string> {
  shout(): string[] { const out: string[] = []; for (const w of this) out.push(w.toUpperCase()); return out; }
  copy(): string[] { return [...this]; }
  addAll(xs: string[]): this { this.push(...xs); return this; }
  firstTwo(): string { const [a, b] = this; return a + b; }
}
async function later(n: number): Promise<number> { return n * 2; }
class Jobs extends Array<Promise<number>> {}
async function run(): Promise<void> {
  const j = new Jobs(); j.push(later(1)); j.push(later(2));
  console.log(await Promise.all(j));
}
function main(): void {
  const m = new Matrix(); m.push([1, 2], [3, 4]);
  console.log(m.rows(), m.cell(1, 0), m);
  const w = new Words().addAll(["b", "a", "c"]);
  console.log(w.shout(), w.copy(), w.firstTwo(), "" + w, `${w}`, w.toString());
  console.log(w.sort(), w.keys().next().value, [...w.values()]);
  console.log(JSON.stringify({ w }), JSON.stringify([w]));
  console.log(w.length === 3 ? "three" : "other", w.indexOf("a"), w.lastIndexOf("c"));
  const copy = w.slice();
  console.log(copy.length);
  run();
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_nested_promise_strings"),
        "2 3 Matrix(2) [ [ 1, 2 ], [ 3, 4 ] ]\n[ 'B', 'A', 'C' ] [ 'b', 'a', 'c' ] ba b,a,c b,a,c b,a,c\nWords(3) [ 'a', 'b', 'c' ] 0 [ 'a', 'b', 'c' ]\n{\"w\":[\"a\",\"b\",\"c\"]} [[\"a\",\"b\",\"c\"]]\nthree 0 2\n3\n[ 2, 4 ]\n"
    );
}

#[test]
fn arrsub_contract_array_species_console_json() {
    let source = r#"class Stack extends Array<number> { top(): number { return this[this.length - 1]; } }
class Tagged extends Array<string> { tag = "t"; }
function main(): void {
  const s = new Stack(); s.push(1, 2);
  const t = new Tagged(); t.push("x");
  console.log({ s, t });
  console.log([s, t]);
  const o = { list: s, n: 1 };
  console.log(o);
  console.log(JSON.stringify(o), JSON.stringify(t));
  const e = new Tagged();
  console.log(e);
  console.log(s.filter((x) => x > 1), s.map((x) => x + 1), s.slice(0, 1));
  console.log(s.concat([3]), s.concat(s));
  const arr: (number[] | null) = s;
  console.log(arr?.length);
  console.log(s.flat(), s.flatMap((x) => [x, x]));
  console.log(s.splice(0, 1));
  console.log(Stack.from([4, 5]));
  console.log(Stack.of(6, 7));
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_species_console_json"),
        "{ s: Stack(2) [ 1, 2 ], t: Tagged(1) [ 'x', tag: 't' ] }\n[ Stack(2) [ 1, 2 ], Tagged(1) [ 'x', tag: 't' ] ]\n{ list: Stack(2) [ 1, 2 ], n: 1 }\n{\"list\":[1,2],\"n\":1} [\"x\"]\nTagged(0) [ tag: 't' ]\nStack(1) [ 2 ] Stack(2) [ 2, 3 ] Stack(1) [ 1 ]\nStack(3) [ 1, 2, 3 ] Stack(4) [ 1, 2, 1, 2 ]\n2\nStack(2) [ 1, 2 ] Stack(4) [ 1, 1, 2, 2 ]\nStack(1) [ 1 ]\nStack(2) [ 4, 5 ]\nStack(2) [ 6, 7 ]\n"
    );
}

#[test]
fn arrsub_contract_array_ctor_length_semantics() {
    let source = r#"class Grid extends Array<number> {}
class Names extends Array<string> {}
class Sized extends Array<number> {
  constructor(n: number) { super(n); }
}
function main(): void {
  const g = new Grid(3);
  console.log(g.length, g);
  const g2 = new Grid(1, 2);
  console.log(g2.length, g2);
  const g3 = new Grid(7, 8, 9);
  console.log(g3);
  const n = new Names("a");
  console.log(n.length, n);
  const z = new Grid();
  console.log(z.length);
  const s = new Sized(4);
  console.log(s.length, s);
  s.fill(2);
  console.log(s.join(""));
}
"#;
    assert_eq!(
        compile_and_run(source, "arrsub_contract_array_ctor_length_semantics"),
        "3 Grid(3) [ <3 empty items> ]\n2 Grid(2) [ 1, 2 ]\nGrid(3) [ 7, 8, 9 ]\n1 Names(1) [ 'a' ]\n0\n4 Sized(4) [ <4 empty items> ]\n2222\n"
    );
}
