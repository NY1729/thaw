// Buffers inside array / object literals keep their Buffer type. Expected output from node 22.

#[test]
fn buffer_in_array_literal_dispatches_as_buffer() {
    let source = r#"
function main(): void {
  const b = Buffer.from("hello");
  const arr = [b, b];
  arr[0][0] = 72;
  console.log(arr[1].toString(), arr[0].toString("hex"), arr.length);
  const c = Buffer.from("xy");
  const mixed = [b, c];
  console.log(Buffer.concat(mixed).toString());
  console.log(Buffer.concat([b, c]).toString());
  console.log(Buffer.isBuffer(mixed[1]), mixed[1].readUInt8(0));
  const nums: number[][] = [b];
  console.log(nums[0].length, nums[0][0]);
}
"#;
    let output = compile_and_run(source, "buffer_in_array_literal_dispatches_as_buffer");
    assert_eq!(output, "Hello 48656c6c6f 2\nHelloxy\nHelloxy\ntrue 120\n5 72\n");
}

#[test]
fn buffer_in_array_and_nested_object_prints_like_node() {
    let source = r#"
function main(): void {
  const b = Buffer.from("hi");
  console.log({ b });
  console.log({ b: Buffer.from("hi") });
  console.log([b, Buffer.from("yo")]);
  console.log({ list: [b], n: 1 });
  const o = { b };
  o.b[0] = 72;
  console.log(b.toString(), o.b.length, Buffer.isBuffer(o.b));
}
"#;
    let output = compile_and_run(source, "buffer_in_array_and_nested_object_prints_like_node");
    assert_eq!(output, "{ b: <Buffer 68 69> }\n{ b: <Buffer 68 69> }\n[ <Buffer 68 69>, <Buffer 79 6f> ]\n{ list: [ <Buffer 68 69> ], n: 1 }\nHi 2 true\n");
}

#[test]
fn mixed_buffer_and_number_array_literal_stays_number_array_of_arrays() {
    let source = r#"
function main(): void {
  const b = Buffer.from([1, 2, 3]);
  const mixed = [b, [4, 5]];
  console.log(mixed.length, (mixed[1] as number[]).length);
  const cond = b.length > 2 ? b : Buffer.alloc(1);
  console.log(cond.toString("hex"));
  const pair: [Buffer, number] = [b, 7];
  console.log(pair[0].toString("hex"), pair[1]);
}
"#;
    let output = compile_and_run(source, "mixed_buffer_and_number_array_literal_stays_number_array_of_arrays");
    assert_eq!(output, "2 2\n010203\n010203 7\n");
}

// Byte lists with a dynamic or mixed element type (`Json` / `Tuple`) coerce element-wise
// to bytes. Expected output from node 22.

#[test]
fn buffer_concat_mixed_literal_with_any_element() {
    let source = r#"
function f(p: any): any {
  return Buffer.concat([Buffer.from([0x81, p.length]), p]);
}
function main(): void {
  console.log(f(Buffer.from("hi")).toString("hex"));
}
"#;
    let output = compile_and_run(source, "buffer_concat_mixed_literal_with_any_element");
    assert_eq!(output, "81026869\n");
}

#[test]
fn buffer_concat_any_list_of_any_buffers_still_joins() {
    let source = r#"
function main(): void {
  let b: any = Buffer.alloc(0);
  const d: any = Buffer.from("ab");
  b = Buffer.concat([b, d]);
  console.log(b.length, b.toString());
}
"#;
    let output = compile_and_run(source, "buffer_concat_any_list_of_any_buffers_still_joins");
    assert_eq!(output, "2 ab\n");
}

#[test]
fn buffer_concat_mixed_literal_with_total_length() {
    let source = r#"
function main(): void {
  const anyBuf: any = Buffer.from("cd");
  console.log(Buffer.concat([Buffer.from("ab"), anyBuf], 3).toString("hex"));
}
"#;
    let output = compile_and_run(source, "buffer_concat_mixed_literal_with_total_length");
    assert_eq!(output, "616263\n");
}

#[test]
fn buffer_concat_non_byte_element_is_a_compile_error() {
    let source = r#"
function main(): void {
  console.log(Buffer.concat([Buffer.from("a"), "s" as any as string]).length);
}
"#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let error = thaw_hir::lower_module(&module).unwrap_err();
    assert!(error.contains("expects an array of byte buffers"), "unexpected diagnostic: {error}");
}

#[test]
fn uint8array_from_any_array_source() {
    let source = r#"
function main(): void {
  const anyArr: any = [1, 2, 3];
  console.log(Uint8Array.from(anyArr).length);
}
"#;
    let output = compile_and_run(source, "uint8array_from_any_array_source");
    assert_eq!(output, "3\n");
}
