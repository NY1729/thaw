// A base-typed reference to a subclass instance dispatches getters, setters
// and signature-extended methods to the subclass. Expected output from node 22.

#[test]
fn class_accessor_and_extended_signature_overrides_dispatch_virtually() {
    let source = r##"class P { greet(x: string): string { return "P" + x; } }
class Q extends P { greet(x: string, y?: string): string { return "Q" + x + (y ?? ""); } }
class A {
  _n: number = 1;
  get v(): number { return this._n; }
  set v(x: number) { this._n = x; }
  show(): string { return "v=" + this.v; }
  bump(): void { this.v = this.v + 1; }
}
class B extends A {
  get v(): number { return super.v * 10; }
  set v(x: number) { super.v = x + 100; }
}
class C extends B {
  get v(): number { return super.v + 5; }
  set v(x: number) { super.v = x; }
}
function main(): void {
  const pq: P = new Q();
  console.log(pq.greet("a"));
  const items: A[] = [new A(), new B(), new C()];
  for (const it of items) {
    it.v = 2;
    console.log(it.v, it.show());
    it.bump();
    console.log(it.v);
  }
}
"##;
    assert_eq!(
        compile_and_run(source, "class_accessor_and_extended_signature_overrides_dispatch_virtually"),
        "Qa
2 v=2
3
1020 v=1020
11210
1025 v=1025
11265
"
    );
}
