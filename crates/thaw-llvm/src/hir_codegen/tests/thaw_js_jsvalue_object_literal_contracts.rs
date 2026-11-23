// A `JsValue` parameter of a thaw-js routed call that receives an object literal (the createHook
// shape) gets the literal sent as Json, with its methods as receiver-callback handle markers, not as
// a handle id (which a members object resolves to nothing). Plain handles keep their marker.

fn thaw_js_jsvalue_literal_program(members: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_{run}(o: JsValue): void;
         function main(): void {{
           __thaw_typed_tjs_{run}({members});
         }}",
        run = hex("nest::run"),
        members = members,
    )
}

/// Any `call` line that calls `symbol` (a declaration of it does not count).
fn ir_calls(ir: &str, symbol: &str) -> bool {
    ir.lines()
        .any(|line| line.contains(" call ") && line.contains(&format!("@{symbol}(")))
}

#[test]
fn thaw_js_jsvalue_object_literal_is_sent_as_json_not_a_handle_id() {
    let source = thaw_js_jsvalue_literal_program("{ init(a: number) {}, n: 1 }");
    let (uses_thaw_js, ir) = thaw_js_routed_ir(&source);
    assert!(uses_thaw_js, "{ir}");
    // the members are built as Json: the method is a receiver callback, passed as a handle marker
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_native_callback"), "{ir}");
    assert!(ir.contains("@thaw_json_object_set_number("), "{ir}");
    // the literal is not converted to a handle id, and the JS engine builder is not needed
    assert!(!ir_calls(&ir, "thaw_json_handle_id"), "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_object_literal_class_method_is_sent_as_json() {
    let new = hex("$new$k::Svc$arity0");
    let take = hex("$methodvoid$k::Svc$take$overload0$arity1");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{take}(receiver: JsValue, o: JsValue): void;
         function main(): void {{
           const svc = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{take}(svc, {{ init(a: number) {{}}, n: 1 }});
         }}"
    );
    let (uses_thaw_js, ir) = thaw_js_routed_ir(&source);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_native_callback"), "{ir}");
    assert!(!ir_calls(&ir, "thaw_json_handle_id"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_plain_handle_argument_keeps_its_marker() {
    let make = hex("nest::make");
    let take = hex("nest::take");
    let source = format!(
        "declare function __thaw_typed_tjs_{make}(v: number): JsValue;
         declare function __thaw_typed_tjs_{take}(o: JsValue): void;
         function main(): void {{
           const box = __thaw_typed_tjs_{make}(1);
           __thaw_typed_tjs_{take}(box);
         }}"
    );
    let (uses_thaw_js, ir) = thaw_js_routed_ir(&source);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_literal_peel_matches_only_the_literal_coercion() {
    let literal = HirExpr::Call(
        Box::new(HirExpr::Var("__thaw_dynamic_object_literal".into())),
        vec![HirExpr::Lit(HirLit::Null)],
    );
    // the coercion `coerce_to_declared` adds for a Json literal in a JsValue slot
    let coerced = HirExpr::JsonAsNative(Box::new(literal.clone()), HirType::JsValue);
    assert!(thaw_js_object_literal_for_jsvalue(&coerced).is_some());
    // the bare literal (a Json slot, no coercion) and other `JsValue` coercions keep today's path
    assert!(thaw_js_object_literal_for_jsvalue(&literal).is_none());
    let json = HirExpr::JsonAsNative(Box::new(literal), HirType::Json);
    assert!(thaw_js_object_literal_for_jsvalue(&json).is_none());
    let other_call = HirExpr::Call(Box::new(HirExpr::Var("make".into())), vec![]);
    let other = HirExpr::JsonAsNative(Box::new(other_call), HirType::JsValue);
    assert!(thaw_js_object_literal_for_jsvalue(&other).is_none());
}

#[test]
fn thaw_js_jsvalue_leaf_inside_an_object_param_crosses_as_a_handle_marker() {
    // zod's `z.object({ name: z.string() })`: a plain object whose field is a live handle, not a bare JsValue param.
    let make = hex("nest::make");
    let object = hex("nest::object");
    let source = format!(
        "declare function __thaw_typed_tjs_{make}(v: number): JsValue;
         declare function __thaw_typed_tjs_{object}(shape: {{ name: JsValue }}): void;
         function main(): void {{
           const field = __thaw_typed_tjs_{make}(1);
           __thaw_typed_tjs_{object}({{ name: field }});
         }}"
    );
    let (uses_thaw_js, ir) = thaw_js_routed_ir(&source);
    assert!(uses_thaw_js, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    // The field is a Host lease inside the container (no in-IR marker any more); the call argument is
    // serialised by the shared tjs stringifier, which re-marks the lease at run time.
    assert!(ir.contains("@thaw_json_from_borrowed_handle("), "{ir}");
    assert!(ir.contains("@thaw_json_stringify_tjs("), "{ir}");
    assert!(!ir.contains("__thaw_tjs_handle__"), "{ir}");
}

#[test]
fn every_thaw_js_json_argument_backend_takes_the_marker_serializer() {
    use crate::hir_codegen::json_argument_stringifier;
    // Plain thaw-js backends re-mark leases; graph backends keep the graph wire; other engines are unchanged.
    for symbol in [
        "thaw_tjs_call_typed_result",
        "thaw_tjs_class_call",
        "thaw_tjs_new_function",
    ] {
        assert_eq!(json_argument_stringifier(symbol), "thaw_json_stringify_tjs", "{symbol}");
    }
    for symbol in ["thaw_tjs_call_method_graph_result", "thaw_napi_call_typed_graph_result"] {
        assert_eq!(json_argument_stringifier(symbol), "thaw_json_graph_encode", "{symbol}");
    }
    for symbol in ["thaw_js_new_function", "thaw_napi_call_with_functions_typed_graph_result"] {
        assert!(json_argument_stringifier(symbol) != "thaw_json_stringify_tjs", "{symbol}");
    }
    assert_eq!(json_argument_stringifier("thaw_js_new_function"), "thaw_json_stringify");
}

// Node 22: a getter literal handed to JS is an accessor property whose getter runs on every read.
#[test]
fn thaw_js_jsvalue_object_literal_getter_is_a_live_accessor_handle() {
    let source = thaw_js_jsvalue_literal_program("{ get x(): number { return 1; }, n: 1 }");
    let (uses_thaw_js, ir) = thaw_js_routed_ir(&source);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("__thaw_dynamic_accessor_object"), "{ir}");
    assert!(!ir.contains("%thaw_js_object_literal"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_object_literal_getter_runs_on_every_read_program() {
    let probe = hex("probe");
    let source = format!(
        r#"declare function __thaw_typed_tjs_{probe}(o: JsValue): string;
function main(): void {{
  loadScript("globalThis.probe = o => {{ const d = Object.getOwnPropertyDescriptor(o, 'x'); return [typeof d.get, d.set === undefined, d.enumerable, o.x, o.x, o.n].join('|'); }};");
  let reads = 0;
  console.log(__thaw_typed_tjs_{probe}({{ get x(): number {{ reads++; return reads; }}, n: 7 }}));
  console.log(reads);
}}
"#
    );
    // Expected output recorded from node v22: the getter is a function, runs once per read.
    assert_eq!(
        compile_and_run(&source, "jsvalue_literal_getter_live"),
        "function|true|true|1|2|7\n2\n"
    );
}

// Narrowing a `number | undefined | null` by loose/strict equality (and `&&`, ternary, early return)
// lets the value cross a typed thaw-js call as a plain number. Expected output recorded from node v22.
#[test]
fn thaw_js_typed_call_accepts_a_value_narrowed_by_nullish_equality_program() {
    let take = hex("take");
    let source = format!(
        r#"declare function __thaw_typed_tjs_{take}(value: number): string;
function loose(v?: number | null): string {{ if (v != undefined) return __thaw_typed_tjs_{take}(v); return "none"; }}
function looseNull(v?: number | null): string {{ if (v == null) return "none"; return __thaw_typed_tjs_{take}(v); }}
function both(v?: number | null): string {{ if (v !== undefined && v !== null) return __thaw_typed_tjs_{take}(v); return "none"; }}
function early(v?: number | null): string {{ if (v === undefined || v === null) return "none"; return __thaw_typed_tjs_{take}(v); }}
function ternary(v?: number | null): string {{ return v != null ? __thaw_typed_tjs_{take}(v) : "none"; }}
function main(): void {{
  loadScript("globalThis.take = v => typeof v + ':' + (v * 2);");
  for (const f of [loose, looseNull, both, early, ternary]) console.log(f(4), f(null), f());
}}
"#
    );
    let line = "number:8 none none\n";
    assert_eq!(compile_and_run(&source, "nullish_equality_typed_call"), line.repeat(5));
}

// Node 22: `{ a: 1 }` is assignable to `Record<string, number>`, so the union parameter accepts it and
// the callee sees the plain object.
#[test]
fn thaw_js_typed_call_accepts_an_object_literal_for_a_record_or_object_union_program() {
    let show = hex("show");
    let source = format!(
        r#"declare function __thaw_typed_tjs_{show}(value: Record<string, number> | {{ tag: string }}): string;
function main(): void {{
  loadScript("globalThis.show = v => JSON.stringify(v);");
  console.log(__thaw_typed_tjs_{show}({{ a: 1, b: 2 }}));
  console.log(__thaw_typed_tjs_{show}({{ tag: "t" }}));
}}
"#
    );
    assert_eq!(
        compile_and_run(&source, "record_or_object_union_literal"),
        "{\"a\":1,\"b\":2}\n{\"tag\":\"t\"}\n"
    );
}
