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
    assert_eq!(thaw_js_run_event_loop(), 0);
    assert_eq!(call("readLoopValue", "[]"), "42");
}

#[test]
fn timer_exceptions_reach_the_process_event_surface() {
    assert_eq!(
        load("globalThis.timerError = ''; process.once('uncaughtException', error => { timerError = error.message; }); setTimeout(() => { throw new Error('timer boom'); }, 0);"),
        1
    );
    assert_eq!(thaw_js_run_event_loop(), 0);
    assert_eq!(eval_json("timerError"), Ok(Some("\"timer boom\"".into())));
}

#[test]
fn unhandled_timer_exception_fails_the_event_loop() {
    assert_eq!(
        load("setTimeout(() => { throw new Error('timer fatal'); }, 0);"),
        1
    );
    assert_eq!(thaw_js_run_event_loop(), 1);
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
    // `refTimer` has no explicit `return`, so its real result is a
    // genuine JS `undefined` -- reported as the same napi-undefined
    // sentinel every other genuinely-undefined dynamic-call result now
    // marshals as, not bare `null` (see `result_abi_marshals_a_
    // genuinely_undefined_return_value_distinctly_from_null`).
    assert_eq!(
        call("refTimer", "[]"),
        r#"{"$__thaw_napi_undefined$":true}"#
    );
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
fn dynamic_json_boundary_omits_cycles_without_dropping_repeated_values() {
    assert_eq!(
        load(
            "function cyclicResult() { const shared = { n: 1 }; const root = { a: shared, b: shared }; root.self = root; return root; }"
        ),
        1
    );
    assert_eq!(call("cyclicResult", "[]"), r#"{"a":{"n":1},"b":{"n":1}}"#);
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
                   const original = process.cwd(); process.chdir('/tmp'); const changed = process.cwd(); process.chdir(original);\n\
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
        r#"["/tmp",true,"ThawWarning:THAW001:careful",0,true,2,true,true,true,0,0,"thaw",0,1,2,"function"]"#
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
                   const numeric32 = Buffer.alloc(8); numeric32.writeInt32BE(-123456, 0); numeric32.writeUInt32LE(0xfedcba98, 4);\n\
                   const floating = Buffer.alloc(16); floating.writeFloatLE(1.5, 0); floating.writeDoubleBE(-2.25, 4);\n\
                   const joined = Buffer.concat([base64, Buffer.from('!')]);\n\
                   return [Buffer.isBuffer(utf8), Buffer.isEncoding('base64url'), Buffer.byteLength('雪'), utf8.toString(), hex.toString('base64url'), joined.toString(), allocated.toString(), allocated.indexOf('y'), allocated.lastIndexOf('x'), allocated.includes('Z'), numeric.readUInt16LE(0), numeric.readUInt16BE(2), numeric32.readInt32BE(0), numeric32.readUInt32LE(4), floating.readFloatLE(0), floating.readDoubleBE(4), Buffer.from([0, 1, 2, 3]).swap16().toString('hex'), Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from(utf8.toJSON()).equals(utf8), shared.buffer === utf8.buffer, Object.keys(Buffer).includes('from'), Buffer.from([0xff]).readInt8(), Buffer.from([0xff, 0xfe]).readInt16BE()];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("buffers", "[]"),
        r#"[true,true,3,"雪Ab","AP8Q","hi!","xyZyx",1,4,true,4660,22136,-123456,4275878552,1.5,-2.25,"01000302",-1,true,true,true,-1,-2]"#
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
                   const hmacKey = await crypto.subtle.importKey('raw', new TextEncoder().encode('key'), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);\n\
                   const webHmac = Buffer.from(await crypto.subtle.sign('HMAC', hmacKey, new TextEncoder().encode('The quick brown fox jumps over the lazy dog'))).toString('hex');\n\
                   const passwordKey = await crypto.subtle.importKey('raw', new TextEncoder().encode('password'), 'PBKDF2', false, ['deriveBits']);\n\
                   const derived = Buffer.from(await crypto.subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt: new TextEncoder().encode('salt'), iterations: 1 }, passwordKey, 256)).toString('hex');\n\
                   const uuid = crypto.randomUUID();\n\
                   return [sha256, sha512, hmac, random.length, same, filled.length, callbackValue, integer >= 10 && integer < 20, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid), __thaw_crypto_module.timingSafeEqual(Buffer.from('same'), Buffer.from('same')), webDigest, webHmac, derived, __thaw_crypto_module.getHashes()];\n\
                 }"
            ),
            1
        );
    assert_eq!(
        call("cryptoHelpers", "[]"),
        r#"["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f","f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",12,true,8,5,true,true,true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8","120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",["sha256","sha512"]]"#
    );
}

#[test]
fn crypto_scrypt_sync_matches_node() {
    assert_eq!(
        load(
            "function scryptVector() { return __thaw_crypto_module.scryptSync('password', 'salt', 8, { N: 16 }).toString('hex'); }"
        ),
        1
    );
    assert_eq!(call("scryptVector", "[]"), r#""f876178f94837d87""#);
}

#[test]
fn crypto_aes_256_cbc_round_trips() {
    assert_eq!(
        load(
            "function cipherRoundTrip() { const key = Buffer.alloc(32, 1), iv = Buffer.alloc(16, 2), cipher = __thaw_crypto_module.createCipheriv('aes-256-cbc', key, iv), encrypted = Buffer.concat([cipher.update('hello'), cipher.final()]), decipher = __thaw_crypto_module.createDecipheriv('aes-256-cbc', key, iv); return Buffer.concat([decipher.update(encrypted), decipher.final()]).toString(); } function cipherStreaming() { const cipher = __thaw_crypto_module.createCipheriv('aes-256-cbc', Buffer.alloc(32), Buffer.alloc(16)), first = cipher.update('abcdefghijklmnopqrstuvwxyz012345'), last = cipher.final(); return [first.length, last.length, first.length + last.length]; }"
        ),
        1
    );
    assert_eq!(call("cipherRoundTrip", "[]"), r#""hello""#);
    assert_eq!(call("cipherStreaming", "[]"), "[32,16,48]");
}

/// `KeyObject`/`createSecretKey`/`createPrivateKey`/`createPublicKey`,
/// added for jsonwebtoken's own real-world sign/verify flow: real
/// `sign.js`/`verify.js` both do `secret instanceof KeyObject`, then (for
/// any plain string/Buffer secret) normalize it via a `createPrivateKey`/
/// `createPublicKey` attempt that's *expected* to fail for a symmetric
/// (HMAC) secret, falling back to `createSecretKey`. A transitive
/// dependency (`jwa`) separately feature-detects `typeof
/// crypto.createPublicKey === 'function'` before trusting *any*
/// `KeyObject` at all -- including the symmetric one this shim supports --
/// so `createPrivateKey`/`createPublicKey` must exist and be callable
/// (honestly throwing, since this shim has no RSA/ECDSA/PEM support at
/// all) even though nothing here successfully calls them for the
/// symmetric case.
#[test]
fn crypto_key_object_and_secret_key_support_hmac_normalization() {
    assert_eq!(
        load(
            "function keyObjectHelpers() {\n\
               const secretKey = __thaw_crypto_module.createSecretKey(Buffer.from('key'));\n\
               const isKeyObject = secretKey instanceof __thaw_crypto_module.KeyObject;\n\
               const type = secretKey.type;\n\
               const viaHmac = __thaw_crypto_module.createHmac('sha256', secretKey).update('The quick brown fox jumps over the lazy dog').digest('hex');\n\
               const supportsKeyObjects = typeof __thaw_crypto_module.createPublicKey === 'function';\n\
               let privateKeyThrew = false;\n\
               try { __thaw_crypto_module.createPrivateKey('not-a-real-key'); } catch (error) { privateKeyThrew = true; }\n\
               return [isKeyObject, type, viaHmac, supportsKeyObjects, privateKeyThrew];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("keyObjectHelpers", "[]"),
        r#"[true,"secret","f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",true,true]"#
    );
}

/// RSA and ECDSA (P-256) `createSign`/`createVerify`/`crypto.sign`/
/// `crypto.verify`, real support added in place of the previous "no
/// RSA/ECDSA/EC support at all" boundary (see the fixed doc comments
/// above). Covers: RSA PKCS1v15 (default) and RSA-PSS padding, both
/// PKCS#8 and legacy PKCS#1 RSA private-key PEM, both PKCS#8 and SEC1
/// EC private-key PEM, a tampered signature/message failing
/// verification, and the one-shot `crypto.sign`/`crypto.verify`
/// functions -- plus, most importantly, **verifying a signature
/// produced by real OpenSSL** (`openssl dgst -sha256 -sign ...`/
/// `-sigopt rsa_padding_mode:pss`) against the exact same key and
/// message, over both RSA forms and ECDSA, confirming real
/// interoperability rather than just internal round-trip consistency.
#[test]
fn crypto_rsa_and_ecdsa_sign_and_verify_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function rsaPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\\nMIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCwwf4NV+7sbU0z\\nwaL5V1ABx9epdw571f4wNuNcic8x7CRchIv4DuDlKrHsI6T0y8+pKMdq4rHrDeEt\\neeTylrgK7HnqKDe31b8Cxr7BkDLlandEmP3/VPQu7EFDN70rF68IdMn/y9ywbpwY\\nNENCIyjyRkfraM5Lr5sBVKjfma8Kfu1PdoE3iMmJX2zhQXQjlnCVAEvelZfSRh64\\nTmBv8XoB3dp93HQmRlvpuEObVi6p0wViMYueSaYFviqJONtnwu9xRVBiXXg9qKz6\\nsDsRwBuNQTgmCpAXsgwcKmIjl1pvEAroln+LTB3HL20aIJvTD1doJ4Wq3N/17Z5p\\ni4jouRUNAgMBAAECgf9ClNB1KqSVMFEkbdIeyNZgMmzKQE6XS36dE4Q/6NfXtkSf\\nCWX14gH38+jXSn6vzrj71qCNYp0LFmO2FOjGBCL9+mdPJCiTBYJcjy9XaNzaazp2\\nXIgLFJ3mbAxvKGcfQumxqKmdCJCqWV0KY+s3vobLGUV+EDDrF286i0ZulqnHZtAO\\nkGGTXsptC86gZHaRYsIuZ0US5KYC1h1tGmwbISdvmfwaXm7N5XW2qU5W3VUZYx4Q\\nwjiZ+JJ5RddKYM8TIvK6+AGPqcLdqJxDp/rr5jb0MVJsv+m9BkWRVDSBvnp+zE+b\\nJWvUMRc3FeMwqxYAzWVpfsUFUDuc/94fuL6Z68kCgYEA6zuo1Jbe8yEFd8KwJvEj\\nN6S7OYmaud9wtKfLTJLa0OmkiXqd88Lw1Mf8uvb4TBP+IOS0XwJYL/R2dmBix4TY\\neMesaK1h9twsbKp86sq2bDL5SbT/cWLI6xpgV27vnYIcXKHdtO48FbUrYriTf1sb\\n8XYGKAxygcjYewYiLAow6MkCgYEAwFzGDj2HntZaLvHAi/10d7CMhF1bLKNK2l/Q\\n46TW+xjAjlAToQ1T+2gfhEvTQ0b+nLJt/ld/FqTogT/3TKpoFG3YWlLZ8ruoPOXY\\nxBhL/kxZLchmHN45hzXkd91lOlRLf9TsfHxsYEkOU56kkpIw9zSbHxHbTgUgpwM+\\nB5qT8CUCgYB9uuyZfG58O1kl0uy+U8MEGctsjI0j7jbaiJkUO6YzZb5pMR29zaNV\\nx/Lgp+K9Hy6EvFlgMuuZ7itnSEtj4zClFeykIpArFzGzf0i3YlQw7unpqJGkNC25\\n4+Y8tXHjmUi5hlbvPyrkW2puIMPNnZAI9pGB1G1by1NSJkwbh/LuaQKBgCBB7nyI\\n2OtL6seghrdzA0rm8kloFlf/8hd4peDmzZ5B4lh7GS+SupiYN2DKDl1j1GKWkVdr\\neMZlVRAHmALlOJrkaLmM1zubOHUt3hHUOTolt3az+luw8Fi6Mtve5pDHffmrzRR7\\nEPl8hsiC+/oQReHOkoy9Q9driLQ5GPfRdil5AoGAX64TOs4kpbzM045OgM65Cm2t\\nW+SreofkSv2ErT4dzAoPhs0x1xX6BUDNCqn74einRfr1NBJj/llkbMiD8QAHJNq2\\npqS4IynQWgf4hYsJlrvG0F79WWadArLVQNLxebGxThzx6l+mIm1wGn/xHibbYBB2\\nJz01Ou8GC/69JgjMjas=\\n-----END PRIVATE KEY-----\\n'; }\n\
             function rsaPrivatePkcs1() { return '-----BEGIN RSA PRIVATE KEY-----\\nMIIEoAIBAAKCAQEAsMH+DVfu7G1NM8Gi+VdQAcfXqXcOe9X+MDbjXInPMewkXISL\\n+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4Cux56ig3t9W/Asa+wZAy5Wp3RJj9\\n/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo8kZH62jOS6+bAVSo35mvCn7tT3aB\\nN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6Ad3afdx0JkZb6bhDm1YuqdMFYjGL\\nnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAbjUE4JgqQF7IMHCpiI5dabxAK6JZ/\\ni0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkVDQIDAQABAoH/QpTQdSqklTBRJG3S\\nHsjWYDJsykBOl0t+nROEP+jX17ZEnwll9eIB9/Po10p+r864+9agjWKdCxZjthTo\\nxgQi/fpnTyQokwWCXI8vV2jc2ms6dlyICxSd5mwMbyhnH0LpsaipnQiQqlldCmPr\\nN76GyxlFfhAw6xdvOotGbpapx2bQDpBhk17KbQvOoGR2kWLCLmdFEuSmAtYdbRps\\nGyEnb5n8Gl5uzeV1tqlOVt1VGWMeEMI4mfiSeUXXSmDPEyLyuvgBj6nC3aicQ6f6\\n6+Y29DFSbL/pvQZFkVQ0gb56fsxPmyVr1DEXNxXjMKsWAM1laX7FBVA7nP/eH7i+\\nmevJAoGBAOs7qNSW3vMhBXfCsCbxIzekuzmJmrnfcLSny0yS2tDppIl6nfPC8NTH\\n/Lr2+EwT/iDktF8CWC/0dnZgYseE2HjHrGitYfbcLGyqfOrKtmwy+Um0/3FiyOsa\\nYFdu752CHFyh3bTuPBW1K2K4k39bG/F2BigMcoHI2HsGIiwKMOjJAoGBAMBcxg49\\nh57WWi7xwIv9dHewjIRdWyyjStpf0OOk1vsYwI5QE6ENU/toH4RL00NG/pyybf5X\\nfxak6IE/90yqaBRt2FpS2fK7qDzl2MQYS/5MWS3IZhzeOYc15HfdZTpUS3/U7Hx8\\nbGBJDlOepJKSMPc0mx8R204FIKcDPgeak/AlAoGAfbrsmXxufDtZJdLsvlPDBBnL\\nbIyNI+422oiZFDumM2W+aTEdvc2jVcfy4KfivR8uhLxZYDLrme4rZ0hLY+MwpRXs\\npCKQKxcxs39It2JUMO7p6aiRpDQtuePmPLVx45lIuYZW7z8q5FtqbiDDzZ2QCPaR\\ngdRtW8tTUiZMG4fy7mkCgYAgQe58iNjrS+rHoIa3cwNK5vJJaBZX//IXeKXg5s2e\\nQeJYexkvkrqYmDdgyg5dY9RilpFXa3jGZVUQB5gC5Tia5Gi5jNc7mzh1Ld4R1Dk6\\nJbd2s/pbsPBYujLb3uaQx335q80UexD5fIbIgvv6EEXhzpKMvUPXa4i0ORj30XYp\\neQKBgF+uEzrOJKW8zNOOToDOuQptrVvkq3qH5Er9hK0+HcwKD4bNMdcV+gVAzQqp\\n++Hop0X69TQSY/5ZZGzIg/EAByTatqakuCMp0FoH+IWLCZa7xtBe/VlmnQKy1UDS\\n8XmxsU4c8epfpiJtcBp/8R4m22AQdic9NTrvBgv+vSYIzI2r\\n-----END RSA PRIVATE KEY-----\\n'; }\n\
             function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsMH+DVfu7G1NM8Gi+VdQ\\nAcfXqXcOe9X+MDbjXInPMewkXISL+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4\\nCux56ig3t9W/Asa+wZAy5Wp3RJj9/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo\\n8kZH62jOS6+bAVSo35mvCn7tT3aBN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6\\nAd3afdx0JkZb6bhDm1YuqdMFYjGLnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAb\\njUE4JgqQF7IMHCpiI5dabxAK6JZ/i0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkV\\nDQIDAQAB\\n-----END PUBLIC KEY-----\\n'; }\n\
             function ecPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgHqV2FCeyxUa9ivDr\\nlBxTfnDLAsoI5yqk63+Gu/wHFFehRANCAARuk+iU6lIfCrZ55skHVPDCLJokBINo\\nHetn8HlbmnNJwQvHw0cTM3BelUyYVXZkLPW5kCLhhNiRpRgGOm+3vLOI\\n-----END PRIVATE KEY-----\\n'; }\n\
             function ecPrivateSec1() { return '-----BEGIN EC PRIVATE KEY-----\\nMHcCAQEEIB6ldhQnssVGvYrw65QcU35wywLKCOcqpOt/hrv8BxRXoAoGCCqGSM49\\nAwEHoUQDQgAEbpPolOpSHwq2eebJB1TwwiyaJASDaB3rZ/B5W5pzScELx8NHEzNw\\nXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\\n-----END EC PRIVATE KEY-----\\n'; }\n\
             function ecPublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEbpPolOpSHwq2eebJB1TwwiyaJASD\\naB3rZ/B5W5pzScELx8NHEzNwXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\\n-----END PUBLIC KEY-----\\n'; }\n\
             function rsaRoundTrip() {\n\
               const sig = __thaw_crypto_module.createSign('RSA-SHA256').update('hello world').sign(rsaPrivatePkcs8(), 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('RSA-SHA256').update('hello world').verify(rsaPublicKey(), sig, 'hex');\n\
               const okPkcs1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(rsaPrivatePkcs1(), 'hex'), 'hex');\n\
               const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(rsaPublicKey(), sig, 'hex');\n\
               const tamperedSignature = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), sig.slice(0, -2) + '00', 'hex');\n\
               const opensslSig = 'a7e86b1a31fe823a693f5fedd193ebfb60098de3e41bac389e9e52215d785ee5cc3d211560be5dd87cbadbc264bdea375f96e87420f71b30891352d9a7631e2e1d42d0f8422a4f9af640b3a41617d754e7bdc1738436ac81fa79f6455ed431ab9221671d9582687bf3999ecf618085426bcc3a41c69ec89d0aedcbfaa835d37a89e215859d85d4509afbfb22b569318e779193764f63b9dabfb11c5360b08c2bfcfe257ab5f08f1438f397b2bf284f46848c8ff660ead19e35c67727fef275baf9d91fe56f7a9544f04dcc1ed1152bf737c17a141aaca770fdf4241cef76141722d8910a178b8ca3f183502d49a0c8cf51443e2a797b4ee6ad2be49aea9c9e40';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), opensslSig, 'hex');\n\
               return [ok, okPkcs1Key, tamperedMessage, tamperedSignature, opensslVerifies];\n\
             }\n\
             function rsaPssRoundTrip() {\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign({ key: rsaPrivatePkcs8(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, sig, 'hex');\n\
               const opensslSig = '37ceb5072ae254254115b5910f7a8e45723f1a6ca203eb2084074ab4f95672a0c9d6db585f5bed06cef0208bc5ed474ae0177422928739b05eb40ea4dc2da6204bb73d3e88174f32e01cb94b21b317615e61b8df7e561303261f0868ff704013b20f976c8a77ab5a802f15348b0969a35e57802cd7068f7a386481831c43f19bb511882e5163368ef25f93f73bf47f4374483772dddbb8ca2c03defdb5c4fca89549fea202fab7bf5ca82fc55953119781219ff67b58689866466f2c2bddb4b2f152244b8562fd5e13e3e5e087137e5835cd53978b10f4d4dd37e57ee4aff83dafc1973b0600a780938bddc559bd57b46cdc099895fbd284963a67fe0e9d120b';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, opensslSig, 'hex');\n\
               return [ok, opensslVerifies];\n\
             }\n\
             function ecdsaRoundTrip() {\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivatePkcs8(), 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), sig, 'hex');\n\
               const okSec1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivateSec1(), 'hex'), 'hex');\n\
               const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(ecPublicKey(), sig, 'hex');\n\
               const opensslSig = '304502210091b5f1192a836c1d9582197cab6ceac74a952a794d1276781e8f1ac1cf9a001b02203c65bd65ec0b91e24d1f5b52ccaa466e28748e2e56b944d5e7a9aec1bc712210';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), opensslSig, 'hex');\n\
               return [ok, okSec1Key, tamperedMessage, opensslVerifies];\n\
             }\n\
             function oneShotSignVerify() {\n\
               const sig = __thaw_crypto_module.sign('sha256', 'one shot', rsaPrivatePkcs8());\n\
               return __thaw_crypto_module.verify('sha256', 'one shot', rsaPublicKey(), sig);\n\
             }\n\
             function keyObjectAsymmetricInfo() {\n\
               const priv = __thaw_crypto_module.createPrivateKey(rsaPrivatePkcs8());\n\
               const pub = __thaw_crypto_module.createPublicKey(rsaPublicKey());\n\
               const ecPub = __thaw_crypto_module.createPublicKey(ecPublicKey());\n\
               return [priv.type, priv.asymmetricKeyType, pub.type, pub.asymmetricKeyType, ecPub.asymmetricKeyType, ecPub.asymmetricKeyDetails.namedCurve];\n\
             }"
        ),
        1
    );
    assert_eq!(call("rsaRoundTrip", "[]"), "[true,true,false,false,true]");
    assert_eq!(call("rsaPssRoundTrip", "[]"), "[true,true]");
    assert_eq!(call("ecdsaRoundTrip", "[]"), "[true,true,false,true]");
    assert_eq!(call("oneShotSignVerify", "[]"), "true");
    assert_eq!(
        call("keyObjectAsymmetricInfo", "[]"),
        r#"["private","rsa","public","rsa","ec","P-256"]"#
    );
}

/// P-521 (ES512) and Ed25519 (EdDSA) sign/verify -- the two algorithms
/// added to round out the ECDSA curve trio (P-256/P-384/P-521) and add
/// modern JWT's other common asymmetric scheme. Same methodology as
/// `crypto_rsa_and_ecdsa_sign_and_verify_interoperate_with_real_openssl`:
/// verifies a signature produced by real OpenSSL, not just internal
/// round-trip. Ed25519 ignores the `algorithm` argument entirely
/// (`'sha256'` here is just a placeholder -- real Node's own
/// `crypto.sign(null, data, key)` passes `null`), matching real EdDSA's
/// whole-message, no-external-digest design.
#[test]
fn crypto_p521_and_ed25519_sign_and_verify_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function ec521PrivateKey() { return '-----BEGIN PRIVATE KEY-----\\nMIHuAgEAMBAGByqGSM49AgEGBSuBBAAjBIHWMIHTAgEBBEIBjSW4dVBkRUfwm4RF\\nl+sHT3Bb+K3g3OK8rBc9FKUI7yseNOqjMJH4NWAA4COGQLrcrEZd8i3mC6PMiYlP\\nW2kM0rWhgYkDgYYABABj3B3pln7Wni4OZxl1rPfCXviip4IG30MzhbXxCmsdtWZW\\n9rs74AS0HzfD3JmxhpsyykdhymyRcCUTK6Dg0AMn4AFkkRuVJ6EH5naQbrW/gm1S\\nVhMwsFhDH8U4eFloVh5F224ipvr8StOkFbX7bc8SEAInT4mJoPOVhHIh3Nic5RPT\\nzw==\\n-----END PRIVATE KEY-----\\n'; }\n\
             function ec521PublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMIGbMBAGByqGSM49AgEGBSuBBAAjA4GGAAQAY9wd6ZZ+1p4uDmcZdaz3wl74oqeC\\nBt9DM4W18QprHbVmVva7O+AEtB83w9yZsYabMspHYcpskXAlEyug4NADJ+ABZJEb\\nlSehB+Z2kG61v4JtUlYTMLBYQx/FOHhZaFYeRdtuIqb6/ErTpBW1+23PEhACJ0+J\\niaDzlYRyIdzYnOUT088=\\n-----END PUBLIC KEY-----\\n'; }\n\
             function ed25519PrivateKey() { return '-----BEGIN PRIVATE KEY-----\\nMC4CAQAwBQYDK2VwBCIEIIQeFoclOMRan6+SUSRbRw78/TZyhWbyoWJcEbyE9MM+\\n-----END PRIVATE KEY-----\\n'; }\n\
             function ed25519PublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMCowBQYDK2VwAyEAbKI14Nm8FASaqC+aL64yTxj+nIFlCG8Ylhs1gQwQDls=\\n-----END PUBLIC KEY-----\\n'; }\n\
             function ec521RoundTrip() {\n\
               const sig = __thaw_crypto_module.createSign('sha512').update('hello world').sign(ec521PrivateKey(), 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha512').update('hello world').verify(ec521PublicKey(), sig, 'hex');\n\
               const tamperedMessage = __thaw_crypto_module.createVerify('sha512').update('goodbye world').verify(ec521PublicKey(), sig, 'hex');\n\
               const opensslSig = '30818702414b69e53677eca1ee5f0d40de0ad064064a480a1de9b0b9fe82e898778f705e56f57b8899e53b58d32603aee99f18c4921330487c339e1b3c8d05c0011c90122c1b02420091998e2df97d18f040bcca159eae7f27f3d046188801b92c88771e0dbd2fe74767b20bde678eb460f1819e30523b5c9f5ba1580b74a495acdf67b83009728d3225';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha512').update('hello world').verify(ec521PublicKey(), opensslSig, 'hex');\n\
               return [ok, tamperedMessage, opensslVerifies];\n\
             }\n\
             function ed25519RoundTrip() {\n\
               const sig = __thaw_crypto_module.sign('sha256', 'hello world', ed25519PrivateKey());\n\
               const ok = __thaw_crypto_module.verify('sha256', 'hello world', ed25519PublicKey(), sig);\n\
               const tamperedMessage = __thaw_crypto_module.verify('sha256', 'goodbye world', ed25519PublicKey(), sig);\n\
               const opensslSig = Buffer.from('34eb966388de794ba39493a2da457c263e716beea079da49cb8ce8cd544dfaf1bc494987096177c3edda654fe1fbb1efe34e8266eec20c2e17a69481ddc1780d', 'hex');\n\
               const opensslVerifies = __thaw_crypto_module.verify('sha256', 'hello world', ed25519PublicKey(), opensslSig);\n\
               return [ok, tamperedMessage, opensslVerifies];\n\
             }\n\
             function keyObjectAsymmetricInfoP521AndEd25519() {\n\
               const ecPriv = __thaw_crypto_module.createPrivateKey(ec521PrivateKey());\n\
               const edPub = __thaw_crypto_module.createPublicKey(ed25519PublicKey());\n\
               return [ecPriv.asymmetricKeyType, ecPriv.asymmetricKeyDetails.namedCurve, edPub.asymmetricKeyType];\n\
             }"
        ),
        1
    );
    assert_eq!(call("ec521RoundTrip", "[]"), "[true,false,true]");
    assert_eq!(call("ed25519RoundTrip", "[]"), "[true,false,true]");
    assert_eq!(
        call("keyObjectAsymmetricInfoP521AndEd25519", "[]"),
        r#"["ec","P-521","ed25519"]"#
    );
}

/// DER-format key input and passphrase-protected PKCS8 private keys --
/// the two remaining `node:crypto` key-import gaps. Same real-key
/// methodology as the other crypto tests: a fresh RSA keypair, signed
/// by real OpenSSL, verified through DER-imported and passphrase-
/// decrypted `KeyObject`s built from the *same* key material.
#[test]
fn crypto_der_and_passphrase_protected_key_import_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function rsaPublicPem() { return '-----BEGIN PUBLIC KEY-----\\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoORKaNK2ZuUTmkCb/Oqh\\n3vw+cO8OCUFJRz+nPYP9gYArPbwy8I3nJe2ft5rVHPTn8u4gzSVwQGBpQ5fYMx4g\\n80htGj3JD16v1DHg8qIGDnnk9MGGEHJ6DcdJsKiWK8SK2xEttirRTY5fHGQwkAWg\\nMDFIjzG6OIoGDCZyRH+1zY1xdP617629Z2UvNL5eB9A6FPHwpmSlgr8T/nHmqGeN\\nL6VJuC61pnZTehGm6hvUMdqPEm0ciiHmavx0DAaWqzVjLBb5jTssayMabrmRCUdq\\nTpe3g/MeBcdQ/LCwF/j0QKmgoL3gzWubFA8IO+e2M+AnkV25wbrVTV4ysvWlzdKK\\nxwIDAQAB\\n-----END PUBLIC KEY-----\\n'; }\n\
             function rsaPrivateDerHex() { return '308204be020100300d06092a864886f70d0101010500048204a8308204a40201000282010100a0e44a68d2b666e5139a409bfceaa1defc3e70ef0e094149473fa73d83fd81802b3dbc32f08de725ed9fb79ad51cf4e7f2ee20cd25704060694397d8331e20f3486d1a3dc90f5eafd431e0f2a2060e79e4f4c18610727a0dc749b0a8962bc48adb112db62ad14d8e5f1c64309005a03031488f31ba388a060c2672447fb5cd8d7174feb5efadbd67652f34be5e07d03a14f1f0a664a582bf13fe71e6a8678d2fa549b82eb5a676537a11a6ea1bd431da8f126d1c8a21e66afc740c0696ab35632c16f98d3b2c6b231a6eb99109476a4e97b783f31e05c750fcb0b017f8f440a9a0a0bde0cd6b9b140f083be7b633e027915db9c1bad54d5e32b2f5a5cdd28ac7020301000102820100061c457b2fad7fc0e97aad437f5a85e54b1d2ffad444a3b71dbe9c2268f5e2ca345a36e094643f48207b3564eafd1b8c079ce5a004f0fb70edee8440d0c82f262e34fe8f2428b246e93f2fb4e754658e5994b618da5d0ea7a14efa279cf472957776728efd974f63bdd6fd331ef527bd4cd1dda65cd532e0c1eb5fe19c1c127f6237172346eb71d7d5a74e5d0bfafbe7bd1de6da4d361c011100cc7881b2b824df573445e8a166938960942ee19ad27fd6e96da35b954341c1896f6e47ae27d6d4e8b10e21bdcc75ac8f2023866920271dd6b4dce35b11db1e68ac0f9c8901617ea9cb2790b6879417a8f85eb1eff48573eb31580c008e37fae17666bf677d9102818100dffe4d703426d74947f8bf6fd8cdfc709276efe554364ebed55bb74c2555d17c80765d9523426fce8d22082c3fdb0fe3b779b2839348caa9866e2a24c643a75f722618e0ab5d3db6ceec1704bb28b4809714b5cc14811e0412adee71c68ae4f098cfc03c71744288394ce6c4a5cd19b0039dc657e1d0e01d92ef55881aa5a4bf02818100b7e1b9c5782fe8f801f13c994244057e527c3d84e38c641e2c8c27cd33702b073a36077bc12824de002856493f0cf8f5fa20a4b641258b594ae878482414e924aa40e68e09146063a7467f4e3a82d4a2b3d2c3e8afa12135066b6979b40b90be457f8ce0f3f8f627bc1d7368f8e25b5b04af0e56107ee6c4e1da53acc9acf3f902818039a35689e8e195c465a0bca22b47d60da1a2b95869b30fd04b56ae7409a76ba07dedf766c90bef795717cac2982be68ad24b9e83fd025e24015397c49ec009f1a58de818e7ffb641b43d4c2f0b7a0df888e7eb5ff866c1328b1bf69f90576d51fc007997141ab684173a92a74782df794b74edf4ef46b064ebca6a57fb83644102818100a5a93bf776bf1b110c96ec745aa9f39509f51a6b75a18eb54c86fc78b765cfae143886e76c6ea1404c3e0af6b452189d6aba2c0a7288c3912f965e7f07dabaeca8620e145a83bc0f2badac95aacb218c6f9b6b9a5f583815907206b5798a8ddd8db94b0f835d814eed004f707c015a3296f6ab60c83dbbe41661decea56726e9028181009557356289cef177e2aeb16a98f4dae588341eeda0dd47565736836a0338ea0c70f87dc077fe2cf75e0529b2c8c4b07a28a3d8052ce434bcf8bf61bcd8d4c0c7015b629af00743c6cf69a1b868df316b5886986fa1c98a4626975a27660efc84e8b2c61b4a57b89bd36af4b11a4828b8baac12bebe1ae990cb59b81591f9442b'; }\n\
             function rsaPublicDerHex() { return '30820122300d06092a864886f70d01010105000382010f003082010a0282010100a0e44a68d2b666e5139a409bfceaa1defc3e70ef0e094149473fa73d83fd81802b3dbc32f08de725ed9fb79ad51cf4e7f2ee20cd25704060694397d8331e20f3486d1a3dc90f5eafd431e0f2a2060e79e4f4c18610727a0dc749b0a8962bc48adb112db62ad14d8e5f1c64309005a03031488f31ba388a060c2672447fb5cd8d7174feb5efadbd67652f34be5e07d03a14f1f0a664a582bf13fe71e6a8678d2fa549b82eb5a676537a11a6ea1bd431da8f126d1c8a21e66afc740c0696ab35632c16f98d3b2c6b231a6eb99109476a4e97b783f31e05c750fcb0b017f8f440a9a0a0bde0cd6b9b140f083be7b633e027915db9c1bad54d5e32b2f5a5cdd28ac70203010001'; }\n\
             function rsaPrivateEncryptedPem() { return '-----BEGIN ENCRYPTED PRIVATE KEY-----\\nMIIFNTBfBgkqhkiG9w0BBQ0wUjAxBgkqhkiG9w0BBQwwJAQQvq2l3eEnfnFD3GVd\\nhtvb+QICCAAwDAYIKoZIhvcNAgkFADAdBglghkgBZQMEASoEECBT8TeR7sQt2Vmo\\nNpQJ+i0EggTQDM28rgAAPQmRjennOGuBEspP8qH2Yv0tWH51YDJ2xu+FswPDs8H2\\n7hA+kQRHhR6U+vw4UM31iQofb0qZ+XSRtfYldZ/4qfk0ND5sTuL4wPzktA/BlCY3\\n98m7TDW6OCYN+N1lzHn7FzNqmmEDD3uEUptgCkE2FVgN1bfvMBb25+2zXV6nNbEw\\nw+nDIQbdtsn+9P9+TN4DAO43l4Ep/wsVulPyiC2JWxMrly/59V+KMatJ4sPuvVEq\\na6JXgdllICzUMZz2cR1/vNzAQklZpI3ePZTSGPJNbH+0QvcPgun8OQGkDnTe3Eb0\\nQ3wcHbzjfb+w1x5M6l1KKtkQG1fq7tPHsV/4/HFkabxkQV50ZjpMIfwYeZlYo1Tl\\nEHSEgqR/GB5BcefaQSwBAzTJ7Jcn4SIWirIplG77+IG1FftdnIKq6Hx0uok9jC97\\nKdP9PTJFO/x7Istm4D844IuZ1wPe4cIZ3gTF/URptaDZYnYUQw2X3tH5mwdCO1Sl\\ngaQLsVTAhvPzR6pm4iUGHUwFa6vq6gA9AM8mnGXJdHSdcTeaPnOeJcVxtz+/DKma\\nI8dXUeECfzmW30jhQuBaYm4WyKTATjXHxuU3GsbhmKJn51LbfGePu7dXxGsaPvnQ\\nIfESiysT0BkdXPql7AEmjEG1HVqMxr/cxMW4Lm2YAHL9mqIig+w9lMiD/CCjOrXR\\nZIkgPqOyFEdtZJ6pVvAKg/GDiKX9Yv9V8pjxuGqLdcKv2nf8jXtNE8DusvIoCGtz\\n12nHHdBQwDKI4wQyqlEeRrgAALa+lbYHtxIlA7RktRjFA5PtFMdy5ZJf5Ft6Esns\\nIxEJSI6AVP0OKQaaTaNZpErfHbgFO/5fqMoLjS6XEy0UVX0ykjW7voNMV91GduGS\\nJI3HwH7OkjnvPyG81PwjOzdi/VqtzCYmI2zl6LuHemndEOD06DXUa6mztudIs1cM\\nN+WaiAAmbX5ppzN+YzAzhTM3bn3W/vN0vkLydKzekCs1hMs8ErwPkNkmdzySg+Zu\\nntvR3OOQXcl+7kNRISEr1OVELB3Th78TDeUcsx/JIGhIFpOIjtGeXz6rjusoJ/FJ\\n1jmonh830SiRmBvtdOA+uLHXw4Xsx2ucmWSIYpiQMRlZcsPfQBmU75l5bXrSpGtt\\ncEPngFVsSBeS5HuyVpU2fsYmmTZ2S3Q6imzg+1zsVnyXYo00V/3cyFIs+DIYB/a0\\ni1iKpns68Bj54qCrkxeaD/G1o1aMlQeekw7fRDZ5PxKI/EzIi/7P/3YL7zL8eRqE\\nTa/Bq9Z/Xm7nTVf3oUvzJdL4V3nSS1tmruktz0KXr/d1di2u7gtp6YqyR94DZtju\\ncrmuEKVdLhtdrYvstI8Rk9fJm7QR0il8FriwBaeti9aJjPPXDEjmeYf0o01wcpfn\\nuxtFM4JLtxOIKORU/enB1XSOsAmLG27IpIJfBWauAJmoPBiE8uOmiYb7Hx5DHUx5\\nmgWejWMOTCI/9gYu7RfYc5ZY2lO4BVMefpPWAWufag6Nwolti2l2FHLJBjfd2RBR\\ndv44+nFunyx1gE+bEiOUr4YbKRfaxvlTeDLbTTuZFX0wV24asByJhEeFFqJI6SA8\\nVK13uyQCON9eCknnEQKxkowmiXsgwzUX4pUmB48WpaHArrk8Ynifkxg=\\n-----END ENCRYPTED PRIVATE KEY-----\\n'; }\n\
             function derRoundTrip() {\n\
               const priv = __thaw_crypto_module.createPrivateKey({ key: Buffer.from(rsaPrivateDerHex(), 'hex'), format: 'der' });\n\
               const pub = __thaw_crypto_module.createPublicKey({ key: Buffer.from(rsaPublicDerHex(), 'hex'), format: 'der' });\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(priv, 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, sig, 'hex');\n\
               const opensslSig = '8d357033bbae2065e82bd5cea6be16e4520ed5b5e2513862d0ab4e9817542944202c8dd7b0e738d71618a7cf93b0d75ea3a04022822ad6858446237729883dfa7c4f399472a742cb68fdb80cfd46a1865cd28f4d21a5b8f539ba6ac7ce8fcf67d03fd159bcd6b47f148822eedf83d93d3815c4fffe3943e008c2f68d07c2ef31c5e2ead3cd82bcf2b6164a4bff2e5ea199ad000f29af65d4cb633eff190a1add38cfea512bf6757a4a04ff868fe530973bf66ad0dc69775da151c86d78e174ded32722b3339da059823efc4e2f6645ca814be5bfe3663808c048ef60aec4a34223615819a354e340935684132fdf93641a4047cf623c7dac075357b7f4e6f514';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, opensslSig, 'hex');\n\
               return [priv.asymmetricKeyType, pub.asymmetricKeyType, ok, opensslVerifies];\n\
             }\n\
             function passphraseRoundTrip() {\n\
               let wrongPassphraseThrew = false;\n\
               try { __thaw_crypto_module.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }\n\
               const priv = __thaw_crypto_module.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'hunter2' });\n\
               const pub = __thaw_crypto_module.createPublicKey(rsaPublicPem());\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(priv, 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, sig, 'hex');\n\
               const opensslSig = '8d357033bbae2065e82bd5cea6be16e4520ed5b5e2513862d0ab4e9817542944202c8dd7b0e738d71618a7cf93b0d75ea3a04022822ad6858446237729883dfa7c4f399472a742cb68fdb80cfd46a1865cd28f4d21a5b8f539ba6ac7ce8fcf67d03fd159bcd6b47f148822eedf83d93d3815c4fffe3943e008c2f68d07c2ef31c5e2ead3cd82bcf2b6164a4bff2e5ea199ad000f29af65d4cb633eff190a1add38cfea512bf6757a4a04ff868fe530973bf66ad0dc69775da151c86d78e174ded32722b3339da059823efc4e2f6645ca814be5bfe3663808c048ef60aec4a34223615819a354e340935684132fdf93641a4047cf623c7dac075357b7f4e6f514';\n\
               const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, opensslSig, 'hex');\n\
               return [wrongPassphraseThrew, priv.asymmetricKeyType, ok, opensslVerifies];\n\
             }"
        ),
        1
    );
    assert_eq!(call("derRoundTrip", "[]"), r#"["rsa","rsa",true,true]"#);
    assert_eq!(
        call("passphraseRoundTrip", "[]"),
        r#"[true,"rsa",true,true]"#
    );
}

/// RSA `publicEncrypt`/`privateDecrypt` -- PKCS1v15 and OAEP (default)
/// padding. Decrypts ciphertexts produced by real OpenSSL for both
/// schemes (proving cross-implementation correctness of the actual RSA
/// math, since RSA encryption's own randomized padding means a fixed
/// expected-ciphertext comparison isn't meaningful the other
/// direction), then round-trips thaw's own `publicEncrypt` output back
/// through `privateDecrypt` for both padding schemes too.
#[test]
fn crypto_rsa_encrypt_and_decrypt_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function rsaPrivateKey() { return '-----BEGIN PRIVATE KEY-----\\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCUL37AkzIH5rjG\\npZh4Kim6Xbv/yQqtJ+Tt/yK1csBwIO2Gt9PA1dPi4FAjkIk8pLD57pDQO8MQP7eu\\nB/K3vpoqNeUCMEJY0ZeQYdRU9saBJTH7yzopljcdbKTe4peT0Dcfp1mJmL19s4RN\\nd/oz1ZNHMBEZ1Xfx/ODc6gLgZkYNKeOXr6Fjc8zRxnId6q3O9/QXkt+jz4a1a/Gh\\nrMT9E0CKY+QBKuqc+20epfegcZckBJBTixlUFuYOIrCOh6kkwuYBQpkeNq2STDPW\\n8HtMjduKvh2a9JGE42Gdwolq5JMPlv04I0fA5pRNEYOa9aitp4L5B03ekoX6xCaW\\neavOj/UzAgMBAAECggEAF9VFuRpTfyLUEBr9GUKKwI8n1/1RKsVSVBbnUbCZk88v\\n9K1nMMoTUJeMPBQYhnjkf+YnQ16BQoFE/QgJORU+PVC6uu3hFeDr1Axv9pRUG9xM\\nHDe07JBc3+4j3DcsctkXrI8hXviCbY+sVTtZMfIFRHtOHM4RAwoNbmpyuP2qAZ6+\\n5mNXRj4YFBhu+kSRf8fhEirmSnPJmFnc2if+A0OV2Dex0nsCm/qxTO+AcKzWUSAB\\n2c/FahmtjXb73vZrTaO1EYJkP7HEl2M9SLJux+VkkVmeq/5XEcAnMg1pGdgRj2+H\\n1Br59BH3p0bk84bPqjoHFtQSq4YqtUwCpXaJBzJDaQKBgQDLeg8gjaBIMCqiznrb\\nX3TPeQzA92cTmpjc713ZlsKNXuWSoxTb12P94bLEV2iqMkl/LzuFnUJivaF3rMvc\\n/2SVPgRluB0xWYb0btQImL+ELyiNvl1H8OeJy223Ow4togfW0iYvUOtcwi8kTe/m\\n4DWQa7pZpaxjYugKq8bl6eioSQKBgQC6b7yDwoVj6WbJ9lNgCLgnU301GB7Cybfc\\nrW4pN3mPgkI+B9V//Lp8hSYSsxpQwPQ6Rtgpw5pdkY6ua9xDWLMGQZYaL8GhmJmc\\nho/l7OOX7oSGHqbtSMK7m2D5HPlrt+BDWHQHX95e0j02mOnxhq3xBn164XTj3Cyy\\nEtEF5YuJmwKBgQC1gFRcClkN64EsppgqdNSCeQzqWAVnFEEE2rPRcsxqRFrt2XCy\\nxUfZYGkRAJNJNgAfZidnAScFYvfUA5v5rwquoZpUjc3khmJ+SRnz7STwqQw4m7Uj\\nhf1TCdX9Wr1D8UOi2OPc0waPQFvCu46iWB8Pizi33LOQF9q6Ig4SafrxmQKBgCbR\\nDtHsFTO5K8KO+8r55cWiV2ZPkFAECbjzjwUb3L5pY3tgzC3qo7U7T7MDAU6g7fiY\\nOXdwl1o17RwZrvGCrTt3OlZXbRxFFm6Fgb5gdP50FbmK9jxfMtQ2xJj5VGD+Fr5O\\n01GZv0XExiPw8HxuCxcsv8Fu4ZRzigbFbimpIkVTAoGASsv6qCjPGzMygvPkh80p\\ngn8ulNxSfekhhhiuWevPoAsUfJgrCuGmQN7uAxDm4pfTRTR+SrNFqSR70UDZykKY\\ntTKrnvNIiUHNtNZBFvnbmiE4NBP4vGS09dftIUo0luMJCHfFlBySAclkoIkIq8kd\\nSOKNyXbBeMijLpWo3JwVmkY=\\n-----END PRIVATE KEY-----\\n'; }\n\
             function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAlC9+wJMyB+a4xqWYeCop\\nul27/8kKrSfk7f8itXLAcCDthrfTwNXT4uBQI5CJPKSw+e6Q0DvDED+3rgfyt76a\\nKjXlAjBCWNGXkGHUVPbGgSUx+8s6KZY3HWyk3uKXk9A3H6dZiZi9fbOETXf6M9WT\\nRzARGdV38fzg3OoC4GZGDSnjl6+hY3PM0cZyHeqtzvf0F5Lfo8+GtWvxoazE/RNA\\nimPkASrqnPttHqX3oHGXJASQU4sZVBbmDiKwjoepJMLmAUKZHjatkkwz1vB7TI3b\\nir4dmvSRhONhncKJauSTD5b9OCNHwOaUTRGDmvWoraeC+QdN3pKF+sQmlnmrzo/1\\nMwIDAQAB\\n-----END PUBLIC KEY-----\\n'; }\n\
             function opensslDecryptOaep() {\n\
               const cipher = Buffer.from('45ea8547c345aa265034f3e7c48246361a91115c2b58a9795292485f836ca110a2e6e7c27e1bd47d8210edb5f0edab91e9832e85b07ab474255ea50d291a90f2f480c67cf553709cf30872b2905066a554b2700bfdf617772c586d02146845e8402d1961bccc72bf08f600b56ad561995f87d9b62021b01496b95a7161c91a8dcc6325221ab3cc085185dd6b5d91d8d258704643992dd8f530d259e1c300767efc45be7851352c7eb82291659b5a92da345ec0a2f60e402e2fabe43e11aa3e44082fed7a1740a974989a34ff6b0a6559f551dc0aec981fc1dac0c7b724129944ce9efdb5f6ea980441479390a3b7e05b6b7ccce6237ebe5c61d0d1e1afd31f48', 'hex');\n\
               return __thaw_crypto_module.privateDecrypt(rsaPrivateKey(), cipher).toString();\n\
             }\n\
             function opensslDecryptPkcs1() {\n\
               const cipher = Buffer.from('0b68b51512114dbd3fe6f4049138f04033cc9b56a5c4609bc8bf51b26f4b653499a18a144343b983f0fccdb1933e9bd04f24f63bbe908d5f8dc3d8f90defbf55fb2284db690ee81d99605e6106cf7f9aa211188cddcf3dceabf4f6d6279a7b3e1d680d1e5751c94985f9d32b44d5b8117e16acf636bb5d17600c4b1f5967bb73d430cd792a3df78aebc474322fd3e48567326252864bd285c540857d12b08f6f2300a2cd33fb58bcc0d47e5d1f58096c70968eb70ec7333e0ccce450df9f89c62da27e5488911278a374cd0d400d1f113492fdfdee1d782968cbf49da7f72843f5cc38f8e14cc31123c106ce5345df967c925f7b139bae21c9b5979574aec262', 'hex');\n\
               return __thaw_crypto_module.privateDecrypt({ key: rsaPrivateKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PADDING }, cipher).toString();\n\
             }\n\
             function thawRoundTripOaep() {\n\
               const cipher = __thaw_crypto_module.publicEncrypt(rsaPublicKey(), Buffer.from('round trip oaep'));\n\
               return __thaw_crypto_module.privateDecrypt(rsaPrivateKey(), cipher).toString();\n\
             }\n\
             function thawRoundTripPkcs1() {\n\
               const options = { key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PADDING };\n\
               const cipher = __thaw_crypto_module.publicEncrypt(options, Buffer.from('round trip pkcs1'));\n\
               return __thaw_crypto_module.privateDecrypt({ key: rsaPrivateKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PADDING }, cipher).toString();\n\
             }"
        ),
        1
    );
    assert_eq!(call("opensslDecryptOaep", "[]"), r#""secret message""#);
    assert_eq!(call("opensslDecryptPkcs1", "[]"), r#""secret message""#);
    assert_eq!(call("thawRoundTripOaep", "[]"), r#""round trip oaep""#);
    assert_eq!(call("thawRoundTripPkcs1", "[]"), r#""round trip pkcs1""#);
}

/// `generateKeyPairSync`/`generateKeyPair` for RSA/EC/Ed25519 -- every
/// freshly generated keypair signs and verifies correctly, and every
/// `privateKeyEncoding`/`publicKeyEncoding` output shape (an actual
/// `KeyObject` when omitted -- real Node's own default -- a PEM
/// string, or raw DER bytes) round-trips back through
/// `createPrivateKey`/`createPublicKey`. A 512-bit RSA modulus keeps
/// this fast; real usage should always use 2048+ (not enforced here,
/// matching this shim's existing "no validation beyond what the
/// underlying crypto call itself performs" style).
#[test]
fn crypto_generates_rsa_ec_and_ed25519_key_pairs_that_round_trip() {
    assert_eq!(
        load(
            "function signAndVerify(priv, pub) {\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('generated key test').sign(priv, 'hex');\n\
               return __thaw_crypto_module.createVerify('sha256').update('generated key test').verify(pub, sig, 'hex');\n\
             }\n\
             function rsaKeyObjectRoundTrip() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('rsa', { modulusLength: 512 });\n\
               return [publicKey instanceof __thaw_crypto_module.KeyObject, privateKey instanceof __thaw_crypto_module.KeyObject, publicKey.asymmetricKeyType, privateKey.asymmetricKeyType, signAndVerify(privateKey, publicKey)];\n\
             }\n\
             function ecPemRoundTrip() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('ec', { namedCurve: 'P-256', publicKeyEncoding: { type: 'spki', format: 'pem' }, privateKeyEncoding: { type: 'pkcs8', format: 'pem' } });\n\
               const isPem = typeof publicKey === 'string' && publicKey.indexOf('-----BEGIN PUBLIC KEY-----') === 0 && typeof privateKey === 'string' && privateKey.indexOf('-----BEGIN PRIVATE KEY-----') === 0;\n\
               const priv = __thaw_crypto_module.createPrivateKey(privateKey);\n\
               const pub = __thaw_crypto_module.createPublicKey(publicKey);\n\
               return [isPem, priv.asymmetricKeyType, priv.asymmetricKeyDetails.namedCurve, signAndVerify(priv, pub)];\n\
             }\n\
             function ed25519DerRoundTrip() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('ed25519', { publicKeyEncoding: { type: 'spki', format: 'der' }, privateKeyEncoding: { type: 'pkcs8', format: 'der' } });\n\
               const isBuffer = publicKey instanceof Buffer && privateKey instanceof Buffer;\n\
               const priv = __thaw_crypto_module.createPrivateKey({ key: privateKey, format: 'der' });\n\
               const pub = __thaw_crypto_module.createPublicKey({ key: publicKey, format: 'der' });\n\
               return [isBuffer, priv.asymmetricKeyType, signAndVerify(priv, pub)];\n\
             }\n\
             async function asyncGeneration() {\n\
               const [publicKey, privateKey] = await new Promise((resolve, reject) => {\n\
                 __thaw_crypto_module.generateKeyPair('ed25519', {}, (error, pub, priv) => {\n\
                   if (error) reject(error); else resolve([pub, priv]);\n\
                 });\n\
               });\n\
               return [publicKey instanceof __thaw_crypto_module.KeyObject, signAndVerify(privateKey, publicKey)];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("rsaKeyObjectRoundTrip", "[]"),
        r#"[true,true,"rsa","rsa",true]"#
    );
    assert_eq!(call("ecPemRoundTrip", "[]"), r#"[true,"ec","P-256",true]"#);
    assert_eq!(
        call("ed25519DerRoundTrip", "[]"),
        r#"[true,"ed25519",true]"#
    );
    assert_eq!(call("asyncGeneration", "[]"), "[true,true]");
}

/// `generateKeyPairSync`'s `privateKeyEncoding`/`publicKeyEncoding`
/// `type: 'pkcs1'` (RSA)/`'sec1'` (EC) -- the native format real
/// OpenSSL itself produces (`-----BEGIN RSA PRIVATE KEY-----`/`-----
/// BEGIN EC PRIVATE KEY-----`), as opposed to the default PKCS8/SPKI
/// wrapper -- plus a custom RSA `publicExponent`. Each generated key
/// signs and verifies (proving the PKCS1/SEC1-encoded key is actually
/// functional, not just cosmetically labeled), and the custom exponent
/// is confirmed via `asymmetricKeyDetails.publicExponent` (a real
/// value, not merely "sign/verify still succeeds", since a
/// self-consistent key would sign/verify correctly under *any*
/// exponent -- this is the one property that only holds if the
/// requested exponent was actually honored). Every PEM header line
/// below was also cross-checked once against real `openssl rsa
/// -in - -text -noout` / `openssl ec -in - -text -noout`, confirming
/// OpenSSL itself parses Thaw's PKCS1/SEC1 output as a genuine RSA/EC
/// key with the requested exponent/curve.
#[test]
fn crypto_generates_pkcs1_and_sec1_keys_with_a_custom_rsa_exponent() {
    assert_eq!(
        load(
            "function signAndVerify(priv, pub) {\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('pkcs1/sec1 test').sign(priv, 'hex');\n\
               return __thaw_crypto_module.createVerify('sha256').update('pkcs1/sec1 test').verify(pub, sig, 'hex');\n\
             }\n\
             function rsaPkcs1WithCustomExponent() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('rsa', {\n\
                 modulusLength: 512, publicExponent: 3,\n\
                 privateKeyEncoding: { type: 'pkcs1', format: 'pem' },\n\
                 publicKeyEncoding: { type: 'pkcs1', format: 'pem' }\n\
               });\n\
               const privOk = privateKey.indexOf('-----BEGIN RSA PRIVATE KEY-----') === 0;\n\
               const pubOk = publicKey.indexOf('-----BEGIN RSA PUBLIC KEY-----') === 0;\n\
               const priv = __thaw_crypto_module.createPrivateKey(privateKey);\n\
               const pub = __thaw_crypto_module.createPublicKey(publicKey);\n\
               return [privOk, pubOk, priv.asymmetricKeyDetails.publicExponent === 3n, signAndVerify(priv, pub)];\n\
             }\n\
             function ecSec1PrivateKey() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('ec', {\n\
                 namedCurve: 'P-256',\n\
                 privateKeyEncoding: { type: 'sec1', format: 'pem' },\n\
                 publicKeyEncoding: { type: 'spki', format: 'pem' }\n\
               });\n\
               const privOk = privateKey.indexOf('-----BEGIN EC PRIVATE KEY-----') === 0;\n\
               const priv = __thaw_crypto_module.createPrivateKey(privateKey);\n\
               const pub = __thaw_crypto_module.createPublicKey(publicKey);\n\
               return [privOk, priv.asymmetricKeyDetails.namedCurve, signAndVerify(priv, pub)];\n\
             }\n\
             function rsaDefaultExponentIsStill65537() {\n\
               const { privateKey } = __thaw_crypto_module.generateKeyPairSync('rsa', { modulusLength: 512 });\n\
               return privateKey.asymmetricKeyDetails.publicExponent === 65537n;\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("rsaPkcs1WithCustomExponent", "[]"),
        "[true,true,true,true]"
    );
    assert_eq!(call("ecSec1PrivateKey", "[]"), r#"[true,"P-256",true]"#);
    assert_eq!(call("rsaDefaultExponentIsStill65537", "[]"), "true");
}

/// `crypto.diffieHellman({privateKey, publicKey})` (EC P-256 and
/// X25519) and the classic `crypto.createECDH('P-256')` raw-byte API,
/// both cross-checked against a real `openssl pkeyutl -derive` run
/// over the same real OpenSSL-generated key pairs -- proving the
/// shared secret is actually correct, not just "both sides of a
/// self-generated exchange happen to agree" (which would hold even if
/// the whole implementation used the wrong curve arithmetic, as long
/// as it was wrong the same way both directions). The EC case is also
/// checked both directions (Alice's private + Bob's public, and vice
/// versa) to confirm real ECDH's symmetry, and classic `ECDH.
/// generateKeys()`/`computeSecret()` get one more self-consistency
/// round trip between two fresh instances.
#[test]
fn crypto_diffie_hellman_and_ecdh_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function alicePrivPem() { return '-----BEGIN EC PRIVATE KEY-----\\nMHcCAQEEID8/kIPxluh6keGFqU7b+3PMEAqaumkIFLrSgZZmVJa1oAoGCCqGSM49\\nAwEHoUQDQgAELfzRLsK1BBFn4pglfQV7Gazp4V61JldbTEjrf6eZ2F6javGqeCV2\\nUkovc0eH52rsLp2GA08farLjd+gMQ1Cezg==\\n-----END EC PRIVATE KEY-----\\n'; }\n\
             function alicePubPem() { return '-----BEGIN PUBLIC KEY-----\\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELfzRLsK1BBFn4pglfQV7Gazp4V61\\nJldbTEjrf6eZ2F6javGqeCV2Ukovc0eH52rsLp2GA08farLjd+gMQ1Cezg==\\n-----END PUBLIC KEY-----\\n'; }\n\
             function bobPrivPem() { return '-----BEGIN EC PRIVATE KEY-----\\nMHcCAQEEIMwxYX5kMWvdBF6UKBmHbwviYmWvZqF0XmH/Stn6+RjdoAoGCCqGSM49\\nAwEHoUQDQgAELR0lReKZrqy9p6px1daxStA6z3UArX/kMx4PHGbFDVeCMaRozDLx\\nAauzY9WI3CBAoIt8J2cFy1EiGvlcmrzUrA==\\n-----END EC PRIVATE KEY-----\\n'; }\n\
             function bobPubPem() { return '-----BEGIN PUBLIC KEY-----\\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELR0lReKZrqy9p6px1daxStA6z3UA\\nrX/kMx4PHGbFDVeCMaRozDLxAauzY9WI3CBAoIt8J2cFy1EiGvlcmrzUrA==\\n-----END PUBLIC KEY-----\\n'; }\n\
             function ecDiffieHellmanBothDirections() {\n\
               const ab = __thaw_crypto_module.diffieHellman({\n\
                 privateKey: __thaw_crypto_module.createPrivateKey(alicePrivPem()),\n\
                 publicKey: __thaw_crypto_module.createPublicKey(bobPubPem())\n\
               }).toString('hex');\n\
               const ba = __thaw_crypto_module.diffieHellman({\n\
                 privateKey: __thaw_crypto_module.createPrivateKey(bobPrivPem()),\n\
                 publicKey: __thaw_crypto_module.createPublicKey(alicePubPem())\n\
               }).toString('hex');\n\
               return [ab, ab === ba];\n\
             }\n\
             function x25519DiffieHellman() {\n\
               const privPem = '-----BEGIN PRIVATE KEY-----\\nMC4CAQAwBQYDK2VuBCIEIDDdFhjLq58TVyVinr+MzvuL85xQJ/jpJzm4LL6e4ARP\\n-----END PRIVATE KEY-----\\n';\n\
               const peerPubPem = '-----BEGIN PUBLIC KEY-----\\nMCowBQYDK2VuAyEAZ3DdQhuKTv05mGmqCtCNs+G0vJtx9eNnUnbCNwVEAXY=\\n-----END PUBLIC KEY-----\\n';\n\
               return __thaw_crypto_module.diffieHellman({\n\
                 privateKey: __thaw_crypto_module.createPrivateKey(privPem),\n\
                 publicKey: __thaw_crypto_module.createPublicKey(peerPubPem)\n\
               }).toString('hex');\n\
             }\n\
             function classicEcdhMatchesOpenssl() {\n\
               const ecdh = new __thaw_crypto_module.ECDH('P-256');\n\
               ecdh.setPrivateKey(Buffer.from('3f3f9083f196e87a91e185a94edbfb73cc100a9aba690814bad28196665496b5', 'hex'));\n\
               return ecdh.computeSecret(Buffer.from('042d1d2545e299aeacbda7aa71d5d6b14ad03acf7500ad7fe4331e0f1c66c50d578231a468cc32f101abb363d588dc2040a08b7c276705cb51221af95c9abcd4ac', 'hex')).toString('hex');\n\
             }\n\
             function ecdhGenerateKeysRoundTrip() {\n\
               const a = new __thaw_crypto_module.ECDH('P-256');\n\
               const b = new __thaw_crypto_module.ECDH('P-256');\n\
               const aPub = a.generateKeys();\n\
               const bPub = b.generateKeys();\n\
               const secretA = a.computeSecret(bPub).toString('hex');\n\
               const secretB = b.computeSecret(aPub).toString('hex');\n\
               return [secretA === secretB, secretA.length === 64];\n\
             }\n\
             function x25519KeygenRoundTrip() {\n\
               const alice = __thaw_crypto_module.generateKeyPairSync('x25519');\n\
               const bob = __thaw_crypto_module.generateKeyPairSync('x25519');\n\
               const secretA = __thaw_crypto_module.diffieHellman({ privateKey: alice.privateKey, publicKey: bob.publicKey }).toString('hex');\n\
               const secretB = __thaw_crypto_module.diffieHellman({ privateKey: bob.privateKey, publicKey: alice.publicKey }).toString('hex');\n\
               return [alice.privateKey.asymmetricKeyType, secretA === secretB, secretA.length === 64];\n\
             }"
        ),
        1
    );
    let expected_shared_secret = "a011fde311c142bd1dfdfa0e6853f2033fe6acb3d15c9e20fc824793d39a5615";
    assert_eq!(
        call("ecDiffieHellmanBothDirections", "[]"),
        format!(r#"["{expected_shared_secret}",true]"#)
    );
    assert_eq!(
        call("x25519DiffieHellman", "[]"),
        r#""1fd9b5a6bff7ce9782c32c6b88fc505af7a98fce9b9c1e5d3f9f52f879b0fb56""#
    );
    assert_eq!(
        call("classicEcdhMatchesOpenssl", "[]"),
        format!(r#""{expected_shared_secret}""#)
    );
    assert_eq!(call("ecdhGenerateKeysRoundTrip", "[]"), "[true,true]");
    assert_eq!(
        call("x25519KeygenRoundTrip", "[]"),
        r#"["x25519",true,true]"#
    );
}

/// `Intl.DateTimeFormat` -- the practical, English/Latin-numeral-only
/// polyfill (`docs/design/intl-polyfill.md`) added for real luxon, whose
/// entire timezone system is built on exactly this shape (real luxon's
/// own `IANAZone.offset()`: `new Intl.DateTimeFormat("en-US", {hour12:
/// false, timeZone, year:"numeric", month:"2-digit", day:"2-digit",
/// hour:"2-digit", minute:"2-digit", second:"2-digit",
/// era:"short"}).formatToParts(date)`, deriving the UTC offset by
/// diffing the reported *local* fields against the real timestamp).
/// Covers a real DST-transition zone (`America/New_York`, one summer +
/// one winter date -- confirming `jiff`'s bundled tzdata, not a
/// hand-rolled DST rule, actually drives this) and a fixed-offset zone
/// (`Asia/Tokyo`), every field style `.formatToParts` renders, and the
/// `h11`/`h12`/`h23`/`h24` hour-padding quirk (confirmed against real
/// Node: `hourCycle` `h23`/`h24` always zero-pads even under the
/// `'numeric'` style, unlike `h11`/`h12`).
#[test]
fn intl_date_time_format_matches_real_node_for_every_field_and_style_luxon_uses() {
    assert_eq!(
        load(
            "function ianaZoneOffsetShape() {\n\
               const summer = new Date('2024-07-04T16:30:45.000Z');\n\
               const winter = new Date('2024-01-04T16:30:45.000Z');\n\
               const dtf = new Intl.DateTimeFormat('en-US', { hour12: false, timeZone: 'America/New_York', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', era: 'short' });\n\
               return [dtf.format(summer), dtf.format(winter)];\n\
             }\n\
             function fixedOffsetZone() {\n\
               const date = new Date('2024-07-04T16:30:45.000Z');\n\
               return new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: 'numeric', second: 'numeric', timeZoneName: 'short', timeZone: 'Asia/Tokyo' }).format(date);\n\
             }\n\
             function fieldStyles() {\n\
               const date = new Date('2024-07-04T16:30:45.000Z');\n\
               const opts = (o) => new Intl.DateTimeFormat('en-US', { ...o, timeZone: 'UTC' }).format(date);\n\
               return [\n\
                 opts({ weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' }),\n\
                 opts({ year: 'numeric', month: 'short', day: 'numeric', weekday: 'short' }),\n\
                 opts({ hour: 'numeric', hourCycle: 'h12' }),\n\
                 opts({ hour: 'numeric', minute: 'numeric', hourCycle: 'h23' }),\n\
               ];\n\
             }\n\
             function invalidZoneThrows() {\n\
               try { new Intl.DateTimeFormat('en-US', { timeZone: 'Not/AReal' }); return 'no-throw'; }\n\
               catch (error) { return error instanceof RangeError; }\n\
             }\n\
             function defaultsToUtc() {\n\
               return new Intl.DateTimeFormat('en-US').resolvedOptions().timeZone;\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("ianaZoneOffsetShape", "[]"),
        r#"["07/04/2024 AD, 12:30:45","01/04/2024 AD, 11:30:45"]"#
    );
    assert_eq!(call("fixedOffsetZone", "[]"), r#""1:30:45 AM JST""#);
    assert_eq!(
        call("fieldStyles", "[]"),
        r#"["Thursday, July 4, 2024","Thu, Jul 4, 2024","4 PM","16:30"]"#
    );
    assert_eq!(call("invalidZoneThrows", "[]"), "true");
    assert_eq!(call("defaultsToUtc", "[]"), r#""UTC""#);
}

/// `timeZoneName: 'long'`/`'longGeneric'` -- real per-zone English
/// names (`intl_time_zone_names.rs`, mechanically extracted from a
/// real `node`/ICU run, not typed from memory), replacing the
/// previous synthesized `"GMT+H:MM"` fallback for these two styles.
/// Covers a real DST-transition zone in both seasons (`America/
/// New_York`: "Eastern Daylight/Standard Time", same "Eastern Time"
/// either way for `longGeneric`), a fixed-offset zone (`Asia/Tokyo`),
/// and a real *southern-hemisphere* DST-inverted zone (`Australia/
/// Sydney`: daylight in the northern winter, standard in the northern
/// summer -- the reverse of `America/New_York`'s own pattern, so this
/// isn't just "whichever season happened to be sampled").
#[test]
fn intl_time_zone_name_long_and_long_generic_match_real_node() {
    assert_eq!(
        load(
            "function longName(zone, epochMs) {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: zone, timeZoneName: 'long' }).formatToParts(new Date(epochMs)).find(p => p.type === 'timeZoneName').value;\n\
             }\n\
             function longGenericName(zone, epochMs) {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: zone, timeZoneName: 'longGeneric' }).formatToParts(new Date(epochMs)).find(p => p.type === 'timeZoneName').value;\n\
             }\n\
             function allCases() {\n\
               return [\n\
                 longName('America/New_York', 1782907200000),\n\
                 longName('America/New_York', 1767268800000),\n\
                 longGenericName('America/New_York', 1782907200000),\n\
                 longName('Asia/Tokyo', 1782907200000),\n\
                 longGenericName('Asia/Tokyo', 1782907200000),\n\
                 longName('Australia/Sydney', 1782907200000),\n\
                 longName('Australia/Sydney', 1767268800000),\n\
                 longGenericName('Australia/Sydney', 1782907200000),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("allCases", "[]"),
        r#"["Eastern Daylight Time","Eastern Standard Time","Eastern Time","Japan Standard Time","Japan Standard Time","Australian Eastern Standard Time","Australian Eastern Daylight Time","Australian Eastern Time"]"#
    );
}

/// `Intl.NumberFormat`/`Intl.ListFormat` -- covers `PolyNumberFormatter`'s
/// actual usage (plain grouping/padding), the `style: 'unit'` shape real
/// luxon's own `Duration.toHuman()` uses (found while getting luxon's
/// `.toHuman()` itself to match real Node -- the initial, narrower plan
/// only anticipated the plain-decimal shape), and `Intl.ListFormat`'s
/// conjunction/disjunction joining across 1/2/3+ items.
#[test]
fn intl_number_format_and_list_format_match_real_node() {
    assert_eq!(
        load(
            "function paddedNoGrouping() {\n\
               const nf = new Intl.NumberFormat('en-US', { useGrouping: false, minimumIntegerDigits: 2 });\n\
               return [nf.format(5), nf.format(1234)];\n\
             }\n\
             function defaultGrouping() {\n\
               return new Intl.NumberFormat('en-US').format(1234567.891);\n\
             }\n\
             function unitDurations() {\n\
               const of = (n, unit, unitDisplay) => new Intl.NumberFormat('en-US', { style: 'unit', unit, unitDisplay }).format(n);\n\
               return [of(3, 'day', 'long'), of(1, 'day', 'long'), of(3, 'hour', 'short'), of(3, 'year', 'short'), of(3, 'day', 'narrow')];\n\
             }\n\
             function lists() {\n\
               const lf = new Intl.ListFormat('en-US');\n\
               const disjunction = new Intl.ListFormat('en-US', { type: 'disjunction' });\n\
               return [lf.format(['a']), lf.format(['a', 'b']), lf.format(['a', 'b', 'c']), disjunction.format(['a', 'b', 'c'])];\n\
             }"
        ),
        1
    );
    assert_eq!(call("paddedNoGrouping", "[]"), r#"["05","1234"]"#);
    assert_eq!(call("defaultGrouping", "[]"), r#""1,234,567.891""#);
    assert_eq!(
        call("unitDurations", "[]"),
        r#"["3 days","1 day","3 hr","3 yrs","3d"]"#
    );
    assert_eq!(
        call("lists", "[]"),
        r#"["a","a and b","a, b, and c","a, b, or c"]"#
    );
}

/// `__thaw_intl_locale_resolve`/`__thaw_intl_locale_maximize` (M2 of
/// `docs/design/intl-polyfill.md`'s "Real CLDR data via icu4x" plan) --
/// real BCP-47 resolution against `thaw-icu-data`'s curated locale list,
/// gated behind the `intl` Cargo feature (default off, so this test only
/// runs with `--features intl`; see `crates/thaw-cli/src/build.rs`'s
/// `source_uses_intl`). Cross-checked against real Node's own
/// `Intl.Locale`/`.maximize()` output for both curated tags and
/// deliberately-uncurated ones, to prove the fallback chain (exact ->
/// language+region -> language+script -> bare language -> `en-US`)
/// degrades the way this file's own doc comment describes rather than
/// throwing.
#[cfg(feature = "intl")]
#[test]
fn intl_locale_resolves_curated_and_uncurated_tags_without_throwing() {
    assert_eq!(
        load(
            "function resolve(tag) {\n\
               return JSON.parse(__thaw_intl_locale_resolve(tag));\n\
             }\n\
             function maximize(tag) {\n\
               return JSON.parse(__thaw_intl_locale_maximize(tag));\n\
             }\n\
             function curated() {\n\
               return [resolve('ja-JP'), resolve('de-CH'), resolve('zh-Hant-TW')];\n\
             }\n\
             function uncuratedFallsBackGracefully() {\n\
               // 'de-AT' isn't itself curated, but 'de' is -- language\n\
               // match should win over falling all the way back to en-US.\n\
               return resolve('de-AT');\n\
             }\n\
             function unknownLanguageFallsBackToEnUs() {\n\
               // A well-formed but wholly unrecognized language subtag.\n\
               return resolve('zz-Zzzz-ZZ');\n\
             }\n\
             function malformedTagIsInvalid() {\n\
               return resolve('not a locale');\n\
             }\n\
             function likelySubtags() {\n\
               return [maximize('ja'), maximize('zh-TW'), maximize('en')];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("curated", "[]"),
        r#"[{"valid":true,"locale":"ja","language":"ja","script":null,"region":null,"calendar":"gregory","numberingSystem":"latn","collation":null},{"valid":true,"locale":"de","language":"de","script":null,"region":null,"calendar":"gregory","numberingSystem":"latn","collation":null},{"valid":true,"locale":"zh-Hant","language":"zh","script":"Hant","region":null,"calendar":"gregory","numberingSystem":"latn","collation":null}]"#
    );
    assert_eq!(
        call("uncuratedFallsBackGracefully", "[]"),
        r#"{"valid":true,"locale":"de","language":"de","script":null,"region":null,"calendar":"gregory","numberingSystem":"latn","collation":null}"#
    );
    assert_eq!(
        call("unknownLanguageFallsBackToEnUs", "[]"),
        r#"{"valid":true,"locale":"en-US","language":"en","script":null,"region":"US","calendar":"gregory","numberingSystem":"latn","collation":null}"#
    );
    assert_eq!(call("malformedTagIsInvalid", "[]"), r#"{"valid":false}"#);
    assert_eq!(
        call("likelySubtags", "[]"),
        r#"[{"valid":true,"language":"ja","script":"Jpan","region":"JP"},{"valid":true,"language":"zh","script":"Hant","region":"TW"},{"valid":true,"language":"en","script":"Latn","region":"US"}]"#
    );
}

/// `Intl.Locale` (M3) -- constructor (plain tag, tag + `calendar`/
/// `numberingSystem` options overriding any existing `-u-` extension),
/// `baseName`/`toString()`/getters, and `maximize()`/`minimize()`
/// (preserving the original's own Unicode extension keywords). Every
/// case cross-checked against real Node's own `Intl.Locale` output.
#[cfg(feature = "intl")]
#[test]
fn intl_locale_class_matches_real_node() {
    assert_eq!(
        load(
            "function baseNameAndProps(tag, opts) {\n\
               const l = new Intl.Locale(tag, opts);\n\
               return [l.baseName, l.toString(), l.language, l.script ?? null, l.region ?? null, l.calendar ?? null, l.numberingSystem ?? null];\n\
             }\n\
             function plainJaJp() { return baseNameAndProps('ja-JP'); }\n\
             function withCalendarAndNumberingSystemOptions() {\n\
               return baseNameAndProps('th', { calendar: 'buddhist', numberingSystem: 'thai' });\n\
             }\n\
             function deAt() { return baseNameAndProps('de-AT'); }\n\
             function malformedTagThrowsRangeError() {\n\
               try { new Intl.Locale('not a locale'); return 'no-throw'; }\n\
               catch (error) { return error instanceof RangeError; }\n\
             }\n\
             function missingTagThrowsTypeError() {\n\
               try { new Intl.Locale(); return 'no-throw'; }\n\
               catch (error) { return error instanceof TypeError; }\n\
             }\n\
             function maximizePreservesExtension() {\n\
               const max = new Intl.Locale('ja', { calendar: 'japanese' }).maximize();\n\
               return [max.baseName, max.toString(), max.calendar];\n\
             }\n\
             function minimizePreservesExtension() {\n\
               const min = new Intl.Locale('ja-Jpan-JP', { calendar: 'japanese' }).minimize();\n\
               return [min.baseName, min.toString(), min.calendar];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("plainJaJp", "[]"),
        r#"["ja-JP","ja-JP","ja",null,"JP",null,null]"#
    );
    assert_eq!(
        call("withCalendarAndNumberingSystemOptions", "[]"),
        r#"["th","th-u-ca-buddhist-nu-thai","th",null,null,"buddhist","thai"]"#
    );
    assert_eq!(
        call("deAt", "[]"),
        r#"["de-AT","de-AT","de",null,"AT",null,null]"#
    );
    assert_eq!(call("malformedTagThrowsRangeError", "[]"), "true");
    assert_eq!(call("missingTagThrowsTypeError", "[]"), "true");
    assert_eq!(
        call("maximizePreservesExtension", "[]"),
        r#"["ja-Jpan-JP","ja-Jpan-JP-u-ca-japanese","japanese"]"#
    );
    assert_eq!(
        call("minimizePreservesExtension", "[]"),
        r#"["ja","ja-u-ca-japanese","japanese"]"#
    );
}

/// `thaw_js_get_global`'s dotted-path support (added for `new Intl.
/// DateTimeFormat(...)` -- see `constructs_a_namespaced_global_value_
/// via_new`, thaw-llvm) tries `name` as a single, literal global
/// property *first*, falling back to a nested-property walk on `.` only
/// when no such literal global exists. Regression coverage for a real
/// bug this exact ordering fixes: a first attempt split on every `.`
/// unconditionally, which broke looking up any *literal* global whose
/// own name contains a `.` (real example: a runtime key derived from a
/// package literally named `socket.io`) -- confirmed to crash real
/// `socket.io-client`'s own `io(...)` factory call at runtime
/// ("invalid JavaScript value handle 0") before this fix.
#[test]
fn get_global_prefers_a_literal_dotted_name_over_a_nested_property_path() {
    assert_eq!(
        load("globalThis['a.b'] = 'literal'; globalThis.Namespaced = { Inner: 'nested' };"),
        1
    );
    let literal = CString::new("a.b").unwrap();
    let literal_handle = thaw_js_get_global(literal.as_ptr());
    assert_ne!(literal_handle, 0);
    assert_eq!(
        unsafe { CStr::from_ptr(thaw_js_handle_to_string(literal_handle)) }
            .to_str()
            .unwrap(),
        "literal"
    );
    let nested = CString::new("Namespaced.Inner").unwrap();
    let nested_handle = thaw_js_get_global(nested.as_ptr());
    assert_ne!(nested_handle, 0);
    assert_eq!(
        unsafe { CStr::from_ptr(thaw_js_handle_to_string(nested_handle)) }
            .to_str()
            .unwrap(),
        "nested"
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
fn reuses_a_handle_for_the_same_javascript_object() {
    assert_eq!(
        load("globalThis.sharedIdentity = {}; globalThis.returnSharedIdentity = () => sharedIdentity;"),
        1
    );
    let value = CString::new("sharedIdentity").unwrap();
    let function = CString::new("returnSharedIdentity").unwrap();
    let original = thaw_js_get_global(value.as_ptr());
    let callable = thaw_js_get_global(function.as_ptr());
    let arguments = CString::new("[]").unwrap();
    let returned = thaw_js_call_handle_handle_result(callable, arguments.as_ptr(), true);
    assert!(returned.error.is_null());
    assert_eq!(returned.value, original);
    assert_eq!(thaw_js_release_handle(original), 1);
    let text = thaw_js_handle_to_string(returned.value);
    assert_eq!(
        unsafe { CStr::from_ptr(text) }.to_str().unwrap(),
        "[object Object]"
    );
    assert_eq!(thaw_js_release_handle(returned.value), 1);
    assert_eq!(thaw_js_release_handle(returned.value), 0);
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

/// A JS function that genuinely returns `undefined` used to have that
/// result marshaled back as bare JSON `null` -- indistinguishable from
/// a real `null`, since `JSON.stringify(undefined)` returns actual JS
/// `undefined`, which `resolve_value_impl` used to substitute a literal
/// `"null"` for. Every typed decoder that actually distinguishes the
/// two (`Optional`/`Nullable`/`Nullish`/`Union`, and `thaw_json_typeof`/
/// `as_bool`/`as_string`) already recognizes the same
/// `$__thaw_napi_undefined$`-tagged sentinel native-callback argument
/// marshaling uses for a real `undefined` -- fixed by substituting that
/// sentinel here too, instead of `null`.
#[test]
fn result_abi_marshals_a_genuinely_undefined_return_value_distinctly_from_null() {
    assert_eq!(
        load(
            "function getUndefined() { return undefined; }\n\
             function getNull() { return null; }"
        ),
        1
    );
    let args = CString::new("[]").unwrap();

    let undefined_name = CString::new("getUndefined").unwrap();
    let result = thaw_js_call_result(undefined_name.as_ptr(), args.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(
        unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
        r#"{"$__thaw_napi_undefined$":true}"#
    );

    let null_name = CString::new("getNull").unwrap();
    let result = thaw_js_call_result(null_name.as_ptr(), args.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(
        unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
        "null"
    );
}

#[test]
fn result_abi_preserves_javascript_error_names() {
    assert_eq!(
        load("function fail() { const error = new Error('no'); error.name = 'AssertionError'; throw error; }"),
        1
    );
    let name = CString::new("fail").unwrap();
    let args = CString::new("[]").unwrap();
    let failed = thaw_js_call_result(name.as_ptr(), args.as_ptr());
    let error = unsafe { CStr::from_ptr(failed.error) }.to_string_lossy();
    assert!(error.starts_with("\u{1}AssertionError\u{1}"), "{error:?}");
}

/// Regression coverage for a bug where `invoke_raw` (backing
/// `thaw_js_call_handle_handle_result` -- calling a live, retained
/// function *value* whose own result also stays a retained handle, used
/// whenever a registry function's declared return type is `JsValue`,
/// real-world example: jsonwebtoken's own `verify(token, secret)`) used
/// the untagged `describe_exception` instead of `describe_tagged_exception`
/// -- unlike `thaw_js_call_result`'s own `describe_host_exception`, already
/// covered by `result_abi_preserves_javascript_error_names` above -- so a
/// caught custom `Error` subclass's `.name` was silently discarded and
/// always defaulted to plain `"Error"` on this call convention specifically,
/// even though the same fix already existed for calls by name.
#[test]
fn handle_call_result_abi_preserves_javascript_error_names() {
    assert_eq!(
        load(
            "globalThis.fail = () => { const error = new Error('no'); error.name = 'AssertionError'; throw error; };"
        ),
        1
    );
    let name = CString::new("fail").unwrap();
    let handle = thaw_js_get_global(name.as_ptr());
    assert_ne!(handle, 0);
    let args = CString::new("[]").unwrap();
    let failed = thaw_js_call_handle_handle_result(handle, args.as_ptr(), true);
    let error = unsafe { CStr::from_ptr(failed.error) }.to_string_lossy();
    assert!(error.starts_with("\u{1}AssertionError\u{1}"), "{error:?}");
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

#[test]
fn native_callback_registration_preserves_closure_identity() {
    unsafe extern "C" fn callback(
        _context: *const c_void,
        _arguments: *const c_char,
    ) -> *const c_char {
        c"null".as_ptr()
    }

    let context = &0u8 as *const u8 as *const c_void;
    let first = thaw_js_register_native_callback(
        callback as *const c_void,
        context,
        0,
        2,
        0,
        std::ptr::null(),
    );
    let second = thaw_js_register_native_callback(
        callback as *const c_void,
        context,
        0,
        2,
        0,
        std::ptr::null(),
    );
    assert!(first.error.is_null());
    assert_eq!(first.value, second.value);
}

#[test]
fn shared_json_reviver_reconstructs_node_buffers() {
    assert_eq!(
        load(
            "function reviveBuffer() { const value = JSON.parse('{\"type\":\"Buffer\",\"data\":[1,2,255]}', __thaw_json_date_reviver); return Buffer.isBuffer(value) && value.toString('hex'); }"
        ),
        1
    );
    assert_eq!(call("reviveBuffer", "[]"), r#""0102ff""#);
}

#[test]
fn shared_json_reviver_reconstructs_maps_and_sets() {
    assert_eq!(
        load(
            "function reviveCollections() { const value = JSON.parse('{\"map\":{\"__thaw_map_entries__\":[[\"a\",1]]},\"set\":{\"__thaw_set_values__\":[2,3]}}', __thaw_json_date_reviver); return [value.map instanceof Map, value.map.get('a'), value.set instanceof Set, value.set.has(3)]; }"
        ),
        1
    );
    assert_eq!(call("reviveCollections", "[]"), "[true,1,true,true]");
}

#[test]
fn shared_json_reviver_reconstructs_regular_expressions() {
    assert_eq!(
        load(
            "function reviveRegExp() { const value = JSON.parse('{\"__thaw_regexp__\":{\"source\":\"a+\",\"flags\":\"gi\",\"lastIndex\":2}}', __thaw_json_date_reviver); return [value instanceof RegExp, value.source, value.flags, value.lastIndex]; }"
        ),
        1
    );
    assert_eq!(call("reviveRegExp", "[]"), "[true,\"a+\",\"gi\",2]");
}

#[test]
fn shared_json_reviver_reconstructs_native_errors() {
    assert_eq!(
        load(
            "function reviveError() { const value = JSON.parse('\\\"\\\\u0001TypeError\\\\u0001bad\\\"', __thaw_json_date_reviver); return [value instanceof Error, value.name, value.message]; }"
        ),
        1
    );
    assert_eq!(call("reviveError", "[]"), "[true,\"TypeError\",\"bad\"]");
}

/// Regression guard for a previously-documented (2026-09-05,
/// [[project_npm_interop_gaps_2]]) rquickjs/QuickJS-NG limitation: a
/// function value reached through a *two-level* property chain
/// (`module.exports.sub.fn`), captured via a **separate**, later
/// `loadScript`/`eval` call from the one that set `module.exports` --
/// exactly the shape `ModuleBundle::qualified_aliases` (thaw-bridge)
/// still uses today for a single-level name -- was reported to become
/// silently uninvokable through the native `callDynamic` FFI boundary
/// (clean `exit(1)`, no exception). `nested_namespace_aliases` was
/// added as a workaround (capturing from *inside* the same wrapped
/// script instead). Re-investigated 2026-09-12: this no longer
/// reproduces under any tested condition, including through the real
/// compiled thaw-cli pipeline with the exact original separate-
/// loadScript mechanism deliberately restored -- most likely fixed as
/// a side effect of commit `170f6a23`'s `retain_value` self-heal (the
/// JsValue handle registry no longer assumes it was already
/// bootstrapped by an earlier call). This test pins the historically-
/// broken shape directly against thaw-quickjs to guard against a
/// future regression, independent of whether the compiler keeps using
/// the same-script-capture workaround.
#[test]
fn a_function_reached_via_a_two_level_property_chain_calls_correctly_when_captured_via_a_separate_load_script(
) {
    assert_eq!(
        load(
            "globalThis.module = { exports: {} };\n\
             (function(module, exports) {\n\
             \x20\x20var secret = 7;\n\
             \x20\x20module.exports = { sub: { fn: function() { return 42 + secret; } } };\n\
             })(globalThis.module, globalThis.module.exports);"
        ),
        1
    );
    // A separate `loadScript` call, matching `qualified_aliases`'s own
    // capture shape (including its `.bind()`-preserving-statics wrap).
    assert_eq!(
        load(
            "globalThis.__thaw_bind_preserving_statics = (fn, receiver) => {\n\
             \x20\x20if (typeof fn !== 'function') return fn;\n\
             \x20\x20const bound = fn.bind(receiver);\n\
             \x20\x20for (const prop of Object.getOwnPropertyNames(fn)) {\n\
             \x20\x20\x20\x20if (prop === 'length' || prop === 'name' || prop === 'prototype' || prop === 'arguments' || prop === 'caller') continue;\n\
             \x20\x20\x20\x20try { Object.defineProperty(bound, prop, Object.getOwnPropertyDescriptor(fn, prop)); } catch (error) {}\n\
             \x20\x20}\n\
             \x20\x20return bound;\n\
             };\n\
             if (typeof globalThis.module !== 'undefined' && globalThis.module && typeof globalThis.module.exports !== 'undefined' && globalThis.module.exports !== null && typeof globalThis.module.exports.sub.fn !== 'undefined') { globalThis[\"nested_chain_capture\"] = typeof globalThis.module.exports.sub.fn === 'function' ? globalThis.__thaw_bind_preserving_statics(globalThis.module.exports.sub.fn, globalThis.module.exports.sub) : globalThis.module.exports.sub.fn; }"
        ),
        1
    );
    assert_eq!(call("nested_chain_capture", "[]"), "49");
}
