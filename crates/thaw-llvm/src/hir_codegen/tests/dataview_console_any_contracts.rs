// A DataView held in `any` prints its offset and whole backing store like Node (outputs from node 22).

#[test]
fn any_dataview_console_log_shows_offset_and_store() {
    let source = r##"function main(): void {
  const ab = new ArrayBuffer(4);
  const dv = new DataView(ab, 1, 2);
  const a: any = dv;
  console.log(a);
  console.log(String(a), Object.prototype.toString.call(a));
}
"##;
    assert_eq!(
        compile_and_run(source, "any_dataview_console_log_shows_offset_and_store"),
        "DataView {\n  [byteLength]: 2,\n  [byteOffset]: 1,\n  [buffer]: ArrayBuffer { [Uint8Contents]: <00 00 00 00>, [byteLength]: 4 }\n}\n[object DataView] [object DataView]\n"
    );
}
