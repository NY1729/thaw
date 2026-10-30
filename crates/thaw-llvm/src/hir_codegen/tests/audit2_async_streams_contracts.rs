// Audit2 async/streams rows. Expected output recorded from node 22 (--experimental-strip-types).

#[test]
fn async_streams_for_await_over_process_stdin_ends_with_empty_stdin() {
    let source = r##"async function main(): Promise<void> {
  let n = 0;
  for await (const chunk of process.stdin) n += chunk.length;
  console.log("stdin chars", n);
}
"##;
    assert_eq!(
        compile_and_run(source, "async_streams_for_await_over_process_stdin_ends_with_empty_stdin"),
        "stdin chars 0\n"
    );
}

#[test]
fn async_streams_async_generator_with_timer_await_runs_under_for_await() {
    let source = r##"const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
async function* gen() { await sleep(1); yield 1; await sleep(1); yield 2; }
async function main() {
  for await (const v of gen()) console.log("g", v);
  console.log("end");
  for await (const v of gen()) { if (v === 1) break; }
  console.log("broke");
}
"##;
    assert_eq!(
        compile_and_run(source, "async_streams_async_generator_with_timer_await_runs_under_for_await"),
        "g 1\ng 2\nend\nbroke\n"
    );
}

#[test]
fn async_streams_catch_with_a_void_callback_widens_the_promise_result() {
    let source = r##"async function fail(): Promise<number> { throw new Error("x"); }
async function main(): Promise<void> {
  const ok: Promise<number> = Promise.resolve(1);
  const swallowed = ok.catch(() => {});
  const failed = fail().catch(() => {});
  console.log(await swallowed, await failed);
  const mapped = fail().catch((e: Error) => e.message.length);
  console.log(await mapped);
  const chain = Promise.resolve().catch(() => 5);
  console.log(await chain);
}
"##;
    assert_eq!(
        compile_and_run(source, "async_streams_catch_with_a_void_callback_widens_the_promise_result"),
        "1 undefined\n1\nundefined\n"
    );
}

#[test]
fn async_streams_instanceof_promise_on_an_any_typed_js_promise() {
    let source = r##"function main(): void {
  const P: any = (globalThis as any).Promise;
  const p: any = new P((r: any) => r(1));
  const o: any = {};
  console.log(p instanceof Promise, o instanceof Promise);
}
"##;
    assert_eq!(
        compile_and_run(source, "async_streams_instanceof_promise_on_an_any_typed_js_promise"),
        "true false\n"
    );
}

#[test]
fn async_streams_throw_inside_next_tick_exits_one_without_aborting() {
    let source = r##"function main(): void {
  process.nextTick(() => { throw new Error("from tick"); });
  console.log("sync done");
}
"##;
    let (stdout, stderr, code) =
        compile_and_run_failing(source, "async_streams_throw_inside_next_tick_exits_one_without_aborting");
    assert_eq!(stdout, "sync done\n");
    assert!(stderr.contains("Error: from tick"), "stderr: {stderr}");
    assert_eq!(code, Some(1), "stderr: {stderr}");
}
