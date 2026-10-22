// Private field read through a base-typed parameter on a subclass instance.
// Expected output from node 22.

#[test]
fn private_field_read_through_base_param_on_subclass_instance() {
    let source = r##"class P {
  #x = 1;
  read(p: P): number { return p.#x; }
}
class Q extends P { }
async function main(): Promise<void> {
  console.log(new P().read(new Q()));
  console.log(new P().read(new P()));
}
"##;
    assert_eq!(
        compile_and_run(source, "private_field_read_through_base_param_on_subclass_instance"),
        "1\n1\n"
    );
}
