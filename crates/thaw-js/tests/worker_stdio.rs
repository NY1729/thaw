//! Worker stdio on thaw-js (W5): `worker.stdin` / `worker.stdout` / `worker.stderr` and the worker's `process.stdin`,
//! through the worker_stream.js shim. Each test runs a main script on `Interp` with the real
//! `__thaw_worker_threads_exports` and the real worker threads, as worker_threads_worker.rs does.
use thaw_js::Interp;

/// Runs `src` on the main thread and its event loop; returns the main thread's console output joined by `|`.
fn run_main(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

#[test]
fn worker_stdin_end_lets_the_worker_exit() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); let text = ''; process.stdin.setEncoding('utf8'); process.stdin.on('data', (c) => { text += c; }); process.stdin.on('end', () => { parentPort.postMessage('got ' + text); });`, { eval: true, stdin: true });
        const log = [];
        worker.on('message', (v) => { log.push(String(v)); });
        worker.stdin.write('abc');
        worker.stdin.end('def');
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        log.push('exit ' + code);
        console.log(log.join('|'));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "got abcdef|exit 0");
}

#[test]
fn worker_stdin_pause_holds_back_data() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); const log = []; process.stdin.setEncoding('utf8'); process.stdin.on('data', (c) => { log.push('data ' + c); if (c === 'a') { process.stdin.pause(); setTimeout(() => { log.push('resume'); process.stdin.resume(); }, 40); } }); process.stdin.on('end', () => { parentPort.postMessage(log.join(',')); });`, { eval: true, stdin: true });
        worker.on('message', (v) => { console.log(v); });
        worker.stdin.write('a');
        setTimeout(() => { worker.stdin.write('b'); worker.stdin.end(); }, 10);
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "data a,resume,data b");
}

#[test]
fn worker_stdout_capture_delivers_console_output_before_exit() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`console.log('hi'); process.stdout.write('x\\n');`, { eval: true, stdout: true });
        let text = '';
        const log = [];
        worker.stdout.setEncoding('utf8');
        worker.stdout.on('data', (c) => { text += c; });
        worker.stdout.on('end', () => { log.push('end'); });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        log.push('exit ' + code);
        console.log(JSON.stringify(text) + '|' + log.join(','));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), r#""hi\nx\n"|end,exit 0"#);
}

#[test]
fn worker_stderr_capture_delivers_console_error() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`console.error('oops');`, { eval: true, stderr: true });
        let text = '';
        const log = [];
        worker.stderr.setEncoding('utf8');
        worker.stderr.on('data', (c) => { text += c; });
        worker.stderr.on('end', () => { log.push('end'); });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        log.push('exit ' + code);
        console.log(JSON.stringify(text) + '|' + log.join(','));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), r#""oops\n"|end,exit 0"#);
}

#[test]
fn worker_output_is_not_captured_without_the_option() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`console.log('hi'); console.error('oops');`, { eval: true });
        let text = '';
        const log = [];
        worker.stdout.setEncoding('utf8');
        worker.stdout.on('data', (c) => { text += c; });
        worker.stdout.on('end', () => { log.push('end'); });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        log.push('exit ' + code);
        console.log(JSON.stringify(text) + '|' + log.join(','));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), r#"""|end,exit 0"#);
}

#[test]
fn worker_stdout_false_still_exits_normally() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`console.log('hi');`, { eval: true, stdout: false });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "exit 0");
}

#[test]
fn worker_stdin_chunks_are_bytes_or_utf8_text() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const run = (workerSource, write) => new Promise((resolve) => {
        const worker = new wt.Worker(workerSource, { eval: true, stdin: true });
        let got = '';
        worker.on('message', (v) => { got = String(v); });
        worker.on('exit', () => resolve(got));
        write(worker.stdin);
      });
      (async () => {
        const bytes = await run(`const { parentPort } = require('node:worker_threads'); const parts = []; process.stdin.on('data', (c) => parts.push(Array.from(c).join(','))); process.stdin.on('end', () => parentPort.postMessage(parts.join(';')));`, (stdin) => { stdin.write(new Uint8Array([1, 2, 255])); stdin.end(); });
        const text = await run(`const { parentPort } = require('node:worker_threads'); let text = ''; process.stdin.setEncoding('utf8'); process.stdin.on('data', (c) => { text += c; }); process.stdin.on('end', () => parentPort.postMessage(text));`, (stdin) => { stdin.write(new Uint8Array([0xe2, 0x82])); stdin.end(new Uint8Array([0xac])); });
        console.log(bytes + '|' + text);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "1,2,255|\u{20ac}");
}

#[test]
fn worker_stdin_is_null_without_the_option_and_writes_after_end_fail() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const guard = setTimeout(() => process.exit(9), 5000);
        const log = [];
        const plain = new wt.Worker(`process.stdin.on('data', () => {}); process.stdin.on('end', () => {});`, { eval: true });
        log.push('null ' + (plain.stdin === null));
        const plainExit = await new Promise((resolve) => { plain.on('exit', (c) => resolve(c)); });
        log.push('plain exit ' + plainExit);
        const writer = new wt.Worker('1 + 1', { eval: true, stdin: true });
        writer.stdin.end();
        writer.stdin.on('error', (e) => log.push(e.code));
        log.push('write ' + writer.stdin.write('x'));
        const writerExit = await new Promise((resolve) => { writer.on('exit', (c) => resolve(c)); });
        log.push('writer exit ' + writerExit);
        clearTimeout(guard);
        console.log(log.join('|'));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "null true|plain exit 0|write false|ERR_STREAM_WRITE_AFTER_END|writer exit 0"
    );
}

#[test]
fn terminate_ends_a_worker_held_open_by_stdin() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const guard = setTimeout(() => process.exit(9), 5000);
        const worker = new wt.Worker(`process.stdin.on('data', () => {});`, { eval: true, stdin: true });
        const code = await worker.terminate();
        clearTimeout(guard);
        console.log('terminated ' + typeof code);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "terminated number");
}

#[test]
fn worker_stream_shim_is_scan_clean_and_present() {
    let form =
        thaw_js::scan::script_as_bundle(include_str!("../src/web/worker_stream.js")).into_owned();
    let report = thaw_js::scan::scan_bundle(&form).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(
        run_main("console.log(typeof __thaw_worker_stream.Readable + ' ' + typeof __thaw_worker_stream.Writable + ' ' + Object.isFrozen(__thaw_worker_stream));"),
        "function function true"
    );
}

#[test]
fn worker_stdio_stream_finished_and_ended_flags() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`process.stdin.resume(); process.stdin.on('end', () => { process.stdout.write('done'); });`, { eval: true, stdin: true, stdout: true, stderr: true });
        const log = [];
        log.push(worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded);
        worker.stdout.resume();
        worker.stderr.resume();
        worker.stdin.end('x');
        log.push(worker.stdin.writableFinished);
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
        await new Promise((resolve) => setTimeout(resolve, 20));
        log.push(worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded);
        console.log(log.join(','));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "false,false,false,false,true,true,true");
}

#[test]
fn worker_stdio_stdin_write_errors_surface_once() {
    // Not checked against Node: Node has no host sink to make fail. The expectations are the W5 error contract: a
    // throwing sink and a sink that reports the worker as gone each fail the write once, with the write callback and
    // one 'error' event.
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const original = globalThis.__thaw_worker_stdin;
        const run = async (sink) => {
          const worker = new wt.Worker(`require('node:worker_threads').parentPort.on('message', function() {});`, { eval: true, stdin: true });
          await new Promise((resolve) => worker.on('online', resolve));
          let errors = 0;
          let emitted = '';
          worker.stdin.on('error', (e) => { errors++; emitted = e.message; });
          globalThis.__thaw_worker_stdin = sink;
          const callback = await new Promise((resolve) => {
            worker.stdin.write(new Uint8Array([1]), (e) => resolve(e ? (e.code || 'error') + ':' + e.message : 'ok'));
          });
          globalThis.__thaw_worker_stdin = original;
          const exited = new Promise((resolve) => { worker.on('exit', resolve); });
          await worker.terminate();
          const code = await exited;
          return [callback, errors, emitted, code].join('|');
        };
        const thrown = await run(() => { throw new Error('host stdin failure'); });
        const gone = await run(() => false);
        console.log(thrown + ' || ' + gone);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "error:host stdin failure|1|host stdin failure|1 || ERR_WORKER_NOT_RUNNING:Worker stdin is not available|1|Worker stdin is not available|1"
    );
}

#[test]
fn worker_stdio_stdout_set_encoding_joins_split_utf8() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`process.stdout.write(Buffer.from([226, 130])); process.stdout.write(Buffer.from([172]));`, { eval: true, stdout: true });
        const events = [];
        worker.stdout.setEncoding('utf8');
        worker.stdout.on('data', (c) => events.push(JSON.stringify(c)));
        worker.stdout.on('end', () => events.push('end'));
        await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log(events.join(',') + ',' + worker.stdout.readableEnded);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), r#""€",end,true"#);
}

#[test]
fn worker_stdio_process_write_is_byte_exact_when_captured() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker("process.stdout.write(Buffer.from([250, 0, 255, 128, 195, 40, 251])); process.stderr.write(Buffer.from([251, 40, 195, 128, 255, 0, 250])); process.stdout.write('a'); process.stdout.write(Buffer.from([10]));", { eval: true, stdout: true, stderr: true });
        const hex = (c) => Array.from(c, (b) => b.toString(16).padStart(2, '0')).join('');
        let out = '';
        let err = '';
        worker.stdout.on('data', (c) => { out += hex(c); });
        worker.stderr.on('data', (c) => { err += hex(c); });
        await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        await new Promise((resolve) => setTimeout(resolve, 20));
        console.log(out + ' ' + err);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "fa00ff80c328fb610a fb28c380ff00fa");
}
