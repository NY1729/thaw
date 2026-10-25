// Class fields the constructor may leave unset read `undefined` like Node (C1-C5 of
// designs/plans/class-fields-undefined.md). Expected output recorded from node 22.

#[test]
fn classfields_memo_repro() {
    let source = r##"class P { n!: number; b!: boolean; o?: number; ready = false; init(): void { this.n = 5; this.b = true; } }
class Q { n: number; constructor(flag: boolean) { if (flag) this.n = 1; } }
function main(): void {
  const p = new P();
  console.log(p.n, p.b, p.o, typeof p.n, p.n + 1, p.n === undefined);
  console.log(p);
  const q = new Q(false); console.log(q.n, q);
  console.log((p.o as number) + 1, p.o! + 1);
  p.init();
  console.log(p.n + 1, p.b, p.n);
}
"##;
    assert_eq!(
        compile_and_run(source, "classfields_memo_repro"),
        "undefined undefined undefined undefined NaN true\nP { n: undefined, b: undefined, o: undefined, ready: false }\nundefined Q { n: undefined }\nNaN NaN\n6 true 5\n"
    );
}

#[test]
fn classfields_all_field_types() {
    let source = r##"class Inner { v: number = 1; }
class C {
  s!: string; a!: number[]; inner!: Inner; big!: bigint;
  cnt!: number;
  tag: string = "t";
  set(): void { this.s = "hi"; this.a = [1]; this.inner = new Inner(); this.big = 3n; this.cnt = 1; }
}
class Both { n: number; constructor(f: boolean) { if (f) this.n = 1; else this.n = 2; } }
class D extends C { extra!: boolean; }
function show(x: number): number { return x * 2; }
function main(): void {
  const c = new C();
  console.log(c.s, typeof c.s, c.a, c.inner, c.big, c.cnt);
  console.log(c);
  console.log(JSON.stringify(c), Object.keys(c));
  console.log(`${c.s} ${c.cnt}`);
  console.log(show(c.cnt), c.cnt === undefined, c.s === undefined, c.a === undefined);
  c.cnt += 1; console.log(c.cnt);
  c.set();
  console.log(c.s, c.a, c.inner, c.big, c.cnt, show(c.cnt), c.s.length);
  c.a.push(2); c.cnt++; console.log(c.a, c.cnt);
  console.log(c);
  console.log(new Both(true), new Both(false).n);
  const d = new D();
  console.log(d.extra, d.cnt, d);
  console.log(d.cnt ?? 7, d.s ?? "dflt");
}
"##;
    assert_eq!(
        compile_and_run(source, "classfields_all_field_types"),
        "undefined undefined undefined undefined undefined undefined\nC {\n  s: undefined,\n  a: undefined,\n  inner: undefined,\n  big: undefined,\n  cnt: undefined,\n  tag: 't'\n}\n{\"tag\":\"t\"} [ 's', 'a', 'inner', 'big', 'cnt', 'tag' ]\nundefined undefined\nNaN true true true\nNaN\nhi [ 1 ] Inner { v: 1 } 3n 1 2 2\n[ 1, 2 ] 2\nC {\n  s: 'hi',\n  a: [ 1, 2 ],\n  inner: Inner { v: 1 },\n  big: 3n,\n  cnt: 2,\n  tag: 't'\n}\nBoth { n: 1 } 2\nundefined undefined D {\n  s: undefined,\n  a: undefined,\n  inner: undefined,\n  big: undefined,\n  cnt: undefined,\n  tag: 't',\n  extra: undefined\n}\n7 dflt\n"
    );
}

#[test]
fn classfields_inner_late_cond() {
    let source = r##"class Inner { v: number = 1; }
class Holder { inner!: Inner; items!: string[]; name!: string; }
class Late {
  total!: number;
  label!: string;
  constructor(seed: number) { Object.assign(this, { total: seed }); }
}
class Cond {
  a: number; b: number;
  constructor(f: boolean) { this.a = 1; if (f) { this.b = 2; } }
}
function main(): void {
  const h = new Holder();
  console.log(h.inner, h.items, h.name);
  h.inner = new Inner();
  h.items = ["x"];
  h.name = "n";
  console.log(h.inner, h.items, h.name, h);
  console.log(new Late(4).label, new Late(4).total);
  const c = new Cond(false);
  console.log(c.a, c.b, c);
  console.log(new Cond(true).b);
}
"##;
    assert_eq!(
        compile_and_run(source, "classfields_inner_late_cond"),
        "undefined undefined undefined\nInner { v: 1 } [ 'x' ] n Holder { inner: Inner { v: 1 }, items: [ 'x' ], name: 'n' }\nundefined 4\n1 undefined Cond { a: 1, b: undefined }\n2\n"
    );
}

#[test]
fn classfields_inherited_acc_and_arrays() {
    let source = r##"interface HasN { n: number }
class Svc {
  n!: number; name!: string; items!: number[]; ok!: boolean;
  start(): void { this.n = 3; this.name = "svc"; this.items = [1, 2, 3]; this.ok = true; }
  calc(): number {
    let total = 0;
    for (let i = 0; i < this.n; i++) { total += this.items[i] * this.n; }
    if (this.n > 2 && this.ok) total += Math.max(this.n, 10);
    const arr: number[] = [];
    arr.push(this.n);
    return total + (this.ok ? 1 : 0) + arr[0];
  }
  desc(): string { return "name=" + this.name + " len=" + this.name.length + ` n=${this.n}`; }
}
function useN(x: HasN): number { return x.n + 1; }
function takes(n: number, s: string, o: boolean): string { return n + s + o; }
function main(): void {
  const s = new Svc();
  s.start();
  console.log(useN(s), takes(s.n, s.name, s.ok));
  const { n, name } = s;
  console.log(n, name);
  const copy = { ...s };
  console.log(copy.n, JSON.stringify(s));
  const m = new Map<string, number>(); m.set("a", s.n);
  console.log(m.get("a"), s.items.map((x) => x * s.n), [s.n, s.n + 1]);
  switch (s.n) { case 3: console.log("three"); break; default: console.log("other"); }
  const fresh = new Svc();
  console.log(fresh.n > 1, fresh.n < 1, !fresh.name, fresh.ok ? "y" : "n", fresh.name ?? "none", fresh.n || 9);
  console.log(fresh.n == null, fresh.n === 0, typeof fresh.ok, `${fresh.name}`, "" + fresh.n);
  if (fresh.name === undefined) console.log("unset"); else console.log("set");
  console.log(fresh.items?.length, fresh.name?.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "classfields_inherited_acc_and_arrays"),
        "4 3svctrue\n3 svc\n3 {\"n\":3,\"name\":\"svc\",\"items\":[1,2,3],\"ok\":true}\n3 [ 3, 6, 9 ] [ 3, 4 ]\nthree\nfalse false true n none 9\ntrue false undefined undefined undefined\nunset\nundefined undefined\n"
    );
}

#[test]
fn classfields_inherited_methods_and_truthiness() {
    let source = r##"abstract class Base {
  id!: number;
  log: string[] = [];
  constructor() { this.log.push("base"); }
}
class Impl extends Base {
  extra!: string;
  constructor(p: number) { super(); this.id = p; }
  bump(): void { this.id += 10; this.extra = (this.extra ?? "") + "x"; }
  get twice(): number { return this.id * 2; }
}
class Acc {
  s!: string; n!: number; list!: string[]; flag!: boolean;
  run(): string {
    const f = () => this.n;
    if (this.n !== undefined) return "has";
    if (!this.list) this.list = [];
    this.list.push("a");
    this.s = "q"; this.s += "z";
    this.n = 5;
    const g = () => this.n + f();
    if (this.flag) return "flag"; else if (this.s) return this.s + g() + this.list.length + this.n!;
    return "none";
  }
}
async function later(a: Acc): Promise<number> { await Promise.resolve(1); return a.n + 1; }
function main(): void {
  const i = new Impl(3);
  console.log(i.id, i.twice, i.extra, i.log);
  i.bump(); i.bump();
  console.log(i.id, i.twice, i.extra, i);
  const a = new Acc();
  console.log(a.run(), a.run(), a);
  const arr: Acc[] = [a, new Acc()];
  console.log(arr.map((x) => x.n ?? 0), arr.filter((x) => x.n === undefined).length);
  later(a).then((v) => console.log("later", v));
  let total = 0;
  for (const x of arr) { if (x.n) total += x.n; }
  console.log(total);
}
"##;
    assert_eq!(
        compile_and_run(source, "classfields_inherited_methods_and_truthiness"),
        "3 6 undefined [ 'base' ]\n23 46 xx Impl { id: 23, log: [ 'base' ], extra: 'xx' }\nqz1015 has Acc { s: 'qz', n: 5, list: [ 'a' ], flag: undefined }\n[ 5, 0 ] 1\n5\nlater 6\n"
    );
}
