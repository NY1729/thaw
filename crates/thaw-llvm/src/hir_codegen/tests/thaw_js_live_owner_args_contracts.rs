// Fix A (plan-tj88) contracts: a named object variable passed to a thaw-js extern FUNCTION `any`/generic slot is
// sent as its live handle and a returned handle maps back to the owner; literals and QuickJS keep the old IR.

// `SYM` in `body` stands for the extern identity function's symbol in the given lane.
fn live_owner_ir(symbol_lane: &str, body: &str, routed: bool) -> String {
    let hex: String = "k::identity".bytes().map(|byte| format!("{byte:02x}")).collect();
    let symbol = format!("__thaw_typed_{symbol_lane}_{hex}");
    let source = format!(
        "declare function {symbol}<T>(value: T): T;
         function main(): void {{
           {}
         }}",
        body.replace("SYM", &symbol)
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_live_owner_args");
    compiler.set_thaw_js_routed(routed);
    compiler.compile_program(&program).unwrap();
    compiler.print_to_string()
}

// Routed (thaw-js) IR renames the owner lookup to `thaw_tjs_native_object_pointer`.
fn calls_owner_pointer(ir: &str) -> bool {
    ir.lines().any(|line| {
        line.contains("call ")
            && (line.contains("@thaw_tjs_native_object_pointer(") || line.contains("@thaw_js_native_object_pointer("))
    })
}

#[test]
fn thaw_js_function_named_object_arg_sends_a_live_handle_and_maps_the_owner_back() {
    let ir = live_owner_ir("tjs", "const source = { n: 1 }; const back = SYM(source);", true);
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(calls_owner_pointer(&ir), "{ir}");
}

#[test]
fn thaw_js_function_literal_and_array_args_keep_the_by_value_ir() {
    let literal = live_owner_ir("tjs", "const back = SYM({ n: 1 });", true);
    assert!(!calls_owner_pointer(&literal), "{literal}");
    let array = live_owner_ir("tjs", "const arr = [1, 2]; const back = SYM(arr);", true);
    assert!(!calls_owner_pointer(&array), "{array}");
}

#[test]
fn quickjs_twin_named_object_arg_gets_no_live_owner_ir() {
    let ir = live_owner_ir("js", "const source = { n: 1 }; const back = SYM(source);", false);
    assert!(!ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!calls_owner_pointer(&ir), "{ir}");
}
