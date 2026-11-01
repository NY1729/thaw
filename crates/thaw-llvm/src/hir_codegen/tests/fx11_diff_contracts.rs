// fx11 round-5 diff contracts; expected output recorded from Node 22.

#[test]
fn fx11_typeof_realm_globals_and_process_binding() {
    let source = r#"function main(): void {
  console.log(typeof process.binding, typeof navigator, typeof fetch, typeof FinalizationRegistry, typeof CompressionStream, typeof DecompressionStream, typeof escape, typeof NopeNope);
  console.log(typeof Navigator, navigator.platform, navigator.userAgent);
}"#;
    assert_eq!(
        compile_and_run(source, "fx11_typeof_realm_globals"),
        "function object function function function function function undefined\nfunction Linux x86_64 Node.js/22\n"
    );
}

#[test]
fn fx11_namespace_objects_inspect_with_object_tag() {
    let source = r#"function main(): void {
  console.log(Math, JSON, Reflect, Atomics, Intl, [Math]);
  console.log(Object.keys(Intl).length);
}"#;
    assert_eq!(
        compile_and_run(source, "fx11_namespace_inspect"),
        "Object [Math] {} Object [JSON] {} Object [Reflect] {} Object [Atomics] {} Object [Intl] {} [ Object [Math] {} ]\n0\n"
    );
}

#[test]
fn fx11_has_ref_of_cleared_timer_keeps_ref_state() {
    let source = r#"function main(): void {
  const t = setTimeout(() => {}, 1);
  console.log(t.hasRef());
  clearTimeout(t);
  console.log(t.hasRef());
}"#;
    assert_eq!(compile_and_run(source, "fx11_has_ref_cleared"), "true\ntrue\n");
}
