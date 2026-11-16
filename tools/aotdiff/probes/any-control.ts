// Differential probes (family: any-control). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("anyToNum", () => { const a: any = "5"; const n: number = a; return typeof n; });
  p("anyAdd", () => id("5") + id(1));
  p("anySub", () => id("5") - id(1));
  p("anyEq", () => [id(null) == undefined, id(0) == "", id("0") == false, id([]) == "", id([1]) == 1, id(null) == 0]);
  p("anyStrictEq", () => [id(NaN) === NaN, id(0) === -0, id("a") === "a"]);
  p("anyTruthy", () => [id(0), id(""), id([]), id({}), id(NaN), id(0n), id(null)].map(v => !!v));
  p("anyTypeof", () => [id(null), id(undefined), id(()=>1), id([]), id(1n), id(Symbol.iterator), id(new Date())].map(v => typeof v));
  p("anyUnaryPlus", () => [+id(""), +id(" 12 "), +id("1_0"), +id([]), +id([5]), +id({}), +id(null), +id(undefined), +id(true)]);
  p("anyObjKeys", () => Object.keys(id({ b: 1, a: 2, 1: 3, 0: 4 })));
  p("anyArrHole", () => { const a: any = [1, , 3]; return [a.length, 1 in a, a.indexOf(undefined)]; });
  p("arrLenSet", () => { const a: any = [1, 2, 3]; a.length = 1; a[3] = 4; return JSON.stringify(a) + a.length; });
  p("arrSort", () => [10, 9, 1, undefined, 2].sort() as any);
  p("arrSortNum", () => [10, 9, 1].sort((a, b) => a - b));
  p("arrFlat", () => [1, [2, [3, [4]]]].flat(Infinity));
  p("arrToSorted", () => [3, 1, 2].toSorted());
  p("arrFrom", () => Array.from({ length: 3 }, (_, i) => i * 2));
  p("arrJoinNull", () => [1, null, undefined, 2].join("-"));
  p("arrNeg", () => { const a: any = []; a[-1] = 1; return [a.length, Object.keys(a)]; });
  p("destr", () => { const { a, b: { c = 5 } = {}, ...rest } = id({ a: 1, x: 2, y: 3 }); return [a, c, rest]; });
  p("destrArr", () => { const [a, , b = 9, ...r] = id([1, 2, undefined, 4, 5]); return [a, b, r]; });
  p("destrNull", () => { const { a } = id(null); return a; });
  p("spreadStr", () => [...id("héy")]);
  p("spreadMap", () => [...new Map([[1, 2]])]);
  p("order", () => { const l: string[] = []; const o = { get a() { l.push("a"); return 1; }, get b() { l.push("b"); return 2; } }; const { b, a } = o; return l; });
  p("labeled", () => { let n = 0; o: for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) { if (j == 1) continue o; n++; } return n; });
  p("switchFall", () => { let s = ""; switch (id(2)) { case 1: s += "1"; case 2: s += "2"; case 3: s += "3"; break; default: s += "d"; } return s; });
  p("tryFin", () => { const f = (): number => { try { return 1; } finally { return 2; } }; return f(); });
  p("optChain", () => { const o: any = { a: { b: null } }; return [o?.a?.b?.c, o.x?.(), o.a.b ?? "d", (0 as any) ?? "d", o?.["a"]]; });
  p("logical", () => { let a: any = null; a ??= 5; let b: any = 0; b ||= 7; let c: any = 1; c &&= 9; return [a, b, c]; });
  p("comma", () => (1, 2, 3));
  p("void", () => void 0);
  p("typeofUndecl", () => typeof (globalThis as any).nope);
  p("delete", () => { const o: any = { a: 1 }; return [delete o.a, delete o.zz, "a" in o]; });
  p("inOp", () => ["length" in [], 0 in [1], "toString" in {}]);
  p("sparse", () => Object.keys([, 1, , 2]));
  p("numKeyOrder", () => JSON.stringify({ b: 1, 2: 1, a: 1, 1: 1 }));
}
