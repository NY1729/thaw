// Object.create(proto) results keep own-ness; expected output recorded from node 22.

#[test]
fn object_create_contract_o1() {
    let source = r##"const proto = { f() { return 1; }, a: 1 };
const o = Object.create(proto) as { a: number; f(): number; b?: number };
o.b = 2;
console.log(JSON.stringify(o), o.a, o.f(), Object.keys(o), o.hasOwnProperty("a"), o.hasOwnProperty("b"));
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o1"), "{\"b\":2} 1 1 [ 'b' ] false true\n");
}

#[test]
fn object_create_contract_o5() {
    let source = r##"const proto = { a: 1 };
const o = Object.create(proto) as { a: number; b?: number };
o.b = 2;
console.log(JSON.stringify(o));
console.log(Object.keys(o));
console.log(o.b);
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o5"), "{\"b\":2}\n[ 'b' ]\n2\n");
}

#[test]
fn object_create_contract_o7() {
    let source = r##"const proto = { a: 1 };
const o = Object.create(proto) as { a: number; b?: number };
o.a = 5;
console.log(JSON.stringify(o), JSON.stringify(proto));
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o7"), "{\"a\":5} {\"a\":1}\n");
}

#[test]
fn object_create_contract_o9() {
    let source = r##"const proto = { a: 1 };
const o = Object.create(proto);
o.b = 2;
console.log(JSON.stringify(o));
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o9"), "{\"b\":2}\n");
}

#[test]
fn object_create_contract_o10() {
    let source = r##"interface V { a: number; b?: number }
const proto: V = { a: 1 };
const o: V = Object.create(proto);
o.b = 2;
const s = JSON.stringify(o);
console.log(s);
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o10"), "{\"b\":2}\n");
}

#[test]
fn object_create_contract_o2() {
    let source = r##"const proto = { greet() { return "hi " + this.name; }, name: "base" };
const o = Object.create(proto) as { name: string; greet(): string };
console.log(o.greet(), JSON.stringify(o));
o.name = "own";
console.log(o.greet(), JSON.stringify(o));
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_o2"), "hi base {}\nhi own {\"name\":\"own\"}\n");
}

#[test]
fn object_create_contract_proto_chain() {
    let source = r##"const proto = { a: 1, f() { return this.a; } };
const o = Object.create(proto) as { a: number; f(): number; b?: number };
console.log(Object.getPrototypeOf(o) === proto);
proto.a = 7;
console.log(o.a, o.f(), JSON.stringify(o));
for (const k in o) console.log("key", k);
console.log({ ...o }, Object.keys(o).length);
o.b = 3;
console.log(JSON.stringify(o), Object.keys(o), Object.hasOwn(o, "a"));
"##;
    assert_eq!(compile_and_run(source, "object_create_contract_proto_chain"), "true\n7 7 {}\nkey a\nkey f\n{} 0\n{\"b\":3} [ 'b' ] false\n");
}
