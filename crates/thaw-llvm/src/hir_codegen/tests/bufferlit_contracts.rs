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
