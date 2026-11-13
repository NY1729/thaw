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
