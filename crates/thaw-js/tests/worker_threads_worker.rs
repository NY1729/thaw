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
          const keep = setTimeout(() => {}, 400);
          process.on('workerMessage', (value, src) => {
            wt.parentPort.postMessage('worker got ' + JSON.stringify(value) + ' from ' + src);
            setTimeout(() => wt.postMessageToThread(0, { back: value.hello }).then((r) => { wt.parentPort.postMessage('worker send ' + r); }, (e) => wt.parentPort.postMessage('worker send ' + e.code)), 50);
          });
          wt.parentPort.postMessage('ready');
        `, { eval: true });
        echo.on('message', (m) => console.log('from echo:', m));
        await new Promise((resolve) => echo.on('message', (m) => { if (m === 'ready') resolve(); }));
        const sent = await wt.postMessageToThread(echo.threadId, { hello: 'worker' });
        console.log('A main send resolved', sent);
        await new Promise((resolve) => echo.on('exit', resolve));
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
        "from echo: ready|from echo: worker got {\"hello\":\"worker\"} from 0|A main send resolved undefined|main workerMessage {\"back\":\"worker\"} 1|from echo: worker send undefined|main workerMessage {\"from\":\"sender\",\"go\":\"go\"} 2|from sender: sender resolved undefined|done"
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
fn post_message_to_thread_refuses_transfer_lists_as_before() {
    // not Node parity (Node transfers the buffer): the thaw-specific not-supported error, as Worker.postMessage gives
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('setTimeout(() => {}, 100)', { eval: true });
        try { await wt.postMessageToThread(worker.threadId, { a: 1 }, [new ArrayBuffer(1)]); console.log('resolved'); }
        catch (e) { console.log(e.name, e.message); }
        await new Promise((resolve) => worker.on('exit', resolve));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(
        run_main(src),
        "DataCloneError transfer lists are not supported on thaw-js yet"
    );
}
