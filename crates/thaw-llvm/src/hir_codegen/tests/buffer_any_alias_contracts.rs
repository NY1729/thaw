// S3: a Buffer / Uint8Array / ArrayBuffer crossing into `any` is a `Value::Bytes` aliasing the
// native view, not a copy. Expected output recorded from node 22.

#[test]
fn s3_any_index_write_reaches_the_buffer() {
    let source = r##"function main(): void {
    const buf = Buffer.from("abc");
    const a: any = buf;
    a[0] = 65;
    console.log(buf[0], a.length, typeof a, a[1]);
}
"##;
    assert_eq!(compile_and_run(source, "s3_any_index_write_reaches_the_buffer"), "65 3 object 98\n");
}

#[test]
fn s3_any_mutator_writes_through_and_json_matches_node() {
    let source = r##"function main(): void {
    const buf = Buffer.from("abc");
    const a: any = buf;
    a.fill(1);
    console.log(buf);
    console.log(JSON.stringify(a), String(Buffer.from("hi") as any));
}
"##;
    assert_eq!(
        compile_and_run(source, "s3_any_mutator_writes_through_and_json_matches_node"),
        "<Buffer 01 01 01>\n{\"type\":\"Buffer\",\"data\":[1,1,1]} hi\n"
    );
}

#[test]
fn s3_any_round_trip_keeps_the_same_store() {
    let source = r##"function id(x: any): any { return x; }
function main(): void {
    const buf = Buffer.alloc(3);
    const back: Buffer = id(buf);
    back[1] = 5;
    console.log(buf[1], back === buf);
}
"##;
    assert_eq!(compile_and_run(source, "s3_any_round_trip_keeps_the_same_store"), "5 true\n");
}

#[test]
fn s3_any_buffer_brand_checks() {
    let source = r##"function main(): void {
    const a: any = Buffer.from("hi");
    console.log(Object.prototype.toString.call(a), a instanceof Uint8Array, Buffer.isBuffer(a));
    let sum = 0;
    for (const byte of a) sum += byte;
    console.log(sum, [...a], a.toString("hex"));
}
"##;
    assert_eq!(
        compile_and_run(source, "s3_any_buffer_brand_checks"),
        "[object Uint8Array] true true\n209 [ 104, 105 ] 6869\n"
    );
}
