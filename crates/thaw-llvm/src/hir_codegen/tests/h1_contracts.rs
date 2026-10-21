// Wave H1: a subclass instance is accepted where its base class is expected.
// Expected output from node 22.

#[test]
fn h1_contract_subclass_instance_passes_as_base_class() {
    let source = r##"class Base {
  name(): string { return "base"; }
  read(x: Base): number { return 1; }
}
class Child extends Base {
  extra(): string { return "child"; }
}
function takes(b: Base): string { return b.name(); }
function make(): Base { return new Child(); }
async function main(): Promise<void> {
  console.log(new Base().read(new Child()));
  console.log(takes(new Child()));
  const list: Base[] = [new Child(), new Base()];
  console.log(list.length, list[0].name());
  const up: Base = make();
  console.log(up.name(), up instanceof Child);
  console.log(new Child().extra());
}
"##;
    assert_eq!(
        compile_and_run(source, "h1_contract_subclass_instance_passes_as_base_class"),
        "1\nbase\n2 base\nbase true\nchild\n"
    );
}
