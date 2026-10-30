#[test]
fn audit2_abstract_base_unannotated_method_calls_abstract() {
    let source = r##"abstract class Base { abstract area(): number; d() { return this.area() + 1; } }
class Sq extends Base { area(): number { return 4; } }
function main(): void { console.log(new Sq().d()); const b: Base = new Sq(); console.log(b.d(), b.area()); }
"##;
    assert_eq!(compile_and_run(source, "audit2_abstract_unannotated"), "5\n5 4\n");
}

#[test]
fn audit2_class_instance_through_interface_calls_methods() {
    let source = r##"interface Shape { area(): number; name: string }
class Circle implements Shape { name = "c"; constructor(private r: number) {} area(): number { return 2 * this.r; } }
class Rect implements Shape { name = "r"; constructor(private w: number, private h: number) {} area(): number { return this.w * this.h; } }
interface Counter { n: number; inc(by: number): number }
class C implements Counter { n = 0; inc(by: number): number { this.n += by; return this.n; } }
function show(s: Shape) { return s.name + ":" + s.area(); }
function bump(c: Counter) { c.inc(2); c.n += 1; return c.n; }
function main(): void {
  console.log(show(new Circle(1)), show(new Rect(2, 3)));
  const shapes: Shape[] = [new Circle(2), new Rect(1, 1)];
  console.log(shapes.map((s) => s.area()), shapes[0].area());
  const c = new C();
  console.log(bump(c), c.n);
  const k: Counter = c;
  console.log(k.inc(5), c.n);
  const d = new Circle(3);
  const s: Shape = d;
  console.log(s.name, s.area());
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_class_interface_methods"),
        "c:2 r:6\n[ 4, 1 ] 4\n3 3\n8 8\nc 6\n"
    );
}

#[test]
fn audit2_instanceof_narrows_base_to_subclass() {
    let source = r##"class A { x = 1; }
class B extends A { y = 2; }
class C extends B { z = 3; }
function f(a: A): number { return a instanceof B ? a.y : a.x; }
function g(a: A): string {
  if (a instanceof C) { return "C" + a.z + a.y; }
  else if (a instanceof B) { a.y += 1; return "B" + a.y; }
  return "A" + a.x;
}
function main(): void { console.log(f(new A()), f(new B()), g(new A()), g(new B()), g(new C())); }
"##;
    assert_eq!(compile_and_run(source, "audit2_instanceof_narrow"), "1 2 A1 B3 C32\n");
}

#[test]
fn audit2_class_field_initialisers_infer_collection_and_enum_types() {
    let source = r##"enum State { Idle, Running }
class M {
  private m = new Map<string, number>();
  seen = new Set<string>();
  state = State.Idle;
  set(k: string, v: number) { this.m.set(k, v); this.seen.add(k); }
  size() { return this.m.size + this.seen.size; }
  go() { this.state = State.Running; }
}
function main(): void { const m = new M(); m.set("a", 1); m.go(); console.log(m.size(), m.state); }
"##;
    assert_eq!(compile_and_run(source, "audit2_field_init_infer"), "2 1\n");
}

#[test]
fn audit2_abstract_override_without_return_annotation() {
    let source = r##"abstract class Base { abstract area(): number; d() { return this.area(); } }
class Rect extends Base { constructor(private w: number) { super(); } area() { return this.w * 2; } }
class Sq extends Base { area(): number { return 4; } }
function main(): void { console.log(new Rect(2).d(), new Sq().d()); }
"##;
    assert_eq!(compile_and_run(source, "audit2_abstract_override_infer"), "4 4\n");
}

#[test]
fn audit2_interface_methods_in_array_params_callbacks_and_constrained_generics() {
    let source = r##"interface Shape { area(): number; name: string }
class Circle implements Shape { name = "c"; constructor(private r: number) {} area(): number { return 3 * this.r * this.r; } }
class Rect implements Shape { name = "r"; constructor(private w: number, private h: number) {} area(): number { return this.w * this.h; } }
function largest(xs: Shape[]): Shape { return xs.reduce((a, b) => (a.area() >= b.area() ? a : b)); }
function total(xs: Shape[]): number { let t = 0; for (const x of xs) t += x.area(); return t; }
function areas(xs: Shape[]): number[] { return xs.map((s) => s.area()); }
function biggest<T extends Shape>(xs: T[]): T { let m = xs[0]; for (const x of xs) if (x.area() > m.area()) m = x; return m; }
function one<T extends Shape>(x: T): number { return x.area(); }
function main(): void {
  const shapes: Shape[] = [new Circle(2), new Rect(3, 5), new Circle(1)];
  console.log(largest(shapes).name, total(shapes), areas(shapes));
  console.log(biggest(shapes).name, one(new Rect(2, 2)));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_interface_array_generics"),
        "r 30 [ 12, 15, 3 ]\nr 4\n"
    );
}

#[test]
fn audit2_generic_class_with_generic_method_named_map() {
    let source = r##"class Stack<T> {
  private items: T[] = [];
  push(x: T): this { this.items.push(x); return this; }
  map<U>(f: (x: T) => U): Stack<U> { const s = new Stack<U>(); this.items.forEach((i) => s.push(f(i))); return s; }
  toArray(): T[] { return [...this.items]; }
}
class Box<T> {
  constructor(public v: T) {}
  map<U>(f: (x: T) => U): Box<U> { return new Box<U>(f(this.v)); }
}
function main(): void {
  const st = new Stack<number>().push(1).push(2);
  console.log(st.map((x) => x * 2).toArray(), st.map((x) => "n" + x).toArray());
  const b = new Box<number>(2);
  console.log(b.map((x) => x * 2).v, b.map((x) => "s" + x).v);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_generic_map_method"),
        "[ 2, 4 ] [ 'n1', 'n2' ]\n4 s2\n"
    );
}
