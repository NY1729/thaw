// Exit-code and fetch-typing contracts. Expected behavior recorded from node 22.

#[test]
fn exitfix_contract_sync_throw_from_main_exits_one_and_prints_to_stderr() {
    let source = r##"function main(): void {
  console.log("before");
  throw new Error("boom");
}
"##;
    let (stdout, stderr, code) = compile_and_run_failing(source, "exitfix_sync_throw");
    assert_eq!((stdout.as_str(), code), ("before\n", Some(1)));
    assert!(stderr.contains("boom"), "stderr: {stderr}");
}

/// The runtime half (status/ok/text against a local server) is the node-compat case
/// "unannotated await fetch is a Response"; here the property reads must merely type-check.
#[test]
fn exitfix_contract_unannotated_await_fetch_is_a_response() {
    let source = r##"async function main(): Promise<void> {
  const r = await fetch("http://127.0.0.1:1/");
  console.log(r.status, r.ok);
  console.log(await r.text());
}
"##;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "exitfix_unannotated_fetch");
    compiler.compile_program(&program).unwrap();
}
