// Out-of-range reads of typed arrays are `undefined` (NaN in numeric positions): expected output from node 22.

#[test]
fn oobread_contract_numbers_observed() {
    let source = r#"function firstOr(xs: number[], d: number): number { const v = xs[0]; return v === undefined ? d : v; }
function firstOr2(xs: number[], d: number): number { const v: number | undefined = xs[0]; return v === undefined ? d : v; }
function pick(a: number[], k: number): number | undefined { const v = a[k]; return v; }
function ret(a: number[]): number | undefined { return a[3]; }
function main(): void {
  console.log(firstOr([], 9), firstOr([4], 9), firstOr2([], 9));
  const xs: number[] = [3, 1, 2];
  const i = 7;
  console.log(xs[5], xs[i], xs[-1], xs[1.5], xs[NaN], xs[xs.length]);
  console.log(xs.at(5), xs.at(-1), xs.at(-9));
  console.log(typeof xs[5], typeof xs[1], `${xs[5]} ${xs[0]}`);
  console.log(xs[5] ?? 42, xs[5] === undefined, xs[0] === undefined, xs[5] == null, xs[5] !== undefined);
  const [a, , , d] = xs;
  console.log(a, d);
  const w = xs[9];
  console.log(w);
  let z = xs[9];
  if (z === undefined) console.log("z undef");
  z = xs[0];
  console.log(z);
  const u: number | undefined = xs[9];
  console.log(u, xs[9]?.toFixed(1), xs[0]?.toFixed(1));
  console.log([xs[9], xs[0]], JSON.stringify({ a: xs[9], b: xs[0] }), JSON.stringify([xs[9]]));
  console.log(xs[9] ? "t" : "f", !xs[9], xs[9] || 7, xs[9] && 7);
  console.log(pick(xs, 1), pick(xs, 8), ret(xs), ret([1, 2, 3, 4]));
  const found = xs.indexOf(99);
  console.log(xs[found], xs[found] === undefined);
}
"#;
    assert_eq!(
        compile_and_run(source, "oobread_contract_numbers_observed"),
        "9 4 9\nundefined undefined undefined undefined undefined undefined\nundefined 2 undefined\nundefined number undefined 3\n42 true false true false\n3 undefined\nundefined\nz undef\n3\nundefined undefined 3.0\n[ undefined, 3 ] {\"b\":3} [null]\nf true 7 undefined\n1 undefined undefined 4\nundefined true\n"
    );
}

#[test]
fn oobread_contract_numbers_arithmetic() {
    let source = r#"function sumTo(a: number[], n: number): number { let t = 0; for (let k = 0; k < n; k++) t += a[k]; return t; }
function main(): void {
  const xs: number[] = [3, 1, 2];
  console.log(xs[5] + 1, xs[5] * 2, xs[5] < 1, xs[5] > 1, xs[5] >= 0, xs[5] <= 0);
  console.log(xs[0] + xs[1] * xs[2], xs[4] - 1, -xs[4], xs[4] ** 2, xs[4] % 2, Math.floor(xs[4]), Math.max(xs[9], 1));
  let s = 0;
  for (let k = 0; k < 5; k++) s += xs[k];
  console.log(s, sumTo(xs, 3), sumTo(xs, 5));
  let acc = 0; acc += xs[9];
  console.log(acc, isNaN(xs[9]), Number(xs[9]), +xs[9], xs[9] | 0, ~xs[9], xs[9] >>> 0);
  const x = xs[10];
  console.log(x + 1, `${xs[9] + 1}`, xs[9] + "", "" + xs[9]);
  const diffs: number[] = [];
  for (let k = 0; k < xs.length; k++) diffs.push(xs[k + 1] - xs[k]);
  console.log(diffs, xs.map((v, k) => xs[k + 1]));
  let cnt = 0; let j = 0;
  while (xs[j] !== undefined) { cnt++; j++; }
  let m = 0;
  for (let k = 0; k < 10; k++) { const v = xs[k]; if (v === undefined) { m++; } }
  console.log(cnt, m);
  const out: number[] = [];
  for (let k = 0; k < 5; k++) out.push(xs[k] ?? -1);
  console.log(out);
  const cache: number[] = [];
  function fib(n: number): number { if (n < 2) return n; const c = cache[n]; if (c !== undefined) return c; const r = fib(n - 1) + fib(n - 2); cache[n] = r; return r; }
  console.log(fib(20));
}
"#;
    assert_eq!(
        compile_and_run(source, "oobread_contract_numbers_arithmetic"),
        "NaN NaN false false false false\n5 NaN NaN NaN NaN NaN NaN\nNaN 6 NaN\nNaN true NaN NaN 0 -1 0\nNaN NaN undefined undefined\n[ -2, 1, NaN ] [ 1, 2, undefined ]\n3 7\n[ 3, 1, 2, -1, -1 ]\n6765\n"
    );
}

#[test]
fn oobread_contract_other_elements() {
    let source = r#"function f(a: string[]): string { const x = a[0]; return x === undefined ? "none" : x; }
function g(a: boolean[]): string { const x = a[0]; if (x === undefined) return "none"; return String(x); }
class Stack extends Array<number> {}
class Box {
  items: number[] = [1, 2];
  get(k: number): number | undefined { return this.items[k]; }
  has(k: number): boolean { return this.items[k] !== undefined; }
}
function main(): void {
  const ss: string[] = ["a", "b"];
  console.log(ss[5], ss[0], typeof ss[5], ss[5] === undefined, ss[5] ?? "dflt", `${ss[5]}`, ss[5] + "x", ss[5]?.length);
  const bs: boolean[] = [true, false];
  console.log(bs[5], bs[0], typeof bs[5], bs[5] === undefined, bs[5] ?? false);
  const os: { a: number }[] = [{ a: 1 }, { a: 2 }];
  console.log(os[5], typeof os[5], os[5] === undefined, os[5]?.a, os[5]?.a ?? 3);
  try { console.log(os[5].a); } catch (err) { console.log((err as Error).constructor.name); }
  const nn: number[][] = [[1], [2, 3]];
  console.log(nn[5], nn[0][3], nn[1][5], nn[5] === undefined, nn[5]?.length);
  const mm: (number | undefined)[] = [1, undefined, 3];
  console.log(mm[1], mm[5], mm[0], mm[1] === undefined, mm[5] === undefined, mm[5] ?? 0, mm[1] ?? 0);
  const sparse: number[] = []; sparse.length = 2;
  const explicit: (number | undefined)[] = [undefined];
  console.log(sparse[0], sparse[0] === undefined, explicit[0], explicit[0] === undefined);
  const subclass = new Stack(); subclass.push(7);
  console.log(subclass[4], subclass[4] === undefined, subclass[0]);
  const anyArray: any = [5];
  console.log(anyArray[4], anyArray[4] === undefined, anyArray[0]);
  const bytes = new Uint8Array([1, 2, 3]);
  console.log(bytes[5], bytes[1], typeof bytes[5], bytes[5] === undefined, bytes[5] ?? 9, bytes[1] ?? 9);
  const buf = Buffer.from("abc");
  console.log(buf[9], buf[0], buf[9] ?? 8);
  const sv = ss[5]; console.log(sv === undefined ? "U" : sv);
  const bv = bs[5]; console.log(bv === undefined ? "U" : String(bv));
  const ov = os[5]; console.log(ov === undefined ? "U" : ov.a);
  console.log(f([]), f(["z"]), g([]), g([false]));
  const t: [number, string] = [1, "a"];
  console.log(t[0], t[1]);
  const bx = new Box();
  console.log(bx.get(0), bx.get(5), bx.has(1), bx.has(9));
  const grid: number[][] = [[1, 2], [3]];
  let tot = 0;
  for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) { const v = grid[r]?.[c]; tot += v ?? 0; }
  console.log(tot);
}
"#;
    assert_eq!(
        compile_and_run(source, "oobread_contract_other_elements"),
        "undefined a undefined true dflt undefined undefinedx undefined\nundefined true undefined true false\nundefined undefined true undefined 3\nTypeError\nundefined undefined undefined true undefined\nundefined undefined 1 true true 0 0\nundefined true undefined true\nundefined true 7\nundefined true 5\nundefined 2 undefined true 9 2\nundefined 97 8\nU\nU\nU\nnone z none false\n1 a\n1 undefined true false\n6\n"
    );
}
