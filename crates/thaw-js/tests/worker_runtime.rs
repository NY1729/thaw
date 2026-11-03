//! Worker threads, W2a: spawn, the event-loop poll, the wire, loop lifetime. Driven from JavaScript through the
//! `__thaw_worker_*` natives; the main script's `__thaw_worker_dispatch` logs each event, printed at the worker's exit.
use thaw_js::Interp;

/// Runs a main script that spawns `source` as a worker, then `after_spawn` (with the handle as `h`).
fn run_worker(source: &str, after_spawn: &str) -> String {
    let main = format!(
        r#"const seen = [];
        globalThis.__thaw_worker_dispatch = (json) => {{
          for (const e of JSON.parse(json)) {{
            if (e.type === 'exit') seen.push('exit:' + e.code);
            else if (e.type === 'error') seen.push('error:' + __thaw_worker_decode(e.value).message);
            else if (e.type === 'message') seen.push('message:' + JSON.stringify(__thaw_worker_decode(e.value)));
            else seen.push(e.type);
            if (e.type === 'exit') console.log(seen.join('|'));
          }}
        }};
        const h = __thaw_worker_spawn({source:?});
        {after_spawn}"#
    );
    let mut it = Interp::new();
    it.run(&main).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

#[test]
fn spawned_worker_reports_online_then_exit_zero() {
    assert_eq!(run_worker("1 + 1", ""), "online|exit:0");
}

#[test]
fn uncaught_worker_error_is_reported_then_exit_one() {
    assert_eq!(
        run_worker("throw new Error('boom')", ""),
        "online|error:boom|exit:1"
    );
}

#[test]
fn process_exit_and_exit_code_set_the_worker_exit_code() {
    assert_eq!(run_worker("process.exit(5)", ""), "online|exit:5");
    assert_eq!(run_worker("process.exitCode = 3", ""), "online|exit:3");
}

#[test]
fn worker_posts_a_structured_clone_to_the_parent() {
    assert_eq!(
        run_worker(
            "__thaw_worker_send(__thaw_worker_encode({ a: [1, 'x'] }))",
            ""
        ),
        r#"online|message:{"a":[1,"x"]}|exit:0"#
    );
}

#[test]
fn parent_message_reaches_the_worker_dispatch() {
    let worker = "globalThis.__thaw_worker_dispatch = (json) => { for (const e of JSON.parse(json)) if (e.type === 'message') __thaw_worker_send(__thaw_worker_encode(__thaw_worker_decode(e.value) * 2)); }; setTimeout(() => {}, 50);";
    assert_eq!(
        run_worker(worker, "__thaw_worker_post(h, __thaw_worker_encode(21));"),
        "online|message:42|exit:0"
    );
}

#[test]
fn refed_worker_keeps_the_parent_loop_alive() {
    let worker = "setTimeout(() => __thaw_worker_send(__thaw_worker_encode('late')), 100);";
    assert_eq!(run_worker(worker, ""), r#"online|message:"late"|exit:0"#);
}

#[test]
fn unrefed_worker_does_not_keep_the_parent_loop_alive() {
    let worker = "setTimeout(() => __thaw_worker_send(__thaw_worker_encode('late')), 300);";
    assert_eq!(run_worker(worker, "__thaw_worker_ref(h, false);"), "");
}

#[test]
fn wire_round_trips_the_structured_clone_types() {
    let src = "const v = { a: [1, 'x', null, undefined, -0, NaN, Infinity, 10n], m: new Map([[1, 'one']]), s: new Set([2]), d: new Date(0), r: /ab+c/gi, u: new Uint8Array([1, 2, 3]), e: new RangeError('bad') };
      v.self = v;
      const c = __thaw_worker_decode(__thaw_worker_encode(v));
      const parts = [c.a[0], c.a[1], c.a[2], c.a[3] === undefined, Object.is(c.a[4], -0), c.a[5] !== c.a[5], c.a[6], c.a[7], c.m.get(1), c.s.has(2), c.d.getTime(), String(c.r), c.u.join(','), c.e instanceof RangeError, c.e.message, c.self === c];
      console.log(parts.map(String).join('|'));";
    let mut it = Interp::new();
    it.run(src).expect("round trip runs");
    assert_eq!(
        it.out.join("|"),
        "1|x|null|true|true|true|Infinity|10|one|true|0|/ab+c/gi|1,2,3|true|bad|true"
    );
}

#[test]
fn interp_startup_cost_per_thread_is_measured() {
    let started = std::time::Instant::now();
    let _it = Interp::new();
    eprintln!("Interp::new took {:?}", started.elapsed());
}
