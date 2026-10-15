// Wave F: language-semantics contracts. Expected output recorded from node 22.

#[test]
fn f_contract_c01() {
    let source = r##"class A { #x = 1; y = 2; z = 3; }
function main() { const a = new A(); console.log(JSON.stringify(a), Object.keys(a).join(","), Object.getOwnPropertyNames(a).join(",")); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_c01"), "{\"y\":2,\"z\":3} y,z y,z\n");
}

#[test]
fn f_contract_k01() {
    let source = r##"class A { #x = 1; y = 2; hasX(o: object): boolean { return #x in o; } }
function main() { const a = new A(); console.log(JSON.stringify(a), Object.keys(a).join(","), JSON.stringify({ ...a }), a.hasX(a), a.hasX({})); const j: any = a; console.log(JSON.stringify(j), Object.keys(j).join(",")); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_k01"), "{\"y\":2} y {\"y\":2} true false\n{\"y\":2} y\n");
}

#[test]
fn f_contract_c14() {
    let source = r##"function main() { const o = { m: 10 }; console.log([1].map(function (this: any, x: number) { return x + this.m; }, o).join()); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_c14"), "11\n");
}

#[test]
fn f_contract_c15() {
    let source = r##"function main() { const o = { m: 10 }; [1].forEach(function (this: any) { console.log(this.m); }, o); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_c15"), "10\n");
}

#[test]
fn f_contract_e01() {
    let source = r##"function main() { const o = { m: 10 }; console.log([1].map(function (this: {m: number}, x: number) { return x + this.m; }, o).join()); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_e01"), "11\n");
}

#[test]
fn f_contract_e02() {
    let source = r##"function main() { const o = { m: 10 }; [1].forEach(function (this: {m: number}) { console.log(this.m); }, o); const r = [1,2,3].filter(function (this: {m: number}, x: number) { return x > this.m - 9; }, o); console.log(r.join()); console.log([1,2,3].some(function (this: {m: number}, x: number) { return x > this.m - 8; }, o)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_e02"), "10\n2,3\ntrue\n");
}

#[test]
fn f_contract_g01() {
    let source = r##"function main() { const x: any = null; const y: any = undefined; console.log(Object.is(x, null), Object.is(y, undefined), Object.is(x, y)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_g01"), "true true false\n");
}

#[test]
fn f_contract_g02() {
    let source = r##"function main() { const n: number | null = 5; console.log(Number.isNaN(n as any), Number.isFinite(n as any), Number.isInteger(n as any)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_g02"), "false true true\n");
}

#[test]
fn f_contract_g04() {
    let source = r##"function main() { const n: number | null = 5; console.log(Number.isInteger(n)); const m: any = n; console.log(Number.isInteger(m)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_g04"), "true\ntrue\n");
}

#[test]
fn f_contract_d12() {
    let source = r##"function main() { const x: any = null; const y: any = undefined; console.log(Object.is(x, null), Object.is(y, undefined), Object.is(x, y)); const n: number | null = 5; console.log(Number.isNaN(n as any), Number.isFinite(n as any), Number.isInteger(n as any)); const j: any = "5"; console.log(Number.isFinite(j), Number.isInteger(j)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_d12"), "true true false\nfalse true true\nfalse false\n");
}

#[test]
fn f_contract_c10() {
    let source = r##"function main() { const w = new WeakMap<object, number>(); try { (w as any).getOrInsert(null, 1); console.log("noerr"); } catch (e: any) { console.log(e.name); } try { (w as any).set(null, 1); } catch (e: any) { console.log(e.name); } const ws = new WeakSet<object>(); console.log((ws as any).has(null)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_c10"), "TypeError\nTypeError\nfalse\n");
}

#[test]
fn f_contract_h03() {
    let source = r##"async function main() {
  const p = Promise.resolve(1).then((v: number) => { throw new Error("from ok"); }, (e: any) => console.log("wrongly caught"));
  try { await p; } catch (e: any) { console.log("outer", e.message); }
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h03"), "outer from ok\n");
}

#[test]
fn f_contract_h05() {
    let source = r##"async function main() {
  const p = Promise.resolve(1).then((v: number): number => { throw new Error("from ok"); });
  try { await p; } catch (e: any) { console.log("outer", e.message); }
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h05"), "outer from ok\n");
}

#[test]
fn f_contract_h08() {
    let source = r##"function boom(): number { throw new Error("boom"); }
async function main() {
  const p = Promise.resolve(1).then((v: number) => boom());
  try { await p; console.log("resolved?"); } catch (e: any) { console.log("outer", e.message); }
  const q = Promise.resolve(1).then((v: number) => { const t = 1; throw new Error("x" + t); });
  try { await q; console.log("resolved?"); } catch (e: any) { console.log("outer", e.message); }
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h08"), "outer boom\nouter x1\n");
}

#[test]
fn f_contract_h09() {
    let source = r##"async function main() {
  const p = Promise.resolve(1).then((v: number) => { if (v > 0) throw new Error("c"); });
  try { await p; console.log("resolved?"); } catch (e: any) { console.log("outer", e.message); }
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h09"), "outer c\n");
}

#[test]
fn f_contract_h11() {
    let source = r##"async function main() {
  const p = Promise.resolve(1).then((v: number) => { console.log("cb"); });
  await p; console.log("after");
  const q = Promise.resolve(1).then((v: number) => { throw new Error("q"); });
  try { await q; console.log("after-q"); } catch (e: any) { console.log("caught q"); }
  console.log("end");
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h11"), "cb\nafter\ncaught q\nend\n");
}

#[test]
fn f_contract_h12() {
    let source = r##"async function main() {
  await Promise.resolve(1).then((v: number) => { console.log("cb1"); });
  console.log("after1");
  const p = Promise.resolve(2).then((v: number) => { console.log("cb2"); });
  await p;
  console.log("after2");
  const p3 = Promise.resolve(3).then((v: number) => { console.log("cb3"); return v; });
  await p3;
  console.log("after3");
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h12"), "cb1\nafter1\ncb2\nafter2\ncb3\nafter3\n");
}

#[test]
fn f_contract_h13() {
    let source = r##"async function work(): Promise<void> { await new Promise<void>((r) => setTimeout(r, 5)); console.log("work"); }
async function main() {
  const a = work();
  await a; console.log("after-a");
  const p = Promise.resolve(2).then((v: number) => { console.log("cb2"); });
  const q = p;
  await q; console.log("after-q");
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h13"), "work\nafter-a\ncb2\nafter-q\n");
}

#[test]
fn f_contract_h14() {
    let source = r##"async function bad(): Promise<void> { throw new Error("bad"); }
async function main() {
  try { await bad(); console.log("no"); } catch (e: any) { console.log("c1", e.message); }
  const b = bad();
  try { await b; console.log("no2"); } catch (e: any) { console.log("c2", e.message); }
  const q = Promise.resolve(1).then((v: number) => { throw new Error("q"); });
  try { await q; console.log("no3"); } catch (e: any) { console.log("c3", e.message); }
  try { await Promise.resolve(1).then((v: number) => { throw new Error("r"); }); console.log("no4"); } catch (e: any) { console.log("c4", e.message); }
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_h14"), "c1 bad\nc2 bad\nc3 q\nc4 r\n");
}

#[test]
fn f_contract_i01() {
    let source = r##"function main() { try { (1).toFixed(101); } catch (e: any) { console.log(e.name, e instanceof RangeError); } try { (1.5).toPrecision(0); } catch (e: any) { console.log(e.name); } try { (255).toString(1); } catch (e: any) { console.log(e.name); } try { (1).toExponential(-1); } catch (e: any) { console.log(e.name); } console.log((1.005).toFixed(2), (12).toString(2)); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_i01"), "RangeError true\nRangeError\nRangeError\nRangeError\n1.00 1100\n");
}

#[test]
fn f_contract_i07() {
    let source = r##"function tag(s: any, ...v: any[]) { return s.length; }
function main() { console.log(tag`a${1}b`); }
"##;
    assert_eq!(compile_and_run(source, "f_contract_i07"), "2\n");
}

#[test]
fn f_contract_j01() {
    let source = r##"function main() {
  const it: any = { [Symbol.iterator]() { return { next() { return { done: true, value: undefined }; }, return() { console.log("return called"); return {}; } }; } };
  for (const x of it) { }
  console.log("end");
}
"##;
    assert_eq!(compile_and_run(source, "f_contract_j01"), "end\n");
}

#[test]
fn f_contract_d26() {
    let source = r##"function main() { const it: any = { [Symbol.iterator]() { return { next() { return { done: true, value: undefined }; }, return() { console.log("return called"); return {}; } }; } }; for (const x of it) { } console.log("end"); const it2: any = { [Symbol.iterator]() { return { next() { return { done: false, value: 1 }; }, return() { throw new Error("from return"); } }; } }; try { for (const x of it2) { throw new Error("body"); } } catch (e: any) { console.log(e.message); } }
"##;
    assert_eq!(compile_and_run(source, "f_contract_d26"), "end\nbody\n");
}
