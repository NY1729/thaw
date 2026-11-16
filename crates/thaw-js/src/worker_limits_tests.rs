//! Worker options (plan B1): the Error `code` on the worker wire, the `resourceLimits` getter (shapes recorded from
//! Node 22.23.1), and `stackSizeMb` as the worker thread's stack. Limits are not enforced yet (B2).

use super::*;

/// Runs `src` on a fresh Interp and returns its console lines (`it.out` is moved out, not borrowed).
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    it.run_event_loop().expect("event loop runs");
    std::mem::take(&mut it.out)
}

#[test]
fn worker_stack_bytes_follow_stack_size_mb() {
    const MIB: usize = 1 << 20;
    // unset (or not a positive number) keeps the 64 MiB worker stack of before
    assert_eq!(worker::stack_bytes(f64::NAN), 64 * MIB);
    assert_eq!(worker::stack_bytes(0.0), 64 * MIB);
    assert_eq!(worker::stack_bytes(-1.0), 64 * MIB);
    // Node's default 4 MiB stack keeps the 64 MiB default; a smaller one gives a smaller stack
    assert_eq!(worker::stack_bytes(4.0), 64 * MIB);
    assert_eq!(worker::stack_bytes(1.0), 16 * MIB);
    // clamped to [8 MiB, 512 MiB]
    assert_eq!(worker::stack_bytes(0.1), 8 * MIB);
    assert_eq!(worker::stack_bytes(1000.0), 512 * MIB);
}

#[test]
fn worker_error_code_round_trips_the_wire() {
    let src = "const e = Object.assign(new Error('x'), { code: 'E_X' });
      const back = __thaw_worker_decode(__thaw_worker_encode(e));
      const plain = __thaw_worker_decode(__thaw_worker_encode(new TypeError('no code')));
      console.log([back.name, back.message, back.code, plain.code === undefined, plain.name].join(' '));";
    assert_eq!(lines(src), ["Error x E_X true TypeError"]);
}

#[test]
fn worker_error_event_carries_the_error_code() {
    // Node 22.23.1: `error x E_X Error` then `exit 1`
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`throw Object.assign(new Error('x'), { code: 'E_X' });`, { eval: true });
        worker.on('error', (e) => console.log('error ' + e.message + ' ' + e.code + ' ' + e.name));
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(lines(src), ["error x E_X Error", "exit 1"]);
}

#[test]
fn worker_resource_limits_getter_matches_node_22() {
    // Node 22.23.1, read right after construction (the values V8 reports once the worker runs are not reproduced)
    let src = "const wt = __thaw_worker_threads_exports;
      const a = new wt.Worker('1', { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });
      const b = new wt.Worker('1', { eval: true });
      const c = new wt.Worker('1', { eval: true, resourceLimits: { stackSizeMb: 8 } });
      console.log(JSON.stringify(a.resourceLimits));
      console.log(JSON.stringify(b.resourceLimits));
      console.log(JSON.stringify(c.resourceLimits));
      for (const w of [a, b, c]) w.terminate();";
    assert_eq!(
        lines(src),
        [
            r#"{"maxYoungGenerationSizeMb":-1,"maxOldGenerationSizeMb":16,"codeRangeSizeMb":-1,"stackSizeMb":4}"#,
            r#"{"maxYoungGenerationSizeMb":-1,"maxOldGenerationSizeMb":-1,"codeRangeSizeMb":-1,"stackSizeMb":4}"#,
            r#"{"maxYoungGenerationSizeMb":-1,"maxOldGenerationSizeMb":-1,"codeRangeSizeMb":-1,"stackSizeMb":8}"#,
        ]
    );
}

#[test]
fn worker_with_a_small_stack_size_runs_shallow_recursion() {
    // Node 22.23.1 with stackSizeMb: 1 prints `shallow 100`. Deep recursion is not tested here: a worker overflows
    // its native stack on unbounded recursion even with the default stack (a pre-existing gap, see the report).
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads');
          function g(n) { return n === 0 ? 0 : 1 + g(n - 1); }
          parentPort.postMessage('shallow ' + g(100));`,
          { eval: true, resourceLimits: { stackSizeMb: 1 } });
        worker.on('message', (m) => console.log(m));
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(lines(src), ["shallow 100"]);
}
