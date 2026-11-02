// Round 10 (ta04): a typed array's non-index own properties print after its elements, as Node
// does (`Uint8Array(4) [ 0, 0, 0, 0, x: 3 ]`). Out-of-range and negative index writes are dropped.
// Expected output recorded from node 22.

#[test]
fn typed_array_inspect_round10_expando_prints_after_elements() {
    let source = r#"function main(): void {
  const u = new Uint8Array(4);
  u[5] = 1;
  u[-1] = 2;
  (u as any).x = 3;
  console.log(u);
}
"#;
    assert_eq!(
        compile_and_run(source, "typed_array_inspect_round10_expando_prints_after_elements"),
        "Uint8Array(4) [ 0, 0, 0, 0, x: 3 ]\n"
    );
}

#[test]
fn typed_array_inspect_round10_expando_on_empty_typed_array() {
    let source = r#"function main(): void {
  const e = new Uint8Array(0);
  (e as any).x = 3;
  console.log(e);
}
"#;
    assert_eq!(
        compile_and_run(source, "typed_array_inspect_round10_expando_on_empty_typed_array"),
        "Uint8Array(0) [ x: 3 ]\n"
    );
}

#[test]
fn typed_array_inspect_round10_expando_primitive_kinds() {
    let source = r#"function main(): void {
  const b = new BigInt64Array(2);
  (b as any).y = 'hi';
  (b as any).z = -0;
  (b as any).w = true;
  console.log(b);
}
"#;
    assert_eq!(
        compile_and_run(source, "typed_array_inspect_round10_expando_primitive_kinds"),
        "BigInt64Array(2) [ 0n, 0n, y: 'hi', z: -0, w: true ]\n"
    );
}
