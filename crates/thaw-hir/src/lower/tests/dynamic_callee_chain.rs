// A call whose callee is itself a call: `mk().hex("x")("y")`. The inner dynamic method call must be
// hinted `JsValue` so the outer call takes the dynamic handle path instead of failing as `Json`.

// Lowers the source and returns the Debug text of `main`'s body.
fn dynamic_callee_chain_main_body(source: &str) -> String {
    let program = lower(source);
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main is lowered");
    format!("{:?}", main.body)
}

#[test]
fn call_of_dynamic_method_call_result_lowers_as_dynamic_handle_call() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): JsValue;
        function main(): void {
            const c = mk();
            const s: string = c.hex("#f80")("x") as string;
        }
        "##,
    );
    assert!(body.contains("callDynamicValueHandle"), "{body}");
    assert!(body.contains("__thaw_call_selected_dynamic_method_handle"), "{body}");
}

#[test]
fn call_of_typed_closure_returning_call_keeps_static_function_call() {
    let body = dynamic_callee_chain_main_body(
        r##"
        function mk(a: number): (b: number) => number {
            return (b) => a + b;
        }
        function main(): void {
            const r: number = mk(1)(2);
        }
        "##,
    );
    assert!(!body.contains("callDynamicValueHandle"), "{body}");
}
