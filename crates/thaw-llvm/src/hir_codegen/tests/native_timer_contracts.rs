// Native timer queue contracts (setTimeout/setInterval/setImmediate lowered to thaw-runtime's queue,
// no JS context). Expected output was recorded from Node 22.23.

/// Compiles with native timers enabled, asserts the program needs no JS engine, then runs it.
fn native_timer_run(source: &str, name: &str) -> (String, String, Option<i32>) {
    thaw_hir::set_native_timers(true);
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module);
    thaw_hir::set_native_timers(false);
    let program = program.unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, name);
    compiler.compile_program(&program).unwrap();
    assert!(compiler.uses_native_timers(), "timers must lower natively");
    assert!(!compiler.uses_quickjs(), "native timers must not create the JS realm");
    assert!(!compiler.native_timers_conflict());
    drop(compiler);
    thaw_hir::set_native_timers(true);
    let result = std::panic::catch_unwind(|| compile_and_run_failing(source, name));
    thaw_hir::set_native_timers(false);
    result.unwrap()
}

fn native_timer_ok(source: &str, name: &str) -> String {
    let (stdout, stderr, code) = native_timer_run(source, name);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    stdout
}

#[test]
fn native_timers_fire_in_due_then_creation_order() {
    let source = r#"function main(): void {
  setTimeout(() => console.log("t10"), 10);
  setTimeout(() => console.log("t0"), 0);
  setTimeout(() => console.log("t1"), 1);
  setTimeout(() => console.log("t5"), 5);
  setTimeout(() => console.log("t2a"), 2);
  setTimeout(() => console.log("t2b"), 2);
  console.log("sync");
}"#;
    assert_eq!(
        native_timer_ok(source, "native_timer_order"),
        "sync\nt0\nt1\nt2a\nt2b\nt5\nt10\n"
    );
}

#[test]
fn native_interval_stops_when_cleared_in_its_callback() {
    let source = r#"function main(): void {
  let n = 0;
  const id = setInterval(() => {
    n++;
    console.log("tick " + n);
    if (n === 3) { clearInterval(id); console.log("cleared"); }
  }, 5);
  const gone = setTimeout(() => console.log("never"), 3);
  clearTimeout(gone);
  const imm = setImmediate(() => console.log("never imm"));
  clearImmediate(imm);
}"#;
    assert_eq!(
        native_timer_ok(source, "native_timer_interval"),
        "tick 1\ntick 2\ntick 3\ncleared\n"
    );
}

#[test]
fn native_unref_ref_and_nested_timers() {
    let source = r#"function main(): void {
  setTimeout(() => console.log("unref never"), 500).unref();
  const t = setTimeout(() => console.log("kept"), 80);
  console.log(t.hasRef());
  t.unref();
  console.log(t.hasRef());
  t.ref();
  console.log(t.hasRef());
  setTimeout(() => {
    console.log("outer");
    setTimeout(() => {
      console.log("inner");
      setTimeout(() => console.log("innermost"), 0);
    }, 0);
  }, 2);
}"#;
    assert_eq!(
        native_timer_ok(source, "native_timer_unref"),
        "true\nfalse\ntrue\nouter\ninner\ninnermost\nkept\n"
    );
}

#[test]
fn native_timers_drain_promise_jobs_between_callbacks() {
    let source = r#"function main(): void {
  setTimeout(() => { console.log("a"); Promise.resolve().then(() => console.log("a micro")); }, 1);
  setTimeout(() => { console.log("b"); Promise.resolve().then(() => console.log("b micro")); }, 1);
  setTimeout(() => console.log("c"), 1);
  Promise.resolve().then(() => console.log("main micro"));
}"#;
    assert_eq!(
        native_timer_ok(source, "native_timer_microtasks"),
        "main micro\na\na micro\nb\nb micro\nc\n"
    );
}

#[test]
fn native_set_immediate_runs_before_a_timer_scheduled_from_a_timer() {
    let source = r#"function main(): void {
  setTimeout(() => {
    console.log("timer");
    setImmediate(() => console.log("imm1"));
    setTimeout(() => console.log("timeout0"), 0);
    setImmediate(() => console.log("imm2"));
  }, 1);
}"#;
    assert_eq!(
        native_timer_ok(source, "native_timer_immediate"),
        "timer\nimm1\nimm2\ntimeout0\n"
    );
}

#[test]
fn native_timer_exception_is_fatal_and_skips_later_timers() {
    let source = r#"function main(): void {
  setTimeout(() => { console.log("throws"); throw new Error("boom"); }, 1);
  setTimeout(() => console.log("after"), 5);
}"#;
    let (stdout, stderr, code) = native_timer_run(source, "native_timer_throw");
    assert_eq!(stdout, "throws\n");
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(stderr.contains("boom"), "{stderr}");
}

#[test]
fn timers_combined_with_a_js_engine_conflict_and_keep_the_quickjs_path() {
    thaw_hir::set_native_timers(true);
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
  setTimeout(() => console.log("native"), 1);
  setTimeout((a: number) => console.log("with arg", a), 1, 7);
}"#,
    )
    .unwrap();
    let program = thaw_hir::lower_module(&module);
    thaw_hir::set_native_timers(false);
    let program = program.unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "native_timer_conflict");
    compiler.compile_program(&program).unwrap();
    assert!(compiler.uses_native_timers());
    assert!(compiler.uses_quickjs());
    assert!(compiler.native_timers_conflict(), "the CLI must rebuild this on QuickJS timers");
}

// A self-referencing `any` timer handle: the callback captures the handle while its storage is still being
// decided (thaw-hir pre-binds it with the final JsValue storage type). Expected output recorded from Node 22.23.

#[test]
fn any_set_interval_self_clear_prints_once() {
    let source = r#"function main(): void {
  const iv: any = setInterval((): void => { clearInterval(iv); console.log("once"); }, 5);
}"#;
    assert_eq!(compile_and_run(source, "any_interval_self_clear"), "once\n");
}

#[test]
fn any_set_timeout_self_clear_prints_once() {
    let source = r#"function main(): void {
  const t: any = setTimeout((): void => { clearTimeout(t); console.log("once"); }, 5);
}"#;
    assert_eq!(compile_and_run(source, "any_timeout_self_clear"), "once\n");
}

#[test]
fn any_interval_counter_clears_itself_after_three_ticks() {
    let source = r#"function main(): void {
  let n = 0;
  const iv: any = setInterval((): void => { n++; if (n === 3) clearInterval(iv); }, 5);
  setTimeout((): void => console.log("n", n), 150);
}"#;
    assert_eq!(compile_and_run(source, "any_interval_counter"), "n 3\n");
}

#[test]
fn any_timer_handle_cleared_from_outside_its_callback() {
    let source = r#"function main(): void {
  let ticks = 0;
  const iv: any = setInterval((): void => { ticks++; }, 5);
  let at = 0;
  setTimeout((): void => { clearInterval(iv); at = ticks; }, 50);
  setTimeout((): void => console.log("stopped", ticks === at && at > 0), 120);
}"#;
    assert_eq!(compile_and_run(source, "any_timer_outside_clear"), "stopped true\n");
    let cleared = r#"function main(): void {
  const t: any = setTimeout((): void => console.log("never"), 50);
  clearTimeout(t);
  console.log("cleared");
}"#;
    assert_eq!(compile_and_run(cleared, "any_timeout_outside_clear"), "cleared\n");
}

// A timer global read as a value is the realm's function (Node 22: `setTimeout === globalThis.setTimeout`).
#[test]
fn timer_global_values_are_the_realm_functions() {
    let source = r#"function main(): void {
  const same: boolean = setTimeout === globalThis.setTimeout && clearTimeout === globalThis.clearTimeout;
  const kind: string = typeof setTimeout;
  console.log(same, kind, setImmediate === globalThis.setImmediate);
}"#;
    assert_eq!(compile_and_run(source, "timer_global_values_are_the_realm_functions").trim(), "true function true");
}
