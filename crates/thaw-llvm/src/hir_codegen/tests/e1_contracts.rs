// Wave E1: an unset process.env variable is `undefined`, not the empty string.
// Expected output from node 22.

#[test]
fn e1_contract_unset_env_var_is_undefined() {
    let source = r##"function main(): void {
  console.log(process.env.E1_UNSET_VAR_X ?? "3000");
  console.log(process.env.E1_UNSET_VAR_X || "fallback");
  console.log(process.env.E1_UNSET_VAR_X === undefined);
  console.log(typeof process.env.E1_UNSET_VAR_X);
  process.env.E1_SET_VAR_Y = "hello";
  console.log(process.env.E1_SET_VAR_Y, typeof process.env.E1_SET_VAR_Y);
  delete process.env.E1_SET_VAR_Y;
  console.log(process.env.E1_SET_VAR_Y === undefined, process.env.E1_SET_VAR_Y ?? "gone");
  console.log(Number(process.env.E1_UNSET_VAR_X) || 3000);
  console.log(Object.keys(process.env).includes("E1_UNSET_VAR_X"));
}
"##;
    assert_eq!(
        compile_and_run(source, "e1_contract_unset_env_var_is_undefined"),
        "3000\nfallback\ntrue\nundefined\nhello string\ntrue gone\n3000\nfalse\n"
    );
}
