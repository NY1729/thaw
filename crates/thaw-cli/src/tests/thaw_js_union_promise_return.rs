// thaw-js class methods whose declared return is a union with a Promise member (`Response |
// Promise<Response>`) get a `$await` runtime-key suffix, which makes thaw-js settle the Promise.
// QuickJS keys and every other method keep their exact keys.

fn union_promise_key(symbol: &str) -> String {
    let hex = symbol.strip_prefix("__thaw_typed_js_").unwrap();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn thaw_js_union_promise_method_key_carries_the_await_suffix_only_on_thaw_js() {
    let class = thaw_bridge::parse_dts_classes(
        "export declare class Hono {
            request(p: string): Response | Promise<Response>;
            plain(): Response;
        }",
    )
    .unwrap()
    .remove(0);
    let keys = |thaw_js: bool| {
        let mut shim = String::new();
        generate_napi_class_method_overloads_with_callback_instances(
            &class,
            false,
            &std::collections::HashMap::new(),
            &mut shim,
            false,
            thaw_js,
            &mut Vec::new(),
        )
        .into_iter()
        .map(|(method, symbol, ..)| (method, union_promise_key(&symbol)))
        .collect::<Vec<_>>()
    };
    let thaw_js = keys(true);
    assert!(
        thaw_js.contains(&(
            "request".into(),
            "$method$Hono$request$overload0$arity1$await".into()
        )),
        "{thaw_js:?}"
    );
    assert!(
        thaw_js.contains(&("plain".into(), "$method$Hono$plain$overload0$arity0".into())),
        "{thaw_js:?}"
    );
    let quickjs = keys(false);
    assert!(
        quickjs.contains(&(
            "request".into(),
            "$method$Hono$request$overload0$arity1".into()
        )),
        "{quickjs:?}"
    );
    assert!(
        quickjs.iter().all(|(_, key)| !key.ends_with("$await")),
        "{quickjs:?}"
    );
}

#[test]
fn thaw_js_class_member_ignores_the_await_suffix() {
    let key = "$method$p::C$request$overload0$arity1$await";
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    let line =
        format!("declare function __thaw_typed_js_{hex}(receiver: JsValue, p: string): JsValue;");
    assert_eq!(thaw_js_class_member(&line).as_deref(), Some("request"));
}
