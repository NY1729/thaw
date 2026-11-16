use super::*;

/// Run `src` and join its console.log lines with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

// Node 22: Headers/Request/Response.prototype[@@toStringTag] is a data property (not an accessor),
// non-writable, non-enumerable, configurable.

#[test]
fn web_class_prototype_to_string_tag_is_data_property() {
    assert_eq!(
        one("for (const C of [Headers, Request, Response]) { const d = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag); console.log(C.name, d.value, 'get' in d, d.writable, d.enumerable, d.configurable); }"),
        "Headers Headers false false false true|Request Request false false false true|Response Response false false false true"
    );
}

#[test]
fn web_class_instances_stringify_with_their_tag() {
    assert_eq!(
        one("console.log(Object.prototype.toString.call(new Headers()), Object.prototype.toString.call(new Request('http://x/')), Object.prototype.toString.call(new Response()));"),
        "[object Headers] [object Request] [object Response]"
    );
}
