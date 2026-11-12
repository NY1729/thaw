// sqlite3-shaped class: fixed-arity methods whose callbacks take a JsValue error (`Function([JsValue])`)
// and a Json row array. Both callbacks cross as graph handle leases, so the class builds on thaw-js.

#[test]
fn sqlite3_shaped_class_callbacks_build_on_the_graph_route() {
    let new = hex("$new$k::Database$arity1");
    let run = hex("$method$k::Database$run$overload0$arity3");
    let all = hex("$method$k::Database$all$overload0$arity2");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(filename: string): JsValue;
         declare function __thaw_typed_tjs_{run}(receiver: JsValue, sql: string, params: Json[], callback: (error: JsValue) => void): Json;
         declare function __thaw_typed_tjs_{all}(receiver: JsValue, sql: string, callback: (error: JsValue, rows: Json[]) => void): Json;
         function main(): void {{
           const db = __thaw_typed_tjs_{new}(\":memory:\");
           __thaw_typed_tjs_{run}(db, \"select ?\", [42], (error: JsValue): void => {{ console.log(\"ran\"); }});
           __thaw_typed_tjs_{all}(db, \"select 1\", (error: JsValue, rows: Json[]): void => {{ console.log(rows); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_sqlite3_shape");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("cannot cross"), "{ir}");
    let graph_calls = ir
        .lines()
        .filter(|line| line.contains("call ") && line.contains("@thaw_tjs_register_native_callback_graph("))
        .count();
    assert_eq!(graph_calls, 2, "{ir}");
}
