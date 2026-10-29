// Generators: resume values, yield* delegation, throw(), numeric consumption and generator methods
// as [Symbol.iterator] (catalog G034). Expected output recorded from node 22.

#[test]
fn generators_g034_class_iterator() {
    let source = r####"class C {
  *items() { yield 1; yield 2; }
  [Symbol.iterator]() { return this.items(); }
  wrap() { return this.items(); }
}
class D { *[Symbol.iterator]() { yield "x"; yield "y"; } }
function main(): void {
  console.log([...new C()], Array.from(new C()), [...new C().wrap()]);
  console.log([...new D()]);
  for (const v of new C()) console.log(v + 1);
}
"####;
    assert_eq!(compile_and_run(source, "generators_g034_class_iterator"), r####"[ 1, 2 ] [ 1, 2 ] [ 1, 2 ]
[ 'x', 'y' ]
2
3
"####);
}

#[test]
fn generators_g034_delegation_throw() {
    let source = r####"function* inner() { yield "i"; return "ret"; }
function* d() { yield* [1, 2]; const r: string = yield* inner(); yield r; }
function* g2() { try { yield 1; } catch (e) { console.log("caught", e); yield "after"; } }
function main(): void {
  console.log([...d()]);
  const gt = g2();
  gt.next();
  console.log(gt.throw("boom"));
  console.log(gt.next().done);
}
"####;
    assert_eq!(compile_and_run(source, "generators_g034_delegation_throw"), r####"[ 1, 2, 'i', 'ret' ]
caught boom
{ value: 'after', done: false }
true
"####);
}

#[test]
fn generators_g034_iterate_numeric() {
    let source = r####"function* fib() { let [a, b] = [0, 1]; for (;;) { yield a; [a, b] = [b, a + b]; } }
function* count() { yield 1; yield 2; yield 3; }
function main(): void {
  const out: number[] = [];
  for (const v of fib()) { if (v > 50) break; out.push(v); }
  console.log(out);
  let sum = 0;
  for (const v of count()) { if (v >= 2) sum += v * 2; }
  console.log(sum, [...count()].map((v) => v + 1));
}
"####;
    assert_eq!(compile_and_run(source, "generators_g034_iterate_numeric"), r####"[
  0, 1,  1,  2,  3,
  5, 8, 13, 21, 34
]
10 [ 2, 3, 4 ]
"####);
}

#[test]
fn generators_g034_resume_value() {
    let source = r####"function* g() { const x: number = yield 1; console.log("got", x); try { yield 2; } finally { console.log("cleanup"); } return 3; }
function* acc() { let total = 0; while (total < 10) { const n: number = yield total; total += n; } return total; }
function main(): void {
  const it = g();
  console.log(it.next(), it.next(5), it.return(7), it.next());
  const a = acc();
  a.next();
  console.log(a.next(4).value, a.next(3).value, a.next(5));
}
"####;
    assert_eq!(compile_and_run(source, "generators_g034_resume_value"), r####"got 5
cleanup
{ value: 1, done: false } { value: 2, done: false } { value: 7, done: true } { value: undefined, done: true }
4 7 { value: 12, done: true }
"####);
}
