// S3 (rest): ArrayBuffer / DataView / Uint8Array in `any` keep their own kind and alias the
// native store. Expected output recorded from node 22.

#[test]
fn s3_any_arraybuffer_keeps_its_kind() {
    let source = r##"function main(): void {
    const ab = new ArrayBuffer(4);
    const a: any = ab;
    console.log(Object.prototype.toString.call(a), a.byteLength, Buffer.isBuffer(a));
    new Uint8Array(ab)[1] = 7;
    console.log(new Uint8Array(a)[1]);
}
"##;
    assert_eq!(
        compile_and_run(source, "s3_any_arraybuffer_keeps_its_kind"),
        "[object ArrayBuffer] 4 false\n7\n"
    );
}

#[test]
fn s3_any_cast_arraybuffer_keeps_its_kind() {
    let source = r##"function main(): void {
    const b = new ArrayBuffer(2) as any;
    console.log(String(b), b.byteLength, Buffer.isBuffer(b));
}
"##;
    assert_eq!(
        compile_and_run(source, "s3_any_cast_arraybuffer_keeps_its_kind"),
        "[object ArrayBuffer] 2 false\n"
    );
}

#[test]
fn s3_any_dataview_methods_write_through() {
    let source = r##"function main(): void {
    const dv = new DataView(new ArrayBuffer(4));
    const d: any = dv;
    d.setUint8(0, 9);
    d.setUint16(2, 258);
    console.log(dv.getUint8(0), d.getUint8(0), d.getUint16(2));
}
"##;
    assert_eq!(compile_and_run(source, "s3_any_dataview_methods_write_through"), "9 9 258\n");
}

#[test]
fn s3_any_dataview_brand() {
    let source = r##"function main(): void {
    const d: any = new DataView(new ArrayBuffer(4));
    console.log(Object.prototype.toString.call(d), d.byteLength, Buffer.isBuffer(d));
}
"##;
    assert_eq!(compile_and_run(source, "s3_any_dataview_brand"), "[object DataView] 4 false\n");
}

#[test]
fn s3_any_dataview_get_reads_the_store() {
    let source = r##"function main(): void {
    const dv = new DataView(new ArrayBuffer(4));
    dv.setUint8(0, 9);
    const d: any = dv;
    console.log(d.getUint8(0));
}
"##;
    assert_eq!(compile_and_run(source, "s3_any_dataview_get_reads_the_store"), "9\n");
}

#[test]
fn s3_any_arraybuffer_and_dataview_stringify_as_empty_objects() {
    let source = r##"function main(): void {
    const ab = new ArrayBuffer(2);
    const a: any = ab;
    const dv: any = new DataView(ab);
    console.log(JSON.stringify({ x: a, y: dv, z: Buffer.from("a") as any }));
}
"##;
    assert_eq!(
        compile_and_run(source, "s3_any_arraybuffer_and_dataview_stringify_as_empty_objects"),
        "{\"x\":{},\"y\":{},\"z\":{\"type\":\"Buffer\",\"data\":[97]}}\n"
    );
}
