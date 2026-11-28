// `structuredClone` of a native byte view copies its store and keeps its kind (outputs from node 22).

#[test]
fn structured_clone_of_any_bytes_copies_the_store() {
    let source = r##"function main(): void {
  const buf = Buffer.from("abc");
  const c: any = structuredClone(buf as any);
  c[0] = 90;
  console.log(buf[0], c[0], Object.prototype.toString.call(c));
  const ab = new ArrayBuffer(2);
  const ac: any = structuredClone(ab as any);
  new Uint8Array(ac)[0] = 7;
  console.log(new Uint8Array(ab)[0], ac.byteLength, Object.prototype.toString.call(ac));
  const dv = new DataView(new ArrayBuffer(4));
  const dc: any = structuredClone(dv as any);
  dc.setUint8(0, 5);
  console.log(dv.getUint8(0), dc.getUint8(0), Object.prototype.toString.call(dc));
  const typed = structuredClone(buf);
  typed[2] = 5;
  console.log(buf[2], typed[2]);
}
"##;
    assert_eq!(
        compile_and_run(source, "structured_clone_of_any_bytes_copies_the_store"),
        "97 90 [object Uint8Array]\n0 2 [object ArrayBuffer]\n0 5 [object DataView]\n99 5\n"
    );
}
