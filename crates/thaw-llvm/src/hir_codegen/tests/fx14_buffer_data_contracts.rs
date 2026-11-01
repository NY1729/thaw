// Buffer data contracts: console.log of a variable holding nested Buffers, and a
// JSON.parse reviver / ternary returning a Buffer into a Json slot. Expected output from node 22.

#[test]
fn buffer_data_console_log_var_holding_nested_buffer() {
    let source = r#"
function main(): void {
  const m = new Map<string, Buffer>();
  m.set("a", Buffer.from("ab"));
  console.log(m);
  const s = new Set<Buffer>();
  s.add(Buffer.from("ab"));
  console.log(s);
  const a: Buffer[] = [Buffer.from("ab")];
  console.log(a);
  const o: { x: Buffer } = { x: Buffer.from("ab") };
  console.log(o);
}
"#;
    let output = compile_and_run(source, "buffer_data_console_log_var_holding_nested_buffer");
    assert_eq!(
        output,
        "Map(1) { 'a' => <Buffer 61 62> }\nSet(1) { <Buffer 61 62> }\n[ <Buffer 61 62> ]\n{ x: <Buffer 61 62> }\n"
    );
}

#[test]
fn buffer_data_ternary_buffer_arm_joined_with_json_keeps_buffer() {
    let source = r#"
function main(): void {
  const k = (v: any): any => v === 1 ? Buffer.from("ab") : v;
  const b = k(1);
  console.log(Buffer.isBuffer(b), b.length);
  console.log(k(2));
  const r = JSON.parse('{"x":{"type":"Buffer","data":[97,98]}}', (key: string, v: any) => v && v.type === 'Buffer' ? Buffer.from(v.data) : v);
  console.log(Buffer.isBuffer(r.x), r.x.length, r.x.toString());
}
"#;
    let output = compile_and_run(source, "buffer_data_ternary_buffer_arm_joined_with_json_keeps_buffer");
    assert_eq!(output, "true 2\n2\ntrue 2 ab\n");
}
