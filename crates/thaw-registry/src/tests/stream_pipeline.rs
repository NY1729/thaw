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


