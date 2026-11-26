// Stack-trace fidelity: the sequence of frame names in `Error.stack` for errors raised through shapes the
// optimiser likes to rewrite (tail calls, inlining, recursion, callbacks, async). Expected output is real
// Node 22 output (frames outside user code cut off by `names`). Runs under THAW_TEST_DIFF_MIDEND too.

#[test]
fn stack_fidelity_tail() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function c(): Error { return new Error("x"); }
function b(): Error { return c(); }
function a(): Error { return b(); }
function main(): void { console.log(names(a())); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_tail"),
        "at c|at b|at a|at main\n"
    );
}

#[test]
fn stack_fidelity_inline() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function leaf(): Error { return new Error("x"); }
function h1(x: number): Error { return leaf(); }
function h2(x: number): Error { const e = h1(x + 1); return e; }
function main(): void { console.log(names(h2(1))); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_inline"),
        "at leaf|at h1|at h2|at main\n"
    );
}

#[test]
fn stack_fidelity_recur() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function rec(n: number): Error { if (n === 0) return new Error("x"); return rec(n - 1); }
function main(): void { console.log(names(rec(3))); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_recur"),
        "at rec|at rec|at rec|at rec|at main\n"
    );
}

#[test]
fn stack_fidelity_mutual() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function ev(n: number): Error { if (n === 0) return new Error("x"); return od(n - 1); }
function od(n: number): Error { if (n === 0) return new Error("y"); return ev(n - 1); }
function main(): void { console.log(names(ev(3))); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_mutual"),
        "at od|at ev|at od|at ev|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: class getter/static/constructor frames differ (new K, get g, K.s)"]
fn stack_fidelity_class() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
class K {
  constructor() { this.make(); }
  make(): void { console.log(names(new Error("c"))); }
  static s(): Error { return new Error("s"); }
  get g(): Error { return new Error("g"); }
  m(): Error { return new Error("m"); }
}
function main(): void {
  const k = new K();
  console.log(names(K.s()));
  console.log(names(k.g));
  console.log(names(k.m()));
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_class"),
        "at K.make|at new K|at main\nat K.s|at main\nat get g|at main\nat K.m|at main\n"
    );
}

#[test]
fn stack_fidelity_arrow() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
const f = (): Error => new Error("x");
const g = (): Error => f();
function main(): void { console.log(names(g())); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_arrow"),
        "at f|at g|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: anonymous arrows and named function-expression (iife) frames are not in the stack section"]
fn stack_fidelity_closure() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function mk(): () => Error { const z = 1; return () => new Error("x" + z); }
function main(): void {
  const e = mk()();
  console.log(names(e));
  const e2 = (function iife(): Error { return new Error("i"); })();
  console.log(names(e2));
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_closure"),
        "at <anon>|at main\nat iife|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: callback frames from Array.map/forEach/sort missing or wrong"]
fn stack_fidelity_callbacks() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function cb(x: number): Error { return new Error("x"); }
function main(): void {
  let m: Error[] = [1].map((x: number) => cb(x));
  console.log(names(m[0]));
  let f: string = "";
  [1].forEach((x: number) => { f = names(cb(x)); });
  console.log(f);
  let s: string = "";
  [2, 1].sort((a: number, b: number) => { if (!s) s = names(cb(a)); return a - b; });
  console.log(s);
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_callbacks"),
        "at cb|at <anon>|at Array.map|at main\nat cb|at <anon>|at Array.forEach|at main\nat cb|at <anon>|at Array.sort|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: async stack: no `async` caller frames and missing frames after await"]
fn stack_fidelity_async() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
async function inner(): Promise<Error> { await Promise.resolve(); return new Error("x"); }
async function outer(): Promise<Error> { const e = await inner(); return e; }
async function sync1(): Promise<Error> { return new Error("s"); }
async function main(): Promise<void> {
  console.log(names(await outer()));
  console.log(names(await sync1()));
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_async"),
        "at inner|at async outer|at async main\nat sync1|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: generator frames (gen, gen.next) missing"]
fn stack_fidelity_generator() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function* gen(): Generator<Error> { yield new Error("x"); }
function drive(): Error { return gen().next().value; }
function main(): void { console.log(names(drive())); }
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_generator"),
        "at gen|at gen.next|at drive|at main\n"
    );
}

#[test]
#[ignore = "known gap vs Node: promise .then callback frames missing after the first"]
fn stack_fidelity_then() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function mk(): Error { return new Error("x"); }
function main(): void {
  Promise.resolve(1).then(() => mk()).then((e: Error) => { console.log(names(e)); });
  Promise.resolve(2).then(() => { console.log(names(mk())); });
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_then"),
        "at mk|at <anon>\nat mk|at <anon>\n"
    );
}

#[test]
fn stack_fidelity_timeout() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function mk(): Error { return new Error("x"); }
function main(): void {
  setTimeout(() => { console.log(names(mk())); }, 1);
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_timeout"),
        "at mk\n"
    );
}

#[test]
fn stack_fidelity_capture() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function cap(): any { const o: any = {}; Error.captureStackTrace(o); return o; }
function mid(): any { return cap(); }
function viaFn(): any { const o: any = {}; Error.captureStackTrace(o, viaFn); return o; }
function outer2(): any { return viaFn(); }
function main(): void {
  console.log(names(mid()));
  console.log(names(outer2()));
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_capture"),
        "at cap|at mid|at main\nat outer2|at main\n"
    );
}

#[test]
fn stack_fidelity_limit() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function r(n: number): Error { if (n === 0) return new Error("x"); return r(n - 1) ; }
function main(): void {
  Error.stackTraceLimit = 3;
  console.log(names(r(10)));
  Error.stackTraceLimit = 0;
  console.log(String(r(2).stack));
  Error.stackTraceLimit = 10;
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_limit"),
        "at r|at r|at r\nError: x\n"
    );
}

#[test]
#[ignore = "known gap vs Node: the `cause` of a caught error has no frames (its text copy loses the side table)"]
fn stack_fidelity_rethrow() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function thrower(): void { throw new Error("x"); }
function mid(): void { try { thrower(); } catch (e) { throw e; } }
function wrap(): void { try { thrower(); } catch (e) { throw new Error("w", { cause: e }); } }
function main(): void {
  try { mid(); } catch (e: any) { console.log(names(e)); }
  try { wrap(); } catch (e: any) { console.log(names(e)); console.log(names(e.cause)); }
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_rethrow"),
        "at thrower|at mid|at main\nat wrap|at main\nat thrower|at wrap|at main\n"
    );
}

#[test]
fn stack_fidelity_deep() {
    let source = r#"function names(e: any): string {
  const out: string[] = [];
  for (const raw of String(e.stack).split("\n").slice(1)) {
    const l = raw.trim().split(" (")[0];
    if (l.startsWith("at Object.<anonymous>") || l.startsWith("at Timeout.") || l.startsWith("at listOnTimeout")) break;
    out.push(l.includes("/") ? "at <anon>" : l);
    if (l === "at main" || l === "at async main") break;
  }
  return out.join("|");
}
function down(n: number): number { return down(n + 1) + 1; }
function main(): void {
  try { down(0); } catch (e: any) {
    console.log(e instanceof RangeError, e.message);
    const lines = String(e.stack).split("\n");
    console.log(lines[0], lines.length > 5, lines[1].trim().split(" (")[0]);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_fidelity_deep"),
        "true Maximum call stack size exceeded\nRangeError: Maximum call stack size exceeded true at down\n"
    );
}

#[test]
fn stack_overflow_is_a_catchable_range_error_in_lambdas_async_and_methods() {
    let source = r#"const f = (n: number): number => f(n + 1) + 1;
class K { m(n: number): number { return this.m(n + 1) + 1; } }
async function g(n: number): Promise<number> { return (await g(n + 1)) + 1; }
async function main(): Promise<void> {
  try { f(0); } catch (e: any) { console.log("arrow", e instanceof RangeError, e.message); }
  try { new K().m(0); } catch (e: any) { console.log("method", e instanceof RangeError); }
  try { await g(0); } catch (e: any) { console.log("async", e instanceof RangeError); }
  console.log("after");
}
"#;
    assert_eq!(
        compile_and_run(source, "stack_overflow_lambda_async"),
        "arrow true Maximum call stack size exceeded\nmethod true\nasync true\nafter\n"
    );
}
