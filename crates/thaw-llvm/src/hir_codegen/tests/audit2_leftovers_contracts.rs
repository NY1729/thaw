// audit2 leftovers; expected output from node 22.

#[test]
fn arguments_length_counts_passed_arguments() {
    let source = r####"function f(a?: number, b?: number): number { return arguments.length; }
function h(a: number, b: number): string { return `${arguments.length}:${arguments[0]}`; }
function s(): number { let t = 0; for (let i = 0; i < arguments.length; i++) t += arguments[i]; return t; }
function main(): void {
  console.log(f(), f(1), f(1, 2));
  console.log((h as any)(7, 8, 9), (s as any)(1, 2, 3));
}
"####;
    assert_eq!(compile_and_run(source, "audit2_left_arguments_length_counts_passed_arguments"), r####"0 1 2
3:7 6
"####);
}

#[test]
fn from_entries_number_keyed_map() {
    let source = r####"function main(): void {
  const m = new Map<number, string>([[1, "a"], [2, "b"]]);
  console.log(Object.fromEntries(m));
  const n = new Map<string, number>([["x", 1]]);
  console.log(Object.fromEntries(n));
  console.log(Object.keys(Object.fromEntries(new Map<number, number>([[10, 1], [3, 2]]))));
}
"####;
    assert_eq!(compile_and_run(source, "audit2_left_from_entries_number_keyed_map"), r####"{ '1': 'a', '2': 'b' }
{ x: 1 }
[ '3', '10' ]
"####);
}

#[test]
fn finished_generator_result_prints_value_undefined() {
    let source = r####"function* g(): Generator<number, void, undefined> { yield 1; yield 2; }
function* h(): Generator<number, string, undefined> { yield 1; return "r"; }
function main(): void {
  const it = g();
  console.log(it.next(), it.next(), it.next(), it.next());
  const it3 = h();
  console.log(it3.next(), it3.next(), it3.next());
  const r = g().next();
  if (!r.done) console.log(r.value + 1);
}
"####;
    assert_eq!(compile_and_run(source, "audit2_left_finished_generator_result_prints_value_undefined"), r####"{ value: 1, done: false } { value: 2, done: false } { value: undefined, done: true } { value: undefined, done: true }
{ value: 1, done: false } { value: 'r', done: true } { value: undefined, done: true }
2
"####);
}

#[test]
fn class_field_from_conditional_constructions() {
    let source = r####"class A { x = 1; hi(): string { return "hi" + this.x; } }
class B {
  a = process.argv.length > 0 ? new A() : new A();
  c = process.argv.length > 0 ? [1, 2] : [3];
}
function main(): void {
  const b = new B();
  b.a.x = 7;
  console.log(b.a.hi(), b.c.length, b);
}
"####;
    assert_eq!(compile_and_run(source, "audit2_left_class_field_from_conditional_constructions"), r####"hi7 2 B { a: A { x: 7 }, c: [ 1, 2 ] }
"####);
}

#[test]
fn closure_referencing_later_declared_binding() {
    let source = r####"const f = () => y;
let y = 2;
console.log(f());
function main(): void {
  const k = () => w + y;
  let w = 5;
  console.log(k());
  const p = (): number => q() + 1;
  const q = (): number => 10;
  console.log(p());
}
"####;
    assert_eq!(compile_and_run(source, "audit2_left_closure_referencing_later_declared_binding"), r####"2
7
11
"####);
}
