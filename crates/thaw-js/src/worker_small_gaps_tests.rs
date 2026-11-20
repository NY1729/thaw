//! Node 22.23.1 parity for four small worker gaps: `Worker[Symbol.asyncDispose]`, `process.chdir` in a worker (and the
//! worker's own 'exit' handler on `process.exit`), `process.stdin` listener prepends in a worker, and `threadId` after exit.
//! The expected lines were recorded from `node` 22.23.1 with the same program bodies.

use super::*;

/// Runs `src` on a fresh Interp and returns its console lines (as worker_limits_tests.rs does).
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
fn worker_async_dispose_terminates_and_resolves_undefined() {
    // a failing run terminates the worker, so the loop never waits on the keep-alive interval
    let src = "const wt = __thaw_worker_threads_exports;
      const worker = new wt.Worker('setInterval(() => {}, 1000)', { eval: true });
      (async () => {
        console.log(typeof worker[Symbol.asyncDispose], worker[Symbol.asyncDispose].length);
        const p = worker[Symbol.asyncDispose]();
        console.log(p instanceof Promise);
        console.log('value ' + (await p));
      })().catch((e) => { console.log('failed ' + e); worker.terminate(); });";
    assert_eq!(lines(src), ["function 0", "true", "value undefined"]);
}

#[test]
fn worker_chdir_throws_and_exit_handler_runs_on_process_exit() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`
          const { parentPort } = require('node:worker_threads');
          try { process.chdir('/tmp'); } catch (e) { parentPort.postMessage(['chdir', e.name, e.code, e.message]); }
          process.on('exit', (c) => parentPort.postMessage(['exit-handler', c]));
          process.exit(7);`, { eval: true });
        worker.on('message', (m) => console.log('msg ' + JSON.stringify(m)));
        const code = await new Promise((resolve) => worker.on('exit', resolve));
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(
        lines(src),
        [
            r#"msg ["chdir","TypeError","ERR_WORKER_UNSUPPORTED_OPERATION","process.chdir() is not supported in workers"]"#,
            r#"msg ["exit-handler",7]"#,
            "exit 7",
        ]
    );
}

#[test]
fn worker_stdin_prepend_once_listener_runs_before_existing_listeners() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`
          const { parentPort } = require('node:worker_threads');
          const s = process.stdin;
          s.prependOnceListener('x', () => parentPort.postMessage('a'));
          s.on('x', () => parentPort.postMessage('b'));
          s.emit('x'); s.emit('x');
          parentPort.postMessage([typeof s.prependOnceListener, typeof s.prependListener, typeof s.once]);`, { eval: true });
        worker.on('message', (m) => console.log('msg ' + JSON.stringify(m)));
        await new Promise((resolve) => worker.on('exit', resolve));
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(
        lines(src),
        [
            r#"msg "a""#,
            r#"msg "b""#,
            r#"msg "b""#,
            r#"msg ["function","function","function"]"#,
        ]
    );
}

#[test]
fn worker_thread_id_is_minus_one_after_exit() {
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('1', { eval: true });
        console.log('before ' + (worker.threadId > 0));
        await new Promise((resolve) => worker.on('exit', resolve));
        console.log('after ' + worker.threadId);
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(lines(src), ["before true", "after -1"]);
}

#[test]
fn worker_and_port_ref_unref_return_undefined() {
    // Node 22: Worker and MessagePort ref()/unref() resolve undefined (no chaining), even after the worker exits
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker('setInterval(() => {}, 1000)', { eval: true });
        console.log('worker ref', worker.ref() === undefined, worker.unref() === undefined);
        worker.ref();
        const exited = new Promise((resolve) => worker.on('exit', resolve));
        worker.terminate();
        await exited;
        console.log('exited ref', worker.ref() === undefined, worker.unref() === undefined);
        const { port1, port2 } = new wt.MessageChannel();
        console.log('port ref', port1.ref() === undefined, port1.unref() === undefined, port1.hasRef());
        console.log('port2 unref', port2.unref() === undefined);
        port1.close(); port2.close();
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(
        lines(src),
        [
            "worker ref true true",
            "exited ref true true",
            "port ref true true false",
            "port2 unref true",
        ]
    );
}
