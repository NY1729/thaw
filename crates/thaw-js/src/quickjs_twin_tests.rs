//! Twins of retired-engine `tests.rs` cases. Each one replays the original JS snippet (same function, same
//! JSON arguments) on thaw-js and asserts the result; every expectation was cross-checked against Node 22.
//! WebAssembly snippets were assembled from their WAT text to binary (Node never accepts text).

use super::*;

enum Step {
    Load(&'static str),
    Call(&'static str, &'static str, &'static str),
}

fn json_str(s: &str) -> String {
    format!("{s:?}")
}

/// Run the steps like the retired-engine `load`/`call` helpers: `Load` evaluates a script, `Call` invokes a
/// global function with JSON arguments, awaits its promise and returns `JSON.stringify` of the value.
fn twin(steps: &[Step]) {
    let mut it = Interp::new();
    for step in steps {
        match step {
            Step::Load(src) => {
                if it.run(src).is_err() {
                    panic!("load failed");
                }
                let _ = it.run_jobs();
            }
            Step::Call(name, args, expected) => {
                let driver = format!(
                    "new Promise(function (r) {{ r(globalThis[{}].apply(undefined, JSON.parse({}))); }}).then(function (v) {{ var s = JSON.stringify(v); return s === undefined ? '$undef' : s; }}, function (e) {{ return 'ERR:' + e.name + ': ' + e.message; }})",
                    json_str(name),
                    json_str(args)
                );
                let promise = it.run(&driver).expect("driver runs");
                let actual = match it.await_settled(&promise) {
                    Ok(Some(Ok(v))) => it.to_str(&v).to_string(),
                    _ => "UNSETTLED".to_string(),
                };
                assert_eq!(&actual, expected, "{name}({args})");
            }
        }
    }
}

#[test]
fn twin_web_compression_streams_roundtrip_while_readers_consume() {
    twin(&[
        Step::Load(
            r#"
      async function webCompressionStreamRoundtrip() {
        const compressedStream = new CompressionStream('gzip');
        const compressedReader = compressedStream.readable.getReader();
        const compressedWriter = compressedStream.writable.getWriter();
        const encoded = [];
        const collectCompressed = (async () => {
          while (true) {
            const { value, done } = await compressedReader.read();
            if (done) break;
            encoded.push(...value);
          }
        })();
        await compressedWriter.write(new Uint8Array());
        await compressedWriter.write(new Uint8Array([111, 110, 101]));
        await compressedWriter.write(new Uint8Array([116, 119, 111]));
        await compressedWriter.close();
        await collectCompressed;
        compressedReader.releaseLock();
        compressedWriter.releaseLock();

        const decompressedStream = new DecompressionStream('gzip');
        const decompressedReader = decompressedStream.readable.getReader();
        const decompressedWriter = decompressedStream.writable.getWriter();
        const decoded = [];
        const collectDecoded = (async () => {
          while (true) {
            const { value, done } = await decompressedReader.read();
            if (done) break;
            decoded.push(...value);
          }
        })();
        const first = Math.floor(encoded.length / 2);
        await decompressedWriter.write(new Uint8Array(encoded.slice(0, first)));
        await decompressedWriter.write(new Uint8Array(encoded.slice(first)));
        await decompressedWriter.close();
        await collectDecoded;
        decompressedReader.releaseLock();
        decompressedWriter.releaseLock();
        return [encoded.length > 0, String.fromCharCode(...decoded) === 'onetwo', decoded.length === 6];
      }
    "#,
        ),
        Step::Call(
            "webCompressionStreamRoundtrip",
            r#"[]"#,
            r#"[true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_loads_and_calls_a_simple_function() {
    twin(&[
        Step::Load(r#"function add(a, b) { return a + b; }"#),
        Step::Call("add", r#"[2, 3]"#, r#"5"#),
    ]);
}

#[test]
fn twin_exposes_the_node_global_alias() {
    twin(&[
        Step::Load(r#"function hasGlobal() { return global === globalThis; }"#),
        Step::Call("hasGlobal", r#"[]"#, r#"true"#),
    ]);
}

#[test]
fn twin_process_env_reads_the_host_environment() {
    // the expectation is the host's own PATH (the CI runner's differs from a developer machine's)
    let path = std::env::var("PATH").expect("PATH is set");
    assert!(
        path.is_ascii(),
        "the JSON quoting below assumes an ASCII PATH"
    );
    let expected: &'static str = Box::leak(json_str(&path).into_boxed_str());
    twin(&[
        Step::Load(r#"function hostPath() { return process.env.PATH; }"#),
        Step::Call("hostPath", r#"[]"#, expected),
    ]);
}

#[test]
fn twin_console_methods_remain_callable_when_detached() {
    twin(&[
        Step::Load(
            r#"function detachedConsoleLog() { const log = console.log; log('detached'); return true; }"#,
        ),
        Step::Call("detachedConsoleLog", r#"[]"#, r#"true"#),
    ]);
}

#[test]
fn twin_round_trips_objects_and_arrays() {
    twin(&[
        Step::Load(r#"function identity(x) { return x; }"#),
        Step::Call(
            "identity",
            r#"[{"a": 1, "b": [true, "x"]}]"#,
            r#"{"a":1,"b":[true,"x"]}"#,
        ),
    ]);
}

#[test]
fn twin_resolves_a_returned_promise() {
    twin(&[
        Step::Load(r#"function later(x) { return Promise.resolve(x * 2); }"#),
        Step::Call("later", r#"[21]"#, r#"42"#),
    ]);
}

#[test]
fn twin_timeout_resolves_promises_and_forwards_arguments() {
    twin(&[
        Step::Load(
            r#"function timed(value) { return new Promise(resolve => {
setTimeout((left, right) => resolve(left + right), 2, value, 2);
}); }"#,
        ),
        Step::Call("timed", r#"[40]"#, r#"42"#),
    ]);
}

#[test]
fn twin_immediate_forwards_arguments_and_can_be_cancelled() {
    twin(&[
        Step::Load(
            r#"function immediate(value) { return new Promise(resolve => {
const cancelled = setImmediate(() => resolve('wrong'));
clearImmediate(cancelled);
setImmediate((left, right) => resolve(left + right), value, 2);
}); }"#,
        ),
        Step::Call("immediate", r#"[40]"#, r#"42"#),
    ]);
}

#[test]
fn twin_microtasks_run_before_zero_delay_timers() {
    twin(&[
        Step::Load(
            r#"function eventOrder() { return new Promise(resolve => {
const events = [];
setTimeout(() => { events.push('timer'); resolve(events); }, 0);
queueMicrotask(() => events.push('microtask'));
events.push('sync');
}); }"#,
        ),
        Step::Call("eventOrder", r#"[]"#, r#"["sync","microtask","timer"]"#),
    ]);
}

#[test]
fn twin_process_next_tick_is_async_and_forwards_arguments() {
    twin(&[
        Step::Load(
            r#"function tickOrder() { return new Promise(resolve => {
const events = ['sync'];
process.nextTick((left, right) => { events.push(left + right); resolve(events); }, 'next', 'Tick');
events.push('after');
}); }"#,
        ),
        Step::Call("tickOrder", r#"[]"#, r#"["sync","after","nextTick"]"#),
    ]);
}

#[test]
fn twin_process_chdir_failure_is_a_node_shaped_error() {
    twin(&[
        Step::Load(
            r#"function chdirError() { try { process.chdir('/thaw/missing/dir'); } catch (e) { return [e.name, e.code, e.errno, e.syscall, e.path === process.cwd(), e.dest, e.message.startsWith('ENOENT: no such file or directory, chdir '), e.message.endsWith("-> '/thaw/missing/dir'")]; } return 'no error'; }"#,
        ),
        Step::Call(
            "chdirError",
            r#"[]"#,
            r#"["Error","ENOENT",-2,"chdir",true,"/thaw/missing/dir",true,true]"#,
        ),
    ]);
}

#[test]
fn twin_process_listener_removal_removes_one_last_matching_registration() {
    twin(&[
        Step::Load(
            r#"
      function processListenerRemovalCases() {
        const firstEvent = 'thaw:remove:last-once';
        const secondEvent = 'thaw:remove:last-regular';
        const thirdEvent = 'thaw:remove:three';
        const fourthEvent = 'thaw:remove:unmatched';
        let first = 0, second = 0, third = 0, fourth = 0;
        const f1 = () => first++;
        const f2 = () => second++;
        const f3 = () => third++;
        const f4 = () => fourth++;
        try {
          process.on(firstEvent, f1); process.once(firstEvent, f1);
          const firstEmit = process.emit(firstEvent);
          const firstCount = process.listenerCount(firstEvent);
          const secondEmit = process.emit(firstEvent);

          process.once(secondEvent, f2); process.on(secondEvent, f2);
          const offChain = process.removeListener(secondEvent, f2) === process;
          const secondCount = process.listenerCount(secondEvent);
          const secondFirstEmit = process.emit(secondEvent);
          const secondSecondEmit = process.emit(secondEvent);

          process.on(thirdEvent, f3); process.on(thirdEvent, f3); process.on(thirdEvent, f3);
          process.off(thirdEvent, f3);
          const thirdCount = process.listenerCount(thirdEvent);
          process.emit(thirdEvent);
          const thirdCalls = third;
          process.removeListener(thirdEvent, f3);
          const thirdAfterRemoveListener = process.listenerCount(thirdEvent);

          process.on(fourthEvent, f4);
          const noMatchChain = process.off(fourthEvent, () => {}) === process;
          const fourthCount = process.listenerCount(fourthEvent);
          const fourthEmit = process.emit(fourthEvent);
          return [firstEmit, firstCount, secondEmit, first,
                  offChain, secondFirstEmit, secondCount, secondSecondEmit, second,
                  thirdCount, thirdCalls, thirdAfterRemoveListener,
                  noMatchChain, fourthCount, fourthEmit, fourth];
        } finally {
          process.off(firstEvent, f1); process.off(firstEvent, f1);
          process.off(secondEvent, f2); process.off(secondEvent, f2);
          process.off(thirdEvent, f3); process.off(thirdEvent, f3); process.off(thirdEvent, f3);
          process.off(fourthEvent, f4);
        }
      }
    "#,
        ),
        Step::Call(
            "processListenerRemovalCases",
            r#"[]"#,
            r#"[true,1,true,3,true,true,1,false,1,2,2,1,true,1,true,1]"#,
        ),
    ]);
}

#[test]
fn twin_process_listener_removal_preserves_emit_snapshots_and_once_reentrancy() {
    twin(&[
        Step::Load(
            r#"
      function processListenerSnapshotCases() {
        const reentrantEvent = 'thaw:remove:reentrant-once';
        const recursiveEvent = 'thaw:remove:recursive-once';
        const ordinaryEvent = 'thaw:remove:snapshot-ordinary';
        const removedOnceEvent = 'thaw:remove:snapshot-once';
        const freshEvent = 'thaw:remove:fresh-same-callback';
        const reentrantLog = [], ordinaryLog = [], removedOnceLog = [];
        let depth = 0, recursiveCalls = 0, nestedEmit = true;
        let freshCalls = 0;
        const lead = function(label) {
          reentrantLog.push('lead:' + label);
          if (depth === 0) {
            depth++;
            process.emit(reentrantEvent, 'inner');
            depth--;
          }
        };
        const recursiveOnce = function() {
          recursiveCalls++;
          nestedEmit = process.emit(recursiveEvent);
        };
        const ordinary = () => ordinaryLog.push('later');
        const removeOrdinary = () => process.off(ordinaryEvent, ordinary);
        const removedOnce = () => removedOnceLog.push('later-once');
        const removeOnce = () => process.off(removedOnceEvent, removedOnce);
        const reentrantOnce = label => reentrantLog.push('once:' + label);
        const same = () => freshCalls++;
        const replaceOnceWithRegular = () => {
          process.off(freshEvent, same);
          process.on(freshEvent, same);
        };
        try {
          process.on(reentrantEvent, lead);
          process.once(reentrantEvent, reentrantOnce);
          process.emit(reentrantEvent, 'outer');

          process.once(recursiveEvent, recursiveOnce);
          const recursiveOuter = process.emit(recursiveEvent);
          const recursiveAfter = process.emit(recursiveEvent);

          process.on(ordinaryEvent, removeOrdinary);
          process.on(ordinaryEvent, ordinary);
          process.emit(ordinaryEvent);
          const ordinaryAfter = process.listenerCount(ordinaryEvent);
          ordinaryLog.push(process.emit(ordinaryEvent) ? 'still-emits' : 'empty');

          process.on(removedOnceEvent, removeOnce);
          process.once(removedOnceEvent, removedOnce);
          process.emit(removedOnceEvent);
          const removedOnceCount = process.listenerCount(removedOnceEvent);

          process.once(freshEvent, replaceOnceWithRegular);
          process.once(freshEvent, same);
          process.emit(freshEvent);
          const freshCount = process.listenerCount(freshEvent);
          process.emit(freshEvent);
          return [reentrantLog, recursiveOuter, nestedEmit, recursiveAfter, recursiveCalls,
                  ordinaryLog, ordinaryAfter, removedOnceLog, removedOnceCount,
                  freshCalls, freshCount];
        } finally {
          process.off(reentrantEvent, lead);
          process.off(reentrantEvent, reentrantOnce);
          process.off(recursiveEvent, recursiveOnce);
          process.off(ordinaryEvent, removeOrdinary);
          process.off(ordinaryEvent, ordinary);
          process.off(removedOnceEvent, removeOnce);
          process.off(removedOnceEvent, removedOnce);
          process.off(freshEvent, replaceOnceWithRegular);
          process.off(freshEvent, same);
          process.off(freshEvent, same);
        }
      }
    "#,
        ),
        Step::Call(
            "processListenerSnapshotCases",
            r#"[]"#,
            r#"[["lead:outer","lead:inner","once:inner"],true,false,false,1,["later","still-emits"],1,["later-once"],1,2,1]"#,
        ),
    ]);
}

#[test]
fn twin_process_report_exposes_the_real_glibc_version() {
    twin(&[
        Step::Load(
            r#"function glibcVersion() {
const header = process.report.getReport().header;
return [typeof header.glibcVersionRuntime, header.glibcVersionRuntime.length > 0];
}"#,
        ),
        Step::Call("glibcVersion", r#"[]"#, r#"["string",true]"#),
    ]);
}

#[test]
fn twin_console_exposes_node_style_stdout_and_stderr_streams() {
    twin(&[
        Step::Load(
            r#"function consoleStreams() {
return [typeof console._stdout, typeof console._stdout.write, typeof console._stderr.write];
}"#,
        ),
        Step::Call(
            "consoleStreams",
            r#"[]"#,
            r#"["object","function","function"]"#,
        ),
    ]);
}

#[test]
fn twin_base64_globals_round_trip_latin1_and_validate_input() {
    twin(&[
        Step::Load(
            r#"function base64() {
let invalid = false;
try { atob('%%%'); } catch (error) { invalid = error instanceof DOMException && error.name === 'InvalidCharacterError' && error.code === DOMException.INVALID_CHARACTER_ERR; }
let invalidUnicode = false;
try { btoa('雪'); } catch (error) { invalidUnicode = error instanceof DOMException && error.name === 'InvalidCharacterError'; }
return [btoa('hello\u00ff'), atob('aGVs bG//\n'), invalid, invalidUnicode];
}"#,
        ),
        Step::Call("base64", r#"[]"#, r#"["aGVsbG//","helloÿ",true,true]"#),
    ]);
}

#[test]
fn twin_performance_exposes_time_origin_and_monotonic_elapsed_time() {
    twin(&[
        Step::Load(
            r#"function timing() {
const first = performance.now();
const second = performance.now();
return [typeof performance.timeOrigin, first >= 0, second >= first];
}"#,
        ),
        Step::Call("timing", r#"[]"#, r#"["number",true,true]"#),
    ]);
}

#[test]
fn twin_blob_file_mime_type_uses_printable_ascii_range() {
    twin(&[
        Step::Load(
            r#"async function blobFileMimeTypes() {
              const good = 'A B~Z';
              const invalidValues = ['\t', '\n', '\r', '\0', '\u001f', '\u007f', '\u0080', 'é', '\uD800'];
              const invalidBlob = invalidValues.every(value => new Blob([], { type: `x${value}y` }).type === '');
              const invalidFile = invalidValues.every(value => new File([], 'x', { type: `x${value}y` }).type === '');
              const invalidSlice = invalidValues.every(value => new Blob(['x']).slice(0, 1, `x${value}y`).type === '');
              const blob = new Blob([], { type: good });
              const file = new File([], 'x', { type: good });
              const slice = new Blob(['x']).slice(0, 1, good);
              let reads = 0, conversions = 0;
              const once = new Blob([], { get type() {
                reads++;
                return { toString() { conversions++; return 'X/Y'; } };
              } });
              const falsy = [new Blob([], { type: 0 }).type,
                new Blob([], { type: false }).type, new Blob([], { type: null }).type];
              let symbolRejected = false, laterReads = 0;
              try {
                new File([], 'x', { type: Symbol('type'), get lastModified() { laterReads++; return 0; } });
              } catch (error) { symbolRejected = error instanceof TypeError; }
              const sentinel = {};
              let thrownOriginal = false;
              try { new Blob([], { get type() { throw sentinel; } }); }
              catch (error) { thrownOriginal = error === sentinel; }
              return [blob.type === 'a b~z', file.type === 'a b~z', slice.type === 'a b~z',
                invalidBlob && invalidFile && invalidSlice,
                JSON.stringify(falsy) === '["0","false","null"]',
                once.type === 'x/y' && reads === 1 && conversions === 1,
                symbolRejected && laterReads === 0, thrownOriginal];
            }"#,
        ),
        Step::Call(
            "blobFileMimeTypes",
            r#"[]"#,
            r#"[true,true,true,true,true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_structured_clone_copies_cycles_and_builtins() {
    twin(&[
        Step::Load(
            r#"function cloneValues() {
const source = { nested: { value: 1 }, bytes: new Uint8Array([2, 3]), map: new Map([['key', 4]]), set: new Set([5]), date: new Date(6) };
source.self = source;
const copy = structuredClone(source);
copy.nested.value = 7; copy.bytes[0] = 8;
let rejected = false;
try { structuredClone(() => 1); } catch (error) { rejected = error instanceof DOMException && error.name === 'DataCloneError' && error.code === DOMException.DATA_CLONE_ERR; }
return [copy !== source, copy.self === copy, source.nested.value, source.bytes[0], copy.map.get('key'), copy.set.has(5), copy.date.getTime(), rejected];
}"#,
        ),
        Step::Call("cloneValues", r#"[]"#, r#"[true,true,1,2,4,true,6,true]"#),
    ]);
}

#[test]
fn twin_structured_clone_transfer_detaches_array_buffers() {
    twin(&[
        Step::Load(
            r#"function transferBuffer() {
const source = new ArrayBuffer(4); new Uint8Array(source).set([1, 2, 3, 4]);
const copy = structuredClone(source, { transfer: [source] });
let duplicate = false, invalid = false; const other = new ArrayBuffer(1);
try { structuredClone(other, { transfer: [other, other] }); } catch (error) { duplicate = error.name === 'DataCloneError'; }
try { structuredClone({}, { transfer: [{}] }); } catch (error) { invalid = error.name === 'DataCloneError'; }
return [source.byteLength, copy.byteLength, Array.from(new Uint8Array(copy)), duplicate, invalid, other.byteLength];
}"#,
        ),
        Step::Call("transferBuffer", r#"[]"#, r#"[0,4,[1,2,3,4],true,true,1]"#),
    ]);
}

#[test]
fn twin_url_search_params_skips_empty_fields_but_keeps_empty_names() {
    twin(&[
        Step::Load(
            r#"function emptyQueryFields() {
        const check = (value, expected) => { if (value !== expected) throw new Error('query field assertion'); };
        const params = new URLSearchParams('?&&a=1&&');
        check(params.size, 1); check(params.getAll('').length, 0); check(params.toString(), 'a=1');
        check(new URLSearchParams('&&').size, 0);
        const emptyNames = new URLSearchParams('&&=&=value&&');
        check(emptyNames.size, 2); check(emptyNames.getAll('').join('|'), '|value'); check(emptyNames.toString(), '=&=value');
        const url = new URL('https://example.test/?&&a=1&&');
        check(url.searchParams.size, 1); url.searchParams.append('b', '2'); check(url.search, '?a=1&b=2');
        return true;
    }"#,
        ),
        Step::Call("emptyQueryFields", r#"[]"#, r#"true"#),
    ]);
}

#[test]
fn twin_url_search_params_preserves_duplicates_and_iterates() {
    twin(&[
        Step::Load(
            r#"function queryParams() {
const params = new URLSearchParams('?b=two+words&a=1&a=2');
params.append('snow', '雪');
const before = [params.get('a'), params.getAll('a').join(','), params.has('a', '2'), params.size];
params.set('a', 3); params.delete('b', 'wrong'); params.sort();
const visited = []; params.forEach((value, key, owner) => visited.push(key + ':' + value + ':' + (owner === params)));
const record = new URLSearchParams({ x: 1, y: false });
const pairs = new URLSearchParams([['z', 4], ['z', 5]]); pairs.delete('z', 4);
return [before, params.toString(), [...params.keys()].join(','), visited.join('|'), record.toString(), pairs.toString()];
}"#,
        ),
        Step::Call(
            "queryParams",
            r#"[]"#,
            r#"[["1","1,2",true,4],"a=3&b=two+words&snow=%E9%9B%AA","a,b,snow","a:3:true|b:two words:true|snow:雪:true","x=1&y=false","z=5"]"#,
        ),
    ]);
}

#[test]
fn twin_url_preserves_params_reference_and_decodes_malformed_form_input() {
    twin(&[
        Step::Load(
            r#"function checkUrlParamsIdentity() {
        const check = (value, expected) => { if (value !== expected) throw new Error('URL assertion'); };
        const url = new URL('https://example.test/?a=1'), params = url.searchParams;
        url.search = '?b=2'; check(url.searchParams, params); check(params.get('a'), null); check(params.get('b'), '2');
        params.append('c', '3'); check(url.search, '?b=2&c=3');
        url.href = 'https://other.test/?d=4'; check(url.searchParams, params); check(params.get('b'), null); check(params.get('d'), '4');
        params.set('d', 'updated'); check(url.search, '?d=updated');
        const malformed = new URLSearchParams('raw=%zz%2&bad=%E0%80%80&bom=%EF%BB%BF&plus=a+b&unicode=雪');
        check(malformed.get('raw'), '%zz%2'); check(malformed.get('bad'), '���');
        check(malformed.get('bom'), '\uFEFF'); check(malformed.get('plus'), 'a b'); check(malformed.get('unicode'), '雪');
        check(new URL('https://example.test/?x=%FF').searchParams.get('x'), '�');
        return true;
    }"#,
        ),
        Step::Call("checkUrlParamsIdentity", r#"[]"#, r#"true"#),
    ]);
}

#[test]
fn twin_url_resolves_relative_paths_and_synchronizes_search_params() {
    twin(&[
        Step::Load(
            r#"function urls() {
const url = new URL('../next?x=1#part', 'https://User:Pass@Example.COM:443/a/b/file');
url.searchParams.append('x', 2);
url.searchParams.set('space', 'two words');
const first = [url.href, url.origin, url.protocol, url.username, url.password, url.host, url.hostname, url.port, url.pathname, url.search, url.hash, url.toJSON()];
url.search = '?fresh=yes';
const oldParamsDetached = url.searchParams.get('x') === null && url.searchParams.get('fresh') === 'yes';
url.host = 'Other.test:8080'; url.pathname = 'root/./child/../end'; url.hash = 'done';
return [first, oldParamsDetached, url.href, URL.canParse('/ok', 'https://example.test'), URL.canParse('/bad'), URL.parse('bad')];
}"#,
        ),
        Step::Call(
            "urls",
            r#"[]"#,
            r##"[["https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part","https://example.com","https:","User","Pass","example.com","example.com","","/a/next","?x=1&x=2&space=two+words","#part","https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part"],true,"https://User:Pass@other.test:8080/root/end?fresh=yes#done",true,false,null]"##,
        ),
    ]);
}

#[test]
fn twin_event_target_dispatches_listeners_and_cancellation() {
    twin(&[
        Step::Load(
            r#"function dispatchEvents() {
const target = new EventTarget();
const events = [];
const removed = () => events.push('removed');
target.addEventListener('work', removed);
target.removeEventListener('work', removed);
target.addEventListener('work', event => { events.push(event.target === target && event.currentTarget === target); event.preventDefault(); }, { once: true });
target.addEventListener('work', { handleEvent(event) { events.push('object'); event.stopImmediatePropagation(); } });
target.addEventListener('work', () => events.push('late'));
const first = target.dispatchEvent(new Event('work', { cancelable: true }));
const second = target.dispatchEvent(new Event('work', { cancelable: true }));
return [events.join(','), first, second];
}"#,
        ),
        Step::Call(
            "dispatchEvents",
            r#"[]"#,
            r#"["true,object,object",false,true]"#,
        ),
    ]);
}

#[test]
fn twin_text_encoder_and_decoder_support_utf8() {
    twin(&[
        Step::Load(
            r#"function utf8RoundTrip(value) {
const encoder = new TextEncoder();
const encoded = encoder.encode(value);
return { bytes: Array.from(encoded), text: new TextDecoder().decode(encoded) };
}
function encodeInto() {
const output = new Uint8Array(5);
const result = new TextEncoder().encodeInto('A😀B', output);
return { result, bytes: Array.from(output) };
}
function decodeInvalid() {
return new TextDecoder().decode(Uint8Array.from([0x61, 0xff, 0x62]));
}"#,
        ),
        Step::Call(
            "utf8RoundTrip",
            r#"["A\ud83d\ude00"]"#,
            r#"{"bytes":[65,240,159,152,128],"text":"A😀"}"#,
        ),
        Step::Call(
            "encodeInto",
            r#"[]"#,
            r#"{"result":{"read":3,"written":5},"bytes":[65,240,159,152,128]}"#,
        ),
        Step::Call("decodeInvalid", r#"[]"#, r#""a�b""#),
    ]);
}

#[test]
fn twin_text_decoder_streaming_retains_incomplete_utf8() {
    twin(&[
        Step::Load(
            r#"function decodeStreaming() {
const decoder = new TextDecoder();
const first = decoder.decode(Uint8Array.from([0xf0, 0x9f]), { stream: true });
const second = decoder.decode(Uint8Array.from([0x98, 0x80, 0x41]), { stream: true });
const last = decoder.decode();
const invalid = new TextDecoder();
const invalidFirst = invalid.decode(Uint8Array.from([0xe2, 0x82]), { stream: true });
const invalidSecond = invalid.decode(Uint8Array.from([0x41]), { stream: true });
const incomplete = new TextDecoder();
incomplete.decode(Uint8Array.from([0xe2]), { stream: true });
const flushed = incomplete.decode();
let fatal = false; const strict = new TextDecoder('utf-8', { fatal: true });
strict.decode(Uint8Array.from([0xf0]), { stream: true });
try { strict.decode(); } catch (error) { fatal = error instanceof TypeError; }
const bom = new TextDecoder();
const bomFirst = bom.decode(Uint8Array.from([0xef, 0xbb]), { stream: true });
const bomSecond = bom.decode(Uint8Array.from([0xbf, 0x42]), { stream: true });
return [first, second, last, invalidFirst, invalidSecond, flushed, fatal, bomFirst, bomSecond];
}"#,
        ),
        Step::Call(
            "decodeStreaming",
            r#"[]"#,
            r#"["","😀A","","","�A","�",true,"","B"]"#,
        ),
    ]);
}

#[test]
fn twin_buffer_supports_encodings_views_search_and_numeric_access() {
    twin(&[
        Step::Load(
            r#"function buffers() {
const utf8 = Buffer.from('雪ab'); const hex = Buffer.from('00ff10', 'hex'); const base64 = Buffer.from('aGk=', 'base64');
const shared = utf8.slice(3); shared[0] = 65;
const allocated = Buffer.alloc(5, 'xy'); allocated.write('Z', 2);
const numeric = Buffer.alloc(4); numeric.writeUInt16LE(0x1234, 0); numeric.writeUInt16BE(0x5678, 2);
const numeric32 = Buffer.alloc(8); numeric32.writeInt32BE(-123456, 0); numeric32.writeUInt32LE(0xfedcba98, 4);
const floating = Buffer.alloc(16); floating.writeFloatLE(1.5, 0); floating.writeDoubleBE(-2.25, 4);
const joined = Buffer.concat([base64, Buffer.from('!')]);
return [Buffer.isBuffer(utf8), Buffer.isEncoding('base64url'), Buffer.byteLength('雪'), utf8.toString(), hex.toString('base64url'), joined.toString(), allocated.toString(), allocated.indexOf('y'), allocated.lastIndexOf('x'), allocated.includes('Z'), numeric.readUInt16LE(0), numeric.readUInt16BE(2), numeric32.readInt32BE(0), numeric32.readUInt32LE(4), floating.readFloatLE(0), floating.readDoubleBE(4), Buffer.from([0, 1, 2, 3]).swap16().toString('hex'), Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from(utf8.toJSON()).equals(utf8), shared.buffer === utf8.buffer, Object.keys(Buffer).includes('from'), Buffer.from([0xff]).readInt8(), Buffer.from([0xff, 0xfe]).readInt16BE()];
}"#,
        ),
        Step::Call(
            "buffers",
            r#"[]"#,
            r#"[true,true,3,"雪Ab","AP8Q","hi!","xyZyx",1,4,true,4660,22136,-123456,4275878552,1.5,-2.25,"01000302",-1,true,true,true,-1,-2]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_javascript_function_imports_preserve_externrefs() {
    twin(&[
        Step::Load(
            r#"function wasmImportExternRefs() {
const source = new Uint8Array([0,97,115,109,1,0,0,0,1,16,3,96,1,111,1,111,96,1,111,2,111,111,96,0,1,111,2,38,3,4,104,111,115,116,4,101,99,104,111,0,0,4,104,111,115,116,4,112,97,105,114,0,1,4,104,111,115,116,5,101,109,112,116,121,0,2,3,4,3,0,1,2,7,21,3,3,114,117,110,0,3,4,109,97,110,121,0,4,4,110,111,110,101,0,5,10,20,3,6,0,32,0,16,0,11,6,0,32,0,16,1,11,4,0,16,2,11,0,27,4,110,97,109,101,1,20,3,0,4,101,99,104,111,1,4,112,97,105,114,2,5,101,109,112,116,121]);
const first = { id: 1 }, second = [2], seen = [];
const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { host: {
echo(value) { seen.push(value); return value; },
pair(value) { return [value, second]; },
empty() { return null; }
} });
const echoed = instance.exports.run(first), many = instance.exports.many(first), missing = instance.exports.run(undefined);
return [seen[0] === first, echoed === first, many[0] === first, many[1] === second, seen[1] === undefined, missing === undefined, instance.exports.none() === null];
}"#,
        ),
        Step::Call(
            "wasmImportExternRefs",
            r#"[]"#,
            r#"[true,true,true,true,true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_synchronizes_imported_memory_and_mutable_globals() {
    twin(&[
        Step::Load(
            r#"function wasmImportedState() {
const source = new Uint8Array([0,97,115,109,1,0,0,0,1,9,2,96,0,1,127,96,1,127,0,2,31,2,3,101,110,118,6,109,101,109,111,114,121,2,1,1,2,3,101,110,118,7,99,111,117,110,116,101,114,3,127,1,3,5,4,0,1,0,0,7,30,4,4,114,101,97,100,0,0,5,119,114,105,116,101,0,1,4,98,117,109,112,0,2,4,103,114,111,119,0,3,10,38,4,7,0,65,0,45,0,0,11,9,0,65,0,32,0,58,0,0,11,11,0,35,0,65,1,106,36,0,35,0,11,6,0,65,1,64,0,11,0,17,4,110,97,109,101,7,10,1,0,7,99,111,117,110,116,101,114]);
const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 }), old = memory.buffer; new Uint8Array(memory.buffer)[0] = 10;
const counter = new WebAssembly.Global({ value: 'i32', mutable: true }, 5);
const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { memory, counter } });
const sibling = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { memory, counter } });
const before = instance.exports.read(); instance.exports.write(33); const written = new Uint8Array(memory.buffer)[0], siblingRead = sibling.exports.read();
const first = instance.exports.bump(), reflected = counter.value, siblingBump = sibling.exports.bump(); counter.value = 20; const second = instance.exports.bump();
const previous = instance.exports.grow(), detached = old.byteLength, length = memory.buffer.byteLength; new Uint8Array(memory.buffer)[65536] = 77;
return [before, written, siblingRead, first, reflected, siblingBump, second, counter.value, previous, detached, length, instance.exports.read()];
}"#,
        ),
        Step::Call(
            "wasmImportedState",
            r#"[]"#,
            r#"[10,33,33,6,6,7,21,21,1,0,131072,33]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_externref_preserves_javascript_identity() {
    twin(&[
        Step::Load(
            r#"function wasmExternRefs() {
const source = new Uint8Array([0,97,115,109,1,0,0,0,1,14,3,96,1,111,1,111,96,0,1,111,96,1,111,0,2,15,1,3,101,110,118,6,115,104,97,114,101,100,3,111,1,3,4,3,0,1,2,7,20,3,4,101,99,104,111,0,0,3,103,101,116,0,1,3,115,101,116,0,2,10,18,3,4,0,32,0,11,4,0,35,0,11,6,0,32,0,36,0,11,0,16,4,110,97,109,101,7,9,1,0,6,115,104,97,114,101,100]);
const first = { name: 'first' }, second = [1, 2], shared = new WebAssembly.Global({ value: 'externref', mutable: true }, first);
const instance = new WebAssembly.Instance(new WebAssembly.Module(source), { env: { shared } });
const initial = instance.exports.get() === first, echoed = instance.exports.echo(second) === second, undefinedValue = instance.exports.echo(undefined) === undefined, nullValue = instance.exports.echo(null) === null;
instance.exports.set(second); const reflected = shared.value === second; shared.value = first; const reset = instance.exports.get() === first;
return [initial, echoed, undefinedValue, nullValue, reflected, reset];
}"#,
        ),
        Step::Call(
            "wasmExternRefs",
            r#"[]"#,
            r#"[true,true,true,true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_imported_externref_tables_share_values_and_growth() {
    twin(&[
        Step::Load(
            r#"function wasmTables() {
const source = new Uint8Array([0,97,115,109,1,0,0,0,1,16,3,96,1,127,1,111,96,2,127,111,0,96,1,111,1,127,2,16,1,3,101,110,118,5,105,116,101,109,115,1,111,1,2,4,3,4,3,0,1,2,7,28,4,5,105,116,101,109,115,1,0,3,103,101,116,0,0,3,115,101,116,0,1,4,103,114,111,119,0,2,10,27,3,6,0,32,0,37,0,11,8,0,32,0,32,1,38,0,11,9,0,32,0,65,1,252,15,0,11]);
const first = { id: 1 }, second = { id: 2 }, third = { id: 3 }, table = new WebAssembly.Table({ element: 'externref', initial: 2, maximum: 4 }, first), module = new WebAssembly.Module(source);
const instance = new WebAssembly.Instance(module, { env: { items: table } }), sibling = new WebAssembly.Instance(module, { env: { items: table } });
const initial = instance.exports.get(0) === first && instance.exports.items.get(1) === first; instance.exports.set(1, second);
const reflected = table.get(1) === second, siblingValue = sibling.exports.get(1) === second, previous = instance.exports.grow(third);
const grown = table.length === 3 && table.get(2) === third && sibling.exports.items.length === 3; table.set(0, third); const reverse = instance.exports.get(0) === third;
return [initial, reflected, siblingValue, previous, grown, reverse, instance.exports.items instanceof WebAssembly.Table];
}"#,
        ),
        Step::Call(
            "wasmTables",
            r#"[]"#,
            r#"[true,true,true,2,true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_funcref_tables_return_and_accept_exported_functions() {
    twin(&[
        Step::Load(
            r#"function wasmFunctionTables() {
const source = new Uint8Array([0,97,115,109,1,0,0,0,1,12,2,96,1,127,1,127,96,2,127,127,1,127,3,4,3,0,0,1,4,5,1,112,1,2,4,7,43,4,9,105,110,99,114,101,109,101,110,116,0,0,6,100,111,117,98,108,101,0,1,9,102,117,110,99,116,105,111,110,115,1,0,6,105,110,118,111,107,101,0,2,9,8,1,0,65,0,11,2,0,1,10,27,3,7,0,32,0,65,1,106,11,7,0,32,0,65,2,108,11,9,0,32,1,32,0,17,0,0,11,0,37,4,110,97,109,101,1,20,2,0,9,105,110,99,114,101,109,101,110,116,1,6,100,111,117,98,108,101,4,8,1,0,5,117,110,97,114,121]);
const instance = new WebAssembly.Instance(new WebAssembly.Module(source)), table = instance.exports.functions;
const first = table.get(0), stable = first === table.get(0), direct = first(9);
table.set(0, instance.exports.double); const replaced = instance.exports.invoke(0, 6);
const previous = table.grow(1, instance.exports.increment), grown = instance.exports.invoke(2, 8);
table.set(1, null); const empty = table.get(1) === null; let trapped = false;
try { instance.exports.invoke(1, 1); } catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
const sibling = new WebAssembly.Instance(new WebAssembly.Module(source)); table.set(0, sibling.exports.double); const foreign = instance.exports.invoke(0, 7);
return [table instanceof WebAssembly.Table, stable, direct, replaced, previous, table.length, grown, empty, trapped, foreign];
}"#,
        ),
        Step::Call(
            "wasmFunctionTables",
            r#"[]"#,
            r#"[true,true,10,12,2,3,9,true,true,14]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_time_zone_name_long_and_long_generic_match_real_node() {
    twin(&[
        Step::Load(
            r#"function longName(zone, epochMs) {
return new Intl.DateTimeFormat('en-US', { timeZone: zone, timeZoneName: 'long' }).formatToParts(new Date(epochMs)).find(p => p.type === 'timeZoneName').value;
}
function longGenericName(zone, epochMs) {
return new Intl.DateTimeFormat('en-US', { timeZone: zone, timeZoneName: 'longGeneric' }).formatToParts(new Date(epochMs)).find(p => p.type === 'timeZoneName').value;
}
function allCases() {
return [
longName('America/New_York', 1782907200000),
longName('America/New_York', 1767268800000),
longGenericName('America/New_York', 1782907200000),
longName('Asia/Tokyo', 1782907200000),
longGenericName('Asia/Tokyo', 1782907200000),
longName('Australia/Sydney', 1782907200000),
longName('Australia/Sydney', 1767268800000),
longGenericName('Australia/Sydney', 1782907200000),
longGenericName('Asia/Seoul', 1782907200000),
longGenericName('Asia/Hong_Kong', 1782907200000),
longGenericName('Asia/Dubai', 1782907200000),
longName('Asia/Kolkata', 1782907200000),
longGenericName('Asia/Kolkata', 1782907200000),
];
}"#,
        ),
        Step::Call(
            "allCases",
            r#"[]"#,
            r#"["Eastern Daylight Time","Eastern Standard Time","Eastern Time","Japan Standard Time","Japan Standard Time","Australian Eastern Standard Time","Australian Eastern Daylight Time","Australian Eastern Time","Korean Standard Time","Hong Kong Standard Time","Gulf Standard Time","India Standard Time","India Standard Time"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_format_and_list_format_match_real_node() {
    twin(&[
        Step::Load(
            r#"function paddedNoGrouping() {
const nf = new Intl.NumberFormat('en-US', { useGrouping: false, minimumIntegerDigits: 2 });
return [nf.format(5), nf.format(1234)];
}
function defaultGrouping() {
return new Intl.NumberFormat('en-US').format(1234567.891);
}
function significantDigits() {
const f = (opts, n) => new Intl.NumberFormat('en-US', { useGrouping: false, ...opts }).format(n);
return [f({ maximumSignificantDigits: 3 }, 1234), f({ maximumSignificantDigits: 3 }, 1.2345), f({ minimumSignificantDigits: 3 }, 2), f({ maximumSignificantDigits: 2, style: 'percent' }, 0.1234)];
}
function useGroupingModes() {
const f = (u, n) => new Intl.NumberFormat('en-US', { useGrouping: u }).format(n);
return [f(true, 1000), f(false, 1000), f('auto', 1000), f('always', 1000), f('min2', 1000), f('min2', 10000)];
}
function groupingResolved() {
return [
new Intl.NumberFormat('en-US').resolvedOptions().useGrouping,
new Intl.NumberFormat('en-US', { useGrouping: true }).resolvedOptions().useGrouping,
new Intl.NumberFormat('en-US', { useGrouping: false }).resolvedOptions().useGrouping,
];
}
function signDisplays() {
const f = (s, n) => new Intl.NumberFormat('en-US', { useGrouping: false, signDisplay: s }).format(n);
return [
f('auto', 5), f('auto', 0), f('auto', -5), f('auto', -0),
f('never', -5),
f('always', 5), f('always', 0), f('always', -5), f('always', -0),
f('exceptZero', 5), f('exceptZero', 0),
f('negative', -0),
new Intl.NumberFormat('en-US', { useGrouping: false, style: 'percent', signDisplay: 'always' }).format(0.5),
];
}
function unitDurations() {
const of = (n, unit, unitDisplay) => new Intl.NumberFormat('en-US', { style: 'unit', unit, unitDisplay }).format(n);
return [of(3, 'day', 'long'), of(1, 'day', 'long'), of(3, 'hour', 'short'), of(3, 'year', 'short'), of(3, 'day', 'narrow')];
}
function lists() {
const lf = new Intl.ListFormat('en-US');
const disjunction = new Intl.ListFormat('en-US', { type: 'disjunction' });
return [lf.format(['a']), lf.format(['a', 'b']), lf.format(['a', 'b', 'c']), disjunction.format(['a', 'b', 'c'])];
}"#,
        ),
        Step::Call("paddedNoGrouping", r#"[]"#, r#"["05","1234"]"#),
        Step::Call("defaultGrouping", r#"[]"#, r#""1,234,567.891""#),
        Step::Call(
            "significantDigits",
            r#"[]"#,
            r#"["1230","1.23","2.00","12%"]"#,
        ),
        Step::Call(
            "useGroupingModes",
            r#"[]"#,
            r#"["1,000","1000","1,000","1,000","1000","10,000"]"#,
        ),
        Step::Call("groupingResolved", r#"[]"#, r#"["auto","always",false]"#),
        Step::Call(
            "signDisplays",
            r#"[]"#,
            r#"["5","0","-5","-0","5","+5","+0","-5","-0","+5","0","0","+50%"]"#,
        ),
        Step::Call(
            "unitDurations",
            r#"[]"#,
            r#"["3 days","1 day","3 hr","3 yrs","3d"]"#,
        ),
        Step::Call(
            "lists",
            r#"[]"#,
            r#"["a","a and b","a, b, and c","a, b, or c"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_locale_class_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function baseNameAndProps(tag, opts) {
const l = new Intl.Locale(tag, opts);
return [l.baseName, l.toString(), l.language, l.script ?? null, l.region ?? null, l.calendar ?? null, l.numberingSystem ?? null];
}
function plainJaJp() { return baseNameAndProps('ja-JP'); }
function withCalendarAndNumberingSystemOptions() {
return baseNameAndProps('th', { calendar: 'buddhist', numberingSystem: 'thai' });
}
function deAt() { return baseNameAndProps('de-AT'); }
function malformedTagThrowsRangeError() {
try { new Intl.Locale('not a locale'); return 'no-throw'; }
catch (error) { return error instanceof RangeError; }
}
function missingTagThrowsTypeError() {
try { new Intl.Locale(); return 'no-throw'; }
catch (error) { return error instanceof TypeError; }
}
function maximizePreservesExtension() {
const max = new Intl.Locale('ja', { calendar: 'japanese' }).maximize();
return [max.baseName, max.toString(), max.calendar];
}
function minimizePreservesExtension() {
const min = new Intl.Locale('ja-Jpan-JP', { calendar: 'japanese' }).minimize();
return [min.baseName, min.toString(), min.calendar];
}"#,
        ),
        Step::Call(
            "plainJaJp",
            r#"[]"#,
            r#"["ja-JP","ja-JP","ja",null,"JP",null,null]"#,
        ),
        Step::Call(
            "withCalendarAndNumberingSystemOptions",
            r#"[]"#,
            r#"["th","th-u-ca-buddhist-nu-thai","th",null,null,"buddhist","thai"]"#,
        ),
        Step::Call(
            "deAt",
            r#"[]"#,
            r#"["de-AT","de-AT","de",null,"AT",null,null]"#,
        ),
        Step::Call("malformedTagThrowsRangeError", r#"[]"#, r#"true"#),
        Step::Call("missingTagThrowsTypeError", r#"[]"#, r#"true"#),
        Step::Call(
            "maximizePreservesExtension",
            r#"[]"#,
            r#"["ja-Jpan-JP","ja-Jpan-JP-u-ca-japanese","japanese"]"#,
        ),
        Step::Call(
            "minimizePreservesExtension",
            r#"[]"#,
            r#"["ja","ja-u-ca-japanese","japanese"]"#,
        ),
    ]);
}

#[test]
fn twin_assimilates_foreign_thenables_once_and_reports_then_errors() {
    twin(&[
        Step::Load(
            r#"function thenable() {
return { then(resolve, reject) { resolve(42); reject('late'); resolve(99); } };
}
function throwingThen() {
return Object.create(null, { then: { get() { throw new Error('bad then'); } } });
}"#,
        ),
        Step::Call("thenable", r#"[]"#, r#"42"#),
        Step::Call("throwingThen", r#"[]"#, r#"ERR:Error: bad then"#),
    ]);
}

#[test]
fn twin_unknown_function_reports_an_error_object_instead_of_crashing() {
    twin(&[Step::Call(
        "doesNotExist",
        r#"[]"#,
        r#"ERR:TypeError: Cannot read properties of undefined (reading 'apply')"#,
    )]);
}

#[test]
fn twin_thrown_exception_reports_an_error_object_instead_of_crashing() {
    twin(&[
        Step::Load(r#"function boom() { throw new Error('kaboom'); }"#),
        Step::Call("boom", r#"[]"#, r#"ERR:Error: kaboom"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_format_matches_real_node_for_curated_non_english_locales() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, epochMs) {
return new Intl.DateTimeFormat(locale, opts).format(new Date(epochMs));
}
function all() {
const t = Date.parse('2024-07-04T16:30:45.000Z');
const midnight = Date.parse('2024-07-04T00:00:00.000Z');
return [
show('ja-JP', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),
show('ja-JP', {year:'numeric',month:'2-digit',day:'2-digit',timeZone:'UTC'}, t),
show('ja-JP', {weekday:'long',timeZone:'UTC'}, t),
show('de-DE', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),
show('de-DE', {hour:'numeric',minute:'numeric',timeZone:'UTC'}, t),
show('fr', {weekday:'long',year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),
show('ar-SA', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),
show('en-US', {hour:'numeric',timeZone:'UTC'}, t),
show('en-US', {hour:'numeric',hour12:false,timeZone:'UTC'}, t),
show('en-US', {hour:'2-digit',minute:'2-digit',hourCycle:'h24',timeZone:'UTC'}, midnight),
show('en-US', {hour:'numeric',minute:'2-digit',hourCycle:'h24',timeZone:'UTC'}, midnight),
show('en-US', {hour:'2-digit',minute:'2-digit',hourCycle:'h23',timeZone:'UTC'}, midnight),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["2024年7月4日","2024/07/04","木曜日","4. Juli 2024","16:30","jeudi 4 juillet 2024","٤ يوليو ٢٠٢٤","4 PM","16","24:00","24:00","00:00"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_dispatches_non_gregorian_calendars_matching_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, epochMs) {
return new Intl.DateTimeFormat(locale, opts).format(new Date(epochMs));
}
function all() {
const t = Date.parse('2024-07-04T00:00:00Z');
const opts = {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'};
return [
show('th-TH', opts, t),
show('ja-JP-u-ca-japanese', opts, t),
show('ja-JP', {...opts, calendar:'japanese'}, t),
show('ar-SA-u-ca-islamic-umalqura', opts, t),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["4 กรกฎาคม 2567","令和6年7月4日","令和6年7月4日","٢٨ ذو الحجة ١٤٤٥ هـ"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_styles_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(opts) {
return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', ...opts })
.format(new Date(Date.UTC(2024, 6, 4, 16, 30, 45)));
}
function all() {
return [
show({ dateStyle: 'full' }),
show({ dateStyle: 'long' }),
show({ dateStyle: 'medium' }),
show({ dateStyle: 'short' }),
show({ timeStyle: 'full' }),
show({ timeStyle: 'long' }),
show({ timeStyle: 'medium' }),
show({ timeStyle: 'short' }),
show({ dateStyle: 'full', timeStyle: 'short' }),
];
}
function styleOption() {
return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', dateStyle: 'short' })
.resolvedOptions().dateStyle;
}
function rejectsMixed() {
try {
new Intl.DateTimeFormat('en-US', { dateStyle: 'short', year: 'numeric' });
return 'no-throw';
} catch (error) {
return error.constructor.name;
}
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["Thursday, July 4, 2024","July 4, 2024","Jul 4, 2024","7/4/24","4:30:45 PM Coordinated Universal Time","4:30:45 PM UTC","4:30:45 PM","4:30 PM","Thursday, July 4, 2024 at 4:30 PM"]"#,
        ),
        Step::Call("styleOption", r#"[]"#, r#""short""#),
        Step::Call("rejectsMixed", r#"[]"#, r#""TypeError""#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_fractional_second_digits_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })
.format(new Date(Date.UTC(2024, 0, 1, 13, 5, 45, 678)));
}
function all() {
return [
show('en-US', { fractionalSecondDigits: 1 }),
show('en-US', { fractionalSecondDigits: 2 }),
show('en-US', { fractionalSecondDigits: 3 }),
show('en-US', { hour: 'numeric', minute: '2-digit', second: 'numeric', fractionalSecondDigits: 3 }),
show('en-US', { hour: 'numeric', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 2 }),
show('de-DE', { hour: 'numeric', minute: '2-digit', second: 'numeric', fractionalSecondDigits: 3 }),
];
}
function rejectsInvalid() {
try { new Intl.DateTimeFormat('en-US', { fractionalSecondDigits: 4 }); return 'no-throw'; }
catch (error) { return error.constructor.name; }
}
function option() {
return new Intl.DateTimeFormat('en-US', { fractionalSecondDigits: 2 }).resolvedOptions().fractionalSecondDigits;
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["6","67","678","1:05:45.678 PM","1:05:45.67 PM","13:05:45,678"]"#,
        ),
        Step::Call("rejectsInvalid", r#"[]"#, r#""RangeError""#),
        Step::Call("option", r#"[]"#, r#"2"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_hour_precedence_and_option_validation_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(opts) {
const d = new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', hour: 'numeric', minute: '2-digit', ...opts });
const r = d.resolvedOptions();
return [r.hourCycle, r.hour12, d.format(new Date(Date.UTC(2024, 0, 1, 13, 5)))];
}
function rejectsInvalid() {
const bad = [];
for (const opts of [{ hourCycle: 'h25' }, { weekday: 'x' }, { month: 'x' }, { year: 'x' }]) {
try { new Intl.DateTimeFormat('en-US', opts); bad.push('no-throw'); }
catch (error) { bad.push(error.constructor.name); }
}
return bad;
}"#,
        ),
        Step::Call(
            "show",
            r#"[{"hourCycle":"h23","hour12":true}]"#,
            r#"["h12",true,"1:05 PM"]"#,
        ),
        Step::Call(
            "show",
            r#"[{"hourCycle":"h12","hour12":false}]"#,
            r#"["h23",false,"13:05"]"#,
        ),
        Step::Call(
            "show",
            r#"[{"hourCycle":"h11"}]"#,
            r#"["h11",true,"1:05 PM"]"#,
        ),
        Step::Call("show", r#"[{"hour12":false}]"#, r#"["h23",false,"13:05"]"#),
        Step::Call(
            "rejectsInvalid",
            r#"[]"#,
            r#"["RangeError","RangeError","RangeError","RangeError"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_format_matches_real_node_for_curated_non_english_locales() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, value) {
return new Intl.NumberFormat(locale, opts).format(value);
}
function all() {
return [
show('de-DE', {}, 1234567.891),
show('fr', {}, 1234567.891),
show('bn', {}, 12345),
show('th', {}, 12345),
show('ar-SA', {}, 12345),
show('de-DE', {useGrouping:false}, 1234567.891),
show('en-US', {}, 1234567.891),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["1.234.567,891","1 234 567,891","১২,৩৪৫","12,345","١٢٬٣٤٥","1234567,891","1,234,567.891"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_notation_numbering_system_and_accounting_match_real_node() {
    twin(&[
        Step::Load(
            r#"function f(locale, options, value) {
return new Intl.NumberFormat(locale, { useGrouping: false, ...options }).format(value);
}
function notation() {
return [
f('en', { notation: 'scientific' }, 1234),
f('en', { notation: 'scientific', maximumFractionDigits: 2 }, 1234),
f('en', { notation: 'scientific' }, 0),
f('en', { notation: 'engineering' }, 12345),
f('en', { notation: 'engineering' }, 0.0001234),
f('en', { notation: 'scientific' }, -1234),
f('de', { notation: 'scientific' }, 1234),
];
}
function numberingSystem() {
const g = (nu, n) => new Intl.NumberFormat('en', { numberingSystem: nu }).format(n);
return [
g('arab', 1234), g('arab', 1234567),
g('arabext', 1234567.5),
g('beng', 1234567), g('deva', 1234567), g('thai', 1234567), g('hanidec', 1234567),
new Intl.NumberFormat('en', { numberingSystem: 'arab' }).resolvedOptions().numberingSystem,
];
}
function accounting() {
const nf = new Intl.NumberFormat('en', { style: 'currency', currency: 'USD', currencySign: 'accounting' });
return [nf.format(-5), nf.format(5), nf.format(-0)];
}"#,
        ),
        Step::Call(
            "notation",
            r#"[]"#,
            r#"["1.234E3","1.23E3","0E0","12.345E3","123.4E-6","-1.234E3","1,234E3"]"#,
        ),
        Step::Call(
            "numberingSystem",
            r#"[]"#,
            r#"["١٬٢٣٤","١٬٢٣٤٬٥٦٧","۱٬۲۳۴٬۵۶۷٫۵","১,২৩৪,৫৬৭","१,२३४,५६७","๑,๒๓๔,๕๖๗","一,二三四,五六七","arab"]"#,
        ),
        Step::Call("accounting", r#"[]"#, r#"["($5.00)","$5.00","($0.00)"]"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_currency_and_percent_match_real_node() {
    twin(&[
        Step::Load(
            r#"function all() {
const f = (locale, options, value) => new Intl.NumberFormat(locale, options).format(value);
return [
f('en-US', { style: 'percent' }, 0.56),
f('tr-TR', { style: 'percent' }, 0.56),
f('en-US', { style: 'currency', currency: 'USD' }, 1234.5),
f('de-DE', { style: 'currency', currency: 'EUR' }, 1234.5),
f('fr-FR', { style: 'currency', currency: 'USD', currencyDisplay: 'code' }, 1234.5),
f('ja-JP', { style: 'currency', currency: 'JPY' }, 1234.5),
f('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name' }, 1234.5),
f('en-US', { style: 'currency', currency: 'KRW' }, 1234.5),
f('en-US', { style: 'currency', currency: 'BHD' }, 1234.5)
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["56%","%56","$1,234.50","1.234,50 €","1 234,50 USD","￥1,235","1,234.50 US dollars","₩1,235","BHD 1,234.500"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_units_match_real_node() {
    twin(&[
        Step::Load(
            r#"function all() {
const f = (locale, unit, unitDisplay, value) => new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay }).format(value);
return [f('de-DE', 'meter', 'long', 2), f('fr-FR', 'liter', 'long', 1), f('ja-JP', 'kilogram', 'short', 3)];
}"#,
        ),
        Step::Call("all", r#"[]"#, r#"["2 Meter","1 litre","3 kg"]"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_list_format_matches_real_node_for_curated_non_english_locales() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, items) {
return new Intl.ListFormat(locale, opts).format(items);
}
function all() {
return [
show('ja', {}, ['a', 'b', 'c']),
show('ja', { type: 'disjunction' }, ['a', 'b', 'c']),
show('fr', {}, ['a', 'b', 'c']),
show('de', { style: 'short' }, ['a', 'b', 'c']),
show('en-US', {}, ['a', 'b', 'c']),
show('en-US', { type: 'disjunction' }, ['a', 'b', 'c']),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["a、b、c","a、b、またはc","a, b et c","a, b und c","a, b, and c","a, b, or c"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_plural_rules_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function cardinal(locale, n) {
return new Intl.PluralRules(locale).select(n);
}
function ordinal(locale, n) {
return new Intl.PluralRules(locale, { type: 'ordinal' }).select(n);
}
function categories(locale) {
return new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
}
function byOptions(locale, opts, n) {
return new Intl.PluralRules(locale, opts).select(n);
}
function all() {
return [
cardinal('en', 1),
cardinal('en', 2),
cardinal('en', 1.0),
cardinal('en', 1.5),
cardinal('ar', 0),
cardinal('ar', 1),
cardinal('ar', 2),
cardinal('ar', 3),
cardinal('ar', 11),
cardinal('ar', 100),
cardinal('ja', 1),
ordinal('en', 1),
ordinal('en', 2),
ordinal('en', 3),
ordinal('en', 4),
ordinal('en', 11),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["one","other","one","other","zero","one","two","few","many","other","other","one","two","few","other","other"]"#,
        ),
        Step::Call("categories", r#"["en"]"#, r#"["one","other"]"#),
        Step::Call("categories", r#"["ru"]"#, r#"["few","many","one","other"]"#),
        Step::Call(
            "categories",
            r#"["ar"]"#,
            r#"["few","many","one","two","zero","other"]"#,
        ),
        Step::Call("categories", r#"["ja"]"#, r#"["other"]"#),
        Step::Call("byOptions", r#"["en", {}, 1]"#, r#""one""#),
        Step::Call(
            "byOptions",
            r#"["en", {"minimumFractionDigits":1}, 1]"#,
            r#""other""#,
        ),
        Step::Call(
            "byOptions",
            r#"["en", {"maximumFractionDigits":0}, 1.4]"#,
            r#""one""#,
        ),
        Step::Call(
            "byOptions",
            r#"["en", {"maximumFractionDigits":0}, 1.5]"#,
            r#""other""#,
        ),
        Step::Call(
            "byOptions",
            r#"["en", {"maximumFractionDigits":0}, 2]"#,
            r#""other""#,
        ),
        Step::Call(
            "byOptions",
            r#"["en", {"maximumSignificantDigits":1}, 1.4]"#,
            r#""one""#,
        ),
        Step::Call(
            "byOptions",
            r#"["en", {"minimumSignificantDigits":3}, 2]"#,
            r#""other""#,
        ),
        Step::Call(
            "byOptions",
            r#"["ro", {"minimumFractionDigits":1}, 1]"#,
            r#""few""#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_collator_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function sign(n) { return n < 0 ? -1 : n > 0 ? 1 : 0; }
function cmp(locale, opts, a, b) {
return sign(new Intl.Collator(locale, opts).compare(a, b));
}
function all() {
return [
cmp('de', {}, 'a', 'ä'),
cmp('de', {}, 'ä', 'z'),
cmp('sv', {}, 'a', 'å'),
cmp('sv', {}, 'å', 'z'),
cmp('en', { sensitivity: 'base' }, 'a', 'A'),
cmp('en', { sensitivity: 'base' }, 'e', 'é'),
cmp('en', { sensitivity: 'case' }, 'a', 'A'),
cmp('en', { numeric: true }, 'img2', 'img10'),
cmp('en', {}, 'img2', 'img10'),
cmp('en', { ignorePunctuation: true }, 'a,b', 'ab'),
];
}"#,
        ),
        Step::Call("all", r#"[]"#, r#"[-1,-1,-1,1,0,0,-1,-1,1,0]"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_segmenter_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function segs(locale, granularity, text) {
const seg = new Intl.Segmenter(locale, { granularity });
return Array.from(seg.segment(text), s => ({
segment: s.segment, index: s.index,
isWordLike: s.isWordLike === undefined ? null : s.isWordLike,
}));
}
function thaiWord() { return segs('th', 'word', 'สวัสดีชาวโลก'); }
function enWord() { return segs('en', 'word', 'Hello world!'); }
function enGrapheme() { return segs('en', 'grapheme', 'a👍bc'); }
function enSentence() { return segs('en', 'sentence', 'Hi. Bye!'); }
function containing(index) {
const seg = new Intl.Segmenter('en', { granularity: 'word' });
const s = seg.segment('Hello world').containing(index);
return s === undefined ? null : {
segment: s.segment, index: s.index,
isWordLike: s.isWordLike === undefined ? null : s.isWordLike,
};
}"#,
        ),
        Step::Call(
            "thaiWord",
            r#"[]"#,
            r#"[{"segment":"สวัสดี","index":0,"isWordLike":true},{"segment":"ชาว","index":6,"isWordLike":true},{"segment":"โลก","index":9,"isWordLike":true}]"#,
        ),
        Step::Call(
            "enWord",
            r#"[]"#,
            r#"[{"segment":"Hello","index":0,"isWordLike":true},{"segment":" ","index":5,"isWordLike":false},{"segment":"world","index":6,"isWordLike":true},{"segment":"!","index":11,"isWordLike":false}]"#,
        ),
        Step::Call(
            "enGrapheme",
            r#"[]"#,
            r#"[{"segment":"a","index":0,"isWordLike":null},{"segment":"👍","index":1,"isWordLike":null},{"segment":"b","index":3,"isWordLike":null},{"segment":"c","index":4,"isWordLike":null}]"#,
        ),
        Step::Call(
            "enSentence",
            r#"[]"#,
            r#"[{"segment":"Hi. ","index":0,"isWordLike":null},{"segment":"Bye!","index":4,"isWordLike":null}]"#,
        ),
        Step::Call(
            "containing",
            r#"[7]"#,
            r#"{"segment":"world","index":6,"isWordLike":true}"#,
        ),
        Step::Call(
            "containing",
            r#"[0]"#,
            r#"{"segment":"Hello","index":0,"isWordLike":true}"#,
        ),
        Step::Call("containing", r#"[99]"#, r#"null"#),
        Step::Call("containing", r#"[-1]"#, r#"null"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_relative_time_numbering_system_extension_and_option() {
    twin(&[
        Step::Load(
            r#"function intlRelativeNumbering() {
          const arab = new Intl.RelativeTimeFormat('en-u-nu-arab', { numeric: 'always' });
          const override = new Intl.RelativeTimeFormat('en-u-nu-latn', { numberingSystem: 'arab' });
          const fallback = new Intl.RelativeTimeFormat('en-u-nu-zzzz');
          let reads = 0;
          const once = new Intl.RelativeTimeFormat('en', {
            get numberingSystem() { reads++; return 'arab'; }
          });
          let invalid = false;
          try { new Intl.RelativeTimeFormat('en', { numberingSystem: 'bad!' }); }
          catch (error) { invalid = error instanceof RangeError; }
          return [
            arab.resolvedOptions().numberingSystem === 'arab' && arab.format(12, 'day').includes('١٢'),
            override.resolvedOptions().numberingSystem === 'arab' && override.format(-12, 'day').includes('١٢'),
            fallback.resolvedOptions().numberingSystem === 'latn' && fallback.format(12, 'day').includes('12'),
            new Intl.RelativeTimeFormat('en').resolvedOptions().numberingSystem === 'latn',
            reads === 1 && once.resolvedOptions().numberingSystem === 'arab',
            invalid,
          ];
        }"#,
        ),
        Step::Call(
            "intlRelativeNumbering",
            r#"[]"#,
            r#"[true,true,true,true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_relative_time_rejects_inherited_unit_names() {
    twin(&[
        Step::Load(
            r#"
        function inheritedRelativeUnits() {
            const formatter = new Intl.RelativeTimeFormat('en');
            return ['toString', 'constructor', '__proto__', 'hasOwnProperty'].map(unit => {
                try { formatter.format(1, unit); return 'no-throw'; }
                catch (error) { return error.name; }
            });
        }
    "#,
        ),
        Step::Call(
            "inheritedRelativeUnits",
            r#"[]"#,
            r#"["RangeError","RangeError","RangeError","RangeError"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_relative_time_format_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function fmt(locale, opts, value, unit) {
return new Intl.RelativeTimeFormat(locale, opts).format(value, unit);
}
function all() {
return [
fmt('en', {}, -1, 'day'),
fmt('en', {}, 3, 'days'),
fmt('en', { numeric: 'auto' }, -1, 'day'),
fmt('en', { style: 'short' }, -3, 'hour'),
fmt('ja', {}, -1, 'day'),
fmt('ja', { numeric: 'auto' }, -1, 'day'),
fmt('de', {}, 2, 'year'),
fmt('en', {}, 1.5, 'day'),
new Intl.RelativeTimeFormat('en').resolvedOptions().numberingSystem,
(() => { try { fmt('en', {}, Infinity, 'day'); return 'no-throw'; } catch (e) { return e.name; } })(),
(() => { try { new Intl.RelativeTimeFormat('en', { style: 'bad' }); return 'no-throw'; } catch (e) { return e.name; } })(),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["1 day ago","in 3 days","yesterday","3 hr. ago","1 日前","昨日","in 2 Jahren","in 1.5 days","latn","RangeError","RangeError"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_option_validation_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function ctorErrors() {
const cases = [
() => new Intl.Collator('en', { sensitivity: 'x' }),
() => new Intl.Collator('en', { usage: 'x' }),
() => new Intl.Collator('en', { caseFirst: 'x' }),
() => new Intl.Segmenter('en', { granularity: 'x' }),
() => new Intl.PluralRules('en', { type: 'x' }),
() => new Intl.NumberFormat('en', { minimumIntegerDigits: 0 }),
() => new Intl.NumberFormat('en', { minimumIntegerDigits: 22 }),
() => new Intl.ListFormat('en', { type: 'x' }),
() => new Intl.ListFormat('en', { style: 'x' }),
];
return cases.map(fn => { try { fn(); return 'no-throw'; } catch (error) { return error.constructor.name; } });
}
function resolved() {
return [
new Intl.Collator('en', { usage: 'search' }).resolvedOptions().usage,
new Intl.Collator('en', { caseFirst: false }).resolvedOptions().caseFirst,
new Intl.Segmenter('en', { granularity: 'word' }).resolvedOptions().granularity,
];
}"#,
        ),
        Step::Call(
            "ctorErrors",
            r#"[]"#,
            r#"["RangeError","RangeError","RangeError","RangeError","RangeError","RangeError","RangeError","RangeError","RangeError"]"#,
        ),
        Step::Call("resolved", r#"[]"#, r#"["search","false","word"]"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_rounding_mode_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function nm(mode) {
const f = n => new Intl.NumberFormat('en', { useGrouping: false, maximumFractionDigits: 0, roundingMode: mode }).format(n);
return [f(1.5), f(-1.5), f(2.5)];
}
function all() {
return ['ceil', 'floor', 'expand', 'trunc', 'halfCeil', 'halfFloor', 'halfTrunc', 'halfEven', 'halfExpand'].map(nm);
}
function plural(mode, n) {
return new Intl.PluralRules('en', { maximumFractionDigits: 0, roundingMode: mode }).select(n);
}
function pluralModes() {
return [plural('floor', 1.9), plural('ceil', 1.9), plural('halfEven', 2.5)];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"[["2","-1","3"],["1","-2","2"],["2","-2","3"],["1","-1","2"],["2","-1","3"],["1","-2","2"],["1","-1","2"],["2","-2","2"],["2","-2","3"]]"#,
        ),
        Step::Call("pluralModes", r#"[]"#, r#"["one","other","other"]"#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_time_field_independence_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })
.format(new Date(Date.UTC(2024, 0, 1, 13, 5, 45, 678)));
}
function all() {
return [
show('en-US', { second: 'numeric' }),
show('en-US', { minute: 'numeric' }),
show('en-US', { minute: 'numeric', second: 'numeric' }),
show('en-US', { minute: '2-digit' }),
show('en-US', { second: 'numeric', fractionalSecondDigits: 3 }),
show('en-US', { minute: '2-digit', second: '2-digit', fractionalSecondDigits: 2 }),
show('en-US', { second: 'numeric', timeZoneName: 'short' }),
show('de-DE', { minute: 'numeric', second: 'numeric' }),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["45","5","05:45","5","45.678","05:45.67","45 UTC","05:45"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_calendar_period_fields_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })
.format(new Date(Date.UTC(2024, 6, 4)));
}
function all() {
return [
show('en-US', { month: 'numeric' }),
show('en-US', { month: 'short' }),
show('en-US', { month: 'long' }),
show('en-US', { year: 'numeric' }),
show('en-US', { year: 'numeric', month: 'long' }),
show('en-US', { year: 'numeric', month: 'short' }),
show('de-DE', { month: 'long' }),
show('ja-JP', { month: 'long' }),
show('fr', { month: 'long' }),
show('ja-JP', { year: 'numeric', month: 'numeric' }),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["7","Jul","July","2024","July 2024","Jul 2024","Juli","7月","juillet","2024/7"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_collator_search_usage_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function cmp(locale, opts, a, b) {
const sign = n => n < 0 ? -1 : n > 0 ? 1 : 0;
return sign(new Intl.Collator(locale, { usage: 'search', ...opts }).compare(a, b));
}
function all() {
return [
cmp('en', {}, 'a', 'A'), cmp('en', {}, 'a', 'á'), cmp('en', {}, 'æ', 'ae'),
cmp('en', {}, 'ß', 'ss'), cmp('en', {}, 'e', 'é'),
cmp('de', {}, 'a', 'ä'), cmp('de', {}, 'ss', 'ß'),
cmp('en', { sensitivity: 'base' }, 'a', 'á'),
cmp('en', {}, 'ا', 'أ'), cmp('ja', {}, 'あ', 'ア'),
];
}
function usageOption() {
return new Intl.Collator('en', { usage: 'search' }).resolvedOptions().usage;
}"#,
        ),
        Step::Call("all", r#"[]"#, r#"[-1,-1,1,1,-1,-1,-1,0,-1,-1]"#),
        Step::Call("usageOption", r#"[]"#, r#""search""#),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_narrow_icu4c_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })
.format(new Date(Date.UTC(2024, 6, 4)));
}
function all() {
return [
show('en-US', { month: 'narrow' }),
show('en-US', { month: 'narrow', day: 'numeric' }),
show('en-US', { month: 'narrow', year: 'numeric' }),
show('en-US', { weekday: 'narrow' }),
show('en-US', { weekday: 'narrow', day: 'numeric' }),
show('en-US', { weekday: 'narrow', month: 'narrow', day: 'numeric' }),
show('de-DE', { month: 'narrow', day: 'numeric' }),
show('de-DE', { weekday: 'narrow', day: 'numeric' }),
show('ja-JP', { month: 'narrow' }),
show('ja-JP', { month: 'narrow', day: 'numeric' }),
show('ja-JP', { weekday: 'narrow' }),
show('fr', { month: 'narrow', day: 'numeric' }),
show('fr', { weekday: 'narrow', day: 'numeric' }),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["J","J 4","J 2024","T","4 T","T, J 4","4. J","D, 4.","7月","7月4日","木","4 J","J 4"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_format_to_parts_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function parts(locale, opts, value) {
return new Intl.NumberFormat(locale, opts).formatToParts(value);
}
function all() {
return [
parts('en-US', {}, 1234.5),
parts('en-US', {}, -1234.5),
parts('en-US', { style: 'percent' }, 0.56),
parts('en-US', { style: 'currency', currency: 'USD' }, 1234.5),
parts('de-DE', { style: 'currency', currency: 'EUR' }, -1234.5),
parts('en-US', { style: 'unit', unit: 'day' }, 3),
parts('en-US', { style: 'currency', currency: 'USD', currencySign: 'accounting' }, -5),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"[[{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"}],[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"}],[{"type":"integer","value":"56"},{"type":"percentSign","value":"%"}],[{"type":"currency","value":"$"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"50"}],[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"group","value":"."},{"type":"integer","value":"234"},{"type":"decimal","value":","},{"type":"fraction","value":"50"},{"type":"literal","value":" "},{"type":"currency","value":"€"}],[{"type":"integer","value":"3"},{"type":"literal","value":" "},{"type":"unit","value":"days"}],[{"type":"literal","value":"("},{"type":"currency","value":"$"},{"type":"integer","value":"5"},{"type":"decimal","value":"."},{"type":"fraction","value":"00"},{"type":"literal","value":")"}]]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_number_format_to_parts_compact_and_scientific_match_real_node() {
    twin(&[
        Step::Load(
            r#"function partsFor(locale, opts, value) {
return new Intl.NumberFormat(locale, opts).formatToParts(value);
}
function sci() { return partsFor('en-US', { notation: 'scientific' }, 1234); }
function eng() { return partsFor('en-US', { notation: 'engineering' }, 12345); }
function scineg() { return partsFor('en-US', { notation: 'scientific' }, -1234.5); }
function scinegexp() { return partsFor('en-US', { notation: 'scientific' }, 0.001234); }
function compact() { return partsFor('en-US', { notation: 'compact' }, 1234); }
function compactde() { return partsFor('de-DE', { notation: 'compact' }, 1234); }
function compactde2() { return partsFor('de-DE', { notation: 'compact' }, 123456); }
function compactja() { return partsFor('ja-JP', { notation: 'compact' }, 15127); }
function compactlong() { return partsFor('en-US', { notation: 'compact', compactDisplay: 'long' }, 1234); }"#,
        ),
        Step::Call(
            "sci",
            r#"[]"#,
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"234"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#,
        ),
        Step::Call(
            "eng",
            r#"[]"#,
            r#"[{"type":"integer","value":"12"},{"type":"decimal","value":"."},{"type":"fraction","value":"345"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#,
        ),
        Step::Call(
            "scineg",
            r#"[]"#,
            r#"[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"235"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#,
        ),
        Step::Call(
            "scinegexp",
            r#"[]"#,
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"234"},{"type":"exponentSeparator","value":"E"},{"type":"exponentMinusSign","value":"-"},{"type":"exponentInteger","value":"3"}]"#,
        ),
        Step::Call(
            "compact",
            r#"[]"#,
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"2"},{"type":"compact","value":"K"}]"#,
        ),
        Step::Call(
            "compactde",
            r#"[]"#,
            r#"[{"type":"integer","value":"1234"}]"#,
        ),
        Step::Call(
            "compactde2",
            r#"[]"#,
            r#"[{"type":"integer","value":"123"},{"type":"group","value":"."},{"type":"integer","value":"456"}]"#,
        ),
        Step::Call(
            "compactja",
            r#"[]"#,
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"},{"type":"compact","value":"万"}]"#,
        ),
        Step::Call(
            "compactlong",
            r#"[]"#,
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"2"},{"type":"literal","value":" "},{"type":"compact","value":"thousand"}]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_range_fallback_partitions_endpoints_like_real_node() {
    twin(&[
        Step::Load(
            r#"function dateRange() {
return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' })
.formatRange(new Date(Date.UTC(2024, 6, 4)), new Date(Date.UTC(2024, 6, 5)));
}
function dateParts() {
return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' })
.formatRangeToParts(new Date(Date.UTC(2024, 6, 4)), new Date(Date.UTC(2024, 6, 5)));
}
function num(locale, opts, start, end) {
return new Intl.NumberFormat(locale, opts).formatRange(start, end);
}
function numParts() {
return new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day' }).formatRangeToParts(3, 5);
}
function currencyParts() {
return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).formatRangeToParts(3, 5);
}
function all() {
return [
num('en-US', {}, 3, 5),
num('en-US', {}, 3, 3),
num('en-US', { style: 'currency', currency: 'USD' }, 3, 5),
num('en-US', { style: 'percent' }, 0.03, 0.05),
num('en-US', { style: 'unit', unit: 'day' }, 3, 5),
num('en-US', { notation: 'compact' }, 3000, 5000),
];
}"#,
        ),
        Step::Call("dateRange", r#"[]"#, r#""Jul 4 – 5, 2024""#),
        Step::Call(
            "dateParts",
            r#"[]"#,
            r#"[{"type":"month","value":"Jul","source":"shared"},{"type":"literal","value":" ","source":"shared"},{"type":"day","value":"4","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"day","value":"5","source":"endRange"},{"type":"literal","value":", ","source":"shared"},{"type":"year","value":"2024","source":"shared"}]"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["3–5","~3","$3.00 – $5.00","3% – 5%","3–5 days","3K – 5K"]"#,
        ),
        Step::Call(
            "numParts",
            r#"[]"#,
            r#"[{"type":"integer","value":"3","source":"startRange"},{"type":"literal","value":"–","source":"shared"},{"type":"integer","value":"5","source":"endRange"},{"type":"literal","value":" ","source":"shared"},{"type":"unit","value":"days","source":"shared"}]"#,
        ),
        Step::Call(
            "currencyParts",
            r#"[]"#,
            r#"[{"type":"currency","value":"$","source":"startRange"},{"type":"integer","value":"3","source":"startRange"},{"type":"decimal","value":".","source":"startRange"},{"type":"fraction","value":"00","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"currency","value":"$","source":"endRange"},{"type":"integer","value":"5","source":"endRange"},{"type":"decimal","value":".","source":"endRange"},{"type":"fraction","value":"00","source":"endRange"}]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_skeleton_matching_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4);
return [
show('de-DE', {year:'numeric',month:'short',day:'numeric'}, t),
show('de-DE', {year:'numeric',month:'2-digit',day:'numeric'}, t),
show('ja-JP', {year:'numeric',month:'short',day:'numeric'}, t),
show('ko', {year:'numeric',month:'short',day:'numeric'}, t),
show('ar', {year:'numeric',month:'short',day:'numeric'}, t),
show('fr', {year:'numeric',month:'2-digit',day:'numeric'}, t),
show('ru', {year:'numeric',month:'2-digit',day:'numeric'}, t),
show('en-US', {year:'numeric',month:'short',day:'numeric'}, t),
show('en-US', {weekday:'long'}, t),
show('ca', {year:'numeric',month:'short',day:'numeric'}, t),
show('hr', {year:'numeric',month:'long',day:'numeric'}, t),
show('sk', {year:'numeric',month:'short',day:'numeric'}, t),
show('bg', {year:'numeric',month:'long',day:'numeric'}, t),
show('ko', {hour:'numeric',minute:'numeric'}, Date.UTC(2024, 6, 4, 16, 30)),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["4. Juli 2024","4.07.2024","2024年7月4日","2024년 7월 4일","4 يوليو 2024","4/07/2024","4.07.2024","Jul 4, 2024","Thursday","4 de jul. del 2024","4. srpnja 2024.","4. 7. 2024","4 юли 2024 г.","PM 4:30"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_default_fields_and_era_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4);
return [
show('en-US', {}, t),
show('en-US', {era:'long'}, t),
show('en-GB', {year:'numeric',month:'numeric',day:'numeric'}, t),
show('en-GB', {era:'long'}, t),
show('fr', {year:'numeric',month:'numeric',day:'numeric'}, t),
show('de', {era:'long'}, t),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["7/4/2024","7/4/2024 Anno Domini","04/07/2024","04/07/2024 Anno Domini","04/07/2024","04.07.2024 n. Chr."]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_weekday_context_and_ms_separator_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4, 16, 30, 45);
return [
show('fi', {weekday:'long',year:'numeric',month:'long',day:'numeric'}, t),
show('id', {minute:'numeric',second:'numeric'}, t),
show('da', {minute:'numeric',second:'numeric'}, t),
show('fi', {minute:'numeric',second:'numeric'}, t),
show('en-US', {minute:'2-digit'}, t),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["torstaina 4. heinäkuuta 2024","30.45","30.45","30.45","30"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_fa_persian_defaults_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4);
return [
show('fa', {year:'numeric',month:'long',day:'numeric'}, t),
show('fa', {year:'numeric',month:'short',day:'numeric'}, t),
show('fa', {year:'numeric',month:'numeric',day:'numeric'}, t),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["۱۴ تیر ۱۴۰۳","۱۴ تیر ۱۴۰۳","۱۴۰۳/۴/۱۴"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_chinese_related_year_digits_match_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4);
const o = { calendar: 'chinese', year: 'numeric', month: 'long', day: 'numeric' };
return [show('ar-SA', o, t), show('fa', o, t), show('zh', o, t)];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["٢٠٢٤(jia-chen) M05 ٢٩","۲۰۲۴(jia-chen) M05 ۲۹","2024甲辰年五月29"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_patternmap_selection_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const t = Date.UTC(2024, 6, 4, 16, 30, 45);
return [
show('bg', {hour:'numeric',minute:'numeric'}, t),
show('bg', {hour:'2-digit',minute:'2-digit'}, t),
show('bg', {hour:'numeric',minute:'numeric',second:'numeric'}, t),
show('cs', {year:'numeric',month:'2-digit',day:'2-digit'}, t),
show('it', {year:'numeric',month:'2-digit',day:'numeric'}, t),
show('uk', {year:'numeric',month:'2-digit',day:'numeric'}, t),
show('fr', {year:'numeric',month:'numeric',day:'numeric'}, t),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["16:30 ч.","16:30","16:30:45 ч.","04. 07. 2024","04/07/2024","04.07.2024","04/07/2024"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_hebrew_ordinal_month_matches_real_node() {
    twin(&[
        Step::Load(
            r#"function show(locale, opts, ms) {
return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));
}
function all() {
const opts = { calendar: 'hebrew', year: 'numeric', month: 'long', day: 'numeric' };
return [
show('ja', opts, Date.UTC(2024, 6, 4)),
show('ja', opts, Date.UTC(2023, 6, 4)),
show('ja', opts, Date.UTC(2024, 1, 15)),
show('ja', opts, Date.UTC(2024, 2, 15)),
show('ja', { calendar: 'hebrew', year: 'numeric', month: 'numeric', day: 'numeric' }, Date.UTC(2024, 8, 15)),
];
}"#,
        ),
        Step::Call(
            "all",
            r#"[]"#,
            r#"["AM5784年10月28日","AM5783年10月15日","AM5784年6月6日","AM5784年7月5日","AM5784/13/12"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_followup_exact_notation_and_style_grouping() {
    twin(&[
        Step::Load(
            r#"function intlDeferredNumberCases() {
          const scientific = new Intl.NumberFormat('en-US', { notation: 'scientific', maximumFractionDigits: 15 });
          const engineering = new Intl.NumberFormat('en-US', { notation: 'engineering', maximumFractionDigits: 18 });
          const big = 9007199254740993n;
          const groupedCurrency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', useGrouping: false });
          const groupedPercent = new Intl.NumberFormat('en-US', { style: 'percent', useGrouping: false });
          const groupedUnit = new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day', useGrouping: false });
          const wideExponent = new Intl.NumberFormat('en-US', { notation: 'scientific' });
          const range = wideExponent.formatRange('1e20000', '2e20000');
          return [
            scientific.format(big),
            scientific.format('9007199254740993'),
            scientific.formatToParts(big).map(part => part.value).join(''),
            engineering.format('123456789012345678901'),
            new Intl.NumberFormat('en-US', { notation: 'engineering', maximumFractionDigits: 0 }).format(999999999999999999999n),
            groupedCurrency.format(1234),
            groupedPercent.format(1234.56),
            groupedUnit.format(1234).includes(','),
            wideExponent.formatToParts('1e20000').map(part => part.value).join('') === wideExponent.format('1e20000'),
            wideExponent.formatToParts('1e-20000').map(part => part.value).join('') === wideExponent.format('1e-20000'),
            wideExponent.formatRangeToParts('1e20000', '2e20000').map(part => part.value).join('') === range && !range.includes('Infinity'),
          ];
        }"#,
        ),
        Step::Call(
            "intlDeferredNumberCases",
            r#"[]"#,
            r#"["9.007199254740993E15","9.007199254740993E15","9.007199254740993E15","123.456789012345678901E18","1E21","$1234.00","123456%",false,true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_followup_flexible_day_period_without_icu4c() {
    twin(&[
        Step::Load(
            r#"function intlFlexibleDayPeriod() {
          const date = new Date('2024-07-04T15:00:00Z');
          const options = { hour: 'numeric', dayPeriod: 'long', timeZone: 'UTC' };
          return new Intl.DateTimeFormat('en-US', options).formatToParts(date)
            .some(part => part.type === 'dayPeriod' && part.value.toLowerCase().includes('afternoon'));
        }"#,
        ),
        Step::Call("intlFlexibleDayPeriod", r#"[]"#, r#"true"#),
    ]);
}

#[test]
fn twin_stream_start_exceptions_escape_constructors_synchronously() {
    twin(&[
        Step::Load(
            r#"
      function streamStartThrowsSynchronously() {
        const sentinel = new Error('start failure');
        const throwsSame = construct => { try { construct(); return false; } catch (error) { return error === sentinel; } };
        return [
          throwsSame(() => new ReadableStream({ start() { throw sentinel; } })),
          throwsSame(() => new WritableStream({ start() { throw sentinel; } })),
          throwsSame(() => new TransformStream({ start() { throw sentinel; } }))
        ];
      }
    "#,
        ),
        Step::Call(
            "streamStartThrowsSynchronously",
            r#"[]"#,
            r#"[true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writable_pending_writes_skip_sink_after_error_or_abort() {
    twin(&[
        Step::Load(
            r#"
      async function writablePendingWriteStops() {
        const events = [], error = new Error('controller');
        let controller;
        const writer = new WritableStream({
          start(value) { controller = value; },
          write(chunk) { events.push(['write', chunk]); },
          abort(reason) { events.push(['abort', reason]); }
        }).getWriter();
        const pending = writer.write('skipped');
        controller.error(error);
        const writeError = await pending.then(() => false, reason => reason === error);
        const closedError = await writer.closed.then(() => false, reason => reason === error);
        const second = new WritableStream({
          write(chunk) { events.push(['second-write', chunk]); },
          abort(reason) { events.push(['second-abort', reason]); }
        }).getWriter();
        const queued = second.write('skipped');
        const abort = second.abort('stop');
        const queuedError = await queued.then(() => false, reason => reason === 'stop');
        await abort;
        const abortClosed = await second.closed.then(() => false, reason => reason === 'stop');
        return [events, writeError, closedError, queuedError, abortClosed];
      }
    "#,
        ),
        Step::Call(
            "writablePendingWriteStops",
            r#"[]"#,
            r#"[[["second-abort","stop"]],true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writable_queued_writes_drain_before_close_and_reject_on_failure() {
    twin(&[
        Step::Load(
            r#"
      async function writableCloseAfterQueuedWrites() {
        const events = [];
        const writer = new WritableStream({
          write(chunk) { events.push(chunk); },
          close() { events.push('close'); }
        }).getWriter();
        const first = writer.write('a'), second = writer.write('b'), close = writer.close();
        await Promise.all([first, second, close, writer.closed]);
        const failure = new Error('write failure');
        const failedWriter = new WritableStream({
          write() { throw failure; },
          close() { events.push('wrong-close'); }
        }).getWriter();
        const failedWrite = failedWriter.write('x'), failedClose = failedWriter.close();
        return [events,
          await failedWrite.then(() => false, error => error === failure),
          await failedClose.then(() => false, error => error === failure),
          await failedWriter.closed.then(() => false, error => error === failure)];
      }
    "#,
        ),
        Step::Call(
            "writableCloseAfterQueuedWrites",
            r#"[]"#,
            r#"[["a","b","close"],true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writable_abort_failure_settles_writer_closed_with_original_reason() {
    twin(&[
        Step::Load(
            r#"
      async function writableAbortFailureSettlement() {
        const reason = new Error('abort reason'), failure = new Error('sink failure');
        const events = [];
        const writer = new WritableStream({ abort(value) { events.push(value === reason); throw failure; } }).getWriter();
        const abortFailure = await writer.abort(reason).then(() => false, error => error === failure);
        const closedReason = await writer.closed.then(() => false, error => error === reason);
        const secondAbort = await writer.abort('again').then(() => true, () => false);
        return [events, abortFailure, closedReason, secondAbort];
      }
    "#,
        ),
        Step::Call(
            "writableAbortFailureSettlement",
            r#"[]"#,
            r#"[[true],true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writable_abort_during_in_flight_close_does_not_call_sink_abort() {
    twin(&[
        Step::Load(
            r#"
      async function writableAbortDuringClose() {
        const events = [];
        let finish, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const writer = new WritableStream({
          close() { events.push('close'); resolveEntered(); return new Promise(resolve => { finish = resolve; }); },
          abort() { events.push('wrong-abort'); }
        }).getWriter();
        const closing = writer.close();
        await entered;
        const aborting = writer.abort('stop');
        finish();
        await Promise.all([closing, aborting, writer.closed]);
        return events;
      }
    "#,
        ),
        Step::Call("writableAbortDuringClose", r#"[]"#, r#"["close"]"#),
    ]);
}

#[test]
fn twin_writable_close_queued_keeps_controller_desired_size() {
    twin(&[
        Step::Load(
            r#"
      function writableDesiredSizeWhileClosing() {
        const writer = new WritableStream({ write() {} }, { highWaterMark: 3 }).getWriter();
        const pending = writer.write('x');
        const before = writer.desiredSize;
        const closing = writer.close();
        const during = writer.desiredSize;
        pending.catch(() => {}); closing.catch(() => {});
        return [before, during];
      }
    "#,
        ),
        Step::Call("writableDesiredSizeWhileClosing", r#"[]"#, r#"[2,2]"#),
    ]);
}

#[test]
fn twin_writable_in_flight_close_success_overrides_controller_error() {
    twin(&[
        Step::Load(
            r#"
      async function writableCloseWinsControllerError() {
        let controller, finish, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const error = new Error('controller');
        const writer = new WritableStream({
          start(value) { controller = value; },
          close() { resolveEntered(); return new Promise(resolve => { finish = resolve; }); }
        }).getWriter();
        const closing = writer.close();
        await entered;
        controller.error(error);
        finish();
        const closeResult = await closing.then(() => true, () => false);
        const closedResult = await writer.closed.then(() => true, () => false);
        return [closeResult, closedResult, writer.desiredSize];
      }
    "#,
        ),
        Step::Call(
            "writableCloseWinsControllerError",
            r#"[]"#,
            r#"[true,true,0]"#,
        ),
    ]);
}

#[test]
fn twin_writable_in_flight_close_failure_preserves_stored_error() {
    twin(&[
        Step::Load(
            r#"
      async function writableCloseFailsAfterAbort() {
        let controller, fail, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const reason = new Error('abort'), closeError = new Error('close');
        const writer = new WritableStream({
          start(value) { controller = value; },
          close() { resolveEntered(); return new Promise((resolve, reject) => { fail = reject; }); },
          abort() { throw new Error('must not run'); }
        }).getWriter();
        const closing = writer.close();
        await entered;
        const aborting = writer.abort(reason);
        fail(closeError);
        return [
          await closing.then(() => false, error => error === closeError),
          await aborting.then(() => false, error => error === closeError),
          await writer.closed.then(() => false, error => error === reason),
          controller.signal.aborted
        ];
      }
    "#,
        ),
        Step::Call(
            "writableCloseFailsAfterAbort",
            r#"[]"#,
            r#"[true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writable_in_flight_close_failure_after_controller_error() {
    twin(&[
        Step::Load(
            r#"
      async function writableCloseFailsAfterControllerError() {
        let controller, fail, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const primary = new Error('controller'), closeError = new Error('close');
        const writer = new WritableStream({
          start(value) { controller = value; },
          close() { resolveEntered(); return new Promise((resolve, reject) => { fail = reject; }); }
        }).getWriter();
        const closing = writer.close();
        await entered;
        controller.error(primary);
        fail(closeError);
        return [
          await closing.then(() => false, error => error === closeError),
          await writer.closed.then(() => false, error => error === primary),
          writer.desiredSize
        ];
      }
    "#,
        ),
        Step::Call(
            "writableCloseFailsAfterControllerError",
            r#"[]"#,
            r#"[true,true,null]"#,
        ),
    ]);
}

#[test]
fn twin_headers_iterators_observe_mutation_and_resume_after_done() {
    twin(&[
        Step::Load(
            r#"
      function headersIteratorsObserveMutation() {
        const headers = new Headers([['b', 'B'], ['d', 'D']]);
        const keys = headers.keys(), values = headers.values(), entries = headers.entries();
        const first = [keys.next().value, values.next().value, entries.next().value];
        headers.append('c', 'C');
        const second = [keys.next().value, values.next().value, entries.next().value];
        headers.delete('b');
        const third = [keys.next().value, values.next().value, entries.next().value];
        const done = [keys.next().done, values.next().done, entries.next().done];
        headers.append('e', 'E');
        const resumed = [keys.next().value, values.next().value, entries.next().value];
        return [first, second, third, done, resumed];
      }
    "#,
        ),
        Step::Call(
            "headersIteratorsObserveMutation",
            r#"[]"#,
            r#"[["b","B",["b","B"]],["c","C",["c","C"]],[null,null,null],[true,true,true],["e","E",["e","E"]]]"#,
        ),
    ]);
}

#[test]
fn twin_form_data_iterators_observe_set_and_resume_after_done() {
    twin(&[
        Step::Load(
            r#"
      function formDataIteratorsObserveSet() {
        const data = new FormData();
        data.append('a', 'A'); data.append('b', 'B'); data.append('c', 'C');
        const keys = data.keys(), values = data.values(), entries = data.entries(), defaultIterator = data[Symbol.iterator]();
        const first = [keys.next().value, values.next().value, entries.next().value, defaultIterator.next().value];
        data.set('b', 'B2');
        const second = [keys.next().value, values.next().value, entries.next().value, defaultIterator.next().value];
        const third = [keys.next().value, values.next().value, entries.next().value, defaultIterator.next().value];
        const done = [keys.next().done, values.next().done, entries.next().done, defaultIterator.next().done];
        data.append('d', 'D');
        const resumed = [keys.next().value, values.next().value, entries.next().value, defaultIterator.next().value];
        return [first, second, third, done, resumed];
      }
    "#,
        ),
        Step::Call(
            "formDataIteratorsObserveSet",
            r#"[]"#,
            r#"[["a","A",["a","A"],["a","A"]],["b","B2",["b","B2"],["b","B2"]],["c","C",["c","C"],["c","C"]],[true,true,true,true],["d","D",["d","D"],["d","D"]]]"#,
        ),
    ]);
}

#[test]
fn twin_readable_reader_closed_promises_follow_each_lock() {
    twin(&[
        Step::Load(
            r#"
      async function readableReaderClosedPromisesFollowLock() {
        let controller;
        const stream = new ReadableStream({ start(value) { controller = value; } });
        const first = stream.getReader(), firstClosed = first.closed;
        first.releaseLock();
        const firstReleased = await firstClosed.then(() => false, error => error instanceof TypeError);
        const firstAfterRelease = await first.closed.then(() => false, error => error instanceof TypeError);
        const second = stream.getReader(), secondClosed = second.closed;
        controller.close();
        const secondResolved = await secondClosed.then(() => true, () => false);
        second.releaseLock();
        const settledStayedResolved = await secondClosed.then(() => true, () => false);
        const releasedGetter = await second.closed.then(() => false, error => error instanceof TypeError);
        const third = stream.getReader();
        const closedLock = await third.closed.then(() => true, () => false);
        third.releaseLock();
        return [firstReleased, firstAfterRelease, secondResolved, settledStayedResolved, releasedGetter, closedLock];
      }
    "#,
        ),
        Step::Call(
            "readableReaderClosedPromisesFollowLock",
            r#"[]"#,
            r#"[true,true,true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_byob_reader_closed_promises_follow_each_lock_and_error() {
    twin(&[
        Step::Load(
            r#"
      async function byobReaderClosedPromisesFollowLock() {
        let controller;
        const reason = new Error('source');
        const stream = new ReadableStream({ type: 'bytes', start(value) { controller = value; } });
        const first = stream.getReader({ mode: 'byob' }), firstClosed = first.closed;
        first.releaseLock();
        const firstReleased = await firstClosed.then(() => false, error => error instanceof TypeError);
        const second = stream.getReader({ mode: 'byob' }), secondClosed = second.closed;
        controller.error(reason);
        const secondErrored = await secondClosed.then(() => false, error => error === reason);
        second.releaseLock();
        const errorStayed = await secondClosed.then(() => false, error => error === reason);
        const releasedGetter = await second.closed.then(() => false, error => error instanceof TypeError);
        const third = stream.getReader({ mode: 'byob' });
        const erroredLock = await third.closed.then(() => false, error => error === reason);
        third.releaseLock();
        return [firstReleased, secondErrored, errorStayed, releasedGetter, erroredLock];
      }
    "#,
        ),
        Step::Call(
            "byobReaderClosedPromisesFollowLock",
            r#"[]"#,
            r#"[true,true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writer_ready_and_closed_promises_follow_each_lock() {
    twin(&[
        Step::Load(
            r#"
      async function writerReadyAndClosedPromisesFollowLock() {
        const reason = new Error('abort');
        const stream = new WritableStream({}, { highWaterMark: 0 });
        const first = stream.getWriter(), firstReady = first.ready, firstClosed = first.closed;
        first.releaseLock();
        const releasedReady = await firstReady.then(() => false, error => error instanceof TypeError);
        const releasedClosed = await firstClosed.then(() => false, error => error instanceof TypeError);
        const firstAfterRelease = await first.ready.then(() => false, error => error instanceof TypeError);
        const second = stream.getWriter(), secondReady = second.ready, secondClosed = second.closed;
        await second.abort(reason);
        const abortReady = await secondReady.then(() => false, error => error === reason);
        const abortClosed = await secondClosed.then(() => false, error => error === reason);
        second.releaseLock();
        const releasedGetter = await second.closed.then(() => false, error => error instanceof TypeError);
        const third = stream.getWriter();
        const erroredLock = await third.closed.then(() => false, error => error === reason);
        third.releaseLock();
        return [releasedReady, releasedClosed, firstAfterRelease, abortReady, abortClosed, releasedGetter, erroredLock];
      }
    "#,
        ),
        Step::Call(
            "writerReadyAndClosedPromisesFollowLock",
            r#"[]"#,
            r#"[true,true,true,true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writer_backpressure_reassignment_keeps_old_promise_rejected() {
    twin(&[
        Step::Load(
            r#"
      async function writerBackpressureReassignment() {
        let resolveWrite, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const stream = new WritableStream({ write() { resolveEntered(); return new Promise(resolve => { resolveWrite = resolve; }); } });
        const first = stream.getWriter(), initialReady = first.ready;
        const writing = first.write('chunk');
        const pendingReady = first.ready, pendingClosed = first.closed;
        await entered;
        first.releaseLock();
        const oldReady = await pendingReady.then(() => false, error => error instanceof TypeError);
        const oldClosed = await pendingClosed.then(() => false, error => error instanceof TypeError);
        const settledInitial = await initialReady.then(() => true, () => false);
        const second = stream.getWriter(), nextReady = second.ready, nextClosed = second.closed;
        resolveWrite();
        await writing;
        const resumedReady = await nextReady.then(() => true, () => false);
        await second.close();
        const finishedClosed = await nextClosed.then(() => true, () => false);
        second.releaseLock();
        const releasedGetter = await second.ready.then(() => false, error => error instanceof TypeError);
        const third = stream.getWriter();
        const closedLock = await Promise.all([third.ready, third.closed]).then(() => true, () => false);
        third.releaseLock();
        return [oldReady, oldClosed, settledInitial, resumedReady, finishedClosed, releasedGetter, closedLock];
      }
    "#,
        ),
        Step::Call(
            "writerBackpressureReassignment",
            r#"[]"#,
            r#"[true,true,true,true,true,true,true]"#,
        ),
    ]);
}

#[test]
fn twin_writer_relock_during_abort_waits_for_closed_settlement() {
    twin(&[
        Step::Load(
            r#"
      async function writerRelockDuringAbort() {
        let resolveAbort, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const reason = new Error('abort');
        const stream = new WritableStream({ abort() { resolveEntered(); return new Promise(resolve => { resolveAbort = resolve; }); } });
        const first = stream.getWriter(), firstClosed = first.closed;
        const aborting = first.abort(reason);
        await entered;
        first.releaseLock();
        const firstReleased = await firstClosed.then(() => false, error => error instanceof TypeError);
        const second = stream.getWriter(), secondClosed = second.closed;
        const secondReady = await second.ready.then(() => false, error => error === reason);
        resolveAbort();
        await aborting;
        const secondReason = await secondClosed.then(() => false, error => error === reason);
        second.releaseLock();
        return [firstReleased, secondReady, secondReason];
      }
    "#,
        ),
        Step::Call("writerRelockDuringAbort", r#"[]"#, r#"[true,true,true]"#),
    ]);
}

#[test]
fn twin_tee_preserves_falsy_error_reasons_in_default_and_byte_branches() {
    twin(&[
        Step::Load(
            r#"
      async function teePreservesFalsyErrors() {
        const reasons = [0, false, '', null, undefined, NaN];
        const results = [];
        for (const bytes of [false, true]) {
          for (const reason of reasons) {
            const source = new ReadableStream({ type: bytes ? 'bytes' : undefined,
              start(controller) { controller.error(reason); } });
            const branches = source.tee();
            results.push(await Promise.all(branches.map(branch => branch.getReader().read()
              .then(() => false, error => Object.is(error, reason)))));
          }
        }
        return results;
      }
    "#,
        ),
        Step::Call(
            "teePreservesFalsyErrors",
            r#"[]"#,
            r#"[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]"#,
        ),
    ]);
}

#[test]
fn twin_tee_closes_normally_and_keeps_cancellation_separate_from_errors() {
    twin(&[
        Step::Load(
            r#"
      async function teeCloseAndCancelPaths() {
        const results = [];
        for (const bytes of [false, true]) {
          const closed = new ReadableStream({ type: bytes ? 'bytes' : undefined,
            start(controller) { controller.close(); } }).tee();
          results.push(await Promise.all(closed.map(branch => branch.getReader().read()
            .then(result => result.done, () => false))));

          let controller;
          const source = new ReadableStream({ type: bytes ? 'bytes' : undefined,
            start(value) { controller = value; } });
          const [left, right] = source.tee();
          const canceled = left.cancel('left');
          const reading = right.getReader().read();
          controller.error(0);
          results.push([await canceled.then(() => true, () => false),
            await reading.then(() => false, error => Object.is(error, 0))]);

          let reasons;
          const both = new ReadableStream({ type: bytes ? 'bytes' : undefined,
            cancel(value) { reasons = value; } }).tee();
          await Promise.all([both[0].cancel(false), both[1].cancel(0)]);
          results.push([Object.is(reasons[0], false), Object.is(reasons[1], 0)]);
        }
        return results;
      }
    "#,
        ),
        Step::Call(
            "teeCloseAndCancelPaths",
            r#"[]"#,
            r#"[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]"#,
        ),
    ]);
}

#[test]
fn twin_pipe_to_detects_destination_error_while_read_is_pending() {
    twin(&[
        Step::Load(
            r#"
      async function pipeToDetectsDestinationError(preventCancel) {
        let destinationController, resolvePull, canceled;
        const pullEntered = new Promise(resolve => { resolvePull = resolve; });
        const source = new ReadableStream({
          pull() { resolvePull(); },
          cancel(reason) { canceled = reason; }
        }, { highWaterMark: 0 });
        const destination = new WritableStream({ start(controller) { destinationController = controller; } });
        const error = new Error('destination');
        const piping = source.pipeTo(destination, { preventCancel });
        await pullEntered;
        destinationController.error(error);
        const rejected = await piping.then(() => false, reason => reason === error);
        return [rejected, canceled === error, source.locked, destination.locked];
      }
    "#,
        ),
        Step::Call(
            "pipeToDetectsDestinationError",
            r#"[false]"#,
            r#"[true,true,false,false]"#,
        ),
        Step::Call(
            "pipeToDetectsDestinationError",
            r#"[true]"#,
            r#"[true,false,false,false]"#,
        ),
    ]);
}

#[test]
fn twin_pipe_to_observes_source_termination_while_backpressured() {
    twin(&[
        Step::Load(
            r#"
      async function pipeToObservesSourceTermination(errored) {
        let sourceController, abortReason, closeCount = 0;
        const source = new ReadableStream({ start(controller) { sourceController = controller; } }, { highWaterMark: 0 });
        const destination = new WritableStream({
          abort(reason) { abortReason = reason; },
          close() { closeCount++; }
        }, { highWaterMark: 0 });
        const reason = new Error('source');
        const piping = source.pipeTo(destination);
        if (errored) sourceController.error(reason);
        else sourceController.close();
        const outcome = await piping.then(() => 'resolved', error => error === reason ? 'source-error' : 'other-error');
        return [outcome, abortReason === reason, closeCount, source.locked, destination.locked];
      }
    "#,
        ),
        Step::Call(
            "pipeToObservesSourceTermination",
            r#"[false]"#,
            r#"["resolved",false,1,false,false]"#,
        ),
        Step::Call(
            "pipeToObservesSourceTermination",
            r#"[true]"#,
            r#"["source-error",true,0,false,false]"#,
        ),
    ]);
}

#[test]
fn twin_pipe_to_rejects_an_already_closed_destination() {
    twin(&[
        Step::Load(
            r#"
      async function pipeToRejectsClosedDestination() {
        let canceled;
        const source = new ReadableStream({ cancel(reason) { canceled = reason; } }, { highWaterMark: 0 });
        const destination = new WritableStream();
        await destination.close();
        const outcome = await source.pipeTo(destination).then(() => false, error => error instanceof TypeError);
        return [outcome, canceled instanceof TypeError, source.locked, destination.locked];
      }
    "#,
        ),
        Step::Call(
            "pipeToRejectsClosedDestination",
            r#"[]"#,
            r#"[true,true,false,false]"#,
        ),
    ]);
}

#[test]
fn twin_pipe_to_respects_ordered_initial_close_and_pending_destination_close() {
    twin(&[
        Step::Load(
            r#"
      async function pipeToOrderedCloseStates() {
        const source = new ReadableStream({ start(controller) { controller.close(); } });
        const destination = new WritableStream();
        await destination.close();
        const bothClosed = await source.pipeTo(destination).then(() => true, () => false);

        let canceled, resolveClose, resolveEntered;
        const entered = new Promise(resolve => { resolveEntered = resolve; });
        const liveSource = new ReadableStream({ cancel(reason) { canceled = reason; } }, { highWaterMark: 0 });
        const closingDestination = new WritableStream({ close() {
          resolveEntered(); return new Promise(resolve => { resolveClose = resolve; });
        } });
        const closing = closingDestination.close();
        const prematureClose = await liveSource.pipeTo(closingDestination)
          .then(() => false, error => error instanceof TypeError);
        await entered;
        resolveClose();
        await closing;
        return [bothClosed, prematureClose, canceled instanceof TypeError,
          source.locked, destination.locked, liveSource.locked, closingDestination.locked];
      }
    "#,
        ),
        Step::Call(
            "pipeToOrderedCloseStates",
            r#"[]"#,
            r#"[true,true,true,false,false,false,false]"#,
        ),
    ]);
}

#[test]
fn twin_pipe_to_preserves_prevent_flags_and_falsy_destination_reason() {
    twin(&[
        Step::Load(
            r#"
      async function pipeToPreventFlagsAndFalsyReason() {
        let sourceController, abortCount = 0, closeCount = 0;
        const failedSource = new ReadableStream({ start(controller) { sourceController = controller; } }, { highWaterMark: 0 });
        const destination = new WritableStream({ abort() { abortCount++; } });
        sourceController.error(false);
        const sourceError = await failedSource.pipeTo(destination, { preventAbort: true })
          .then(() => false, reason => reason === false);

        const closedSource = new ReadableStream({ start(controller) { controller.close(); } });
        const openDestination = new WritableStream({ close() { closeCount++; } });
        const preventedClose = await closedSource.pipeTo(openDestination, { preventClose: true })
          .then(() => true, () => false);

        let errorController, canceled = false, cancelReason;
        const waitingSource = new ReadableStream({ cancel(reason) { canceled = true; cancelReason = reason; } }, { highWaterMark: 0 });
        const failedDestination = new WritableStream({ start(controller) { errorController = controller; } }, { highWaterMark: 0 });
        const piping = waitingSource.pipeTo(failedDestination);
        errorController.error(undefined);
        const destinationError = await piping.then(() => false, reason => reason === undefined);
        return [sourceError, abortCount, preventedClose, closeCount, openDestination.locked,
          destinationError, canceled, cancelReason === undefined, waitingSource.locked, failedDestination.locked];
      }
    "#,
        ),
        Step::Call(
            "pipeToPreventFlagsAndFalsyReason",
            r#"[]"#,
            r#"[true,0,true,0,false,true,true,true,false,false]"#,
        ),
    ]);
}

#[test]
fn twin_event_target_skips_listeners_removed_during_dispatch() {
    twin(&[
        Step::Load(
            r#"function removedDuringDispatch() {
            const removed = new EventTarget(), removedCalls = [];
            const oldB = () => removedCalls.push('b');
            removed.addEventListener('work', () => { removedCalls.push('a'); removed.removeEventListener('work', oldB); });
            removed.addEventListener('work', oldB);
            removed.dispatchEvent(new Event('work'));

            const readded = new EventTarget(), readdedCalls = [];
            const nextB = () => readdedCalls.push('b');
            const late = () => readdedCalls.push('late');
            let first = true;
            readded.addEventListener('work', () => {
                readdedCalls.push('a');
                if (first) {
                    first = false;
                    readded.removeEventListener('work', nextB);
                    readded.addEventListener('work', nextB);
                    readded.addEventListener('work', late);
                }
            });
            readded.addEventListener('work', nextB);
            readded.dispatchEvent(new Event('work'));
            const firstDispatch = readdedCalls.join(',');
            readded.dispatchEvent(new Event('work'));

            const nested = new EventTarget();
            let onceCount = 0;
            nested.addEventListener('work', () => {
                onceCount++;
                nested.dispatchEvent(new Event('work'));
            }, { once: true });
            nested.dispatchEvent(new Event('work'));

            const changed = new EventTarget(), changedCalls = [];
            changed.addEventListener('work', event => { changedCalls.push('first'); event.type = 'other'; });
            changed.addEventListener('work', () => changedCalls.push('second'), { once: true });
            changed.dispatchEvent(new Event('work'));
            changed.dispatchEvent(new Event('work'));
            return [removedCalls.join(','), firstDispatch, readdedCalls.join(','), onceCount, changedCalls.join(',')];
        }"#,
        ),
        Step::Call(
            "removedDuringDispatch",
            r#"[]"#,
            r#"["a","a","a,a,b,late",1,"first,second,first"]"#,
        ),
    ]);
}

#[test]
fn twin_atomics_notify_nonshared_waitable_arrays_delegate_to_native() {
    twin(&[
        Step::Load(
            r#"function notifyNonsharedWaitable() {
            const ordinary = new Int32Array(1);
            let countCalls = 0;
            const count = { valueOf() { countCalls++; return 1; } };
            const direct = Atomics.notify(ordinary, 0);
            const counted = Atomics.notify(ordinary, 0, count);
            let invalidIndex = false;
            try { Atomics.notify(ordinary, 1); }
            catch (error) { invalidIndex = error instanceof RangeError; }
            let nonsharedWait = false;
            try { Atomics.waitAsync(ordinary, 0, 0); }
            catch (error) { nonsharedWait = error instanceof TypeError; }
            const bigInt64 = typeof BigInt64Array !== 'function'
                || Atomics.notify(new BigInt64Array(1), 0) === 0;
            const shared = new Int32Array(new SharedArrayBuffer(4));
            return [direct, counted, countCalls, invalidIndex, nonsharedWait,
                bigInt64, Atomics.notify(shared, 0), Atomics.waitAsync(shared, 0, 1).value];
        }"#,
        ),
        Step::Call(
            "notifyNonsharedWaitable",
            r#"[]"#,
            r#"[0,0,1,true,true,true,0,"not-equal"]"#,
        ),
    ]);
}

#[cfg(feature = "intl")]
#[test]
fn twin_intl_datetime_resolved_hour_cycle_matches_locale_and_midnight() {
    twin(&[
        Step::Load(
            r#"
        function cycles() {
            return ['en-US', 'de', 'en-US-u-hc-h23', 'de-u-hc-h12', 'en-US-u-hc-h24']
                .map(locale => {
                    const d = new Intl.DateTimeFormat(locale, {hour: 'numeric', timeZone: 'UTC'});
                    const r = d.resolvedOptions();
                    return [r.hourCycle, r.hour12,
                        d.formatToParts(new Date(Date.UTC(2024, 0, 1, 0)))
                            .find(p => p.type === 'hour').value];
                });
        }
        function localizedMidnight() {
            const d = new Intl.DateTimeFormat('ar-u-hc-h24', {hour: 'numeric', timeZone: 'UTC'});
            return [d.resolvedOptions().hourCycle,
                d.formatToParts(new Date(Date.UTC(2024, 0, 1, 0))).find(p => p.type === 'hour').value];
        }
    "#,
        ),
        Step::Call(
            "cycles",
            r#"[]"#,
            r#"[["h12",true,"12"],["h23",false,"00"],["h23",false,"00"],["h12",true,"12"],["h24",false,"24"]]"#,
        ),
        Step::Call("localizedMidnight", r#"[]"#, r#"["h24","24"]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_export_aliases_share_function_identity() {
    twin(&[
        Step::Load(
            r#"
        function wasmExportIdentity() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,4,4,1,112,0,1,7,17,3,1,97,0,0,1,98,0,0,5,116,97,98,108,101,1,0,9,7,1,0,65,0,11,1,0,10,6,1,4,0,65,42,11,0,11,4,110,97,109,101,1,4,1,0,1,102]);
            const module = new WebAssembly.Module(bytes);
            const first = new WebAssembly.Instance(module);
            const second = new WebAssembly.Instance(module);
            const forwarding = new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,10,1,4,104,111,115,116,1,102,0,0,4,4,1,112,0,1,7,17,3,1,97,0,0,1,98,0,0,5,116,97,98,108,101,1,0,9,7,1,0,65,0,11,1,0,0,11,4,110,97,109,101,1,4,1,0,1,102]));
            const forwarded = new WebAssembly.Instance(forwarding, { host: { f: first.exports.a } });
            return [forwarded.exports.a === first.exports.a,
                forwarded.exports.a === forwarded.exports.b,
                forwarded.exports.table.get(0) === first.exports.a,
                forwarded.exports.a(), first.exports.a === first.exports.b,
                first.exports.table.get(0) === first.exports.a,
                second.exports.a === second.exports.b,
                first.exports.a !== second.exports.a,
                first.exports.a(), first.exports.b()];
        }
    "#,
        ),
        Step::Call(
            "wasmExportIdentity",
            r#"[]"#,
            r#"[true,true,true,42,true,true,true,true,42,42]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_codec_preserves_special_numbers_and_reference_values() {
    twin(&[
        Step::Load(
            r#"
        function wasmCodecBoundaries() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,10,2,96,1,124,1,124,96,0,1,111,2,31,2,4,104,111,115,116,6,103,108,111,98,97,108,3,111,1,4,104,111,115,116,5,116,97,98,108,101,1,111,0,2,3,3,2,0,1,6,13,1,124,1,68,0,0,0,0,0,0,0,0,11,7,53,5,6,103,108,111,98,97,108,3,0,5,116,97,98,108,101,1,0,11,102,108,111,97,116,71,108,111,98,97,108,3,1,5,102,108,111,97,116,0,0,10,114,101,97,100,71,108,111,98,97,108,0,1,10,11,2,4,0,32,0,11,4,0,35,0,11,0,17,4,110,97,109,101,5,4,1,0,1,116,7,4,1,0,1,103]);
            const global = new WebAssembly.Global({ value: 'externref', mutable: true }, 7n);
            const table = new WebAssembly.Table({ element: 'externref', initial: 2, maximum: 3 }, 1);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { global, table } }).exports;
            const initial = [exports.readGlobal() === 7n, exports.table.get(0) === 1];
            exports.global.value = 0; exports.table.set(0, 9n); exports.table.grow(1, 17);
            const refs = [exports.global.value === 0, exports.table.get(0) === 9n, exports.table.get(2) === 17];
            const floats = [NaN, Infinity, -Infinity, -0].map(value => {
                exports.floatGlobal.value = value;
                return Object.is(exports.float(value), value) && Object.is(exports.floatGlobal.value, value);
            });
            return [initial, refs, floats];
        }
    "#,
        ),
        Step::Call(
            "wasmCodecBoundaries",
            r#"[]"#,
            r#"[[true,true],[true,true,true],[true,true,true,true]]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_start_refreshes_imported_memory() {
    twin(&[
        Step::Load(
            r#"
        function wasmStartMemory() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const oldBuffer = memory.buffer;
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,8,2,96,0,0,96,0,1,127,2,17,1,4,104,111,115,116,6,109,101,109,111,114,121,2,1,1,2,3,3,2,0,1,7,8,1,4,114,101,97,100,0,1,8,1,0,10,41,2,23,0,65,0,65,42,58,0,0,65,1,64,0,26,65,128,128,4,65,8,58,0,0,11,15,0,65,0,45,0,0,65,128,128,4,45,0,0,106,11,11,7,1,0,65,1,11,1,65,0,15,4,110,97,109,101,1,8,1,0,5,115,116,97,114,116]);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { memory } });
            const view = new Uint8Array(memory.buffer);
            const before = [view[0], view[1], view[65536], oldBuffer.byteLength, memory.buffer.byteLength];
            return [before, instance.exports.read(), new Uint8Array(memory.buffer)[0]];
        }
    "#,
        ),
        Step::Call("wasmStartMemory", r#"[]"#, r#"[[42,65,8,0,131072],50,42]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_failed_start_refreshes_imported_memory() {
    twin(&[
        Step::Load(
            r#"
        function wasmFailedStartMemory() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const oldBuffer = memory.buffer;
            const failed = new Uint8Array([0,97,115,109,1,0,0,0,1,4,1,96,0,0,2,17,1,4,104,111,115,116,6,109,101,109,111,114,121,2,1,1,2,3,2,1,0,8,1,0,10,26,1,24,0,65,0,65,42,58,0,0,65,1,64,0,26,65,128,128,4,65,8,58,0,0,0,11,0,15,4,110,97,109,101,1,8,1,0,5,115,116,97,114,116]);
            let trapped = false;
            try { new WebAssembly.Instance(new WebAssembly.Module(failed), { host: { memory } }); }
            catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
            const immediate = [new Uint8Array(memory.buffer)[0], new Uint8Array(memory.buffer)[65536]];
            const reader = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,17,1,4,104,111,115,116,6,109,101,109,111,114,121,2,1,1,2,3,2,1,0,7,8,1,4,114,101,97,100,0,0,10,17,1,15,0,65,0,45,0,0,65,128,128,4,45,0,0,106,11]);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(reader), { host: { memory } });
            const marker = { reason: "callback" };
            const callbackStart = new Uint8Array([0,97,115,109,1,0,0,0,1,4,1,96,0,0,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,2,1,0,8,1,1,10,6,1,4,0,16,0,11,0,25,4,110,97,109,101,1,18,2,0,8,99,97,108,108,98,97,99,107,1,5,115,116,97,114,116]);
            let exact = false;
            try { new WebAssembly.Instance(new WebAssembly.Module(callbackStart), { host: { callback() { throw marker; } } }); }
            catch (error) { exact = error === marker; }
            return [trapped, oldBuffer.byteLength, memory.buffer.byteLength, immediate,
                instance.exports.read(), exact];
        }
    "#,
        ),
        Step::Call(
            "wasmFailedStartMemory",
            r#"[]"#,
            r#"[true,0,131072,[42,8],50,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_memory_export_aliases_reuse_original_buffer() {
    twin(&[
        Step::Load(
            r#"
        function wasmMemoryIdentity() {
            const memory = new WebAssembly.Memory({ initial: 1 });
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,16,1,4,104,111,115,116,6,109,101,109,111,114,121,2,0,1,3,2,1,0,7,16,3,1,97,2,0,1,98,2,0,4,114,101,97,100,0,0,10,9,1,7,0,65,0,45,0,0,11]);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { memory } }).exports;
            new Uint8Array(memory.buffer)[0] = 73;
            const read = exports.read();
            const own = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0,5,3,1,0,0,7,9,2,1,97,2,0,1,98,2,0]))).exports;
            return [exports.a === memory, exports.b === memory, exports.a.buffer === memory.buffer, read, new Uint8Array(memory.buffer)[0], own.a === own.b, own.a !== memory];
        }
    "#,
        ),
        Step::Call(
            "wasmMemoryIdentity",
            r#"[]"#,
            r#"[true,true,true,73,73,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_duplicate_memory_imports_share_one_resource() {
    twin(&[
        Step::Load(
            r#"
        function wasmMemoryImportAliases() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const oldBuffer = memory.buffer;
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,8,2,96,0,0,96,0,1,127,2,32,2,4,104,111,115,116,5,102,105,114,115,116,2,1,1,2,4,104,111,115,116,6,115,101,99,111,110,100,2,1,1,2,3,3,2,0,1,7,16,3,1,97,2,0,1,98,2,1,4,114,101,97,100,0,1,8,1,0,10,25,2,14,0,65,0,65,42,58,0,0,65,1,64,0,26,11,8,0,65,0,45,64,1,0,11,0,33,4,110,97,109,101,1,8,1,0,5,115,116,97,114,116,6,16,2,0,5,102,105,114,115,116,1,6,115,101,99,111,110,100]);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { first: memory, second: memory } }).exports;
            const before = exports.read();
            new Uint8Array(memory.buffer)[0] = 73;
            return [exports.a === memory, exports.b === memory, oldBuffer.byteLength, memory.buffer.byteLength, before, exports.read()];
        }
    "#,
        ),
        Step::Call(
            "wasmMemoryImportAliases",
            r#"[]"#,
            r#"[true,true,0,131072,42,73]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_import_callback_reenters_same_instance() {
    twin(&[
        Step::Load(
            r#"
        function wasmReentry() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,3,2,0,0,5,3,1,0,1,7,26,3,6,109,101,109,111,114,121,2,0,5,105,110,110,101,114,0,1,5,111,117,116,101,114,0,2,10,17,2,7,0,65,0,45,0,0,11,7,0,16,0,65,1,106,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let instance;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { callback() {
                new Uint8Array(instance.exports.memory.buffer)[0] = 41;
                return instance.exports.inner();
            } } });
            return [instance.exports.outer(), instance.exports.inner()];
        }
    "#,
        ),
        Step::Call("wasmReentry", r#"[]"#, r#"[42,41]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_import_result_coercion_reenters_same_instance() {
    twin(&[
        Step::Load(
            r#"
        function wasmResultReentry() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,3,2,0,0,7,17,2,5,105,110,110,101,114,0,1,5,111,117,116,101,114,0,2,10,14,2,4,0,65,41,11,7,0,16,0,65,1,106,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let instance, conversions = 0;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { callback() {
                return { valueOf() { conversions++; return instance.exports.inner(); } };
            } } });
            return [instance.exports.outer(), conversions];
        }
    "#,
        ),
        Step::Call("wasmResultReentry", r#"[]"#, r#"[42,1]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_start_import_calls_existing_instance() {
    twin(&[
        Step::Load(
            r#"
        function wasmStartReentry() {
            const firstBytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,8,1,4,114,101,97,100,0,0,10,6,1,4,0,65,41,11]);
            const first = new WebAssembly.Instance(new WebAssembly.Module(firstBytes));
            const nextBytes = new Uint8Array([0,97,115,109,1,0,0,0,1,8,2,96,0,1,127,96,0,0,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,3,2,1,0,6,6,1,127,1,65,0,11,7,8,1,4,114,101,97,100,0,2,8,1,1,10,13,2,6,0,16,0,36,0,11,4,0,35,0,11,0,35,4,110,97,109,101,1,18,2,0,8,99,97,108,108,98,97,99,107,1,5,115,116,97,114,116,7,8,1,0,5,118,97,108,117,101]);
            const next = new WebAssembly.Instance(new WebAssembly.Module(nextBytes), {
                host: { callback() { return first.exports.read(); } }
            });
            return [next.exports.read(), first.exports.read()];
        }
    "#,
        ),
        Step::Call("wasmStartReentry", r#"[]"#, r#"[41,41]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_nested_import_reentry() {
    twin(&[
        Step::Load(
            r#"
        function wasmNestedReentry() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,6,1,96,1,127,1,127,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,2,1,0,7,11,1,7,114,101,99,117,114,115,101,0,1,10,11,1,9,0,32,0,16,0,65,1,106,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let instance;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: {
                callback(depth) { return depth === 0 ? 40 : instance.exports.recurse(depth - 1); }
            } });
            return instance.exports.recurse(2);
        }
    "#,
        ),
        Step::Call("wasmNestedReentry", r#"[]"#, r#"43"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_shared_memory_cross_instance_callback() {
    twin(&[
        Step::Load(
            r#"
        function wasmSharedCallback() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const readerBytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,16,1,4,104,111,115,116,6,109,101,109,111,114,121,2,0,1,3,3,2,0,0,7,15,2,4,114,101,97,100,0,0,4,115,105,122,101,0,1,10,14,2,7,0,65,0,45,0,0,11,4,0,63,0,11]);
            const reader = new WebAssembly.Instance(new WebAssembly.Module(readerBytes), { host: { memory } });
            const writerBytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,32,2,4,104,111,115,116,6,109,101,109,111,114,121,2,0,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,2,1,0,7,16,1,12,119,114,105,116,101,65,110,100,82,101,97,100,0,1,10,13,1,11,0,65,0,65,42,58,0,0,16,0,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let observedBuffer;
            const writer = new WebAssembly.Instance(new WebAssembly.Module(writerBytes), {
                host: { memory, callback() { observedBuffer = new Uint8Array(memory.buffer)[0]; const value = reader.exports.read(); new Uint8Array(memory.buffer)[1] = 73; return value; } }
            });
            const fromCallback = writer.exports.writeAndRead();
            const afterCallback = [fromCallback, reader.exports.read(), observedBuffer, new Uint8Array(memory.buffer)[0], new Uint8Array(memory.buffer)[1]];
            const previousPages = memory.grow(1);
            return afterCallback.concat([previousPages, memory.buffer.byteLength, reader.exports.size()]);
        }
    "#,
        ),
        Step::Call(
            "wasmSharedCallback",
            r#"[]"#,
            r#"[42,42,42,42,73,1,131072,2]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_memory_zero_growth_detaches_buffer() {
    twin(&[
        Step::Load(
            r#"
        function wasmZeroGrowth() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
            const oldBuffer = memory.buffer;
            new Uint8Array(oldBuffer)[0] = 91;
            const previous = memory.grow(0);
            const current = memory.buffer;
            let failed = false;
            try { memory.grow(1); } catch (error) { failed = error instanceof RangeError; }
            return [previous, oldBuffer.byteLength, current !== oldBuffer,
                current.byteLength, new Uint8Array(current)[0], failed, memory.buffer === current];
        }
    "#,
        ),
        Step::Call(
            "wasmZeroGrowth",
            r#"[]"#,
            r#"[1,0,true,65536,91,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_internal_memory_callback_coherence() {
    twin(&[
        Step::Load(
            r#"
        function wasmInternalCallback() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,8,2,96,0,0,96,0,1,127,2,17,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,2,1,1,5,3,1,0,1,7,25,2,6,109,101,109,111,114,121,2,0,12,119,114,105,116,101,65,110,100,67,97,108,108,0,1,10,18,1,16,0,65,0,65,42,58,0,0,16,0,65,1,45,0,0,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let memory, observed;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                host: { callback() { observed = new Uint8Array(memory.buffer)[0]; new Uint8Array(memory.buffer)[1] = 73; } }
            });
            memory = instance.exports.memory;
            return [instance.exports.writeAndCall(), observed, new Uint8Array(memory.buffer)[1]];
        }
    "#,
        ),
        Step::Call("wasmInternalCallback", r#"[]"#, r#"[73,42,73]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_table_funcref_memory_coherence() {
    twin(&[
        Step::Load(
            r#"
        function wasmTableMemory() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,1,127,0,3,2,1,0,4,4,1,112,0,1,5,3,1,0,1,7,18,2,6,109,101,109,111,114,121,2,0,5,116,97,98,108,101,1,0,9,7,1,0,65,0,11,1,0,10,11,1,9,0,65,0,32,0,58,0,0,11,0,15,4,110,97,109,101,1,8,1,0,5,119,114,105,116,101]);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
            const memory = instance.exports.memory;
            new Uint8Array(memory.buffer)[1] = 91;
            const write = instance.exports.table.get(0);
            write(73);
            return [new Uint8Array(memory.buffer)[0], new Uint8Array(memory.buffer)[1]];
        }
    "#,
        ),
        Step::Call("wasmTableMemory", r#"[]"#, r#"[73,91]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_import_getters_evaluated_once() {
    twin(&[
        Step::Load(
            r#"
        function wasmImportGetters() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,29,2,4,104,111,115,116,5,118,97,108,117,101,0,0,4,104,111,115,116,6,109,101,109,111,114,121,2,0,1,3,2,1,0,7,8,1,4,114,101,97,100,0,1,10,6,1,4,0,16,0,11,0,15,4,110,97,109,101,1,8,1,0,5,118,97,108,117,101]);
            let namespaces = 0, functions = 0, memories = 0;
            const memory = new WebAssembly.Memory({ initial: 1 });
            const namespace = {
                get value() { functions++; return functions === 1 ? () => 42 : () => 91; },
                get memory() { memories++; return memories === 1 ? memory : null; }
            };
            const imports = { get host() { namespaces++; return namespace; } };
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), imports);
            return [instance.exports.read(), functions, memories, namespaces];
        }
    "#,
        ),
        Step::Call("wasmImportGetters", r#"[]"#, r#"[42,1,1,2]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_import_result_coercion_memory() {
    twin(&[
        Step::Load(
            r#"
        function wasmResultCoercionMemory() {
            const memory = new WebAssembly.Memory({ initial: 1 });
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,32,2,4,104,111,115,116,6,109,101,109,111,114,121,2,0,1,4,104,111,115,116,8,99,97,108,108,98,97,99,107,0,0,3,2,1,0,7,7,1,3,114,117,110,0,1,10,19,1,17,0,65,0,65,42,58,0,0,16,0,65,1,45,0,0,106,11,0,18,4,110,97,109,101,1,11,1,0,8,99,97,108,108,98,97,99,107]);
            let observed;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: {
                memory, callback() { return { valueOf() { observed = new Uint8Array(memory.buffer)[0]; new Uint8Array(memory.buffer)[1] = 73; return 91; } }; }
            } });
            return [instance.exports.run(), observed, new Uint8Array(memory.buffer)[1]];
        }
    "#,
        ),
        Step::Call("wasmResultCoercionMemory", r#"[]"#, r#"[164,42,73]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_unsigned_limits_and_table_growth() {
    twin(&[
        Step::Load(
            r#"
        function wasmUnsignedLimits() {
            const rejects = action => { try { action(); return false; } catch (error) { return error instanceof TypeError; } };
            const invalid = [-1, NaN, Infinity, -Infinity, 4294967296, 1n];
            const memoryLimits = invalid.every(maximum => rejects(() => new WebAssembly.Memory({ initial: 0, maximum })));
            const tableLimits = invalid.every(maximum => rejects(() => new WebAssembly.Table({ element: "externref", initial: 0, maximum })));
            let initialReads = 0, maximumReads = 0, coercions = 0;
            const memory = new WebAssembly.Memory({
                get initial() { initialReads++; return 0.9; },
                get maximum() { maximumReads++; return { valueOf() { coercions++; return 1.9; } }; }
            });
            const before = memory.buffer;
            const rejectedGrowth = invalid.every(delta => rejects(() => memory.grow(delta)));
            const keptBuffer = memory.buffer === before;
            const previous = memory.grow(0.9);
            const table = new WebAssembly.Table({ element: "externref", initial: 1.9, maximum: 150001.9 });
            const tablePrevious = table.grow(150000.9);
            return [memoryLimits, tableLimits, rejectedGrowth, keptBuffer,
                initialReads, maximumReads, coercions, previous, before.byteLength,
                memory.buffer !== before, tablePrevious, table.length, table.get(150000) === undefined];
        }
    "#,
        ),
        Step::Call(
            "wasmUnsignedLimits",
            r#"[]"#,
            r#"[true,true,true,true,1,1,1,0,0,true,1,150001,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_length_conversion_captures_intrinsics() {
    twin(&[
        Step::Load(
            r#"
        function wasmCapturedLimits() {
            const trunc = Math.trunc, finite = Number.isFinite;
            try {
                const memory = new WebAssembly.Memory({ initial: {
                    valueOf() { Math.trunc = () => 1; Number.isFinite = () => false; return 0.9; }
                } });
                return memory.buffer.byteLength;
            } finally { Math.trunc = trunc; Number.isFinite = finite; }
        }
    "#,
        ),
        Step::Call("wasmCapturedLimits", r#"[]"#, r#"0"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_table_growth_captures_push() {
    twin(&[
        Step::Load(
            r#"
        function wasmCapturedPush() {
            const table = new WebAssembly.Table({ element: "externref", initial: 0, maximum: 3 });
            const push = Array.prototype.push;
            try {
                Array.prototype.push = () => { throw new Error("unexpected push"); };
                const previous = table.grow(3, "entry");
                return [previous, table.length, table.get(2)];
            } finally { Array.prototype.push = push; }
        }
    "#,
        ),
        Step::Call("wasmCapturedPush", r#"[]"#, r#"[0,3,"entry"]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_table_index_conversion() {
    twin(&[
        Step::Load(
            r#"
        function wasmTableIndices() {
            const table = new WebAssembly.Table({ element: "externref", initial: 1 });
            table.set(0.9, "value");
            const rejects = action => { try { action(); return false; } catch (error) { return error instanceof TypeError; } };
            const invalid = [-1, NaN, Infinity, 4294967296, 0n];
            const gets = invalid.every(index => rejects(() => table.get(index)));
            const sets = invalid.every(index => rejects(() => table.set(index, "other")));
            let outside = false;
            try { table.get(1); } catch (error) { outside = error instanceof RangeError; }
            return [table.get(0.9), table.get(-0), gets, sets, outside];
        }
    "#,
        ),
        Step::Call(
            "wasmTableIndices",
            r#"[]"#,
            r#"["value","value",true,true,true]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_detached_instantiate_streaming() {
    twin(&[
        Step::Load(
            r#"
        async function wasmDetachedStreaming() {
            const { instantiateStreaming } = WebAssembly;
            const result = await instantiateStreaming(new Response(
                new Uint8Array([0,97,115,109,1,0,0,0]),
                { headers: { "Content-Type": "application/wasm" } }));
            return result.module instanceof WebAssembly.Module && result.instance instanceof WebAssembly.Instance;
        }
    "#,
        ),
        Step::Call("wasmDetachedStreaming", r#"[]"#, r#"true"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_validate_input_errors() {
    twin(&[
        Step::Load(
            r#"
        function wasmValidateInputs() {
            const invalidInputs = [null, undefined, "text", {}, []].every(value => {
                try { WebAssembly.validate(value); return false; }
                catch (error) { return error instanceof TypeError; }
            });
            const header = new Uint8Array([0,97,115,109,1,0,0,0]);
            return [invalidInputs, WebAssembly.validate(header), WebAssembly.validate(new Uint8Array([0]))];
        }
    "#,
        ),
        Step::Call("wasmValidateInputs", r#"[]"#, r#"[true,true,false]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_immutable_global_import_sync() {
    twin(&[
        Step::Load(
            r#"
        function wasmImmutableGlobals() {
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,2,15,1,4,104,111,115,116,5,118,97,108,117,101,3,127,0,3,2,1,0,7,8,1,4,114,101,97,100,0,0,10,6,1,4,0,35,0,11,0,15,4,110,97,109,101,7,8,1,0,5,118,97,108,117,101]);
            const module = new WebAssembly.Module(bytes);
            const value = new WebAssembly.Global({ value: "i32", mutable: false }, 42);
            const first = new WebAssembly.Instance(module, { host: { value } });
            const second = new WebAssembly.Instance(module, { host: { value } });
            return [first.exports.read(), second.exports.read(), value.value];
        }
    "#,
        ),
        Step::Call("wasmImmutableGlobals", r#"[]"#, r#"[42,42,42]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_externref_global_default() {
    twin(&[
        Step::Load(
            r#"
        function wasmGlobalDefault() {
            const omitted = new WebAssembly.Global({ value: "externref" });
            const explicitNull = new WebAssembly.Global({ value: "externref" }, null);
            const explicitUndefined = new WebAssembly.Global({ value: "externref" }, undefined);
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,111,2,15,1,4,104,111,115,116,5,118,97,108,117,101,3,111,0,3,2,1,0,7,8,1,4,114,101,97,100,0,0,10,6,1,4,0,35,0,11,0,15,4,110,97,109,101,7,8,1,0,5,118,97,108,117,101]);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { value: omitted } });
            return [omitted.value === undefined, explicitNull.value === null,
                explicitUndefined.value === undefined, instance.exports.read() === undefined];
        }
    "#,
        ),
        Step::Call("wasmGlobalDefault", r#"[]"#, r#"[true,true,true,true]"#),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_descriptor_member_reads() {
    twin(&[
        Step::Load(
            r#"
        function wasmDescriptorReads() {
            let valueReads = 0, elementReads = 0, coercions = 0;
            const order = [];
            const global = new WebAssembly.Global({
                get mutable() { order.push("mutable"); return true; },
                get value() { valueReads++; order.push("value"); return {
                    toString() { coercions++; return "i32"; }
                }; }
            }, 3);
            const table = new WebAssembly.Table({
                get element() { elementReads++; return elementReads === 1 ? "externref" : "anyfunc"; },
                initial: 1
            }, "entry");
            return [valueReads, elementReads, coercions, order.join(","), global.value, table.get(0)];
        }
    "#,
        ),
        Step::Call(
            "wasmDescriptorReads",
            r#"[]"#,
            r#"[1,1,1,"mutable,value",3,"entry"]"#,
        ),
    ]);
}

#[cfg(feature = "wasm")]
#[test]
fn twin_webassembly_failed_start_preserves_imported_table_function() {
    twin(&[
        Step::Load(
            r#"
        function wasmFailedStartTableFunction() {
            const table = new WebAssembly.Table({ element: "anyfunc", initial: 1 });
            const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,8,2,96,0,1,127,96,0,0,2,16,1,4,104,111,115,116,5,116,97,98,108,101,1,112,0,1,3,3,2,0,1,8,1,1,9,7,1,0,65,0,11,1,0,10,10,2,4,0,65,31,11,3,0,0,11,0,32,4,110,97,109,101,1,15,2,0,5,118,97,108,117,101,1,5,115,116,97,114,116,5,8,1,0,5,116,97,98,108,101]);
            let trapped = false;
            try { new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { table } }); }
            catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
            const value = table.get(0);
            return [trapped, typeof value, value()];
        }
    "#,
        ),
        Step::Call(
            "wasmFailedStartTableFunction",
            r#"[]"#,
            r#"[true,"function",31]"#,
        ),
    ]);
}

#[test]
fn twin_proxy_get_own_property_descriptor_trap_keeps_accessor_descriptors() {
    twin(&[
        Step::Load(
            r#"
        globalThis.proxyAccessorDescriptors = function () {
          const getter = function () { return 7; };
          const setter = function (v) {};
          const source = Object.defineProperties({}, {
            both: { get: getter, set: setter, configurable: true, enumerable: true },
            getOnly: { get: getter, configurable: true, enumerable: false },
            setOnly: { set: setter, configurable: true, enumerable: true },
          });
          const shape = (target) => {
            const p = new Proxy(target, {
              getOwnPropertyDescriptor(_, key) {
                return Reflect.getOwnPropertyDescriptor(source, key);
              },
            });
            return ['both', 'getOnly', 'setOnly'].map((key) => {
              const d = Reflect.getOwnPropertyDescriptor(p, key);
              return [
                'value' in d || 'writable' in d,
                d.get === Reflect.getOwnPropertyDescriptor(source, key).get,
                d.set === Reflect.getOwnPropertyDescriptor(source, key).set,
                d.enumerable, d.configurable,
              ].join(':');
            }).join('|');
          };
          // Both an empty object target and an array target (indices) must work.
          const overArray = new Proxy([1, 2, 3], {
            getOwnPropertyDescriptor() { return Reflect.getOwnPropertyDescriptor(source, 'both'); },
          });
          const element = Object.getOwnPropertyDescriptor(overArray, '1');
          return [shape({}), shape([]), typeof element.get, 'value' in element].join('/');
        };
    "#,
        ),
        Step::Call(
            "proxyAccessorDescriptors",
            r#"[]"#,
            r#""false:true:true:true:true|false:true:true:false:true|false:true:true:true:true/false:true:true:true:true|false:true:true:false:true|false:true:true:true:true/function/false""#,
        ),
    ]);
}

#[test]
fn twin_date_wire_legacy_results_preserve_nested_undefined() {
    twin(&[Step::Load(
        r#"globalThis.wireHost = {
                 nestedObject() { return { x: undefined }; },
                 nestedArray() { return [undefined]; },
                 nestedMixed() { return { a: [undefined, { b: undefined, c: null }], d: 1 }; },
                 nestedAbsent() { return {}; },
                 nestedNull() { return { x: null }; },
                 nestedSparse() { return [, 1]; },
                 nestedOmitted() { return { f() {}, s: Symbol('s'), u: undefined }; },
                 nestedFunctionInArray() { return [function () {}, undefined]; },
               };"#,
    )]);
}

#[test]
fn twin_date_wire_legacy_custom_to_json_runs_once_with_key_and_receiver() {
    twin(&[
        Step::Load(
            r#"globalThis.toJsonLog = [];
               function customToJson() {
                 const inner = { toJSON(key) { globalThis.toJsonLog.push([key, this === inner]); return { kept: key }; } };
                 return { inner, list: [inner] };
               }
               function toJsonCalls() { return globalThis.toJsonLog; }"#,
        ),
        Step::Call(
            "customToJson",
            r#"[]"#,
            r#"{"inner":{"kept":"inner"},"list":[{"kept":"0"}]}"#,
        ),
        Step::Call("toJsonCalls", r#"[]"#, r#"[["inner",true],["0",true]]"#),
    ]);
}

#[test]
fn twin_text_decoder_supports_utf16le_and_latin1_labels() {
    twin(&[
        Step::Load(
            r#"function decoders() { const r = []; const d = new TextDecoder('utf-16le');
r.push(d.decode(new Uint8Array([104,0,105,0])));
r.push(d.decode(new Uint8Array([0x3d,0xd8,0x00,0xde,0x41,0])));
r.push(d.decode(new Uint8Array([0xff,0xfe,0x68,0])));
r.push(d.decode(new Uint8Array([0x00,0xd8,0x41,0,0x42])));
r.push(new TextDecoder('utf-16le',{ignoreBOM:true}).decode(new Uint8Array([0xff,0xfe,0x68,0])).length);
const s = new TextDecoder('utf-16le');
r.push(s.decode(new Uint8Array([0x3d,0xd8,0x00]),{stream:true}) + '|' + s.decode(new Uint8Array([0xde]),{stream:true}) + '|' + s.decode());
r.push(new TextDecoder('latin1').decode(new Uint8Array([233,65,0x80,0x9f,0xff])));
r.push(new TextDecoder('latin1').encoding, new TextDecoder('ascii').encoding, new TextDecoder('utf-16').encoding, new TextDecoder('UTF-16LE').encoding);
return r; }"#,
        ),
        Step::Call(
            "decoders",
            r#"[]"#,
            r#"["hi","😀A","h","�A�",2,"|😀|","éA€Ÿÿ","windows-1252","windows-1252","utf-16le","utf-16le"]"#,
        ),
    ]);
}

#[test]
fn twin_process_exposes_ids_umask_kill_release_and_rss() {
    twin(&[
        Step::Load(
            r#"function processIds() { const mask = process.umask(); const restored = process.umask(mask);
let missing = 'no error'; try { process.kill(2147483646, 0); } catch (e) { missing = [e.code, e.errno, e.syscall, e.message].join(','); }
let unknown = 'no error'; try { process.kill(process.pid, 'SIGNOPE'); } catch (e) { unknown = [e.name, e.code, e.message].join(','); }
return [typeof process.getuid(), typeof process.geteuid(), typeof process.getgid(), typeof process.getegid(), mask === restored, process.umask() === mask,
process.kill(process.pid, 0), missing, unknown, process.release.name, typeof process.pid, process.ppid > 0, process.memoryUsage.rss() > 0, typeof process.features]; }"#,
        ),
        Step::Call(
            "processIds",
            r#"[]"#,
            r#"["number","number","number","number",true,true,true,"ESRCH,-3,kill,kill ESRCH","TypeError,ERR_UNKNOWN_SIGNAL,Unknown signal: SIGNOPE","node","number",true,true,"object"]"#,
        ),
    ]);
}

#[test]
fn twin_iterator_helpers_close_sources_and_complete_after_errors() {
    twin(&[
        Step::Load(
            r#"
      function __iterator_helper_close_controls() {
        const results = [];
        for (const method of ['map', 'filter', 'drop', 'take', 'flatMap']) {
          const source = new Map([['a', 1], ['b', 2]]).values();
          const helper = method === 'map' ? source.map(x => x)
            : method === 'filter' ? source.filter(() => true)
            : method === 'drop' ? source.drop(0)
            : method === 'take' ? source.take(2)
            : source.flatMap(x => [x, x + 1]);
          helper.next();
          try { results.push(helper.return().done && helper.next().done); }
          catch { results.push(false); }
        }
        const failed = [1, 2].values().map(() => { throw new Error('mapper'); });
        try { failed.next(); } catch {}
        try { results.push(failed.next().done); } catch { results.push(false); }
        const order = [];
        const outer = { next() { return { value: 1, done: false }; },
          return() { order.push('outer'); throw new Error('outer close'); } };
        const inner = { next() { return { value: 2, done: false }; },
          return() { order.push('inner'); throw new Error('inner close'); } };
        const flattened = Iterator.from(outer).flatMap(() => inner);
        flattened.next();
        try { flattened.return(); } catch (error) { results.push(error.message); }
        results.push(flattened.next().done, flattened.return().done);
        results.push(order.join(','));
        return results;
      }
    "#,
        ),
        Step::Call(
            "__iterator_helper_close_controls",
            r#"[]"#,
            r#"[true,true,true,true,true,true,"inner close",true,true,"inner,outer"]"#,
        ),
    ]);
}
