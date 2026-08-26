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

#[test]
fn streams_expose_event_emitter_listener_management() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_event_emitter");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var value = new stream.PassThrough(), symbol = Symbol('event'), calls = []; function regular(input) { calls.push('regular:' + input); } function prepended(input) { calls.push('prepended:' + input); } function once(input) { calls.push('once:' + input); } value.on(symbol, regular); value.prependListener(symbol, prepended); value.prependOnceListener(symbol, once); var before = [value.listenerCount(symbol), value.listenerCount(symbol, regular), value.listeners(symbol)[1] === prepended, value.rawListeners(symbol).length, value.eventNames()[0] === symbol]; value.emit(symbol, 1); value.emit(symbol, 2); var afterOnce = value.listenerCount(symbol); value.removeListener(symbol, regular); var afterRemove = value.listenerCount(symbol); value.removeAllListeners(symbol); var afterAll = [value.listenerCount(symbol), value.eventNames().length]; var chained = value.setMaxListeners(25) === value, maximum = value.getMaxListeners(); return [before, calls, afterOnce, afterRemove, afterAll, chained, maximum, value.addListener === value.on]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_event_emitter_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamEvents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamEvents").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[3,1,true,3,true],["once:1","prepended:1","regular:1","prepended:2","regular:2"],2,1,[0,0],true,25,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_listener_meta_events_track_single_and_bulk_removal() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_listener_meta_events");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var value = new stream.Stream(), meta = [], calls = []; value.on('newListener', function(name, listener) { if (name !== 'newListener') meta.push('new:' + String(name) + ':' + listener.name); }); value.on('removeListener', function(name, listener) { meta.push('remove:' + String(name) + ':' + listener.name); }); function duplicate() { calls.push('duplicate'); } value.on('target', duplicate); value.on('target', duplicate); var before = value.listenerCount('target', duplicate); value.removeListener('target', duplicate); var afterOne = value.listenerCount('target', duplicate); value.emit('target'); function first() {} function second() {} value.on('bulk', first); value.on('bulk', second); value.removeAllListeners('bulk'); var bulkCount = value.listenerCount('bulk'), removeTail = meta.slice(-2); function oneTime() { calls.push('once'); } value.once('once', oneTime); var raw = value.rawListeners('once')[0], rawWrapper = raw !== oneTime && raw.listener === oneTime; raw(); return [before, afterOne, calls, bulkCount, removeTail, meta.indexOf('remove:once:oneTime') >= 0, rawWrapper, value.listenerCount('once')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_listener_meta_events_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamMetaEvents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamMetaEvents").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,1,["duplicate","once"],0,["remove:bulk:second","remove:bulk:first"],true,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_defer_destroy_errors_and_still_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_unhandled_errors");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var expected = new Error('expected'), direct, contextError; try { new stream.Stream().emit('error', expected); } catch (error) { direct = error === expected; } var context = { value: 1 }; try { new stream.Stream().emit('error', context); } catch (error) { contextError = [error.code, error.context === context]; } var handledValue, handled = new stream.Stream(); handled.on('error', function(error) { handledValue = error; }); var handledResult = handled.emit('error', expected); var events = [], destroyed = new stream.Readable(); destroyed.on('error', function(error) { events.push('error:' + error.message); }); destroyed.on('close', function() { events.push('close'); }); destroyed.destroy(expected); events.push('after'); var immediate = [destroyed.destroyed, destroyed.closed, destroyed.errored === expected]; var hooked = new stream.Readable({ destroy: function(error, callback) { callback(error); callback(new Error('ignored')); } }); hooked.on('error', function(error) { events.push('hook-error:' + error.message); }); hooked.on('close', function() { events.push('hook-close'); }); hooked.destroy(expected); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [direct, contextError, handledResult, handledValue === expected, immediate, hooked.closed, hooked.errored === expected, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_unhandled_errors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamUnhandledErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamUnhandledErrors")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,["ERR_UNHANDLED_ERROR",true],true,true,[true,true,true],true,true,["after","error:expected","close","hook-error:expected","hook-close"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_set_encoding_preserves_split_multibyte_characters() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_incremental_encoding");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var snow = Buffer.from('雪'), flowing = new stream.Readable().setEncoding('utf8'), events = []; flowing.on('data', function(value) { events.push(value); }); flowing.push(snow.subarray(0, 1)); flowing.push(snow.subarray(1, 2)); flowing.push(snow.subarray(2)); flowing.push(null); var buffered = new stream.Readable(); buffered.push(Buffer.from([0x41, snow[0]])); buffered.push(snow.subarray(1)); buffered.setEncoding('utf-8'); var bufferedLength = buffered.readableLength, first = buffered.read(1), second = buffered.read(); var face = Buffer.from('😀', 'utf16le'), wide = new stream.Readable().setEncoding('utf16le'), wideEvents = []; wide.on('data', function(value) { wideEvents.push(value); }); wide.push(face.subarray(0, 2)); wide.push(face.subarray(2)); wide.push(null); var incomplete = new stream.Readable().setEncoding('utf8'), incompleteValues = []; incomplete.on('data', function(value) { incompleteValues.push(value); }); incomplete.push(Buffer.from([0xe9])); incomplete.push(null); var invalid = false; try { new stream.Readable().setEncoding('missing'); } catch (error) { invalid = error instanceof TypeError; } return [events, bufferedLength, first, second, wideEvents, incompleteValues, invalid, flowing.readableEncoding]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_incremental_encoding_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamEncoding = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamEncoding").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["雪"],2,"A","雪",["😀"],["�"],true,"utf8"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_exposes_node_compatibility_helpers_and_duplex_pairs() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compatibility_helpers");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var bytes = new Uint8Array([1, 2]), shared = stream._uint8ArrayToBuffer(bytes); bytes[0] = 9; var views = [stream._isUint8Array(shared), stream._isUint8Array(new Uint16Array(1)), stream._isArrayBufferView(new DataView(new ArrayBuffer(2))), stream._isArrayBufferView(new ArrayBuffer(2))]; var pair = stream.duplexPair({ objectMode: true }), leftValues = [], rightValues = []; pair[0].on('data', function(value) { leftValues.push(value); }); pair[1].on('data', function(value) { rightValues.push(value); }); pair[0].write({ side: 'right' }); pair[1].write({ side: 'left' }); pair[0].end(); pair[1].end(); var destroyed = new stream.Readable(), destroyError; destroyed.on('error', function(error) { destroyError = [error.name, error.code]; }); var destroyResult = stream.destroy(destroyed); await Promise.resolve(); var promiseResult = await stream.promises.pipeline([1, 2], async function(source) { var total = 0; for await (var value of source) total += value; return total; }); return [[].slice.call(shared), views, leftValues, rightValues, pair[0].readableEnded, pair[1].readableEnded, destroyResult === undefined, destroyed.destroyed, destroyError, promiseResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compatibility_helpers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamHelpers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamHelpers").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[9,2],[true,false,true,false],[{"side":"left"}],[{"side":"right"}],true,true,true,true,["AbortError","ABORT_ERR"],3]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_pipe_reports_node_error_and_streams_can_be_undestroyed() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipe_error_undestroy");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), destination = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), observed; writable.on('error', function(error) { observed = [error.code, error.message]; }); var returned = writable.pipe(destination); var immediate = observed; await Promise.resolve(); await Promise.resolve(); var destroyed = writable.destroyed; writable._undestroy(); var restored = [writable.destroyed, writable.closed, writable.errored, writable.writable, writable.writableAborted]; var readable = new stream.Readable(); readable.destroy(); readable._undestroy(); return [returned === destination, immediate === undefined, observed, destroyed, restored, readable.destroyed, readable.closed, readable.readable, readable.readableAborted]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipe_error_undestroy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritablePipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritablePipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,["ERR_STREAM_CANNOT_PIPE","Cannot pipe, not readable"],true,[false,false,null,true,false],false,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_expose_buffer_state_and_readable_compose() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_buffer_state_compose");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable(); readable.push('a'); readable.push('b'); var before = [readable.readableDidRead, readable.readableBuffer.length, readable.readableBuffer[0].toString()]; var value = readable.read().toString(), after = [readable.readableDidRead, readable.readableBuffer.length]; var callbacks = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callbacks.push(callback); } }); writable.write('first'); writable.write('second'); var queued = [writable.writableBuffer.length, writable.writableBuffer[0].chunk.toString(), writable.writableBuffer[0].encoding]; callbacks.shift()(); callbacks.shift()(); var doubled = stream.Readable.from([1, 2]).compose(new stream.Transform({ objectMode: true, transform: function(item, encoding, callback) { callback(null, item * 2); } })); var composed = await doubled.toArray(); return [before, value, after, queued, composed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_buffer_state_compose_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamBufferState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamBufferState").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[false,2,"a"],"ab",[true,0],[1,"second","buffer"],[2,4]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn legacy_stream_pipe_forwards_manual_events_and_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_legacy_stream_pipe");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var source = new stream.Stream(), values = [], pipeSource, destination = new stream.Writable({ write: function(chunk, encoding, callback) { values.push(chunk.toString()); callback(); } }); destination.on('pipe', function(value) { pipeSource = value; }); var returned = source.pipe(destination); source.emit('data', Buffer.from('a')); source.emit('data', Buffer.from('b')); source.emit('end'); var openSource = new stream.Stream(), openEnded = false, openDestination = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); openDestination.on('finish', function() { openEnded = true; }); openSource.pipe(openDestination, { end: false }); openSource.emit('end'); return [returned === destination, pipeSource === source, values, destination.writableEnded, destination.writableFinished, openDestination.writableEnded, openEnded, source.listenerCount('data'), openSource.listenerCount('end')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_legacy_stream_pipe_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseLegacyStreamPipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseLegacyStreamPipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,["a","b"],true,false,false,false,0,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_subclasses_keep_prototype_lifecycle_hooks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_subclass_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { class Source extends stream.Readable { constructor() { super(); this.sent = false; } _read() { if (!this.sent) { this.sent = true; this.push('source'); this.push(null); } } } class Sink extends stream.Writable { constructor() { super(); this.values = []; this.finalized = false; this.cleaned = false; } _write(chunk, encoding, callback) { this.values.push(chunk.toString()); callback(); } _final(callback) { this.finalized = true; callback(); } _destroy(error, callback) { this.cleaned = true; callback(error); } } class Upper extends stream.Transform { _transform(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } _flush(callback) { callback(null, '!'); } } var source = new Source(), read = source.read().toString(), sink = new Sink(); sink.write('a'); sink.end('b'); sink.destroy(); var upper = new Upper(), transformed = []; upper.on('data', function(chunk) { transformed.push(chunk.toString()); }); upper.end('thaw'); var missing = new stream.Writable(), missingCode, callbackCode; missing.on('error', function(error) { missingCode = error.code; }); missing.write('x', function(error) { callbackCode = error.code; }); await Promise.resolve(); return [read, sink.values, sink.finalized, sink.cleaned, transformed, missingCode, callbackCode, missing.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_subclass_hooks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamSubclassHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamSubclassHooks")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["source",["a","b"],true,true,["THAW","!"],"ERR_METHOD_NOT_IMPLEMENTED","ERR_METHOD_NOT_IMPLEMENTED",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_subclass_construct_hooks_gate_io_and_fail_normally() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_subclass_construct");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readableEvents = [], releaseReadable; class Source extends stream.Readable { _construct(callback) { readableEvents.push('construct'); releaseReadable = callback; } _read() { readableEvents.push('read'); this.push('ready'); this.push(null); } } var source = new Source(), values = []; source.on('data', function(value) { values.push(value.toString()); }); await Promise.resolve(); var readableBefore = readableEvents.slice(); releaseReadable(); await Promise.resolve(); var writableEvents = [], releaseWritable; class Sink extends stream.Writable { _construct(callback) { writableEvents.push('construct'); releaseWritable = callback; } _write(chunk, encoding, callback) { writableEvents.push(chunk.toString()); callback(); } } var sink = new Sink(); sink.end('queued'); await Promise.resolve(); var writableBefore = writableEvents.slice(); releaseWritable(); await Promise.resolve(); class Broken extends stream.Readable { _construct(callback) { callback(new Error('construct-failed')); } } var broken = new Broken(), failure; broken.on('error', function(error) { failure = error.message; }); broken.resume(); await Promise.resolve(); await Promise.resolve(); return [readableBefore, readableEvents, values, writableBefore, writableEvents, broken.destroyed, broken.closed, failure]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_subclass_construct_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamSubclassConstruct = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamSubclassConstruct")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["construct"],["construct","read"],["ready"],["construct"],["construct","queued"],true,true,"construct-failed"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_auto_destroy_after_completion_and_honor_close_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_auto_destroy");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readableEvents = [], readable = new stream.Readable({ read: function() { this.push(null); } }); readable.on('end', function() { readableEvents.push('end'); }); readable.on('close', function() { readableEvents.push('close'); }); readable.resume(); var writableEvents = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); writable.on('finish', function() { writableEvents.push('finish'); }); writable.on('close', function() { writableEvents.push('close'); }); writable.end(); var persistent = new stream.Readable({ autoDestroy: false, read: function() { this.push(null); } }); persistent.resume(); var silentEvents = [], silent = new stream.Writable({ emitClose: false, write: function(chunk, encoding, callback) { callback(); } }); silent.on('finish', function() { silentEvents.push('finish'); }); silent.on('close', function() { silentEvents.push('close'); }); silent.end(); var duplex = new stream.Duplex({ read: function() {}, write: function(chunk, encoding, callback) { callback(); } }); duplex.end(); var afterOneSide = duplex.destroyed; duplex.push(null); await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); return [readableEvents, readable.destroyed, readable.closed, writableEvents, writable.destroyed, writable.closed, persistent.destroyed, persistent.closed, silentEvents, silent.destroyed, silent.closed, afterOneSide, duplex.destroyed, duplex.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_auto_destroy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamAutoDestroy = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamAutoDestroy").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["end","close"],true,true,["finish","close"],true,true,false,false,["finish"],true,true,false,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_decode_strings_and_chunk_validation_match_node() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_writable_decode_strings");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var release, preserved, writable = new stream.Writable({ decodeStrings: false, write: function(chunk, encoding, callback) { preserved = [typeof chunk, chunk, encoding]; release = callback; } }); writable.write('雪'); var preservedLength = writable.writableLength; release(); var converted, normal = new stream.Writable({ write: function(chunk, encoding, callback) { converted = [Buffer.isBuffer(chunk), chunk.toString(), encoding]; callback(); } }); normal.end('thaw'); var binary = [], binarySink = new stream.Writable({ write: function(chunk, encoding, callback) { binary.push([].slice.call(chunk)); callback(); } }); binarySink.write(new Uint16Array([258])); binarySink.end(new DataView(Uint8Array.from([3, 4]).buffer)); var nullCode, typeCode; try { new stream.Writable().write(null); } catch (error) { nullCode = error.code; } try { new stream.Writable().write({}); } catch (error) { typeCode = error.code; } var objects = [], objectSink = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { objects.push(value); callback(); } }); objectSink.end({ valid: true }); return [preserved, preservedLength, converted, binary, nullCode, typeCode, objects]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_writable_decode_strings_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritableDecoding = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritableDecoding").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["string","雪","utf8"],1,[true,"thaw","buffer"],[[2,1],[3,4]],"ERR_STREAM_NULL_VALUES","ERR_INVALID_ARG_TYPE",[{"valid":true}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_rejects_writes_after_end_or_destroy_asynchronously() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_writable_late_writes");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var endedEvents = [], ended = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); ended.on('error', function(error) { endedEvents.push(['error', error.code]); }); ended.end(); var endedReturn = ended.write('late', function(error) { endedEvents.push(['callback', error.code]); }); var endedImmediate = endedEvents.slice(); var destroyedEvents = [], writes = 0, destroyed = new stream.Writable({ write: function(chunk, encoding, callback) { writes++; callback(); } }); destroyed.on('error', function(error) { destroyedEvents.push(['error', error.code]); }); destroyed.destroy(); var destroyedReturn = destroyed.write('late', function(error) { destroyedEvents.push(['callback', error.code]); }); var destroyedImmediate = destroyedEvents.slice(); await Promise.resolve(); await Promise.resolve(); return [endedReturn, endedImmediate, endedEvents, ended.errored.code, destroyedReturn, destroyedImmediate, destroyedEvents, writes]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_writable_late_writes_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritableLateWrites = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritableLateWrites").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,[],[["callback","ERR_STREAM_WRITE_AFTER_END"],["error","ERR_STREAM_WRITE_AFTER_END"]],"ERR_STREAM_WRITE_AFTER_END",false,[],[["callback","ERR_STREAM_DESTROYED"]],0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_rejects_invalid_and_post_eof_chunks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_chunk_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var events = [], ended = new stream.Readable({ autoDestroy: false }); ended.on('error', function(error) { events.push(error.code); }); var eof = ended.push(null), late = ended.push('late'); var invalidEvents = [], invalid = new stream.Readable({ autoDestroy: false }); invalid.on('error', function(error) { invalidEvents.push(error.code); }); var invalidReturn = invalid.push({}); var binary = new stream.Readable({ autoDestroy: false }), first = binary.push(new Uint16Array([258])), second = binary.push(new DataView(Uint8Array.from([3, 4]).buffer)), bytes = binary.read(); var destroyed = new stream.Readable(); destroyed.destroy(); var destroyedReturn = destroyed.push('ignored'); var empty = new stream.Readable({ autoDestroy: false }), undefinedReturn = empty.push(undefined); return [eof, late, events, ended.errored.code, ended.readableAborted, ended.destroyed, invalidReturn, invalidEvents, invalid.errored.code, first, second, [].slice.call(bytes), destroyedReturn, destroyed.readableLength, undefinedReturn, empty.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_chunk_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableValidation").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,false,["ERR_STREAM_PUSH_AFTER_EOF"],"ERR_STREAM_PUSH_AFTER_EOF",true,false,false,["ERR_INVALID_ARG_TYPE"],"ERR_INVALID_ARG_TYPE",true,true,[2,1,3,4],false,0,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_read_waits_for_requested_size_and_preserves_modes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_sized_reads");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var buffered = new stream.Readable({ autoDestroy: false }); buffered.push('ab'); var zero = buffered.read(0), tooSoon = buffered.read(3), before = buffered.readableLength; buffered.push('c'); var exact = buffered.read(3).toString(); var ended = new stream.Readable({ autoDestroy: false }); ended.push('xy'); ended.push(null); var final = ended.read(3).toString(), endedLength = ended.readableLength; var encoded = new stream.Readable({ autoDestroy: false }).setEncoding('utf8'); encoded.push('A雪'); encoded.push(null); var first = encoded.read(1), encodedBefore = encoded.readableLength, rest = encoded.read(2); var objects = new stream.Readable({ autoDestroy: false, objectMode: true }), object = { value: 1 }; objects.push(object); var objectRead = objects.read(99) === object; var requested = [], generated = new stream.Readable({ autoDestroy: false, read: function(size) { requested.push(size); if (requested.length === 1) this.push('a'); else this.push('bc'); } }); var generatedFirst = generated.read(3), generatedSecond = generated.read(3).toString(); return [zero, tooSoon, before, exact, final, endedLength, first, encodedBefore, rest, objectRead, requested, generatedFirst, generatedSecond]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_sized_reads_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableSizes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableSizes").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[null,null,2,"abc","xy",0,"A",1,"雪",true,[3,3],null,"abc"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_unshift_validates_chunks_and_distinguishes_end_event() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_unshift_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var beforeEnd = new stream.Readable({ autoDestroy: false }); beforeEnd.push('b'); beforeEnd.push(null); var beforeReturn = beforeEnd.unshift('a'), beforeValue = beforeEnd.read().toString(); var afterEnd = new stream.Readable({ autoDestroy: false }), afterCode; afterEnd.on('error', function(error) { afterCode = error.code; }); var ended = new Promise(function(resolve) { afterEnd.on('end', resolve); }); afterEnd.push('x'); afterEnd.push(null); afterEnd.resume(); await ended; var afterReturn = afterEnd.unshift('z'); var binary = new stream.Readable({ autoDestroy: false }); var typed = binary.unshift(new Uint16Array([258])), view = binary.unshift(new DataView(Uint8Array.from([3, 4]).buffer)), bytes = binary.read(); var invalid = new stream.Readable({ autoDestroy: false }), invalidCode; invalid.on('error', function(error) { invalidCode = error.code; }); var invalidReturn = invalid.unshift({}), undefinedReturn = invalid.unshift(undefined); var destroyed = new stream.Readable(); destroyed.destroy(); return [beforeReturn, beforeValue, afterReturn, afterCode, afterEnd.readableLength, typed, view, [].slice.call(bytes), invalidReturn, invalidCode, undefinedReturn, invalid.readableLength, destroyed.unshift('ignored')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_unshift_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableUnshift = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableUnshift").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,"ab",false,"ERR_STREAM_UNSHIFT_AFTER_END_EVENT",0,true,true,[3,4,2,1],false,"ERR_INVALID_ARG_TYPE",true,0,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_unpipe_detaches_destinations_and_tracks_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_unpipe");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { function sink(values, unpipes) { var output = new stream.Writable({ write: function(chunk, encoding, callback) { values.push(chunk.toString()); callback(); } }); output.on('unpipe', function(source, info) { unpipes.push([source === readable, info.hasUnpiped]); }); return output; } var readable = new stream.Readable({ autoDestroy: false }), left = [], right = [], leftUnpipes = [], rightUnpipes = [], leftSink = sink(left, leftUnpipes), rightSink = sink(right, rightUnpipes); readable.pipe(leftSink); readable.pipe(rightSink); readable.push('x'); readable.unpipe(leftSink); readable.push('y'); var individualListeners = readable.listenerCount('data'); var allReturn = readable.unpipe(); readable.push('z'); var allListeners = readable.listenerCount('data'); var releases = [], slowValues = [], fastValues = [], pressured = new stream.Readable({ autoDestroy: false }), slow = new stream.Writable({ highWaterMark: 1, write: function(chunk, encoding, callback) { slowValues.push(chunk.toString()); releases.push(callback); } }), fast = new stream.Writable({ write: function(chunk, encoding, callback) { fastValues.push(chunk.toString()); callback(); } }); pressured.pipe(slow); pressured.pipe(fast); pressured.push('a'); pressured.push('b'); var paused = pressured.isPaused(), buffered = pressured.readableLength; releases.shift()(); await Promise.resolve(); var resumed = !pressured.isPaused(); releases.shift()(); return [left, right, leftUnpipes, rightUnpipes, individualListeners, allReturn === readable, allListeners, readable.readableLength, slowValues, fastValues, paused, buffered, resumed, pressured.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_unpipe_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableUnpipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableUnpipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["x"],["x","y"],[[true,false]],[[true,false]],1,true,0,1,["a","b"],["a","b"],true,1,false,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_pause_resume_transitions_are_idempotent() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_pause_resume");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable({ autoDestroy: false }), events = [], values = []; readable.on('pause', function() { events.push('pause'); }); readable.on('resume', function() { events.push('resume'); }); var initial = [readable.readableFlowing, readable.isPaused()]; var pauseOne = readable.pause() === readable, pauseTwo = readable.pause() === readable, paused = [readable.readableFlowing, readable.isPaused(), events.slice()]; var resumeOne = readable.resume() === readable, resumeTwo = readable.resume() === readable, immediate = [readable.readableFlowing, readable.isPaused(), events.slice()]; await Promise.resolve(); var resumedEvents = events.slice(); readable.on('data', function(value) { values.push(value.toString()); }); readable.pause(); readable.push('buffered'); var buffered = [values.slice(), readable.readableLength]; readable.resume(); var drained = [values.slice(), readable.readableLength]; await Promise.resolve(); return [initial, pauseOne, pauseTwo, paused, resumeOne, resumeTwo, immediate, resumedEvents, buffered, drained, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_pause_resume_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableFlowing = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableFlowing").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[null,false],true,true,[false,true,["pause"]],true,true,[true,false,["pause"]],["pause","resume"],[[],8],[["buffered"],0],["pause","resume","pause","resume"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_object_modes_preserve_values_and_directional_state() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_object_mode");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var objects = [], doubled = new stream.Transform({ objectMode: true, transform: function(value, encoding, callback) { callback(null, { value: value.value * 2 }); } }); doubled.on('data', function(value) { objects.push(value); }); var first = doubled.write({ value: 2 }); doubled.end({ value: 3 }); var encoded = [], encoder = new stream.Transform({ writableObjectMode: true, transform: function(value, encoding, callback) { callback(null, String(value.name)); } }); encoder.on('data', function(value) { encoded.push(value.toString()); }); encoder.end({ name: 'thaw' }); var decoded = [], decoder = new stream.Transform({ readableObjectMode: true, transform: function(value, encoding, callback) { callback(null, { text: value.toString() }); } }); decoder.on('data', function(value) { decoded.push(value); }); decoder.end('node'); var sinkValues = [], sink = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { sinkValues.push(value); callback(); } }); sink.end({ final: true }); return [first, doubled.readableHighWaterMark, doubled.writableHighWaterMark, objects, encoder._writableObjectMode, encoder._readableObjectMode, encoded, decoder._writableObjectMode, decoder._readableObjectMode, decoded, sinkValues]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_object_mode_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamObjectMode = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamObjectMode").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,16,16,[{"value":4},{"value":6}],true,false,["thaw"],false,true,[{"text":"node"}],[{"final":true}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_default_high_water_marks_apply_to_new_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_default_high_water_mark");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var before = [stream.getDefaultHighWaterMark(false), stream.getDefaultHighWaterMark(true)]; stream.setDefaultHighWaterMark(false, 7); stream.setDefaultHighWaterMark(true, 3); var readable = new stream.Readable(), writable = new stream.Writable({ objectMode: true }), zero = new stream.Duplex({ highWaterMark: 0 }); var invalid = false; try { stream.setDefaultHighWaterMark(false, -1); } catch (error) { invalid = error instanceof RangeError; } var values = [before, readable.readableHighWaterMark, writable.writableHighWaterMark, zero.readableHighWaterMark, zero.writableHighWaterMark, stream.getDefaultHighWaterMark(false), stream.getDefaultHighWaterMark(true), invalid]; stream.setDefaultHighWaterMark(false, before[0]); stream.setDefaultHighWaterMark(true, before[1]); return values; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_default_high_water_mark_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamHighWaterMark = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamHighWaterMark")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[16384,16],7,3,0,0,7,3,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_adapters_transfer_readable_writable_and_duplex_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_adapters");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var source = new stream.Readable({ objectMode: true }), webReadable = stream.Readable.toWeb(source), reader = webReadable.getReader(); source.push({ value: 1 }); source.push(null); var first = await reader.read(), last = await reader.read(); reader.releaseLock(); var webSource = new ReadableStream({ start: function(controller) { controller.enqueue({ value: 2 }); controller.close(); } }), nodeReadable = stream.Readable.fromWeb(webSource, { objectMode: true }), fromWeb = []; for await (var value of nodeReadable) fromWeb.push(value); var nodeWrites = [], nodeWritable = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { nodeWrites.push(value); callback(); } }), webWritable = stream.Writable.toWeb(nodeWritable), writer = webWritable.getWriter(); await writer.write({ value: 3 }); await writer.close(); writer.releaseLock(); var webWrites = [], nativeWebWritable = new WritableStream({ write: function(value) { webWrites.push(value); }, close: function() { webWrites.push('closed'); } }), fromWritable = stream.Writable.fromWeb(nativeWebWritable, { objectMode: true }); await new Promise(function(resolve, reject) { fromWritable.on('error', reject); fromWritable.end({ value: 4 }, resolve); }); var pass = new stream.PassThrough({ objectMode: true }), webPair = stream.Duplex.toWeb(pass), pairReader = webPair.readable.getReader(), pairWriter = webPair.writable.getWriter(); await pairWriter.write({ value: 5 }); var pairValue = await pairReader.read(); await pairWriter.close(); await pairReader.read(); pairWriter.releaseLock(); pairReader.releaseLock(); var pairWrites = [], fromPair = stream.Duplex.fromWeb({ readable: new ReadableStream({ start: function(controller) { controller.enqueue({ value: 6 }); controller.close(); } }), writable: new WritableStream({ write: function(value) { pairWrites.push(value); } }) }, { objectMode: true }), pairOutput = []; fromPair.on('data', function(value) { pairOutput.push(value); }); fromPair.write({ value: 7 }); fromPair.end(); await new Promise(function(resolve, reject) { fromPair.on('end', resolve); fromPair.on('error', reject); }); return [first.value, last.done, fromWeb, nodeWrites, webWrites, pairValue.value, pairOutput, pairWrites]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_adapters_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWebAdapters = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWebAdapters").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"value":1},true,[{"value":2}],[{"value":3}],[{"value":4},"closed"],{"value":5},[{"value":6}],[{"value":7}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_collection_helpers_transform_and_reduce_async_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_collection_helpers");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var transformed = await stream.Readable.from([1, 2, 3, 4]).map(async function(value) { await Promise.resolve(); return value * 2; }).filter(function(value) { return value > 2; }).flatMap(function(value) { return [value, value + 1]; }).drop(1).take(4).toArray(); var reduced = await stream.Readable.from([1, 2, 3]).reduce(async function(total, value) { return total + value; }, 10); var some = await stream.Readable.from([1, 2, 3]).some(function(value) { return value === 2; }), every = await stream.Readable.from([2, 4]).every(function(value) { return value % 2 === 0; }), found = await stream.Readable.from([1, 2, 3]).find(function(value) { return value > 1; }), visited = []; await stream.Readable.from(['a', 'b']).forEach(function(value, context) { visited.push([value, context.index]); }); var controller = new AbortController(); controller.abort('stop'); var aborted; try { await stream.Readable.from([1]).toArray({ signal: controller.signal }); } catch (error) { aborted = error; } var emptyError = false; try { await stream.Readable.from([]).reduce(function(a, b) { return a + b; }); } catch (error) { emptyError = error instanceof TypeError; } return [transformed, reduced, some, every, found, visited, aborted, emptyError, stream.Readable.from(['value'])._readableObjectMode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_collection_helpers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableHelpers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableHelpers").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[5,6,7,8],16,true,true,2,[["a",0],["b",1]],"stop",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_from_normalizes_scalar_iterables_and_awaits_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_readable_from");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var text = await stream.Readable.from('abc').toArray(); var bytes = await stream.Readable.from(Buffer.from([1, 2])).toArray(); var promised = await stream.Readable.from([Promise.resolve(3), Promise.resolve(4)]).toArray(); var nullCode; try { await stream.Readable.from([1, null]).toArray(); } catch (error) { nullCode = error.code; } var invalidCode; try { stream.Readable.from(42); } catch (error) { invalidCode = error.code; } return [text, bytes.length, Buffer.isBuffer(bytes[0]), Array.from(bytes[0]), promised, nullCode, invalidCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_readable_from_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableFrom = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableFrom").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["abc"],1,true,[1,2],[3,4],"ERR_STREAM_NULL_VALUES","ERR_INVALID_ARG_TYPE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_iterator_can_preserve_or_destroy_its_source() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_iterator_options");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var preserved = stream.Readable.from([1, 2, 3]), iterator = preserved.iterator({ destroyOnReturn: false }), first = await iterator.next(); await iterator.return(); var afterReturn = preserved.destroyed, rest = []; for await (var value of preserved) rest.push(value); var destroyed = stream.Readable.from([4, 5]), destructive = destroyed.iterator(); await destructive.next(); await destructive.return(); var disposable = stream.Readable.from([6]); if (Symbol.asyncDispose && disposable[Symbol.asyncDispose]) await disposable[Symbol.asyncDispose](); return [first, afterReturn, rest, destroyed.destroyed, disposable.destroyed, stream.isDisturbed(preserved)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_iterator_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableIterator = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableIterator").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"value":1,"done":false},false,[2,3],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_helpers_honor_bounded_concurrency_and_order() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_helper_concurrency");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var active = 0, maximum = 0, started = []; var mapped = await stream.Readable.from([1, 2, 3, 4]).map(async function(value, context) { active++; maximum = Math.max(maximum, active); started.push([value, context.index]); await new Promise(function(resolve) { setTimeout(resolve, (5 - value) * 2); }); active--; return value * 10; }, { concurrency: 2 }).toArray(); var filtered = await stream.Readable.from([1, 2, 3, 4]).filter(async function(value) { await Promise.resolve(); return value % 2 === 0; }, { concurrency: 3 }).toArray(); var visited = [], forEachActive = 0, forEachMaximum = 0; await stream.Readable.from([1, 2, 3]).forEach(async function(value) { forEachActive++; forEachMaximum = Math.max(forEachMaximum, forEachActive); await new Promise(function(resolve) { setTimeout(resolve, 1); }); visited.push(value); forEachActive--; }, { concurrency: 2 }); var invalid = false; try { stream.Readable.from([1]).map(function(value) { return value; }, { concurrency: 0 }); } catch (error) { invalid = error instanceof RangeError; } return [mapped, maximum, started, filtered, forEachMaximum, visited.sort(), invalid]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_helper_concurrency_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableConcurrency = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableConcurrency")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[10,20,30,40],2,[[1,0],[2,1],[3,2],[4,3]],[2,4],2,[1,2,3],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_propagate_abort_and_original_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_abort");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); function delayed() { return new stream.Transform({ transform: function(chunk, encoding, callback) { setTimeout(function() { callback(null, chunk); }, 2); } }); } module.exports = async function () { var source = new stream.Readable(), transform = delayed(), sink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), controller = new AbortController(), pending = promises.pipeline(source, transform, sink, { signal: controller.signal }); source.push('value'); controller.abort('stop'); var aborted; try { await pending; } catch (error) { aborted = [error.name, error.cause]; } var callbackSource = new stream.Readable(), callbackTransform = delayed(), callbackSink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), callbackController = new AbortController(), callbackResult = new Promise(function(resolve) { stream.pipeline(callbackSource, callbackTransform, callbackSink, { signal: callbackController.signal }, function(error) { resolve([error.name, error.cause]); }); }); callbackSource.push('value'); callbackController.abort('callback-stop'); var waiting = new stream.Readable(), finishedController = new AbortController(), finished = promises.finished(waiting, { signal: finishedController.signal }); finishedController.abort('finished-stop'); var finishedError; try { await finished; } catch (error) { finishedError = [error.name, error.cause, waiting.destroyed]; } var errorSource = new stream.Readable(), errorTransform = new stream.Transform({ transform: function(chunk, encoding, callback) { setTimeout(function() { callback(new Error('transform-failed')); }, 1); } }), errorSink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), failed = promises.pipeline(errorSource, errorTransform, errorSink); errorSource.push('value'); var original; try { await failed; } catch (error) { original = error.message; } return [aborted, source.destroyed, transform.destroyed, sink.destroyed, await callbackResult, callbackSource.destroyed, callbackTransform.destroyed, callbackSink.destroyed, finishedError, original, errorSource.destroyed, errorTransform.destroyed, errorSink.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_abort_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamPipelineAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamPipelineAbort")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["AbortError","stop"],true,true,true,["AbortError","callback-stop"],true,true,true,["AbortError","finished-stop",false],"transform-failed",true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_reject_premature_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_premature_close");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var source = new stream.Readable(), sink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), callbackCount = 0; var callbackResult = new Promise(function(resolve) { stream.pipeline(source, sink, function(error) { callbackCount++; queueMicrotask(function() { resolve([error.code, error.message, callbackCount]); }); }); }); source.destroy(); var first = await callbackResult; var sourceTwo = new stream.Readable(), sinkTwo = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), pending = promises.pipeline(sourceTwo, sinkTwo); sinkTwo.destroy(); var second; try { await pending; } catch (error) { second = [error.code, error.message]; } return [first, source.destroyed, sink.destroyed, second, sourceTwo.destroyed, sinkTwo.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_premature_close_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePipelinePrematureClose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exercisePipelinePrematureClose")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["ERR_STREAM_PREMATURE_CLOSE","Premature close",1],true,true,["ERR_STREAM_PREMATURE_CLOSE","Premature close"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_completion_helpers_validate_required_arguments() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_completion_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { function code(action) { try { action(); } catch (error) { return error.code; } } var source = function() { return new stream.Readable(); }, sink = function() { return new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); }; return [code(function() { stream.pipeline(source(), sink()); }), code(function() { stream.pipeline(source(), function() {}); }), code(function() { stream.pipeline([], function() {}); }), code(function() { stream.finished(source()); }), code(function() { stream.finished({}, function() {}); }), typeof stream.pipeline([source(), sink()], function() {})]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_completion_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompletionValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseCompletionValidation")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_ARG_TYPE","ERR_MISSING_ARGS","ERR_MISSING_ARGS","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE","object"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn finished_defers_callbacks_and_honors_cleanup_and_error_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_finished_options");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var order = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); stream.finished(writable, { cleanup: true }, function() { order.push('finished'); }); writable.on('finish', function() { order.push('finish'); }); writable.end(); order.push('after'); await Promise.resolve(); await Promise.resolve(); var observed = [], ignored = new stream.Readable(); ignored.on('error', function(error) { observed.push('own:' + error.message); }); var ignoredDone = new Promise(function(resolve) { stream.finished(ignored, { error: false, cleanup: true }, function(error) { observed.push(error ? error.code : 'ok'); resolve(); }); }); ignored.destroy(new Error('ignored')); await ignoredDone; var controller = new AbortController(); controller.abort('stop'); var abortedOrder = [], waiting = new stream.Readable(); var abortedDone = new Promise(function(resolve) { stream.finished(waiting, { signal: controller.signal, cleanup: true }, function(error) { abortedOrder.push([error.name, error.code, error.cause]); resolve(); }); }); abortedOrder.push('after'); await abortedDone; var cleaned = new stream.Readable(), before, after; var cleanedDone = new Promise(function(resolve) { stream.finished(cleaned, { cleanup: true }, function() { after = ['end', 'finish', 'error', 'close'].map(function(name) { return cleaned.listenerCount(name); }); resolve(); }); }); before = ['end', 'finish', 'error', 'close'].map(function(name) { return cleaned.listenerCount(name); }); cleaned.push(null); cleaned.resume(); await cleanedDone; return [order, observed, abortedOrder, before, after]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_finished_options_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinishedOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinishedOptions").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["after","finish","finished"],["own:ignored","ok"],["after",["AbortError","ABORT_ERR","stop"]],[1,1,1,1],[0,0,0,0]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn promise_pipeline_can_leave_destination_open() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_end_false");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var values = [], destination = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { values.push(value); callback(); } }); var result = await promises.pipeline(stream.Readable.from([1, 2]), destination, { end: false }); var open = [result, values, destination.writableEnded, destination.writableFinished, destination.destroyed]; destination.write(3); await new Promise(function(resolve, reject) { destination.on('error', reject); destination.end(resolve); }); return [open, values, destination.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_end_false_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePipelineEndFalse = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exercisePipelineEndFalse").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[null,[1,2,3],false,false,false],[1,2,3],true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_accept_iterables_and_function_stages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_function_pipeline");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var callbackValue = await new Promise(function(resolve, reject) { stream.pipeline([1, 2, 3], async function*(source) { for await (var value of source) yield value * 2; }, async function(source) { var total = 0; for await (var value of source) total += value; return total; }, function(error, value) { if (error) reject(error); else resolve(value); }); }); var promiseValue = await promises.pipeline(async function* source(options) { yield options.signal ? 'signal' : 'missing'; yield 'source'; }, async function*(source) { for await (var value of source) yield value.toUpperCase(); }, async function(source) { var values = []; for await (var value of source) values.push(value); return values.join(':'); }); var transformed = new stream.Transform({ objectMode: true, transform: function(value, encoding, callback) { callback(null, value + 1); } }); var mixedValue = await promises.pipeline([4, 5], transformed, async function(source) { return (await source.toArray()).join(','); }); var controller = new AbortController(), aborted = promises.pipeline(async function*() { await new Promise(function(resolve) { setTimeout(resolve, 5); }); yield 1; }, async function(source) { for await (var value of source) return value; }, { signal: controller.signal }); controller.abort('pipeline-stop'); var abortResult; try { await aborted; } catch (error) { abortResult = [error.name, error.cause]; } return [callbackValue, promiseValue, mixedValue, abortResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_function_pipeline_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFunctionPipeline = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFunctionPipeline").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[12,"SIGNAL:SOURCE","5,6",["AbortError","pipeline-stop"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn functional_pipeline_errors_abort_sources_and_destroy_stages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_function_pipeline_cleanup");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var signalAborted = false, sourceClosed = false, middle = new stream.PassThrough({ objectMode: true }), expected = new Error('destination-failed'), observed; try { await promises.pipeline(async function* source(options) { options.signal.addEventListener('abort', function() { signalAborted = true; }, { once: true }); try { yield 1; await new Promise(function(resolve) { options.signal.addEventListener('abort', resolve, { once: true }); }); yield 2; } finally { sourceClosed = true; } }, middle, async function(destination) { for await (var value of destination) throw expected; }); } catch (error) { observed = error; } await Promise.resolve(); await Promise.resolve(); var callbackMiddle = new stream.PassThrough({ objectMode: true }), callbackError = await new Promise(function(resolve) { stream.pipeline([1], callbackMiddle, async function() { throw new Error('callback-failed'); }, function(error) { resolve(error); }); }); return [observed === expected, observed.message, signalAborted, sourceClosed, middle.destroyed, middle.errored === expected, callbackError.message, callbackMiddle.destroyed, callbackMiddle.errored === callbackError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_function_pipeline_cleanup_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFunctionPipelineCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFunctionPipelineCleanup")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"destination-failed",true,true,true,true,"callback-failed",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn finished_waits_for_both_duplex_sides_and_reports_early_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_finished_duplex");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var duplex = new stream.Duplex(), resolved = false, pending = promises.finished(duplex).then(function() { resolved = true; }); duplex.end(); await Promise.resolve(); var afterFinish = resolved; duplex.push(null); await pending; var early = new stream.Readable(), earlyPending = promises.finished(early); early.destroy(); var earlyCode; try { await earlyPending; } catch (error) { earlyCode = error.code; } var writableOnly = new stream.Duplex(), writablePending = promises.finished(writableOnly, { readable: false }); writableOnly.end(); await writablePending; var callbackDuplex = new stream.Duplex(), callbackCount = 0, cleanup = stream.finished(callbackDuplex, { cleanup: true }, function() { callbackCount++; }); cleanup(); callbackDuplex.end(); callbackDuplex.push(null); await Promise.resolve(); return [afterFinish, resolved, earlyCode, writableOnly.writableFinished, writableOnly.readableEnded, callbackCount]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_finished_duplex_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinishedDuplex = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinishedDuplex").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,"ERR_STREAM_PREMATURE_CLOSE",true,false,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_writables_serialize_async_work_and_signal_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], sink = new stream.Writable({ highWaterMark: 3, write: function(chunk, encoding, callback) { var value = chunk.toString(); events.push('start:' + value); setTimeout(function() { events.push('end:' + value); callback(); }, 1); } }); sink.on('drain', function() { events.push('drain:' + sink.writableNeedDrain); }); sink.on('finish', function() { events.push('finish'); }); var first = sink.write('a', function() { events.push('callback:a'); }), second = sink.write('bb', function() { events.push('callback:bb'); }), completion = new Promise(function(resolve, reject) { sink.on('error', reject); sink.on('finish', resolve); }); sink.end('c', function() { events.push('end-callback'); }); await completion; var transformed = [], transform = new stream.Transform({ highWaterMark: 2, transform: function(chunk, encoding, callback) { setTimeout(function() { callback(null, chunk.toString().toUpperCase()); }, 1); } }); transform.on('data', function(chunk) { transformed.push(chunk.toString()); }); var transformFirst = transform.write('x'), transformSecond = transform.write('y'), transformCompletion = new Promise(function(resolve, reject) { transform.on('error', reject); transform.on('finish', resolve); }); transform.end('z'); await transformCompletion; return [first, second, sink.writableLength, sink.writableFinished, events, transformFirst, transformSecond, transformed.join(''), transform.readableEnded]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamBackpressure").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,0,true,["start:a","end:a","start:bb","callback:a","end:bb","drain:false","start:c","callback:bb","end:c","end-callback","finish"],true,false,"XYZ",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_end_callbacks_settle_once_before_finish() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_repeated_end");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write:' + chunk.toString()); callback(); }, final: function(callback) { events.push('final'); callback(); } }); writable.on('finish', function() { events.push('finish'); }); writable.end('x', function(error) { events.push('first:' + (error && error.code || 'ok')); }); events.push('after-first'); writable.end(function(error) { events.push('second:' + (error && error.code || 'ok')); }); events.push('after-second'); var immediate = writable.writableFinished; await new Promise(function(resolve) { queueMicrotask(resolve); }); writable.end(function(error) { events.push('late:' + (error && error.code || 'ok')); }); events.push('after-late'); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [immediate, writable.writableFinished, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_repeated_end_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRepeatedEnd = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRepeatedEnd").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["write:x","final","after-first","after-second","first:ok","second:ok","finish","after-late","late:ok"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn synchronous_writable_callbacks_are_deferred_and_guarded() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_sync_write_callbacks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write'); callback(); } }); var result = writable.write('x', function(error) { events.push('callback:' + (error && error.code || 'ok')); }); events.push('after:' + result + ':' + writable.writableLength); var before = events.slice(); await Promise.resolve(); var vectorEvents = [], vector = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); }, writev: function(chunks, callback) { vectorEvents.push('writev:' + chunks.length); callback(); } }); vector.cork(); vector.write('a', function() { vectorEvents.push('a'); }); vector.write('b', function() { vectorEvents.push('b'); }); vector.uncork(); vectorEvents.push('after'); var vectorBefore = vectorEvents.slice(); await Promise.resolve(); var repeatedEvents = [], repeated = new stream.Writable({ write: function(chunk, encoding, callback) { repeatedEvents.push('write'); callback(); callback(); } }); repeated.on('error', function(error) { repeatedEvents.push('error:' + error.code); }); repeated.on('close', function() { repeatedEvents.push('close'); }); var repeatedResult = repeated.write('z', function(error) { repeatedEvents.push('callback:' + (error && error.code || 'ok')); }); repeatedEvents.push('after:' + repeatedResult + ':' + repeated.destroyed); await Promise.resolve(); return [before, events, vectorBefore, vectorEvents, repeatedEvents]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_sync_write_callbacks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseSyncWriteCallbacks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseSyncWriteCallbacks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["write","after:true:0"],["write","after:true:0","callback:ok"],["writev:2","after"],["writev:2","after","a","b"],["write","after:false:true","callback:ok","error:ERR_MULTIPLE_CALLBACK","close"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_final_exceptions_and_repeated_callbacks_destroy_stream() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_final_failures");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function exercise(finalizer) { return new Promise(function(resolve) { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); }, final: finalizer }); writable.on('error', function(error) { events.push('error:' + error.code + ':' + error.message); }); writable.on('close', function() { events.push('close'); queueMicrotask(function() { resolve([events, writable.destroyed, writable.closed, writable.writableFinished]); }); }); writable.end('x', function(error) { events.push('end:' + (error && (error.code || error.message))); }); events.push('after:' + writable.destroyed); }); } module.exports = async function () { var thrown = await exercise(function() { throw new Error('final-threw'); }); var repeated = await exercise(function(callback) { callback(); callback(); }); return [thrown, repeated]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_final_failures_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinalFailures = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinalFailures").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["after:true","error:undefined:final-threw","close","end:final-threw"],true,true,false],[["after:true","error:ERR_MULTIPLE_CALLBACK:Callback called multiple times","close","end:ERR_MULTIPLE_CALLBACK"],true,true,false]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_streams_cork_and_batch_writev_chunks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_writev");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var batches = [], singles = [], callbacks = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { singles.push(chunk.toString()); callback(); }, writev: function(chunks, callback) { batches.push(chunks.map(function(entry) { return entry.chunk.toString(); })); setTimeout(callback, 1); } }); writable.cork(); writable.cork(); writable.write('a', function() { callbacks.push('a'); }); writable.write('b', function() { callbacks.push('b'); }); var nested = [writable.writableCorked, batches.length, writable.writableLength]; writable.uncork(); var stillCorked = [writable.writableCorked, batches.length]; writable.uncork(); writable.cork(); writable.write('c', function() { callbacks.push('c'); }); var completion = new Promise(function(resolve, reject) { writable.on('error', reject); writable.on('finish', resolve); }); writable.end('d', function() { callbacks.push('end'); }); await completion; return [nested, stillCorked, writable.writableCorked, batches, singles, callbacks, writable.writableLength, writable.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_writev_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWritev = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWritev").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[2,0,2],[1,0],0,[["a","b"],["c","d"]],[],["a","b","c","end"],0,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_destroy_rejects_pending_work_exactly_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_destroy_pending");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], release, writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write:' + chunk.toString()); release = callback; } }); writable.on('error', function(error) { events.push('error:' + error.message); }); writable.on('close', function() { events.push('close'); }); writable.on('finish', function() { events.push('finish'); }); writable.write('a', function(error) { events.push('callback:a:' + (error ? error.message : 'ok')); }); writable.write('b', function(error) { events.push('callback:b:' + error.message); }); writable.end('c', function(error) { events.push('end:' + error.message); }); writable.destroy(new Error('boom')); writable.destroy(new Error('again')); release(); await new Promise(function(resolve) { queueMicrotask(resolve); }); var silentEvents = [], silentRelease, silent = new stream.Writable({ write: function(chunk, encoding, callback) { silentRelease = callback; } }); silent.write('x', function(error) { silentEvents.push(error ? error.code : 'ok'); }); silent.on('error', function() { silentEvents.push('unexpected-error'); }); silent.destroy(); silentRelease(); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [events, writable.writableLength, writable.destroyed, writable.writableFinished, silentEvents, silent.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_destroy_pending_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamDestroyPending = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamDestroyPending")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["write:a","callback:a:ok","callback:b:boom","end:boom","error:boom","close"],0,true,false,["ok"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_streams_track_buffers_and_pause_for_pipe_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable({ highWaterMark: 3 }), first = readable.push('ab'), second = readable.push('cd'), before = readable.readableLength, partial = readable.read(3).toString(), after = readable.readableLength, rest = readable.read().toString(), objects = new stream.Readable({ objectMode: true, highWaterMark: 2 }), object = { value: 1 }, objectFirst = objects.push(object), objectSecond = objects.push({ value: 2 }), sameObject = objects.read() === object; var values = [], source = new stream.Readable({ highWaterMark: 2 }), sink = new stream.Writable({ highWaterMark: 2, write: function(chunk, encoding, callback) { values.push(chunk.toString()); setTimeout(callback, 1); } }), finished = new Promise(function(resolve, reject) { sink.on('error', reject); sink.on('finish', resolve); }); source.pipe(sink); source.push('a'); source.push('b'); var paused = source.isPaused(); source.push('c'); var queued = source.readableLength; source.push(null); await finished; return [first, second, before, partial, after, rest, objects.readableLength, objectFirst, objectSecond, sameObject, paused, queued, source.isPaused(), source.readableLength, values.join(''), sink.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,4,"abc",1,"d",1,true,false,true,true,1,false,0,"abc",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_async_iterators_handle_buffering_waits_and_cleanup() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_async_iterator");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var buffered = new stream.Readable(); buffered.push('a'); buffered.push('b'); buffered.push(null); var values = []; for await (var value of buffered) values.push(value.toString()); var future = new stream.Readable({ objectMode: true }), iterator = future[Symbol.asyncIterator](), pending = iterator.next(), object = { answer: 42 }; setTimeout(function() { future.push(object); }, 1); var arrived = await pending; future.push(null); var ended = await iterator.next(); var broken = new stream.Readable(), brokenIterator = broken[Symbol.asyncIterator](), rejected = brokenIterator.next(); setTimeout(function() { broken.destroy(new Error('broken')); }, 1); var message; try { await rejected; } catch (error) { message = error.message; } var early = new stream.Readable(); early.push('first'); early.push('second'); var earlyValues = []; for await (var item of early) { earlyValues.push(item.toString()); break; } return [values, arrived.value === object, arrived.done, ended.done, message, earlyValues, early.destroyed, early.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_async_iterator_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableAsyncIterator = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableAsyncIterator")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","b"],true,false,true,"broken",["first"],true,6]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn diagnostics_channels_publish_bind_stores_and_trace() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_diagnostics_channel");
    fs::write(
            dir.join("index.js"),
            "var diagnostics = require('node:diagnostics_channel');\n\
             module.exports = async function () {\n\
             \x20 var events = []; var work = diagnostics.channel('work'); var same = work === diagnostics.channel('work');\n\
             \x20 function subscriber(data, name) { events.push(name + ':' + data.value); } diagnostics.subscribe('work', subscriber); diagnostics.subscribe('work', subscriber);\n\
             \x20 var store = { run: function(value, callback) { events.push('store:' + value); return callback(); } }; work.bindStore(store, function(data) { return data.value * 2; });\n\
             \x20 var storeResult = work.runStores({ value: 2 }, function(left, right) { return left + right; }, null, 3, 4); work.publish({ value: 5 }); var removed = diagnostics.unsubscribe('work', subscriber); work.publish({ value: 6 }); work.unbindStore(store);\n\
             \x20 var trace = diagnostics.tracingChannel('operation'); ['start', 'end', 'asyncStart', 'asyncEnd', 'error'].forEach(function(name) { trace[name].subscribe(function(context) { events.push(name + ':' + (context.result || context.error && context.error.message || '')); }); });\n\
             \x20 var sync = trace.traceSync(function(value) { return value + 1; }, {}, null, 4); var promised = await trace.tracePromise(async function(value) { return value * 2; }, {}, null, 3); var failed; try { trace.traceSync(function() { throw new Error('bad'); }, {}); } catch (error) { failed = error.message; }\n\
             \x20 return [same, storeResult, removed, diagnostics.hasSubscribers('work'), sync, promised, failed, events];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_diagnostics_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseDiagnostics = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseDiagnostics").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,7,true,false,5,6,"bad",["store:4","store:10","work:5","store:12","start:","end:5","start:","asyncStart:6","asyncEnd:6","end:6","start:","error:bad","end:bad"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn async_hooks_preserve_storage_and_resource_scope() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_async_hooks");
    fs::write(
            dir.join("index.js"),
            "var hooks = require('node:async_hooks');\n\
             module.exports = async function () {\n\
             \x20 var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default', name: 'request' }); var events = [storage.getStore()]; var bound; var snapshot;\n\
             \x20 var result = await storage.run('outer', async function(value) { events.push(storage.getStore() + ':' + value); bound = hooks.AsyncLocalStorage.bind(function(suffix) { return storage.getStore() + suffix; }); snapshot = hooks.AsyncLocalStorage.snapshot(); await Promise.resolve(); events.push(storage.getStore()); var nested = storage.run('inner', function() { return storage.getStore(); }); events.push(nested + ':' + storage.getStore()); var exited = storage.exit(function() { return storage.getStore(); }); events.push(String(exited) + ':' + storage.getStore()); return 'done'; }, 4);\n\
             \x20 storage.enterWith('changed'); var rebound = bound('!'); var snapped = snapshot(function() { return storage.getStore(); });\n\
             \x20 var resource = new hooks.AsyncResource('work'); var outside = hooks.executionAsyncId(); var inside = resource.runInAsyncScope(function(left, right) { return [hooks.executionAsyncId(), hooks.triggerAsyncId(), hooks.executionAsyncResource() === resource, left + right]; }, null, 2, 3); var reboundResource = resource.bind(function() { return hooks.executionAsyncId(); })(); resource.emitDestroy();\n\
             \x20 storage.disable(); return [events, result, rebound, snapped, storage.getStore() === undefined, outside, inside, reboundResource, resource.asyncId(), resource.triggerAsyncId(), resource._destroyed, hooks.createHook({}).enable().disable().callbacks];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_async_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAsyncHooks = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseAsyncHooks").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["default","outer:4","outer","inner:outer","undefined:outer"],"done","outer!","outer",true,1,[2,1,true,5],2,2,1,true,{}]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn tty_builtin_reports_capabilities_and_emits_ansi_sequences() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_tty");
    fs::write(
            dir.join("index.js"),
            "var tty = require('node:tty');\n\
             module.exports = async function () {\n\
             \x20 var input = new tty.ReadStream(0); var same = input.setRawMode(true) === input; input.setRawMode(false);\n\
             \x20 var output = new tty.WriteStream(1); var callbacks = []; output.write('text'); output.cursorTo(2, 3, function() { callbacks.push('cursor'); }); output.moveCursor(-1, 2, function() { callbacks.push('move'); }); output.clearLine(0); output.clearScreenDown(); await Promise.resolve();\n\
             \x20 return [tty.isatty(1), input.isTTY, input.isRaw, same, output.getWindowSize(), output.getColorDepth({ FORCE_COLOR: '3' }), output.getColorDepth({ TERM: 'xterm-256color' }), output.hasColors(256, { TERM: 'xterm-256color' }), output.hasColors(16, {}), output._output, callbacks.sort().join(',')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_tty_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTty = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseTty").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
            result,
            "[false,true,false,true,[80,24],24,8,true,false,\"text\\u001b[4;3H\\u001b[1D\\u001b[2B\\u001b[2K\\u001b[0J\",\"cursor,move\"]"
        );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn module_create_require_loads_relative_and_builtin_dependencies() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_module_create_require");
    fs::write(
            dir.join("index.js"),
            "import { createRequire, isBuiltin, builtinModules, registerHooks } from 'node:module';\n\
             const localRequire = createRequire('pkg/index.js');\n\
             export default function () {\n\
             \x20 const dependency = localRequire('./dependency'); const path = localRequire('node:path'); const hooks = registerHooks({}); hooks.deregister();\n\
             \x20 return [dependency.value, path.basename('/tmp/file.txt'), localRequire.resolve('./dependency'), Object.keys(localRequire.cache).length >= 3, isBuiltin('node:path'), isBuiltin('missing'), builtinModules.includes('stream'), hooks.active];\n\
             }",
        )
        .unwrap();
    fs::write(dir.join("dependency.js"), "module.exports = { value: 42 };").unwrap();
    let empty_node_modules = temp_registry("builtin_module_create_require_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseModule = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseModule").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[42,"file.txt","pkg/dependency.js",true,true,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

