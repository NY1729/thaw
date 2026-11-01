// A real Buffer printed through the Json inspect path must show as <Buffer ..> in
// object fields, array elements, class fields and closure results, while the plain
// `{ type: "Buffer", data }` object stays a plain object.
// Expected output recorded from node 22 (--experimental-transform-types).

#[test]
fn fx13_contract_buffer_tojson_prints_as_plain_object() {
    let source = r##"function main(): void {
    const b = Buffer.from("ab");
    console.log(b.toJSON(), { type: "Buffer", data: [97, 98] });
}
"##;
    assert_eq!(
        compile_and_run(source, "fx13_contract_buffer_tojson_prints_as_plain_object"),
        "{ type: 'Buffer', data: [ 97, 98 ] } { type: 'Buffer', data: [ 97, 98 ] }\n"
    );
}

#[test]
fn fx13_contract_buffer_in_containers_prints_as_buffer() {
    let source = r##"function main(): void {
    const b = Buffer.from("ab");
    console.log({ b }, [b], { list: [b], n: 1 });
}
"##;
    assert_eq!(
        compile_and_run(source, "fx13_contract_buffer_in_containers_prints_as_buffer"),
        "{ b: <Buffer 61 62> } [ <Buffer 61 62> ] { list: [ <Buffer 61 62> ], n: 1 }\n"
    );
}

#[test]
fn fx13_contract_buffer_class_field_and_closure_print_as_buffer() {
    let source = r##"class K {
    buf: Buffer = Buffer.from("ab");
    n = 1;
}
function main(): void {
    const b = Buffer.from("ab");
    const f = () => b;
    console.log(new K());
    console.log({ f: f() });
}
"##;
    assert_eq!(
        compile_and_run(source, "fx13_contract_buffer_class_field_and_closure_print_as_buffer"),
        "K { buf: <Buffer 61 62>, n: 1 }\n{ f: <Buffer 61 62> }\n"
    );
}

#[test]
fn fx13_contract_buffer_coerced_to_any_prints_as_buffer() {
    let source = r##"function main(): void {
    const b = Buffer.from("ab");
    const w: any = b;
    console.log(w, [w]);
}
"##;
    assert_eq!(
        compile_and_run(source, "fx13_contract_buffer_coerced_to_any_prints_as_buffer"),
        "<Buffer 61 62> [ <Buffer 61 62> ]\n"
    );
}

#[test]
fn fx13_contract_json_round_trip_of_buffer_keeps_node_shapes() {
    let source = r##"function main(): void {
    const b = Buffer.from("ab");
    console.log(JSON.parse(JSON.stringify({ b })));
}
"##;
    assert_eq!(
        compile_and_run(source, "fx13_contract_json_round_trip_of_buffer_keeps_node_shapes"),
        "{ b: { type: 'Buffer', data: [ 97, 98 ] } }\n"
    );
}
