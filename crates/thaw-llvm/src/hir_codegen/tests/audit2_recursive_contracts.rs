// Audit2 group B: return-path analysis, Math.max spreads, checked native bigint.
// Expected output recorded from node 22.

#[test]
fn audit2_contract_forever_loops_and_grouped_switch_return_on_all_paths() {
    let source = r##"function h(n: number): string { while (true) { if (n++ > 3) return "w" + n; } }
function f(n: number): string { for (;;) { if (n > 2) return "x"; n++; } }
function g(n: number): string { do { if (n > 2) return "y"; n++; } while (true); }
function s(n: number): string { switch (n) { case 1: case 2: return "12"; default: return "d"; } }
function main(): void { console.log(h(0), f(0), g(0), s(1), s(2), s(3)); }
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_forever_loops_and_grouped_switch"),
        "w5 x y 12 12 d\n"
    );
}

#[test]
fn audit2_contract_math_max_spread_with_extra_arguments_and_sets() {
    let source = r##"function main(): void {
  const a = [1, 2]; const s = new Set([1, 5]);
  console.log(Math.max(...a, 3), Math.min(9, ...a), Math.max(...s), Math.max(...[1, 5], 10));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_math_max_spread"),
        "3 1 5 10\n"
    );
}

#[test]
fn audit2_contract_native_bigint_overflow_throws_instead_of_wrapping() {
    let source = r##"function main(): void {
  console.log(2n ** 62n, 3n * 4n, 1n << 62n);
  try { console.log(2n ** 64n); } catch (e) { console.log((e as Error).name); }
  try { let x = 1n; let i = 1n; while (i < 30n) { x = x * i; i = i + 1n; } console.log(x); } catch (e) { console.log((e as Error).name); }
  try { console.log(1n << 70n); } catch (e) { console.log((e as Error).name); }
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_bigint_overflow"),
        "4611686018427387904n 12n 4611686018427387904n\nRangeError\nRangeError\nRangeError\n"
    );
}

#[test]
fn audit2_contract_dictionary_array_values_are_shared_and_mutable() {
    let source = r##"function main(): void {
  const o: { [k: string]: number[] } = { x: [1] };
  o.x.push(2);
  console.log(o);
  const g: Record<string, string[]> = {};
  g["a"] = ["x"]; g["a"].push("y"); console.log(g);
  const groups: Record<string, string[]> = {};
  for (const w of ["apple", "avocado", "banana"]) { const k = w[0]; const l = groups[k] || (groups[k] = []); l.push(w); }
  console.log(groups);
  const m: Record<string, number[]> = {};
  (m["k"] ??= []).push(1); (m["k"] ??= []).push(2);
  console.log(m, m.k.length, m["k"][1] + 1);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_dictionary_array_values"),
        "{ x: [ 1, 2 ] }\n{ a: [ 'x', 'y' ] }\n{ a: [ 'apple', 'avocado' ], b: [ 'banana' ] }\n{ k: [ 1, 2 ] } 2 3\n"
    );
}

#[test]
fn audit2_contract_any_array_cursor_keeps_nested_links() {
    let source = r##"function main(): void {
  const root: any[] = []; let cur = root;
  for (let i = 0; i < 3; i++) { const n: any[] = []; cur.push(n); cur = n; }
  console.log(JSON.stringify(root));
  const r: any[] = []; const n: any[] = []; r.push(n); n.push(1); console.log(JSON.stringify(r));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_any_array_cursor"),
        "[[[[]]]]\n[[1]]\n"
    );
}

#[test]
fn audit2_contract_self_referential_class_linked_list() {
    let source = r##"class N {
  next: N | null = null;
  v: number;
  constructor(v: number) { this.v = v; }
}
class List {
  head: N | null = null;
  size = 0;
  push(v: number): void {
    const n = new N(v);
    const h = this.head;
    if (h === null) { this.head = n; } else {
      let cur: N | null = h;
      while (cur !== null) { if (cur.next === null) { cur.next = n; break; } cur = cur.next; }
    }
    this.size++;
  }
  sum(): number { let s = 0; let c = this.head; while (c !== null) { s += c.v; c = c.next; } return s; }
  reverse(): void {
    let prev: N | null = null; let cur = this.head;
    while (cur !== null) { const nx: N | null = cur.next; cur.next = prev; prev = cur; cur = nx; }
    this.head = prev;
  }
  toArray(): number[] { const out: number[] = []; for (let c = this.head; c; c = c.next) out.push(c.v); return out; }
}
function main(): void {
  const l = new List();
  for (let i = 1; i <= 4; i++) l.push(i);
  console.log(l.sum(), l.size, l.toArray());
  l.reverse();
  console.log(l.toArray(), l.head!.next!.v, l.head!.next!.next!.next!.next === null);
  const a = new N(1);
  console.log(a.next === null, a.v);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_self_referential_class_linked_list"),
        "10 4 [ 1, 2, 3, 4 ]\n[ 4, 3, 2, 1 ] 3 true\ntrue 1\n"
    );
}

#[test]
fn audit2_contract_self_referential_class_tree_and_optional_link() {
    let source = r##"class T {
  left: T | null = null; right: T | null = null; v: number;
  constructor(v: number) { this.v = v; }
  insert(v: number): void {
    if (v < this.v) { const l = this.left; if (l === null) this.left = new T(v); else l.insert(v); }
    else { const r = this.right; if (r === null) this.right = new T(v); else r.insert(v); }
  }
  inorder(out: number[]): void {
    const l = this.left; if (l !== null) l.inorder(out);
    out.push(this.v);
    const r = this.right; if (r !== null) r.inorder(out);
  }
}
class N2 { next?: N2; v = 1; }
function main(): void {
  const root = new T(5);
  for (const x of [3, 8, 1, 4, 9, 7]) root.insert(x);
  const out: number[] = []; root.inorder(out);
  console.log(out.join(","));
  const h = new N2(); h.next = new N2();
  console.log(h.next.v, h.next.next === undefined);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_self_referential_class_tree"),
        "1,3,4,5,7,8,9\n1 true\n"
    );
}

#[test]
fn audit2_contract_mutually_recursive_and_forward_referenced_classes() {
    let source = r##"class Dept { name: string; staff: Emp[] = []; boss: Emp | null = null; constructor(n: string) { this.name = n; } }
class Emp { name: string; dept: Dept | null = null; constructor(n: string) { this.name = n; } }
class Tree { kids: Tree[] = []; label: string; constructor(l: string) { this.label = l; }
  add(k: Tree): Tree { this.kids.push(k); return this; }
  count(): number { let n = 1; for (const k of this.kids) n += k.count(); return n; }
}
class Base { link: Base | null = null; id = 1; who(): string { return "base" + this.id; } }
class Derived extends Base { extra = 2; who(): string { return "derived" + this.id; } }
function main(): void {
  const d = new Dept("eng"); const e = new Emp("ann"); e.dept = d; d.staff.push(e); d.boss = e;
  const b = d.boss;
  console.log(d.staff.length, d.staff[0].name, b !== null ? b.name : "none", e.dept === d);
  const t = new Tree("r").add(new Tree("a").add(new Tree("b"))).add(new Tree("c"));
  console.log(t.count(), t.kids[0].kids[0].label);
  const x = new Base(); const y = new Derived(); x.link = y;
  const l = x.link; console.log(l !== null ? l.who() : "-", x.who());
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_mutually_recursive_classes"),
        "1 ann ann true\n4 b\nderived1 base1\n"
    );
}

#[test]
fn audit2_contract_self_referential_interface_and_type_alias() {
    let source = r##"interface TreeNode { v: number; kids: TreeNode[]; parent?: TreeNode }
type Tree = { label: string; kids: Tree[] };
type Opt = { next: Opt | null; n: number };
function sum(t: TreeNode): number { let s = t.v; for (const k of t.kids) s += sum(k); return s; }
function depth(t: Tree): number { let d = 0; for (const k of t.kids) d = Math.max(d, depth(k)); return d + 1; }
function len(o: Opt | null): number { return o === null ? 0 : 1 + len(o.next); }
function main(): void {
  const t: TreeNode = { v: 1, kids: [{ v: 2, kids: [] }, { v: 3, kids: [{ v: 4, kids: [] }] }] };
  console.log(sum(t));
  const u: Tree = { label: "a", kids: [{ label: "b", kids: [{ label: "c", kids: [] }] }] };
  console.log(depth(u), u.kids[0].kids[0].label);
  const o: Opt = { n: 1, next: { n: 2, next: null } };
  console.log(len(o));
  const extra: TreeNode = { v: 10, kids: [] }; t.kids.push(extra); console.log(sum(t));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_self_referential_interface_alias"),
        "10\n3 c\n2\n20\n"
    );
}

#[test]
fn audit2_contract_method_call_on_narrowed_property_local() {
    let source = r##"class B { v = 1; m(): number { return this.v; } }
class A { b: B | null = null; }
function main(): void {
  const a = new A(); a.b = new B();
  const x = a.b;
  if (x !== null) console.log(x.m());
  let c: B | null = new B();
  while (c !== null) { c.v = 5; console.log(c.v); c = null; }
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_method_call_on_narrowed_property_local"),
        "1\n5\n"
    );
}

#[test]
fn audit2_contract_recursive_instances_print_stringify_and_instanceof() {
    let source = r##"class N { next: N | null = null; v: number; constructor(v: number) { this.v = v; } }
class T { kids: T[] = []; label: string; constructor(l: string) { this.label = l; } }
function main(): void {
  const a = new N(1); a.next = new N(2); a.next.next = new N(3);
  console.log(a);
  console.log(JSON.stringify(a));
  console.log(a instanceof N, a.next instanceof N);
  const t = new T("r"); t.kids.push(new T("a")); t.kids[0].kids.push(new T("b")); t.kids.push(new T("c"));
  console.log(t);
  console.log(JSON.stringify(t));
  const c = new N(9); c.next = c;
  try { console.log(JSON.stringify(c)); } catch (e) { console.log((e as Error).name); }
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_recursive_print_stringify_instanceof"),
        "N { next: N { next: N { next: null, v: 3 }, v: 2 }, v: 1 }\n{\"next\":{\"next\":{\"next\":null,\"v\":3},\"v\":2},\"v\":1}\ntrue true\nT {\n  kids: [ T { kids: [Array], label: 'a' }, T { kids: [], label: 'c' } ],\n  label: 'r'\n}\n{\"kids\":[{\"kids\":[{\"kids\":[],\"label\":\"b\"}],\"label\":\"a\"},{\"kids\":[],\"label\":\"c\"}],\"label\":\"r\"}\nTypeError\n"
    );
}

#[test]
fn audit2_contract_generic_recursive_classes() {
    let source = r##"class Node<T> { next: Node<T> | null = null; value: T; constructor(v: T) { this.value = v; } }
class Stack<T> {
  head: Node<T> | null = null; size = 0;
  push(v: T): void { const n = new Node<T>(v); n.next = this.head; this.head = n; this.size++; }
  pop(): T | undefined { const h = this.head; if (h === null) return undefined; this.head = h.next; this.size--; return h.value; }
}
function main1(): void {
  const s = new Stack<number>(); s.push(1); s.push(2); s.push(3);
  console.log(s.pop(), s.pop(), s.size);
  const t = new Stack<string>(); t.push("a"); console.log(t.pop(), t.pop());
}

class TNode<K, V> {
  left: TNode<K, V> | null = null; right: TNode<K, V> | null = null;
  key: K; val: V;
  constructor(k: K, v: V) { this.key = k; this.val = v; }
}
class Tree<V> {
  root: TNode<number, V> | null = null;
  put(k: number, v: V): void {
    const n = new TNode<number, V>(k, v);
    let cur = this.root;
    if (cur === null) { this.root = n; return; }
    while (true) {
      if (k < cur.key) { const l: TNode<number, V> | null = cur.left; if (l === null) { cur.left = n; return; } cur = l; }
      else { const r: TNode<number, V> | null = cur.right; if (r === null) { cur.right = n; return; } cur = r; }
    }
  }
  walk(n: TNode<number, V> | null, out: V[]): void {
    if (n === null) return;
    this.walk(n.left, out); out.push(n.val); this.walk(n.right, out);
  }
  all(): V[] { const out: V[] = []; this.walk(this.root, out); return out; }
}
class Link<T> { next: Link<T> | null = null; v: T; constructor(v: T) { this.v = v; } }
function main(): void {
  main1();
  const t = new Tree<string>();
  t.put(5, "five"); t.put(2, "two"); t.put(8, "eight"); t.put(1, "one");
  console.log(t.all());
  const n = new Tree<number>(); n.put(2, 20); n.put(1, 10); console.log(n.all());
  const l = new Link<number>(1); l.next = new Link<number>(2);
  console.log(l.v, l.next !== null);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_generic_recursive_classes"),
        "3 2 1\na undefined\n[ 'one', 'two', 'five', 'eight' ]\n[ 10, 20 ]\n1 true\n"
    );
}

#[test]
fn audit2_contract_cyclic_recursive_instances_print_circular_refs() {
    let source = r##"class N { next: N | null = null; v = 1; }
class P { kids: P[] = []; up: P | null = null; name: string; constructor(n: string) { this.name = n; } }
function main(): void {
  const a = new N(); a.next = a; console.log(a);
  console.log(a.next === a);
  try { console.log(JSON.stringify(a)); } catch (e) { console.log((e as Error).name); }
  const r = new P("r"); const c = new P("c"); c.up = r; r.kids.push(c); console.log(r);
  const x = new N(); x.next = new N(); x.next.next = x; console.log(x);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_cyclic_recursive_instances"),
        "<ref *1> N { next: [Circular *1], v: 1 }\ntrue\nTypeError\n<ref *1> P {\n  kids: [ P { kids: [], up: [Circular *1], name: 'c' } ],\n  up: null,\n  name: 'r'\n}\n<ref *1> N { next: N { next: [Circular *1], v: 1 }, v: 1 }\n"
    );
}

#[test]
fn audit2_contract_non_literal_class_field_initializers_infer_their_type() {
    let source = r##"class A { static x = 1; static y = A.x + 1; static f = (A.x, "s" + A.x); }
class C { static count = 0; id = ++C.count; label = "n" + this.id; }
class B0 { x = 1; }
class B extends B0 { y = this.x + 1; big = this.y > 1; }
function main(): void {
  console.log(A.y, A.f);
  new C(); const c = new C();
  console.log(c.id, c.label, new B().y, new B().big);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_non_literal_class_field_initializers"),
        "2 s1\n2 n2 2 true\n"
    );
}

#[test]
fn audit2_contract_to_string_is_used_by_join_and_array_conversion() {
    let source = r##"class A { toString(): string { return "A!"; } }
function main(): void {
  const a = new A();
  const o = { toString() { return "O"; } };
  console.log(String(a), `${a}`, a + "", [a].join(), [a, a].join("-"), [a].toString(), "" + [a]);
  console.log(String(o), `${o}`, [o].join());
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_to_string_is_used_by_join_and_array_conversion"),
        "A! A! A! A! A!-A! A! A!\nO O O\n"
    );
}

#[test]
fn audit2_contract_nested_to_json_methods_are_called() {
    let source = r##"class A { toJSON(): { t: number } { return { t: 1 }; } }
function main(): void {
  console.log(JSON.stringify(new A()), JSON.stringify([new A()]), JSON.stringify({ k: new A() }));
  console.log(JSON.stringify({ toJSON(k: string) { return "key:" + k; } }), JSON.stringify({ a: { toJSON(k: string) { return "key:" + k; } } }));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_nested_to_json_methods_are_called"),
        "{\"t\":1} [{\"t\":1}] {\"k\":{\"t\":1}}\n\"key:\" {\"a\":\"key:a\"}\n"
    );
}

#[test]
fn audit2_contract_this_in_plain_function_and_arrow() {
    let source = r##"function main(): void {
  console.log("before");
  console.log((() => { return typeof this; })());
  console.log(typeof this, "after");
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_this_in_plain_function_and_arrow"),
        "before\nobject\nobject after\n"
    );
}

#[test]
fn audit2_contract_sort_follows_v8_timsort_call_order() {
    let source = r##"function main(): void {
  console.log([3, 1, 2, 5, 4].sort((a, b) => (a > b ? 1 : 0)));
  console.log([5, 2, 9, 1, 7, 3, 8].sort((a, b) => (a < b ? -1 : 0)));
  const a = [5, 2, 9, 1, 7, 3, 8, 4, 6, 0, 11, 15, 12, 13, 10, 14];
  console.log(a.sort((x, y) => (x > y ? 1 : 0)).join());
  const c = [3, 1, 2]; console.log(c.sort(() => -1), [3, 1, 2].sort(() => 1));
  const calls: string[] = []; [4, 2, 3, 1].sort((x, y) => { calls.push(x + ">" + y); return x - y; });
  console.log(calls.join(" "));
  const s: string[] = []; ["d", "a", "c", "b", "e", "f", "g"].sort((x, y) => { s.push(x + y); return x < y ? -1 : x > y ? 1 : 0; });
  console.log(s.join(" "));
  console.log([2, 1].sort((x, y) => x - y), [1].sort((x, y) => x - y));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_sort_follows_v8_timsort_call_order"),
        "[ 3, 1, 2, 5, 4 ]\n[\n  1, 2, 3, 5,\n  7, 8, 9\n]\n5,2,9,1,7,3,8,4,6,0,11,15,12,13,10,14\n[ 2, 1, 3 ] [ 3, 1, 2 ]\n2>4 3>2 3>4 3>2 1>3 1>2\nad ca cd ca bc ba ec ed fc fe gd gf\n[ 1, 2 ] [ 1 ]\n"
    );
}
