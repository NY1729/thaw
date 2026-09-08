#[test]
fn fs_read_and_write_streams_integrate_with_node_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_streams");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), stream = require('node:stream'); module.exports = async function (path) { var writer = fs.createWriteStream(path, { highWaterMark: 2 }), writerEvents = []; writer.on('open', function() { writerEvents.push('open'); }); writer.on('ready', function() { writerEvents.push('ready'); }); writer.on('close', function() { writerEvents.push('close'); }); var written = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.write('abc'); writer.end('def'); await written; await new Promise(function(resolve) { queueMicrotask(resolve); }); var appender = fs.createWriteStream(path, { flags: 'a' }), appended = new Promise(function(resolve, reject) { appender.on('error', reject); appender.on('finish', resolve); }); appender.end('!'); await appended; var reader = fs.createReadStream(path, { start: 1, end: 4, highWaterMark: 2 }), chunks = [], readerEvents = []; reader.on('open', function() { readerEvents.push('open'); }); reader.on('ready', function() { readerEvents.push('ready'); }); reader.on('data', function(value) { chunks.push(value.toString()); }); reader.on('close', function() { readerEvents.push('close'); }); var read = new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await read; await new Promise(function(resolve) { queueMicrotask(resolve); }); return [writer instanceof fs.WriteStream, writer instanceof stream.Writable, writer.bytesWritten, writerEvents, reader instanceof fs.ReadStream, reader instanceof stream.Readable, reader.bytesRead, chunks, readerEvents, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_streams_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("stream.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStreams").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,6,["open","ready","close"],true,true,4,["bc","de"],["open","ready","close"],"abcdef!"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_streams_use_incremental_positioned_host_io() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_incremental_streams");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); var writer = fs.createWriteStream(path, { flags: 'r+', start: 2 }), observed; await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.write('XY', function(error) { if (error) return reject(error); observed = fs.readFileSync(path, 'utf8'); writer.end('Z'); }); }); var beforeRead = fs.readFileSync(path, 'utf8'), chunks = [], reader = fs.createReadStream(path, { highWaterMark: 2 }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('data', function(chunk) { chunks.push(chunk.toString()); if (chunks.length === 1) fs.writeFileSync(path, 'abWXYZ'); }); reader.on('end', resolve); }); return [observed, beforeRead, writer.bytesWritten, chunks, reader.bytesRead, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_incremental_streams_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseIncrementalFsStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("stream.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseIncrementalFsStreams")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["abXYef","abXYZf",3,["ab","WX","YZ"],6,"abWXYZ"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_write_streams_apply_backpressure_and_serialize_callbacks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/queued.txt', directPath = root + '/direct.txt', events = [], writer = fs.createWriteStream(path, { highWaterMark: 3 }); writer.on('drain', function() { events.push('drain'); }); writer.on('finish', function() { events.push('finish'); }); var first = writer.write('ab', function() { events.push('first'); }), second = writer.write('cd', function() { events.push('second'); }), beforeDrain = fs.readFileSync(path, 'utf8'); var finished = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.end('ef', function() { events.push('end'); }); await finished; await new Promise(function(resolve) { queueMicrotask(resolve); }); var direct = new fs.WriteStream(directPath, { highWaterMark: 1 }), directResult = direct.write('x'); var directFinished = new Promise(function(resolve, reject) { direct.on('error', reject); direct.on('finish', resolve); }); direct.end(); await directFinished; return [first, second, beforeDrain, writer.writableLength, writer.writableHighWaterMark, events, fs.readFileSync(path, 'utf8'), directResult, direct.writableHighWaterMark, fs.readFileSync(directPath, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_stream_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStreamBackpressure")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,"ab",0,3,["first","drain","second","end","finish"],"abcdef",false,1,"x"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_builtin_pipes_transforms_and_finishes() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_stream");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); var streamPromises = require('node:stream/promises');\n\
             module.exports = async function () {\n\
             \x20 var output = []; var source = stream.Readable.from(['a', Buffer.from('b')]);\n\
             \x20 var upper = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } });\n\
             \x20 var sink = new stream.Writable({ write: function(chunk, encoding, callback) { output.push(chunk.toString()); callback(); } });\n\
             \x20 await new Promise(function(resolve, reject) { stream.pipeline(source, upper, sink, function(error) { if (error) reject(error); else resolve(); }); });\n\
             \x20 var queued = new stream.Readable(); queued.push('left'); queued.push('right'); queued.push(null); var combined = queued.read().toString();\n\
             \x20 var pass = new stream.PassThrough(); var passed = []; pass.on('data', function(chunk) { passed.push(chunk.toString()); }); var completion = streamPromises.finished(pass); pass.end('pass'); await completion;\n\
             \x20 var promiseOutput = []; await streamPromises.pipeline(stream.Readable.from(['promise']), new stream.Writable({ write: function(chunk, encoding, callback) { promiseOutput.push(chunk.toString()); callback(); } }));\n\
             \x20 var controller = new AbortController(); var aborted = new stream.Readable(); var reason; aborted.on('error', function(error) { reason = error; }); stream.addAbortSignal(controller.signal, aborted); controller.abort('stop'); await Promise.resolve();\n\
             \x20 return [output.join(''), sink.writableFinished, source.readableEnded, combined, passed.join(''), pass.readableEnded, promiseOutput.join(''), aborted.destroyed, reason.name, reason.code, reason.message, reason.cause];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStream = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseStream").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AB",true,true,"leftright","pass",true,"promise",true,"AbortError","ABORT_ERR","The operation was aborted","stop"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_and_duplex_from_bridge_supported_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function collect(value) { var output = []; value.on('data', function(chunk) { output.push(chunk.toString()); }); return new Promise(function(resolve, reject) { value.on('error', reject); value.on('end', function() { resolve(output.join('')); }); }); } module.exports = async function () { var upper = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } }), suffix = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString() + '!'); } }), composed = stream.compose(upper, suffix), composedResult = collect(composed); composed.write('a'); composed.end('b'); var iterable = stream.Duplex.from((async function*() { yield 'x'; await Promise.resolve(); yield 'y'; })()), iterableValues = []; for await (var value of iterable) iterableValues.push(value.toString()); var promised = stream.Duplex.from(Promise.resolve('z')), promisedValues = []; for await (var value of promised) promisedValues.push(value.toString()); var written = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { written.push(chunk.toString()); callback(); } }), readable = new stream.Readable(), pair = stream.Duplex.from({ writable: writable, readable: readable }), pairResult = collect(pair); pair.write('input'); pair.end(); readable.push('output'); readable.push(null); var functional = stream.Duplex.from(async function*(source) { for await (var chunk of source) yield chunk.toString().toUpperCase(); }), functionalResult = collect(functional); functional.write('function'); functional.end(); return [composed instanceof stream.Duplex, await composedResult, iterable instanceof stream.Duplex, iterableValues, promisedValues, await pairResult, written, await functionalResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamCompose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamCompose").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"A!B!",true,["x","y"],["z"],"output",["input"],"FUNCTION"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_bridges_function_stages_and_abort_signals() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_functions");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function collect(value) { var chunks = []; value.on('data', function(chunk) { chunks.push(String(chunk)); }); return new Promise(function(resolve, reject) { value.on('error', reject); value.on('end', function() { resolve(chunks.join('')); }); }); } module.exports = async function () { var signals = []; async function* upper(source, options) { signals.push(options.signal instanceof AbortSignal); for await (var chunk of source) yield String(chunk).toUpperCase(); } async function* suffix(source, options) { signals.push(options.signal instanceof AbortSignal); for await (var chunk of source) yield String(chunk) + '!'; } var transformed = stream.compose(upper, suffix), result = collect(transformed); transformed.write('a'); transformed.end('b'); var output = await result, aborted = false; async function* waiting(source, options) { options.signal.addEventListener('abort', function() { aborted = true; }); for await (var chunk of source) yield chunk; } var cancellable = stream.compose(waiting); cancellable.on('error', function() {}); var closed = new Promise(function(resolve) { cancellable.once('close', resolve); }); cancellable.resume(); await Promise.resolve(); var reason = new Error('stop'); cancellable.destroy(reason); await closed; return [output, signals, aborted, cancellable.errored === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_functions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeFunctions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeFunctions")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["A!B!",[true,true],true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_abort_signals_use_node_abort_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_abort_errors");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function code(action) { try { action(); } catch (error) { return error.code; } } module.exports = async function () { var reason = new Error('stop'), pre = new AbortController(); pre.abort(reason); var direct = new stream.PassThrough(), directError = new Promise(function(resolve) { direct.once('error', resolve); }); stream.addAbortSignal(pre.signal, direct); var directFailure = await directError, source = new stream.Readable({ read: function() {} }), controller = new AbortController(), stageSignal, finalized = false; async function* stage(input, options) { stageSignal = options.signal; try { for await (var value of input) yield value; } finally { finalized = true; } } var composed = source.compose(stage, { signal: controller.signal }), composedError = new Promise(function(resolve) { composed.once('error', resolve); }); composed.resume(); await Promise.resolve(); controller.abort(reason); var composedFailure = await composedError; await Promise.resolve(); return [directFailure.name, directFailure.code, directFailure.cause === reason, composedFailure.name, composedFailure.code, composedFailure.cause === reason, stageSignal.aborted, finalized, source.destroyed, composed.destroyed, code(function() { stream.addAbortSignal({}, direct); }), code(function() { stream.addAbortSignal(pre.signal, {}); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_abort_errors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamAbortErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamAbortErrors").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AbortError","ABORT_ERR",true,"AbortError","ABORT_ERR",true,true,true,true,true,"ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_abort_signals_error_locked_web_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_abort");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var reason = new Error('stop'), readableCancelled = false, readable = new ReadableStream({ cancel: function() { readableCancelled = true; } }), reader = readable.getReader(), pendingRead = reader.read(), readableController = new AbortController(); stream.addAbortSignal(readableController.signal, readable); readableController.abort(reason); var readError; try { await pendingRead; } catch (error) { readError = error; } var writableAborted = false, writable = new WritableStream({ abort: function() { writableAborted = true; } }), writer = writable.getWriter(), writerClosed = writer.closed.catch(function(error) { return error; }), writableController = new AbortController(); stream.addAbortSignal(writableController.signal, writable); writableController.abort(reason); var writeError; try { await writer.write('value'); } catch (error) { writeError = error; } var closedError = await writerClosed, pre = new AbortController(); pre.abort(reason); var preReadable = new ReadableStream(); stream.addAbortSignal(pre.signal, preReadable); var preError; try { await preReadable.getReader().read(); } catch (error) { preError = error; } return [readError.name, readError.code, readError.cause === reason, readableCancelled, readable.locked, writeError === closedError, writeError.name, writeError.code, writeError.cause === reason, writableAborted, writable.locked, preError.name, preError.code, preError.cause === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_abort_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWebAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWebAbort").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AbortError","ABORT_ERR",true,false,true,true,"AbortError","ABORT_ERR",true,false,true,"AbortError","ABORT_ERR",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_writable_serializes_abort_behind_pending_writes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_writable_abort_order");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var release, gate = new Promise(function(resolve) { release = resolve; }), events = [], reason = new Error('stop'), writable = new WritableStream({ write: function() { events.push('write-start'); return gate.then(function() { events.push('write-end'); }); }, abort: function(error) { events.push(['abort', error === reason]); } }), writer = writable.getWriter(), directAbort, directClose; try { await writable.abort(reason); } catch (error) { directAbort = error.code; } try { await writable.close(); } catch (error) { directClose = error.code; } var write = writer.write('x').then(function() { events.push('write-resolve'); }), abort = writer.abort(reason).then(function() { events.push('abort-resolve'); }), pending = events.slice(); release(); await Promise.all([write, abort]); var closedError; try { await writer.closed; } catch (error) { closedError = error; } var laterError; try { await writer.write('y'); } catch (error) { laterError = error; } var repeated = await writer.abort(new Error('ignored')); return [directAbort, directClose, pending, events, closedError === reason, laterError === reason, repeated]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_writable_abort_order_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebWritableAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebWritableAbort").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE",[],["write-start","write-end",["abort",true],"write-resolve","abort-resolve"],true,true,null]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_writable_tracks_strategy_backpressure_and_ready() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_writable_backpressure");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var releases = [], sized = [], stream = new WritableStream({ write: function() { return new Promise(function(resolve) { releases.push(resolve); }); } }, { highWaterMark: 2, size: function(value) { sized.push(value); return value; } }), writer = stream.getWriter(), initial = writer.desiredSize, first = writer.write(1), afterFirst = writer.desiredSize, firstReady = false; writer.ready.then(function() { firstReady = true; }); await Promise.resolve(); var firstHadCapacity = firstReady, second = writer.write(2), afterSecond = writer.desiredSize, secondReady = false, ready = writer.ready.then(function() { secondReady = true; }); await Promise.resolve(); await Promise.resolve(); var secondBackpressured = !secondReady; releases.shift()(); await first; await Promise.resolve(); var afterFirstCompletion = [writer.desiredSize, secondReady]; await Promise.resolve(); releases.shift()(); await second; await ready; var final = [writer.desiredSize, secondReady], zero = new WritableStream({}, { highWaterMark: 0 }), zeroWriter = zero.getWriter(), zeroReady = false; zeroWriter.ready.then(function() { zeroReady = true; }); await Promise.resolve(); var zeroInitiallyPending = !zeroReady; await zeroWriter.close(); await zeroWriter.ready; var invalidHighWaterMark, invalidSizeStream = new WritableStream({}, { size: function() { return -1; } }), invalidSizeWriter = invalidSizeStream.getWriter(), invalidSize = await invalidSizeWriter.write('x').catch(function(error) { return error; }), invalidClosed = await invalidSizeWriter.closed.catch(function(error) { return error; }), invalidReady = await invalidSizeWriter.ready.catch(function(error) { return error; }); try { new WritableStream({}, { highWaterMark: -1 }); } catch (error) { invalidHighWaterMark = error.code; } return [initial, afterFirst, firstHadCapacity, afterSecond, secondBackpressured, afterFirstCompletion, final, sized, zeroInitiallyPending, zeroWriter.desiredSize, invalidHighWaterMark, invalidSize.name, invalidSize.code, invalidClosed === invalidSize, invalidReady === invalidSize]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_writable_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebWritableBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebWritableBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,1,true,-1,true,[0,false],[2,true],[1,2],true,0,"ERR_INVALID_ARG_VALUE","RangeError","ERR_INVALID_ARG_VALUE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_locks_can_be_released_and_reacquired() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_release_locks");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var readableController, readable = new ReadableStream({ start: function(controller) { readableController = controller; } }), reader = readable.getReader(), pendingRead = reader.read().catch(function(error) { return error; }); reader.releaseLock(); var pendingReadError = await pendingRead, oldReadError, oldReaderClosed; try { await reader.read(); } catch (error) { oldReadError = error; } try { await reader.closed; } catch (error) { oldReaderClosed = error; } var readerUnlocked = !readable.locked, replacementReader = readable.getReader(); readableController.enqueue('value'); var replacementRead = await replacementReader.read(), releaseWrite, writeGate = new Promise(function(resolve) { releaseWrite = resolve; }), writable = new WritableStream({ write: function() { return writeGate; } }), writer = writable.getWriter(), pendingWrite = writer.write('x'); writer.releaseLock(); var oldWriteError, oldWriterReady, oldWriterClosed; try { await writer.write('y'); } catch (error) { oldWriteError = error; } try { await writer.ready; } catch (error) { oldWriterReady = error; } try { await writer.closed; } catch (error) { oldWriterClosed = error; } var writerUnlocked = !writable.locked, replacementWriter = writable.getWriter(); releaseWrite(); await pendingWrite; await replacementWriter.close(); return [pendingReadError.code, oldReadError.code, oldReaderClosed.code, readerUnlocked, replacementRead, oldWriteError.code, oldWriterReady.code, oldWriterClosed.code, writerUnlocked, writable.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_release_locks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamLocks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamLocks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE","ERR_INVALID_STATE",true,{"value":"value","done":false},"ERR_INVALID_STATE","ERR_INVALID_STATE","ERR_INVALID_STATE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_stream_tee_branches_and_aggregates_cancellation() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_tee");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function code(action) { try { action(); } catch (error) { return error.code; } } var pulls = 0, chunks = [{ n: 1 }, { n: 2 }], source = new ReadableStream({ pull: function(controller) { controller.enqueue(chunks[pulls++]); if (pulls === 2) controller.close(); } }), branches = source.tee(), leftReader = branches[0].getReader(), rightReader = branches[1].getReader(), leftFirst = await leftReader.read(), leftSecond = await leftReader.read(), rightFirst = await rightReader.read(), rightSecond = await rightReader.read(), leftDone = await leftReader.read(), rightDone = await rightReader.read(), cancelReasons, cancelSource = new ReadableStream({ cancel: function(reasons) { cancelReasons = reasons; } }), cancelBranches = cancelSource.tee(), firstSettled = false, firstCancel = cancelBranches[0].cancel('left').then(function() { firstSettled = true; }); await Promise.resolve(); await Promise.resolve(); var firstPending = !firstSettled, secondCancel = cancelBranches[1].cancel('right'); await Promise.all([firstCancel, secondCancel]); var errorController, failed = new ReadableStream({ start: function(controller) { errorController = controller; } }), failedBranches = failed.tee(), failedLeftReader = failedBranches[0].getReader(), failedRightReader = failedBranches[1].getReader(), failedLeft = failedLeftReader.read().catch(function(error) { return error; }), failedRight = failedRightReader.read().catch(function(error) { return error; }), failure = new Error('boom'); errorController.error(failure); var failures = await Promise.all([failedLeft, failedRight]), locked = new ReadableStream(), lockedReader = locked.getReader(), lockedCode = code(function() { locked.tee(); }); lockedReader.releaseLock(); return [pulls, leftFirst.value === chunks[0], leftSecond.value === chunks[1], rightFirst.value === chunks[0], rightSecond.value === chunks[1], leftDone.done, rightDone.done, source.locked, firstPending, cancelReasons, cancelSource.locked, failures[0] === failure, failures[1] === failure, failed.locked, lockedCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_tee_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamTee = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamTee").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,true,true,true,true,true,true,true,true,["left","right"],true,true,true,true,"ERR_INVALID_STATE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_byte_streams_support_byob_readers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_byob");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var requestClass, viewClass, desired, source = new ReadableStream({ type: 'bytes', pull: function(controller) { var request = controller.byobRequest; requestClass = request.constructor === web.ReadableStreamBYOBRequest; viewClass = request.view instanceof Uint8Array; desired = controller.desiredSize; request.view[0] = 65; request.view[1] = 66; request.respond(2); controller.close(); } }), reader = source.getReader({ mode: 'byob' }), first = await reader.read(new Uint8Array(8)), done = await reader.read(new Uint8Array(4)), queued = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([1, 2, 3])); controller.close(); } }), queuedReader = queued.getReader({ mode: 'byob' }), queuedFirst = await queuedReader.read(new Uint8Array(2)), queuedSecond = await queuedReader.read(new Uint8Array(4)), queuedDone = await queuedReader.read(new Uint8Array(1)), defaultReader = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([9])); controller.close(); } }).getReader(), defaultValue = await defaultReader.read(), minController, minStream = new ReadableStream({ type: 'bytes', start: function(controller) { minController = controller; } }), minReader = minStream.getReader({ mode: 'byob' }), minSettled = false, minRead = minReader.read(new Uint8Array(5), { min: 3 }).then(function(result) { minSettled = true; return result; }); minController.enqueue(new Uint8Array([1, 2])); await Promise.resolve(); await Promise.resolve(); var waitedForMin = !minSettled; minController.enqueue(new Uint8Array([3, 4])); var minResult = await minRead, allocatedInfo, allocated = new ReadableStream({ type: 'bytes', autoAllocateChunkSize: 4, pull: function(controller) { allocatedInfo = controller.byobRequest.view.byteLength; controller.byobRequest.view[0] = 7; controller.byobRequest.respond(1); controller.close(); } }), allocatedReader = allocated.getReader(), allocatedValue = await allocatedReader.read(), teeSource = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([5, 6])); controller.close(); } }), teeBranches = teeSource.tee(), teeLeft = await teeBranches[0].getReader({ mode: 'byob' }).read(new Uint8Array(2)), teeRight = await teeBranches[1].getReader({ mode: 'byob' }).read(new Uint8Array(2)), wrongStream, wrongMode; try { new ReadableStream().getReader({ mode: 'byob' }); } catch (error) { wrongStream = error.code; } try { new ReadableStream().getReader({ mode: 'invalid' }); } catch (error) { wrongMode = error.code; } return [requestClass, viewClass, desired, Array.from(first.value), first.done, done.value.byteLength, done.done, Array.from(queuedFirst.value), Array.from(queuedSecond.value), queuedDone.value.byteLength, queuedDone.done, Array.from(defaultValue.value), waitedForMin, Array.from(minResult.value), allocatedInfo, Array.from(allocatedValue.value), Array.from(teeLeft.value), Array.from(teeRight.value), teeLeft.value !== teeRight.value, wrongStream, wrongMode, web.ReadableByteStreamController === globalThis.ReadableByteStreamController, web.ReadableStreamBYOBReader === globalThis.ReadableStreamBYOBReader]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_byob_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamByob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamByob").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,0,[65,66],false,0,true,[1,2],[3],0,true,[9],true,[1,2,3,4],4,[7],[5,6],[5,6],true,"ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn byob_supports_dataview_alignment_and_request_lifecycle() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_byob_views");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var dataInfo, dataStream = new ReadableStream({ type: 'bytes', pull: function(controller) { var request = controller.byobRequest; dataInfo = [request.view.constructor.name, request.view.byteLength]; request.view[0] = 3; request.respond(1); controller.close(); } }), dataResult = await dataStream.getReader({ mode: 'byob' }).read(new DataView(new ArrayBuffer(4))), calls = 0, closeCode, viewCode, staleRequest, typed = new ReadableStream({ type: 'bytes', pull: function(controller) { calls++; var request = controller.byobRequest; staleRequest = request; if (calls === 1) { try { request.respondWithNewView(new Uint8Array(request.view.buffer, request.view.byteOffset + 1, 1)); } catch (error) { viewCode = error.code; } request.view[0] = 1; request.respond(1); try { controller.close(); } catch (error) { closeCode = error.code; } } else { request.view[0] = 2; request.respond(1); controller.close(); } } }), typedResult = await typed.getReader({ mode: 'byob' }).read(new Uint16Array(2)), staleCode; try { staleRequest.respond(0); } catch (error) { staleCode = error.code; } return [dataInfo, dataResult.value.constructor.name, dataResult.value.byteLength, dataResult.value.getUint8(0), calls, typedResult.value.constructor.name, typedResult.value.length, Array.from(new Uint8Array(typedResult.value.buffer, typedResult.value.byteOffset, typedResult.value.byteLength)), viewCode, closeCode, staleCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_byob_views_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamByobViews = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamByobViews")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["Uint8Array",4],"DataView",1,3,2,"Uint16Array",1,[1,2],"ERR_INVALID_ARG_VALUE","ERR_INVALID_STATE","ERR_INVALID_STATE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_default_controllers_manage_errors_and_termination() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_controllers");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var writableController, writableReason = new Error('writable'), writable = new WritableStream({ start: function(controller) { writableController = controller; } }), writableWriter = writable.getWriter(); writableController.error(writableReason); var writableClosed = await writableWriter.closed.catch(function(error) { return error; }), writableWrite = await writableWriter.write('x').catch(function(error) { return error; }), abortController, abortReason = new Error('abort'), abortSeen, abortable = new WritableStream({ start: function(controller) { abortController = controller; }, abort: function(reason) { abortSeen = reason; } }), abortWriter = abortable.getWriter(); await abortWriter.abort(abortReason); var transformController, started = false, terminated = new TransformStream({ start: function(controller) { transformController = controller; started = true; }, transform: function(value, controller) { controller.enqueue(value + 1); controller.terminate(); } }), terminatedWriter = terminated.writable.getWriter(), terminatedReader = terminated.readable.getReader(), terminatedWrite = terminatedWriter.write(1), transformed = await terminatedReader.read(), transformedDone = await terminatedReader.read(); await terminatedWrite; var laterWrite = await terminatedWriter.write(2).catch(function(error) { return error; }), transformReason = new Error('transform'), failed = new TransformStream({ transform: function(value, controller) { controller.error(transformReason); } }), failedWriter = failed.writable.getWriter(), failedReader = failed.readable.getReader(), failedRead = failedReader.read().catch(function(error) { return error; }); await failedWriter.write(1); var failedValue = await failedRead, failedClosed = await failedWriter.closed.catch(function(error) { return error; }), failedLater = await failedWriter.write(2).catch(function(error) { return error; }); return [writableController.constructor === web.WritableStreamDefaultController, writableController.signal.aborted, writableClosed === writableReason, writableWrite === writableReason, abortController.signal.aborted, abortController.signal.reason === abortReason, abortSeen === abortReason, started, transformController.constructor === web.TransformStreamDefaultController, transformController.desiredSize, transformed.value, transformedDone.done, laterWrite.name, laterWrite.code, failedValue === transformReason, failedClosed === transformReason, failedLater === transformReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_controllers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamControllers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamControllers")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,true,true,true,true,true,true,true,null,2,true,"TypeError","ERR_INVALID_STATE",true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_state_errors_match_node_codes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_state_errors");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } async function captureAsync(action) { try { await action(); return ['resolved']; } catch (error) { return [error.name, error.code]; } } var readableController, readable = new ReadableStream({ start: function(controller) { readableController = controller; controller.close(); } }), closeAgain = capture(function() { readableController.close(); }), enqueueClosed = capture(function() { readableController.enqueue(1); }), reader = readable.getReader(), cancelLocked = await captureAsync(function() { return readable.cancel(); }), cancelClosed = await captureAsync(function() { return reader.cancel(); }), writableController, writable = new WritableStream({ start: function(controller) { writableController = controller; } }), writer = writable.getWriter(), firstClose = await captureAsync(function() { return writer.close(); }), secondClose = await captureAsync(function() { return writer.close(); }), writeClosed = await captureAsync(function() { return writer.write(1); }), abortClosed = await captureAsync(function() { return writer.abort('ignored'); }), directLocked = await captureAsync(function() { return writable.close(); }), controllerErrorClosed = capture(function() { writableController.error(new Error('ignored')); }), errorReason = new Error('failure'), erroredController, errored = new WritableStream({ start: function(controller) { erroredController = controller; } }), erroredWriter = errored.getWriter(); erroredController.error(errorReason); var closeError = await erroredWriter.close().catch(function(error) { return error; }), abortErrored = await captureAsync(function() { return erroredWriter.abort(); }); return [closeAgain, enqueueClosed, cancelLocked, cancelClosed, firstClose, secondClose, writeClosed, abortClosed, directLocked, controllerErrorClosed, closeError === errorReason, abortErrored]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_state_errors_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamStateErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamStateErrors")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["resolved"],["resolved"],["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["resolved"],["TypeError","ERR_INVALID_STATE"],null,true,["resolved"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn transform_stream_applies_readable_backpressure_and_strategies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_transform_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var events = [], transform = new TransformStream({ transform: function(value, controller) { events.push('transform' + value); controller.enqueue(value); } }, { highWaterMark: 3 }, { highWaterMark: 1 }), writer = transform.writable.getWriter(), reader = transform.readable.getReader(), firstWrite = writer.write(1).then(function() { events.push('write1'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var afterFirst = events.slice(), secondWrite = writer.write(2).then(function() { events.push('write2'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var whileFull = events.slice(), first = await reader.read(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var afterRead = events.slice(), second = await reader.read(); await Promise.all([firstWrite, secondWrite]); var defaultEvents = [], defaults = new TransformStream({ transform: function(value, controller) { defaultEvents.push('transform'); controller.enqueue(value); } }), defaultWriter = defaults.writable.getWriter(), defaultReader = defaults.readable.getReader(), defaultWrite = defaultWriter.write(3).then(function() { defaultEvents.push('write'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var defaultBlocked = defaultEvents.slice(), defaultRead = defaultReader.read(); await Promise.all([defaultWrite, defaultRead]); var sizes = [], desired = [], strategic = new TransformStream({ transform: function(value, controller) { desired.push(controller.desiredSize); controller.enqueue(value); desired.push(controller.desiredSize); } }, { highWaterMark: 5, size: function(value) { sizes.push(['w', value]); return 2; } }, { highWaterMark: 4, size: function(value) { sizes.push(['r', value]); return 3; } }), strategicWriter = strategic.writable.getWriter(), strategicReader = strategic.readable.getReader(), strategicWrite = strategicWriter.write('x'); await strategicWrite; var strategicValue = await strategicReader.read(), reason = new Error('stop'), cancelled = new TransformStream(), cancelledWriter = cancelled.writable.getWriter(), cancelledReader = cancelled.readable.getReader(), blockedWrite = cancelledWriter.write(1).catch(function(error) { return error; }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); await cancelledReader.cancel(reason); var writeError = await blockedWrite, closedError = await cancelledWriter.closed.catch(function(error) { return error; }); return [afterFirst, whileFull, first.value, afterRead, second.value, writer.desiredSize, defaultBlocked, defaultEvents, sizes, desired, strategicValue.value, strategicWriter.desiredSize, writeError === reason, closedError === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_transform_stream_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTransformStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTransformStreamBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["transform1","write1"],["transform1","write1"],1,["transform1","write1","transform2","write2"],2,3,[],["transform","write"],[["w","x"],["r","x"]],[4,1],"x",5,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_stream_from_adapts_sync_and_async_iterables() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_from");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var values = []; for await (var value of ReadableStream.from([Promise.resolve(1), 2])) values.push(value); var asyncCalls = 0, asyncIterable = { next: async function() { asyncCalls++; if (asyncCalls === 1) return { value: 'async', done: false }; return { done: true }; } }; asyncIterable[Symbol.asyncIterator] = function() { return this; }; var asyncValues = []; for await (var asyncValue of web.ReadableStream.from(asyncIterable)) asyncValues.push(asyncValue); var returned = [], endless = { next: function() { return { value: 7, done: false }; }, return: function(reason) { returned.push(reason); return { done: true }; } }; endless[Symbol.iterator] = function() { return this; }; var cancelled = ReadableStream.from(endless), cancelledReader = cancelled.getReader(); await cancelledReader.read(); await cancelledReader.cancel('stop'); var originalController, original = new ReadableStream({ start: function(controller) { originalController = controller; } }), adapted = ReadableStream.from(original), lockedImmediately = original.locked, adaptedReader = adapted.getReader(); originalController.enqueue('source'); originalController.close(); var adaptedValue = await adaptedReader.read(), adaptedDone = await adaptedReader.read(), calls = 0, failure = new Error('boom'), failing = { next: function() { calls++; if (calls === 1) return { value: 'first', done: false }; throw failure; } }; failing[Symbol.iterator] = function() { return this; }; var failingReader = ReadableStream.from(failing).getReader(), first = await failingReader.read(), received = await failingReader.read().catch(function(error) { return error; }), invalid; try { ReadableStream.from(1); } catch (error) { invalid = [error.name, error.code]; } return [values, asyncValues, asyncCalls, returned, lockedImmediately, adaptedValue.value, adaptedDone.done, first.value, received === failure, invalid, web.ReadableStream.from === globalThis.ReadableStream.from]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_from_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamFrom = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamFrom").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[1,2],["async"],2,["stop"],true,"source",true,"first",true,["TypeError","ERR_ARG_NOT_ITERABLE"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_readable_tracks_strategy_pull_and_delayed_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_readable_strategy");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var sizes = [], desiredAtClose, source = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('bb'); controller.close(); desiredAtClose = controller.desiredSize; } }, { highWaterMark: 4, size: function(value) { sizes.push(value); return value.length; } }), reader = source.getReader(), closed = false; reader.closed.then(function() { closed = true; }); await Promise.resolve(); var closedWithQueue = closed, first = await reader.read(), closedAfterFirst = closed, second = await reader.read(); await reader.closed; var releases = [], pulls = 0, active = 0, maxActive = 0, pulling = new ReadableStream({ pull: function(controller) { pulls++; active++; maxActive = Math.max(maxActive, active); var value = pulls; return new Promise(function(resolve) { releases.push(function() { controller.enqueue(value); active--; resolve(); }); }); } }, { highWaterMark: 2 }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var initialPulls = pulls; releases.shift()(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var refilledPulls = pulls; releases.shift()(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var stoppedAtMark = pulls, pullingReader = pulling.getReader(), pulledFirst = await pullingReader.read(), pulledSecond = await pullingReader.read(), byteController, bytes = new ReadableStream({ type: 'bytes', start: function(controller) { byteController = controller; controller.enqueue(Uint8Array.from([1, 2])); } }, { highWaterMark: 3 }), byteDesiredBefore = byteController.desiredSize, byteReader = bytes.getReader({ mode: 'byob' }), byteValue = await byteReader.read(new Uint8Array(1)), byteDesiredAfter = byteController.desiredSize; byteController.close(); await byteReader.read(new Uint8Array(2)); await byteReader.read(new Uint8Array(1)); var invalidHighWaterMark, invalidByteSize; try { new ReadableStream({}, { highWaterMark: -1 }); } catch (error) { invalidHighWaterMark = error.code; } try { new ReadableStream({ type: 'bytes' }, { size: function() { return 1; } }); } catch (error) { invalidByteSize = error.code; } return [sizes, desiredAtClose, closedWithQueue, first.value, closedAfterFirst, second.value, closed, initialPulls, refilledPulls, stoppedAtMark, maxActive, pulledFirst.value, pulledSecond.value, byteDesiredBefore, Array.from(byteValue.value), byteDesiredAfter, invalidHighWaterMark, invalidByteSize]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_readable_strategy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebReadableStrategy = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebReadableStrategy")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","bb"],1,false,"a",false,"bb",true,1,2,2,1,1,2,1,[1],2,"ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_queuing_strategies_validate_and_feed_stream_sizes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_queuing_strategies");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } var byte = new web.ByteLengthQueuingStrategy({ highWaterMark: '4' }), count = new web.CountQueuingStrategy({ highWaterMark: 2 }), byteDesired, byteStream = new ReadableStream({ start: function(controller) { controller.enqueue(new Uint8Array(3)); byteDesired = controller.desiredSize; controller.close(); } }, byte), countDesired, countStream = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); countDesired = controller.desiredSize; controller.close(); } }, count), negative = new CountQueuingStrategy({ highWaterMark: -1 }); return [byte.highWaterMark, count.highWaterMark, byte.size(new Uint8Array(3)), byte.size({ byteLength: 5 }), count.size(null), Object.keys(byte), Object.keys(ByteLengthQueuingStrategy.prototype), byteDesired, countDesired, byteStream.locked, countStream.locked, capture(function() { new ByteLengthQueuingStrategy(); }), capture(function() { new CountQueuingStrategy({}); }), capture(function() { new ReadableStream({}, negative); }), capture(function() { Object.getOwnPropertyDescriptor(CountQueuingStrategy.prototype, 'highWaterMark').get.call({}); }), web.ByteLengthQueuingStrategy === globalThis.ByteLengthQueuingStrategy]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_queuing_strategies_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebQueuingStrategies = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebQueuingStrategies")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,2,3,5,1,[],["highWaterMark","size"],1,0,false,false,["TypeError","ERR_INVALID_ARG_TYPE"],["TypeError","ERR_MISSING_OPTION"],["RangeError","ERR_INVALID_ARG_VALUE"],["TypeError",null],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_pipe_to_propagates_completion_errors_and_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_pipe_to_options");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var writes = [], closed = false, normalSource = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); controller.close(); } }), normalDestination = new WritableStream({ write: function(value) { writes.push(value); }, close: function() { closed = true; } }); await normalSource.pipeTo(normalDestination, { preventClose: true }); var destinationFailure = new Error('destination'), cancelledWith, failingDestination = new WritableStream({ write: function() { throw destinationFailure; } }), cancellableSource = new ReadableStream({ pull: function(controller) { controller.enqueue('x'); }, cancel: function(error) { cancelledWith = error; } }), destinationResult; try { await cancellableSource.pipeTo(failingDestination); } catch (error) { destinationResult = error; } var sourceFailure = new Error('source'), abortedWith, failedSource = new ReadableStream({ start: function(controller) { controller.error(sourceFailure); } }), abortableDestination = new WritableStream({ abort: function(error) { abortedWith = error; } }), sourceResult; try { await failedSource.pipeTo(abortableDestination); } catch (error) { sourceResult = error; } var controller = new AbortController(), abortReason = new Error('stop'), signalCancelled, signalAborted, pendingSource = new ReadableStream({ pull: function() {}, cancel: function(error) { signalCancelled = error; } }), pendingDestination = new WritableStream({ abort: function(error) { signalAborted = error; } }), signalled = pendingSource.pipeTo(pendingDestination, { signal: controller.signal }).catch(function(error) { return error; }); controller.abort(abortReason); var signalResult = await signalled, preventedController = new AbortController(), preventedCancelled = false, preventedAborted = false, preventedSource = new ReadableStream({ pull: function() {}, cancel: function() { preventedCancelled = true; } }), preventedDestination = new WritableStream({ abort: function() { preventedAborted = true; } }), prevented = preventedSource.pipeTo(preventedDestination, { signal: preventedController.signal, preventCancel: true, preventAbort: true }).catch(function(error) { return error; }); preventedController.abort(abortReason); var preventedResult = await prevented; return [writes, closed, !normalSource.locked, !normalDestination.locked, destinationResult === destinationFailure, cancelledWith === destinationFailure, !cancellableSource.locked, !failingDestination.locked, sourceResult === sourceFailure, abortedWith === sourceFailure, signalResult === abortReason, signalCancelled === abortReason, signalAborted === abortReason, !pendingSource.locked, !pendingDestination.locked, preventedResult === abortReason, preventedCancelled, preventedAborted, !preventedSource.locked, !preventedDestination.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_pipe_to_options_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebPipeTo = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebPipeTo").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","b"],false,true,true,true,true,true,true,true,true,true,true,true,true,true,true,false,false,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_pipe_through_validates_locks_and_propagates_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_pipe_through");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function code(action) { try { action(); } catch (error) { return error.code; } } var lockedSource = new ReadableStream(), lockedReader = lockedSource.getReader(), sourceLockCode = code(function() { lockedSource.pipeThrough(new TransformStream()); }); lockedReader.releaseLock(); var lockedTransform = new TransformStream(), lockedWriter = lockedTransform.writable.getWriter(), destinationLockCode = code(function() { new ReadableStream().pipeThrough(lockedTransform); }); lockedWriter.releaseLock(); var source = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); controller.close(); } }), upper = new TransformStream({ transform: function(value, controller) { controller.enqueue(value.toUpperCase()); } }), output = source.pipeThrough(upper), values = []; for await (var value of output) values.push(value); var failure = new Error('transform'), cancelledWith, failingSource = new ReadableStream({ start: function(controller) { controller.enqueue('x'); }, cancel: function(error) { cancelledWith = error; } }), failingTransform = new TransformStream({ transform: function() { throw failure; } }), failedOutput = failingSource.pipeThrough(failingTransform), failedReader = failedOutput.getReader(), received; try { await failedReader.read(); } catch (error) { received = error; } await new Promise(function(resolve) { setTimeout(resolve, 0); }); return [sourceLockCode, destinationLockCode, values, !source.locked, !upper.writable.locked, received === failure, cancelledWith === failure, !failingSource.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_pipe_through_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebPipeThrough = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebPipeThrough").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE",["A","B"],true,true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_adapters_retain_locks_and_await_teardown() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_adapter_locks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var normalReadable = new ReadableStream({ start: function(controller) { controller.close(); } }), nodeReadable = stream.Readable.fromWeb(normalReadable); for await (var value of nodeReadable) {} var normalWritable = new WritableStream(), nodeWritable = stream.Writable.fromWeb(normalWritable), normalWritableClosed = new Promise(function(resolve, reject) { nodeWritable.once('error', reject); nodeWritable.once('close', resolve); }); nodeWritable.end(); await normalWritableClosed; var cancelResolve, cancelGate = new Promise(function(resolve) { cancelResolve = resolve; }), readableEvents = [], pendingReadable = new ReadableStream({ cancel: function() { readableEvents.push('cancel'); return cancelGate; } }), destroyedReadable = stream.Readable.fromWeb(pendingReadable), readableClosed = new Promise(function(resolve) { destroyedReadable.once('error', function() { readableEvents.push('error'); }); destroyedReadable.once('close', function() { readableEvents.push('close'); resolve(); }); }), readableReason = new Error('read stop'); destroyedReadable.destroy(readableReason); await Promise.resolve(); var readablePending = [pendingReadable.locked, readableEvents.slice()]; cancelResolve(); await readableClosed; var abortResolve, abortGate = new Promise(function(resolve) { abortResolve = resolve; }), writableEvents = [], pendingWritable = new WritableStream({ abort: function() { writableEvents.push('abort'); return abortGate; } }), destroyedWritable = stream.Writable.fromWeb(pendingWritable), writableClosed = new Promise(function(resolve) { destroyedWritable.once('error', function() { writableEvents.push('error'); }); destroyedWritable.once('close', function() { writableEvents.push('close'); resolve(); }); }), writableReason = new Error('write stop'); destroyedWritable.destroy(writableReason); await Promise.resolve(); await Promise.resolve(); var writablePending = [pendingWritable.locked, writableEvents.slice()]; abortResolve(); await writableClosed; return [normalReadable.locked, normalWritable.locked, readablePending, readableEvents, destroyedReadable.errored === readableReason, pendingReadable.locked, writablePending, writableEvents, destroyedWritable.errored === writableReason, pendingWritable.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_adapter_locks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebAdapterLocks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebAdapterLocks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,[true,["cancel"]],["cancel","error","close"],true,true,[true,["abort"]],["abort","error","close"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_to_web_applies_backpressure_and_awaits_cancel() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_to_web_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var source = new stream.Readable({ autoDestroy: false, read: function() {} }), web = stream.Readable.toWeb(source), initiallyPaused = source.isPaused(); source.push('a'); source.push('b'); var beforeRead = [source.isPaused(), source.readableLength], reader = web.getReader(), first = await reader.read(), afterFirst = [source.isPaused(), source.readableLength], second = await reader.read(); source.push(null); var done = await reader.read(), reason = new Error('stop'), events = [], cancelledSource = new stream.Readable({ read: function() {} }), cancelledWeb = stream.Readable.toWeb(cancelledSource), cancelledReader = cancelledWeb.getReader(); cancelledSource.once('error', function(error) { events.push(['error', error === reason]); }); cancelledSource.once('close', function() { events.push(['close']); }); var cancellation = cancelledReader.cancel(reason).then(function() { events.push(['cancelled']); }); var beforeCancelSettlement = events.slice(); await cancellation; return [initiallyPaused, beforeRead, first.value.toString(), afterFirst, second.value.toString(), done.done, beforeCancelSettlement, events, cancelledSource.destroyed, cancelledWeb.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_to_web_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableToWebBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableToWebBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,[true,2],"a",[true,0],"b",true,[],[["error",true],["close"],["cancelled"]],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_body_functions_are_writable_sinks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_functions");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var values = [], receivedSignal = false, sink = stream.Duplex.from(async function(source, options) { receivedSignal = options.signal instanceof AbortSignal; for await (var value of source) values.push(String(value)); }), sinkReadable = sink.readable, sinkWritable = sink.writable, closed = new Promise(function(resolve, reject) { sink.on('error', reject); sink.once('close', resolve); }); sink.write('a'); sink.end('b'); await closed; var invalid = stream.Duplex.from(async function(source) { for await (var value of source) {} return 42; }), invalidCode = new Promise(function(resolve) { invalid.once('error', function(error) { resolve(error.code); }); }); invalid.end('x'); return [sinkReadable, sinkWritable, values, receivedSignal, await invalidCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_functions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexFromFunctions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexFromFunctions")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["a","b"],true,"ERR_INVALID_RETURN_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_waits_for_functional_sinks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_sink");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var values = [], order = []; async function* identity(source) { for await (var value of source) yield value; } async function sink(source) { for await (var value of source) { await Promise.resolve(); values.push(String(value)); } order.push('sink'); } var composed = stream.compose(identity, sink), readable = composed.readable, writable = composed.writable, closed = new Promise(function(resolve, reject) { composed.once('error', reject); composed.once('finish', function() { order.push('finish'); }); composed.once('close', resolve); }); composed.write('a'); composed.end('b'); await closed; var failure = new Error('sink failed'); async function reject(source) { for await (var value of source) throw failure; } var rejected = stream.compose(identity, reject), rejection = new Promise(function(resolve) { rejected.once('error', resolve); }); rejected.end('x'); var received = await rejection; return [readable, writable, values, order, composed.destroyed, received === failure, rejected.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_sink_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeSink = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeSink").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["a","b"],["sink","finish"],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_and_duplex_from_validate_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function code(action) { try { action(); } catch (error) { return error.code; } } module.exports = function () { function pass() { return new stream.PassThrough(); } function writable() { return new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); } function readable() { return new stream.Readable(); } var single = stream.compose(pass()), promised = stream.Duplex.from(Promise.resolve('x')), text = stream.Duplex.from('abc'); return [code(function() { stream.compose(); }), code(function() { stream.compose({}); }), code(function() { stream.compose(writable(), pass()); }), code(function() { stream.compose(pass(), readable()); }), code(function() { stream.Duplex.from(null); }), code(function() { stream.Duplex.from(42); }), code(function() { stream.Duplex.from({}); }), single.readable, single.writable, promised.readable, promised.writable, text.readable, text.writable]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_validation_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeValidation")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_MISSING_ARGS","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE",true,true,true,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_normalizes_scalar_chunks_and_null_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_chunks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); async function consume(source) { var duplex = stream.Duplex.from(source), values = []; try { for await (var value of duplex) values.push(Buffer.isBuffer(value) ? 'buffer:' + value.toString() : typeof value + ':' + String(value)); return [values, duplex.readableObjectMode, duplex.writableObjectMode]; } catch (error) { return error.code; } } module.exports = async function () { return [await consume('abc'), await consume(Buffer.from('abc')), await consume(new Uint8Array([97, 98, 99])), await consume(Promise.resolve('abc')), await consume(Promise.resolve(null)), await consume([null])]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_chunks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexFromChunks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexFromChunks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["string:abc"],true,true],[["buffer:abc"],true,true],[["number:97","number:98","number:99"],true,true],[["string:abc"],true,true],[[],true,true],"ERR_STREAM_NULL_VALUES"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_destroy_closes_the_source_iterator() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_iterator_cleanup");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var finalized = false, chunks = [], reason = new Error('stop'); async function* values() { try { yield 'first'; yield 'second'; } finally { finalized = true; } } var duplex = stream.Duplex.from(values()), closed = new Promise(function(resolve) { duplex.once('close', resolve); }); duplex.on('error', function() {}); duplex.on('data', function(chunk) { chunks.push(chunk); if (chunks.length === 1) duplex.destroy(reason); }); await closed; await Promise.resolve(); var returns = 0, count = 0, iterable = { [Symbol.asyncIterator]: function() { return { next: function() { count++; return Promise.resolve({ value: count, done: false }); }, return: function() { returns++; return Promise.resolve({ done: true }); } }; } }, repeated = stream.Duplex.from(iterable), repeatedClosed = new Promise(function(resolve) { repeated.once('close', resolve); }); repeated.on('error', function() {}); repeated.once('data', function() { repeated.destroy(); repeated.destroy(); }); await repeatedClosed; return [chunks, finalized, duplex.errored === reason, duplex.destroyed, returns]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_iterator_cleanup_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexIteratorCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexIteratorCleanup")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["first"],true,true,true,1]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_propagates_premature_stage_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_close");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); async function closeStage(position) { var first = new stream.PassThrough(), last = new stream.PassThrough(), composed = stream.compose(first, last), events = []; first.on('error', function(error) { events.push(['first', error.code]); }); last.on('error', function(error) { events.push(['last', error.code]); }); composed.on('error', function(error) { events.push(['composed', error.code]); }); var closed = new Promise(function(resolve) { composed.once('close', resolve); }); (position === 'first' ? first : last).destroy(); await closed; await Promise.resolve(); await Promise.resolve(); return [events, first.destroyed, last.destroyed, composed.destroyed, composed.errored.code]; } module.exports = async function () { return [await closeStage('first'), await closeStage('last')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_close_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeClose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeClose").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[["last","ERR_STREAM_PREMATURE_CLOSE"],["composed","ERR_STREAM_PREMATURE_CLOSE"]],true,true,true,"ERR_STREAM_PREMATURE_CLOSE"],[[["first","ERR_STREAM_PREMATURE_CLOSE"],["composed","ERR_STREAM_PREMATURE_CLOSE"]],true,true,true,"ERR_STREAM_PREMATURE_CLOSE"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_state_predicates_track_lifecycle() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_state");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var readable = new stream.Readable(), writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); var initial = [stream.isReadable(readable), stream.isWritable(writable), stream.isDestroyed(readable), stream.isDisturbed(readable), stream.isErrored(readable), stream.isReadable(null)]; readable.on('error', function() {}); readable.on('data', function() {}); readable.push('value'); var consumed = stream.isDisturbed(readable); writable.end(); var ended = stream.isWritable(writable); var failure = new Error('failed'); readable.destroy(failure); return [initial, consumed, ended, stream.isDestroyed(readable), stream.isErrored(readable), readable.errored === failure, stream.isReadable(readable)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_state_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamState").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[true,true,false,false,false,false],true,false,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_extended_state_unshift_wrap_and_default_encoding_work() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_extended_state");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var ordered = new stream.Readable(); ordered.pause(); ordered.push('b'); ordered.unshift('a'); var unshifted = ordered.read().toString(); var source = new stream.Readable(), wrapped = new stream.Readable().wrap(source), wrappedValues = []; wrapped.on('data', function(value) { wrappedValues.push(value.toString()); }); source.push('wrapped'); source.push(null); var completion, draining = new stream.Writable({ highWaterMark: 1, write: function(chunk, encoding, callback) { completion = callback; } }), writeResult = draining.write('x'), needed = draining.writableNeedDrain; completion(); var afterDrain = draining.writableNeedDrain, encoded = []; var encoder = new stream.Writable({ write: function(chunk, encoding, callback) { encoded.push([chunk.toString(), encoding]); callback(); } }); encoder.setDefaultEncoding('hex').end('6869'); var readableError = new Error('readable failure'), abortedReadable = new stream.Readable(); abortedReadable.on('error', function() {}); abortedReadable.destroy(readableError); var abortedWritable = new stream.Writable(); abortedWritable.destroy(); return [unshifted, wrappedValues, wrapped.readableFlowing, writeResult, needed, afterDrain, encoded, abortedReadable.closed, abortedReadable.readableAborted, abortedReadable.errored === readableError, abortedWritable.writableAborted, ordered.readableObjectMode, encoder.writableObjectMode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_extended_state_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamExtendedState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamExtendedState")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ab",["wrapped"],true,false,true,false,[["hi","buffer"]],true,true,true,true,false,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_indexed_pairs_static_checks_and_half_open_control_work() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_remaining_lifecycle");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var pairs = await stream.Readable.from(['a', 'b']).asIndexedPairs().toArray(); var readable = stream.Readable.from([1]), before = [stream.Readable.isDisturbed(readable), stream.Stream.isDestroyed(readable), stream.Stream.isReadable(readable), stream.Stream.isWritable(readable)]; await readable.toArray(); var duplex = new stream.Duplex({ allowHalfOpen: false }); duplex.push(null); var disposed = new stream.Writable(), symbolDisposed = false; if (Symbol.asyncDispose && disposed[Symbol.asyncDispose]) { await disposed[Symbol.asyncDispose](); symbolDisposed = disposed.destroyed; } var soon = new stream.Writable(); soon.destroySoon(); return [pairs, before, stream.Readable.isDisturbed(readable), duplex.allowHalfOpen, duplex.writableEnded, duplex.writableFinished, symbolDisposed, soon.writableEnded, soon.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_remaining_lifecycle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamRemainingLifecycle = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamRemainingLifecycle")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[0,"a"],[1,"b"]],[false,false,true,false],true,false,true,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_destroy_hooks_finish_error_and_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_destroy_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], replacement = new Error('replacement'), readable = new stream.Readable({ destroy: function(error, callback) { events.push(['hook', error.message]); queueMicrotask(function() { callback(replacement); callback(new Error('ignored')); }); } }); readable.on('error', function(error) { events.push(['error', error.message]); }); var closed = new Promise(function(resolve) { readable.on('close', function() { events.push(['close']); resolve(); }); }); readable.destroy(new Error('original')); var immediate = [readable.destroyed, readable.closed, readable.readableAborted]; await closed; var writableError, writable = new stream.Writable({ destroy: function(error, callback) { callback(null); } }); writable.on('error', function(error) { writableError = error; }); var writableClosed = new Promise(function(resolve) { writable.on('close', resolve); }); writable.destroy(new Error('writable-original')); var thrownError, throwing = new stream.Readable({ destroy: function() { throw new Error('hook-threw'); } }); throwing.on('error', function(error) { thrownError = error; }); var throwingClosed = new Promise(function(resolve) { throwing.on('close', resolve); }); throwing.destroy(); await Promise.all([writableClosed, throwingClosed]); return [immediate, events, readable.closed, readable.errored === replacement, writableError.message, writable.closed, thrownError.message, throwing.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_destroy_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamDestroyHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamDestroyHooks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[true,false,true],[["hook","original"],["error","replacement"],["close"]],true,true,"writable-original",true,"hook-threw",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_construct_hooks_gate_io_and_propagate_failures() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_construct_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var writableEvents = [], writable = new stream.Writable({ construct: function(callback) { writableEvents.push('construct'); queueMicrotask(function() { writableEvents.push('constructed'); callback(); }); }, write: function(chunk, encoding, callback) { writableEvents.push('write:' + chunk.toString()); callback(); }, final: function(callback) { writableEvents.push('final'); callback(); } }); writable.on('finish', function() { writableEvents.push('finish'); }); writable.end('value'); var beforeConstruct = writableEvents.slice(); await new Promise(function(resolve, reject) { writable.on('finish', resolve); writable.on('error', reject); }); var readableEvents = [], readable = new stream.Readable({ construct: function(callback) { readableEvents.push('construct'); queueMicrotask(callback); }, read: function() { readableEvents.push('read'); this.push('data'); this.push(null); } }), values = []; readable.on('data', function(chunk) { values.push(chunk.toString()); }); await new Promise(function(resolve, reject) { readable.on('end', resolve); readable.on('error', reject); }); var failure = new Error('construct-failed'), failedEvents = [], failed = new stream.Writable({ construct: function(callback) { queueMicrotask(function() { callback(failure); }); }, write: function(chunk, encoding, callback) { failedEvents.push('write'); callback(); }, destroy: function(error, callback) { failedEvents.push('destroy:' + error.message); callback(error); } }); failed.on('error', function(error) { failedEvents.push('error:' + error.message); }); var writeError, failedClosed = new Promise(function(resolve) { failed.on('close', resolve); }); failed.write('never', function(error) { writeError = error; }); await failedClosed; return [beforeConstruct, writableEvents, readableEvents, values, failedEvents, writeError === failure, failed.closed, failed.errored === failure]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_construct_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamConstructHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamConstructHooks")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[],["construct","constructed","write:value","final","finish"],["construct","read"],["data"],["destroy:construct-failed","error:construct-failed"],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}


