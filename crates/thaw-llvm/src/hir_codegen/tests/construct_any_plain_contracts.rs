// `new p()` where `p: any` holds a compiled plain function declaration.
// Expected output recorded from node 22.

#[test]
fn construct_any_plain_function_declaration_runs_body() {
    let source = r##"function plain(): void { console.log("plain body ran"); }
function main(): void {
  const p: any = plain;
  try { const o = new p(); console.log("c2c ok", typeof o); } catch (e: any) { console.log("c2c threw", e && e.name, e && e.message); }
}
"##;
    assert_eq!(
        compile_and_run(source, "construct_any_plain_function_declaration_runs_body"),
        "plain body ran
c2c ok object
"
    );
}
