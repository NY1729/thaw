// Known Node-22 divergences closed in the third "known limits" round. Expected output is real
// node 22 output (`node --experimental-strip-types`).

#[test]
fn lim3_new_property_on_a_caught_class_instance_is_an_ordinary_own_property() {
    let source = r#"class C { a: number = 1; }
function main(): void {
  try { throw new C(); } catch (e: any) {
    e.extra = 5;
    console.log(e.extra, Object.keys(e));
  }
  const r: any = new C();
  r.more = 6;
  r.a = 9;
  console.log(r.more, r.a, Object.keys(r));
}
"#;
    assert_eq!(
        compile_and_run(source, "lim3_new_property_on_a_caught_class_instance"),
        "5 [ 'a', 'extra' ]\n6 9 [ 'a', 'more' ]\n"
    );
}

#[test]
fn lim3_stack_on_a_class_instance_without_a_slot_is_listed_like_node() {
    let source = r#"class Q { v: number = 1; }
function main(): void {
  const q = new Q();
  Error.captureStackTrace(q);
  console.log(Object.keys(q), Object.getOwnPropertyNames(q), JSON.stringify(q), typeof (q as any).stack);
  const p = new Q();
  (p as any).stack = "y";
  console.log(Object.keys(p), Object.getOwnPropertyNames(p));
}
"#;
    assert_eq!(
        compile_and_run(source, "lim3_stack_on_a_class_instance_without_a_slot"),
        "[ 'v' ] [ 'v', 'stack' ] {\"v\":1} string\n[ 'v', 'stack' ] [ 'v', 'stack' ]\n"
    );
}

#[test]
fn lim3_array_subclass_keeps_its_type_through_an_annotated_binding() {
    let source = r#"class MyArr extends Array<number> { tag: string; constructor(n: number) { super(); this.tag = n as any; } }
function show(a: MyArr): void { console.log(a); }
function main(): void {
  const a = new MyArr(3);
  a.push(1);
  const b: MyArr = a;
  console.log(b === a, b.length, b[0]);
  b.push(7);
  console.log(a.length);
  show(a);
}
"#;
    assert_eq!(
        compile_and_run(source, "lim3_array_subclass_annotated_binding"),
        "true 1 1\n2\nMyArr(2) [ 1, 7, tag: 3 ]\n"
    );
}

#[test]
fn lim3_class_fields_infer_bytes_and_empty_function_initializers() {
    let source = r#"class X { x = new Uint8Array(2); y = function () {}; }
function main(): void {
  const o = new X();
  console.log(o.x.length, typeof o.y, o.y());
}
"#;
    assert_eq!(
        compile_and_run(source, "lim3_class_field_inference"),
        "2 function undefined\n"
    );
}

#[test]
fn lim3_json_stringify_of_a_function_is_undefined() {
    let source = r#"function f(): void {}
function main(): void {
  console.log(JSON.stringify(f));
  console.log(JSON.stringify(function () {}));
}
"#;
    assert_eq!(compile_and_run(source, "lim3_json_stringify_function"), "undefined\nundefined\n");
}

#[test]
fn lim3_generic_result_bound_survives_an_opaque_expected_type() {
    let source = r#"declare function __thaw_typed_js_61646444<R extends Date>(d: Date, n: number): R;
declare function __thaw_typed_js_73686f77(d: JsValue): string;
function main(): void {
  loadScript("globalThis.addD = (d, n) => new Date(d.getTime() + n * 86400000); globalThis.show = d => d.toISOString();");
  console.log(__thaw_typed_js_73686f77(__thaw_typed_js_61646444(new Date(0), 1)));
}
"#;
    assert_eq!(
        compile_and_run(source, "lim3_generic_result_bound"),
        "1970-01-02T00:00:00.000Z\n"
    );
}
