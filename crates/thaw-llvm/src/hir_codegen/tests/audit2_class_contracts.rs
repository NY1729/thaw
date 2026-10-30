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
