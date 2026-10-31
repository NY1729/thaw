// thaw-js (`JsEngine::ThawJs`): a value export served as a handle (`$value$pkg::name$arity0`) and
// calls of it (`chalk.red("x")`, `marked.parse(..)`) name only `thaw_tjs_*` symbols.

fn thaw_js_value_export_program(statement: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_2476616c756524633a3a6368616c6b24617269747930(): JsValue;
         function main(): void {{
           const chalk: JsValue = __thaw_typed_tjs_2476616c756524633a3a6368616c6b24617269747930();
           {statement}
         }}"
    )
}

#[test]
#[allow(clippy::single_element_loop)]
fn thaw_js_routed_value_export_handle_calls_link_no_quickjs() {
    for statement in ["const text: JsValue = chalk.red(\"x\"); console.log(text);"] {
        let source = thaw_js_value_export_program(statement);
        let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
        assert!(uses_thaw_js && !uses_quickjs, "{statement}\n{ir}");
        assert!(ir.contains("@thaw_tjs_class_call("), "{statement}\n{ir}");
        assert!(
            !ir.lines().any(|line| {
                line.contains(" call ")
                    && line
                        .split('(')
                        .next()
                        .is_some_and(|callee| callee.contains("@thaw_js_"))
            }),
            "{statement}\n{ir}"
        );
    }
}
