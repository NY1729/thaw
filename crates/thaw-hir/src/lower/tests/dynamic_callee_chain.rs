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

#[test]
fn call_of_call_on_new_jsvalue_global_lowers_as_dynamic_handle_calls() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function __thaw_typed_js_ctor(): JsValue;
        let Chalk_: JsValue = __thaw_typed_js_ctor();
        function main(): void {
            const chalk = new Chalk_({ level: 1 });
            const h = chalk.hex("#ff8800");
            console.log(h("x"));
            const r = chalk.rgb(1, 2, 3);
            console.log(r("y"));
            console.log(chalk.hex("#ff8800")("z"));
        }
        "##,
    );
    assert!(body.matches("callDynamicValueHandle").count() >= 3, "{body}");
    assert!(body.contains("constructDynamicValue"), "{body}");
    assert!(!body.contains("JsonAsNative"), "{body}");
}

#[test]
fn call_of_call_on_jsvalue_argument_inside_console_log_lowers_as_dynamic_handle_call() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): JsValue;
        function main(): void {
            const c: JsValue = mk();
            console.log(c.hex("#f80")("z"));
        }
        "##,
    );
    assert!(body.contains("callDynamicValueHandle"), "{body}");
}

#[test]
fn call_of_call_on_json_receiver_lowers_without_build_error() {
    let body = dynamic_callee_chain_main_body(
        r##"
        declare function mk(): any;
        function main(): void {
            const c = mk();
            console.log(c.hex("#f80")("z"));
            const h = c.hex("#f80");
            console.log(h("x"));
        }
        "##,
    );
    assert!(!body.is_empty(), "{body}");
}
