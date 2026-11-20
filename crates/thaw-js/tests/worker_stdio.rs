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
    assert_eq!(run_main(src), r#"hi|""|end,exit 0"#);
}

#[test]
fn worker_stdout_false_still_exits_normally() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`console.log('hi');`, { eval: true, stdout: false });
        const code = await new Promise((resolve) => { worker.on('exit', (c) => resolve(c)); });
        console.log('exit ' + code);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "hi|exit 0");
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
fn worker_stdin_set_encoding_leaves_encoding_unset_and_sets_readable_encoding() {
    // Node 22.23.1: process.stdin.encoding is undefined (never set) before and after setEncoding, readableEncoding is null then "utf8"; split UTF-8 still decodes.
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const { parentPort } = require('node:worker_threads'); const before = [typeof process.stdin.encoding, process.stdin.readableEncoding]; process.stdin.setEncoding('utf8'); const events = []; process.stdin.on('data', (c) => events.push(c)); process.stdin.on('end', () => { parentPort.postMessage(JSON.stringify([before, typeof process.stdin.encoding, process.stdin.readableEncoding, events])); });`, { eval: true, stdin: true });
        let got = '';
        worker.on('message', (v) => { got = String(v); });
        worker.stdin.write(new Uint8Array([226, 130]));
        worker.stdin.end(new Uint8Array([172]));
        await new Promise((resolve) => { worker.on('exit', () => resolve()); });
        console.log(got);
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        r#"[["undefined",null],"undefined","utf8",["€"]]"#
    );
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

// Node 22.23.1 (scratch: thaw-tmp/tmp-run/wk4/item1.js): unpiped worker output goes to the parent's process.stdout /
// process.stderr write (Buffers), captured output stays on worker.stdout / worker.stderr.
#[test]
fn worker_stdio_unpiped_output_reaches_parent_write() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const log = [];
      const origOut = process.stdout.write;
      const origErr = process.stderr.write;
      process.stdout.write = function (v, enc, cb) { if (typeof enc === 'function') cb = enc; log.push('out:' + Buffer.from(v).toString('hex') + ':' + Buffer.isBuffer(v)); if (cb) queueMicrotask(cb); return true; };
      process.stderr.write = function (v, enc, cb) { if (typeof enc === 'function') cb = enc; log.push('err:' + Buffer.from(v).toString('hex') + ':' + Buffer.isBuffer(v)); if (cb) queueMicrotask(cb); return true; };
      (async () => {
        const w = new wt.Worker("var wt = require('node:worker_threads'), out = Buffer.from([99, 0, 255, 128, 100]); process.stdout.write(out.subarray(1, 4)); process.stderr.write('01fe81', 'hex'); wt.parentPort.close();", { eval: true });
        const types = [w.stdout === null, w.stderr === null];
        await new Promise((resolve) => { w.on('exit', resolve); });
        const cap = new wt.Worker("var wt = require('node:worker_threads'), out = new Uint8Array([99, 2, 253, 130, 100]); process.stdout.write(new DataView(out.buffer, 1, 3)); process.stderr.write('03fc83', 'hex'); wt.parentPort.close();", { eval: true, stdout: true, stderr: true });
        let co = '';
        let ce = '';
        cap.stdout.on('data', (v) => { co += Buffer.from(v).toString('hex'); });
        cap.stderr.on('data', (v) => { ce += Buffer.from(v).toString('hex'); });
        await new Promise((resolve) => { cap.on('exit', resolve); });
        process.stdout.write = origOut;
        process.stderr.write = origErr;
        console.log(JSON.stringify({ types, log, co, ce, end: [cap.stdout.readableEnded, cap.stderr.readableEnded] }));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        r#"{"types":[false,false],"log":["out:00ff80:true","err:01fe81:true"],"co":"02fd82","ce":"03fc83","end":[true,true]}"#
    );
}

// Node 22.23.1: a DataView written by an unpiped worker reaches the parent's write as its bytes.
#[test]
fn worker_stdio_dataview_write_reaches_parent_as_bytes() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      const log = [];
      const origOut = process.stdout.write;
      process.stdout.write = function (v, enc, cb) { if (typeof enc === 'function') cb = enc; log.push(Buffer.from(v).toString('hex') + ':' + Buffer.isBuffer(v)); if (cb) queueMicrotask(cb); return true; };
      (async () => {
        const w = new wt.Worker("var out = new Uint8Array([99, 2, 253, 130, 100]); process.stdout.write(new DataView(out.buffer, 1, 3));", { eval: true });
        await new Promise((resolve) => { w.on('exit', resolve); });
        process.stdout.write = origOut;
        console.log(log.join(','));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "02fd82:true");
}

// Node 22.23.1 (scratch: thaw-tmp/tmp-run/wk4/item3.js): worker process.stdin chunks are Buffers, a string written with
// 'latin1' is encoded as latin1 ('Aÿ' -> 41ff), and end(chunk, callback) fires its callback once with 'finish' once.
#[test]
fn worker_stdin_buffers_latin1_and_end_callback_fire_once() {
    let src = r#"const wt = __thaw_worker_threads_exports;
      (async () => {
        const worker = new wt.Worker(`const wt = require('node:worker_threads'), received = []; process.stdin.on('data', (chunk) => { received.push([Buffer.isBuffer(chunk), Buffer.from(chunk).toString('hex')]); }); process.stdin.on('end', () => { wt.parentPort.postMessage(received); wt.parentPort.close(); });`, { eval: true, stdin: true });
        let writes = 0;
        let ends = 0;
        let finishes = 0;
        const backing = Buffer.from([99, 0, 255, 88]);
        const finalBacking = Buffer.from([77, 128, 66]);
        worker.stdin.on('finish', () => { finishes++; });
        const messages = new Promise((resolve) => { worker.on('message', resolve); });
        const exit = new Promise((resolve) => { worker.on('exit', resolve); });
        worker.stdin.write(backing.subarray(1, 3), (e) => { if (e) throw e; writes++; });
        worker.stdin.write('Aÿ', 'latin1', (e) => { if (e) throw e; writes++; });
        worker.stdin.end(finalBacking.subarray(1, 2), (e) => { if (e) throw e; ends++; });
        const m = await messages;
        const code = await exit;
        await new Promise((resolve) => setImmediate(resolve));
        console.log(JSON.stringify([code, writes, ends, finishes, m]));
      })().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        r#"[0,2,1,1,[[true,"00ff"],[true,"41ff"],[true,"80"]]]"#
    );
}
