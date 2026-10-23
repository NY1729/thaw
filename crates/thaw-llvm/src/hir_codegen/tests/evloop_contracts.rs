// Event-loop ordering contracts. Expected output recorded from node 22.

#[test]
fn evloop_contract_zero_ms_timeout_counts_as_one_ms_and_keeps_insertion_order() {
    let source = r##"function main(): void {
  setTimeout(() => console.log("a1"), 1);
  setTimeout(() => console.log("a0"), 0);
  setTimeout(() => console.log("b1"), 1);
  setTimeout(() => console.log("c2"), 2);
}
"##;
    assert_eq!(compile_and_run(source, "evloop_contract_zero_ms_timeout"), "a1\na0\nb1\nc2\n");
}

#[test]
fn evloop_contract_before_exit_can_rearm_and_exit_runs_once_last() {
    let source = r##"function main(): void {
  let n = 0;
  process.on("beforeExit", (code: number) => {
    console.log("beforeExit", code, n);
    if (n++ < 1) setTimeout(() => console.log("more"), 1);
  });
  process.on("exit", (code: number) => { console.log("exit", code); setTimeout(() => console.log("never"), 0); });
  console.log("main");
}
"##;
    assert_eq!(
        compile_and_run(source, "evloop_contract_before_exit_and_exit"),
        "main\nbeforeExit 0 0\nmore\nbeforeExit 0 1\nexit 0\n"
    );
}

#[test]
fn evloop_contract_unhandled_rejection_stops_the_process_before_later_timers() {
    let source = r##"function main(): void {
  Promise.reject(new Error("x"));
  setTimeout(() => console.log("after"), 10);
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(source, "evloop_contract_unhandled_before_timers");
    assert_eq!((stdout.as_str(), code), ("", Some(1)));
}

#[test]
fn evloop_contract_process_version_is_v22_while_versions_node_stays_empty() {
    // versions.node is deliberately empty (see runtime.js); node 22 prints a non-empty one, so only
    // the process.version half is a node comparison.
    let source = r##"function main(): void {
  console.log(process.version.startsWith("v22."), typeof process.versions.node);
}
"##;
    assert_eq!(compile_and_run(source, "evloop_contract_process_version"), "true string\n");
}

#[test]
fn evloop_contract_awaiting_main_stops_at_an_unhandled_rejection() {
    let source = r##"async function f(): Promise<void> { throw new Error("x"); }
async function main(): Promise<void> {
  f();
  await new Promise<void>((r) => setTimeout(r, 10));
  console.log("after");
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(source, "evloop_contract_await_unhandled");
    assert_eq!((stdout.as_str(), code), ("", Some(1)));
}

#[test]
fn evloop_contract_exit_is_emitted_with_code_1_on_uncaught_and_unhandled() {
    let uncaught = r##"function main(): void {
  process.on("exit", (code: number) => console.log("exit", code));
  console.log("a");
  setTimeout(() => { throw new Error("boom"); }, 5);
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(uncaught, "evloop_contract_exit_uncaught");
    assert_eq!((stdout.as_str(), code), ("a\nexit 1\n", Some(1)));
    let rejected = r##"async function f(): Promise<void> { throw new Error("x"); }
function main(): void {
  process.on("exit", (code: number) => console.log("exit", code));
  f();
  setTimeout(() => console.log("timer"), 50);
  console.log("a");
}
"##;
    let (stdout, _stderr, code) = compile_and_run_failing(rejected, "evloop_contract_exit_unhandled");
    assert_eq!((stdout.as_str(), code), ("a\nexit 1\n", Some(1)));
}
