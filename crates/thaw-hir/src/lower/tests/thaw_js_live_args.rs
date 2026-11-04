// Native arrays, Maps, Sets and class instances passed to a thaw-js instance method's `Json` slot
// must reach the method as live views (`__thaw_*_with_native_view`), not as copies. Method-only scope.

fn thaw_js_live_arg_symbol(backend: &str, symbol: &str) -> String {
    let hex: String = symbol.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("__thaw_typed_{backend}_{hex}")
}

// Lowers `main` that builds an emitter, then passes `argument` (after `prelude`) to `callee`.
fn thaw_js_live_arg_main_body(callee: &str, prelude: &str, argument: &str) -> String {
    thaw_js_live_arg_main_body_with_decls(callee, "", prelude, argument)
}

fn thaw_js_live_arg_main_body_with_decls(
    callee: &str,
    decls: &str,
    prelude: &str,
    argument: &str,
) -> String {
    let new = thaw_js_live_arg_symbol("tjs", "$new$k::Emitter$arity0");
    let source = format!(
        r#"
        {decls}
        declare function {new}(): JsValue;
        declare function {callee}(receiver: JsValue, event: Json, value: Json): JsValue;
        function main(): void {{
            {prelude}
            const emitter = {new}();
            {callee}(emitter, "e", {argument});
        }}
        "#,
        decls = decls,
        new = new,
        callee = callee,
        prelude = prelude,
        argument = argument,
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
fn thaw_js_instance_method_json_slot_takes_a_live_array_view_for_a_variable() {
    let emit = thaw_js_live_arg_symbol("tjs", "$method$k::Emitter$emit$overload0$arity3");
    let body = thaw_js_live_arg_main_body(&emit, "const arr = [1, 2];", "arr");
    assert!(body.contains("__thaw_array_with_native_view"), "{body}");
}

#[test]
fn thaw_js_instance_method_json_slot_takes_live_map_set_and_class_views_for_variables() {
    let emit = thaw_js_live_arg_symbol("tjs", "$methodvoid$k::Emitter$emit$overload0$arity3");
    let map = thaw_js_live_arg_main_body(&emit, "const m = new Map<string, number>();", "m");
    assert!(map.contains("__thaw_collection_with_native_view"), "{map}");
    let set = thaw_js_live_arg_main_body(&emit, "const s = new Set<number>();", "s");
    assert!(set.contains("__thaw_collection_with_native_view"), "{set}");
}

#[test]
fn thaw_js_instance_method_json_slot_takes_a_live_class_view_for_a_variable() {
    let emit = thaw_js_live_arg_symbol("tjs", "$method$k::Emitter$emit$overload0$arity3");
    let body = thaw_js_live_arg_main_body_with_decls(
        &emit,
        "class Point { x: number = 1; inc(): void { this.x += 1; } }",
        "const p = new Point();",
        "p",
    );
    assert!(body.contains("__thaw_object_with_native_getters"), "{body}");
}

#[test]
fn thaw_js_instance_method_json_slot_keeps_copying_literals_numbers_and_non_method_symbols() {
    let emit = thaw_js_live_arg_symbol("tjs", "$method$k::Emitter$emit$overload0$arity3");
    let literal = thaw_js_live_arg_main_body(&emit, "", "[1, 2]");
    assert!(!literal.contains("__thaw_array_with_native_view"), "{literal}");
    let number = thaw_js_live_arg_main_body(&emit, "const n = 5;", "n");
    assert!(!number.contains("__thaw_array_with_native_view"), "{number}");
    // a non-method (`$fn$`-style) thaw-js symbol is outside the method-only scope
    let function = thaw_js_live_arg_symbol("tjs", "$fn$k::emit$arity3");
    let non_method = thaw_js_live_arg_main_body(&function, "const arr = [1, 2];", "arr");
    assert!(!non_method.contains("__thaw_array_with_native_view"), "{non_method}");
    // a QuickJS symbol keeps its own lowering
    let quickjs = thaw_js_live_arg_symbol("js", "$method$k::Emitter$emit$overload0$arity3");
    let quickjs_body = thaw_js_live_arg_main_body(&quickjs, "const arr = [1, 2];", "arr");
    assert!(!quickjs_body.contains("__thaw_array_with_native_view"), "{quickjs_body}");
}
