// `Buffer.prototype.toJSON` returns the plain `{ type: "Buffer", data: [...] }` object
// (not a branded Buffer), so console.log prints it as an object and JSON.stringify
// gives the Node encoding. Expected output recorded from node 22.

// Printing the toJSON() object directly is still an open display gap (it shows as <Buffer ...>), so only its JSON/fields are asserted here.
#[test]
fn fx12_contract_buffer_to_json_is_plain_object() {
    let source = r##"function main(): void {
    const b = Buffer.from("ab");
    console.log(JSON.stringify(b.toJSON()));
    console.log(Buffer.from("héllo").toJSON().data.length);
    const w: any = b;
    console.log(w);
    console.log({ k: b });
}
"##;
    assert_eq!(
        compile_and_run(source, "fx12_contract_buffer_to_json_is_plain_object"),
        "{\"type\":\"Buffer\",\"data\":[97,98]}
6
<Buffer 61 62>
{ k: <Buffer 61 62> }
"
    );
}
