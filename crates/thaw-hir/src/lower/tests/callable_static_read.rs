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
