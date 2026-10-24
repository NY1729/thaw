// Class values passed through `any`: closure-call arguments and `new k()`.
// Expected output recorded from node 22.

#[test]
fn any_param_of_function_variable_receives_a_class_value() {
    let source = r##"class C { x = 1; }
const show = (K: any) => typeof K;
function f1(): number { return 1; }
function main(): void { console.log(show(f1), show({ a: 1 }), show(5), show(C), show(() => 2), show(Date), show(Map)); const k: any = C; console.log(show(k)); }
"##;
    assert_eq!(
        compile_and_run(source, "any_param_of_function_variable_receives_a_class_value"),
        "function object number function function function function\nfunction\n"
    );
}

#[test]
fn new_through_any_parameter_class_value_is_native_instance() {
    let source = r##"function helper(n: number): number { return n * 2; }
class C { x = 1; dbl(): number { return helper(this.x); } }
const mk = (K: any) => new K();
const id = (K: any) => K;
function main(): void {
  const o = mk(C); const t: C = o; t.x = 3;
  console.log(t.dbl(), o.x, t === o, typeof o);
  const k2 = id(C); const p = new k2(); console.log(p.dbl());
}
"##;
    assert_eq!(
        compile_and_run(source, "new_through_any_parameter_class_value_is_native_instance"),
        "6 3 true object\n2\n"
    );
}

#[test]
fn new_through_any_variable_builds_native_instance() {
    let source = r##"function helper(n: number): number { return n * 2; }
class C { x = 1; dbl(): number { return helper(this.x); } }
function main(): void {
  const k: any = C; const o = new k(); const p = new k();
  console.log(o.dbl(), o instanceof k, o.x, p === o);
  o.x = 7; console.log(o.dbl(), p.x, o.constructor === k);
}
"##;
    assert_eq!(
        compile_and_run(source, "new_through_any_variable_builds_native_instance"),
        "2 true 1 false\n14 1 true\n"
    );
}

#[test]
fn any_class_value_constructor_reaches_native_helpers() {
    let source = r##"function helper(n: number): number { return n * 2; }
class C { x = 1; dbl(): number { return helper(this.x); } }
class D { c: C = new C(); run(): number { return this.c.dbl(); } }
class E extends C { y = 5; sum(): number { return this.dbl() + this.y; } }
function main(): void {
  const k: any = D; console.log(new k().run());
  const e: any = E; const inst = new e(); console.log(inst.sum(), inst instanceof e);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_class_value_constructor_reaches_native_helpers"),
        "2\n7 true\n"
    );
}

#[test]
fn any_class_value_constructor_args_and_throw() {
    let source = r##"class Q { v: number; constructor(v: number) { if (v < 0) throw new RangeError("neg"); this.v = v; } get(): number { return this.v; } }
class In { a = 2; get(): number { return this.a; } }
class W { items: number[]; inner: In; label: string;
  constructor(items: number[], inner: In, label: string) { this.items = items; this.inner = inner; this.label = label; }
  total(): number { return this.items.reduce((s, x) => s + x, 0) + this.inner.get(); }
  add(n: number): void { this.items.push(n); }
}
function main(): void {
  const q: any = Q; console.log(new q(4).get());
  try { new q(-1); } catch (e: any) { console.log("caught", e.message); }
  const k: any = W; const i = new In(); const arr = [1, 2];
  const w = new k(arr, i, "hi"); w.add(4);
  console.log(w.total(), w.label, arr.length, w.items.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_class_value_constructor_args_and_throw"),
        "4\ncaught neg\n9 hi 3 3\n"
    );
}

#[test]
fn any_class_value_with_default_parameter_keeps_working() {
    let source = r##"class P { n: number; tag: string; constructor(n: number, tag: string = "t") { this.n = n; this.tag = tag; } get twice(): number { return this.n * 2; } }
function main(): void {
  const k: any = P; const o = new k(21, "z");
  console.log(o.n, o.tag, o.twice);
  const d = new k(1); console.log(d.tag);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_class_value_with_default_parameter_keeps_working"),
        "21 z 42\nt\n"
    );
}
