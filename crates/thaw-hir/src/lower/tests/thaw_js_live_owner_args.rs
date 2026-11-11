// Fix A (plan-tj88): named object variables passed to thaw-js extern FUNCTIONS with any/generic slots
// cross as live owner handles (`__thaw_tjs_live_owner_arg`); literals, arrays, Json and concrete
// object parameters keep the by-value path.

fn thaw_js_owner_symbol(backend: &str, symbol: &str) -> String {
    let hex: String = symbol.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("__thaw_typed_{backend}_{hex}")
}

// Lowers `main` with a `source` prelude and passes `argument` to the extern `callee`.
fn thaw_js_owner_main_body(decls: &str, callee: &str, prelude: &str, argument: &str) -> String {
    let source = format!(
        r#"
        {decls}
        function main(): void {{
            {prelude}
            const result = {callee}({argument});
        }}
        "#
    );
    let program = lower(&source);
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main is lowered");
    format!("{:?}", main.body)
}

#[test]
fn thaw_js_function_any_slot_takes_a_live_owner_marker_for_a_named_object() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}(value: Json): Json;");
    let body = thaw_js_owner_main_body(&decls, &identity, "const source = { n: 1 };", "source");
    assert!(body.contains("__thaw_tjs_live_owner_arg"), "{body}");
    assert!(body.contains("__thaw_lookup_native_object"), "{body}");
}

#[test]
fn thaw_js_function_generic_slot_takes_a_live_owner_marker_for_a_method_less_class_instance() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!(
        "class Item {{ value: number = 5; }}\ndeclare function {identity}<T>(value: T): T;"
    );
    let body = thaw_js_owner_main_body(&decls, &identity, "const item = new Item();", "item");
    assert!(body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_function_any_slot_keeps_copying_literals_arrays_numbers_and_json() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}(value: Json): Json;");
    let literal = thaw_js_owner_main_body(&decls, &identity, "", "{ n: 1 }");
    assert!(!literal.contains("__thaw_tjs_live_owner_arg"), "{literal}");
    let array = thaw_js_owner_main_body(&decls, &identity, "const arr = [1, 2];", "arr");
    assert!(!array.contains("__thaw_tjs_live_owner_arg"), "{array}");
    let number = thaw_js_owner_main_body(&decls, &identity, "const n = 5;", "n");
    assert!(!number.contains("__thaw_tjs_live_owner_arg"), "{number}");
    let json = thaw_js_owner_main_body(&decls, &identity, "const j: any = { n: 1 };", "j");
    assert!(!json.contains("__thaw_tjs_live_owner_arg"), "{json}");
}

#[test]
fn thaw_js_function_concrete_object_param_stays_by_value() {
    let identity = thaw_js_owner_symbol("tjs", "k::score");
    let decls = format!("declare function {identity}(value: {{ n: number }}): number;");
    let body = thaw_js_owner_main_body(&decls, &identity, "const source = { n: 1 };", "source");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_method_and_quickjs_symbols_never_get_the_owner_marker() {
    let method = thaw_js_owner_symbol("tjs", "$method$k::Emitter$emit$overload0$arity1");
    let decls = format!("declare function {method}(value: Json): Json;");
    let body = thaw_js_owner_main_body(&decls, &method, "const source = { n: 1 };", "source");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
    let quickjs = thaw_js_owner_symbol("js", "k::identity");
    let decls = format!("declare function {quickjs}(value: Json): Json;");
    let body = thaw_js_owner_main_body(&decls, &quickjs, "const source = { n: 1 };", "source");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

// A variable promoted to `any` is Json-typed storage: its argument is `JsonAsNative(Var, Object)`, which has
// no native owner to project here, so it keeps the by-value copy (no live-owner marker).
#[test]
fn thaw_js_function_any_slot_keeps_promoted_json_variable_by_value() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}(value: Json): Json;");
    let body = thaw_js_owner_main_body(
        &decls,
        &identity,
        "const source = { n: 1 }; const exposed: any = source;",
        "source",
    );
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

// PR1: a storage-promoted object passed to a generic thaw-js extern slot is the Json lease itself, not a
// JsonAsNative native copy, so the package receives the live handle and the result stays Json.
#[test]
fn thaw_js_generic_slot_passes_promoted_json_object_as_lease() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(
        &decls,
        &identity,
        "const source = { n: 1 }; const exposed: any = source;",
        "source",
    );
    assert!(body.contains("params: [Json]"), "{body}");
    assert!(body.contains("ret: Json"), "{body}");
    assert!(body.contains("Let(\"result\", Json"), "{body}");
    assert!(body.contains("[Var(\"source\")]"), "{body}");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_generic_slot_passes_promoted_json_array_as_lease() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(
        &decls,
        &identity,
        "const source = [{ n: 1 }]; const exposed: any = source;",
        "source",
    );
    assert!(body.contains("params: [Json]"), "{body}");
    assert!(body.contains("[Var(\"source\")]"), "{body}");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

// Negative controls: the lease path must not fire for these shapes.
#[test]
fn thaw_js_generic_slot_keeps_non_promoted_named_object_on_owner_marker() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(&decls, &identity, "const source = { n: 1 };", "source");
    assert!(body.contains("__thaw_tjs_live_owner_arg"), "{body}");
    assert!(!body.contains("params: [Json]"), "{body}");
}

#[test]
fn thaw_js_generic_slot_keeps_fresh_literal_by_value() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(&decls, &identity, "", "{ n: 1 }");
    assert!(!body.contains("ret: Json"), "{body}");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_generic_slot_keeps_promoted_primitive_as_number() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!("declare function {identity}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(&decls, &identity, "const source = 5;", "source");
    assert!(!body.contains("params: [Json]"), "{body}");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_concrete_param_does_not_take_promoted_lease() {
    let score = thaw_js_owner_symbol("tjs", "k::score");
    let decls = format!("declare function {score}(value: {{ n: number }}): number;");
    let body = thaw_js_owner_main_body(
        &decls,
        &score,
        "const source = { n: 1 }; const exposed: any = source;",
        "source",
    );
    assert!(!body.contains("params: [Json]"), "{body}");
}

#[test]
fn thaw_js_quickjs_and_method_symbols_do_not_take_promoted_lease() {
    let quickjs = thaw_js_owner_symbol("js", "k::identity");
    let decls = format!("declare function {quickjs}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(
        &decls,
        &quickjs,
        "const source = { n: 1 }; const exposed: any = source;",
        "source",
    );
    // QuickJS keeps its own Json-typed call shape (the lease gate is ThawJs-only).
    assert!(body.contains("backend: QuickJs"), "{body}");
    assert!(!body.contains("__thaw_tjs_live_owner_arg"), "{body}");
    let method = thaw_js_owner_symbol("tjs", "$method$k::Emitter$emit$overload0$arity1");
    let decls = format!("declare function {method}<T>(value: T): T;");
    let body = thaw_js_owner_main_body(
        &decls,
        &method,
        "const source = { n: 1 }; const exposed: any = source;",
        "source",
    );
    assert!(!body.contains("params: [Json]"), "{body}");
}

#[test]
fn thaw_js_any_slot_takes_a_live_owner_marker_for_a_decorated_method_less_class_instance() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!(
        "function Mark(value: JsValue): JsValue {{ return value; }}\n@Mark class Item {{ value: number = 3; }}\ndeclare function {identity}(value: Json): Json;"
    );
    let body = thaw_js_owner_main_body(&decls, &identity, "const item = new Item();", "item");
    assert!(body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

#[test]
fn thaw_js_any_slot_takes_a_live_owner_marker_for_a_decorated_class_with_a_method() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!(
        "function Mark(value: JsValue): JsValue {{ return value; }}\n@Mark class Item {{ value: number = 3; bump(): number {{ return 1; }} }}\ndeclare function {identity}(value: Json): Json;"
    );
    let body = thaw_js_owner_main_body(&decls, &identity, "const item = new Item();", "item");
    assert!(body.contains("__thaw_tjs_live_owner_arg"), "{body}");
}

// tj167: the CLI flattens module-local names with the `__thawmod0_` prefix; the decorator marker and the
// class name must agree on that spelling, and the owner marker must stay a top-level extern call argument.
#[test]
fn thaw_js_any_slot_marker_for_decorated_flattened_class_names() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!(
        "function __thawmod0_Mark(value: JsValue): JsValue {{ return value; }}\n@__thawmod0_Mark class __thawmod0_Item {{ value: number = 3; }}\ndeclare function {identity}(value: any): any;"
    );
    let body = thaw_js_owner_main_body(&decls, &identity, "const item = new __thawmod0_Item();", "item");
    assert!(body.contains("ret: Json }, [Call(Var(\"__thaw_tjs_live_owner_arg\")"), "{body}");
    assert!(!body.contains("__thaw_decorator_instance_json_"), "{body}");
}

#[test]
fn thaw_js_any_slot_marker_for_decorated_flattened_class_with_a_method() {
    let identity = thaw_js_owner_symbol("tjs", "k::identity");
    let decls = format!(
        "function __thawmod0_Mark(value: JsValue): JsValue {{ return value; }}\n@__thawmod0_Mark class __thawmod0_Item {{ value: number = 3; bump(): number {{ return 1; }} }}\ndeclare function {identity}(value: any): any;"
    );
    let body = thaw_js_owner_main_body(&decls, &identity, "const item = new __thawmod0_Item();", "item");
    assert!(body.contains("ret: Json }, [Call(Var(\"__thaw_tjs_live_owner_arg\")"), "{body}");
    assert!(!body.contains("__thaw_decorator_instance_json_"), "{body}");
}
