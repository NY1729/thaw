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


