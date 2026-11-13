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
