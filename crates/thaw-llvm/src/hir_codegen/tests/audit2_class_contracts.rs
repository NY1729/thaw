#[test]
fn audit2_abstract_base_unannotated_method_calls_abstract() {
    let source = r##"abstract class Base { abstract area(): number; d() { return this.area() + 1; } }
class Sq extends Base { area(): number { return 4; } }
function main(): void { console.log(new Sq().d()); const b: Base = new Sq(); console.log(b.d(), b.area()); }
"##;
    assert_eq!(compile_and_run(source, "audit2_abstract_unannotated"), "5\n5 4\n");
}
