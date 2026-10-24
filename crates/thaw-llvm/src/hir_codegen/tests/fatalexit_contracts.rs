// Fatal-exit and out-of-range compound index semantics. Expected output from node 22.

#[test]
fn bytes_out_of_range_compound_and_update_index_are_ignored() {
    let source = r#"
function main(): void {
  const u = new Uint8Array(4);
  const i = 10;
  u[10] += 1;
  u[10]++;
  ++u[10];
  u[i] |= 1;
  u[-1] -= 1;
  u[2] += 5;
  u[2]++;
  console.log(u.length, u[10], u[2]);
}
"#;
    let output = compile_and_run(source, "bytes_oob_compound_update_ignored");
    assert_eq!(output, "4 undefined 6\n");
}

#[test]
fn fatal_sync_throw_in_main_skips_registered_timers() {
    let source = r##"function main(): void {
  setTimeout(() => console.log("timer"), 5);
  console.log("a");
  throw new Error("boom");
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(source, "fatalexit_sync_throw_timers");
    assert_eq!((stdout.as_str(), code), ("a\n", Some(1)));
}

#[test]
fn fatal_sync_throw_in_main_exit_handler_sees_code_1() {
    let source = r##"function main(): void {
  process.on("exit", (code: number) => console.log("exit", code));
  setTimeout(() => console.log("timer"), 5);
  console.log("a");
  throw new Error("boom");
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(source, "fatalexit_sync_throw_exit_code");
    assert_eq!((stdout.as_str(), code), ("a\nexit 1\n", Some(1)));
}
