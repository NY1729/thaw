// `instanceof ArrayBuffer` / `DataView` / `Uint8Array` on an `any` holding a native view (outputs from node 22).

#[test]
fn any_view_instanceof_matches_node() {
    let source = r##"function main(): void {
    const ab = new ArrayBuffer(4);
    const a: any = ab;
    const d: any = new DataView(ab);
    const b: any = Buffer.from("x");
    const o: any = { x: 1 };
    console.log(a instanceof ArrayBuffer, a instanceof DataView, d instanceof DataView, d instanceof ArrayBuffer);
    console.log(b instanceof Uint8Array, b instanceof ArrayBuffer, o instanceof ArrayBuffer, o instanceof DataView);
    console.log(ab instanceof ArrayBuffer, ab instanceof DataView);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_view_instanceof_matches_node"),
        "true false true false\ntrue false false false\ntrue false\n"
    );
}
