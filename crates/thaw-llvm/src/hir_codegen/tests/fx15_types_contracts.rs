// Round 6 type contracts. Expected output from node 22.

#[test]
fn fx15_bytes_member_read_on_call_receiver_keeps_bytes_type() {
    let source = r#"
function main(): void {
  console.log(Buffer.from("xy").byteLength);
  console.log(Buffer.alloc(3).buffer.byteLength > 0);
  console.log(new ArrayBuffer(8).slice(2).byteLength);
  console.log(new DataView(new ArrayBuffer(4)).byteLength);
}
"#;
    let output = compile_and_run(source, "fx15_bytes_member_read_on_call_receiver_keeps_bytes_type");
    assert_eq!(output, "2\ntrue\n6\n4\n");
}
