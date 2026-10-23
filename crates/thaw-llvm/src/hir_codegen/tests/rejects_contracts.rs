// Contracts for valid TypeScript the compiler used to reject: async/Promise.all of void tasks and
// awaiting plain values, unary plus on strings, reassigning `let x: any`, optional calls/indexing on
// `any`, `new Array(n)`, `Math.f` as a value, native bigint operators, enums through `any`, iterable
// classes in spread/Array.from, `x.constructor`, defaulted parameter properties, arrow class fields,
// and `filter(Boolean)`/`map(Number)`. Expected output from node 22 (--experimental-transform-types).

#[test]
fn rejects_any_let_reassign() {
    let source = r###"function main(): void {
  let a: any = 5; a = "s"; console.log(a);
  let b: any = "s"; b = true; console.log(b);
  let c: any = null; c = 4; console.log(c);
  let d: any = "x"; d = 8; console.log(d);
  let n: any = null, z: any = 0, e: any = "", f: any = "x";
  n ??= 5; z ||= 6; e &&= 7; f &&= 8;
  console.log(n, z, e, f);
}
"###;
    assert_eq!(compile_and_run(source, "rejects_any_let_reassign"), "s\ntrue\n4\n8\n5 6  8\n");
}

#[test]
fn rejects_array_constructor() {
    let source = r###"function main(): void {
  const a = new Array(3);
  console.log(a.length);
  const b = Array(3).fill(0);
  console.log(b.join(","));
  const c = new Array<number>(2).fill(1);
  console.log(c);
  const d: number[] = new Array(4);
  d[1] = 7;
  console.log(d.length, d[1]);
  const e = new Array(4).fill("x");
  console.log(e.join("-"));
  const f = new Array(1, 2, 3);
  console.log(f);
  const g = new Array(3).fill(true).map((v, i) => i);
  console.log(g);
  const h: string[] = Array(2).fill("q");
  console.log(h);
  console.log([...Array(3).keys()]);
  const u = new Array(3);
  u[0] = 5; u[1] = "s";
  console.log(u[0], u[1], u.length);
  const grid = Array.from({ length: 2 }, () => new Array(2).fill(0));
  grid[1][0] = 3;
  console.log(grid);
  console.log(new Array(3).fill(0).map((_, i) => i * i));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_array_constructor"), "3\n0,0,0\n[ 1, 1 ]\n4 7\nx-x-x-x\n[ 1, 2, 3 ]\n[ 0, 1, 2 ]\n[ 'q', 'q' ]\n[ 0, 1, 2 ]\n5 s 3\n[ [ 0, 0 ], [ 3, 0 ] ]\n[ 0, 1, 4 ]\n");
}

#[test]
fn rejects_arrow_field_inference() {
    let source = r###"class Counter {
  count = 0;
  label = "c";
  inc = () => { this.count++; };
  add = (by: number) => { this.count += by; };
  show = () => this.label + this.count;
  double = (x: number) => { return x * 2; };
}
function main(): void {
  const c = new Counter();
  c.inc(); c.add(5);
  const f = c.show;
  console.log(f(), c.double(4));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_arrow_field_inference"), "c6 8\n");
}

#[test]
fn rejects_await_plain_value_iife() {
    let source = r###"async function h(): Promise<void> { await null; console.log("h"); }
async function main(): Promise<void> {
  console.log("start");
  const pending = (async () => {
    await null;
    console.log("iife after null");
  })();
  console.log("sync");
  await pending;
  await h();
  const x = await 5;
  const s = await "str";
  console.log(x, s);
  const f = async () => { const y = await 7; console.log(y); };
  await f();
  const v = await (async () => { await Promise.resolve(1); return 5; })();
  console.log(v);
}
"###;
    assert_eq!(compile_and_run(source, "rejects_await_plain_value_iife"), "start\nsync\niife after null\nh\n5 str\n7\n5\n");
}

#[test]
fn rejects_bigint_operators() {
    let source = r###"function main(): void {
  console.log(~5n, ~-1n, 1n < 2, 10n > 5, 3n == 3, 5n != 5, 2n > 1.5, 5n == 5.5);
  console.log(!!0n, !!7n, 0n ? "a" : "b", 0n || 9, 1n && 2n);
  console.log(2n ** 10n, 3n ** 20n, Number(5n) + 1);
  let n = 3n;
  if (n) console.log("truthy");
  n = 0n;
  if (!n) console.log("falsy");
}
"###;
    assert_eq!(compile_and_run(source, "rejects_bigint_operators"), "-6n 0n true true true false true false\nfalse true b 9 2n\n1024n 3486784401n 6\ntruthy\nfalsy\n");
}

#[test]
fn rejects_builtin_conversion_callbacks() {
    let source = r###"function main(): void {
  console.log([0, 1, 2, 0, 3].filter(Boolean));
  console.log(["a", "", "b"].filter(Boolean).length);
  console.log(["1", "2"].map(Number));
  console.log([1, 2].map(String));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_builtin_conversion_callbacks"), "[ 1, 2, 3 ]\n2\n[ 1, 2 ]\n[ '1', '2' ]\n");
}

#[test]
fn rejects_class_constructor_name() {
    let source = r###"class Animal { speak(): string { return "..."; } }
class Dog extends Animal { speak(): string { return "woof"; } }
class Puppy extends Dog {}
function describe(a: Animal): string { return a.constructor.name + ":" + a.speak(); }
function main(): void {
  const d = new Dog();
  console.log(d.constructor.name, new Animal().constructor.name, new Puppy().constructor.name);
  console.log(describe(new Dog()), describe(new Puppy()), describe(new Animal()));
  console.log(d.constructor === Dog, d.constructor === Animal, d.constructor !== Dog);
  const list: Animal[] = [new Animal(), new Dog()];
  for (const a of list) console.log(a.constructor.name, a.constructor === Dog);
}
"###;
    assert_eq!(compile_and_run(source, "rejects_class_constructor_name"), "Dog Animal Puppy\nDog:woof Puppy:woof Animal:...\ntrue false false\nAnimal false\nDog true\n");
}

#[test]
fn rejects_class_iterable_spread() {
    let source = r###"class R {
  n: number;
  constructor(n: number) { this.n = n; }
  *[Symbol.iterator](): Generator<number> { for (let i = 0; i < this.n; i++) yield i; }
}
function main(): void {
  for (const x of new R(3)) console.log(x);
  console.log([...new R(2)]);
  console.log([9, ...new R(2), 8]);
  const a = Array.from(new R(3));
  console.log(a.length, a[0], a[2]);
  const b = Array.from(new R(2), (x) => x * 2);
  console.log(b.join(","));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_class_iterable_spread"), "0\n1\n2\n[ 0, 1 ]\n[ 9, 0, 1, 8 ]\n3 0 2\n0,2\n");
}

#[test]
fn rejects_enum_through_any() {
    let source = r###"enum Color { Red, Green = 5, Blue }
enum Dir { Up = "UP", Down = "DOWN" }
function pick(v: any): string { return Color[v]; }
function main(): void {
  const x: any = Color.Green;
  console.log(x, Color[x], Color.Blue);
  console.log(pick(Color.Blue), pick(0));
  const d: any = Dir.Down;
  console.log(d, d === Dir.Down, typeof d);
  const o: any = { c: Color.Red, d: Dir.Up };
  console.log(o.c, o.d, Color[o.c]);
  const vals: any[] = [Color.Red, Color.Green];
  console.log(vals.map((v: any) => Color[v]).join());
}
"###;
    assert_eq!(compile_and_run(source, "rejects_enum_through_any"), "5 Green 6\nBlue Red\nDOWN true string\n0 UP Red\nRed,Green\n");
}

#[test]
fn rejects_math_function_values() {
    let source = r###"function main(): void {
  console.log([1.5, 2.5, -1.2].map(Math.floor));
  console.log([-1, 2].map(Math.abs));
  const f = Math.max;
  console.log(f(1, 5, 3));
  const g = Math.min;
  console.log(g(4, 2), g());
  const p = Math.pow;
  console.log(p(2, 10), [4, 9].map(Math.sqrt));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_math_function_values"), "[ 1, 2, -2 ]\n[ 1, 2 ]\n5\n2 Infinity\n1024 [ 2, 3 ]\n");
}

#[test]
fn rejects_namespace_flattening() {
    let source = r###"namespace NS {
  export const a = 1;
  let hidden = 10;
  export function f(): number { hidden++; return a + hidden; }
  export class K { v = 3; get(): number { return a + this.v; } }
  export interface Shape { w: number }
  export enum Level { Low, High }
  export namespace Inner {
    export function g(x: number): number { return x * a; }
    export const tag = "inner";
  }
  export function useInner(): number { return Inner.g(2) + Level.High; }
}
namespace Util {
  export function double(n: number): number { return n * 2; }
}
namespace Util {
  export const base = 21;
  export function answer(): number { return Util.double(base); }
  console.log("util ready");
}
function area(s: NS.Shape): number { return s.w * 2; }
function main(): void {
  console.log(NS.a, NS.f(), NS.f(), new NS.K().v, new NS.K().get());
  console.log(NS.Inner.g(5), NS.Inner.tag, NS.useInner(), NS.Level.High);
  const k: NS.K = new NS.K();
  console.log(k.v, area({ w: 4 }));
  console.log(Util.answer(), Util.double(4), Util.base);
}
"###;
    assert_eq!(compile_and_run(source, "rejects_namespace_flattening"), "util ready\n1 12 13 3 4\n5 inner 3 1\n3 8\n42 8 21\n");
}

#[test]
fn rejects_optional_member_call_chain() {
    let source = r###"function main(): void {
  const t: { a?: { f?: () => number } } = { a: { f: () => 3 } };
  console.log(t.a?.f?.());
  const u: { a?: { f?: () => number } } = {};
  console.log(u.a?.f?.());
  const v: { a?: { f?: () => number } } = { a: {} };
  console.log(v.a?.f?.());
}
"###;
    assert_eq!(compile_and_run(source, "rejects_optional_member_call_chain"), "3\nundefined\nundefined\n");
}

#[test]
fn rejects_optional_on_any() {
    let source = r###"function main(): void {
  const o: any = { a: { c: () => 5, d: (x: number) => x + 1 }, arr: [1] };
  console.log(o.a.c?.());
  console.log(o.z?.());
  console.log(o.a.zz?.());
  console.log(o.a.d?.(4));
  console.log(o.arr?.[0], o.q?.[0]);
  let calls = 0;
  const f = () => { calls++; return 1; };
  console.log(o.nothing?.[f()], calls);
  const i = 0;
  console.log(o.arr?.[i]);
}
"###;
    assert_eq!(compile_and_run(source, "rejects_optional_on_any"), "5\nundefined\nundefined\n5\n1 undefined\nundefined 0\n1\n");
}

#[test]
fn rejects_param_property_default() {
    let source = r###"class P {
  constructor(private z = 3, public name = "x", readonly on = true) {}
  get(): string { return this.name + this.z + this.on; }
}
function main(): void { console.log(new P().get(), new P(5, "y", false).get()); }
"###;
    assert_eq!(compile_and_run(source, "rejects_param_property_default"), "x3true y5false\n");
}

#[test]
fn rejects_promise_all_plain_values() {
    let source = r###"async function d(x: number): Promise<number> { await Promise.resolve(0); return x * 2; }
async function main(): Promise<void> {
  const r = await Promise.all([1, Promise.resolve(2), d(3)]);
  console.log(r.join(","));
  const w = await Promise.all(["a", Promise.resolve("b")]);
  console.log(w.join(","));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_promise_all_plain_values"), "1,2,6\na,b\n");
}

#[test]
fn rejects_promise_all_void_array() {
    let source = r###"async function main(): Promise<void> {
  const out: number[] = [];
  const arr = [1, 2, 3];
  await Promise.all(arr.map(async (n) => { out.push(n * 2); }));
  console.log(out.join(","));
  const r = await Promise.all(arr.map(async (n) => n + 1));
  console.log(r.join(","));
  const tasks = [1, 2].map(async (n) => { await Promise.resolve(n); console.log("task", n); });
  await Promise.all(tasks);
  console.log("end");
}
"###;
    assert_eq!(compile_and_run(source, "rejects_promise_all_void_array"), "2,4,6\n2,3,4\ntask 1\ntask 2\nend\n");
}

#[test]
fn rejects_string_index_read() {
    let source = r###"function main(): void {
  const s = "Hello";
  console.log(s[1], s[s.length - 1], s[0] + s[4]);
  let n = 0;
  for (let i = 0; i < s.length; i++) { if (s[i] === "l") n++; }
  console.log(n);
  const words = ["ab", "cd"];
  console.log(words[1][0], words.map((w) => w[1]).join(""));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_string_index_read"), "e o Ho\n2\nc bd\n");
}

#[test]
fn rejects_unary_plus_string() {
    let source = r###"function main(): void {
  const s: string = "12";
  console.log(+s + 1, +"3.5", +"abc");
  const t = "  7 ";
  console.log(+t, +true, +"", +" 0x10 ", +"1e3", +"-5");
  const a: any = "42";
  const arr = ["1", "2"];
  console.log(+a, +arr[1]);
  console.log(["10", "20"].map((x) => +x).reduce((p, c) => p + c, 0));
}
"###;
    assert_eq!(compile_and_run(source, "rejects_unary_plus_string"), "13 3.5 NaN\n7 1 0 16 1000 -5\n42 2\n30\n");
}
