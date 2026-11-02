#[test]
fn fx21_bare_return_in_constructor_keeps_the_instance() {
    let source = r##"class A { x = 1; constructor(f: boolean) { if (f) return; this.x = 2; } }
class N { x = 1; constructor() { return 5 as any; } }
function main(): void {
  console.log(JSON.stringify(new A(true)), JSON.stringify(new A(false)));
  console.log(JSON.stringify(new N()));
}
"##;
    assert_eq!(compile_and_run(source, "fx21_bare_constructor_return"), "{\"x\":1} {\"x\":2}\n{\"x\":1}\n");
}

#[test]
fn fx21_bare_return_in_derived_constructor_keeps_the_instance() {
    let source = r##"class P { y = 3; constructor(f: boolean) { if (f) { return; } this.y = 4; } }
class Dd extends P { z = 9; constructor(f: boolean) { super(f); if (f) return; this.z = 10; } }
function main(): void {
  console.log(JSON.stringify(new Dd(true)), JSON.stringify(new Dd(false)));
}
"##;
    assert_eq!(compile_and_run(source, "fx21_derived_bare_constructor_return"), "{\"y\":3,\"z\":9} {\"y\":4,\"z\":10}\n");
}
