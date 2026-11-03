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
