// Differential probes (family: class-closure). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("classBasic", () => { const q = new Q(); q.dbl = 10; return [q.x, q.dbl, String(q), q instanceof P, Object.keys(q)]; });
  p("classAny", () => { const a: any = new Q(); a.dbl = 4; return [a.x, a.dbl, a.constructor.name, Object.getPrototypeOf(a) === Q.prototype]; });
  p("classJson", () => JSON.stringify(new Q()));
  p("classSpread", () => ({ ...new Q() }));
  p("classObjAssign", () => Object.assign({}, new Q()));
  p("classProto", () => Object.getOwnPropertyNames(P.prototype));
  p("classStatic", () => { class S { static n = 0; static inc(): number { return ++S.n; } } S.inc(); return S.inc(); });
  p("classPriv", () => { class C { #v = 3; get v(): number { return this.#v; } static has(o: any): boolean { return #v in o; } } return [new C().v, C.has(new C()), C.has({})]; });
  p("classToStr", () => Object.prototype.toString.call(new Q()));
  p("instOf", () => [id([]) instanceof Array, id(new Map()) instanceof Object, id(Object.create(null)) instanceof Object, id(1) instanceof Number]);
  p("fnLen", () => [((a: number, b?: number) => a).length, (function (...r: number[]) {}).length, id(Math.max).length]);
  p("fnName", () => { const f = () => 1; const o = { m() {}, ["c" + 1]: () => 2 }; return [f.name, o.m.name, o.c1.name]; });
  p("fnToString", () => String(function foo(a: number) { return a; }));
  p("closure", () => { const fs: Array<() => number> = []; for (let i = 0; i < 3; i++) fs.push(() => i); return fs.map(f => f()); });
  p("closureVar", () => { const fs: any[] = []; for (var i = 0; i < 3; i++) fs.push(() => i); return fs.map(f => f()); });
  p("cbAny", () => [1, 2, 3].map(id as any));
  p("cbParseInt", () => ["1", "2", "3"].map(parseInt));
  p("cbThis", () => { const o = { v: 1, f() { return this?.v; } }; const g = o.f; return [o.f(), g()]; });
  p("bind", () => { const f = function (this: any, a: number, b: number) { return this.k + a + b; }; return f.bind({ k: 1 }, 2)(3); });
  p("gen", () => { function* g() { const x = yield 1; yield x * 2; return 7; } const it = g(); return [it.next().value, it.next(5).value, it.next().value, it.next().done]; });
  p("genReturn", () => { function* g() { try { yield 1; } finally { yield 99; } } const it = g(); it.next(); return [it.return(5).value, it.next().value, it.next().done]; });
  p("genSpread", () => { function* g() { yield* [1, 2]; yield* "ab"; } return [...g()]; });
  p("iterProto", () => typeof (id([]) as any)[Symbol.iterator]);
  p("getset", () => { const o = { _v: 1, get v() { return this._v; }, set v(x: number) { this._v = x * 2; } }; o.v = 5; return [o.v, Object.keys(o)]; });
  p("defineProp", () => { const o: any = {}; Object.defineProperty(o, "x", { value: 1, enumerable: false }); o.x = 2; return [o.x, Object.keys(o), JSON.stringify(o)]; });
  p("frozen", () => { "use strict"; const o = Object.freeze({ a: 1 }); try { (o as any).a = 2; } catch (e: any) { return e.constructor.name; } return "no"; });
  p("proxyGet", () => { const pr = new Proxy({ a: 1 } as any, { get: (t, k) => k in t ? t[k] : "dflt" }); return [pr.a, pr.zz]; });
  p("proxyHas", () => { const pr = new Proxy({}, { has: () => true }); return "x" in pr; });
  p("reflect", () => Reflect.ownKeys({ b: 1, [Symbol.for("s")]: 2, 1: 0 }).map(String));
  p("symbol", () => [Symbol("a").toString(), Symbol("a").description, Symbol.for("a") === Symbol.for("a")]);
}
