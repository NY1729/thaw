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
    assert_eq!(run_main(src), "timer ran|exit 0");
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
    // Node 22.23.1: threadId reads -1 once a worker has exited
    assert_eq!(run_main(src), "-1 -1 0,0");
}

#[test]
fn terminate_returns_a_new_promise_per_call_that_resolves_with_the_exit_code() {
    // Node 22.23.1: each terminate() call returns its own promise; both resolve with the exit code
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('while (true) {}', { eval: true });
        await new Promise((resolve) => worker.once('online', resolve));
        const first = worker.terminate();
        const second = worker.terminate();
        console.log('same ' + (first === second));
        console.log('terminated ' + await first + ' ' + await second);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "same false|terminated 1 1");
}

#[test]
fn terminate_before_the_bootstrap_exits_0() {
    // Node 22.23.1: terminate() right after construction, before the worker started, exits 0
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('while (true) {}', { eval: true });
        console.log('terminated ' + await worker.terminate());
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "terminated 0");
}

#[test]
fn terminate_stops_a_busy_worker_within_the_time_limit() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('while (true) {}', { eval: true });
        await new Promise((resolve) => worker.once('online', resolve));
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
            .run("(async () => { const wt = __thaw_worker_threads_exports; const worker = new wt.Worker('while (true) {}', { eval: true }); await new Promise((resolve) => worker.once('online', resolve)); return worker.terminate(); })();")
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

#[test]
fn file_worker_runs_a_commonjs_file_with_relative_require() {
    let dir = std::env::temp_dir().join(format!("thaw-file-worker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.js"), "module.exports = 'from-helper';\n").unwrap();
    std::fs::write(
        dir.join("worker.js"),
        "const { parentPort } = require('node:worker_threads');\nparentPort.postMessage(require('./helper'));\n",
    )
    .unwrap();
    let file = dir.join("worker.js").display().to_string();
    let src = format!(
        "const wt = __thaw_worker_threads_exports;
      (async () => {{
        const worker = new wt.Worker({file:?});
        await new Promise((resolve, reject) => {{ worker.once('message', (value) => {{ console.log(value); resolve(); }}); worker.once('error', reject); }});
        await worker.terminate();
      }})().catch((e) => console.log('failed ' + e));"
    );
    assert_eq!(run_main(&src), "from-helper");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `body` as an eval `Worker` (no file, so its text goes to the CommonJS entry) and returns what it posted.
fn posted_by_eval_worker(body: &str) -> String {
    let src = format!(
        "const wt = __thaw_worker_threads_exports;
      (async () => {{
        const worker = new wt.Worker({body:?}, {{ eval: true }});
        await new Promise((resolve, reject) => {{ worker.once('message', (value) => {{ console.log(value); resolve(); }}); worker.once('error', reject); }});
        await worker.terminate();
      }})().catch((e) => console.log('failed ' + e));"
    );
    run_main(&src)
}

// Node 22 runs an eval worker's script sloppy (recorded 3 runs: `1,function,7,8,H,true`).
#[test]
fn cjs_eval_worker_main_is_sloppy_like_node() {
    let body = "const { parentPort } = require('node:worker_threads');
var o = {a: 1}; var r = [];
with (o) { r.push(a) }
function eval() {}
r.push(typeof eval);
var arguments = 7; r.push(arguments);
r.push(010);
r.push('H'); <!-- html comment
r.push((function(){ return this === globalThis })());
parentPort.postMessage(r.join(','));
";
    assert_eq!(posted_by_eval_worker(body), "1,function,7,8,H,true");
}

// Node 22 keeps a `'use strict'` eval worker's script strict (recorded 3 runs: `ReferenceError`).
#[test]
fn cjs_eval_worker_use_strict_stays_strict() {
    let body = "'use strict';
const { parentPort } = require('node:worker_threads');
let m; try { undeclaredVar = 1; m = 'sloppy' } catch (e) { m = e.name }
parentPort.postMessage(m);
";
    assert_eq!(posted_by_eval_worker(body), "ReferenceError");
}

#[test]
fn worker_data_shares_a_shared_array_buffer() {
    // node-compat case "worker data shares a SharedArrayBuffer": the worker's store is seen by the parent's view
    let src = "const wt = __thaw_worker_threads_exports;
      const shared = new SharedArrayBuffer(8); const view = new Int32Array(shared);
      (async () => {
        const worker = new wt.Worker(`const { parentPort, workerData } = require('node:worker_threads'); const view = new Int32Array(workerData); Atomics.store(view, 0, 42); parentPort.postMessage(workerData instanceof SharedArrayBuffer);`, { eval: true, workerData: shared });
        const kind = await new Promise((resolve) => { worker.once('message', (value) => resolve(value)); });
        await worker.terminate();
        console.log(kind, view[0]);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "true 42");
}

#[test]
fn worker_stack_overflow_is_a_catchable_range_error() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); function f() { return f(); } try { f(); } catch (e) { parentPort.postMessage(e.name + ' ' + e.message); }`, { eval: true });
        const text = await new Promise((resolve) => { worker.once('message', (v) => resolve(v)); });
        console.log(text);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "RangeError Maximum call stack size exceeded");
}

#[test]
fn two_workers_add_to_one_shared_counter_exactly() {
    // real worker threads: 2 x 20000 Atomics.add on one word of a shared block must total exactly 40000
    let src = "const wt = __thaw_worker_threads_exports;
      const shared = new SharedArrayBuffer(4); const view = new Int32Array(shared);
      (async () => {
        const body = `const { parentPort, workerData } = require('node:worker_threads'); const v = new Int32Array(workerData);
          for (let i = 0; i < 20000; i++) Atomics.add(v, 0, 1); parentPort.postMessage('done');`;
        const workers = [new wt.Worker(body, { eval: true, workerData: shared }), new wt.Worker(body, { eval: true, workerData: shared })];
        await Promise.all(workers.map((w) => new Promise((resolve) => { w.once('message', resolve); })));
        for (const w of workers) await w.terminate();
        console.log(view[0]);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "40000");
}

#[test]
fn main_thread_notify_wakes_a_worker_blocked_in_atomics_wait() {
    let src = "const wt = __thaw_worker_threads_exports;
      const shared = new SharedArrayBuffer(4); const view = new Int32Array(shared);
      (async () => {
        const worker = new wt.Worker(`const { parentPort, workerData } = require('node:worker_threads'); const v = new Int32Array(workerData);
          parentPort.postMessage('ready'); parentPort.postMessage(Atomics.wait(v, 0, 0));`, { eval: true, workerData: shared });
        const log = [];
        worker.on('message', (m) => { log.push(String(m)); });
        await new Promise((resolve) => { worker.once('message', resolve); });
        await new Promise((resolve) => setTimeout(resolve, 100));
        Atomics.store(view, 0, 1);
        log.push('woken ' + Atomics.notify(view, 0, 1));
        await new Promise((resolve) => { worker.once('message', resolve); });
        await worker.terminate();
        console.log(log.join('|'));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "ready|woken 1|ok");
}

#[test]
fn terminate_stops_a_worker_blocked_in_an_infinite_atomics_wait() {
    let src = "const wt = __thaw_worker_threads_exports;
      const shared = new SharedArrayBuffer(4);
      (async () => {
        const worker = new wt.Worker(`const { parentPort, workerData } = require('node:worker_threads'); parentPort.postMessage('waiting'); Atomics.wait(new Int32Array(workerData), 0, 0);`, { eval: true, workerData: shared });
        const seen = [];
        worker.on('error', () => seen.push('error'));
        await new Promise((resolve) => { worker.once('message', resolve); });
        const code = await worker.terminate();
        console.log(seen.concat(['exit ' + code]).join('|'));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "exit 1");
}

// postMessageToThread (W-D). The expected lines are the output of the same programs under Node 22.23.1.

#[test]
fn post_message_to_thread_round_trips_between_main_and_workers() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        process.on('workerMessage', (value, src) => console.log('main workerMessage', JSON.stringify(value), src));
        const echo = new wt.Worker(`
          const wt = require('node:worker_threads');
          let hello = null, src0 = null;
          process.on('workerMessage', (value, src) => { hello = value.hello; src0 = src; });
          wt.parentPort.on('message', (m) => {
            if (m === 'report') wt.parentPort.postMessage('worker got ' + JSON.stringify({ hello }) + ' from ' + src0);
            else if (m === 'send') wt.postMessageToThread(0, { back: hello }).then((r) => { wt.parentPort.postMessage('worker send ' + r); wt.parentPort.close(); }, (e) => { wt.parentPort.postMessage('worker send ' + e.code); wt.parentPort.close(); });
          });
          wt.parentPort.postMessage('ready');
        `, { eval: true });
        let message = undefined;
        await new Promise((resolve) => echo.once('message', (m) => { message = m; resolve(); }));
        console.log('from echo:', message);
        const sent = await wt.postMessageToThread(echo.threadId, { hello: 'worker' });
        console.log('A main send resolved', sent);
        echo.postMessage('report');
        await new Promise((resolve) => echo.once('message', (m) => { message = m; resolve(); }));
        console.log('from echo:', message);
        echo.postMessage('send');
        await new Promise((resolve) => echo.once('message', (m) => { message = m; resolve(); }));
        console.log('from echo:', message);
        await new Promise((resolve) => echo.once('exit', resolve));
        const sender = new wt.Worker(`
          const wt = require('node:worker_threads');
          wt.parentPort.on('message', (go) => {
            wt.postMessageToThread(0, { from: 'sender', go }).then((r) => { wt.parentPort.postMessage('sender resolved ' + r); wt.parentPort.close(); }, (e) => { wt.parentPort.postMessage('sender ' + e.code); wt.parentPort.close(); });
          });
          wt.parentPort.postMessage('ready');
        `, { eval: true });
        await new Promise((resolve) => sender.on('message', (m) => { if (m === 'ready') resolve(); }));
        sender.postMessage('go');
        await new Promise((resolve) => sender.on('message', (m) => { console.log('from sender:', m); }).on('exit', resolve));
        console.log('done');
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "from echo: ready|A main send resolved undefined|from echo: worker got {\"hello\":\"worker\"} from 0|main workerMessage {\"back\":\"worker\"} 1|from echo: worker send undefined|main workerMessage {\"from\":\"sender\",\"go\":\"go\"} 2|from sender: sender resolved undefined|done"
    );
}

#[test]
fn post_message_to_thread_rejects_with_the_node_messaging_codes() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const settle = (label, p) => p.then((v) => console.log(label, 'resolved', v), (e) => console.log(label, e.name, e.code, JSON.stringify(e.message)));
      (async () => {
        await settle('same thread', wt.postMessageToThread(0, 1));
        await settle('unknown id', wt.postMessageToThread(99, 1));
        const bare = new wt.Worker(`
          const wt = require('node:worker_threads');
          setTimeout(() => {}, 400);
          wt.parentPort.postMessage('bare ready');
        `, { eval: true });
        await new Promise((resolve) => bare.on('message', resolve));
        await settle('no listener', wt.postMessageToThread(bare.threadId, 'x'));
        const thrower = new wt.Worker(`
          const wt = require('node:worker_threads');
          process.on('workerMessage', () => { throw new Error('boom'); });
          setTimeout(() => {}, 400);
          wt.parentPort.postMessage('thrower ready');
        `, { eval: true });
        await new Promise((resolve) => thrower.on('message', resolve));
        await settle('listener throws', wt.postMessageToThread(thrower.threadId, 'x'));
        const busy = new wt.Worker(`
          const wt = require('node:worker_threads');
          process.on('workerMessage', () => {});
          wt.parentPort.postMessage('busy started');
          const end = Date.now() + 600; while (Date.now() < end) {}
        `, { eval: true });
        await new Promise((resolve) => busy.on('message', resolve));
        await settle('timeout', wt.postMessageToThread(busy.threadId, 'x', [], 50));
        console.log('done');
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "same thread Error ERR_WORKER_MESSAGING_SAME_THREAD \"Cannot sent a message to the same thread\"|unknown id Error ERR_WORKER_MESSAGING_FAILED \"Cannot find the destination thread or listener\"|no listener Error ERR_WORKER_MESSAGING_FAILED \"Cannot find the destination thread or listener\"|listener throws Error ERR_WORKER_MESSAGING_ERRORED \"The destination thread threw an error while processing the message\"|timeout Error ERR_WORKER_MESSAGING_TIMEOUT \"Sending a message to another thread timed out\"|done"
    );
}

#[test]
fn post_message_to_thread_errored_has_no_cause_like_node() {
    // Node 22.23.1: a listener that throws rejects the sender with ERR_WORKER_MESSAGING_ERRORED and no `cause`
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const thrower = new wt.Worker(`
          const wt = require('node:worker_threads');
          process.on('workerMessage', () => { throw new Error('boom'); });
          setTimeout(() => {}, 400);
          wt.parentPort.postMessage('thrower ready');
        `, { eval: true });
        await new Promise((resolve) => thrower.on('message', resolve));
        await wt.postMessageToThread(thrower.threadId, 'x').then(() => console.log('resolved'), (e) => console.log('errored', e.code, 'cause', e.cause === undefined, JSON.stringify(Object.keys(e))));
        console.log('done');
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "errored ERR_WORKER_MESSAGING_ERRORED cause true [\"code\"]|done"
    );
}

#[test]
fn post_message_to_thread_delivers_between_workers_by_thread_id() {
    let src = r##"const wt = __thaw_worker_threads_exports;
(async () => {
  const inbox = [];
  let wake = () => {};
  const watch = (w) => w.on('message', (m) => { inbox.push(m); wake(); });
  const take = async (prefix) => {
    for (;;) {
      const i = inbox.findIndex((m) => m.startsWith(prefix));
      if (i >= 0) { const [m] = inbox.splice(i, 1); console.log(m); return m; }
      await new Promise((resolve) => { wake = resolve; });
    }
  };
  const flag = new Int32Array(new SharedArrayBuffer(4));
  const b = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', (value, source) => {
      Atomics.add(wt.workerData, 0, 1);
      wt.parentPort.postMessage('B got ' + value + ' from ' + source);
    });
    wt.parentPort.on('message', (aId) => {
      wt.postMessageToThread(aId, 'pong').then(() => wt.parentPort.postMessage('B reply acked'), (e) => wt.parentPort.postMessage('B reply ' + e.code));
    });
    wt.parentPort.postMessage('B ready');`, { eval: true, workerData: flag });
  watch(b);
  const a = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', (value, source) => {
      wt.parentPort.postMessage('A got ' + value + ' from ' + source);
    });
    wt.parentPort.on('message', (bId) => {
      wt.postMessageToThread(bId, 'ping').then(() => wt.parentPort.postMessage('A ack ' + Atomics.load(wt.workerData, 0)), (e) => wt.parentPort.postMessage('A ' + e.code));
    });
    wt.parentPort.postMessage('A ready');`, { eval: true, workerData: flag });
  watch(a);
  await take('B ready');
  await take('A ready');
  a.postMessage(b.threadId);
  await take('B got ping from ' + a.threadId);
  await take('A ack 1');
  b.postMessage(a.threadId);
  await take('A got pong from ' + b.threadId);
  await take('B reply acked');
  await a.terminate();
  await b.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"##;
    assert_eq!(
        run_main(src),
        "B ready|A ready|B got ping from 2|A ack 1|A got pong from 1|B reply acked|done"
    );
}

#[test]
fn post_message_to_thread_fails_for_an_exited_or_unknown_thread() {
    let src = r##"const wt = __thaw_worker_threads_exports;
const settle = (label, p) => p.then((v) => console.log(label, 'resolved', v), (e) => console.log(label, e.name, e.code));
(async () => {
  const inbox = [];
  let wake = () => {};
  const watch = (w) => w.on('message', (m) => { inbox.push(m); wake(); });
  const take = async (prefix) => {
    for (;;) {
      const i = inbox.findIndex((m) => m.startsWith(prefix));
      if (i >= 0) { const [m] = inbox.splice(i, 1); console.log(m); return m; }
      await new Promise((resolve) => { wake = resolve; });
    }
  };
  const gone = new wt.Worker('1 + 1', { eval: true });
  const goneId = gone.threadId;
  await new Promise((resolve) => gone.once('exit', resolve));
  await settle('main gone', wt.postMessageToThread(goneId, 1));
  const a = new wt.Worker(`const wt = require('node:worker_threads');
    wt.parentPort.on('message', async (ids) => {
      for (const id of ids) {
        await wt.postMessageToThread(id, 'x').then(() => wt.parentPort.postMessage('resolved'), (e) => wt.parentPort.postMessage(e.code + ' ' + e.name));
      }
      wt.parentPort.postMessage('A finished');
    });
    wt.parentPort.postMessage('A ready');`, { eval: true });
  watch(a);
  const live = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', (value, source) => { wt.parentPort.postMessage('L got ' + value + ' from ' + source); });
    wt.parentPort.on('message', () => {});
    wt.parentPort.postMessage('L ready');`, { eval: true });
  watch(live);
  await take('L ready');
  await take('A ready');
  a.postMessage([goneId, 12345, live.threadId]);
  await take('ERR_WORKER_MESSAGING_FAILED Error');
  await take('ERR_WORKER_MESSAGING_FAILED Error');
  await take('L got x from ' + a.threadId);
  await take('resolved');
  await take('A finished');
  await live.terminate();
  await a.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"##;
    assert_eq!(
        run_main(src),
        "main gone Error ERR_WORKER_MESSAGING_FAILED|L ready|A ready|ERR_WORKER_MESSAGING_FAILED Error|ERR_WORKER_MESSAGING_FAILED Error|L got x from 2|resolved|A finished|done"
    );
}

#[test]
fn post_message_to_thread_moves_a_transfer_list_between_workers() {
    let src = r##"const wt = __thaw_worker_threads_exports;
(async () => {
  const inbox = [];
  let wake = () => {};
  const watch = (w) => w.on('message', (m) => { inbox.push(m); wake(); });
  const take = async (prefix) => {
    for (;;) {
      const i = inbox.findIndex((m) => m.startsWith(prefix));
      if (i >= 0) { const [m] = inbox.splice(i, 1); console.log(m); return m; }
      await new Promise((resolve) => { wake = resolve; });
    }
  };
  const b = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', (value, source) => {
      wt.parentPort.postMessage('B got ' + value.ab.byteLength + ' ' + new Uint8Array(value.ab)[0] + ' from ' + source);
    });
    wt.parentPort.on('message', () => {});
    wt.parentPort.postMessage('B ready');`, { eval: true });
  watch(b);
  await take('B ready');
  const a = new wt.Worker(`const wt = require('node:worker_threads');
    wt.parentPort.on('message', (bId) => {
      const ab = new ArrayBuffer(8);
      new Uint8Array(ab)[0] = 5;
      const sent = wt.postMessageToThread(bId, { ab }, [ab]);
      wt.parentPort.postMessage('A sync ' + ab.byteLength + ' ' + ab.detached);
      sent.then(() => wt.parentPort.postMessage('A acked ' + ab.byteLength + ' ' + ab.detached));
    });
    wt.parentPort.postMessage('A ready');`, { eval: true });
  watch(a);
  await take('A ready');
  a.postMessage(b.threadId);
  await take('A sync 0 true');
  await take('B got 8 5 from ' + a.threadId);
  await take('A acked 0 true');
  await a.terminate();
  await b.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"##;
    assert_eq!(
        run_main(src),
        "B ready|A ready|A sync 0 true|B got 8 5 from 2|A acked 0 true|done"
    );
}

#[test]
fn post_message_to_thread_worker_same_thread_and_no_listener() {
    let src = r##"const wt = __thaw_worker_threads_exports;
(async () => {
  const inbox = [];
  let wake = () => {};
  const watch = (w) => w.on('message', (m) => { inbox.push(m); wake(); });
  const take = async (prefix) => {
    for (;;) {
      const i = inbox.findIndex((m) => m.startsWith(prefix));
      if (i >= 0) { const [m] = inbox.splice(i, 1); console.log(m); return m; }
      await new Promise((resolve) => { wake = resolve; });
    }
  };
  const b = new wt.Worker(`const wt = require('node:worker_threads');
    wt.parentPort.postMessage('bare ready');
    setTimeout(() => {}, 400);`, { eval: true });
  watch(b);
  await take('bare ready');
  const a = new wt.Worker(`const wt = require('node:worker_threads');
    wt.parentPort.on('message', async (ids) => {
      await wt.postMessageToThread(wt.threadId, 1).then(() => wt.parentPort.postMessage('resolved'), (e) => wt.parentPort.postMessage('self ' + e.name + ' ' + e.code));
      await wt.postMessageToThread(undefined, 1).then(() => wt.parentPort.postMessage('resolved'), (e) => wt.parentPort.postMessage('bad id ' + e.name + ' ' + e.code));
      await wt.postMessageToThread(ids[0], 1).then(() => wt.parentPort.postMessage('resolved'), (e) => wt.parentPort.postMessage('no listener ' + e.name + ' ' + e.code));
      await wt.postMessageToThread(ids[1], 'x').then(() => wt.parentPort.postMessage('resolved'), (e) => wt.parentPort.postMessage('live ' + e.code));
      wt.parentPort.postMessage('A finished');
    });
    wt.parentPort.postMessage('A ready');`, { eval: true });
  watch(a);
  const c = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', (value, source) => { wt.parentPort.postMessage('C got ' + value + ' from ' + source); });
    wt.parentPort.on('message', () => {});
    wt.parentPort.postMessage('C ready');`, { eval: true });
  watch(c);
  await take('C ready');
  await take('A ready');
  a.postMessage([b.threadId, c.threadId]);
  await take('self Error ERR_WORKER_MESSAGING_SAME_THREAD');
  await take('bad id Error ERR_WORKER_MESSAGING_FAILED');
  await take('no listener Error ERR_WORKER_MESSAGING_FAILED');
  await take('C got x from ' + a.threadId);
  await take('resolved');
  await take('A finished');
  await a.terminate();
  await b.terminate();
  await c.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"##;
    assert_eq!(
        run_main(src),
        "bare ready|C ready|A ready|self Error ERR_WORKER_MESSAGING_SAME_THREAD|bad id Error ERR_WORKER_MESSAGING_FAILED|no listener Error ERR_WORKER_MESSAGING_FAILED|C got x from 2|resolved|A finished|done"
    );
}

#[test]
fn post_message_to_thread_worker_times_out_on_a_busy_thread() {
    let src = r##"const wt = __thaw_worker_threads_exports;
(async () => {
  const inbox = [];
  let wake = () => {};
  const watch = (w) => w.on('message', (m) => { inbox.push(m); wake(); });
  const take = async (prefix) => {
    for (;;) {
      const i = inbox.findIndex((m) => m.startsWith(prefix));
      if (i >= 0) { const [m] = inbox.splice(i, 1); console.log(m); return m; }
      await new Promise((resolve) => { wake = resolve; });
    }
  };
  const b = new wt.Worker(`const wt = require('node:worker_threads');
    process.on('workerMessage', () => { wt.parentPort.postMessage('B listener ran'); });
    wt.parentPort.on('message', () => {
      wt.parentPort.postMessage('B busy');
      const end = Date.now() + 400; while (Date.now() < end) {}
    });
    wt.parentPort.postMessage('B ready');`, { eval: true });
  watch(b);
  await take('B ready');
  const a = new wt.Worker(`const wt = require('node:worker_threads');
    wt.parentPort.on('message', (bId) => {
      const start = Date.now();
      wt.postMessageToThread(bId, 'x', [], 50).then(() => wt.parentPort.postMessage('A ok'), (e) => wt.parentPort.postMessage('A ' + e.code + ' ms>=45:' + (Date.now() - start >= 45)));
    });
    wt.parentPort.postMessage('A ready');`, { eval: true });
  watch(a);
  await take('A ready');
  b.postMessage('busy');
  await take('B busy');
  a.postMessage(b.threadId);
  await take('A ERR_WORKER_MESSAGING_TIMEOUT ms>=45:true');
  await take('B listener ran');
  await a.terminate();
  await b.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"##;
    assert_eq!(
        run_main(src),
        "B ready|A ready|B busy|A ERR_WORKER_MESSAGING_TIMEOUT ms>=45:true|B listener ran|done"
    );
}

#[test]
fn is_internal_thread_is_false_and_argument_errors_are_async() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const settle = (label, p) => p.then((v) => console.log(label, 'resolved', v), (e) => console.log(label, e.name, e.code, JSON.stringify(e.message)));
      (async () => {
        console.log('isInternalThread main', wt.isInternalThread);
        const worker = new wt.Worker(`
          const wt = require('node:worker_threads');
          wt.parentPort.postMessage('isInternalThread in worker ' + wt.isInternalThread + ' threadId ' + wt.threadId);
        `, { eval: true });
        worker.on('message', (m) => console.log(m));
        await new Promise((resolve) => worker.on('exit', resolve));
        await settle('timeout type', wt.postMessageToThread(99, 1, [], 'soon'));
        await settle('timeout range', wt.postMessageToThread(99, 1, [], -1));
        await settle('timeout zero unknown', wt.postMessageToThread(99, 1, [], 0));
        console.log('done');
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "isInternalThread main false|isInternalThread in worker false threadId 1|timeout type TypeError ERR_INVALID_ARG_TYPE \"The \\\"timeout\\\" argument must be of type number. Received type string ('soon')\"|timeout range RangeError ERR_OUT_OF_RANGE \"The value of \\\"timeout\\\" is out of range. It must be >= 0. Received -1\"|timeout zero unknown Error ERR_WORKER_MESSAGING_TIMEOUT \"Sending a message to another thread timed out\"|done"
    );
}

#[test]
fn share_env_symbol_is_a_symbol_named_as_in_node() {
    let src = "const wt = __thaw_worker_threads_exports;
      console.log(typeof wt.SHARE_ENV, String(wt.SHARE_ENV), wt.SHARE_ENV === wt.SHARE_ENV);";
    assert_eq!(
        run_main(src),
        "symbol Symbol(nodejs.worker_threads.SHARE_ENV) true"
    );
}

#[test]
fn share_env_worker_sees_parent_writes_and_its_writes_reach_the_parent() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      process.env.SV_A = 'parent1';
      process.env.SV_DEL = 'gone';
      (async () => {
        const worker = new wt.Worker(`
          const { parentPort } = require('node:worker_threads');
          parentPort.on('message', () => {
            parentPort.postMessage([process.env.SV_A, process.env.SV_AFTER, process.env.SV_DEL === undefined, 'SV_DEL' in process.env, Object.keys(process.env).includes('SV_A')].join(' '));
            process.env.SV_B = 'fromworker';
            process.env.SV_N = 7;
            process.env.SV_U = undefined;
            parentPort.postMessage('wrote');
          });
          parentPort.postMessage('ready');
          setInterval(() => {}, 1000);
        `, { eval: true, env: wt.SHARE_ENV });
        const inbox = [];
        let wake = null;
        worker.on('message', (m) => { inbox.push(m); if (wake) { const f = wake; wake = null; f(); } });
        const next = () => (inbox.length ? Promise.resolve(inbox.shift()) : new Promise((r) => { wake = () => r(inbox.shift()); }));
        console.log('main got', await next());
        process.env.SV_AFTER = 'afterstart';
        delete process.env.SV_DEL;
        worker.postMessage('read');
        console.log('main got', await next());
        console.log('main got', await next());
        await worker.terminate();
        console.log('main after', process.env.SV_B, typeof process.env.SV_N, process.env.SV_N, process.env.SV_U, Object.keys(process.env).includes('SV_B'));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "main got ready|main got parent1 afterstart true false true|main got wrote|main after fromworker string 7 undefined true"
    );
}

#[test]
fn default_and_object_env_are_copies_that_the_parent_never_sees() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      process.env.SV_C = 'parent1';
      const alive = 'setInterval(() => {}, 1000);';
      const copySource = `
        const { parentPort } = require('node:worker_threads');
        parentPort.postMessage('view ' + process.env.SV_C + ' ' + Object.keys(process.env).includes('SV_C'));
        process.env.SV_B = 'fromworker';
        parentPort.postMessage('wrote');
        ${alive}
      `;
      const objectSource = `
        const { parentPort } = require('node:worker_threads');
        parentPort.postMessage('view ' + process.env.SV_O + ' ' + process.env.SV_C + ' ' + Object.keys(process.env).join(','));
        process.env.SV_B = 'fromworker';
        parentPort.postMessage('wrote');
        ${alive}
      `;
      function inboxOf(w) {
        const items = [];
        let wake = null;
        w.on('message', (m) => { items.push(m); if (wake) { const f = wake; wake = null; f(); } });
        return () => (items.length ? Promise.resolve(items.shift()) : new Promise((r) => { wake = () => r(items.shift()); }));
      }
      (async () => {
        const w1 = new wt.Worker(copySource, { eval: true });
        const n1 = inboxOf(w1);
        console.log('default', await n1());
        console.log('default', await n1());
        await w1.terminate();
        console.log('default after', process.env.SV_B);
        const w2 = new wt.Worker(objectSource, { eval: true, env: { SV_O: '1' } });
        const n2 = inboxOf(w2);
        console.log('object', await n2());
        console.log('object', await n2());
        await w2.terminate();
        console.log('object after', process.env.SV_B, process.env.SV_O);
        const w3 = new wt.Worker(copySource, { eval: true, env: process.env });
        const n3 = inboxOf(w3);
        console.log('copy-env', await n3());
        console.log('copy-env', await n3());
        await w3.terminate();
        console.log('copy-env after', process.env.SV_B);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "default view parent1 true|default wrote|default after undefined|object view 1 undefined SV_O|object wrote|object after undefined undefined|copy-env view parent1 true|copy-env wrote|copy-env after undefined"
    );
}

#[test]
fn worker_post_message_moves_an_array_buffer_and_detaches_the_sender() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads');
          parentPort.on('message', (m) => { parentPort.postMessage({ len: m.buf.byteLength, first: new Uint8Array(m.buf)[0], det: m.buf.detached }); });
          parentPort.postMessage('ready');`, { eval: true });
        await new Promise((resolve) => worker.once('message', resolve));
        const buf = new ArrayBuffer(8); new Uint8Array(buf)[0] = 7;
        const reply = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ buf }, [buf]);
        console.log('sender', buf.byteLength, buf.detached);
        const got = await reply;
        console.log('receiver', got.len, got.first, got.det);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "sender 0 true|receiver 8 7 false");
}

#[test]
fn worker_transfer_options_form_and_parent_port_transfer_move_buffers() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads');
          parentPort.on('message', (m) => {
            if (m.cmd === 'opt') parentPort.postMessage({ back: m.buf, len: m.buf.byteLength }, { transfer: [m.buf] });
            else parentPort.postMessage({ back: m.buf, len: m.buf.byteLength }, [m.buf]);
          });
          parentPort.postMessage('ready');`, { eval: true });
        await new Promise((resolve) => worker.once('message', resolve));
        const opt = new ArrayBuffer(4);
        const back1 = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ cmd: 'opt', buf: opt }, { transfer: [opt] });
        console.log('options sender', opt.byteLength, opt.detached);
        const first = await back1;
        console.log('options back', first.len, first.back.byteLength, first.back.detached);
        const arr = new ArrayBuffer(6);
        const back2 = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ cmd: 'array', buf: arr }, [arr]);
        console.log('array sender', arr.byteLength, arr.detached);
        const second = await back2;
        console.log('array back', second.len, second.back.byteLength, second.back.detached);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "options sender 0 true|options back 4 4 false|array sender 0 true|array back 6 6 false"
    );
}

#[test]
fn transferred_views_keep_their_window_on_the_receiver_only() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads');
          parentPort.on('message', (m) => {
            parentPort.postMessage({ n: m.n === undefined ? -1 : m.n, length: m.view ? m.view.length : -1, offset: m.view ? m.view.byteOffset : -1, bytes: m.view ? m.view.buffer.byteLength : -1 });
          });
          parentPort.postMessage('ready');`, { eval: true });
        await new Promise((resolve) => worker.once('message', resolve));
        const big = new ArrayBuffer(16); const view = new Uint8Array(big, 4, 4); view[0] = 9;
        const reply1 = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ view }, [big]);
        console.log('view sender', view.length, view.byteOffset, big.byteLength, big.detached);
        const r1 = await reply1;
        console.log('view receiver', r1.length, r1.offset, r1.bytes);
        const big3 = new ArrayBuffer(8); const other = new Uint8Array(big3, 2, 3);
        const reply2 = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ n: 1 }, [big3]);
        console.log('unmentioned sender', other.length, other.byteOffset, big3.byteLength);
        const r2 = await reply2;
        console.log('unmentioned receiver', r2.n);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "view sender 0 0 0 true|view receiver 4 4 16|unmentioned sender 0 0 0|unmentioned receiver 1");
}

#[test]
fn resizable_buffer_transfer_stays_resizable_on_the_receiver() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads');
          parentPort.on('message', (m) => { const b = m.view.buffer; parentPort.postMessage({ len: b.byteLength, res: b.resizable, max: b.maxByteLength, vlen: m.view.length }); });
          parentPort.postMessage('ready');`, { eval: true });
        await new Promise((resolve) => worker.once('message', resolve));
        const rb = new ArrayBuffer(8, { maxByteLength: 16 });
        const reply = new Promise((resolve) => worker.once('message', resolve));
        worker.postMessage({ view: new Uint8Array(rb) }, [rb]);
        console.log('resizable sender', rb.byteLength, rb.detached, rb.resizable, rb.maxByteLength);
        const got = await reply;
        console.log('resizable receiver', got.len, got.res, got.max, got.vlen);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "resizable sender 0 true true 0|resizable receiver 8 true 16 8"
    );
}

#[test]
fn transfer_list_validator_messages_match_node_and_detach_nothing_on_failure() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const err = (label, f) => { try { f(); console.log(label, 'ok'); } catch (e) { console.log(label, e.constructor.name, e.name, e.code, JSON.stringify(e.message)); } };
      const a = new ArrayBuffer(8);
      const port = new wt.MessageChannel().port1;
      err('object', () => port.postMessage(1, [{}]));
      err('number', () => port.postMessage(1, [1]));
      err('null', () => port.postMessage(1, [null]));
      err('view', () => port.postMessage(1, [new Uint8Array(2)]));
      err('function', () => port.postMessage(1, [() => 1]));
      err('shared', () => port.postMessage(1, [new SharedArrayBuffer(2)]));
      err('duplicate', () => port.postMessage(1, [a, a]));
      const untransferable = new ArrayBuffer(3); wt.markAsUntransferable(untransferable);
      err('untransferable', () => port.postMessage(1, [untransferable]));
      const detached = new ArrayBuffer(2); detached.transfer();
      err('detached in list', () => port.postMessage(1, [a, detached]));
      err('detached in value', () => port.postMessage({ x: detached }));
      err('string list', () => port.postMessage(1, 'x'));
      err('number list', () => port.postMessage(1, 5));
      err('options transfer', () => port.postMessage(1, { transfer: 'x' }));
      err('structuredClone duplicate', () => structuredClone(1, { transfer: [a, a] }));
      err('structuredClone string', () => structuredClone(1, { transfer: 'x' }));
      console.log('after failures', a.byteLength, a.detached);
      const ch = new wt.MessageChannel();
      ch.port1.postMessage(a, [a]);
      const got = wt.receiveMessageOnPort(ch.port2);
      console.log('moved', a.byteLength, a.detached, got.message.byteLength, got.message.detached);"#;
    assert_eq!(run_main(src), "object DOMException DataCloneError 25 \"Found invalid value in transferList.\"|number DOMException DataCloneError 25 \"Found invalid value in transferList.\"|null DOMException DataCloneError 25 \"Found invalid value in transferList.\"|view DOMException DataCloneError 25 \"Found invalid value in transferList.\"|function DOMException DataCloneError 25 \"Found invalid value in transferList.\"|shared DOMException DataCloneError 25 \"Found invalid value in transferList.\"|duplicate DOMException DataCloneError 25 \"Transfer list contains duplicate ArrayBuffer\"|untransferable DOMException DataCloneError 25 \"Cannot transfer object of unsupported type.\"|detached in list DOMException DataCloneError 25 \"Cannot transfer object of unsupported type.\"|detached in value DOMException DataCloneError 25 \"An ArrayBuffer is detached and could not be cloned.\"|string list TypeError TypeError ERR_INVALID_ARG_TYPE \"Optional transferList argument must be an iterable\"|number list TypeError TypeError ERR_INVALID_ARG_TYPE \"Optional transferList argument must be an iterable\"|options transfer TypeError TypeError ERR_INVALID_ARG_TYPE \"Optional options.transfer argument must be an iterable\"|structuredClone duplicate DOMException DataCloneError 25 \"Transfer list contains duplicate ArrayBuffer\"|structuredClone string TypeError TypeError ERR_INVALID_ARG_TYPE \"Failed to execute 'structuredClone': transfer in Options can not be converted to sequence.\"|after failures 8 false|moved 0 true 8 false");
}

#[test]
fn worker_constructor_transfer_list_detaches_even_outside_workerdata() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const code = `const { workerData, parentPort } = require('node:worker_threads'); parentPort.postMessage(JSON.stringify(workerData.buf ? [workerData.buf.byteLength, new Uint8Array(workerData.buf)[0]] : [workerData.n, workerData.clone]));`;
        const err = (label, f) => { try { f(); console.log(label, 'ok'); } catch (e) { console.log(label, e.constructor.name, e.name, e.code, JSON.stringify(e.message)); } };
        const outside = new ArrayBuffer(3);
        const w1 = new wt.Worker(code, { eval: true, workerData: { n: 1 }, transferList: [outside] });
        console.log('outside sender', outside.byteLength, outside.detached);
        await new Promise((resolve) => w1.on('message', resolve));
        await w1.terminate();
        const dup = new ArrayBuffer(3);
        err('duplicate', () => new wt.Worker('0', { eval: true, workerData: dup, transferList: [dup, dup] }));
        console.log('duplicate sender', dup.byteLength, dup.detached);
        const shared = new SharedArrayBuffer(3);
        err('shared', () => new wt.Worker('0', { eval: true, workerData: shared, transferList: [shared] }));
        err('string list', () => new wt.Worker('0', { eval: true, transferList: 'x' }));
        const clone = new ArrayBuffer(3);
        const w2 = new wt.Worker(code, { eval: true, workerData: { clone: clone.byteLength }, transferList: [] });
        console.log('no list sender', clone.byteLength, clone.detached);
        await new Promise((resolve) => w2.on('message', resolve));
        await w2.terminate();
        const data = new ArrayBuffer(8); new Uint8Array(data)[0] = 5;
        const w3 = new wt.Worker(code, { eval: true, workerData: { buf: data }, transferList: [data] });
        console.log('data sender', data.byteLength, data.detached);
        const text = await new Promise((resolve) => w3.on('message', resolve));
        console.log('data receiver', text);
        await w3.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "outside sender 0 true|duplicate DOMException DataCloneError 25 \"Transfer list contains duplicate ArrayBuffer\"|duplicate sender 3 false|shared DOMException DataCloneError 25 \"Found invalid value in transferList.\"|string list DOMException DataCloneError 25 \"Found invalid value in transferList.\"|no list sender 3 false|data sender 0 true|data receiver [8,5]");
}

#[test]
fn post_message_to_thread_moves_a_transferred_buffer_both_ways() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const settle = (label, p) => p.then((v) => console.log(label, 'resolved', v), (e) => console.log(label, e.name, e.code, JSON.stringify(e.message)));
      process.on('workerMessage', (v, source) => console.log('main got', v.c.byteLength, v.c.detached, source));
      (async () => {
        const worker = new wt.Worker(`const wt = require('node:worker_threads');
          process.on('workerMessage', (v) => { wt.parentPort.postMessage({ got: v.buf.byteLength, first: new Uint8Array(v.buf)[0] }); });
          const c = new ArrayBuffer(5);
          wt.postMessageToThread(0, { c }, [c]).then(() => wt.parentPort.postMessage({ sent: c.byteLength, det: c.detached }));
          wt.parentPort.postMessage('ready');
          setTimeout(() => {}, 400);`, { eval: true });
        const queue = []; let wake = null;
        worker.on('message', (m) => { queue.push(m); if (wake) { const resume = wake; wake = null; resume(); } });
        const next = async () => { while (queue.length === 0) await new Promise((resolve) => { wake = resolve; }); return queue.shift(); };
        await next();
        console.log('worker sent', JSON.stringify(await next()));
        const buf = new ArrayBuffer(6); new Uint8Array(buf)[0] = 3;
        const pending = wt.postMessageToThread(worker.threadId, { buf }, [buf]);
        console.log('ptt sender', buf.byteLength, buf.detached);
        await settle('ptt', pending);
        console.log('ptt worker', JSON.stringify(await next()));
        const kept = new ArrayBuffer(4);
        await settle('unknown', wt.postMessageToThread(99, 1, [kept]));
        console.log('unknown kept', kept.byteLength, kept.detached);
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "main got 5 false 1|worker sent {\"sent\":0,\"det\":true}|ptt sender 0 true|ptt resolved undefined|ptt worker {\"got\":6,\"first\":3}|unknown Error ERR_WORKER_MESSAGING_FAILED \"Cannot find the destination thread or listener\"|unknown kept 4 false");
}

#[test]
fn post_message_after_exit_does_not_detach_its_transfer_list() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('1 + 1', { eval: true });
        await new Promise((resolve) => { worker.on('exit', resolve); });
        const b = new ArrayBuffer(4);
        worker.postMessage(1, [b]);
        console.log('after exit', b.byteLength, b.detached);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "after exit 4 false");
}

#[test]
fn worker_post_message_to_unknown_thread_detaches_its_transfer_list() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const wt = require('node:worker_threads');
          const keep = setInterval(() => {}, 1000);
          const kept = new ArrayBuffer(4);
          const p = wt.postMessageToThread(99, 1, [kept]);
          const sync = [kept.byteLength, kept.detached];
          p.then(() => {}, (e) => wt.parentPort.postMessage({ sync, name: e.name, code: e.code, after: [kept.byteLength, kept.detached] }));`, { eval: true });
        const got = await new Promise((resolve) => worker.once('message', resolve));
        console.log(JSON.stringify(got));
        await worker.terminate();
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "{\"sync\":[0,true],\"name\":\"Error\",\"code\":\"ERR_WORKER_MESSAGING_FAILED\",\"after\":[0,true]}");
}

/// The factory a bundle's `require('node:worker_threads')` evaluates to (`scan::WORKER_THREADS_THAW_JS_FORM`) must hand a
/// worker realm its worker-side exports (`isMainThread` false, the parent's `workerData`), and the main realm the main-thread
/// form. Lowest level available: a bundled module cannot start a Worker from `/thaw_modules/<key>` yet, so the real
/// substituted expression is evaluated in the worker code text.
#[test]
fn substituted_worker_threads_factory_is_worker_side_in_a_worker() {
    let form = thaw_js::scan::WORKER_THREADS_THAW_JS_FORM;
    let load = format!(
        "const m = {{ exports: {{}} }}; (function (module) {{ {form} }})(m); const wt = m.exports;"
    );
    let worker_code = format!("{load} wt.parentPort.postMessage(JSON.stringify([wt.isMainThread, (wt.workerData || {{}}).n]));");
    let src = format!(
        "{load} const w = new wt.Worker(`{worker_code}`, {{ eval: true, workerData: {{ n: 7 }} }}); \
         w.once('message', (v) => {{ console.log('worker ' + v + ' main ' + wt.isMainThread + ' ' + wt.workerData); w.terminate(); }});"
    );
    assert_eq!(run_main(&src), "worker [false,7] main true null");
}

#[test]
fn worker_string_data_and_file_filenames_are_rejected_like_node() {
    let src = r#"const wt = __thaw_worker_threads_exports;
const tryCtor = (p) => { try { new wt.Worker(p); return 'constructed'; } catch (e) { return JSON.stringify([e.constructor.name, e.code, e.message]); } };
console.log(tryCtor('data:text/javascript,' + encodeURIComponent('process.exitCode = 3;')));
console.log(tryCtor('data:text/javascript;base64,cHJvY2Vzcy5leGl0Q29kZSA9IDM7'));
console.log(tryCtor('data:application/javascript,' + encodeURIComponent('process.exitCode = 3;')));
console.log(tryCtor('data:text/plain,1'));"#;
    assert_eq!(
        run_main(src),
        r#"["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Wrap data: URLs with `new URL`. Received \"data:text/javascript,process.exitCode%20%3D%203%3B\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Wrap data: URLs with `new URL`. Received \"data:text/javascript;base64,cHJvY2Vzcy5leGl0Q29kZSA9IDM7\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Received \"data:application/javascript,process.exitCode%20%3D%203%3B\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Received \"data:text/plain,1\""]"#
    );
}

#[test]
fn worker_path_filenames_resolve_and_report_like_node() {
    let src = r#"(async () => {
const wt = __thaw_worker_threads_exports;
const tryCtor = (p) => { try { new wt.Worker(p); return 'constructed'; } catch (e) { return JSON.stringify([e.constructor.name, e.code, e.message.split(process.cwd()).join('<cwd>')]); } };
for (const p of ['file.js', 'file:///x/y.js', 'data:text/plain,1', 'data:text/javascript,1']) console.log(tryCtor(p));
const run = (label, file) => new Promise((resolve) => {
  const w = new wt.Worker(file);
  w.on('error', (e) => { const p = e.message.split("'")[1]; console.log(label, 'error', e.code, e.constructor.name, 'absolute=' + p.startsWith('/'), 'normalized=' + !/(^|\/)\.{1,2}(\/|$)/.test(p), 'base=' + p.split('/').pop()); });
  w.on('exit', (c) => { console.log(label, 'exit', c); resolve(); });
});
await run('rel', './nonexistent-rel.js');
await run('parent', '../nonexistent-rel.js');
await run('dotdot', '/nonexistent-dir/x/../y/file.js');
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        r#"["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Received \"file.js\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Wrap file:// URLs with `new URL`. Received \"file:///x/y.js\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Received \"data:text/plain,1\""]|["TypeError","ERR_WORKER_PATH","The worker script or module filename must be an absolute path or a relative path starting with './' or '../'. Wrap data: URLs with `new URL`. Received \"data:text/javascript,1\""]|rel error MODULE_NOT_FOUND Error absolute=true normalized=true base=nonexistent-rel.js|rel exit 1|parent error MODULE_NOT_FOUND Error absolute=true normalized=true base=nonexistent-rel.js|parent exit 1|dotdot error MODULE_NOT_FOUND Error absolute=true normalized=true base=file.js|dotdot exit 1"#
    );
}

#[test]
fn receive_message_on_port_accepts_broadcast_channel_and_worker_transfers_like_node() {
    let src = r#"(async () => {
const wt = __thaw_worker_threads_exports;
const bc = new wt.BroadcastChannel('wk6-sync'), bc2 = new wt.BroadcastChannel('wk6-sync');
bc.postMessage({ value: 5 });
console.log('broadcast', JSON.stringify(wt.receiveMessageOnPort(bc2)), JSON.stringify(wt.receiveMessageOnPort(bc2)));
bc.close(); bc2.close();
try { wt.receiveMessageOnPort({}); } catch (e) { console.log('invalid', e.constructor.name, e.code, e.message); }
const ch = new wt.MessageChannel(); const inner = new wt.MessageChannel();
ch.port1.postMessage({ port: inner.port1 }, [inner.port1]);
const moved = wt.receiveMessageOnPort(ch.port2);
moved.message.port.postMessage('through');
console.log('moved', JSON.stringify(wt.receiveMessageOnPort(inner.port2)), JSON.stringify(wt.receiveMessageOnPort(inner.port2)));
const code = `const { parentPort, receiveMessageOnPort, BroadcastChannel } = require('node:worker_threads'); const a = new BroadcastChannel('wk6-worker'), b = new BroadcastChannel('wk6-worker'); a.postMessage('x'); const got = receiveMessageOnPort(b); parentPort.postMessage('bc ' + JSON.stringify(got)); a.close(); b.close(); parentPort.once('message', (m) => { const r = receiveMessageOnPort(m.port); parentPort.postMessage('port ' + (r === undefined ? 'undefined' : r.message)); parentPort.postMessage('done'); });`;
const w = new wt.Worker(code, { eval: true });
const out = [];
const done = new Promise((resolve) => w.on('message', (v) => { out.push(v); if (v === 'done') resolve(); }));
const inner2 = new wt.MessageChannel();
inner2.port2.postMessage('queued');
w.postMessage({ port: inner2.port1 }, [inner2.port1]);
await done;
console.log('worker ' + out.join('|'));
await w.terminate();
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        r#"broadcast {"message":{"value":5}} undefined|invalid TypeError ERR_INVALID_ARG_TYPE The "port" argument must be a MessagePort instance|moved {"message":"through"} undefined|worker bc {"message":"x"}|port queued|done"#
    );
}

// worker.cpuUsage() (Node 22.23.1 expectations recorded from `node` with the same program bodies)

#[test]
fn worker_cpu_usage_is_integer_user_and_system_and_rejects_when_not_running() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const w = new wt.Worker(`require('node:worker_threads').parentPort.on('message',()=>{});`, { eval: true });
        const pre = await w.cpuUsage().then(() => 'resolved', (e) => e.code);
        await new Promise((resolve) => w.once('online', resolve));
        const p = w.cpuUsage();
        const r = await p;
        const r2 = await w.cpuUsage(r);
        console.log(pre, p instanceof Promise, Object.keys(r).join(), Number.isInteger(r.user), Number.isInteger(r.system), r.user >= 0, r.system >= 0, Number.isInteger(r2.user));
        await w.terminate();
        console.log(await w.cpuUsage().then(() => 'resolved', (e) => e.code + ' ' + e.message));
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(
        run_main(src),
        "ERR_WORKER_NOT_RUNNING true user,system true true true true true|ERR_WORKER_NOT_RUNNING Worker instance not running"
    );
}

#[test]
fn worker_cpu_usage_resolves_while_the_worker_is_busy() {
    let src = "const wt = __thaw_worker_threads_exports;
      const log = [];
      const w = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.postMessage('go'); const t = Date.now(); while (Date.now() - t < 800) {} parentPort.postMessage('done');`, { eval: true });
      w.on('message', (m) => {
        if (m !== 'go') { log.push('msg ' + m); return; }
        w.cpuUsage().then((r) => log.push('cpu ' + Number.isInteger(r.user) + ' ' + (r.system >= 0)), (e) => log.push('cpu ' + e.code));
      });
      w.on('exit', () => console.log(log.join(',')));";
    assert_eq!(run_main(src), "cpu true true,msg done");
}

#[test]
fn worker_cpu_usage_is_served_inside_an_infinite_loop_and_terminate_returns_the_exit_code() {
    let src = "const wt = __thaw_worker_threads_exports;
      const w = new wt.Worker('while(true){}', { eval: true });
      (async () => {
        await new Promise((resolve) => w.once('online', resolve));
        const r = await w.cpuUsage();
        console.log('spin', Number.isInteger(r.user), Number.isInteger(r.system), r.user >= 0);
        console.log('code', await w.terminate());
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(run_main(src), "spin true true true|code 1");
}

#[test]
fn worker_cpu_usage_subtracts_prev_client_side() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const w = new wt.Worker(`require('node:worker_threads').parentPort.on('message',()=>{});`, { eval: true });
        await new Promise((resolve) => w.once('online', resolve));
        // the worker's CPU time is far below 1e15 us, so subtracting it gives a negative number near -1e15
        const d = await w.cpuUsage({ user: 1e15, system: 1e15 });
        const z = await w.cpuUsage({ user: 0, system: 0 });
        console.log(Object.keys(d).join(), d.user < 0, d.user + 1e15 >= 0, d.system + 1e15 >= 0, Number.isInteger(d.user), z.user >= 0, z.system >= 0);
        await w.terminate();
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(run_main(src), "user,system true true true true true true");
}

#[test]
fn worker_cpu_usage_validates_prev_synchronously_like_node() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const w = new wt.Worker(`require('node:worker_threads').parentPort.on('message',()=>{});`, { eval: true });
        await new Promise((resolve) => w.once('online', resolve));
        for (const prev of ['x', { user: 'a', system: 1 }, { user: 1 }, { user: -1, system: 0 }, { user: 1, system: NaN }, [], 5]) {
          try { w.cpuUsage(prev); console.log('returned'); }
          catch (e) { console.log(e.name, e.code, e.message); }
        }
        const r = await w.cpuUsage(null);
        console.log('null ok', Number.isInteger(r.user));
        await w.terminate();
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(
        run_main(src),
        "TypeError ERR_INVALID_ARG_TYPE The \"prev\" argument must be of type object. Received type string ('x')\
|TypeError ERR_INVALID_ARG_TYPE The \"prev.user\" property must be of type number. Received type string ('a')\
|TypeError ERR_INVALID_ARG_TYPE The \"prev.system\" property must be of type number. Received undefined\
|RangeError ERR_OUT_OF_RANGE The value of \"prev.user\" is out of range. It must be >= 0 && <= 9007199254740991. Received -1\
|RangeError ERR_OUT_OF_RANGE The value of \"prev.system\" is out of range. It must be >= 0 && <= 9007199254740991. Received NaN\
|TypeError ERR_INVALID_ARG_TYPE The \"prev\" argument must be of type object. Received an instance of Array\
|TypeError ERR_INVALID_ARG_TYPE The \"prev\" argument must be of type object. Received type number (5)\
|null ok true"
    );
}

#[test]
fn worker_cpu_usage_then_terminate_on_an_idle_worker_still_resolves() {
    let src = "const wt = __thaw_worker_threads_exports;
      const w = new wt.Worker(`require('node:worker_threads').parentPort.on('message',()=>{});`, { eval: true });
      (async () => {
        await new Promise((resolve) => w.once('online', resolve));
        const p = w.cpuUsage();
        const t = w.terminate();
        const r = await p;
        console.log('cpu', Number.isInteger(r.user), Number.isInteger(r.system));
        console.log('exit', await t);
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(run_main(src), "cpu true true|exit 1");
}

#[test]
fn worker_cpu_usage_rejects_before_online_and_after_exit() {
    let src = "const wt = __thaw_worker_threads_exports;
      const w = new wt.Worker(`require('node:worker_threads').parentPort.on('message',()=>{});`, { eval: true });
      (async () => {
        console.log('pre', await w.cpuUsage().then(() => 'resolved', (e) => e.code + ' ' + e.message));
        await new Promise((resolve) => w.once('online', resolve));
        await w.terminate();
        console.log('post', await w.cpuUsage().then(() => 'resolved', (e) => e.code + ' ' + e.message));
      })().catch((e) => { console.log('failed ' + e); w.terminate(); });";
    assert_eq!(
        run_main(src),
        "pre ERR_WORKER_NOT_RUNNING Worker instance not running|post ERR_WORKER_NOT_RUNNING Worker instance not running"
    );
}

#[test]
fn worker_prepend_once_listener_runs_first_and_only_once() {
    // Node 22 (verified): `first,regular,regular 1 function true 10 true`
    let src = "const wt = __thaw_worker_threads_exports;
      const worker = new wt.Worker('void 0;', { eval: true });
      const order = [];
      worker.on('probe', () => order.push('regular'));
      worker.prependOnceListener('probe', () => order.push('first'));
      worker.emit('probe'); worker.emit('probe');
      console.log(order.join(',') + ' ' + worker.listenerCount('probe') + ' ' + typeof worker.prependOnceListener + ' ' + worker.eventNames().includes('probe') + ' ' + worker.getMaxListeners() + ' ' + (worker.setMaxListeners(20) === worker));
      worker.terminate();";
    assert_eq!(
        run_main(src),
        "first,regular,regular 1 function true 10 true"
    );
}

#[test]
fn worker_name_and_post_exit_accessors_follow_node() {
    // Node 22 (verified): `name alpha` / `null {} true`
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort, threadName } = require('node:worker_threads'); parentPort.postMessage(threadName);`, { eval: true, name: 'alpha' });
        const message = await new Promise((resolve) => worker.on('message', resolve));
        console.log('name ' + message);
        await new Promise((resolve) => worker.on('exit', resolve));
        console.log(worker.threadName + ' ' + JSON.stringify(worker.resourceLimits) + ' ' + ((await worker.terminate()) === undefined));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "name alpha|null {} true");
}

#[test]
fn worker_post_message_to_thread_rejects_a_transfer_port_closed_by_a_getter() {
    // Node 22 (verified): `rejected 0 0` (a main-thread workerMessage listener is installed, as in the registry test)
    let src = "const wt = __thaw_worker_threads_exports;
      let delivered = 0; process.on('workerMessage', () => { delivered++; });
      (async () => {
        const worker = new wt.Worker(`const wt = require('node:worker_threads');
          const first = new wt.MessageChannel(), second = new wt.MessageChannel();
          wt.postMessageToThread(0, { first: first.port1, get invalidate() { second.port1.close(); return true; } }, [first.port1, second.port1])
            .then(() => wt.parentPort.postMessage('unexpected'), () => wt.parentPort.postMessage('rejected'));`, { eval: true });
        const message = await new Promise((resolve) => worker.once('message', resolve));
        const code = await new Promise((resolve) => worker.once('exit', resolve));
        console.log(message + ' ' + code + ' ' + delivered);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(run_main(src), "rejected 0 0");
}
