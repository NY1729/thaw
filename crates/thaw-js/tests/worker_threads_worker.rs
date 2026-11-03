//! `Worker` on thaw-js (W3): one test per node-compat case that flips, plus the W3 extras. Each runs a main script
//! on `Interp` with the real `__thaw_worker_threads_exports` and the real worker threads of worker.rs.
use thaw_js::Interp;

/// Runs `src` on the main thread and its event loop; returns the main thread's console output joined by `|`.
fn run_main(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

#[test]
fn worker_evaluated_thread_posts_a_message_then_terminates() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.postMessage(42);`, { eval: true });
        await new Promise((resolve, reject) => { worker.once('message', (value) => { console.log(value); resolve(); }); worker.once('error', reject); });
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "42");
}

#[test]
fn worker_inherits_environment_data() {
    let src = "const wt = __thaw_worker_threads_exports;
      wt.setEnvironmentData('w2.key', { a: 1 });
      (async () => {
        const worker = new wt.Worker(`const { parentPort, getEnvironmentData } = require('node:worker_threads'); parentPort.postMessage(JSON.stringify(getEnvironmentData('w2.key')) + ' ' + typeof getEnvironmentData('missing'));`, { eval: true });
        const text = await new Promise((resolve) => { worker.once('message', (v) => resolve(String(v))); });
        console.log(text);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), r#"{"a":1} undefined"#);
}

#[test]
fn parent_port_unref_lets_the_worker_exit() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.on('message', () => {}); parentPort.unref(); parentPort.postMessage('ready');`, { eval: true });
        const log = [];
        worker.on('message', (v) => { log.push('message ' + v); });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        log.push('exit ' + code);
        console.log(log.join('|'));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "message ready|exit 0");
}

#[test]
fn worker_natural_exit_keeps_process_exit_code() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`process.exitCode = 3;`, { eval: true });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "exit 3");
}

#[test]
fn receive_message_on_port_reads_the_parent_port_in_a_worker() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort, receiveMessageOnPort } = require('node:worker_threads'); parentPort.postMessage('ready'); const wait = () => { const r = receiveMessageOnPort(parentPort); if (r) { parentPort.postMessage(JSON.stringify(r.message)); parentPort.close(); } else setTimeout(wait, 5); }; wait();`, { eval: true });
        worker.on('message', (v) => { if (v === 'ready') worker.postMessage({ n: 7 }); else console.log(v); });
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), r#"{"n":7}"#);
}

#[test]
fn parent_port_close_keeps_pending_work_alive() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.on('message', () => {}); parentPort.close(); setTimeout(() => { console.log('timer ran'); }, 30);`, { eval: true, stdout: false });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "exit 0");
}

#[test]
fn parent_port_onmessage_handler_runs() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.onmessage = (event) => { parentPort.postMessage('got ' + JSON.stringify(event.data)); parentPort.close(); };`, { eval: true });
        worker.on('message', (v) => { console.log(v); });
        worker.postMessage({ k: 1 });
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), r#"got {"k":1}"#);
}

#[test]
fn worker_workerdata_is_structured_cloned() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { workerData, parentPort } = require('node:worker_threads'); parentPort.postMessage(JSON.stringify([workerData.m instanceof Map, workerData.d instanceof Date, workerData.u instanceof Uint8Array, workerData.m.get('a')]));`,
          { eval: true, workerData: { m: new Map([['a', 1]]), d: new Date(0), u: new Uint8Array([1, 2]) } });
        const text = await new Promise((resolve) => { worker.once('message', (v) => resolve(v)); });
        console.log(text);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "[true,true,true,1]");
}

#[test]
fn uncaught_worker_error_is_emitted_then_exit_one() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`throw new Error('x');`, { eval: true });
        worker.on('error', (e) => console.log(e.name + ' ' + e.message));
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "Error x|exit 1");
}

#[test]
fn worker_error_without_a_listener_is_uncaught_in_the_main_thread() {
    let mut it = Interp::new();
    it.run("const wt = __thaw_worker_threads_exports; new wt.Worker(`throw new Error('boom');`, { eval: true });")
        .expect("main script runs");
    assert!(it.run_event_loop().is_err());
}

#[test]
fn post_message_after_exit_is_a_no_op() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('1 + 1', { eval: true });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        worker.postMessage('late');
        console.log('exit ' + code + ' ok');
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "exit 0 ok");
}

#[test]
fn two_workers_get_distinct_thread_ids() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const a = new wt.Worker('1', { eval: true });
        const b = new wt.Worker('1', { eval: true });
        const exits = await Promise.all([
          new Promise((resolve) => a.on('exit', (c) => resolve(c))),
          new Promise((resolve) => b.on('exit', (c) => resolve(c))),
        ]);
        console.log(a.threadId + ' ' + b.threadId + ' ' + exits.join(','));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "1 2 0,0");
}

#[test]
fn terminate_returns_the_same_promise_and_resolves_with_the_exit_code() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('while (true) {}', { eval: true });
        const first = worker.terminate();
        console.log('same ' + (first === worker.terminate()));
        console.log('terminated ' + await first);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "same true|terminated 1");
}

#[test]
fn terminate_stops_a_busy_worker_within_the_time_limit() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('while (true) {}', { eval: true });
        await new Promise((resolve) => setTimeout(resolve, 100));
        console.log('terminated ' + await worker.terminate());
      })().catch((e) => console.log('failed ' + e));";
    let started = std::time::Instant::now();
    assert_eq!(run_main(src), "terminated 1");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn worker_without_eval_is_rejected_with_err_worker_path() {
    let src = "const wt = __thaw_worker_threads_exports;
      try { new wt.Worker('1.js'); } catch (e) { console.log(e.code); }";
    assert_eq!(run_main(src), "ERR_WORKER_PATH");
}

/// The compiled path awaits a Promise result with `Interp::await_settled` (typed host calls), not `run_event_loop`.
/// The worker's exit must be polled there too, or an awaited `terminate()` never settles.
#[test]
fn awaited_terminate_settles_through_await_settled() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut it = Interp::new();
        let promise = it
            .run("(async () => { const wt = __thaw_worker_threads_exports; const worker = new wt.Worker('while (true) {}', { eval: true }); await new Promise((resolve) => setTimeout(resolve, 100)); return worker.terminate(); })();")
            .expect("main script runs");
        let text = match it.await_settled(&promise).expect("loop runs") {
            Some(Ok(thaw_js::Value::Number(code))) => code.to_string(),
            _ => "unexpected".to_string(),
        };
        let _ = tx.send(text);
    });
    let text = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("await_settled settles the awaited terminate()");
    assert_eq!(text, "1");
}
