// A `JsValue` parameter of a thaw-js routed call that receives an object literal (the createHook
// shape) gets the literal sent as Json, with its methods as receiver-callback handle markers, not as
// a handle id (which a members object resolves to nothing). Plain handles keep their marker, and the
// QuickJS lane keeps its own conversion.

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
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    // the members are built as Json: the method is a receiver callback, passed as a handle marker
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_native_callback"), "{ir}");
    assert!(ir.contains("@thaw_json_object_set_number("), "{ir}");
    // the literal is not converted to a handle id, and the QuickJS builder is not needed
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
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_native_callback"), "{ir}");
    assert!(!ir_calls(&ir, "thaw_json_handle_id"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_object_literal_getter_is_a_live_accessor_handle() {
    // A getter literal is typed as a native object, so it never reaches the `__thaw_dynamic_object_literal`
    // peel (and the Json builder's loud stub is not reached). It crosses as a live accessor object
    // handle, not as a flattened `{}`.
    let source = thaw_js_jsvalue_literal_program("{ get x(): number { return 1; }, n: 1 }");
    let (uses_thaw_js, _, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("__thaw_dynamic_accessor_object"), "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!ir_calls(&ir, "thaw_json_handle_id"), "{ir}");
    assert!(!ir.contains("%thaw_js_object_literal"), "{ir}");
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
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn quickjs_lane_jsvalue_object_literal_keeps_the_native_projection() {
    // Unrouted, the literal is typed as a native object: it is built by the QuickJS-side
    // `__thaw_object_with_native_getters` builder and projected (`thaw_object_register_field_offsets`),
    // then sent as a handle marker. The Json literal path (`%thaw_js_object_literal`) does not apply.
    let source = thaw_js_jsvalue_literal_program("{ init(a: number) {}, n: 1 }");
    let (_, uses_quickjs, ir) = thaw_js_routed_ir(&source, false);
    assert!(uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_object_with_native_getters"), "{ir}");
    assert!(ir_calls(&ir, "thaw_js_get_global"), "{ir}");
    assert!(ir_calls(&ir, "thaw_object_register_field_offsets"), "{ir}");
    assert!(ir.contains("%thaw_js_handle_marker = call"), "{ir}");
    assert!(!ir.contains("%thaw_js_object_literal"), "{ir}");
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
