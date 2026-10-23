// Out-of-range index semantics. Expected output from node 22.

#[test]
fn bytes_out_of_range_index_write_is_ignored() {
    let source = r#"
function main(): void {
  const u = new Uint8Array(4);
  u[10] = 1;
  u[-1] = 2;
  u[1.5] = 3;
  u[2] = 7;
  const b = Buffer.from([1, 2]);
  b[5] = 9;
  console.log(u.length, u[10], u[2], b.length);
}
"#;
    let output = compile_and_run(source, "bytes_out_of_range_index_write_is_ignored");
    assert_eq!(output, "4 undefined 7 2\n");
}
