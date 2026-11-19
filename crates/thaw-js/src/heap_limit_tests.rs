//! Worker `resourceLimits` enforcement (plan B3): the byte accounting of `heap_limit`, run in-process on the test
//! thread, and the worker-level OOM event. Each test resets the thread's accounting when it ends.

use super::*;
use crate::compile::CompileMode;

/// Turns the accounting off when the test ends (its state is per thread).
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        heap_limit::reset();
    }
}

/// Runs `src` on a fresh Interp and returns its console lines (`it.out` is moved out, not borrowed).
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    it.run_event_loop().expect("event loop runs");
    std::mem::take(&mut it.out)
}

/// Allocates and drops buffers and arrays: 10k iterations of about 112 KiB churn each, with one iteration's worth
/// live at a time.
const GARBAGE: &str = "for (let i = 0; i < 10000; i++) { const a = new ArrayBuffer(64 << 10); const b = new Array(2048).fill(i); }";

#[test]
fn heap_limit_worker_runaway_allocation_terminates_with_oom_error() {
    // Node 22.23.1: `error ERR_WORKER_OUT_OF_MEMORY Error Worker terminated due to reaching memory limit: JS heap out
    // of memory`, then `exit 1`, and the parent carries on
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const keep = []; while (true) { keep.push(new Array(100000).fill(1)); }`, { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });
        worker.on('error', (e) => console.log('error ' + e.code + ' ' + e.name + ' ' + e.message));
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
        console.log('parent ok');
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(
        lines(src),
        [
            "error ERR_WORKER_OUT_OF_MEMORY Error Worker terminated due to reaching memory limit: JS heap out of memory",
            "exit 1",
            "parent ok",
        ]
    );
}

#[test]
fn heap_limit_worker_under_the_limit_is_not_stopped() {
    // Node 22.23.1 prints `fine 1000` then `exit 0`: a limit that is not reached does not trip
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); let sum = 0; for (let i = 0; i < 1000; i++) { const a = new Array(1000).fill(i); sum += a.length; } parentPort.postMessage('fine ' + sum);`, { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });
        worker.on('message', (m) => console.log(m));
        worker.on('error', (e) => console.log('error ' + e.code));
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(lines(src), ["fine 1000000", "exit 0"]);
}

#[test]
fn heap_limit_garbage_under_the_limit_does_not_trip() {
    // credits on drop: 10k iterations of churn far above the 16 MiB limit, but almost nothing is live at once
    let _reset = Reset;
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    heap_limit::enable(16 << 20);
    match it.run(GARBAGE) {
        Ok(_) => {}
        Err(_) => panic!("garbage loop must not be stopped"),
    }
    assert!(!heap_limit::tripped());
    assert!(heap_limit::used() < 16 << 20, "used {}", heap_limit::used());
}

#[test]
fn heap_limit_huge_array_buffer_trips() {
    let _reset = Reset;
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    heap_limit::enable(16 << 20);
    let r = it.run("new ArrayBuffer(1 << 30);");
    assert!(
        matches!(r, Err(Ctl::Exit(_))),
        "a 1 GiB buffer is stopped at the limit"
    );
    assert!(heap_limit::tripped());
}

#[test]
fn heap_limit_off_leaves_no_accounting() {
    // no limit: the same garbage loop passes and nothing is counted (the no-limit path is one bool read)
    let _reset = Reset;
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    heap_limit::reset();
    match it.run(GARBAGE) {
        Ok(_) => {}
        Err(_) => panic!("garbage loop must pass without a limit"),
    }
    assert!(!heap_limit::enabled());
    assert_eq!(heap_limit::used(), 0);
    assert!(!heap_limit::tripped());
}

#[test]
fn v8_memory_usage_grows_with_live_objects_and_falls_when_they_are_dropped() {
    // `__thaw_memory_usage` (the registry's v8 and memoryUsage figures): the estimate over the live objects, which
    // is always on; the resourceLimits accounting is not involved
    let src = "const used = () => JSON.parse(__thaw_memory_usage()).memory_used_size;
      used();
      const quiet0 = used();
      const quiet1 = used();
      const before = used();
      const keep = [];
      for (let i = 0; i < 200; i++) keep.push(new Array(50000).fill(i));
      const after = used();
      keep.length = 0;
      const dropped = used();
      console.log(before > 0);
      console.log(Math.abs(quiet1 - quiet0) < (1 << 20));
      console.log(after - before > (10 << 20));
      console.log(after - dropped > (10 << 20));";
    assert_eq!(lines(src), ["true", "true", "true", "true"]);
}

#[test]
fn v8_memory_usage_counts_objects_allocated_after_an_await() {
    // allocated and read inside one async body: it runs on one coroutine, so both reads see the same registry. A read
    // on another thread than the allocation misses it (see the `sysio` native's comment)
    let src = "const used = () => JSON.parse(__thaw_memory_usage()).memory_used_size;
      (async () => {
        await null;
        const before = used();
        const keep = [];
        for (let i = 0; i < 200; i++) keep.push(new Array(50000).fill(i));
        console.log(used() - before > (10 << 20));
        console.log(keep.length);
      })();";
    assert_eq!(lines(src), ["true", "200"]);
}
