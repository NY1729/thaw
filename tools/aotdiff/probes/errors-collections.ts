// Differential probes (family: errors-collections). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("map1", () => { const m = new Map<any, number>([[NaN, 1], [0, 2]]); return [m.get(NaN), m.get(-0), m.size]; });
  p("map2", () => { const m = new Map<string, number>(); m.set("b", 1).set("a", 2); m.delete("b"); m.set("b", 3); return [...m.keys()]; });
  p("mapObjKey", () => { const k = {}; const m = new Map<any, number>([[k, 1]]); return [m.get(k), m.get({})]; });
  p("set1", () => [...new Set([3, 1, 3, 2, 1])]);
  p("setOps", () => [...(new Set([1, 2, 3]) as any).union(new Set([4]))]);
  p("weak", () => { const w = new WeakMap<object, number>(); const k = {}; w.set(k, 1); return [w.get(k), w.has({})]; });
  p("errMsg1", () => { null!.x; });
  p("errMsg2", () => { (undefined as any).foo(); });
  p("errMsg3", () => { (id(5) as any)(); });
  p("errMsg4", () => { id({}).a.b.c; });
  p("errMsg5", () => { (id({}) as any).f(); });
  p("errMsg6", () => { new (id(1) as any)(); });
  p("errMsg7", () => { id(Symbol()) + ""; });
  p("errMsg8", () => { BigInt(1.5); });
  p("errMsg9", () => { new Array(-1); });
  p("errMsg10", () => { "x".repeat(-1); });
  p("errMsg11", () => { (1).toFixed(101); });
  p("errMsg12", () => { decodeURIComponent("%"); });
  p("errMsg13", () => { const c = 1; (id(c) as any).x.y; });
  p("errMsg14", () => { let f = (): number => f(); f(); });
  p("errMsg15", () => { 1n + (id(1) as any); });
  p("errMsg16", () => { class A { constructor() { (this as any).x; } } class B extends A { constructor() { this.y = 1; super(); } y = 0; } new B(); });
  p("errMsg17", () => { throw id("str"); });
  p("errMsg18", () => { throw id(null); });
  p("errCause", () => { const e = new Error("a", { cause: "c" }); return [e.cause, e.name, String(e), Object.keys(e)]; });
  p("errCustom", () => { class E extends Error { constructor(m: string) { super(m); this.name = "E"; } } const e = new E("m"); return [String(e), e instanceof Error, e instanceof E, typeof e.stack, e.stack!.split("\n")[0]]; });
  p("errAgg", () => new AggregateError([new Error("x")], "m").errors.length);
  p("errStackShape", () => /^Error: boom\n\s+at /.test(new Error("boom").stack!));
  p("errToString", () => Error.prototype.toString.call({ name: "N", message: "" }));
  p("typedArr", () => { const u = new Uint8Array([255, 256, -1, 1.7]); const f = new Float32Array([0.1]); return [Array.from(u), f[0], new Int8Array([200])[0]]; });
  p("typedArr2", () => { const b = new ArrayBuffer(8); new DataView(b).setFloat64(0, 1.5); return Array.from(new Uint8Array(b)); });
  p("buffer", () => [Buffer.from("héllo").length, Buffer.from("aGk=", "base64").toString(), Buffer.from([1, 2]).toString("hex")]);
  p("globalThis", () => [typeof globalThis, typeof structuredClone, typeof queueMicrotask]);
}
