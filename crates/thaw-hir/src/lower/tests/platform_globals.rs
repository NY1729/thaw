use super::platform_globals;

// Standard globals resolved through `platform_globals::PLATFORM_GLOBALS`.

fn lowering_error(source: &str) -> Option<String> {
    let module = thaw_parser::parse_typescript(source).expect("parse error");
    lower_module(&module).err()
}

#[test]
fn every_platform_global_lowers_in_value_and_typeof_position() {
    let mut failures = Vec::new();
    for (name, _) in platform_globals::PLATFORM_GLOBALS {
        let value = format!("function main(): void {{ const value: any = {name}; }}");
        let typeof_ = format!("function main(): void {{ const kind: string = typeof {name}; }}");
        for source in [value, typeof_] {
            if let Some(error) = lowering_error(&source).filter(|error| error.contains("unknown variable")) {
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    assert!(failures.is_empty(), "platform globals rejected as unknown variables: {failures:?}");
}

#[test]
fn platform_constructors_lower_under_new_to_the_realm_constructor() {
    let mut failures = Vec::new();
    for (name, kind) in platform_globals::PLATFORM_GLOBALS {
        if *kind != platform_globals::GlobalKind::Ctor {
            continue;
        }
        let source = format!("function main(): void {{ const made: any = new {name}(); }}");
        match lower_module(&thaw_parser::parse_typescript(&source).expect("parse error")) {
            Ok(program) => {
                if !format!("{program:?}").contains("constructDynamicValue") {
                    failures.push(format!("{name}: lowered without constructDynamicValue"));
                }
            }
            Err(error) => {
                // A dedicated `new` path with its own argument rules (`new WeakRef()`) may reject
                // the bare call; only an unresolved name is a failure here.
                if error.contains("unknown variable") {
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "platform constructors not lowered under new: {failures:?}");
}

#[test]
fn bare_atob_and_btoa_values_coexist_with_the_call_intrinsic() {
    lower(
        "function main(): void { const encode: any = btoa; const decode: any = atob; const text: string = atob(\"YQ==\"); const b64: string = btoa(text); }",
    );
}

#[test]
fn platform_constructor_names_are_opaque_annotations() {
    lower("function main(): void { const socket: WebSocket | undefined = undefined; const init: RequestInit | undefined = undefined; }");
}

#[test]
fn a_user_binding_named_like_a_platform_constructor_is_not_captured() {
    lower(
        "declare function makeSocket(): any; function main(): void { const WebSocket: any = makeSocket(); const socket: any = new WebSocket(); }",
    );
}

// The table must not shadow the pre-table lowering of names with a compiled model.

fn lowered_debug(source: &str) -> String {
    let module = thaw_parser::parse_typescript(source).expect("parse error");
    format!("{:?}", lower_module(&module).expect("lowering failed"))
}

#[test]
fn compiled_models_keep_their_lowering_under_the_table() {
    let weak = lowered_debug(
        "function main(): void { const target: object = { value: 1 }; const reference: WeakRef<object> = new WeakRef(target); const same: boolean = reference.deref() === target; }",
    );
    assert!(!weak.contains("constructDynamicValue"), "WeakRef lowered as a realm construction");
    let typed = lowered_debug(
        "function main(): void { const parts: string = Uint8Array.from([1, 2, 3]).join(\",\"); }",
    );
    assert!(!typed.contains("\"Uint8Array\""), "Uint8Array.from read through the realm global");
    // `new Buffer(...)` has no compiled constructor: pre-table lowering rejects it, so it still does.
    let buffer_error = lowering_error("function main(): void { const made: any = new Buffer(4); }")
        .expect("new Buffer should not lower");
    assert!(buffer_error.contains("`new Buffer`"), "unexpected error: {buffer_error}");
}

#[test]
fn frozen_lists_only_name_platform_constructors_and_exclude_compiled_models() {
    for name in platform_globals::DYNAMIC_NEW_CTORS.iter().chain(platform_globals::DYNAMIC_OPAQUE_CTORS) {
        assert_eq!(
            platform_globals::platform_global(name),
            Some(platform_globals::GlobalKind::Ctor),
            "`{name}` is frozen as a dynamic constructor but is not a table constructor"
        );
    }
    for name in ["WeakRef", "Buffer", "Uint8Array"] {
        assert!(!platform_globals::DYNAMIC_NEW_CTORS.contains(&name), "`{name}` in DYNAMIC_NEW_CTORS");
        assert!(!platform_globals::DYNAMIC_OPAQUE_CTORS.contains(&name), "`{name}` in DYNAMIC_OPAQUE_CTORS");
    }
}

// Erased TS wrappers around a special builtin/class object must not hide the special case.

#[test]
fn cast_wrapped_error_stack_trace_limit_lowers_like_the_bare_builtin() {
    let bare = lowered_debug("function main(): void { Error.stackTraceLimit = 0; }");
    assert!(bare.contains("__thaw_error_set_stack_trace_limit"), "bare Error.stackTraceLimit not special-cased");
    for wrapped in [
        "function main(): void { (Error as any).stackTraceLimit = 0; }",
        "function main(): void { (Error as unknown as any).stackTraceLimit = 0; }",
        "function main(): void { (<any>Error).stackTraceLimit = 0; }",
    ] {
        assert_eq!(lowered_debug(wrapped), bare, "{wrapped}");
    }
}

#[test]
fn cast_wrapped_class_static_getter_only_is_still_rejected_as_readonly() {
    let source = |target: &str| {
        format!(
            "class Box {{ static get current(): number {{ return 1; }} }} function main(): void {{ {target}.current = 2; }}"
        )
    };
    let bare = lowering_error(&source("Box")).expect("bare readonly static assignment should be rejected");
    assert!(bare.contains("readonly static member `Box.current`"), "unexpected error: {bare}");
    assert_eq!(lowering_error(&source("(Box as any)")).as_deref(), Some(bare.as_str()));
}
