// Wave I: language-semantics contracts. Expected output recorded from node 22.

#[test]
fn i_contract_k37() {
    let source = r##"function main() { const u = new Uint8Array(1); try { u.set([1, 2]); console.log("no throw", u[0]); } catch (e) { console.log("threw", (e as any).name); } }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k37"), "threw RangeError\n");
}

#[test]
fn i_contract_k37b() {
    let source = r##"function main() { const u = new Uint8Array(4); u.set([1, 2], 1); u.set(new Uint8Array([9]), 3); let r = ""; try { u.set([1], 4); r += "ok"; } catch (e) { r += "x"; } try { u.set([1, 2], 3); r += "bad"; } catch (e) { r += (e as any).name; } try { u.set([1], -1); r += "bad"; } catch (e) { r += (e as any).name; } console.log(Array.from(u).join(","), r); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k37b"), "0,1,2,9 xRangeErrorRangeError\n");
}

#[test]
fn i_contract_k17() {
    let source = r##"function main() { const o: any = {}; const log: string[] = []; Object.defineProperty(o, "x", { value: 1, writable: (log.push("w"), true), enumerable: (log.push("e"), false), configurable: (log.push("c"), true) }); console.log(log.join(",")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k17"), "w,e,c\n");
}

#[test]
fn i_contract_k18() {
    let source = r##"let calls = 0; function mk(): any { calls++; return {}; }
function main() { Object.defineProperties(mk(), { a: { value: 1 }, b: { value: 2 } }); console.log(calls); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k18"), "1\n");
}

#[test]
fn i_contract_k42b() {
    let source = r##"function t(f: () => any) { try { return String(f()); } catch (e) { return "threw " + (e as any).name; } }
function main() {
  const s = Symbol("x");
  console.log(t(() => "" + s));
  console.log(t(() => `${s}`));
  console.log(t(() => String(s)));
  console.log(t(() => s.toString()));
  console.log(t(() => s.description));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k42b"), "threw TypeError\nthrew TypeError\nSymbol(x)\nSymbol(x)\nx\n");
}

#[test]
fn i_contract_k56() {
    let source = r##"function main() { const t = { x: 0 }; Object.assign(t, { x: 7 }); console.log(t.x); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k56"), "7\n");
}

#[test]
fn i_contract_k56b() {
    let source = r##"function main() {
  const t = { x: 0, y: 1 };
  const r = Object.assign(t, { x: 7 }, { y: 9 });
  console.log(t.x, t.y, r === t);
  const a = { n: 1, s: "a" };
  const b = Object.assign(a, { s: "b" });
  console.log(a.s, b.n);
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k56b"), "7 9 true\nb 1\n");
}

#[test]
fn i_contract_k59() {
    let source = r##"function main() { const o: any = { x: 1 }; Object.defineProperty(o, "x", { writable: false }); console.log(o.x); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k59"), "1\n");
}

#[test]
fn i_contract_k60() {
    let source = r##"let calls = 0; function mk(): any { calls++; return {}; }
function main() { const r = Object.defineProperty(mk(), "x", { value: 7 }); console.log(calls, r.x); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k60"), "1 7\n");
}

#[test]
fn i_contract_k60b() {
    let source = r##"let calls = 0; function mk(): any { calls++; return { z: 1 }; }
function main() {
  const o: any = {};
  const log: string[] = [];
  const r = Object.defineProperty(mk(), "x", { value: 7 });
  console.log(calls, r.x, r.z);
  Object.defineProperty(o, "y", { value: (log.push("v"), 5), enumerable: (log.push("e"), false) });
  console.log(log.join(","), o.y);
  const q: any = { x: 1 };
  Object.defineProperty(q, "x", { writable: false });
  console.log(q.x);
  console.log(Reflect.defineProperty(mk(), "w", { value: 2 }), calls);
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k60b"), "1 7 1\nv,e 5\n1\ntrue 2\n");
}

#[test]
fn i_contract_k60c() {
    let source = r##"let calls = 0; function mk() { calls++; return { x: 1 }; }
function main() {
  const r = Object.defineProperty(mk(), "x", { value: 7 });
  console.log(calls, r.x);
  console.log(Reflect.defineProperty(mk(), "x", { value: 8 }), calls);
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k60c"), "1 7\ntrue 2\n");
}

#[test]
fn i_contract_k61() {
    let source = r##"function main() { const o = {}; Object.preventExtensions(o); console.log(Object.isSealed(o), Object.isFrozen(o)); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k61"), "true true\n");
}

#[test]
fn i_contract_k67() {
    let source = r##"function main() { const p: any = { inherited: 1 }; const o: any = Object.create(p); o.own = 2; const ks: string[] = []; for (const k in o) ks.push(k); console.log(ks.join(",")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k67"), "own,inherited\n");
}

#[test]
fn i_contract_k68() {
    let source = r##"function main() { const o: any = { a: 1, b: 2, c: 3 }; const ks: string[] = []; for (const k in o) { ks.push(k); delete o.b; } console.log(ks.join(",")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k68"), "a,c\n");
}

#[test]
fn i_contract_k69() {
    let source = r##"function main() { const o = { "2": 0, "1": 0, a: 1, get g() { return 1; } }; const ks: string[] = []; for (const k in o) ks.push(k); console.log(ks.join(",")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k69"), "1,2,a,g\n");
}

#[test]
fn i_contract_k72() {
    let source = r##"function main() { const value: any = 42; try { for (const x of value) {} console.log("no throw"); } catch (e) { console.log("threw", (e as any).name); } const s: any = "ab"; const out: string[] = []; for (const c of s) out.push(c); console.log(out.join("|")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k72"), "threw TypeError\na|b\n");
}

#[test]
fn i_contract_k75() {
    let source = r##"class Range { *[Symbol.iterator]() { yield 1; yield 2; } }
function makeRange() { return new Range(); }
function main() { const o: number[] = []; for (const x of new Range()) o.push(x); for (const x of makeRange()) o.push(x * 10); console.log(o.join(",")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k75"), "1,2,10,20\n");
}

#[test]
fn i_contract_k70() {
    let source = r##"function f() { var x = 1; const read = () => x; var x = 2; return read(); }
function main() { console.log(f()); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k70"), "2\n");
}

#[test]
fn i_contract_k71b() {
    let source = r##"function main() {
  console.log(helper(2));
  function helper(n: number): number { return n * twice(n); }
  function twice(n: number): number { return n + n; }
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k71b"), "8\n");
}

#[test]
fn i_contract_k83() {
    let source = r##"function f() { var value = 0; for (var value of [1, 2]) {} return value; }
function main() { console.log(f()); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k83"), "2\n");
}

#[test]
fn i_contract_k38() {
    let source = r##"function main() { console.log(JSON.parse('"2"') < JSON.parse('"10"')); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k38"), "false\n");
}

#[test]
fn i_contract_k38b() {
    let source = r##"function main() {
  const a: any = JSON.parse('"2"'), b: any = JSON.parse('"10"'), n: any = JSON.parse('10'), m: any = JSON.parse('2');
  console.log(a < b, a > b, a <= b, a >= b, n > m, a < n, "10" < a, a < "10", m < "10");
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k38b"), "false true false true true true true false true\n");
}

#[test]
fn i_contract_k39() {
    let source = r##"function main() { console.log(JSON.parse('"1"') == JSON.parse('1')); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k39"), "true\n");
}

#[test]
fn i_contract_k39b() {
    let source = r##"function p(s: string): any { return JSON.parse(s); }
function main() {
  console.log(p('"1"') == p('1'), p('1') == p('"1"'), p('1') == p('1'), p('"a"') == p('"a"'), p('"a"') == p('"b"'));
  console.log(p('null') == p('null'), p('true') == p('1'), p('false') == p('0'), p('"0"') == p('false'), p('2') == p('"x"'));
  console.log(p('{"a":1}') == p('{"a":1}'), p('[1]') == p('[1]'), p('null') != p('1'), p('"1"') != p('1'));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k39b"), "true true true true false\ntrue true true true false\nfalse false true false\n");
}

#[test]
fn i_contract_k40() {
    let source = r##"function main() {
  loadScript("globalThis.lf = function () { return 1; }; globalThis.lo = { valueOf() { return 7; }, toJSON() { return 99; } }; globalThis.lz = 0; globalThis.le = ''; globalThis.ls = 'x'; globalThis.lq = {};");
  const f: JsValue = getDynamicValue("lf");
  const o: JsValue = getDynamicValue("lo");
  const z: JsValue = getDynamicValue("lz");
  const e: JsValue = getDynamicValue("le");
  const s: JsValue = getDynamicValue("ls");
  const q: JsValue = getDynamicValue("lq");
  console.log(f ? "t" : "f", z ? "t" : "f", e ? "t" : "f", s ? "t" : "f", q ? "t" : "f", !!f);
  console.log(Number(o), Math.abs(o as any), +(o as any));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k40"), "t f f t t true\n7 7 7\n");
}

#[test]
fn i_contract_k74a() {
    let source = r##"const x = 99;
function f({ x }: { x: number }, y: number = x): number { return y; }
function main() { console.log(f({ x: 1 })); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k74a"), "1\n");
}

#[test]
fn i_contract_k74b() {
    let source = r##"const x = 99;
function f({ x }: { x: number }, y: number = x): number { return y; }
function g([a, b]: number[], c: number = a + b, d: number = c * 2): number { return d; }
function h({ x }: { x: number } = { x: 5 }, y: number = x): number { return y; }
function main() { console.log(f({ x: 1 }), g([2, 3]), g([2, 3], 1), h(), h({ x: 8 })); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k74b"), "1 10 2 5 8\n");
}

#[test]
fn i_contract_k45() {
    let source = r##"function main() { try { console.log("abc".includes(/a/ as any)); } catch (e) { console.log("threw", (e as any).name); } }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k45"), "threw TypeError\n");
}

#[test]
fn i_contract_k45b() {
    let source = r##"function t(f: () => any) { try { return String(f()); } catch (e) { return "threw " + (e as any).name; } }
function main() {
  const r = /a/;
  console.log(t(() => "abc".includes(/a/ as any)), t(() => "abc".startsWith(r as any)), t(() => "abc".endsWith(r as any)), t(() => "abc".includes("b")), t(() => "abc".indexOf("c")));
  console.log(t(() => [r].includes(r)));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k45b"), "threw TypeError threw TypeError threw TypeError true 2\ntrue\n");
}

#[test]
fn i_contract_k55() {
    let source = r##"function main() { const child: any = Object.create({ inherited: 1 }); console.log(Reflect.has(child, "inherited")); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k55"), "true\n");
}

#[test]
fn i_contract_k55b() {
    let source = r##"function main() {
  const p: any = { inherited: 1 };
  const child: any = Object.create(p); child.own = 2;
  const plain: any = { a: 1 };
  console.log(Reflect.has(child, "inherited"), Reflect.has(child, "own"), Reflect.has(child, "nope"), Reflect.has(plain, "a"));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k55b"), "true true false true\n");
}

#[test]
fn i_contract_k54() {
    let source = r##"function main() { const parent: any = { inh: 1 }; const child: any = {}; console.log(Reflect.setPrototypeOf(child, parent), child.inh); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k54"), "true 1\n");
}

#[test]
fn i_contract_k54b() {
    let source = r##"function main() {
  const parent: any = { inh: 1 };
  const child: any = {}; child.own = 2;
  console.log(Reflect.setPrototypeOf(child, parent), child.inh, child.own);
  const frozen: any = {}; Object.preventExtensions(frozen);
  console.log(Reflect.setPrototypeOf(frozen, parent), frozen.inh);
  const cleared: any = Object.create(parent);
  console.log(cleared.inh, Reflect.setPrototypeOf(cleared, null), cleared.inh);
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k54b"), "true 1 2\nfalse undefined\n1 true undefined\n");
}

#[test]
fn i_contract_k52() {
    let source = r##"function main() { const p: any = { x: 1 }; const child: Record<string, number> = Object.create(p); console.log(Object.getPrototypeOf(child) === p); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k52"), "true\n");
}

#[test]
fn i_contract_k19() {
    let source = r##"function main() { const o: any = { x: 1 }; Object.seal(o); const d1 = Object.getOwnPropertyDescriptor(o, "x") as any; const f: any = { y: 1 }; Object.freeze(f); const d2 = Object.getOwnPropertyDescriptor(f, "y") as any; console.log(d1.configurable, d1.writable, d2.configurable, d2.writable); }
"##;
    assert_eq!(compile_and_run(source, "i_contract_k19"), "false true false false\n");
}

#[test]
fn i_contract_k19c() {
    let source = r##"function main() {
  const n: any = { z: 1 }; const d3 = Object.getOwnPropertyDescriptor(n, "z") as any;
  const s: any = { y: 1 }; Object.seal(s); const d4 = Object.getOwnPropertyDescriptor(s, "y") as any;
  console.log(d3.configurable, d3.writable, d3.enumerable, d4.configurable, d4.writable, Object.getOwnPropertyDescriptor(s, "missing"));
}
"##;
    assert_eq!(compile_and_run(source, "i_contract_k19c"), "true true true false true undefined\n");
}
