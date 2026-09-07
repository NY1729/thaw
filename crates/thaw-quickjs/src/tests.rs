use super::*;

fn call(func_name: &str, args_json: &str) -> String {
    let func_name = CString::new(func_name).unwrap();
    let args_json = CString::new(args_json).unwrap();
    let result_ptr = thaw_js_call(func_name.as_ptr(), args_json.as_ptr());
    unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned()
}

fn load(source: &str) -> u8 {
    let source = CString::new(source).unwrap();
    thaw_js_load(source.as_ptr())
}

#[test]
fn load_script_accepts_gzip_base64_source() {
    use base64::Engine as _;
    use std::io::Write;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(b"globalThis.__thaw_compressed_script = 42;")
        .unwrap();
    let source = format!(
        "gz:{}",
        base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap())
    );

    assert_eq!(load(&source), 1);
    assert_eq!(
        eval_json("globalThis.__thaw_compressed_script"),
        Ok(Some("42".into()))
    );
}

#[cfg(not(feature = "brotli"))]
#[test]
fn minimal_host_keeps_gzip_and_rejects_brotli() {
    let compressed = compress_bytes("gzip", b"hello").unwrap();
    assert_eq!(decompress_bytes("gzip", &compressed).unwrap(), b"hello");
    assert_eq!(
        compress_bytes("brotli", b"hello").unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
}

#[test]
fn loads_and_calls_a_simple_function() {
    assert_eq!(load("function add(a, b) { return a + b; }"), 1);
    assert_eq!(call("add", "[2, 3]"), "5");
}

#[test]
fn exposes_the_node_global_alias() {
    assert_eq!(
        load("function hasGlobal() { return global === globalThis; }"),
        1
    );
    assert_eq!(call("hasGlobal", "[]"), "true");
}

#[test]
fn process_env_reads_the_host_environment() {
    let path = std::env::var("PATH").unwrap();
    assert_eq!(load("function hostPath() { return process.env.PATH; }"), 1);
    assert_eq!(
        call("hostPath", "[]"),
        serde_json::to_string(&path).unwrap()
    );
}

#[test]
fn console_methods_remain_callable_when_detached() {
    assert_eq!(
        load("function detachedConsoleLog() { const log = console.log; log('detached'); return true; }") ,
        1
    );
    assert_eq!(call("detachedConsoleLog", "[]"), "true");
}

#[test]
fn standalone_event_loop_runs_pending_timers() {
    assert_eq!(load("globalThis.loopValue = 0; setTimeout(() => { loopValue = 42; }, 0); function readLoopValue() { return loopValue; }"), 1);
    thaw_js_run_event_loop();
    assert_eq!(call("readLoopValue", "[]"), "42");
}

#[test]
fn standalone_event_loop_ignores_only_unreferenced_timers() {
    assert_eq!(
        load(
            "globalThis.unrefValue = 0; globalThis.unrefTimer = __thaw_set_timeout_ref(() => { unrefValue = 42; }, 0, false); function readUnrefValue() { return unrefValue; } function refTimer() { __thaw_set_timer_ref(unrefTimer, true); }"
        ),
        1
    );
    thaw_js_run_event_loop();
    assert_eq!(call("readUnrefValue", "[]"), "0");
    assert_eq!(call("refTimer", "[]"), "null");
    thaw_js_run_event_loop();
    assert_eq!(call("readUnrefValue", "[]"), "42");
}

#[test]
fn round_trips_objects_and_arrays() {
    assert_eq!(load("function identity(x) { return x; }"), 1);
    assert_eq!(
        call("identity", r#"[{"a": 1, "b": [true, "x"]}]"#),
        r#"{"a":1,"b":[true,"x"]}"#
    );
}

#[test]
fn dynamic_json_boundary_round_trips_binary_values_as_buffers() {
    assert_eq!(
        load(
            "function binaryResult() {\n\
               const buffer = new ArrayBuffer(3);\n\
               new Uint8Array(buffer).set([1, 2, 3]);\n\
               return { buffer, view: new Uint8Array(buffer, 1, 2) };\n\
             }\n\
             function inspectBinary(value) {\n\
               return [Buffer.isBuffer(value.buffer), Array.from(value.buffer), Buffer.isBuffer(value.view), Array.from(value.view)];\n\
             }"
        ),
        1
    );
    let encoded = call("binaryResult", "[]");
    assert_eq!(
        call("inspectBinary", &format!("[{encoded}]")),
        "[true,[1,2,3],true,[2,3]]"
    );
}

#[test]
fn resolves_a_returned_promise() {
    assert_eq!(
        load("function later(x) { return Promise.resolve(x * 2); }"),
        1
    );
    assert_eq!(call("later", "[21]"), "42");
}

#[test]
fn timeout_resolves_promises_and_forwards_arguments() {
    assert_eq!(
        load(
            "function timed(value) { return new Promise(resolve => {\n\
                   setTimeout((left, right) => resolve(left + right), 2, value, 2);\n\
                 }); }"
        ),
        1
    );
    assert_eq!(call("timed", "[40]"), "42");
}

#[test]
fn timeout_can_be_cancelled() {
    assert_eq!(
        load(
            "function cancelled() { return new Promise(resolve => {\n\
                   const id = setTimeout(() => resolve('wrong'), 0);\n\
                   clearTimeout(id);\n\
                   setTimeout(() => resolve('right'), 1);\n\
                 }); }"
        ),
        1
    );
    assert_eq!(call("cancelled", "[]"), r#""right""#);
}

#[test]
fn immediate_forwards_arguments_and_can_be_cancelled() {
    assert_eq!(
        load(
            "function immediate(value) { return new Promise(resolve => {\n\
                   const cancelled = setImmediate(() => resolve('wrong'));\n\
                   clearImmediate(cancelled);\n\
                   setImmediate((left, right) => resolve(left + right), value, 2);\n\
                 }); }"
        ),
        1
    );
    assert_eq!(call("immediate", "[40]"), "42");
}

#[test]
fn interval_repeats_and_can_cancel_itself() {
    assert_eq!(
        load(
            "function countIntervals() { return new Promise(resolve => {\n\
                   let count = 0;\n\
                   const id = setInterval(() => {\n\
                     if (++count === 3) { clearInterval(id); resolve(count); }\n\
                   }, 1);\n\
                 }); }"
        ),
        1
    );
    assert_eq!(call("countIntervals", "[]"), "3");
}

#[test]
fn microtasks_run_before_zero_delay_timers() {
    assert_eq!(
        load(
            "function eventOrder() { return new Promise(resolve => {\n\
                   const events = [];\n\
                   setTimeout(() => { events.push('timer'); resolve(events); }, 0);\n\
                   queueMicrotask(() => events.push('microtask'));\n\
                   events.push('sync');\n\
                 }); }"
        ),
        1
    );
    assert_eq!(call("eventOrder", "[]"), r#"["sync","microtask","timer"]"#);
}

#[test]
fn process_next_tick_is_async_and_forwards_arguments() {
    assert_eq!(
            load(
                "function tickOrder() { return new Promise(resolve => {\n\
                   const events = ['sync'];\n\
                   process.nextTick((left, right) => { events.push(left + right); resolve(events); }, 'next', 'Tick');\n\
                   events.push('after');\n\
                 }); }"
            ),
            1
        );
    assert_eq!(call("tickOrder", "[]"), r#"["sync","after","nextTick"]"#);
}

#[test]
fn process_exposes_time_cwd_and_event_helpers() {
    assert_eq!(
            load(
                "function processHelpers() {\n\
                   const original = process.cwd(); process.chdir('/tmp/app'); const changed = process.cwd(); process.chdir(original);\n\
                   const warnings = []; const removed = () => warnings.push('removed');\n\
                   process.on('warning', removed); process.off('warning', removed);\n\
                   process.once('warning', warning => warnings.push(warning.name + ':' + warning.code + ':' + warning.message));\n\
                   process.emitWarning('careful', { type: 'ThawWarning', code: 'THAW001' });\n\
                   process.emitWarning('ignored by once listener');\n\
                   const start = process.hrtime(); const elapsed = process.hrtime(start); const memory = process.memoryUsage();\n\
                   return [changed, process.cwd() === original, warnings.join(','), process.listenerCount('warning'), process.uptime() >= 0, start.length, elapsed[0] >= 0, elapsed[1] >= 0, typeof process.hrtime.bigint() === 'bigint', memory.rss, process.cpuUsage().user, process.title, process.stdin.fd, process.stdout.fd, process.stderr.fd, typeof process.stderr.write];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("processHelpers", "[]"),
        r#"["/tmp/app",true,"ThawWarning:THAW001:careful",0,true,2,true,true,true,0,0,"thaw",0,1,2,"function"]"#
    );
}

#[test]
fn error_prepare_stack_trace_receives_call_sites() {
    assert_eq!(
        load(
            "function structuredStack() { const original = Error.prepareStackTrace; Error.prepareStackTrace = (_error, frames) => frames; const holder = {}; Error.captureStackTrace(holder); Error.prepareStackTrace = original; const frame = holder.stack[0]; return [typeof frame.getFileName, typeof frame.getLineNumber, typeof frame.getFunctionName, typeof frame.toString]; }"
        ),
        1
    );
    assert_eq!(
        call("structuredStack", "[]"),
        r#"["function","function","function","function"]"#
    );
}

#[test]
fn console_formats_groups_counts_and_writes_to_streams() {
    assert_eq!(
            load(
                "function consoleHelpers() {\n\
                   const stdout = []; const stderr = []; const instance = new Console({ write(value) { stdout.push(value); } }, { write(value) { stderr.push(value); } });\n\
                   instance.log('%s:%d:%j:%%', 'value', 4, { ok: true }); instance.group('group'); instance.log('child'); instance.groupEnd();\n\
                   instance.count('item'); instance.count('item'); instance.countReset('item'); instance.count('item'); instance.assert(false, 'bad %s', 'value'); instance.trace('trace-value');\n\
                   return [stdout.join(''), stderr[0], stderr[1].startsWith('Trace: trace-value\\n'), typeof console.log, console instanceof Console];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("consoleHelpers", "[]"),
        r#"["value:4:{\"ok\":true}:%\ngroup\n  child\nitem: 1\nitem: 2\nitem: 1\n","Assertion failed: bad value\n",true,"function",true]"#
    );
}

#[test]
fn base64_globals_round_trip_latin1_and_validate_input() {
    assert_eq!(
            load(
                "function base64() {\n\
                   let invalid = false;\n\
                   try { atob('%%%'); } catch (error) { invalid = error instanceof DOMException && error.name === 'InvalidCharacterError' && error.code === DOMException.INVALID_CHARACTER_ERR; }\n\
                   let invalidUnicode = false;\n\
                   try { btoa('雪'); } catch (error) { invalidUnicode = error instanceof DOMException && error.name === 'InvalidCharacterError'; }\n\
                   return [btoa('hello\\u00ff'), atob('aGVs bG//\\n'), invalid, invalidUnicode];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("base64", "[]"), r#"["aGVsbG//","helloÿ",true,true]"#);
}

#[test]
fn performance_exposes_time_origin_and_monotonic_elapsed_time() {
    assert_eq!(
        load(
            "function timing() {\n\
                   const first = performance.now();\n\
                   const second = performance.now();\n\
                   return [typeof performance.timeOrigin, first >= 0, second >= first];\n\
                 }"
        ),
        1
    );
    assert_eq!(call("timing", "[]"), r#"["number",true,true]"#);
}

#[test]
fn performance_timeline_marks_measures_and_observes_entries() {
    assert_eq!(
            load(
                "async function performanceTimeline() {\n\
                   performance.clearMarks(); performance.clearMeasures();\n\
                   performance.mark('start', { startTime: 10, detail: { id: 1 } });\n\
                   const observed = []; const observer = new PerformanceObserver(list => observed.push(...list.getEntries().map(entry => entry.name)));\n\
                   observer.observe({ type: 'mark', buffered: true }); performance.mark('end', { startTime: 25 }); await Promise.resolve();\n\
                   const measure = performance.measure('work', { start: 'start', end: 'end', detail: 'detail' });\n\
                   const wrapped = performance.timerify(function double(value) { return value * 2; }); const result = wrapped(4);\n\
                   const asyncWrapped = performance.timerify(async function later(value) { await Promise.resolve(); return value + 1; }); const asyncResult = await asyncWrapped(4);\n\
                   observer.disconnect(); const marks = performance.getEntriesByType('mark'); const functions = performance.getEntriesByType('function');\n\
                   const beforeClear = performance.getEntriesByName('work', 'measure').length; performance.clearMarks('start'); performance.clearMeasures();\n\
                   return [observed.join(','), marks.map(entry => entry.name).join(','), marks[0].detail.id, measure.startTime, measure.duration, measure.detail, result, asyncResult, functions.map(entry => entry.name).sort().join(','), beforeClear, performance.getEntriesByName('start').length, PerformanceObserver.supportedEntryTypes.join(',')];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("performanceTimeline", "[]"),
        r#"["start,end","start,end",1,10,15,"detail",8,5,"double,later",1,0,"function,mark,measure"]"#
    );
}

#[test]
fn structured_clone_copies_cycles_and_builtins() {
    assert_eq!(
            load(
                "function cloneValues() {\n\
                   const source = { nested: { value: 1 }, bytes: new Uint8Array([2, 3]), map: new Map([['key', 4]]), set: new Set([5]), date: new Date(6) };\n\
                   source.self = source;\n\
                   const copy = structuredClone(source);\n\
                   copy.nested.value = 7; copy.bytes[0] = 8;\n\
                   let rejected = false;\n\
                   try { structuredClone(() => 1); } catch (error) { rejected = error instanceof DOMException && error.name === 'DataCloneError' && error.code === DOMException.DATA_CLONE_ERR; }\n\
                   return [copy !== source, copy.self === copy, source.nested.value, source.bytes[0], copy.map.get('key'), copy.set.has(5), copy.date.getTime(), rejected];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("cloneValues", "[]"), "[true,true,1,2,4,true,6,true]");
}

#[test]
fn structured_clone_transfer_detaches_array_buffers() {
    assert_eq!(
            load(
                "function transferBuffer() {\n\
                   const source = new ArrayBuffer(4); new Uint8Array(source).set([1, 2, 3, 4]);\n\
                   const copy = structuredClone(source, { transfer: [source] });\n\
                   let duplicate = false, invalid = false; const other = new ArrayBuffer(1);\n\
                   try { structuredClone(other, { transfer: [other, other] }); } catch (error) { duplicate = error.name === 'DataCloneError'; }\n\
                   try { structuredClone({}, { transfer: [{}] }); } catch (error) { invalid = error.name === 'DataCloneError'; }\n\
                   return [source.byteLength, copy.byteLength, Array.from(new Uint8Array(copy)), duplicate, invalid, other.byteLength];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("transferBuffer", "[]"), "[0,4,[1,2,3,4],true,true,1]");
}

#[test]
fn message_port_transfer_detaches_the_source_port() {
    assert_eq!(
            load(
                "async function transferPort() {\n\
                   const channel = new MessageChannel(), carrier = new MessageChannel();\n\
                   const received = new Promise(resolve => { carrier.port2.onmessage = event => resolve([event.data.port, event.ports]); });\n\
                   carrier.port1.postMessage({ port: channel.port1 }, [channel.port1]);\n\
                   const [port, ports] = await received;\n\
                   const delivered = new Promise(resolve => { channel.port2.onmessage = event => resolve(event.data); });\n\
                   channel.port1.postMessage('ignored'); port.postMessage('through');\n\
                   const value = await delivered; carrier.port1.close(); carrier.port2.close(); port.close(); channel.port2.close();\n\
                   return [channel.port1.__thawClosed, value, ports[0] === port];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("transferPort", "[]"), r#"[true,"through",true]"#);
}

#[test]
fn url_search_params_preserves_duplicates_and_iterates() {
    assert_eq!(
            load(
                "function queryParams() {\n\
                   const params = new URLSearchParams('?b=two+words&a=1&a=2');\n\
                   params.append('snow', '雪');\n\
                   const before = [params.get('a'), params.getAll('a').join(','), params.has('a', '2'), params.size];\n\
                   params.set('a', 3); params.delete('b', 'wrong'); params.sort();\n\
                   const visited = []; params.forEach((value, key, owner) => visited.push(key + ':' + value + ':' + (owner === params)));\n\
                   const record = new URLSearchParams({ x: 1, y: false });\n\
                   const pairs = new URLSearchParams([['z', 4], ['z', 5]]); pairs.delete('z', 4);\n\
                   return [before, params.toString(), [...params.keys()].join(','), visited.join('|'), record.toString(), pairs.toString()];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("queryParams", "[]"),
        r#"[["1","1,2",true,4],"a=3&b=two+words&snow=%E9%9B%AA","a,b,snow","a:3:true|b:two words:true|snow:雪:true","x=1&y=false","z=5"]"#
    );
}

#[test]
fn worker_transport_codec_preserves_graphs_and_builtins() {
    assert_eq!(
            load(
                "function transportValues() {\n\
                   const buffer = new ArrayBuffer(4), bytes = new Uint8Array(buffer, 1, 2); bytes.set([7, 8]);\n\
                   const source = { map: new Map([[1, 'one']]), set: new Set([2]), date: new Date(3), pattern: /x/gi, buffer, bytes, large: 9n, missing: undefined }; source.self = source;\n\
                   const copy = __thaw_worker_decode(__thaw_worker_encode(source));\n\
                   return [copy.self === copy, copy.map.get(1), copy.set.has(2), copy.date.getTime(), copy.pattern.flags, copy.buffer === copy.bytes.buffer, copy.bytes.byteOffset, Array.from(copy.bytes), String(copy.large), copy.missing === undefined];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("transportValues", "[]"),
        r#"[true,"one",true,3,"gi",true,1,[7,8],"9",true]"#
    );
}

#[test]
fn url_resolves_relative_paths_and_synchronizes_search_params() {
    assert_eq!(
            load(
                "function urls() {\n\
                   const url = new URL('../next?x=1#part', 'https://User:Pass@Example.COM:443/a/b/file');\n\
                   url.searchParams.append('x', 2);\n\
                   url.searchParams.set('space', 'two words');\n\
                   const first = [url.href, url.origin, url.protocol, url.username, url.password, url.host, url.hostname, url.port, url.pathname, url.search, url.hash, url.toJSON()];\n\
                   url.search = '?fresh=yes';\n\
                   const oldParamsDetached = url.searchParams.get('x') === null && url.searchParams.get('fresh') === 'yes';\n\
                   url.host = 'Other.test:8080'; url.pathname = 'root/./child/../end'; url.hash = 'done';\n\
                   return [first, oldParamsDetached, url.href, URL.canParse('/ok', 'https://example.test'), URL.canParse('/bad'), URL.parse('bad')];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("urls", "[]"),
        r##"[["https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part","https://example.com","https:","User","Pass","example.com","example.com","","/a/next","?x=1&x=2&space=two+words","#part","https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part"],true,"https://User:Pass@other.test:8080/root/end?fresh=yes#done",true,false,null]"##
    );
}

#[test]
fn event_target_dispatches_listeners_and_cancellation() {
    assert_eq!(
            load(
                "function dispatchEvents() {\n\
                   const target = new EventTarget();\n\
                   const events = [];\n\
                   const removed = () => events.push('removed');\n\
                   target.addEventListener('work', removed);\n\
                   target.removeEventListener('work', removed);\n\
                   target.addEventListener('work', event => { events.push(event.target === target && event.currentTarget === target); event.preventDefault(); }, { once: true });\n\
                   target.addEventListener('work', { handleEvent(event) { events.push('object'); event.stopImmediatePropagation(); } });\n\
                   target.addEventListener('work', () => events.push('late'));\n\
                   const first = target.dispatchEvent(new Event('work', { cancelable: true }));\n\
                   const second = target.dispatchEvent(new Event('work', { cancelable: true }));\n\
                   return [events.join(','), first, second];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("dispatchEvents", "[]"),
        r#"["true,object,object",false,true]"#
    );
}

#[test]
fn abort_controller_dispatches_once_and_timeout_aborts() {
    assert_eq!(
            load(
                "async function abortSignals() {\n\
                   const controller = new AbortController();\n\
                   const events = [];\n\
                   controller.signal.onabort = event => events.push(event instanceof Event ? 'property' : 'wrong');\n\
                   controller.signal.addEventListener('abort', () => events.push('once'), { once: true });\n\
                   controller.abort('reason'); controller.abort('ignored');\n\
                   let thrown = '';\n\
                   try { controller.signal.throwIfAborted(); } catch (error) { thrown = error; }\n\
                   const timed = AbortSignal.timeout(0);\n\
                   await new Promise(resolve => timed.addEventListener('abort', resolve));\n\
                   const first = new AbortController();\n\
                   const second = new AbortController();\n\
                   const combined = AbortSignal.any([first.signal, second.signal]);\n\
                   second.abort('combined');\n\
                   const preAborted = AbortSignal.any([AbortSignal.abort('pre')]);\n\
                   const defaultAbort = AbortSignal.abort();\n\
                   let invalidAny = false;\n\
                   try { AbortSignal.any([first.signal, {}]); } catch (error) { invalidAny = error instanceof TypeError; }\n\
                   return [events.join(','), controller.signal instanceof EventTarget, controller.signal.reason, thrown, timed.aborted, timed.reason.name, timed.reason.code, combined.aborted, combined.reason, preAborted.reason, defaultAbort.reason.name, defaultAbort.reason.code, invalidAny];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("abortSignals", "[]"),
        r#"["property,once",true,"reason","reason",true,"TimeoutError",23,true,"combined","pre","AbortError",20,true]"#
    );
}

#[test]
fn text_encoder_and_decoder_support_utf8() {
    assert_eq!(
            load(
                "function utf8RoundTrip(value) {\n\
                   const encoder = new TextEncoder();\n\
                   const encoded = encoder.encode(value);\n\
                   return { bytes: Array.from(encoded), text: new TextDecoder().decode(encoded) };\n\
                 }\n\
                 function encodeInto() {\n\
                   const output = new Uint8Array(5);\n\
                   const result = new TextEncoder().encodeInto('A😀B', output);\n\
                   return { result, bytes: Array.from(output) };\n\
                 }\n\
                 function decodeInvalid() {\n\
                   return new TextDecoder().decode(Uint8Array.from([0x61, 0xff, 0x62]));\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("utf8RoundTrip", r#"["A😀"]"#),
        r#"{"bytes":[65,240,159,152,128],"text":"A😀"}"#
    );
    assert_eq!(
        call("encodeInto", "[]"),
        r#"{"result":{"read":3,"written":5},"bytes":[65,240,159,152,128]}"#
    );
    assert_eq!(call("decodeInvalid", "[]"), r#""a�b""#);
}

#[test]
fn text_decoder_streaming_retains_incomplete_utf8() {
    assert_eq!(
            load(
                "function decodeStreaming() {\n\
                   const decoder = new TextDecoder();\n\
                   const first = decoder.decode(Uint8Array.from([0xf0, 0x9f]), { stream: true });\n\
                   const second = decoder.decode(Uint8Array.from([0x98, 0x80, 0x41]), { stream: true });\n\
                   const last = decoder.decode();\n\
                   const invalid = new TextDecoder();\n\
                   const invalidFirst = invalid.decode(Uint8Array.from([0xe2, 0x82]), { stream: true });\n\
                   const invalidSecond = invalid.decode(Uint8Array.from([0x41]), { stream: true });\n\
                   const incomplete = new TextDecoder();\n\
                   incomplete.decode(Uint8Array.from([0xe2]), { stream: true });\n\
                   const flushed = incomplete.decode();\n\
                   let fatal = false; const strict = new TextDecoder('utf-8', { fatal: true });\n\
                   strict.decode(Uint8Array.from([0xf0]), { stream: true });\n\
                   try { strict.decode(); } catch (error) { fatal = error instanceof TypeError; }\n\
                   const bom = new TextDecoder();\n\
                   const bomFirst = bom.decode(Uint8Array.from([0xef, 0xbb]), { stream: true });\n\
                   const bomSecond = bom.decode(Uint8Array.from([0xbf, 0x42]), { stream: true });\n\
                   return [first, second, last, invalidFirst, invalidSecond, flushed, fatal, bomFirst, bomSecond];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("decodeStreaming", "[]"),
        r#"["","😀A","","","�A","�",true,"","B"]"#
    );
}

#[test]
fn buffer_supports_encodings_views_search_and_numeric_access() {
    assert_eq!(
            load(
                "function buffers() {\n\
                   const utf8 = Buffer.from('雪ab'); const hex = Buffer.from('00ff10', 'hex'); const base64 = Buffer.from('aGk=', 'base64');\n\
                   const shared = utf8.slice(3); shared[0] = 65;\n\
                   const allocated = Buffer.alloc(5, 'xy'); allocated.write('Z', 2);\n\
                   const numeric = Buffer.alloc(4); numeric.writeUInt16LE(0x1234, 0); numeric.writeUInt16BE(0x5678, 2);\n\
                   const joined = Buffer.concat([base64, Buffer.from('!')]);\n\
                   return [Buffer.isBuffer(utf8), Buffer.isEncoding('base64url'), Buffer.byteLength('雪'), utf8.toString(), hex.toString('base64url'), joined.toString(), allocated.toString(), allocated.indexOf('y'), allocated.lastIndexOf('x'), allocated.includes('Z'), numeric.readUInt16LE(0), numeric.readUInt16BE(2), Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from(utf8.toJSON()).equals(utf8), shared.buffer === utf8.buffer];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("buffers", "[]"),
        r#"[true,true,3,"雪Ab","AP8Q","hi!","xyZyx",1,4,true,4660,22136,-1,true,true]"#
    );
}

#[test]
fn webassembly_compiles_instantiates_and_exposes_numeric_memory_and_global_values() {
    assert_eq!(
            load(
                "async function wasmFoundation() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (memory (export \"memory\") 1 2)\n\
                     (global (export \"counter\") (mut i32) (i32.const 5))\n\
                     (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)\n\
                     (func (export \"add64\") (param i64 i64) (result i64) local.get 0 local.get 1 i64.add)\n\
                     (func (export \"pair\") (result i32 i32) i32.const 3 i32.const 4)\n\
                     (func (export \"read0\") (result i32) i32.const 0 i32.load8_u)\n\
                     (func (export \"write0\") (param i32) i32.const 0 local.get 0 i32.store8))`);\n\
                   const module = new WebAssembly.Module(source);\n\
                   const instance = new WebAssembly.Instance(module);\n\
                   const view = new Uint8Array(instance.exports.memory.buffer); view[0] = 41;\n\
                   const read = instance.exports.read0(); instance.exports.write0(99);\n\
                   const refreshed = new Uint8Array(instance.exports.memory.buffer)[0];\n\
                   const oldGlobal = instance.exports.counter.value; instance.exports.counter.value = 12;\n\
                   const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 }); const oldBuffer = memory.buffer;\n\
                   const previous = memory.grow(1);\n\
                   const standaloneGlobal = new WebAssembly.Global({ value: 'i64', mutable: true }, 7n); standaloneGlobal.value = 9n;\n\
                   const table = new WebAssembly.Table({ element: 'externref', initial: 1, maximum: 2 }, 'a'); const tablePrevious = table.grow(1, 'b');\n\
                   const compiled = await WebAssembly.compile(source);\n\
                   const asyncResult = await WebAssembly.instantiate(source);\n\
                   const customModule = new WebAssembly.Module(new TextEncoder().encode(`(module (@custom \"meta\" \"one\") (@custom \"meta\" \"two\"))`));\n\
                   const custom = WebAssembly.Module.customSections(customModule, 'meta').map(value => new TextDecoder().decode(value));\n\
                   const streamed = await WebAssembly.compileStreaming(new Response(source, { headers: { 'Content-Type': 'application/wasm; charset=binary' } }));\n\
                   let mimeError = false; try { await WebAssembly.compileStreaming(new Response(source)); } catch (error) { mimeError = error instanceof TypeError; }\n\
                   let compileError = false; try { new WebAssembly.Module(new Uint8Array([0])); } catch (error) { compileError = error instanceof WebAssembly.CompileError; }\n\
                   return [WebAssembly.validate(source), WebAssembly.validate(new Uint8Array([0])), WebAssembly.Module.exports(module).map(x => x.name).sort(), WebAssembly.Module.imports(module), instance.exports.add.length, instance.exports.add(20, 22), instance.exports.add64(40n, 2n).toString(), instance.exports.pair(), read, refreshed, oldGlobal, instance.exports.counter.value, previous, oldBuffer.byteLength, memory.buffer.byteLength, standaloneGlobal.value.toString(), tablePrevious, table.length, table.get(1), compiled instanceof WebAssembly.Module, asyncResult.instance.exports.add(1, 2), custom, streamed instanceof WebAssembly.Module, mimeError, compileError];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmFoundation", "[]"),
        r#"[true,false,["add","add64","counter","memory","pair","read0","write0"],[],2,42,"42",[3,4],41,99,5,12,1,0,131072,"9",1,2,"b",true,3,["one","two"],true,true,true]"#
    );
}

#[test]
fn webassembly_calls_javascript_function_imports_with_scalar_and_multi_values() {
    assert_eq!(
            load(
                "function wasmImports() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"host\" \"twice\" (func $twice (param i32) (result i32)))\n\
                     (import \"host\" \"add64\" (func $add64 (param i64 i64) (result i64)))\n\
                     (import \"host\" \"pair\" (func $pair (param f64) (result f64 f64)))\n\
                     (import \"host\" \"notify\" (func $notify (param i32)))\n\
                     (import \"host\" \"fail\" (func $fail))\n\
                     (func (export \"run\") (param i32) (result i32) local.get 0 call $notify local.get 0 call $twice)\n\
                     (func (export \"wide\") (result i64) i64.const 20 i64.const 22 call $add64)\n\
                     (func (export \"many\") (result f64 f64) f64.const 3.5 call $pair)\n\
                     (func (export \"explode\") call $fail))`);\n\
                   const calls = []; const module = new WebAssembly.Module(source);\n\
                   const instance = new WebAssembly.Instance(module, { host: { twice(value) { return value * 2; }, add64(left, right) { return left + right; }, pair(value) { return [value, value + 0.5]; }, notify(value) { calls.push(value); }, fail() { throw new Error('import boom'); } } });\n\
                   let trapped = false; try { instance.exports.explode(); } catch (error) { trapped = error instanceof WebAssembly.RuntimeError && error.message.includes('import boom'); }\n\
                   let missing = false; try { new WebAssembly.Instance(module, {}); } catch (error) { missing = error instanceof WebAssembly.LinkError; }\n\
                   return [instance.exports.run(21), calls, instance.exports.wide().toString(), instance.exports.many(), trapped, missing, WebAssembly.Module.imports(module).length];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmImports", "[]"),
        r#"[42,[21],"42",[3.5,4],true,true,5]"#
    );
}

#[test]
fn webassembly_javascript_function_imports_preserve_externrefs() {
    assert_eq!(
            load(
                "function wasmImportExternRefs() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"host\" \"echo\" (func $echo (param externref) (result externref)))\n\
                     (import \"host\" \"pair\" (func $pair (param externref) (result externref externref)))\n\
                     (import \"host\" \"empty\" (func $empty (result externref)))\n\
                     (func (export \"run\") (param externref) (result externref) local.get 0 call $echo)\n\
                     (func (export \"many\") (param externref) (result externref externref) local.get 0 call $pair)\n\
                     (func (export \"none\") (result externref) call $empty))`);\n\
                   const first = { id: 1 }, second = [2], seen = [];\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { host: {\n\
                     echo(value) { seen.push(value); return value; },\n\
                     pair(value) { return [value, second]; },\n\
                     empty() { return null; }\n\
                   } });\n\
                   const echoed = instance.exports.run(first), many = instance.exports.many(first), missing = instance.exports.run(undefined);\n\
                   return [seen[0] === first, echoed === first, many[0] === first, many[1] === second, seen[1] === undefined, missing === undefined, instance.exports.none() === null];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmImportExternRefs", "[]"),
        r#"[true,true,true,true,true,true,true]"#
    );
}

#[test]
fn webassembly_synchronizes_imported_memory_and_mutable_globals() {
    assert_eq!(
            load(
                "function wasmImportedState() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"env\" \"memory\" (memory 1 2))\n\
                     (import \"env\" \"counter\" (global $counter (mut i32)))\n\
                     (func (export \"read\") (result i32) i32.const 0 i32.load8_u)\n\
                     (func (export \"write\") (param i32) i32.const 0 local.get 0 i32.store8)\n\
                     (func (export \"bump\") (result i32) global.get $counter i32.const 1 i32.add global.set $counter global.get $counter)\n\
                     (func (export \"grow\") (result i32) i32.const 1 memory.grow))`);\n\
                   const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 }), old = memory.buffer; new Uint8Array(memory.buffer)[0] = 10;\n\
                   const counter = new WebAssembly.Global({ value: 'i32', mutable: true }, 5);\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { memory, counter } });\n\
                   const sibling = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { memory, counter } });\n\
                   const before = instance.exports.read(); instance.exports.write(33); const written = new Uint8Array(memory.buffer)[0], siblingRead = sibling.exports.read();\n\
                   const first = instance.exports.bump(), reflected = counter.value, siblingBump = sibling.exports.bump(); counter.value = 20; const second = instance.exports.bump();\n\
                   const previous = instance.exports.grow(), detached = old.byteLength, length = memory.buffer.byteLength; new Uint8Array(memory.buffer)[65536] = 77;\n\
                   return [before, written, siblingRead, first, reflected, siblingBump, second, counter.value, previous, detached, length, instance.exports.read()];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmImportedState", "[]"),
        r#"[10,33,33,6,6,7,21,21,1,0,131072,33]"#
    );
}

#[test]
fn webassembly_externref_preserves_javascript_identity() {
    assert_eq!(
            load(
                "function wasmExternRefs() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"env\" \"shared\" (global $shared (mut externref)))\n\
                     (func (export \"echo\") (param externref) (result externref) local.get 0)\n\
                     (func (export \"get\") (result externref) global.get $shared)\n\
                     (func (export \"set\") (param externref) local.get 0 global.set $shared))`);\n\
                   const first = { name: 'first' }, second = [1, 2], shared = new WebAssembly.Global({ value: 'externref', mutable: true }, first);\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { shared } });\n\
                   const initial = instance.exports.get() === first, echoed = instance.exports.echo(second) === second, undefinedValue = instance.exports.echo(undefined) === undefined, nullValue = instance.exports.echo(null) === null;\n\
                   instance.exports.set(second); const reflected = shared.value === second; shared.value = first; const reset = instance.exports.get() === first;\n\
                   return [initial, echoed, undefinedValue, nullValue, reflected, reset];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmExternRefs", "[]"),
        r#"[true,true,true,true,true,true]"#
    );
}

#[test]
fn webassembly_externref_handles_are_reused_for_repeated_values() {
    assert_eq!(
            load(
                "function wasmExternRefReuse() {\n\
                   const source = new TextEncoder().encode(`(module (func (export \"echo\") (param externref) (result externref) local.get 0))`);\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source)), object = { stable: true }, text = 'stable externref';\n\
                   instance.exports.echo(object); instance.exports.echo(text); instance.exports.echo(undefined);\n\
                   const seeded = JSON.parse(__thaw_wasm_reference_stats());\n\
                   for (let index = 0; index < 1000; index++) {\n\
                     if (instance.exports.echo(object) !== object || instance.exports.echo(text) !== text || instance.exports.echo(undefined) !== undefined) return false;\n\
                   }\n\
                   const final = JSON.parse(__thaw_wasm_reference_stats()); return final.values === seeded.values && final.storeExternrefs === seeded.storeExternrefs;\n\
                 }"
            ),
            1
        );
    assert_eq!(call("wasmExternRefReuse", "[]"), "true");
}

#[test]
fn webassembly_dispose_releases_resources_and_reactivates_cached_values() {
    assert_eq!(
            load(
                "function wasmDispose() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"host\" \"increment\" (func $increment (param i32) (result i32)))\n\
                     (func (export \"call\") (param i32) (result i32) local.get 0 call $increment)\n\
                     (func (export \"echo\") (param externref) (result externref) local.get 0))`);\n\
                   const before = JSON.parse(__thaw_wasm_reference_stats()), object = { reusable: true };\n\
                   const module = new WebAssembly.Module(source), instance = new WebAssembly.Instance(module, { host: { increment: value => value + 1 } });\n\
                   const callable = instance.exports.call, first = instance.exports.echo(object) === object && callable(4) === 5;\n\
                   const active = JSON.parse(__thaw_wasm_reference_stats()); instance.dispose(); instance.dispose(); module.dispose(); module.dispose();\n\
                   const released = JSON.parse(__thaw_wasm_reference_stats()); let invalid = false; try { callable(1); } catch (error) { invalid = error instanceof WebAssembly.RuntimeError; }\n\
                   const module2 = new WebAssembly.Module(source), instance2 = new WebAssembly.Instance(module2, { host: { increment: value => value + 1 } });\n\
                   const reactivated = instance2.exports.echo(object) === object; instance2.dispose(); module2.dispose();\n\
                   const final = JSON.parse(__thaw_wasm_reference_stats());\n\
                   return [first, active.modules === before.modules + 1, active.instances === before.instances + 1, active.imports === before.imports + 1, active.values === before.values + 1, released.modules === before.modules, released.instances === before.instances, released.imports === before.imports, released.values === before.values, invalid, reactivated, final.modules === before.modules, final.instances === before.instances, final.imports === before.imports, final.values === before.values];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmDispose", "[]"),
        r#"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]"#
    );
}

#[test]
fn webassembly_dispose_detaches_imported_resource_bindings() {
    assert_eq!(
            load(
                "function wasmDisposeBindings() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"env\" \"memory\" (memory 1 2))\n\
                     (import \"env\" \"counter\" (global (mut i32)))\n\
                     (import \"env\" \"items\" (table 1 2 externref))\n\
                     (func (export \"noop\")))`);\n\
                   const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 }), counter = new WebAssembly.Global({ value: 'i32', mutable: true }, 1), items = new WebAssembly.Table({ element: 'externref', initial: 1, maximum: 2 });\n\
                   const module = new WebAssembly.Module(source), instance = new WebAssembly.Instance(module, { env: { memory, counter, items } }); instance.dispose(); module.dispose();\n\
                   const previousMemory = memory.grow(1); counter.value = 9; items.set(0, 'detached'); const previousTable = items.grow(1, 'grown');\n\
                   return [previousMemory, memory.buffer.byteLength, counter.value, items.get(0), previousTable, items.length];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmDisposeBindings", "[]"),
        r#"[1,131072,9,"detached",1,2]"#
    );
}

#[test]
fn webassembly_finalizers_release_unreachable_modules_and_instances() {
    assert_eq!(
            load(
                "function wasmCreateGarbage() {\n\
                   globalThis.__thawWasmGcBaseline = JSON.parse(__thaw_wasm_reference_stats());\n\
                   const source = new TextEncoder().encode(`(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))`), module = new WebAssembly.Module(source), instance = new WebAssembly.Instance(module); return instance.exports.add(1, 2) === 3;\n\
                 }\n\
                 async function wasmGcRelease() {\n\
                   for (let index = 0; index < 4; index++) { __thaw_gc(); await Promise.resolve(); }\n\
                   const after = JSON.parse(__thaw_wasm_reference_stats()), before = globalThis.__thawWasmGcBaseline; return [after.modules === before.modules, after.instances === before.instances];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("wasmCreateGarbage", "[]"), "true");
    assert_eq!(call("wasmGcRelease", "[]"), r#"[true,true]"#);
}

#[test]
fn webassembly_link_failures_release_pending_imports_and_externrefs() {
    assert_eq!(
            load(
                "function wasmFailedLinks() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"host\" \"callback\" (func))\n\
                     (import \"host\" \"value\" (global externref))\n\
                     (import \"host\" \"memory\" (memory 2 2)))`), module = new WebAssembly.Module(source);\n\
                   const callback = () => {}, object = { pending: true }, value = new WebAssembly.Global({ value: 'externref' }, object), memory = new WebAssembly.Memory({ initial: 1, maximum: 2 }), before = JSON.parse(__thaw_wasm_reference_stats());\n\
                   let failures = 0; for (let index = 0; index < 20; index++) { try { new WebAssembly.Instance(module, { host: { callback, value, memory } }); } catch (error) { if (error instanceof WebAssembly.LinkError) failures++; } }\n\
                   const after = JSON.parse(__thaw_wasm_reference_stats()); module.dispose(); return [failures, after.imports === before.imports, after.values === before.values, after.instances === before.instances];\n\
                 }"
            ),
            1
        );
    assert_eq!(call("wasmFailedLinks", "[]"), r#"[20,true,true,true]"#);
}

#[test]
fn webassembly_imported_externref_tables_share_values_and_growth() {
    assert_eq!(
            load(
                "function wasmTables() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (import \"env\" \"items\" (table 2 4 externref))\n\
                     (export \"items\" (table 0))\n\
                     (func (export \"get\") (param i32) (result externref) local.get 0 table.get)\n\
                     (func (export \"set\") (param i32 externref) local.get 0 local.get 1 table.set)\n\
                     (func (export \"grow\") (param externref) (result i32) local.get 0 i32.const 1 table.grow))`);\n\
                   const first = { id: 1 }, second = { id: 2 }, third = { id: 3 }, table = new WebAssembly.Table({ element: 'externref', initial: 2, maximum: 4 }, first), module = new WebAssembly.Module(source);\n\
                   const instance = new WebAssembly.Instance(module, { env: { items: table } }), sibling = new WebAssembly.Instance(module, { env: { items: table } });\n\
                   const initial = instance.exports.get(0) === first && instance.exports.items.get(1) === first; instance.exports.set(1, second);\n\
                   const reflected = table.get(1) === second, siblingValue = sibling.exports.get(1) === second, previous = instance.exports.grow(third);\n\
                   const grown = table.length === 3 && table.get(2) === third && sibling.exports.items.length === 3; table.set(0, third); const reverse = instance.exports.get(0) === third;\n\
                   return [initial, reflected, siblingValue, previous, grown, reverse, instance.exports.items instanceof WebAssembly.Table];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmTables", "[]"),
        r#"[true,true,true,2,true,true,true]"#
    );
}

#[test]
fn webassembly_funcref_tables_return_and_accept_exported_functions() {
    assert_eq!(
            load(
                "function wasmFunctionTables() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (type $unary (func (param i32) (result i32)))\n\
                     (func $increment (export \"increment\") (type $unary) local.get 0 i32.const 1 i32.add)\n\
                     (func $double (export \"double\") (type $unary) local.get 0 i32.const 2 i32.mul)\n\
                     (table (export \"functions\") 2 4 funcref)\n\
                     (elem (i32.const 0) $increment $double)\n\
                     (func (export \"invoke\") (param i32 i32) (result i32) local.get 1 local.get 0 call_indirect (type $unary)))`);\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source)), table = instance.exports.functions;\n\
                   const first = table.get(0), stable = first === table.get(0), direct = first(9);\n\
                   table.set(0, instance.exports.double); const replaced = instance.exports.invoke(0, 6);\n\
                   const previous = table.grow(1, instance.exports.increment), grown = instance.exports.invoke(2, 8);\n\
                   table.set(1, null); const empty = table.get(1) === null; let trapped = false;\n\
                   try { instance.exports.invoke(1, 1); } catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }\n\
                   const sibling = new WebAssembly.Instance(new WebAssembly.Module(source)); table.set(0, sibling.exports.double); const foreign = instance.exports.invoke(0, 7);\n\
                   return [table instanceof WebAssembly.Table, stable, direct, replaced, previous, table.length, grown, empty, trapped, foreign];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmFunctionTables", "[]"),
        r#"[true,true,10,12,2,3,9,true,true,14]"#
    );
}

#[test]
fn webassembly_imported_funcref_tables_synchronize_same_instance_functions() {
    assert_eq!(
            load(
                "function wasmImportedFunctionTable() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (type $unary (func (param i32) (result i32)))\n\
                     (import \"env\" \"functions\" (table 1 3 funcref))\n\
                     (func $square (export \"square\") (type $unary) local.get 0 local.get 0 i32.mul)\n\
                     (func $triple (type $unary) local.get 0 i32.const 3 i32.mul)\n\
                     (elem declare func $triple)\n\
                     (func (export \"install\") i32.const 0 ref.func $triple table.set)\n\
                     (func (export \"invoke\") (param i32 i32) (result i32) local.get 1 local.get 0 call_indirect (type $unary)))`);\n\
                   const table = new WebAssembly.Table({ element: 'funcref', initial: 1, maximum: 3 });\n\
                   const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { functions: table } });\n\
                   table.set(0, instance.exports.square); const square = instance.exports.invoke(0, 5), stable = table.get(0) === instance.exports.square;\n\
                   instance.exports.install(); const internal = table.get(0), triple = internal(7), indirect = instance.exports.invoke(0, 4);\n\
                   const previous = table.grow(1, instance.exports.square), grown = instance.exports.invoke(1, 6);\n\
                   let invalid = false; try { table.set(0, {}); } catch (error) { invalid = error instanceof TypeError; }\n\
                   return [square, stable, typeof internal, triple, indirect, previous, table.length, grown, invalid];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmImportedFunctionTable", "[]"),
        r#"[25,true,"function",21,12,1,2,36,true]"#
    );
}

#[test]
fn webassembly_shared_funcref_tables_forward_functions_between_stores() {
    assert_eq!(
            load(
                "function wasmSharedFunctionTable() {\n\
                   const source = new TextEncoder().encode(`(module\n\
                     (type $unary (func (param i32) (result i32)))\n\
                     (import \"env\" \"functions\" (table 1 2 funcref))\n\
                     (func (export \"square\") (type $unary) local.get 0 local.get 0 i32.mul)\n\
                     (func (export \"invoke\") (param i32) (result i32) local.get 0 i32.const 0 call_indirect (type $unary)))`);\n\
                   const module = new WebAssembly.Module(source), table = new WebAssembly.Table({ element: 'funcref', initial: 1, maximum: 2 });\n\
                   const first = new WebAssembly.Instance(module, { env: { functions: table } }), second = new WebAssembly.Instance(module, { env: { functions: table } });\n\
                   table.set(0, first.exports.square); const forwarded = second.exports.invoke(5), firstIdentity = table.get(0) === first.exports.square;\n\
                   table.set(0, second.exports.square); const reverse = first.exports.invoke(6), secondIdentity = table.get(0) === second.exports.square;\n\
                   const seeded = new WebAssembly.Table({ element: 'funcref', initial: 1, maximum: 2 }, first.exports.square);\n\
                   const third = new WebAssembly.Instance(module, { env: { functions: seeded } }); const initial = third.exports.invoke(7), initialIdentity = seeded.get(0) === first.exports.square;\n\
                   return [forwarded, firstIdentity, reverse, secondIdentity, initial, initialIdentity];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("wasmSharedFunctionTable", "[]"),
        r#"[25,true,36,true,49,true]"#
    );
}

#[test]
fn crypto_hash_hmac_random_and_webcrypto_are_available() {
    assert_eq!(
            load(
                "async function cryptoHelpers() {\n\
                   const sha256 = __thaw_crypto_module.createHash('sha-256').update('a').copy().update('bc').digest('hex');\n\
                   const sha512 = __thaw_crypto_module.createHash('sha512').update('abc').digest('hex');\n\
                   const hmac = __thaw_crypto_module.createHmac('sha256', 'key').update('The quick brown fox jumps over the lazy dog').digest('hex');\n\
                   const random = __thaw_crypto_module.randomBytes(12); const filled = new Uint8Array(8); const same = crypto.getRandomValues(filled) === filled;\n\
                   const callbackValue = await new Promise((resolve, reject) => __thaw_crypto_module.randomBytes(5, (error, value) => error ? reject(error) : resolve(value.length)));\n\
                   const integer = await new Promise((resolve, reject) => __thaw_crypto_module.randomInt(10, 20, (error, value) => error ? reject(error) : resolve(value)));\n\
                   const webDigest = Buffer.from(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc'))).toString('hex');\n\
                   const uuid = crypto.randomUUID();\n\
                   return [sha256, sha512, hmac, random.length, same, filled.length, callbackValue, integer >= 10 && integer < 20, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid), __thaw_crypto_module.timingSafeEqual(Buffer.from('same'), Buffer.from('same')), webDigest, __thaw_crypto_module.getHashes()];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("cryptoHelpers", "[]"),
        r#"["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f","f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",12,true,8,5,true,true,true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",["sha256","sha512"]]"#
    );
}

#[test]
fn retains_and_calls_a_callable_javascript_value() {
    assert_eq!(
            load("globalThis.times = factor => value => Promise.resolve(value * factor); globalThis.twice = times(2);"),
            1
        );
    let name = CString::new("twice").unwrap();
    let handle = thaw_js_get_global(name.as_ptr());
    assert_ne!(handle, 0);
    let args = CString::new("[21]").unwrap();
    let result = thaw_js_call_handle_result(handle, args.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(
        unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
        "42"
    );
}

#[test]
fn assimilates_foreign_thenables_once_and_reports_then_errors() {
    assert_eq!(
            load(
                "function thenable() {\n\
                   return { then(resolve, reject) { resolve(42); reject('late'); resolve(99); } };\n\
                 }\n\
                 function throwingThen() {\n\
                   return Object.create(null, { then: { get() { throw new Error('bad then'); } } });\n\
                 }"
            ),
            1
        );
    assert_eq!(call("thenable", "[]"), "42");
    let failed = call("throwingThen", "[]");
    assert!(failed.contains("__thaw_error__"), "{failed}");
    assert!(failed.contains("bad then"), "{failed}");
}

#[test]
fn unknown_function_reports_an_error_object_instead_of_crashing() {
    let result = call("doesNotExist", "[]");
    assert!(
        result.contains("__thaw_error__"),
        "unexpected result: {result}"
    );
}

#[test]
fn thrown_exception_reports_an_error_object_instead_of_crashing() {
    assert_eq!(load("function boom() { throw new Error('kaboom'); }"), 1);
    let result = call("boom", "[]");
    assert!(
        result.contains("__thaw_error__"),
        "unexpected result: {result}"
    );
    assert!(result.contains("kaboom"), "unexpected result: {result}");
}

#[test]
fn result_abi_separates_success_from_javascript_exceptions() {
    assert_eq!(
        load("function ok() { return 42; } function boom() { throw new Error('kaboom'); }"),
        1
    );
    let ok_name = CString::new("ok").unwrap();
    let boom_name = CString::new("boom").unwrap();
    let args = CString::new("[]").unwrap();
    let ok = thaw_js_call_result(ok_name.as_ptr(), args.as_ptr());
    assert!(ok.error.is_null());
    assert_eq!(unsafe { CStr::from_ptr(ok.value) }.to_str().unwrap(), "42");

    let failed = thaw_js_call_result(boom_name.as_ptr(), args.as_ptr());
    assert!(failed.value.is_null());
    let error = unsafe { CStr::from_ptr(failed.error) }.to_string_lossy();
    assert!(error.contains("kaboom"), "{error}");
}

#[test]
fn syntax_error_fails_to_load_instead_of_crashing() {
    assert_eq!(load("function( this is not valid js"), 0);
}

#[test]
fn hpack_huffman_host_functions_match_the_rfc_vector() {
    assert_eq!(
            load(
                "function encodeHpack() { return __thaw_hpack_huffman_encode(Buffer.from('www.example.com').toString('hex')); }\n\
                 function decodeHpack() { return Buffer.from(__thaw_hpack_huffman_decode('f1e3c2e5f23a6ba0ab90f4ff'), 'hex').toString(); }",
            ),
            1
        );
    assert_eq!(call("encodeHpack", "[]"), r#""f1e3c2e5f23a6ba0ab90f4ff""#);
    assert_eq!(call("decodeHpack", "[]"), r#""www.example.com""#);
}

/// Regression test for the pre-fix behavior: a thrown exception during
/// `loadScript` used to report only `rquickjs::Error::Exception`'s
/// generic placeholder message on stderr, not the real thrown message
/// -- undiagnosable for e.g. a real npm package's top-level
/// `require(...)` call. `load_impl` is the testable inner function
/// (`thaw_js_load` itself only returns 0/1, with the message going to
/// stderr).
#[test]
fn load_failure_reports_the_real_thrown_message() {
    with_context(|ctx| {
        let err = load_impl(ctx, "throw new Error('boom');").unwrap_err();
        assert!(err.starts_with("boom"), "{err}");
    });
}
