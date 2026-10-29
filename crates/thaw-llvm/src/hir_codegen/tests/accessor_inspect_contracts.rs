// Node-verified contracts for console.log of accessor properties on object literals:
// Node prints `[Getter]`, `[Setter]` or `[Getter/Setter]` and never invokes the accessor.
// Class prototype accessors are not own properties, so instances print without them.
// Expected text recorded from Node v22.

#[test]
fn inspect_accessor_kinds_on_object_literal() {
    let source = r####"function main(): void { console.log({ get a() { return 1; }, set b(v: number) {}, get c() { return 1; }, set c(v: number) {} }); }
"####;
    assert_eq!(
        compile_and_run(source, "inspect_accessor_kinds_on_object_literal"),
        "{ a: [Getter], b: [Setter], c: [Getter/Setter] }\n"
    );
}

#[test]
fn inspect_accessor_beside_data_keeps_json_values() {
    let source = r####"function main(): void { const o = { x: 1, get y() { return 2; } }; console.log(o, JSON.stringify(o)); }
"####;
    assert_eq!(
        compile_and_run(source, "inspect_accessor_beside_data_keeps_json_values"),
        "{ x: 1, y: [Getter] } {\"x\":1,\"y\":2}\n"
    );
}

#[test]
fn inspect_accessor_does_not_invoke_the_accessor() {
    let source = r####"function main(): void { let calls = 0; const o = { get a() { calls++; return 1; }, set s(v: number) { calls++; } }; console.log(o); console.log(calls); console.log(JSON.stringify(o), calls); }
"####;
    assert_eq!(
        compile_and_run(source, "inspect_accessor_does_not_invoke_the_accessor"),
        "{ a: [Getter], s: [Setter] }\n0\n{\"a\":1} 1\n"
    );
}

#[test]
fn inspect_accessor_nested_and_class_prototype_accessors() {
    let source = r####"class C { get v(): number { return 1; } }
function main(): void { console.log({ inner: { get g() { return 3; } } }); console.log(new C()); }
"####;
    assert_eq!(
        compile_and_run(
            source,
            "inspect_accessor_nested_and_class_prototype_accessors"
        ),
        "{ inner: { g: [Getter] } }\nC {}\n"
    );
}
