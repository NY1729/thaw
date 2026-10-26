// Remaining class-field limits: precise bare-field widening, `undefined` through returns/containers/
// literals, abstract-override creation order, unset numbers through structural views, JSON.stringify
// of function values, nested typed arrays and function name inference. Expected output recorded
// from node 22 (--experimental-strip-types).

#[test]
fn fldlim_bare_fields_stay_raw_until_an_instance_can_be_printed() {
    let source = r##"class A { x: number; s: string; }
class B { y: number[]; }
class C { v: number; w: string; }
function main(): void {
  const a = new A();
  console.log("hi", 1 + 2);
  console.log(a.x === undefined);
  const b = new B();
  b.y = [1];
  console.log(b.y.length);
  const c = new C();
  c.v = 2; c.w = "k";
  console.log(c, c.v + 1);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_bare_fields_stay_raw_until_an_instance_can_be_printed"),
        "hi 3\ntrue\n1\nC { v: 2, w: 'k' } 3\n"
    );
}

#[test]
fn fldlim_value_context_reads_keep_undefined_through_returns_containers_and_literals() {
    let source = r##"class P { n!: number; s!: string; set(n: number): void { this.n = n; } }
function h(p: P): number { return p.n }
const ar = (p: P): number => p.n;
const ar2 = (p: P): number => { return p.n; };
function touch(p: P): number { p.n = 5; return p.n; }
function main(): void {
  const p = new P();
  console.log(h(p), ar(p), ar2(p));
  const arr: number[] = [];
  arr.push(p.n);
  console.log(arr);
  const m = new Map<string, number>();
  m.set("k", p.n);
  console.log(m);
  console.log({ n: p.n });
  const o = { n: p.n, s: p.s };
  console.log(o);
  const ps = [new P(), new P()];
  ps[0].set(3);
  const mapped = ps.map(x => x.n);
  console.log(mapped);
  console.log(Math.max(...mapped));
  console.log(Math.max(...ps.map(x => x.n)));
  const v = p.n;
  console.log(v);
  let w: number = p.n;
  console.log(w, typeof w);
  const z = [p.n, 1];
  console.log(z);
  const q = new P();
  console.log(touch(q), q.n);
  q.set(9);
  console.log(h(q), q);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_value_context_reads_keep_undefined_through_returns_containers_and_literals"),
        "undefined undefined undefined\n[ undefined ]\nMap(1) { 'k' => undefined }\n{ n: undefined }\n{ n: undefined, s: undefined }\n[ 3, undefined ]\nNaN\nNaN\nundefined\nundefined undefined\n[ undefined, 1 ]\n5 5\n9 P { n: 9, s: undefined }\n"
    );
}

#[test]
fn fldlim_abstract_overrides_print_in_node_creation_order() {
    let source = r##"abstract class Base { abstract a: number; abstract b: string; c: number = 1; show(): number { return this.c + this.a; } }
class Impl extends Base { b: string = "x"; a: number = 2; d: number = 3; }
abstract class B2 { abstract p: number; abstract q: number; constructor() { } m(): number { return this.q; } }
class I2 extends B2 { q: number = 5; p: number = 6; }
class I3 extends I2 { r: number = 7; }
function main(): void {
  const i = new Impl();
  console.log(i, i.show(), i.a, i.b);
  const j = new I2();
  console.log(j, j.m(), j.p);
  console.log(JSON.stringify(j), Object.keys(j));
  const k = new I3();
  console.log(k, Object.entries(k));
  for (const key in i) console.log(key);
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_abstract_overrides_print_in_node_creation_order"),
        "Impl { c: 1, b: 'x', a: 2, d: 3 } 3 2 x\nI2 { q: 5, p: 6 } 5 6\n{\"q\":5,\"p\":6} [ 'q', 'p' ]\nI3 { q: 5, p: 6, r: 7 } [ [ 'q', 5 ], [ 'p', 6 ], [ 'r', 7 ] ]\nc\nb\na\nd\n"
    );
}

#[test]
fn fldlim_structural_view_reads_unset_number_as_undefined() {
    let source = r##"class P { n!: number; t: string = "t"; }
function rd(v: { n: number }): number { return v.n; }
function pr(v: { n: number }): void { console.log(v.n); }
function pr2(v: { n: number }): void { console.log(v.n, typeof v.n, v.n === undefined); }
function sum(v: { n: number }): number { return v.n + 1; }
function put(v: { n: number }): void { v.n = 42; }
function main(): void {
  const p = new P();
  console.log(rd(p));
  pr(p);
  pr2(p);
  console.log(sum(p));
  const q = new P(); q.n = 3;
  console.log(rd(q)); pr(q); pr2(q);
  put(p);
  console.log(p.n, rd(p));
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_structural_view_reads_unset_number_as_undefined"),
        "undefined\nundefined\nundefined undefined true\nNaN\n3\n3\n3 number false\n42 42\n"
    );
}

#[test]
fn fldlim_json_stringify_omits_function_values() {
    let source = r##"class H { f!: () => number; g!: () => number; arr: number[] = []; o: { k: number } = { k: 1 }; fill(): void { this.f = () => 7; } }
function main(): void {
  const h = new H(); h.fill();
  console.log(JSON.stringify(h));
  console.log(JSON.stringify({ f: () => 1, n: 1 }));
  console.log(JSON.stringify({ a: [() => 1, () => 2], n: 1 }));
  console.log(JSON.stringify([function () {}, () => 1]));
  console.log(JSON.stringify({ a: [() => 1, 2], n: 1 }));
  console.log(JSON.stringify([function () {}, 1]));
  console.log(JSON.stringify({ f: h.f }));
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_json_stringify_omits_function_values"),
        "{\"arr\":[],\"o\":{\"k\":1}}\n{\"n\":1}\n{\"a\":[null,null],\"n\":1}\n[null,null]\n{\"a\":[null,2],\"n\":1}\n[null,1]\n{}\n"
    );
}

#[test]
fn fldlim_nested_typed_arrays_and_function_names_print_like_node() {
    let source = r##"function named(): number { return 1; }
const loader = async (): Promise<number> => 2;
function main(): void {
  const o = { u: new Uint8Array(2), cb: () => 1, f: function () { return 1; }, g: named, h: loader };
  console.log(o);
  const callback = () => 2;
  const o2 = { callback, named, nested: { u: new Uint8Array(3) } };
  console.log(o2);
  console.log([new Uint8Array(2), callback, () => 3, named]);
  const f = function () {};
  const g = () => {};
  console.log([f, g, named, loader]);
  const arr = [() => 1];
  console.log(arr);
  const m = new Map([["a", new Uint8Array(1)]]);
  console.log(m);
  let asg = () => 0;
  asg = () => 6;
  console.log({ asg });
  const holder: { fn?: () => number } = {};
  holder.fn = () => 4;
  console.log(holder);
  const named2 = function inner(): number { return 1; };
  console.log({ named2 });
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_nested_typed_arrays_and_function_names_print_like_node"),
        "{\n  u: Uint8Array(2) [ 0, 0 ],\n  cb: [Function: cb],\n  f: [Function: f],\n  g: [Function: named],\n  h: [AsyncFunction: loader]\n}\n{\n  callback: [Function: callback],\n  named: [Function: named],\n  nested: { u: Uint8Array(3) [ 0, 0, 0 ] }\n}\n[\n  Uint8Array(2) [ 0, 0 ],\n  [Function: callback],\n  [Function (anonymous)],\n  [Function: named]\n]\n[\n  [Function: f],\n  [Function: g],\n  [Function: named],\n  [AsyncFunction: loader]\n]\n[ [Function (anonymous)] ]\nMap(1) { 'a' => Uint8Array(1) [ 0 ] }\n{ asg: [Function: asg] }\n{ fn: [Function (anonymous)] }\n{ named2: [Function: inner] }\n"
    );
}

#[test]
fn fldlim_inferred_non_arrow_function_kinds_keep_their_names() {
    let source = r##"const syncPromise = function(): Promise<number> { return Promise.resolve(1); };
const syncNewPromise = function(): Promise<number> { return new Promise<number>((resolve) => resolve(2)); };
const syncGenerator = function* (): Generator<number> { yield 1; };
const asyncFunction = async function(): Promise<number> { return 1; };
const asyncAwaited = async function(): Promise<number> { const value = await Promise.resolve(2); return value; };
const asyncGenerator = async function* (): AsyncGenerator<number> { yield 1; };
function main(): void { console.log({ syncPromise, syncNewPromise, syncGenerator, asyncFunction, asyncAwaited, asyncGenerator }); }
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_inferred_non_arrow_function_kinds_keep_their_names"),
        "{\n  syncPromise: [Function: syncPromise],\n  syncNewPromise: [Function: syncNewPromise],\n  syncGenerator: [GeneratorFunction: syncGenerator],\n  asyncFunction: [AsyncFunction: asyncFunction],\n  asyncAwaited: [AsyncFunction: asyncAwaited],\n  asyncGenerator: [AsyncGeneratorFunction: asyncGenerator]\n}\n"
    );
}

#[test]
fn fldlim_class_field_arrow_names_follow_define_semantics() {
    let source = r##"class K { f = () => 1; g!: () => number; constructor() { this.g = () => 2; } }
function main(): void {
  console.log(new K());
}
"##;
    assert_eq!(
        compile_and_run(source, "fldlim_class_field_arrow_names_follow_define_semantics"),
        "K { f: [Function: f], g: [Function (anonymous)] }\n"
    );
}
