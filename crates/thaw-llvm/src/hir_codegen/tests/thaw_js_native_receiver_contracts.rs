// A native receiver on the thaw-js lane: a class instance exposed as `any` calls back into compiled code,
// and the callback's `this` is recovered from its wrapper handle through thaw-js's owner query.

/// The line index of the first `call` to `name`, if any.
fn native_receiver_call_line(ir: &str, name: &str) -> Option<usize> {
    ir.lines()
        .position(|line| line.contains("call ") && line.contains(name))
}

const NATIVE_RECEIVER_MAP_SOURCE: &str = "class Store { calls = 0; use(m: Map<string, number>): number { this.calls += 1; return this.calls; } }
     function main(): void {
       const store = new Store();
       const a: any = store;
       const m = new Map<string, number>();
       console.log(a.use(m), store.calls);
     }";

const NATIVE_RECEIVER_FIELD_SOURCE: &str =
    "class Box { value: number = 1; read(): number { return this.value; } }
     function main(): void {
       const box = new Box();
       const exposed: any = box;
       const original: any = new Function(\"return {x: 7}\")();
       exposed.value = original;
       console.log(box.read() === original, typeof box.value);
     }";

#[test]
fn native_receiver_routed_query_uses_the_thaw_js_owner_symbol() {
    let (uses_thaw_js, uses_quickjs, ir) =
        compile_routed(NATIVE_RECEIVER_MAP_SOURCE, "native_receiver_map");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(
        native_receiver_call_line(&ir, "@thaw_tjs_native_object_pointer(").is_some(),
        "{ir}"
    );
    assert!(
        native_receiver_call_line(&ir, "@thaw_js_native_object_pointer(").is_none(),
        "{ir}"
    );
    assert!(!ir.contains("@thaw_js_native_object_pointer"), "{ir}");
    // the wrapper handle is turned into its borrowed id before the owner query
    let borrowed = native_receiver_call_line(&ir, "@thaw_json_borrowed_handle_id(");
    let pointer = native_receiver_call_line(&ir, "@thaw_tjs_native_object_pointer(");
    assert!(borrowed.is_some() && borrowed < pointer, "{ir}");
}

#[test]
fn native_receiver_routed_field_write_on_a_class_wrapper_uses_the_thaw_js_owner_symbol() {
    let (uses_thaw_js, uses_quickjs, ir) =
        compile_routed(NATIVE_RECEIVER_FIELD_SOURCE, "native_receiver_field");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(
        native_receiver_call_line(&ir, "@thaw_tjs_native_object_pointer(").is_some(),
        "{ir}"
    );
    assert!(!ir.contains("@thaw_js_native_object_pointer"), "{ir}");
}

