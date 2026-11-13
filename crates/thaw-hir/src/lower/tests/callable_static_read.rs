#[test]
fn static_property_read_on_callable_value_falls_back_to_dynamic_get() {
    // `EventEmitter.captureRejections` reads a static property of a callable (class-like) value.
    // It has no compile-time shape, so it must read through the dynamic property path instead of
    // failing with "unsupported property access".
    let program = lower(
        r#"function emitter(value?: number): void {}
        function read(): boolean {
            return emitter.captureRejections;
        }
        function main(): void {}"#,
    );
    let dump = format!("{:?}", program.functions);
    assert!(
        dump.contains("getDynamicProperty"),
        "static read on a callable value did not use the dynamic get: {dump}"
    );
}

#[test]
fn static_read_on_an_imported_class_reads_the_live_class_value() {
    // `EventEmitter.captureRejections` must read the real class value (its shim declares it
    // ambient), not a fresh native-callback wrapper around it, which has no statics.
    let program = lower(
        r#"declare function EventEmitter(): JsValue;
        function read(): boolean {
            return EventEmitter.captureRejections;
        }
        function main(): void {}"#,
    );
    let dump = format!("{:?}", program.functions);
    assert!(
        !dump.contains("registerNativeCallback"),
        "imported class static read wrapped the class in a native callback: {dump}"
    );
    assert!(dump.contains("getDynamicValue"), "{dump}");
}

#[test]
fn static_read_on_a_js_class_dispatcher_reads_the_live_class_by_its_pkg_key() {
    // A JS-realm class (`EventEmitter` from node:events) reaches the lowering as the generated
    // dispatcher `__thaw_class_ctor_<hex of "$new$node:events::EventEmitter">`, a real non-extern fn.
    // Its statics must be read from the live class by key, not wrapped as a native callback.
    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }
    let name = format!("__thaw_class_ctor_{}", hex("$new$node:events::EventEmitter"));
    let program = lower(&format!(
        r#"function {name}(...args: Json[]): JsValue {{
            return getDynamicValue("node:events::EventEmitter");
        }}
        function read(): boolean {{
            return {name}.captureRejections;
        }}
        function main(): void {{}}"#
    ));
    let dump = format!("{:?}", program.functions);
    assert!(dump.contains("Str(\"node:events::EventEmitter\")"), "{dump}");
    assert!(dump.contains("getDynamicValue"), "{dump}");
    assert!(!dump.contains("registerNativeCallback"), "{dump}");
}

#[test]
fn static_read_on_an_extern_single_arity_js_class_reads_the_live_class_by_its_pkg_key() {
    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }
    let name = format!(
        "__thaw_typed_tjs_{}",
        hex("$new$node:events::EventEmitter$arity0")
    );
    let program = lower(&format!(
        r#"declare function {name}(): JsValue;
        function read(): boolean {{
            return {name}.captureRejections;
        }}
        function main(): void {{}}"#
    ));
    let dump = format!("{:?}", program.functions);
    assert!(dump.contains("Str(\"node:events::EventEmitter\")"), "{dump}");
    assert!(dump.contains("getDynamicValue"), "{dump}");
    assert!(!dump.contains("registerNativeCallback"), "{dump}");
}

#[test]
fn imported_class_key_recovers_pkg_class_from_constructor_symbols() {
    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }
    let key = |name: &str| crate::lower::imported_class_key(name);
    let ctor = format!("__thaw_class_ctor_{}", hex("$new$node:events::EventEmitter"));
    assert_eq!(key(&ctor).as_deref(), Some("node:events::EventEmitter"));
    assert_eq!(
        key(&format!("__thawmod3_{ctor}")).as_deref(),
        Some("node:events::EventEmitter")
    );
    let typed = format!(
        "__thaw_typed_tjs_{}",
        hex("$new$node:events::EventEmitter$arity1")
    );
    assert_eq!(key(&typed).as_deref(), Some("node:events::EventEmitter"));
    let overloaded = format!(
        "__thaw_typed_tjs_{}",
        hex("$new$node:events::EventEmitter$overload1$arity2")
    );
    assert_eq!(key(&overloaded).as_deref(), Some("node:events::EventEmitter"));
    assert_eq!(key("EventEmitter"), None);
    assert_eq!(
        key(&format!("__thaw_typed_tjs_{}", hex("$new$NoPackage$arity0"))),
        None
    );
}
