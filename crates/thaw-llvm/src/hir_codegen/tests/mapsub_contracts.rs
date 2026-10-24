// `class X extends Map<K, V>` / `extends Set<T>`: expected output from node 22.

#[test]
fn mapsub_contract_map_subclass_basic() {
    let source = r#"class Counter extends Map<string, number> {
  inc(k: string): void { this.set(k, (this.get(k) ?? 0) + 1); }
}
function main(): void {
  const c = new Counter(); c.inc("a"); c.inc("a"); c.inc("b");
  console.log(c.get("a"), c.size, c instanceof Map, c instanceof Counter);
  console.log(c);
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_basic"),
        "2 2 true true\nCounter(2) [Map] { 'a' => 2, 'b' => 1 }\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_override_super() {
    let source = r#"class LoggedMap extends Map<string, number> {
  log: string[] = [];
  set(k: string, v: number): this { this.log.push("set:" + k); return super.set(k, v * 2); }
  get(k: string): number | undefined { this.log.push("get:" + k); return super.get(k); }
}
function main(): void {
  const m = new LoggedMap(); m.set("y", 5);
  console.log(m.get("y"), m.size, m.log.join(","));
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_override_super"),
        "10 1 set:y,get:y\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_iteration_spread_coercion() {
    let source = r#"class Reg extends Map<string, number> {}
function total(m: Map<string, number>): number { let t = 0; for (const [, v] of m) t += v; m.set("z", 100); return t; }
function main(): void {
  const r = new Reg(); r.set("a", 1); r.set("b", 2);
  for (const [k, v] of r) console.log(k, v);
  console.log([...r], [...r.keys()], Array.from(r.values()));
  console.log(total(r), r.size, r.has("z"));
  console.log(new Map([...r]).size, structuredClone(r) instanceof Reg);
  console.log(JSON.stringify(r), Object.keys(r).length);
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_iteration_spread_coercion"),
        "a 1\nb 2\n[ [ 'a', 1 ], [ 'b', 2 ] ] [ 'a', 'b' ] [ 1, 2 ]\n3 3 true\n3 false\n{} 0\n"
    );
}

#[test]
fn mapsub_contract_set_subclass() {
    let source = r#"class Tags extends Set<string> {
  addAll(xs: string[]): this { for (const x of xs) this.add(x); return this; }
}
function main(): void {
  const t = new Tags(); t.addAll(["a", "b", "a"]);
  console.log(t.size, t.has("a"), t instanceof Set, Array.isArray(t), [...t].join("|"));
  console.log(t);
  console.log(t.union(new Set(["c"])).size);
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_set_subclass"),
        "2 true true false a|b\nTags(2) [Set] { 'a', 'b' }\n3\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_chain_generic() {
    let source = r#"class Base<K, V> extends Map<K, V> { count(): number { return this.size; } }
class Leaf extends Base<string, number> {}
class Q<T> extends Set<T> { first(): T | undefined { for (const x of this) return x; return undefined; } }
function main(): void {
  const l = new Leaf(); l.set("a", 1);
  const q = new Q<number>(); q.add(7);
  console.log(l.count(), l instanceof Map, l instanceof Leaf, q.first(), q instanceof Set);
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_chain_generic"),
        "1 true true 7 true\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_ctor_chain_and_instanceof() {
    let source = r#"class Reg extends Map<string, number> {
  constructor(entries: [string, number][]) { super(entries); }
  total(): number { let t = 0; for (const [, v] of this) t += v; return t; }
}
class Sub extends Reg {
  constructor() { super([["q", 5]]); }
  doubled(): number { return this.total() * 2; }
}
class Tags extends Set<string> {
  add(v: string): this { return super.add(v.toUpperCase()); }
}
function kind(x: Reg | number): string { return x instanceof Map ? "map" : "num"; }
function main(): void {
  const r = new Reg([["a", 1], ["b", 2]]);
  console.log(r.size, r.total(), r.get("a"), r.has("b"));
  const s = new Sub();
  console.log(s.size, s.doubled(), s instanceof Reg, s instanceof Map, s instanceof Sub);
  console.log(new Reg([["z", 9]]).set("y", 1).size);
  const t = new Tags(); t.add("x").add("y");
  console.log(t.size, t.has("X"), t.has("x"), [...t].join(""));
  console.log(kind(r), kind(3));
  const plain = new Map<string, number>();
  console.log(plain instanceof Reg, plain instanceof Map);
  r.delete("a"); r.clear();
  console.log(r.size);
  const keys: string[] = [];
  r.set("k", 1);
  r.forEach((v, k) => keys.push(k + v));
  console.log(keys.join(","));
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_ctor_chain_and_instanceof"),
        "2 3 1 true\n1 10 true true true\n2\n2 true false XY\nmap num\nfalse true\n0\nk1\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_console() {
    let source = r#"class Counter extends Map<string, number> {}
class Sub extends Counter {}
class Q<T> extends Set<T> {}
function main(): void {
  const e = new Counter();
  console.log(e);
  const s = new Sub(); s.set("k", 1);
  console.log(s, [s], { s });
  const q = new Q<number>(); q.add(7);
  console.log(q);
  console.log({ a: { b: { c: { d: s } } } });
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_console"),
        "Counter(0) [Map] {}\nSub(1) [Map] { 'k' => 1 } [ Sub(1) [Map] { 'k' => 1 } ] { s: Sub(1) [Map] { 'k' => 1 } }\nQ(1) [Set] { 7 }\n{ a: { b: { c: [Object] } } }\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_own_field() {
    let source = r#"class Cache extends Map<string, number> {
  hits: number = 0;
  lookup(k: string): number {
    const v = this.get(k);
    if (v === undefined) { return -1; }
    this.hits++;
    return v;
  }
}
function main(): void {
  const c = new Cache();
  c.set("a", 1);
  console.log(c.lookup("a"), c.lookup("b"), c.hits, c.size);
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_own_field"),
        "1 -1 1 1\n"
    );
}

#[test]
fn mapsub_contract_map_subclass_conversions_and_aliasing() {
    let source = r#"class Reg extends Map<string, number> {}
class Tags extends Set<string> {}
function size(m: Map<string, number> | undefined): number { return m === undefined ? -1 : m.size; }
function main(): void {
  const r = new Reg(); r.set("a", 1); r.set("b", 2);
  const t = new Tags(); t.add("x");
  console.log(Array.from(r).length, Array.from(t).join(","), size(r), size(undefined));
  console.log(new Set([...t]).size, new Set<string>([...t, "y"]).size);
  const alias: Map<string, number> = r;
  alias.set("c", 3);
  console.log(r.size, [...r.entries()].length, r.get("c"));
  const list: Reg[] = [r, new Reg()];
  console.log(list.map((m) => m.size).join(","));
  const holder = { m: r };
  console.log(holder.m.size, holder.m.has("a"));
  console.log(Object.keys(r), JSON.stringify({ r }));
}
"#;
    assert_eq!(
        compile_and_run(source, "mapsub_contract_map_subclass_conversions_and_aliasing"),
        "2 x 2 -1\n1 2\n3 3 3\n3,0\n3 true\n[] {\"r\":{}}\n"
    );
}
