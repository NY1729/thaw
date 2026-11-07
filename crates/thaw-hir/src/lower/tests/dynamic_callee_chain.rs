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

#[test]
fn const_bound_dynamic_method_result_called_later_lowers_as_dynamic_handle_call() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): JsValue;
        function main(): void {
            const c = mk();
            const h = c.hex("#f80");
            const s: string = h("x") as string;
        }
        "##,
    );
    assert!(body.contains("__thaw_call_selected_dynamic_method_handle"), "{body}");
    assert!(body.contains("callDynamicValueHandle"), "{body}");
    assert!(!body.contains("JsonAsNative"), "{body}");
}

#[test]
fn const_bound_dynamic_method_result_never_called_stays_json_snapshot() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): JsValue;
        function main(): void {
            const c = mk();
            const d = c.hex("#f80");
            console.log(d);
        }
        "##,
    );
    assert!(body.contains("__thaw_call_selected_dynamic_method"), "{body}");
    assert!(!body.contains("__thaw_call_selected_dynamic_method_handle"), "{body}");
}

#[test]
fn let_bound_dynamic_method_result_called_with_two_args_lowers_as_dynamic_handle_call() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): JsValue;
        function main(): void {
            const c = mk();
            let h = c.rgb(1, 2, 3);
            const s: string = h("a", "b") as string;
        }
        "##,
    );
    assert!(body.contains("__thaw_call_selected_dynamic_method_handle"), "{body}");
    assert!(body.contains("callDynamicValueHandle"), "{body}");
    assert!(!body.contains("JsonAsNative"), "{body}");
}
