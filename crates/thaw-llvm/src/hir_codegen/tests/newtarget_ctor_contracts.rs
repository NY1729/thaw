// `new.target` inside class constructors resolves to the most-derived class (abstract guards,
// `.name`) and `Object.setPrototypeOf(this, new.target.prototype)` is accepted. Expected output
// from node 22.

#[test]
fn new_target_contract_constructor_chain() {
    let source = r#"abstract class Shape {
  kind: string;
  constructor() {
    if (new.target === Shape) throw new TypeError("abstract");
    this.kind = new.target.name;
  }
}
class Sq extends Shape {}
class Big extends Sq {
  constructor() {
    super();
    console.log(new.target === Big, new.target === Sq, new.target !== Sq);
  }
}
class MyErr extends Error {
  constructor(m: string) {
    super(m);
    Object.setPrototypeOf(this, new.target.prototype);
    this.name = new.target.name;
  }
}
class SubErr extends MyErr {}
function main(): void {
  console.log(new Sq().kind, new Big().kind);
  try { throw new SubErr("b"); } catch (e: any) { console.log(e instanceof SubErr, e instanceof MyErr, e.name, e.message); }
}
"#;
    assert_eq!(
        compile_and_run(source, "new_target_contract_constructor_chain"),
        "true false true\nSq Big\ntrue true SubErr b\n"
    );
}
