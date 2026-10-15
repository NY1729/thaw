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
fn stream_core_contracts_u1_lifecycle_matrix() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_core_contracts_u1_matrix");
    fs::write(
        dir.join("index.js"),
        r#"
var stream = require('node:stream');
module.exports = async function () {
  function check(condition, label) { if (!condition) throw new Error('U1 assertion: ' + label); }
  function record(row, expected) { check(JSON.stringify(row) === JSON.stringify(expected), JSON.stringify({ row: row, expected: expected })); observed.push(row); }
  function within(promise, label) {
    var timer;
    return Promise.race([promise, new Promise(function(_, reject) { timer = setTimeout(function() { reject(new Error('watchdog:' + label)); }, 3000); })]).finally(function() { clearTimeout(timer); });
  }
  function event(target, name) { return new Promise(function(resolve) { target.once(name, resolve); }); }
  function failure(target, options) { return within(new Promise(function(resolve) { stream.finished(target, options || {}, resolve); }), 'finished'); }
  function signal(initiallyAborted, reason) {
    var listeners = [], value = { aborted: initiallyAborted, reason: initiallyAborted ? reason : undefined, added: 0, removed: 0 };
    value.addEventListener = function(type, fn, options) { if (type !== 'abort') return; value.added++; listeners.push({ fn: fn, once: Boolean(options && options.once) }); };
    value.removeEventListener = function(type, fn) { if (type !== 'abort') return; value.removed++; listeners = listeners.filter(function(entry) { return entry.fn !== fn; }); };
    value.abort = function(nextReason) { value.aborted = true; value.reason = nextReason; listeners.slice().forEach(function(entry) { entry.fn(); }); listeners = listeners.filter(function(entry) { return !entry.once; }); };
    value.listenerCount = function() { return listeners.length; };
    return value;
  }
  function make(kind, options) {
    if (kind === 'R') return new stream.Readable(Object.assign({ read: function() {} }, options));
    if (kind === 'W') return new stream.Writable(Object.assign({ write: function(chunk, encoding, callback) { callback(); } }, options));
    if (kind === 'D') return new stream.Duplex(Object.assign({ read: function() {}, write: function(chunk, encoding, callback) { callback(); } }, options));
    return new stream.Transform(Object.assign({ transform: function(chunk, encoding, callback) { callback(null, chunk); } }, options));
  }
  var observed = [];

  // Disabled directions complete without fabricated end/finish events; both disabled still auto-destroys once.
  var noRead = 0, noWrite = 0, noFinal = 0, noEnd = 0, noFinish = 0, noCleanup = 0;
  var rd = new stream.Duplex({ readable: false, read: function() { noRead++; }, write: function(c, e, cb) { noWrite++; cb(); }, final: function(cb) { noFinal++; cb(); }, destroy: function(e, cb) { noCleanup++; cb(e); } });
  rd.on('end', function() { noEnd++; }); rd.on('finish', function() { noFinish++; }); var rdClose = event(rd, 'close'); rd.end('x'); await within(failure(rd, { readable: false }), 'duplex-readable-disabled'); await within(rdClose, 'duplex-readable-disabled-close');
  record(['rfalse', rd.readable, rd.readableEnded, rd.writableFinished, noRead, noWrite, noFinal, noEnd, noFinish, noCleanup], ['rfalse', false, true, true, 0, 1, 1, 0, 1, 1]);
  var disabledEndCounts = [0,0,0,0], noHalfOpenEnd = new stream.Duplex({ readable: false, allowHalfOpen: false, autoDestroy: false, write: function(c,e,cb) { cb(); } }); var disabledOn = function() { disabledEndCounts[0]++; }, disabledOnce = function() { disabledEndCounts[1]++; }, disabledPrepend = function() { disabledEndCounts[2]++; }, disabledPrependOnce = function() { disabledEndCounts[3]++; }; noHalfOpenEnd.on('end', disabledOn); noHalfOpenEnd.once('end', disabledOnce); noHalfOpenEnd.prependListener('end', disabledPrepend); noHalfOpenEnd.prependOnceListener('end', disabledPrependOnce); noHalfOpenEnd.end(); await within(new Promise(function(resolve) { stream.finished(noHalfOpenEnd, { readable: false, cleanup: true }, resolve); }), 'disabled-end-listeners-finish'); await within(Promise.resolve(), 'disabled-end-listeners-microtask'); record(['disabled-readable-listeners', disabledEndCounts, noHalfOpenEnd.readableEnded, noHalfOpenEnd.writableFinished, noHalfOpenEnd.listenerCount('end')], ['disabled-readable-listeners', [0,0,0,0], true, true, 5]); noHalfOpenEnd.removeAllListeners('end');
  var disabledWriteCalls = 0, disabledFinalCalls = 0, disabledEnd = 0, disabledFinish = 0, disabledCleanup = 0;
  var wd = new stream.Duplex({ writable: false, write: function(c, e, cb) { disabledWriteCalls++; cb(); }, final: function(cb) { disabledFinalCalls++; cb(); }, destroy: function(e, cb) { disabledCleanup++; cb(e); } });
  wd.on('end', function() { disabledEnd++; }); wd.on('finish', function() { disabledFinish++; }); var wdClose = event(wd, 'close'); wd.push('x'); wd.push(null); var wdValue = wd.read().toString(); await within(failure(wd, { writable: false }), 'duplex-writable-disabled'); await within(wdClose, 'duplex-writable-disabled-close');
  record(['wfalse', wdValue, wd.writable, wd.writableFinished, disabledWriteCalls, disabledFinalCalls, disabledEnd, disabledFinish, disabledCleanup], ['wfalse', 'x', false, true, 0, 0, 1, 0, 1]);
  var bothIO = 0, bothTerminal = 0, bothCleanup = 0;
  var both = new stream.Duplex({ readable: false, writable: false, read: function() { bothIO++; }, write: function(c, e, cb) { bothIO++; cb(); }, final: function(cb) { bothIO++; cb(); }, destroy: function(e, cb) { bothCleanup++; cb(e); } });
  both.on('end', function() { bothTerminal++; }); both.on('finish', function() { bothTerminal++; }); var bothClose = event(both, 'close'); await within(bothClose, 'duplex-both-disabled');
  record(['bothfalse', both.readableEnded, both.writableFinished, bothIO, bothTerminal, bothCleanup], ['bothfalse', true, true, 0, 0, 1]);
  var bothLateCounts = [0,0,0], bothLate = new stream.Duplex({ readable: false, writable: false }); bothLate.on('end', function() { bothLateCounts[0]++; }); bothLate.once('end', function() { bothLateCounts[1]++; }); bothLate.prependListener('end', function() { bothLateCounts[2]++; }); var bothLateClose = event(bothLate, 'close'); await within(bothLateClose, 'both-disabled-late-end-close'); await within(Promise.resolve(), 'both-disabled-late-end-microtask'); record(['both-disabled-late-end', bothLateCounts, bothLate.listenerCount('end')], ['both-disabled-late-end', [0,0,0], 3]); bothLate.removeAllListeners('end');
  var dual = new stream.Duplex({ read: function() {}, write: function(c, e, cb) { cb(); } }), dualEnds = 0; dual.on('end', function() { dualEnds++; }); var dualEnd = event(dual, 'end'), dualClose = event(dual, 'close'); dual.push(null); dual.read(); await within(dualEnd, 'duplex-readable-first'); await within(Promise.resolve(), 'microtask'); var stayedOpen = !dual.destroyed; dual.end(); await within(failure(dual), 'duplex-both-complete'); await within(dualClose, 'duplex-both-close'); record(['dual', stayedOpen, dualEnds, dual.writableFinished, dual.destroyed], ['dual', true, 1, true, true]);
  var manualDestroy = 0, manual = new stream.Duplex({ readable: false, writable: false, autoDestroy: false, destroy: function(e, cb) { manualDestroy++; cb(e); } }); await within(Promise.resolve(), 'microtask'); var manualStillOpen = !manual.destroyed, manualClose = event(manual, 'close'); manual.destroy(); await within(manualClose, 'duplex-no-auto-destroy'); record(['no-auto', manualStillOpen, manualDestroy, manual.destroyed], ['no-auto', true, 1, true]);

  // Every concrete constructor handles pre-abort and abort during pending construction after its state is complete.
  async function abortDuring(kind, preabort) {
    var why = new Error('reason'), sig = signal(preabort, why), constructDone, constructCalls = 0, acquired = false, released = false, destroyCalls = 0, seen, seenReady = false, errors = [], value;
    value = make(kind, { signal: sig, emitClose: !(kind === 'D' && preabort), construct: function(cb) { constructCalls++; constructDone = cb; }, destroy: function(error, cb) { destroyCalls++; seen = error; seenReady = kind === 'R' ? Boolean(this._chunks) : kind === 'W' ? Boolean(this._writeQueue) : Boolean(this._chunks && this._writeQueue && (kind !== 'T' || typeof this._transform === 'function')); released = acquired; cb(error); } });
    value.on('error', function(error) { errors.push(error); }); var terminal = new Promise(function(resolve) { stream.finished(value, { error: false }, resolve); });
    var constructReleased = false, cleanupEarly;
    try {
      if (!preabort) { await within(Promise.resolve(), 'abort-start-' + kind); sig.abort(why); }
      await within(Promise.resolve(), 'construct-start-' + kind); cleanupEarly = destroyCalls !== 0; acquired = true; if (constructDone) { constructDone(); constructReleased = true; } else throw new Error('construct did not start'); await within(terminal, 'abort-' + kind + '-' + preabort);
    } finally { if (!constructReleased && constructDone) { acquired = true; constructReleased = true; constructDone(); } }
    return [kind, preabort, cleanupEarly, constructCalls, destroyCalls, released, seenReady, seen && seen.name, seen && seen.code, seen && seen.cause === why, errors.length, errors[0] === seen, sig.added, sig.removed, sig.listenerCount(), value._destroyFinished];
  }
  for (var kind of ['R', 'W', 'D', 'T']) { record(await abortDuring(kind, true), [kind, true, false, 1, 1, true, true, 'AbortError', 'ABORT_ERR', true, 1, true, 0, 1, 0, true]); record(await abortDuring(kind, false), [kind, false, false, 1, 1, true, true, 'AbortError', 'ABORT_ERR', true, 1, true, 1, 1, 0, true]); }
  async function readyAbort(kind, native) {
    var why = new Error('ready-signal-reason'), localSignal = native ? new AbortController().signal : signal(false), abort = native ? function() { controller.abort(why); } : function() { localSignal.abort(why); }, controller = native ? new AbortController() : null;
    if (native) { localSignal = controller.signal; abort = function() { controller.abort(why); }; }
    var started = 0, cleaned = 0, seen, releaseRead, releaseWrite, readCalls = 0, writeCalls = 0, options = { signal: localSignal, destroy: function(error, cb) { cleaned++; seen = error; cb(error); } };
    options.read = function() { readCalls++; started++; releaseRead = function() { this.push('late'); }; };
    options.write = function(chunk, encoding, cb) { writeCalls++; started++; releaseWrite = cb; };
    options.transform = function(chunk, encoding, cb) { writeCalls++; started++; releaseWrite = function(error) { cb(error, error ? undefined : chunk); }; };
    var value = make(kind, options); value.on('error', function() {}); var terminal = failure(value, { error: false });
    if (kind === 'R' || kind === 'D') value.read();
    if (kind !== 'R') value.write('input');
    var reached = started; var wasReleased = false;
    try { abort(); if (releaseRead) releaseRead.call(value); if (releaseWrite) releaseWrite(seen); wasReleased = true; await within(terminal, 'ready-abort-' + kind + '-' + native); }
    finally { if (!wasReleased) { if (releaseRead) releaseRead.call(value); if (releaseWrite) releaseWrite(seen); } }
    return [kind, native, reached, readCalls, writeCalls, cleaned, seen && seen.name, seen && seen.code, seen && seen.cause === why, localSignal.listenerCount ? localSignal.listenerCount() : null];
  }
  for (var readyKind of ['R', 'W', 'D', 'T']) { record(await readyAbort(readyKind, false), [readyKind, false, readyKind === 'D' ? 2 : 1, readyKind === 'R' || readyKind === 'D' ? 1 : 0, readyKind === 'R' ? 0 : 1, 1, 'AbortError', 'ABORT_ERR', true, 0]); record(await readyAbort(readyKind, true), [readyKind, true, readyKind === 'D' ? 2 : 1, readyKind === 'R' || readyKind === 'D' ? 1 : 0, readyKind === 'R' ? 0 : 1, 1, 'AbortError', 'ABORT_ERR', true, null]); }
  var absent = ['R','W','D','T'].map(function(kind) { return [kind, make(kind, {}).destroyed]; }), invalid = [];
  for (var invalidKind of ['R','W','D','T']) { try { make(invalidKind, { signal: {} }); } catch (error) { invalid.push([invalidKind, error.name, error.code]); } }
  record(['signal-controls', absent, invalid], ['signal-controls', [['R', false], ['W', false], ['D', false], ['T', false]], [['R','TypeError','ERR_INVALID_ARG_TYPE'], ['W','TypeError','ERR_INVALID_ARG_TYPE'], ['D','TypeError','ERR_INVALID_ARG_TYPE'], ['T','TypeError','ERR_INVALID_ARG_TYPE']]]);
  var subclassController = new AbortController(), subclassReason = new Error('subclass-abort'), subclassReady; subclassController.abort(subclassReason); function UserReadable(options) { stream.Readable.call(this, options); this.subclassState = 'ready'; } UserReadable.prototype = Object.create(stream.Readable.prototype); UserReadable.prototype.constructor = UserReadable; var userReadable = new UserReadable({ signal: subclassController.signal, destroy: function(error, cb) { subclassReady = this.subclassState === 'ready'; cb(error); } }); userReadable.on('error', function() {}); await within(failure(userReadable, { error: false }), 'user-readable-subclass-abort'); record(['signal-user-subclass', subclassReady, userReadable.errored && userReadable.errored.cause === subclassReason], ['signal-user-subclass', true, true]);
  var fromReason = new Error('from-signal'), fromController = new AbortController(); fromController.abort(fromReason); var iteratorReturned = 0, fromStream = stream.Readable.from({ [Symbol.iterator]: function() { return { next: function() { return { done: false, value: 'never' }; }, return: function() { iteratorReturned++; return { done: true }; } }; } }, { signal: fromController.signal }); fromStream.on('error', function() {}); await within(failure(fromStream, { error: false }), 'readable-from-preabort'); await within(Promise.resolve(), 'readable-from-return'); record(['readable-from-signal', iteratorReturned, fromStream.errored && fromStream.errored.cause === fromReason], ['readable-from-signal', 1, true]);
  var readerCanceled, webReader = { getReader: function() { return { read: function() { return Promise.resolve({ done: false, value: 'unused' }); }, cancel: function(error) { readerCanceled = error; return Promise.resolve(); } }; } }; var webRead = stream.Readable.fromWeb(webReader, { signal: fromController.signal }); webRead.on('error', function() {}); await within(failure(webRead, { error: false }), 'readable-from-web-preabort'); record(['readable-from-web-signal', readerCanceled && readerCanceled.cause === fromReason, webRead.errored && webRead.errored.cause === fromReason], ['readable-from-web-signal', true, true]);
  var writerAborted, webWriter = { getWriter: function() { return { write: function() { return Promise.resolve(); }, close: function() { return Promise.resolve(); }, abort: function(error) { writerAborted = error; return Promise.resolve(); } }; } }; var webWrite = stream.Writable.fromWeb(webWriter, { signal: fromController.signal }); webWrite.on('error', function() {}); await within(failure(webWrite, { error: false }), 'writable-from-web-preabort'); record(['writable-from-web-signal', writerAborted && writerAborted.cause === fromReason, webWrite.errored && webWrite.errored.cause === fromReason], ['writable-from-web-signal', true, true]);
  async function normalSignalControl(kind) { var sig = signal(false), destroys = 0, value = make(kind, { signal: sig, destroy: function(error, cb) { destroys++; cb(error); } }); value.on('error', function() {}); if (kind === 'R') { value.push(null); value.read(); } else if (kind === 'W') value.end(); else if (kind === 'D') { value.push(null); value.read(); value.end(); } else { value.resume(); value.end('x'); } await within(failure(value), 'signal-normal-' + kind); sig.abort(new Error('late')); return [kind, sig.added, sig.removed, sig.listenerCount(), destroys, value.errored === null]; }
  for (var normalKind of ['R','W','D','T']) record(await normalSignalControl(normalKind), [normalKind, 1, 1, 0, 1, true]);
  var silentSignal = signal(false), silentDestroys = 0, silentStream = new stream.Writable({ signal: silentSignal, emitClose: false, destroy: function(e, cb) { silentDestroys++; cb(e); } }); silentStream.on('error', function() {}); silentSignal.abort(new Error('silent abort')); var silentResult = await failure(silentStream); record(['signal-silent', silentResult && silentResult.code, silentStream.closed, silentDestroys, silentSignal.added, silentSignal.removed, silentSignal.listenerCount()], ['signal-silent', 'ABORT_ERR', true, 1, 1, 1, 0]);

  // Late finished callback and promise paths settle from saved terminal state; silent premature close settles before and after teardown.
  var saved = new Error('saved'), broken = new stream.Readable(), publicErrors = 0; broken.on('error', function(error) { if (error === saved) publicErrors++; }); var brokenClose = event(broken, 'close'); broken.destroy(saved); await within(brokenClose, 'saved-close'); var lateError, lateCalls = 0; await within(new Promise(function(resolve) { stream.finished(broken, { cleanup: true }, function(error) { lateCalls++; lateError = error; resolve(); }); }), 'late-finished-callback'); var promiseError; try { await within(stream.promises.finished(broken), 'late-finished-promise'); } catch (error) { promiseError = error; }
  record(['late-error', lateError === saved, lateCalls, promiseError === saved, publicErrors, broken.listenerCount('close'), broken._destroyWaiters.length], ['late-error', true, 1, true, 1, 0, 0]);
  var soleError = new Error('sole-finished-error'), sole = new stream.Readable(), soleCallback, soleCallbackCalls = 0, solePromiseError; var soleCallbackWait = new Promise(function(resolve) { stream.finished(sole, { cleanup: true }, function(error) { soleCallback = error; soleCallbackCalls++; resolve(); }); }); var solePromise = stream.promises.finished(sole).catch(function(error) { solePromiseError = error; }); sole.destroy(soleError); await within(soleCallbackWait, 'sole-finished-callback'); await within(solePromise, 'sole-finished-promise'); record(['sole-finished-error', soleCallback === soleError, soleCallbackCalls, solePromiseError === soleError, sole.listenerCount('error'), sole._destroyWaiters.length], ['sole-finished-error', true, 1, true, 0, 0]);
  var observerFailure = new Error('observer-failure'), observerFinished = false, observerClose = false, observer = new stream.Readable(); observer.on('close', function() { observerClose = true; }); observer._destroyWaiters.push(function() { throw observerFailure; }); observer._destroyWaiters.push(function() { observerFinished = true; }); var thrownObserver; try { observer.destroy(); } catch (error) { thrownObserver = error; } await within(event(observer, 'close'), 'observer-throw-close'); record(['observer-throw', thrownObserver === observerFailure, observerFinished, observerClose, observer.closed], ['observer-throw', true, true, true, true]);
  var firstCycle = new Error('cycle-one'), secondCycle = new Error('cycle-two'), cycleErrors = [], publicCycles = [], cycles = new stream.Readable(); cycles.on('error', function(error) { publicCycles.push(error); }); cycles._destroyWaiters.push(function(error) { cycleErrors.push(error); cycles._undestroy(); cycles.destroy(secondCycle); }); var cycleClosed = event(cycles, 'close'); cycles.destroy(firstCycle); await within(cycleClosed, 'reentrant-destroy-cycle'); await within(Promise.resolve(), 'microtask'); record(['reentrant-cycle', cycleErrors[0] === firstCycle, publicCycles.length, publicCycles[0] === firstCycle, publicCycles[1] === secondCycle, cycles.errored === secondCycle, cycles.closed], ['reentrant-cycle', true, 2, true, true, true, true]);
  var silentBefore = new stream.Readable({ emitClose: false }), beforeCode, beforeCalls = 0; var beforeFinished = new Promise(function(resolve) { stream.finished(silentBefore, { cleanup: true }, function(error) { beforeCalls++; beforeCode = error && error.code; resolve(); }); }); silentBefore.destroy(); await within(beforeFinished, 'silent-before'); var silentAfter = new stream.Readable({ emitClose: false }); silentAfter.destroy(); var afterCode, afterCalls = 0; await within(new Promise(function(resolve) { stream.finished(silentAfter, { cleanup: true }, function(error) { afterCalls++; afterCode = error && error.code; resolve(); }); }), 'silent-after'); record(['silent-finished', beforeCode, beforeCalls, afterCode, afterCalls, silentBefore.listenerCount('close'), silentAfter.listenerCount('close')], ['silent-finished', 'ERR_STREAM_PREMATURE_CLOSE', 1, 'ERR_STREAM_PREMATURE_CLOSE', 1, 0, 0]);
  var complete = new stream.Duplex({ autoDestroy: false, read: function() {}, write: function(c, e, cb) { cb(); } }); var successErr; var completeWait = new Promise(function(resolve) { stream.finished(complete, { cleanup: true }, function(error) { successErr = error; resolve(); }); }); complete.push(null); complete.read(); complete.end(); await within(completeWait, 'normal-finished'); var falseErr; await within(new Promise(function(resolve) { stream.finished(broken, { error: false }, function(error) { falseErr = error; resolve(); }); }), 'error-false-late'); record(['finished-controls', successErr === undefined, falseErr === undefined, complete.listenerCount('end'), complete.listenerCount('finish'), complete.listenerCount('close')], ['finished-controls', true, true, 0, 0, 0]);
  var lateEnd = new stream.Readable({ autoDestroy: false }), actualEnd = 0; lateEnd.once('end', function() { actualEnd++; }); lateEnd.push(null); lateEnd.read(); await within(new Promise(function(resolve) { stream.finished(lateEnd, { cleanup: true }, function(error) { if (error) throw error; resolve(); }); }), 'late-end-finished-state'); var lateEndCalls = [0,0,0,0,0,0], removedReplay = 0; function lateOn() { lateEndCalls[0]++; lateEndCalls[1] += this === lateEnd ? 1 : 0; } function lateAdd() { lateEndCalls[2]++; } function lateOnce() { lateEndCalls[3]++; } function latePrepend() { lateEndCalls[4]++; } function latePrependOnce() { lateEndCalls[5]++; } function removeBeforeQueued() { removedReplay++; } lateEnd.on('end', lateOn); lateEnd.addListener('end', lateAdd); lateEnd.once('end', lateOnce); lateEnd.prependListener('end', latePrepend); lateEnd.prependOnceListener('end', latePrependOnce); lateEnd.on('end', removeBeforeQueued); lateEnd.removeListener('end', removeBeforeQueued); await within(Promise.resolve(), 'late-end-no-replay-microtask'); var noReplay = [lateEndCalls.slice(), removedReplay, lateEnd.listenerCount('end', lateOnce), lateEnd.listenerCount('end', latePrependOnce)]; await within(new Promise(function(resolve) { stream.finished(lateEnd, { cleanup: true }, function(error) { if (error) throw error; resolve(); }); }), 'late-end-terminal-control'); lateEnd.emit('end'); record(['late-end-registration', actualEnd, noReplay, lateEndCalls, removedReplay, lateEnd.listenerCount('end', lateOnce), lateEnd.listenerCount('end', latePrependOnce)], ['late-end-registration', 1, [[0,0,0,0,0,0], 0, 1, 1], [1,1,1,1,1,1], 0, 0, 0]);

  // _construct stays alive after a scheduled destroy; errors and repeated callbacks enter effective Writable.destroy once.
  async function constructGate(kind, destroyBeforeStart) {
    var done, calls = 0, acquired = false, releases = 0, destroys = 0, received, wrote = 0, callbackError, value = make(kind, { construct: function(cb) { calls++; done = cb; }, read: function() { wrote++; }, transform: function(c,e,cb) { wrote++; cb(null,c); }, write: function(c, e, cb) { wrote++; cb(); }, destroy: function(error, cb) { destroys++; received = error; releases += acquired ? 1 : 0; cb(error); } });
    value.on('error', function() {}); var close = event(value, 'close'), constructReleased = false, early; if (kind === 'R') value.read(); else value.write('queued', function(error) { callbackError = error; });
    try { if (destroyBeforeStart) value.destroy(); await within(Promise.resolve(), 'construct-scheduled-' + kind); if (!destroyBeforeStart) value.destroy(); early = destroys; acquired = true; done(); constructReleased = true; await within(close, 'construct-gate-' + kind); }
    finally { if (!constructReleased && done) { acquired = true; constructReleased = true; done(); } }
    return [kind, destroyBeforeStart, calls, early, destroys, releases, wrote, Boolean(callbackError), received === null, value._constructWaiters.length, value.closed];
  }
  for (var gateKind of ['R', 'W', 'D', 'T']) record(await constructGate(gateKind, false), [gateKind, false, 1, 0, 1, 1, 0, gateKind !== 'R', true, 0, true]);
  for (var scheduledKind of ['R', 'W', 'D', 'T']) record(await constructGate(scheduledKind, true), [scheduledKind, true, 1, 0, 1, 1, 0, scheduledKind !== 'R', true, 0, true]);
  async function constructFailure(kind, throws) {
    var original = new Error('construct-failure'), done, cleanup = 0, got, released = false, value = make(kind, { construct: function(cb) { done = cb; if (throws) throw original; }, destroy: function(error, cb) { cleanup++; got = error; cb(error); } }); value.on('error', function() {}); var close = event(value, 'close');
    try { await within(Promise.resolve(), 'microtask'); if (!throws) { done(original); released = true; } await within(close, 'construct-failure-' + kind); }
    finally { if (!released && done) { released = true; done(original); } }
    return [kind, throws, cleanup, got === original, value.errored === original];
  }
  for (var rejectKind of ['R', 'W', 'D', 'T']) record(await constructFailure(rejectKind, false), [rejectKind, false, 1, true, true]);
  for (var throwKind of ['R', 'W', 'D', 'T']) record(await constructFailure(throwKind, true), [throwKind, true, 1, true, true]);
  var first = new Error('first'), second = new Error('second'), duplicate = new stream.Writable({ construct: function(cb) { this._constructDone = cb; }, write: function(c,e,cb) { cb(); }, destroy: function(error, cb) { this._cleanupError = error; cb(error); } }); duplicate.on('error', function() {}); var duplicateClose = event(duplicate, 'close'); await within(Promise.resolve(), 'microtask'); duplicate._constructDone(); duplicate._constructDone(new Error('ignored construct callback')); duplicate.destroy(first); duplicate.destroy(second); await within(duplicateClose, 'construct-double'); record(['double', duplicate._cleanupError === first, duplicate.errored === first, duplicate.destroyed], ['double', true, true, true]);
  for (var effectiveKind of ['W','D','T']) { var failAfterRequest = new Error('construct-fails-after-destroy'), constructAgain, cleanupAfterRequest = 0, effective = make(effectiveKind, { construct: function(cb) { constructAgain = cb; }, destroy: function(error, cb) { cleanupAfterRequest++; cb(error); } }); effective.on('error', function() {}); var effectiveClose = event(effective, 'close'); if (effectiveKind !== 'T') effective.write('queued'); else effective.write('queued'); effective.destroy(); await within(Promise.resolve(), 'microtask'); constructAgain(failAfterRequest); await within(effectiveClose, 'construct-after-destroy-' + effectiveKind); record(['construct-after-destroy', effectiveKind, cleanupAfterRequest, effective.errored === failAfterRequest, effective.closed], ['construct-after-destroy', effectiveKind, 1, true, true]); }
  for (var readyKind of ['R','W','D','T']) { var readyCount = 0, ready = make(readyKind, { construct: function(cb) { readyCount++; cb(); } }); await within(Promise.resolve(), 'microtask'); record(['construct-ready', readyKind, readyCount, ready._constructState], ['construct-ready', readyKind, 1, 'ready']); }

  // asyncDispose is shared by effective Readable and Writable definitions, joins pending teardown, and preserves Node AbortError semantics.
  async function disposeCase(kind, emitClose, cleanupError) {
    var release, started = 0, passed, errors = [], errorCount = 0, value = make(kind, { autoDestroy: false, emitClose: emitClose, destroy: function(error, cb) { started++; passed = error; release = cb; } }); value.on('error', function(error) { errors.push(error); errorCount++; }); var sawClose = false; value.on('close', function() { sawClose = true; }); var disposed = false, disposeError; var promise = value[Symbol.asyncDispose]().then(function() { disposed = true; }, function(error) { disposed = true; disposeError = error; }); await within(Promise.resolve(), 'dispose-start-' + kind); var waited = !disposed, callbackError = cleanupError || passed, released = false;
    try { check(typeof release === 'function', 'asyncDispose starts cleanup'); release(callbackError); released = true; await within(promise, 'async-dispose-' + kind); await within(Promise.resolve(), 'dispose-event-' + kind); }
    finally { if (!released && release) { released = true; release(callbackError); } }
    return [kind, emitClose, waited, started, passed && passed.name, passed && passed.code, disposed, cleanupError ? disposeError === cleanupError : disposeError === undefined, errorCount, errors[0] === callbackError, sawClose, value.closed];
  }
  for (var disposeKind of ['R', 'W']) record(await disposeCase(disposeKind, true, null), [disposeKind, true, true, 1, 'AbortError', 'ABORT_ERR', true, true, 1, true, true, true]);
  var cleanupFailure = new Error('cleanup-failure'); for (var failKind of ['R', 'W']) record(await disposeCase(failKind, false, cleanupFailure), [failKind, false, true, 1, 'AbortError', 'ABORT_ERR', true, true, 1, true, false, true]);
  async function soleDispose(kind, cleanupError) { var release, received, value = make(kind, { emitClose: false, destroy: function(error, cb) { received = error; release = cb; } }), settled = false, rejected; var promise = value[Symbol.asyncDispose]().then(function() { settled = true; }, function(error) { settled = true; rejected = error; }); var released = false, waiting, listenersWhileWaiting; try { await within(Promise.resolve(), 'sole-dispose-start-' + kind); waiting = !settled; listenersWhileWaiting = value.listenerCount('error'); release(cleanupError || received); released = true; await within(promise, 'sole-dispose-' + kind); } finally { if (!released && release) { released = true; release(cleanupError || received); } } return [kind, waiting, listenersWhileWaiting, received && received.name, received && received.code, settled, cleanupError ? rejected === cleanupError : rejected === undefined, value.listenerCount('error'), value.closed]; }
  for (var soleDisposeKind of ['R','W']) record(await soleDispose(soleDisposeKind, null), [soleDisposeKind, true, 1, 'AbortError', 'ABORT_ERR', true, true, 0, true]); var soleCleanupFailure = new Error('sole-cleanup-failure'); for (var soleFailKind of ['R','W']) record(await soleDispose(soleFailKind, soleCleanupFailure), [soleFailKind, true, 1, 'AbortError', 'ABORT_ERR', true, true, 0, true]);
  async function joinDispose(kind) { var release, calls = 0, value = make(kind, { destroy: function(error, cb) { calls++; release = cb; } }); value.on('error', function() {}); value.destroy(); var settled = false, promise = value[Symbol.asyncDispose]().then(function() { settled = true; }, function() { settled = true; }); var released = false, early; try { await within(Promise.resolve(), 'join-ready-' + kind); early = settled; release(); released = true; await within(promise, 'join-dispose-' + kind); } finally { if (!released && release) { released = true; release(); } } return [kind, early, calls, settled]; }
  for (var joinKind of ['R','W']) record(await joinDispose(joinKind), [joinKind, false, 1, true]);
  async function failedDisposed(kind) { var error = new Error('already-destroyed'), value = make(kind, { destroy: function(e, cb) { cb(e); } }); value.on('error', function() {}); var closed = event(value, 'close'); value.destroy(error); await within(closed, 'failed-dispose-close-' + kind); var seen; try { await within(value[Symbol.asyncDispose](), 'failed-dispose-' + kind); } catch (actual) { seen = actual; } return [kind, seen === error, value.closed]; }
  for (var failedKind of ['R','W']) record(await failedDisposed(failedKind), [failedKind, true, true]);
  async function preexistingAbort(kind) { var error = new Error('preexisting-abort'), value = make(kind), absorb = function() {}; error.name = 'AbortError'; error.code = 'ABORT_ERR'; value.on('error', absorb); var closed = event(value, 'close'); value.destroy(error); await within(closed, 'preexisting-abort-close-' + kind); value.removeListener('error', absorb); var resolved = false, rejected; await within(value[Symbol.asyncDispose]().then(function() { resolved = true; }, function(actual) { rejected = actual; }), 'preexisting-abort-dispose-' + kind); return [kind, resolved, rejected === error, value.listenerCount('error')]; }
  record(await preexistingAbort('R'), ['R', false, true, 0]); record(await preexistingAbort('W'), ['W', true, false, 0]);
  async function immediateDisposed(kind, useAbortError) { var error = new Error(useAbortError ? 'immediate-abort' : 'immediate-saved'); if (useAbortError) { error.name = 'AbortError'; error.code = 'ABORT_ERR'; } var value = make(kind, { emitClose: false }), resolved = false, rejected, calls = 0; value.destroy(error); var promise = value[Symbol.asyncDispose]().then(function() { resolved = true; calls++; }, function(actual) { rejected = actual; calls++; }); await within(promise, 'immediate-dispose-' + kind + '-' + useAbortError); return [kind, useAbortError, resolved, rejected === error, calls, value.listenerCount('error'), value.closed]; }
  for (var immediateKind of ['R','W']) record(await immediateDisposed(immediateKind, false), [immediateKind, false, false, true, 1, 0, true]); record(await immediateDisposed('R', true), ['R', true, false, true, 1, 0, true]); record(await immediateDisposed('W', true), ['W', true, true, false, 1, 0, true]);
  var duplexRelease, duplexReceived, directionalDuplex = new stream.Duplex({ autoDestroy: false, read: function() {}, write: function(c,e,cb) { cb(); }, destroy: function(error, cb) { duplexReceived = error; duplexRelease = cb; } }); directionalDuplex.push(null); directionalDuplex.read(); var directionalResolved = false, directionalPromise = directionalDuplex[Symbol.asyncDispose]().then(function() { directionalResolved = true; }); var directionalReleased = false, directionalWaiting; try { await within(Promise.resolve(), 'duplex-direction-start'); directionalWaiting = !directionalResolved; if (duplexRelease) { duplexRelease(null); directionalReleased = true; } await within(directionalPromise, 'duplex-direction-dispose'); } finally { if (!directionalReleased && duplexRelease) { directionalReleased = true; duplexRelease(null); } } record(['duplex-directional-dispose', directionalDuplex.readableEnded, directionalDuplex.writableFinished, duplexReceived === null, directionalWaiting, directionalResolved], ['duplex-directional-dispose', true, false, true, true, true]);
  async function completedDispose(kind) { var release, calls = 0, received, value = make(kind, { autoDestroy: false, destroy: function(error, cb) { calls++; received = error; release = cb; } }); value.on('error', function() {}); if (kind === 'R') { value.push(null); value.read(); } else { value.end(); await within(failure(value), 'complete-before-dispose-' + kind); } var settled = false, promise = value[Symbol.asyncDispose]().then(function() { settled = true; }); var released = false, early; try { await within(Promise.resolve(), 'complete-dispose-start-' + kind); early = settled; check(typeof release === 'function', 'completed stream begins null destroy'); release(null); released = true; await within(promise, 'completed-dispose-' + kind); } finally { if (!released && release) { released = true; release(null); } } return [kind, calls, received === null, early, settled, value.closed]; }
  for (var completeKind of ['R','W']) record(await completedDispose(completeKind), [completeKind, 1, true, false, true, true]);
  var duplexDisposeDone, duplexDisposeCalls = 0, duplexDisposed = new stream.Duplex({ autoDestroy: false, read: function() {}, write: function(c,e,cb) { cb(); }, destroy: function(e,cb) { duplexDisposeCalls++; duplexDisposeDone = cb; } }); duplexDisposed.on('error', function() {}); var duplexDisposeError, duplexResolved = false; var duplexPromise = duplexDisposed[Symbol.asyncDispose]().then(function() { duplexResolved = true; }, function(error) { duplexDisposeError = error; }); var duplexReleased = false, duplexWaiting; try { await within(Promise.resolve(), 'microtask'); duplexWaiting = !duplexResolved; duplexDisposeDone(); duplexReleased = true; await within(duplexPromise, 'duplex-dispose'); } finally { if (!duplexReleased && duplexDisposeDone) { duplexReleased = true; duplexDisposeDone(); } } record(['duplex-dispose', duplexWaiting, duplexDisposeCalls, duplexResolved, duplexDisposeError], ['duplex-dispose', true, 1, true, undefined]);

  // EOF receipt does not complete readable state until byte/object buffers drain; EOF flushes decoder exactly once.
  var eof = new stream.Readable({ autoDestroy: false }), endCount = 0, finishedBefore = false; eof.on('end', function() { endCount++; }); eof.push('ab'); eof.push('cd'); eof.push(null); stream.finished(eof, function(error) { if (error) throw error; finishedBefore = true; }); await within(Promise.resolve(), 'microtask'); var stateBefore = [eof.readableEnded, finishedBefore, eof.readableLength]; var part = eof.read(1).toString(); var stateMiddle = [eof.readableEnded, endCount]; var rest = eof.read().toString(); await within(Promise.resolve(), 'microtask'); record(['byte-eof', stateBefore, part, stateMiddle, rest, eof.readableEnded, endCount, finishedBefore], ['byte-eof', [false, false, 4], 'a', [false, 0], 'bcd', true, 1, true]);
  var objects = new stream.Readable({ objectMode: true, autoDestroy: false }); objects.push({ n: 1 }); objects.push({ n: 2 }); objects.push(null); var objectBefore = objects.readableEnded, objectFirst = objects.read().n, objectMiddle = objects.readableEnded, objectSecond = objects.read().n; record(['object-eof', objectBefore, objectFirst, objectMiddle, objectSecond, objects.readableEnded], ['object-eof', false, 1, false, 2, true]);
  var afterEof = new stream.Readable({ autoDestroy: false }), eofError; afterEof.on('error', function(error) { eofError = error; }); afterEof.push(null); afterEof.push('late'); record(['push-after-eof', eofError && eofError.code, afterEof.errored === eofError], ['push-after-eof', 'ERR_STREAM_PUSH_AFTER_EOF', true]);
  var decoder = new stream.Readable({ autoDestroy: false }).setEncoding('utf8'), decoderValues = []; decoder.on('data', function(value) { decoderValues.push(value); }); decoder.pause(); decoder.push(Buffer.from([0xe3])); decoder.push(null); decoder.push(null); decoder.resume(); record(['decoder-eof', decoderValues, decoder.readableEnded], ['decoder-eof', ['�'], true]);
  var empty = new stream.Readable({ autoDestroy: false }), emptyEnds = 0; empty.on('end', function() { emptyEnds++; }); empty.push(null); empty.push(null); record(['empty-eof', empty.readableEnded, emptyEnds], ['empty-eof', true, 1]);
  var pipeSource = new stream.Readable({ autoDestroy: false }), pipeValues = [], pipeDestination = new stream.Writable({ write: function(chunk, encoding, cb) { pipeValues.push(chunk.toString()); cb(); } }), pipeFinish = event(pipeDestination, 'finish'); pipeSource.pipe(pipeDestination); pipeSource.push('pipe'); pipeSource.push(null); await within(pipeFinish, 'eof-pipe'); record(['pipe-eof', pipeValues, pipeSource.readableEnded, pipeDestination.writableFinished], ['pipe-eof', ['pipe'], true, true]);
  var iterated = await within(stream.Readable.from(['i1','i2']).toArray(), 'eof-iterator'); record(['eof-iterator', iterated], ['eof-iterator', ['i1','i2']]);
  var pipelineValues = [], pipelineSource = stream.Readable.from(['p1','p2']), pipelineSink = new stream.Writable({ write: function(chunk, encoding, cb) { pipelineValues.push(chunk.toString()); cb(); } }); await within(stream.promises.pipeline(pipelineSource, pipelineSink), 'eof-pipeline'); record(['eof-pipeline', pipelineValues, pipelineSource.readableEnded, pipelineSink.writableFinished], ['eof-pipeline', ['p1','p2'], true, true]);
  var composed = stream.Readable.from(['c1','c2']).compose(new stream.Transform({ transform: function(chunk, encoding, cb) { cb(null, chunk.toString().toUpperCase()); } })); var composedBuffers = await within(composed.toArray(), 'eof-compose'), composedResult = composedBuffers.map(function(value) { return value.toString(); }); record(['eof-compose', composed.readableObjectMode, composedBuffers.every(function(value) { return Buffer.isBuffer(value); }), composedResult], ['eof-compose', false, true, ['C1','C2']]);
  var pair = stream.duplexPair(), pairValues = []; pair[1].on('data', function(chunk) { pairValues.push(chunk.toString()); }); var pairEnd = event(pair[1], 'end'); pair[0].end('pair'); await within(pairEnd, 'eof-duplex-pair'); record(['eof-duplex-pair', pairValues, pair[1].readableEnded], ['eof-duplex-pair', ['pair'], true]);

  return observed;
};
"#,
    ).unwrap();
    let empty_node_modules = temp_registry("builtin_stream_core_contracts_u1_matrix_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamU1 = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamU1").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    // The JS matrix returns the measured trigger/control transcript for the root-owned finite gates.
    assert!(result.starts_with("[[\"rfalse\"") && result.contains("\"pipe-eof\""), "unexpected U1 result: {result}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

