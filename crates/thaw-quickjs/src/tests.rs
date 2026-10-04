use super::*;

fn call(func_name: &str, args_json: &str) -> String {
    let func_name = CString::new(func_name).unwrap();
    let args_json = CString::new(args_json).unwrap();
    let result_ptr = thaw_js_call(func_name.as_ptr(), args_json.as_ptr());
    unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn host_query_callable_slot_does_not_collide_with_number_conversion() {
    assert_eq!(load(r#"
        globalThis.hostQueryNumber = { valueOf() { return 17; } };
        globalThis.hostQueryCallable = new Proxy(function () {}, {});
        globalThis.hostQueryOriginal = globalThis.__thaw_json_host_query;
        globalThis.__thaw_json_host_query = () => 'function';
    "#), 1);
    let number = thaw_js_get_global(c"hostQueryNumber".as_ptr());
    let callable = thaw_js_get_global(c"hostQueryCallable".as_ptr());
    let kind = thaw_js_host_query_result(number, 14);
    let callable_kind = thaw_js_host_query_result(callable, 14);
    assert_eq!(load(r#"
        globalThis.__thaw_json_host_query = globalThis.hostQueryOriginal;
        delete globalThis.hostQueryOriginal;
    "#), 1);
    let coerced = thaw_js_host_query_result(number, 16);
    assert!(kind.error.is_null() && coerced.error.is_null() && callable_kind.error.is_null());
    assert_eq!(unsafe { CStr::from_ptr(kind.value) }.to_bytes(), b"0");
    assert_eq!(unsafe { CStr::from_ptr(coerced.value) }.to_bytes(), b"17");
    assert_eq!(unsafe { CStr::from_ptr(callable_kind.value) }.to_bytes(), b"1");
    unsafe {
        thaw_arena::destroy_string(kind.value.cast_mut());
        thaw_arena::destroy_string(coerced.value.cast_mut());
        thaw_arena::destroy_string(callable_kind.value.cast_mut());
    }
    assert_eq!(thaw_js_release_handle(number), 1);
    assert_eq!(thaw_js_release_handle(callable), 1);
    assert_eq!(load(r#"
        delete globalThis.hostQueryNumber;
        delete globalThis.hostQueryCallable;
    "#), 1);
}

fn load(source: &str) -> u8 {
    let source = CString::new(source).unwrap();
    thaw_js_load(source.as_ptr())
}

#[test]
fn stdin_pause_keeps_the_rest_of_a_polled_batch_until_resume() {
    std::thread::spawn(|| {
        assert_eq!(load(r#"
      function stdinPauseBatch() {
        const originalStart = __thaw_process_start_stdin;
        const originalPoll = __thaw_process_poll_stdin;
        const seen = [];
        let starts = 0, polls = 0;
        const onData = chunk => {
          seen.push(chunk.toString());
          if (seen.length === 1) process.stdin.pause();
        };
        const onEnd = () => seen.push('end');
        try {
          __thaw_process_start_stdin = () => { starts++; };
          __thaw_process_poll_stdin = () => {
            polls++;
            return polls === 1 ? JSON.stringify([
              {type: 'data', value: '61'},
              {type: 'data', value: '62'},
              {type: 'end'}
            ]) : '[]';
          };
          process.stdin.on('data', onData);
          process.stdin.on('end', onEnd);
          process.stdin.pause();
          __thaw_poll_platform_events();
          const beforeResume = seen.slice();
          process.stdin.resume();
          __thaw_drain_next_tick_queue();
          const afterFirstResume = seen.slice();
          process.stdin.resume();
          __thaw_drain_next_tick_queue();
          return [beforeResume, afterFirstResume, seen.slice(), starts, polls];
        } finally {
          process.stdin.pause();
          process.stdin.off('data', onData);
          process.stdin.off('end', onEnd);
          __thaw_process_start_stdin = originalStart;
          __thaw_process_poll_stdin = originalPoll;
        }
      }
    "#), 1);
        assert_eq!(call("stdinPauseBatch", "[]"),
            r#"[[],["a"],["a","b","end"],1,1]"#);
    }).join().unwrap();
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
fn terminal_pending_work_is_separate_from_failure_and_exit_code() {
    unsafe extern "C" fn fail_job(
        ctx: *mut rquickjs::qjs::JSContext, _argc: i32, _argv: *mut rquickjs::qjs::JSValue,
    ) -> rquickjs::qjs::JSValue {
        unsafe { rquickjs::qjs::JS_ThrowTypeError(ctx, c"first job".as_ptr()) }
    }
    unsafe extern "C" fn later_job(
        ctx: *mut rquickjs::qjs::JSContext, _argc: i32, _argv: *mut rquickjs::qjs::JSValue,
    ) -> rquickjs::qjs::JSValue {
        let source = c"globalThis.afterFailedJob = 1";
        unsafe { rquickjs::qjs::JS_Eval(
            ctx, source.as_ptr(), source.to_bytes().len(), c"<terminal-test>".as_ptr(),
            rquickjs::qjs::JS_EVAL_TYPE_GLOBAL as i32,
        ) }
    }
    assert_eq!(load("globalThis.afterFailedJob = 0"), 1);
    with_context(|ctx| unsafe {
        let raw = ctx.as_raw().as_ptr();
        assert!(rquickjs::qjs::JS_EnqueueJob(raw, Some(fail_job), 0, std::ptr::null_mut()) >= 0);
        assert!(rquickjs::qjs::JS_EnqueueJob(raw, Some(later_job), 0, std::ptr::null_mut()) >= 0);
    });
    assert_eq!(thaw_js_run_event_loop(), 1);
    assert_eq!(thaw_js_terminal_work_pending(), 1);
    assert_eq!(thaw_js_run_event_loop(), 0);
    assert_eq!(eval_json("afterFailedJob"), Ok(Some("1".into())));
    assert_eq!(thaw_js_terminal_work_pending(), 0);

    assert_eq!(load("globalThis.afterFailedTick = 0; process.nextTick(() => { throw new Error('first tick'); }); process.nextTick(() => { afterFailedTick = 1; });"), 1);
    assert_eq!(thaw_js_run_event_loop(), 1);
    assert_eq!(thaw_js_terminal_work_pending(), 1);
    assert_eq!(thaw_js_run_event_loop(), 0);
    assert_eq!(eval_json("afterFailedTick"), Ok(Some("1".into())));
    assert_eq!(thaw_js_terminal_work_pending(), 0);

    assert_eq!(load("process.exitCode = 7"), 1);
    assert_eq!(thaw_js_run_event_loop(), 7);
    assert_eq!(thaw_js_terminal_work_pending(), 0);
    assert_eq!(load("process.exitCode = 0"), 1);
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

/// `process.report.getReport().header.glibcVersionRuntime` -- real
/// Node's own way of telling a glibc build apart from a musl one at
/// runtime, which some native-addon loaders check directly instead of
/// trusting `process.platform`/`arch` alone (real trigger:
/// `better-sqlite3`'s own prebuild-selection fallback,
/// `!process.report.getReport().header.glibcVersionRuntime`, which used
/// to throw outright -- `process.report` didn't exist at all). This
/// test environment is a real glibc Linux build, so the field should be
/// a genuine non-empty version string (`libc::gnu_get_libc_version()`),
/// not just present-but-empty.
#[test]
fn process_report_exposes_the_real_glibc_version() {
    assert_eq!(
        load(
            "function glibcVersion() {\n\
               const header = process.report.getReport().header;\n\
               return [typeof header.glibcVersionRuntime, header.glibcVersionRuntime.length > 0];\n\
             }"
        ),
        1
    );
    assert_eq!(call("glibcVersion", "[]"), r#"["string",true]"#);
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
fn console_exposes_node_style_stdout_and_stderr_streams() {
    assert_eq!(
        load(
            "function consoleStreams() {\n\
               return [typeof console._stdout, typeof console._stdout.write, typeof console._stderr.write];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("consoleStreams", "[]"),
        r#"["object","function","function"]"#
    );
}

#[test]
fn error_properties_round_trip_through_the_tagged_exception_format() {
    assert_eq!(
        load(
            "function errorProps() {\n\
               const error = new Error('boom');\n\
               error.status = 418;\n\
               error.statusCode = 418;\n\
               error.expose = true;\n\
               const props = globalThis.__thaw_error_properties_json(error);\n\
               const restored = globalThis.__thaw_error_from_tagged('\\u0001ImATeapotError\\u0001boom\\u0005' + props);\n\
               return [restored.name, restored.message, restored.status, restored.expose];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("errorProps", "[]"),
        r#"["ImATeapotError","boom",418,true]"#
    );
}

#[test]
fn framed_error_rebuilds_original_message_and_keeps_display_separate() {
    assert_eq!(load(r#"
      function framedErrorRoundTrip() {
        const raw = '\u0001Error\u0001\u001eE1:7:5:show\u0002meraw\u0003x\u0005{"status":418}';
        const emptyName = '\u0001\u0001\u001eE1:3:-:a\u0002b';
        const frame = __thaw_error_frame_parts(raw);
        const restored = __thaw_error_from_tagged(raw);
        const emptyRestored = __thaw_error_from_tagged(emptyName);
        const revived = JSON.parse(JSON.stringify(raw), __thaw_json_date_reviver);
        return [frame.display, frame.original, restored.name, restored.message,
                restored.status, revived.message, emptyRestored.name, emptyRestored.message];
      }
    "#), 1);
    assert_eq!(call("framedErrorRoundTrip", "[]"),
        "[\"show\\u0002me\",\"raw\\u0003x\",\"Error\",\"raw\\u0003x\",418,\"raw\\u0003x\",\"\",\"a\\u0002b\"]");
}


#[test]
fn framed_error_actual_throw_and_rejection_keep_empty_name_getter_order_and_original() {
    assert_eq!(load(r#"
      function throwEmptyName() { const error = new Error('a\u0002b'); error.name = ''; throw error; }
      function rejectFramedError() { return Promise.reject(Object.assign(new Error('raw\u0003x'), { status: 418 })); }
      globalThis.errorGetterOrder = '';
      function throwGetterOrder() {
        throw Object.defineProperties({}, {
          message: { get() { errorGetterOrder += 'M'; return 'body\u0004x'; } },
          code: { get() { errorGetterOrder += 'C'; return 'CODE'; } },
          name: { get() { errorGetterOrder += 'N'; return 'TypeError'; } }
        });
      }
      function readErrorGetterOrder() { return errorGetterOrder; }
    "#), 1);
    let empty = thaw_js_call_result(c"throwEmptyName".as_ptr(), c"[]".as_ptr());
    assert!(empty.value.is_null());
    let wire = unsafe { CStr::from_ptr(empty.error) }.to_bytes();
    let frame = thaw_arena::error_wire::parse_tagged(wire).unwrap();
    assert_eq!(frame.chain, b"");
    assert_eq!(frame.display, b"`throwEmptyName` threw: a\x02b");
    assert_eq!(frame.suffix, b"");
    unsafe { thaw_arena::destroy_string(empty.error.cast_mut()) };

    let rejected = thaw_js_call_result(c"rejectFramedError".as_ptr(), c"[]".as_ptr());
    assert!(rejected.value.is_null());
    let wire = unsafe { CStr::from_ptr(rejected.error) }.to_bytes();
    let frame = thaw_arena::error_wire::parse_tagged(wire).unwrap();
    assert_eq!(frame.chain, b"Error");
    assert_eq!(frame.display, b"`rejectFramedError`'s promise rejected: raw\x03x");
    assert_eq!(frame.original, Some(&b"raw\x03x"[..]));
    assert_eq!(frame.suffix, b"\x05{\"status\":418}");
    unsafe { thaw_arena::destroy_string(rejected.error.cast_mut()) };

    let ordered = thaw_js_call_result(c"throwGetterOrder".as_ptr(), c"[]".as_ptr());
    assert!(ordered.value.is_null());
    let wire = unsafe { CStr::from_ptr(ordered.error) }.to_bytes();
    let frame = thaw_arena::error_wire::parse_tagged(wire).unwrap();
    assert_eq!(frame.chain, b"TypeError");
    assert_eq!(frame.display, b"`throwGetterOrder` threw: body\x04x");
    assert_eq!(frame.suffix, b"\x03CODE");
    unsafe { thaw_arena::destroy_string(ordered.error.cast_mut()) };
    assert_eq!(call("readErrorGetterOrder", "[]"), "\"MCN\"");
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
fn performance_measure_end_duration_and_option_dictionary_order() {
    assert_eq!(load(r#"
      function measureEndDuration() {
        performance.clearMarks(); performance.clearMeasures();
        performance.mark('from', { startTime: 10 });
        performance.mark('until', { startTime: 100 });
        const numeric = performance.measure('numeric', { end: 100, duration: 20 });
        const named = performance.measure('named', { end: 'until', duration: 20 });
        const fromDuration = performance.measure('fromDuration', { start: 5, duration: 20 });
        const fromEnd = performance.measure('fromEnd', { start: 5, end: 100 });
        const legacy = performance.measure('legacy', 'from', 'until');
        const empty = performance.measure('empty', {});
        const emptyThird = performance.measure('emptyThird', {}, 'until');
        let missingThird;
        try { performance.measure('missingThird', {}, 'unknown'); missingThird = false; }
        catch (error) { missingThird = error instanceof SyntaxError; }
        const events = [];
        const dictionary = {
          get detail() { events.push('detail'); return 'd'; },
          get duration() { events.push('duration'); return { valueOf() { events.push('coerce'); return 20; } }; },
          get end() { events.push('end'); return 100; },
          get start() { events.push('start'); return undefined; }
        };
        const accessed = performance.measure('accessed', dictionary);
        const errors = [];
        for (const options of [{ duration: 20 }, { detail: 'x' },
                               { start: 5, end: 100, duration: 20 }]) {
          try { performance.measure('bad', options); errors.push(false); }
          catch (error) { errors.push(error instanceof TypeError); }
        }
        try { performance.measure('badThird', { end: 100, duration: 20 }, 'until'); errors.push(false); }
        catch (error) { errors.push(error instanceof TypeError); }
        try { performance.measure('allThree', { start: 5, end: 'missing', duration: 20 }); errors.push(false); }
        catch (error) { errors.push(error instanceof TypeError); }
        return [[numeric.startTime, numeric.duration], [named.startTime, named.duration],
                [fromDuration.startTime, fromDuration.duration], [fromEnd.startTime, fromEnd.duration],
                [legacy.startTime, legacy.duration], empty.startTime === 0 && empty.duration >= 0,
                [emptyThird.startTime, emptyThird.duration], missingThird,
                [accessed.startTime, accessed.duration, accessed.detail], events, errors];
      }
    "#), 1);
    assert_eq!(call("measureEndDuration", "[]"),
        r#"[[80,20],[80,20],[5,20],[5,95],[10,90],true,[0,100],true,[80,20,"d"],["detail","duration","coerce","end","start"],[true,true,true,true,true]]"#);
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
fn message_port_retains_messages_until_started() {
    assert_eq!(
        load(r#"async function portStartQueue() {
            const channel = new MessageChannel(), seen = [];
            channel.port1.postMessage('first'); channel.port1.postMessage('second');
            await Promise.resolve();
            channel.port2.addEventListener('message', event => seen.push(event.data));
            await Promise.resolve();
            const beforeStart = seen.slice();
            channel.port2.start(); channel.port2.start();
            await Promise.resolve();
            const afterStart = seen.slice();

            const assigned = new MessageChannel(), assignedSeen = [];
            assigned.port1.postMessage('assigned');
            await Promise.resolve();
            assigned.port2.onmessage = event => assignedSeen.push(event.data);
            await Promise.resolve();

            const node = new MessageChannel(), nodeSeen = [];
            node.port1.postMessage('node');
            await Promise.resolve();
            node.port2.on('message', value => nodeSeen.push(value));
            await Promise.resolve();

            const source = new MessageChannel(), movedSeen = [];
            source.port2.start();
            source.port1.postMessage('moved');
            const moved = structuredClone(source.port2, { transfer: [source.port2] });
            await Promise.resolve();
            const transferPending = [moved.__thawStarted, moved.__thawQueue.length];
            moved.onmessage = event => movedSeen.push(event.data);
            await Promise.resolve();

            const closed = new MessageChannel(), closedSeen = [];
            closed.port1.postMessage('discarded');
            closed.port2.close();
            closed.port2.onmessage = event => closedSeen.push(event.data);
            await Promise.resolve();
            const discarded = [closedSeen.length, closed.port2.__thawQueue.length];

            const pulled = new MessageChannel();
            pulled.port1.postMessage('pull');
            await Promise.resolve();
            const queuedForPull = pulled.port2.__thawQueue.length;
            channel.port1.close(); channel.port2.close();
            assigned.port1.close(); assigned.port2.close();
            node.port1.close(); node.port2.close();
            source.port1.close(); moved.close();
            closed.port1.close(); pulled.port1.close(); pulled.port2.close();
            return [beforeStart, afterStart, assignedSeen, nodeSeen,
                    transferPending, movedSeen, discarded, queuedForPull];
        }"#),
        1
    );
    assert_eq!(call("portStartQueue", "[]"),
        r#"[[],["first","second"],["assigned"],["node"],[false,1],["moved"],[0,0],1]"#);
}

#[test]
fn url_search_params_skips_empty_fields_but_keeps_empty_names() {
    assert_eq!(load(r#"function emptyQueryFields() {
        const check = (value, expected) => { if (value !== expected) throw new Error('query field assertion'); };
        const params = new URLSearchParams('?&&a=1&&');
        check(params.size, 1); check(params.getAll('').length, 0); check(params.toString(), 'a=1');
        check(new URLSearchParams('&&').size, 0);
        const emptyNames = new URLSearchParams('&&=&=value&&');
        check(emptyNames.size, 2); check(emptyNames.getAll('').join('|'), '|value'); check(emptyNames.toString(), '=&=value');
        const url = new URL('https://example.test/?&&a=1&&');
        check(url.searchParams.size, 1); url.searchParams.append('b', '2'); check(url.search, '?a=1&b=2');
        return true;
    }"#), 1);
    assert_eq!(call("emptyQueryFields", "[]"), "true");
}

#[test]
fn url_search_params_form_encoding_and_live_iteration() {
    assert_eq!(load(r#"function checkLiveParams() {
        const check = (value, expected) => { if (value !== expected) throw new Error(value + ' != ' + expected); };
        check(new URLSearchParams({k: "!'()~* -._"}).toString(), 'k=%21%27%28%29%7E*+-._');
        const params = new URLSearchParams('a=1&b=2'), entries = params.entries();
        check(entries.next().value.join(':'), 'a:1');
        params.set('b', 'changed'); params.append('c', '3');
        check(entries.next().value.join(':'), 'b:changed');
        check(entries.next().value.join(':'), 'c:3');
        check(entries.next().done, true); params.append('d', '4'); check(entries.next().done, true);
        const keys = params.keys(); check(keys.next().value, 'a');
        params.delete('a'); check(keys.next().value, 'c');
        const values = params.values(); check(values.next().value, 'changed');
        params.set('c', 'updated'); check(values.next().value, 'updated');
        const sorted = new URLSearchParams('b=2&a=1'), ordered = sorted.entries();
        check(ordered.next().value[0], 'b'); sorted.sort(); check(ordered.next().value[0], 'b');
        const changed = new URLSearchParams('a=1&b=2&c=3'), seen = [], receiver = {};
        changed.forEach(function(value, key, owner) {
            check(this, receiver); check(owner, changed); seen.push(key + ':' + value);
            if (key === 'a') { changed.delete('a'); changed.append('d', '4'); }
        }, receiver);
        check(seen.join('|'), 'a:1|c:3|d:4'); return true;
    }"#), 1);
    assert_eq!(call("checkLiveParams", "[]"), "true");
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
fn url_preserves_params_reference_and_decodes_malformed_form_input() {
    assert_eq!(load(r#"function checkUrlParamsIdentity() {
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
    }"#), 1);
    assert_eq!(call("checkUrlParamsIdentity", "[]"), "true");
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
fn url_rejects_out_of_range_ports_without_mutating_setters() {
    assert_eq!(
        load(r#"function urlPortBounds() {
            const url = new URL('https://example.test:8080/path');
            const rejectedConstructor = URL.canParse('https://example.test:65536/') === false
              && URL.parse('https://example.test:70000/') === null;
            url.port = '65536';
            const rejectedPort = url.href === 'https://example.test:8080/path';
            url.port = '00070000';
            const rejectedPaddedPort = url.port === '8080';
            url.host = 'other.test:70000';
            const rejectedHost = url.href === 'https://example.test:8080/path';
            url.port = '65535';
            const acceptedPort = url.port === '65535';
            url.host = 'other.test:65535';
            const acceptedHost = url.href === 'https://other.test:65535/path';
            url.port = '';
            return [rejectedConstructor, rejectedPort, rejectedPaddedPort,
              rejectedHost, acceptedPort, acceptedHost, url.port === ''];
        }"#),
        1
    );
    assert_eq!(call("urlPortBounds", "[]"), "[true,true,true,true,true,true,true]");
}

#[test]
fn url_native_parser_preserves_url_components_and_setters() {
    assert_eq!(load(r#"function nativeUrlCases() {
        const opaque = new URL('urn:x/../y');
        opaque.pathname = 'changed';
        const encoded = new URL('https://example.test/');
        encoded.username = 'a@b'; encoded.password = 'p#q'; encoded.pathname = '/a?b#c';
        const host = new URL('https://bücher.example:8080/a');
        host.hostname = 'bad host';
        const oldHost = host.host;
        host.host = 'other.test:70000';
        const unchangedHost = host.host === oldHost;
        host.host = '[::1]:8080';
        const acceptedIPv6 = host.host === '[::1]:8080';
        host.host = 'other.test';
        const keptPort = host.host === 'other.test:8080';
        host.hostname = 'changed.test:9090';
        const rejectedHostnamePort = host.host === 'other.test:8080';
        host.host = 'other.test:443';
        const clearedDefaultPort = host.host === 'other.test';
        const params = host.searchParams;
        host.search = '?a=1';
        const sameParams = host.searchParams === params && params.get('a') === '1';
        params.append('b', '2');
        const linkedParams = host.search === '?a=1&b=2';
        host.protocol = 'foo:';
        const guardedProtocol = host.protocol === 'https:';
        host.href = 'https://other.test/?x=1';
        const hrefIdentity = host.searchParams === params && params.get('x') === '1';
        const empty = new URL('https://example.test/');
        empty.search = '?'; empty.hash = '#';
        const customBase = new URL('https://ignored.test/');
        let baseCalls = 0;
        customBase.toString = () => { baseCalls++; return 'https://base.test/root/'; };
        return [
          new URL('foo://example.com/x').origin === 'null',
          new URL('blob:https://example.com/id').origin === 'https://example.com',
          opaque.href === 'urn:x/../y' && opaque.pathname === 'x/../y',
          new URL('https://example.test/a//b').pathname === '/a//b',
          new URL('https://example.test/a/%2e%2e/b').pathname === '/b',
          new URL('//other.test/x', 'https://first.test/a').href === 'https://other.test/x',
          encoded.username === 'a%40b' && encoded.password === 'p%23q'
            && encoded.pathname === '/a%3Fb%23c',
          oldHost === 'xn--bcher-kva.example:8080' && unchangedHost && acceptedIPv6
            && keptPort && rejectedHostnamePort && clearedDefaultPort,
          host.href === 'https://other.test/?x=1' && sameParams && linkedParams
            && guardedProtocol && hrefIdentity,
          empty.search === '' && empty.hash === '' && empty.href === 'https://example.test/?#',
          new Request('https://bücher.example/a//b').url === 'https://xn--bcher-kva.example/a//b',
          new URL('child', customBase).href === 'https://base.test/root/child' && baseCalls === 1,
          new URL('https://example.test/\uD800').pathname === '/%EF%BF%BD'
        ];
    }"#), 1);
    assert_eq!(call("nativeUrlCases", "[]"), "[true,true,true,true,true,true,true,true,true,true,true,true,true]");
}

#[test]
fn url_setter_reentrant_string_conversion_uses_latest_url() {
    assert_eq!(load(r#"function reentrantUrlSetter() {
        const url = new URL('https://old.test/old');
        let calls = 0;
        url.pathname = { toString() {
            calls++;
            url.href = 'https://new.test/base';
            return '/outer';
        } };
        return calls === 1 && url.href === 'https://new.test/outer';
    }"#), 1);
    assert_eq!(call("reentrantUrlSetter", "[]"), "true");
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
fn text_decoder_rejects_invalid_second_bytes_without_consuming_them() {
    assert_eq!(
        load(
            r#"function decodeContinuationBounds() {
                 const malformed = [
                   [0xe0, 0x80, 0x80], [0xed, 0xa0, 0x80],
                   [0xf0, 0x80, 0x80, 0x80], [0xf4, 0x90, 0x80, 0x80]
                 ].map(bytes => new TextDecoder().decode(Uint8Array.from(bytes)));
                 const boundaries = [
                   [0xe0, 0xa0, 0x80], [0xed, 0x9f, 0xbf],
                   [0xf0, 0x90, 0x80, 0x80], [0xf4, 0x8f, 0xbf, 0xbf]
                 ].map(bytes => new TextDecoder().decode(Uint8Array.from(bytes)).codePointAt(0));
                 const decoder = new TextDecoder();
                 const split = [
                   decoder.decode(Uint8Array.from([0xe0]), { stream: true }),
                   decoder.decode(Uint8Array.from([0x80]), { stream: true }),
                   decoder.decode(Uint8Array.from([0x80]), { stream: true }),
                   decoder.decode()
                 ];
                 const valid = new TextDecoder();
                 const validFirst = valid.decode(Uint8Array.from([0xf4, 0x8f]), { stream: true });
                 const validLast = valid.decode(Uint8Array.from([0xbf, 0xbf]), { stream: true }).codePointAt(0);
                 const strict = new TextDecoder('utf-8', { fatal: true });
                 strict.decode(Uint8Array.from([0xe0]), { stream: true });
                 let fatalSplit = false;
                 try { strict.decode(Uint8Array.from([0x80]), { stream: true }); }
                 catch (error) { fatalSplit = error instanceof TypeError; }
                 let fatalCurrent = false;
                 try { new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from([0xed, 0xa0]), { stream: true }); }
                 catch (error) { fatalCurrent = error instanceof TypeError; }
                 return { malformed, boundaries, split, validFirst, validLast, fatalSplit, fatalCurrent };
               }"#
        ),
        1
    );
    assert_eq!(
        call("decodeContinuationBounds", "[]"),
        r#"{"malformed":["���","���","����","����"],"boundaries":[2048,55295,65536,1114111],"split":["","��","�",""],"validFirst":"","validLast":1114111,"fatalSplit":true,"fatalCurrent":true}"#
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

/// Real Node's own internal, undocumented per-encoding fast paths
/// (`buf.utf8Slice(start, end)`, `buf.latin1Write(str, offset, length)`,
/// one pair per encoding) -- not part of any public Buffer spec, but a
/// real, popular package (busboy's own `lib/utils.js` decoder table,
/// `data.latin1Slice(0, data.length)`, used while parsing a multipart
/// field's Content-Disposition header) calls them directly as a speed
/// shortcut around the public `toString`/`write`, which thaw's own
/// `Buffer` class (extending `Uint8Array`, reimplementing only the
/// public API) never had -- `req.pipe(busboy)` crashed with a bare
/// `not a function` the moment busboy tried to decode any header value
/// this way, well before its own `field`/`file` events ever fired.
/// Fixed by adding each as a thin alias over the exact same logic
/// `toString`/`write` already implement per encoding.
#[test]
fn buffer_supports_nodes_internal_per_encoding_slice_and_write_aliases() {
    assert_eq!(
        load(
            "function bufferSliceWriteAliases() {\n\
               const text = Buffer.from('hello world');\n\
               const slices = [text.utf8Slice(0, text.length), text.latin1Slice(0, 5), text.asciiSlice(6, 11), Buffer.from('01020304', 'hex').hexSlice(0, 4), Buffer.from('aGVsbG8=', 'base64').base64Slice(0, 8), Buffer.from('aGVsbG8', 'base64url').base64urlSlice(0, 7), Buffer.alloc(4).fill(0x41).ucs2Slice(0, 4)];\n\
               const utf8Target = Buffer.alloc(5); utf8Target.utf8Write('hello', 0, 5);\n\
               const latin1Target = Buffer.alloc(3); latin1Target.latin1Write('abc', 0, 3);\n\
               const hexTarget = Buffer.alloc(2); hexTarget.hexWrite('ff10', 0, 2);\n\
               return [slices, utf8Target.toString(), latin1Target.toString(), hexTarget.toString('hex')];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("bufferSliceWriteAliases", "[]"),
        r#"[["hello world","hello","world","01020304","aGVsbG8=","aGVsbG8","䅁䅁"],"hello","abc","ff10"]"#
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

/// Ed448 (EdDSA over curve448) sign/verify -- one of 4 previously
/// out-of-scope `node:crypto` items the user picked up together (see
/// [[project_crypto_rsa_ecdsa]]'s own "deliberately still out of
/// scope" note). Same methodology as the P-521/Ed25519 test above:
/// verifies a signature produced by real OpenSSL 3.5
/// (`openssl genpkey -algorithm ed448` + `openssl pkeyutl -sign
/// -rawin`), not just an internal round-trip.
#[test]
fn crypto_ed448_sign_and_verify_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function ed448PrivateKey() { return '-----BEGIN PRIVATE KEY-----\\nMEcCAQAwBQYDK2VxBDsEOWI45zIeyrnS8uNzaeXrUST94qjNL9kNXt+kgTRCpDQ+\\n2z9MswYirqpG3vBlGeJPDeOTwUTjRfgBpg==\\n-----END PRIVATE KEY-----\\n'; }\n\
             function ed448PublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMEMwBQYDK2VxAzoAsFWCVQcqbRnP1fEgvOSUY+ox4UCiTUB7BBkIi5qa45sFbG+S\\n9Q7VuaLnCrJEg0kZKGFQZNwh2nKA\\n-----END PUBLIC KEY-----\\n'; }\n\
             function ed448RoundTrip() {\n\
               const message = 'hello ed448 signature test';\n\
               const sig = __thaw_crypto_module.sign('sha256', message, ed448PrivateKey());\n\
               const ok = __thaw_crypto_module.verify('sha256', message, ed448PublicKey(), sig);\n\
               const tamperedMessage = __thaw_crypto_module.verify('sha256', 'goodbye world', ed448PublicKey(), sig);\n\
               const opensslSig = Buffer.from('33e1730abda15ccd9421c9692972486b8f5d276ad88c589e55de128c4821a7faa414bd58da8426a9bd4a953387e4277358715f1e7ea2737e80754233aa485b36a9b52dfc405f8c43d6d95cc2e48d90b74c8bd68217410b1be081c7022de70ddf131eea49f012ddf2d7629aad2c4b73761d00', 'hex');\n\
               const opensslVerifies = __thaw_crypto_module.verify('sha256', message, ed448PublicKey(), opensslSig);\n\
               return [ok, tamperedMessage, opensslVerifies];\n\
             }\n\
             function keyObjectAsymmetricInfoEd448() {\n\
               const priv = __thaw_crypto_module.createPrivateKey(ed448PrivateKey());\n\
               const pub = __thaw_crypto_module.createPublicKey(ed448PublicKey());\n\
               return [priv.asymmetricKeyType, pub.asymmetricKeyType];\n\
             }"
        ),
        1
    );
    assert_eq!(call("ed448RoundTrip", "[]"), "[true,false,true]");
    assert_eq!(
        call("keyObjectAsymmetricInfoEd448", "[]"),
        r#"["ed448","ed448"]"#
    );
}

/// A freshly generated Ed448 keypair (`generateKeyPairSync('ed448')`)
/// signs and verifies correctly, and round-trips through
/// `createPrivateKey`/`createPublicKey` again -- same shape as the
/// existing RSA/EC/Ed25519 keygen test.
#[test]
fn crypto_generates_ed448_key_pairs_that_round_trip() {
    assert_eq!(
        load(
            "function ed448KeygenRoundTrip() {\n\
               const { publicKey, privateKey } = __thaw_crypto_module.generateKeyPairSync('ed448', {});\n\
               const message = 'ed448 keygen round trip';\n\
               const sig = __thaw_crypto_module.sign('sha256', message, privateKey);\n\
               const ok = __thaw_crypto_module.verify('sha256', message, publicKey, sig);\n\
               const privPem = privateKey.export();\n\
               const pubPem = publicKey.export();\n\
               const reimportedPriv = __thaw_crypto_module.createPrivateKey(privPem);\n\
               const reimportedPub = __thaw_crypto_module.createPublicKey(pubPem);\n\
               const sig2 = __thaw_crypto_module.sign('sha256', message, reimportedPriv);\n\
               const ok2 = __thaw_crypto_module.verify('sha256', message, reimportedPub, sig2);\n\
               return [ok, ok2, publicKey.asymmetricKeyType];\n\
             }"
        ),
        1
    );
    assert_eq!(call("ed448KeygenRoundTrip", "[]"), r#"[true,true,"ed448"]"#);
}

/// `privateEncrypt`/`publicDecrypt` -- Node's rarer raw-RSA pair, one
/// of 4 previously out-of-scope `node:crypto` items the user picked up
/// together (see [[project_crypto_rsa_ecdsa]]'s own "deliberately
/// still out of scope" note, and this file's own header comment).
/// Same real-key methodology as the other crypto tests: decrypts a
/// blob produced by real OpenSSL 3.5 (`openssl pkeyutl -sign`, the
/// modern name for the deprecated `rsautl -sign` this pair of
/// functions mirrors), then round-trips thaw's own `privateEncrypt`
/// output back through `publicDecrypt`.
#[test]
fn crypto_rsa_private_encrypt_and_public_decrypt_interoperate_with_real_openssl() {
    assert_eq!(
        load(
            "function rsaPrivateKey() { return '-----BEGIN PRIVATE KEY-----\\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDcIyI5ggrGi37a\\nbJlqjIXhx6C/UdL0343U3O8FCJJ6Zj9P+36zVTyrXmu747AtOmM0XBi9XVKy/GnM\\n2VN4onr9VJUfU0tZOcZ4m4JCOvR0VIlCL/aMnIbz/oRkH4BsJTrDCANr4FaCvqTu\\nT4qdPAOv3fXHfl9R1p4MsJ2aNU8oB1EYsFwAlKBjD6R5CPMmdYgK7BKD/dkmNyPr\\nEXCU8mWbmKYlAVUMZ7LDPQ/7Uaqx2Qn5Ue6twCiaacYlZlQkZg99UrcOzdODV+Gm\\nwU85q3jmazMXa+9/fXhzW2O2m++4CpXIuI9418vr9kIlHNLh82yfWQ19XNUGEAWf\\ne0C7EKhBAgMBAAECggEAAjUdipPYcrpg6Q+SVEUY5SFeql6c5UmS3+rDpsoJbYyW\\nx2M5q2Cuj2jhIGwoL5d6BQpF6yX8PIbHnD8wSF8xXG2TGKqNwE9naxxxWM5A6vZ5\\nYxaaaunGP3g7QcqHY6Zp1auHFx9G7hjpEc1UowMSxp0n3gz2J35NoIWMviY7ZFcj\\nL2iIYULm1dBhCTq7/v9qbPlE4nid0SV2OyV7yhRhIJPVqxxlTSC+4ZX3RZvUozbt\\nV2o3b88LoG0F0wy9hsKCwOnZxhYAf4R9p03qk/jYMeMK1DpZgHvUe6pyNOjxCbK1\\n4R6JJEWN0yfQLMHuuZpaW1SxQynMopwIoXFohJUPxwKBgQD7Q+8K8EjHNv+A7oxF\\nCYEtUYZiWBmqsCXQ18RhTGc75KoX1zsLa2sOHVOUSYHF5EKwxZsXDxV9AnOqm5Z3\\nkUDHGTsu9mxnk/iwv9gnS0IwYpLnm02SY4arXQcGHXUVNWAGIWLaOx78ocg8O1HI\\nx4cGDt8PfO3vp14Jpip9yGKqewKBgQDgSQrkV3NMAHWkPs18f9ugfpv9GSHqNhnO\\n1Sw1LH3hoL43zec6pxoHMF6/CctzyyNomkmKGKEZjdTY8CPWhQM4Djo6I4Wq97mp\\nYjT5v+Z7DCj4e0huS+h4bMl1XTwrKxRMlvAi0c8AczfLAltsCD/PVs0qX9Qg+o6i\\nGoPRtxNJcwKBgHJzc0MsSDpWFvQHtOUNe0XFSM0rDCXvron+fnlDcBKcCc5qP37o\\nIw9+1D9LbE1Tt/0FRauvNz6GC2G/FT7JbxRBre+qV56mjDUWbcMYSMH5ZKkS2LbB\\nluofqb9jU52hfmfMdVaqb2br2mV1L7+hAyQDSh+n7EmplvAWPGynBipZAoGAaxYU\\n8D9c2m3hvYEK5aW6fF/XJLoqOkSIf/vCNsU+eUshZ02VWKjOQZ5zrm0Dyg60ok4A\\nTMJDsQrKFKZbxiIODmakoHuzZ5UN/XTZbGGWryt4KGPcimUN4um2KqZQgx/3ejYb\\nA9T/K+zXN8OxWNx7cwizvsawZuqazYUxaSErQUcCgYBH0pgPF+NGOF7/cD9aIdQc\\nyJMlJHHL71tWf2IvxK3k8caht1LaikXCY2rJGIvMYXwaSb314W+7tvba0UnWjzVp\\nPMEP6voIWudjgD4OoXilwddDxde8owbaPcgDZW4E0R0sutIywPM11s6E62nWQRUu\\n6oI/67AYv1wdhI8IsP0LgQ==\\n-----END PRIVATE KEY-----\\n'; }\n\
             function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA3CMiOYIKxot+2myZaoyF\\n4cegv1HS9N+N1NzvBQiSemY/T/t+s1U8q15ru+OwLTpjNFwYvV1SsvxpzNlTeKJ6\\n/VSVH1NLWTnGeJuCQjr0dFSJQi/2jJyG8/6EZB+AbCU6wwgDa+BWgr6k7k+KnTwD\\nr931x35fUdaeDLCdmjVPKAdRGLBcAJSgYw+keQjzJnWICuwSg/3ZJjcj6xFwlPJl\\nm5imJQFVDGeywz0P+1GqsdkJ+VHurcAommnGJWZUJGYPfVK3Ds3Tg1fhpsFPOat4\\n5mszF2vvf314c1tjtpvvuAqVyLiPeNfL6/ZCJRzS4fNsn1kNfVzVBhAFn3tAuxCo\\nQQIDAQAB\\n-----END PUBLIC KEY-----\\n'; }\n\
             function opensslDecrypt() {\n\
               const cipher = Buffer.from('4bbb92f233871b0feb4b6f8dcc01018d96342275b3159486dad381bd785add5643038ed41e0b30e462c42d9f9ea94ed0030a728453ff4c1e3613acb077a2713f9a88e9fc47937e873d22a95fe4f4ffbf334f3190c8f9948815321a329c68861dd0b8a44040eaa847fd0c0b8409a7d3ec65868e3ef930a76462aaa06fc1dff5c8679cccfaa6f591d1921c29db087373e018d2f476ca942cd9f69764e07bfb4410f6a6c484fcbfd140ab421ade5af4163b80bacb0538b7a52fc9cab5ffa380fc99f0a507a287a370f1bd565330eaed8f21812602ef8543614cac7aa517b9b2950fbca6477c222e92503ebdbfa95465f727c07a5ac43429f6f07a40031fdcc5667c', 'hex');\n\
               return __thaw_crypto_module.publicDecrypt(rsaPublicKey(), cipher).toString();\n\
             }\n\
             function thawRoundTrip() {\n\
               const cipher = __thaw_crypto_module.privateEncrypt(rsaPrivateKey(), Buffer.from('round trip raw rsa'));\n\
               return __thaw_crypto_module.publicDecrypt(rsaPublicKey(), cipher).toString();\n\
             }"
        ),
        1
    );
    assert_eq!(call("opensslDecrypt", "[]"), r#""hello raw rsa test""#);
    assert_eq!(call("thawRoundTrip", "[]"), r#""round trip raw rsa""#);
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

/// The legacy OpenSSL "traditional" PKCS#1 PEM encryption format
/// (`Proc-Type: 4,ENCRYPTED` / `DEK-Info: <cipher>,<iv>`, predating
/// PKCS#8's own unrelated `ENCRYPTED PRIVATE KEY` header) -- the third
/// of 4 previously out-of-scope `node:crypto` items the user picked up
/// together (see [[project_crypto_rsa_ecdsa]]'s own "deliberately
/// still out of scope" note, and this module's own header comment).
/// Two real OpenSSL 3.5-produced key files
/// (`openssl rsa -in plain.pem -des3/-aes256 -traditional -passout
/// pass:hunter2`), one per supported cipher family (3DES, AES) --
/// decrypted and used to sign, verified against the plain (unencrypted)
/// public key derived from the same original key.
#[test]
fn crypto_legacy_encrypted_pkcs1_pem_interoperates_with_real_openssl() {
    assert_eq!(
        load(
            "function rsaPublicPem() { return '-----BEGIN PUBLIC KEY-----\\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsFF0EaFZ+6scQKkIVSoa\\nf8oy5gXO8TDhBbKh7SK0P8ALHEB4flyKVaL4Qvzbh5aRrfpOLwCmlo9ucr4V95SH\\nOX7dZ5eo2fONA5CNIaMNY5NzebjV8+kYj4ICRTLugR654Bf22EqCOtMsy4EJ15Ce\\nuNfY6+N54bj8pPDyO81NEwn3bRdZr6Wn3Nut6mRG30j/etwNGSFxXoChUAhCbcfL\\nxdGzEdIOpL3qBLFnJ2cmurC02wVEx6F1eJt2G6fedxoAgogCtDDtMa89FmeKNh11\\nCVxjc6Ma/vGh3jzO5zZegP5yuPGm9mKGsvmNYm6GHCQJUhUmdNNJx0hqDP+gC3X7\\nXQIDAQAB\\n-----END PUBLIC KEY-----\\n'; }\n\
             function rsaPrivateDes3Pem() { return '-----BEGIN RSA PRIVATE KEY-----\\nProc-Type: 4,ENCRYPTED\\nDEK-Info: DES-EDE3-CBC,B000B3DC7910EE4F\\n\\nERnhRI/uyDWAr0JB9jmi8KtmYYV7cUFvVYAIIv7iUSeH3o18T+jLHKvxb5rQDXKy\\nSj+sjFWzf/+DaJo/6SXp81RzS3hvIHj0TG5JzGZT8nPk18JOIXB5BUQ6CbFBtfON\\nE+wPTU4+cwUlL0ugbbHiKx7tdrtuQT5fUjvWhpZOER+fKyi7uCUyrpZyhPGvlsMA\\n2rEv5DfWk494rCnHUg+da2mFT0PKtPGDPUBxIcNwlfwNZERMuwlFYiVTKIclncAn\\n0K1PjIskNixOG3SkYVhp8YOKfVar/sFaU7DzPQTXqGJIGOhHQeT9ILUcY4Yb17jx\\ngPMtH7gw8j5B68F/r4hl+KIQXSg05G1SkGXKM2zvWu1UUIpW2LlYlD5D4UxtiCit\\nNSeV5xVpv4wZZzDTLZAZjRS0SUZ8plY16k9SNQdfTCB+/biFxI/kIHv/hQqB8+ML\\nTe8W0rzxDRIwYjc0MB0nhjsfnb+dT6EiNSW4jZEWS5Z5G88CUVjFNI2UOFjwlIO0\\nOd0e6d0vuMPJ9rRQMGKA0TfPyioYkJUQ0ED+13ML1W9wC+MvkG7SjtQKLJDQzRMk\\nyYGHiToYI9DktYTGipbB2YEt5I/07YWKgEa++dkGYsQZOqFoOGvOvdwZeo2laZjG\\nAAp2Q1F2ji5wQR5mUZOtqBWjFJc0VRYA3S26wwqaOk1axydFt6ilk/E1BeblJNzN\\nxfmtsg1ZQJof3YTYtlkn2ur/3PWsmerip8H5TgyAIsF3bHVy4hWn/GdFE+LCPkgb\\nVIO0Qekz1nANYjl4kfPi/ImCB6VtHkp7aOAC/2QvFu8rs4F/+Mtyxsx+OjRH9LEM\\nplyiA3hNmyccMXlSLnVJA7tQ1ub0fkGb9y9xRwKM7SP8CA+xvnsVjoq3qFp/Wyft\\ngDyB/gLZE/TlXH5NftHi7qkSSb3ZHslFJjJbuG6q/Byv8/VgVk7rwpHRkGH6lPRm\\nTZvAqI0nSR3+4eq586598kJcylrxFgcRysZRQUVVInnmKZcQqZIl24l4D5o0Q5al\\na+RN1ICirl9Gc7TVjxntbt3JpSjFbFkN+LQNz3fiim+N6+jOC/CURQf9T45AWyWM\\n+8zZCiMm20EA+WUGqbiSV4LAPT+3QMOGJZHhbwauipyww2s7Cg+2WJQOnJRzqQ6/\\no0DQqX8v4otuhAPrDFvdex9tIJiFjicOrBTsuMe8DSPHMqYh8EktuI4WSKVHXs95\\nujyVq1BFWfFacE4PyVXjbWXMBHC9Ln/HnwIbMmHNxZQF1vgPZitWITYGuODrZgaV\\naNiNmnrgwlwxETidQlcgz7vbVdm6AQJjg8b8mGzI8Ms1w9JPx3HWjmHUW+bKgbHM\\nTiOEKdwK1R+0yYUpcdaUb8g5tgH9QtBD+zz3amI9bncYjUYBzD9x9lJ5qEFg8F7z\\nHhV8PolH4R9IoDZdJ5NW7hzfw2itcfGiNAn6FOwqjDRvfZz7T4GprFU4r6A5PIrW\\nvXxayS0owhGxa46wjg0h+Pwz2ykuMJwXc6f7LQ2YfO83DAT/BdpA7NEKBA/u3cKG\\ngANfOLN5Qx2EPnPvQ7QzQkhk+ICOVRdga5n8JL4FXK9anDSCdBNjrw==\\n-----END RSA PRIVATE KEY-----\\n'; }\n\
             function rsaPrivateAes256Pem() { return '-----BEGIN RSA PRIVATE KEY-----\\nProc-Type: 4,ENCRYPTED\\nDEK-Info: AES-256-CBC,520C22EC6F81EC4DA4E87A99C3B0EDBB\\n\\nkXE+R6HCuJ+Pl1O79NeU0Zpx/NGzD51NxmHAN6R2ky7zyX3DHhzXvnrVEfXdTIAm\\np7Ptg+BMiZyynLPYP3C5okveUm0RL+K0SLdcssczxYMw8Xrvo1wNVsX4M0ISy+h5\\nvPPLlDlot7BQwWWlnXC4rqJIo0N6Z2KYtDXiDCXTQA870PHGEGHTTey7Cpi82tOS\\nrBVZElzIJ0rVc1i+CSjAFlyctvD75fy+ixZ45Npl50UUMOfHCu8zc3jetLU+1jxy\\nbBkGvhKhhCuUhaNRjhhITiqJcvfNzvuVmjaiIFVC4LWX9p5aq8c2aDPtt10gqRKZ\\n6sH5t3DUYSwvsPQUPrIDpeGapfs3wifeqmdyTkOynk43fvurENUM2MLX4WTs5VG1\\nAHZHAMWR2HeX1nECDGaAr2Qkw5B9y1gUbIugDoq9rD27EDwqbfiCmJWJfWTsePtJ\\nGNvckfDEkAkmlPg1j0oWpMM4G0ojFV/oiyKePebG82rZDuUINM5UFNd+z3jXe+0J\\nw67ZKJvOtneITWL3MKsyjJbc9V6LlsH3xJcGKsSrfFhXF4iPPaHjsGC5q6Snx6IX\\nd/S9RUcmddcmNnZHW48v8MI8wWc+ysEyodPM+zPOk+pyBVPkUcMmpv1HVaNdv3sZ\\nhTij2VxXfuBKU9wp83PtQf3vZcMFkD2J/JHuuAYXrvhxOAHrIl6TWsJBuZa6CChZ\\nB9myNPbPb8b14VuYNjdjec8USsnZPt3U6QP38cEO2WUZeLVBsk9zUZK5Dkj5phy5\\nPBqlIBL0u+EmcUubYYD86aqTomdH5uQkM2wMCdNmM7Xb1g5j06lpyRfdSISZG/ps\\nNu4KxxJGCWa6i5NswtbkyuXExXOjt8CeCLvBjotysUYPnL0AC56bWQG1LNanV3Ef\\nIUip05vJTFSz9jIoB5tCrjSyb4vZ9WtFhOF5I5w9sPmhvinD+AjT//FLF4+O6Wue\\nWQxt0Gyj5BSOjh7nOrQZGVXHNhrQJkAQZMv07OXMoJjgeA7I87Y6WWukv+9QRXn2\\nviBN5I7Iq63VQXzN2RVUHKbZPIgu9P0IwyMHrnfUwnDN8074NeDpRPw6snSAsarZ\\nPzh0ZuvxXV4iS17P4VnGJ1oCZnkKPU4phvH00ZkQg0mXIy4tH9tn/MhQpy9GQEmh\\n//PtfKKAp7s9ctVqDNwNjt1GY++gHP9GmLOdFS2AJFipQqj6mzM+foOsNyhalIqS\\ntXXLaSUAt89F4JED6A8xDJMFANUm5NKmNOTFcbeNzeEjq8gxPZXtCMxyH8k77vwR\\n9oLKZMtToD8yF09sCAM9vQyztwEgV/u7n+9x1t51YnODmVrIsvQbYBU/HxmMUQd7\\nqNkfCugPprJ7OhMNbWafnTeyPacr30AsEcJ7IgOxGPY3rt2VPxObI14clPbVEz+1\\nO4NMzbaSYHwILAkxyNIrnf0ZzXOXKkgN0elMxnc2AwDkjm79R0KGAh4+yLxWPlxo\\nfPgzxmo+3lI7fpILxwotzXnPXV2YxI7QIeFA+/q0kC8NTw2d0WoSDm4m8N0J7MR6\\npbO6Mu0/vl5u6p8WhJKnxPOrSWVtTsDo2vm99fSkBHyUnbqqJr/FwUvaTWBAh/Lk\\n-----END RSA PRIVATE KEY-----\\n'; }\n\
             function legacyRoundTrip(pem) {\n\
               const priv = __thaw_crypto_module.createPrivateKey({ key: pem, passphrase: 'hunter2' });\n\
               const sig = __thaw_crypto_module.createSign('sha256').update('legacy pem test').sign(priv, 'hex');\n\
               const ok = __thaw_crypto_module.createVerify('sha256').update('legacy pem test').verify(rsaPublicPem(), sig, 'hex');\n\
               let wrongPassphraseThrew = false;\n\
               try { __thaw_crypto_module.createPrivateKey({ key: pem, passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }\n\
               return [priv.asymmetricKeyType, ok, wrongPassphraseThrew];\n\
             }\n\
             function des3RoundTrip() { return legacyRoundTrip(rsaPrivateDes3Pem()); }\n\
             function aes256RoundTrip() { return legacyRoundTrip(rsaPrivateAes256Pem()); }"
        ),
        1
    );
    assert_eq!(call("des3RoundTrip", "[]"), r#"["rsa",true,true]"#);
    assert_eq!(call("aes256RoundTrip", "[]"), r#"["rsa",true,true]"#);
}

/// Passphrase-protected X25519 keys -- the last of 4 previously out-of-
/// scope `node:crypto` items the user picked up together (see
/// [[project_crypto_rsa_ecdsa]]'s own "deliberately still out of
/// scope" note, and this module's own header comment). Decrypts a real
/// OpenSSL 3.5-produced encrypted key (`openssl genpkey -algorithm
/// X25519` + `openssl pkey -aes256 -passout pass:hunter2`) and computes
/// a Diffie-Hellman shared secret against a real peer public key,
/// cross-checked against `openssl pkeyutl -derive`'s own output for
/// the same key pair.
#[test]
fn crypto_x25519_passphrase_protected_key_interoperates_with_real_openssl() {
    assert_eq!(
        load(
            "function x25519PrivateEncryptedPem() { return '-----BEGIN ENCRYPTED PRIVATE KEY-----\\nMIGjMF8GCSqGSIb3DQEFDTBSMDEGCSqGSIb3DQEFDDAkBBB7tmOZrnczjFnNG9jN\\nYZ1fAgIIADAMBggqhkiG9w0CCQUAMB0GCWCGSAFlAwQBKgQQ7nt4x5kr1Oyp/30P\\nhjd1KwRAPOtfFpXXehOtGmKdUDrNZt0kN1BGoevUg1UaV6IifpLoD/7rx0yKDYi7\\nj3Mlx1DVoubvAvNgDari+t5VGInS5w==\\n-----END ENCRYPTED PRIVATE KEY-----\\n'; }\n\
             function peerPublicPem() { return '-----BEGIN PUBLIC KEY-----\\nMCowBQYDK2VuAyEA6PZLnkCbBlDxzjBJOcUA+J4MLON3SqHkcn7rLRsauks=\\n-----END PUBLIC KEY-----\\n'; }\n\
             function x25519PassphraseRoundTrip() {\n\
               let wrongPassphraseThrew = false;\n\
               try { __thaw_crypto_module.createPrivateKey({ key: x25519PrivateEncryptedPem(), passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }\n\
               const priv = __thaw_crypto_module.createPrivateKey({ key: x25519PrivateEncryptedPem(), passphrase: 'hunter2' });\n\
               const secret = __thaw_crypto_module.diffieHellman({\n\
                 privateKey: priv,\n\
                 publicKey: __thaw_crypto_module.createPublicKey(peerPublicPem())\n\
               }).toString('hex');\n\
               return [priv.asymmetricKeyType, secret, wrongPassphraseThrew];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("x25519PassphraseRoundTrip", "[]"),
        r#"["x25519","8045311ec4d1998f5439eeec2d2eb0616fe269acce92e20d9626f719db2b7707",true]"#
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

/// Encrypted private-key output is unsupported. Reject every supplied cipher
/// or passphrase before native key generation or plaintext KeyObject export;
/// keep ordinary unencrypted KeyObject, PEM, and DER paths available.
#[test]
fn crypto_rejects_encrypted_private_key_output_before_generation_or_export() {
    assert_eq!(load(r#"
      function encryptedPrivateOutputGuards() {
        const cryptoModule = __thaw_crypto_module;
        const originalGenerate = __thaw_crypto_generate_key_pair_json;
        const originalQueue = queueMicrotask;
        let nativeCalls = 0, queued, callbackCount = 0, callbackFailed = false;
        const rejects = [];
        try {
          __thaw_crypto_generate_key_pair_json = () => { nativeCalls++; return JSON.stringify({ publicPem: 'PUBLIC', privatePem: 'PRIVATE' }); };
          queueMicrotask = job => { queued = job; };
          for (const privateKeyEncoding of [
            { type: 'pkcs8', format: 'pem', cipher: 'aes-256-cbc', passphrase: 'secret' },
            { type: 'pkcs8', format: 'pem', cipher: '' },
            { type: 'pkcs8', format: 'der', passphrase: false },
            { type: 'pkcs8', format: 'der', passphrase: null }
          ]) {
            try { cryptoModule.generateKeyPairSync('rsa', { modulusLength: 512, privateKeyEncoding }); rejects.push(false); }
            catch (error) { rejects.push(error instanceof TypeError); }
          }
          cryptoModule.generateKeyPair('rsa', { modulusLength: 512,
            privateKeyEncoding: { type: 'pkcs8', format: 'pem', passphrase: '' } },
            (error, publicKey, privateKey) => {
              callbackCount++;
              callbackFailed = error instanceof TypeError && publicKey === undefined && privateKey === undefined;
            });
          queued();
          let changingReads = 0;
          try {
            cryptoModule.generateKeyPairSync('rsa', { modulusLength: 512,
              get privateKeyEncoding() {
                changingReads++;
                return changingReads === 1
                  ? { type: 'pkcs8', format: 'pem', cipher: 'aes-256-cbc' }
                  : { type: 'pkcs8', format: 'pem' };
              }
            });
            rejects.push(false);
          } catch (error) { rejects.push(error instanceof TypeError && changingReads === 1); }
          let parentReads = 0;
          const options = {
            modulusLength: 512,
            get privateKeyEncoding() { parentReads++; return { type: 'pkcs8', format: 'pem' }; },
            publicKeyEncoding: { type: 'spki', format: 'pem' }
          };
          const ordinary = cryptoModule.generateKeyPairSync('rsa', options);
          rejects.push(parentReads === 1 && ordinary.publicKey === 'PUBLIC' && ordinary.privateKey === 'PRIVATE');
        } finally {
          __thaw_crypto_generate_key_pair_json = originalGenerate;
          queueMicrotask = originalQueue;
        }
        const generated = cryptoModule.generateKeyPairSync('ed25519');
        const privateKey = generated.privateKey;
        const unencrypted = privateKey.export();
        let directRejected = false, passphraseRejected = false;
        try { privateKey.export({ cipher: 'aes-256-cbc', passphrase: 'secret' }); }
        catch (error) { directRejected = error instanceof TypeError; }
        try { privateKey.export({ passphrase: null }); }
        catch (error) { passphraseRejected = error instanceof TypeError; }
        const publicUnaffected = generated.publicKey.export({ cipher: 'ignored' }) === generated.publicKey.export();
        const secretUnaffected = cryptoModule.createSecretKey(Buffer.from('key')).export({ cipher: 'ignored' }).toString() === 'key';
        return [rejects, nativeCalls, callbackCount, callbackFailed, directRejected,
          passphraseRejected, typeof unencrypted === 'string' && unencrypted.indexOf('-----BEGIN PRIVATE KEY-----') === 0,
          publicUnaffected, secretUnaffected];
      }
    "#), 1);
    assert_eq!(call("encryptedPrivateOutputGuards", "[]"),
      "[[true,true,true,true,true,true],1,1,true,true,true,true,true,true]");
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
                 longGenericName('Asia/Seoul', 1782907200000),\n\
                 longGenericName('Asia/Hong_Kong', 1782907200000),\n\
                 longGenericName('Asia/Dubai', 1782907200000),\n\
                 longName('Asia/Kolkata', 1782907200000),\n\
                 longGenericName('Asia/Kolkata', 1782907200000),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("allCases", "[]"),
        r#"["Eastern Daylight Time","Eastern Standard Time","Eastern Time","Japan Standard Time","Japan Standard Time","Australian Eastern Standard Time","Australian Eastern Daylight Time","Australian Eastern Time","Korean Standard Time","Hong Kong Standard Time","Gulf Standard Time","India Standard Time","India Standard Time"]"#
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
             function significantDigits() {\n\
               const f = (opts, n) => new Intl.NumberFormat('en-US', { useGrouping: false, ...opts }).format(n);\n\
               return [f({ maximumSignificantDigits: 3 }, 1234), f({ maximumSignificantDigits: 3 }, 1.2345), f({ minimumSignificantDigits: 3 }, 2), f({ maximumSignificantDigits: 2, style: 'percent' }, 0.1234)];\n\
             }\n\
             function useGroupingModes() {\n\
               const f = (u, n) => new Intl.NumberFormat('en-US', { useGrouping: u }).format(n);\n\
               return [f(true, 1000), f(false, 1000), f('auto', 1000), f('always', 1000), f('min2', 1000), f('min2', 10000)];\n\
             }\n\
             function groupingResolved() {\n\
               return [\n\
                 new Intl.NumberFormat('en-US').resolvedOptions().useGrouping,\n\
                 new Intl.NumberFormat('en-US', { useGrouping: true }).resolvedOptions().useGrouping,\n\
                 new Intl.NumberFormat('en-US', { useGrouping: false }).resolvedOptions().useGrouping,\n\
               ];\n\
             }\n\
             function signDisplays() {\n\
               const f = (s, n) => new Intl.NumberFormat('en-US', { useGrouping: false, signDisplay: s }).format(n);\n\
               return [\n\
                 f('auto', 5), f('auto', 0), f('auto', -5), f('auto', -0),\n\
                 f('never', -5),\n\
                 f('always', 5), f('always', 0), f('always', -5), f('always', -0),\n\
                 f('exceptZero', 5), f('exceptZero', 0),\n\
                 f('negative', -0),\n\
                 new Intl.NumberFormat('en-US', { useGrouping: false, style: 'percent', signDisplay: 'always' }).format(0.5),\n\
               ];\n\
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
        call("significantDigits", "[]"),
        r#"["1230","1.23","2.00","12%"]"#
    );
    assert_eq!(
        call("useGroupingModes", "[]"),
        r#"["1,000","1000","1,000","1,000","1000","10,000"]"#
    );
    assert_eq!(call("groupingResolved", "[]"), r#"["auto","always",false]"#);
    assert_eq!(
        call("signDisplays", "[]"),
        r#"["5","0","-5","-0","5","+5","+0","-5","-0","+5","0","0","+50%"]"#
    );
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
fn iterator_from_works_through_dynamic_handles() {
    let iterator_name = CString::new("Iterator").unwrap();
    let iterator = thaw_js_get_global(iterator_name.as_ptr());
    assert_ne!(iterator, 0);

    let from = CString::new("from").unwrap();
    let source = CString::new("[[1,2,3,4]]").unwrap();
    let values = thaw_js_call_method_handle_result(iterator, from.as_ptr(), source.as_ptr(), true);
    assert!(values.error.is_null());

    let next = CString::new("next").unwrap();
    let empty = CString::new("[]").unwrap();
    let result = thaw_js_call_method_result(values.value, next.as_ptr(), empty.as_ptr());
    assert!(
        result.error.is_null(),
        "{}",
        unsafe { CStr::from_ptr(result.error) }.to_str().unwrap()
    );
    assert_eq!(
        unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
        "{\"value\":1,\"done\":false}"
    );

    let helper_name = CString::new("__thaw_iterator_from").unwrap();
    let helper = thaw_js_get_global(helper_name.as_ptr());
    let source = CString::new("[[1,2,3,4]]").unwrap();
    let values = thaw_js_call_handle_handle_result(helper, source.as_ptr(), true);
    assert!(values.error.is_null());
    let drop_name = CString::new("drop").unwrap();
    let one = CString::new("[1]").unwrap();
    let dropped =
        thaw_js_call_method_handle_result(values.value, drop_name.as_ptr(), one.as_ptr(), true);
    assert!(
        dropped.error.is_null(),
        "{}",
        unsafe { CStr::from_ptr(dropped.error) }.to_str().unwrap()
    );
    let to_array = CString::new("toArray").unwrap();
    let result = thaw_js_call_method_result(dropped.value, to_array.as_ptr(), empty.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(
        unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
        "[2,3,4]"
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
    assert_eq!(thaw_js_retain_handle(original), 1);
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
    assert_eq!(thaw_js_release_handle(returned.value), 1);
    assert_eq!(thaw_js_release_handle(returned.value), 0);
}

#[test]
fn reuses_a_released_handle_slot() {
    assert_eq!(
        load("globalThis.firstSlot = {}; globalThis.secondSlot = {};"),
        1
    );
    let first = CString::new("firstSlot").unwrap();
    let second = CString::new("secondSlot").unwrap();
    let released = thaw_js_get_global(first.as_ptr());
    assert_eq!(thaw_js_release_handle(released), 1);
    assert_eq!(thaw_js_get_global(second.as_ptr()), released);
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
fn napi_reference_callback_separates_throw_from_returned_error_shaped_object() {
    assert_eq!(load(r#"globalThis.__thaw_napi_reference_746291101 = () => ({ __thaw_error__: 'ordinary' });
globalThis.__thaw_napi_reference_746291102 = () => { throw new TypeError('boom\0tail'); };"#), 1);
    let args = c"[]";
    let returned = unsafe { thaw_js_call_reference(746291101_usize as *mut std::ffi::c_void, args.as_ptr()) };
    let returned = unsafe { CStr::from_ptr(returned) }.to_str().unwrap();
    assert_eq!(returned, r#"{"__thaw_error__":"ordinary"}"#);
    let thrown = unsafe { thaw_js_call_reference(746291102_usize as *mut std::ffi::c_void, args.as_ptr()) };
    let thrown = unsafe { CStr::from_ptr(thrown) }.to_str().unwrap();
    let message: String = serde_json::from_str(thrown.strip_prefix('\u{2}').unwrap()).unwrap();
    assert!(message.starts_with("\u{1}TypeError\u{1}"));
    assert!(message.contains("boom"));
    assert!(message.contains('\0'));
    assert!(message.contains("tail"));
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
        0,
    );
    let second = thaw_js_register_native_callback(
        callback as *const c_void,
        context,
        0,
        2,
        0,
        std::ptr::null(),
        0,
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

#[test]
fn shared_json_reviver_reconstructs_well_known_symbol_values() {
    assert_eq!(
        load(
            "function reviveSymbol() { const value = JSON.parse('[\"\\\\u001f@@iterator\"]', __thaw_json_date_reviver); return value[0] === Symbol.iterator; }"
        ),
        1
    );
    assert_eq!(call("reviveSymbol", "[]"), "true");
}

#[test]
fn property_key_bridge_distinguishes_registered_symbols() {
    assert_eq!(
        load(r#"function registeredSymbolPropertyKeys() {
          const key = 'part:\u0000\ud800';
          const registered = __thaw_property_key('\u0003R41:' + key);
          const repeated = __thaw_property_key('\u0003R41:' + key);
          const anotherWire = __thaw_property_key('\u0003R42:' + key);
          const unique = __thaw_property_key('\u000341:' + key);
          const empty = __thaw_property_key('\u0003R43:');
          const object = { [registered]: 9 };
          const revived = JSON.parse(JSON.stringify([
            '\u0003R44:' + key, '\u0003R45:' + key, '\u000346:' + key, '\u0003R47:'
          ]), __thaw_json_date_reviver);
          return [registered === Symbol.for(key), repeated === registered,
            anotherWire === registered, unique !== registered,
            object[Symbol.for(key)] === 9, empty === Symbol.for(''),
            revived[0] === registered, revived[1] === registered,
            revived[2] !== registered, revived[3] === empty,
            __thaw_property_key('\u001f@@iterator') === Symbol.iterator];
        }"#),
        1
    );
    assert_eq!(call("registeredSymbolPropertyKeys", "[]"), "[true,true,true,true,true,true,true,true,true,true,true]");
}

#[test]
fn native_loose_equality_helper_preserves_primitive_order_and_identity() {
    assert_eq!(load(r#"function nativeLooseEquality() {
      const effects = [];
      const left = function () {};
      const right = function () {};
      left[Symbol.toPrimitive] = () => { effects.push('left'); return 42; };
      right[Symbol.toPrimitive] = () => { effects.push('right'); return 42; };
      const number = __thaw_native_loose_equal(left, 42, 0, 0);
      const reversed = __thaw_native_loose_equal(42, right, 0, 0);
      const bigint = __thaw_native_loose_equal(left, '42', 0, 1);
      const large = function () {};
      large[Symbol.toPrimitive] = () => 9223372036854775807n;
      const largeBigint = __thaw_native_loose_equal(large, '9223372036854775807', 0, 1);
      const registered = function () {};
      registered[Symbol.toPrimitive] = () => Symbol.for('tag');
      const symbol = __thaw_native_loose_equal(registered, '\u0003R100:tag', 0, 2);
      return [number, reversed, bigint, largeBigint, symbol, effects.join(',')];
    }"#), 1);
    assert_eq!(call("nativeLooseEquality", "[]"), "[true,true,true,true,true,\"left,right,left\"]");
}

#[test]
fn property_key_bridge_preserves_symbol_identity() {
    assert_eq!(
        load(
            "function symbolPropertyKeys() { const object = {}; const wellKnown = __thaw_property_key('\\u001f@@iterator'); const first = __thaw_property_key('\\u00031:key'); const same = __thaw_property_key('\\u00031:key'); const other = __thaw_property_key('\\u00032:key'); object[wellKnown] = 1; object[first] = 2; return [wellKnown === Symbol.iterator, Reflect.has(object, Symbol.iterator), first === same, first !== other, object[same]]; }"
        ),
        1
    );
    assert_eq!(call("symbolPropertyKeys", "[]"), "[true,true,true,true,2]");
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

/// Real multi-locale `Intl.DateTimeFormat` (M4 of docs/design/
/// intl-polyfill.md's "Real CLDR data via icu4x" plan) --
/// `intl_datetime.rs`'s `__thaw_intl_datetime_format_parts`, replacing
/// the previous fixed English/Latin-numeral rendering for any curated
/// locale. Every case cross-checked against real Node's own output for
/// the same locale/options/instant, including a locale with real
/// non-Latin digits (`ar-SA`, Arabic-Indic) and a locale whose default
/// hour cycle differs from `en-US`'s (`de-DE`, h23) -- confirming both
/// come from icu4x's own locale-derived defaults, not anything this
/// polyfill hardcodes per locale.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_format_matches_real_node_for_curated_non_english_locales() {
    assert_eq!(
        load(
            "function show(locale, opts, epochMs) {\n\
               return new Intl.DateTimeFormat(locale, opts).format(new Date(epochMs));\n\
             }\n\
             function all() {\n\
                const t = Date.parse('2024-07-04T16:30:45.000Z');\n\
                const midnight = Date.parse('2024-07-04T00:00:00.000Z');\n\
                return [\n\
                  show('ja-JP', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),\n\
                  show('ja-JP', {year:'numeric',month:'2-digit',day:'2-digit',timeZone:'UTC'}, t),\n\
                  show('ja-JP', {weekday:'long',timeZone:'UTC'}, t),\n\
                  show('de-DE', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),\n\
                  show('de-DE', {hour:'numeric',minute:'numeric',timeZone:'UTC'}, t),\n\
                  show('fr', {weekday:'long',year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),\n\
                  show('ar-SA', {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}, t),\n\
                  show('en-US', {hour:'numeric',timeZone:'UTC'}, t),\n\
                  show('en-US', {hour:'numeric',hour12:false,timeZone:'UTC'}, t),\n\
                  show('en-US', {hour:'2-digit',minute:'2-digit',hourCycle:'h24',timeZone:'UTC'}, midnight),\n\
                  show('en-US', {hour:'numeric',minute:'2-digit',hourCycle:'h24',timeZone:'UTC'}, midnight),\n\
                  show('en-US', {hour:'2-digit',minute:'2-digit',hourCycle:'h23',timeZone:'UTC'}, midnight),\n\
                ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "2024年7月4日",
            "2024/07/04",
            "木曜日",
            "4. Juli 2024",
            "16:30",
            "jeudi 4 juillet 2024",
            "٤ يوليو ٢٠٢٤",
            "4 PM",
            "16",
            "24:00",
            "24:00",
            "00:00",
        ])
        .unwrap()
    );
}

/// Non-Gregorian calendar systems (M5) -- `DateTimeFormatter`'s
/// `AnyCalendar` dispatch automatically picks the right calendar from
/// either the locale's own CLDR default (`th-TH` -> Buddhist, no
/// explicit `calendar` needed) or an explicit `calendar` option/`-u-ca-`
/// subtag (`ja-JP-u-ca-japanese` -> real Reiwa-era years, `ar-SA-u-ca-
/// islamic-umalqura` -> real Hijri dates) -- with zero per-calendar
/// dispatch code in `intl_datetime.rs`, confirmed empirically. Every
/// case cross-checked against real Node.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_dispatches_non_gregorian_calendars_matching_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, epochMs) {\n\
               return new Intl.DateTimeFormat(locale, opts).format(new Date(epochMs));\n\
             }\n\
             function all() {\n\
               const t = Date.parse('2024-07-04T00:00:00Z');\n\
               const opts = {year:'numeric',month:'long',day:'numeric',timeZone:'UTC'};\n\
               return [\n\
                 show('th-TH', opts, t),\n\
                 show('ja-JP-u-ca-japanese', opts, t),\n\
                 show('ja-JP', {...opts, calendar:'japanese'}, t),\n\
                 show('ar-SA-u-ca-islamic-umalqura', opts, t),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "4 กรกฎาคม 2567",
            "令和6年7月4日",
            "令和6年7月4日",
            "٢٨ ذو الحجة ١٤٤٥ هـ",
        ])
        .unwrap()
    );
}

/// `dateStyle`/`timeStyle` (ECMA-402) expand into the individual field
/// options with the locale's own CLDR patterns, cross-checked against
/// real Node -- previously silently ignored (a Python-style default
/// numeric date was rendered instead).
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_styles_match_real_node() {
    assert_eq!(
        load(
            "function show(opts) {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', ...opts })\n\
                 .format(new Date(Date.UTC(2024, 6, 4, 16, 30, 45)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 show({ dateStyle: 'full' }),\n\
                 show({ dateStyle: 'long' }),\n\
                 show({ dateStyle: 'medium' }),\n\
                 show({ dateStyle: 'short' }),\n\
                 show({ timeStyle: 'full' }),\n\
                 show({ timeStyle: 'long' }),\n\
                 show({ timeStyle: 'medium' }),\n\
                 show({ timeStyle: 'short' }),\n\
                 show({ dateStyle: 'full', timeStyle: 'short' }),\n\
               ];\n\
             }\n\
             function styleOption() {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', dateStyle: 'short' })\n\
                 .resolvedOptions().dateStyle;\n\
             }\n\
             function rejectsMixed() {\n\
               try {\n\
                 new Intl.DateTimeFormat('en-US', { dateStyle: 'short', year: 'numeric' });\n\
                 return 'no-throw';\n\
               } catch (error) {\n\
                 return error.constructor.name;\n\
               }\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "Thursday, July 4, 2024",
            "July 4, 2024",
            "Jul 4, 2024",
            "7/4/24",
            "4:30:45 PM Coordinated Universal Time",
            "4:30:45 PM UTC",
            "4:30:45 PM",
            "4:30 PM",
            "Thursday, July 4, 2024 at 4:30 PM",
        ])
        .unwrap()
    );
    assert_eq!(
        call("styleOption", "[]"),
        serde_json::to_string("short").unwrap()
    );
    assert_eq!(
        call("rejectsMixed", "[]"),
        serde_json::to_string("TypeError").unwrap()
    );
}

/// `fractionalSecondDigits` (real ECMA-402) -- previously ignored. The
/// fractional digits come from the millisecond/sub-second field; with
/// `second` present they join with the locale decimal separator, and
/// alone they render just the requested leading digits.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_fractional_second_digits_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })\n\
                 .format(new Date(Date.UTC(2024, 0, 1, 13, 5, 45, 678)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('en-US', { fractionalSecondDigits: 1 }),\n\
                 show('en-US', { fractionalSecondDigits: 2 }),\n\
                 show('en-US', { fractionalSecondDigits: 3 }),\n\
                 show('en-US', { hour: 'numeric', minute: '2-digit', second: 'numeric', fractionalSecondDigits: 3 }),\n\
                 show('en-US', { hour: 'numeric', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 2 }),\n\
                 show('de-DE', { hour: 'numeric', minute: '2-digit', second: 'numeric', fractionalSecondDigits: 3 }),\n\
               ];\n\
             }\n\
             function rejectsInvalid() {\n\
               try { new Intl.DateTimeFormat('en-US', { fractionalSecondDigits: 4 }); return 'no-throw'; }\n\
               catch (error) { return error.constructor.name; }\n\
             }\n\
             function option() {\n\
               return new Intl.DateTimeFormat('en-US', { fractionalSecondDigits: 2 }).resolvedOptions().fractionalSecondDigits;\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "6",
            "67",
            "678",
            "1:05:45.678 PM",
            "1:05:45.67 PM",
            "13:05:45,678"
        ])
        .unwrap()
    );
    assert_eq!(
        call("rejectsInvalid", "[]"),
        serde_json::to_string("RangeError").unwrap()
    );
    assert_eq!(call("option", "[]"), "2".to_string());
}
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_hour_precedence_and_option_validation_match_real_node() {
    assert_eq!(
        load(
            "function show(opts) {\n\
               const d = new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', hour: 'numeric', minute: '2-digit', ...opts });\n\
               const r = d.resolvedOptions();\n\
               return [r.hourCycle, r.hour12, d.format(new Date(Date.UTC(2024, 0, 1, 13, 5)))];\n\
             }\n\
             function rejectsInvalid() {\n\
               const bad = [];\n\
               for (const opts of [{ hourCycle: 'h25' }, { weekday: 'x' }, { month: 'x' }, { year: 'x' }]) {\n\
                 try { new Intl.DateTimeFormat('en-US', opts); bad.push('no-throw'); }\n\
                 catch (error) { bad.push(error.constructor.name); }\n\
               }\n\
               return bad;\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("show", "[{\"hourCycle\":\"h23\",\"hour12\":true}]"),
        serde_json::json!(["h12", true, "1:05 PM"]).to_string()
    );
    assert_eq!(
        call("show", "[{\"hourCycle\":\"h12\",\"hour12\":false}]"),
        serde_json::json!(["h23", false, "13:05"]).to_string()
    );
    assert_eq!(
        call("show", "[{\"hourCycle\":\"h11\"}]"),
        serde_json::json!(["h11", true, "1:05 PM"]).to_string()
    );
    assert_eq!(
        call("show", "[{\"hour12\":false}]"),
        serde_json::json!(["h23", false, "13:05"]).to_string()
    );
    assert_eq!(
        call("rejectsInvalid", "[]"),
        serde_json::to_string(&["RangeError", "RangeError", "RangeError", "RangeError"]).unwrap()
    );
}

/// Real multi-locale `Intl.NumberFormat` (M6 of docs/design/
/// intl-polyfill.md's "Real CLDR data via icu4x" plan) --
/// `intl_number.rs`'s `__thaw_intl_number_format`, replacing the
/// previous fixed comma-grouping/Latin-digit rendering for any curated
/// locale. Every case cross-checked against real Node: real non-Latin
/// digits (`bn`, `ar-SA`, the latter also with its own Arabic thousands
/// separator, not a comma), a narrow-no-break-space grouping separator
/// (`fr` -- a genuine CLDR value here, not the M4 NNBSP artifact), and
/// `th`'s own real CLDR default staying plain Latin digits (confirming
/// this isn't "assume every non-Latin-script locale uses non-Latin
/// digits").
#[cfg(feature = "intl")]
#[test]
fn intl_number_format_matches_real_node_for_curated_non_english_locales() {
    assert_eq!(
        load(
            "function show(locale, opts, value) {\n\
               return new Intl.NumberFormat(locale, opts).format(value);\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('de-DE', {}, 1234567.891),\n\
                 show('fr', {}, 1234567.891),\n\
                 show('bn', {}, 12345),\n\
                 show('th', {}, 12345),\n\
                 show('ar-SA', {}, 12345),\n\
                 show('de-DE', {useGrouping:false}, 1234567.891),\n\
                 show('en-US', {}, 1234567.891),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "1.234.567,891",
            "1\u{202f}234\u{202f}567,891",
            "১২,৩৪৫",
            "12,345",
            "١٢٬٣٤٥",
            "1234567,891",
            "1,234,567.891",
        ])
        .unwrap()
    );
}

/// `Intl.NumberFormat`'s `notation` (`scientific`/`engineering`),
/// `numberingSystem` (explicit override), and `currencySign:
/// 'accounting'` -- all cross-checked against real Node. Previously
/// `notation`/`numberingSystem` were silently ignored and negative
/// currency always used a minus sign.
#[cfg(feature = "intl")]
#[test]
fn intl_number_notation_numbering_system_and_accounting_match_real_node() {
    assert_eq!(
        load(
            "function f(locale, options, value) {\n\
               return new Intl.NumberFormat(locale, { useGrouping: false, ...options }).format(value);\n\
             }\n\
             function notation() {\n\
               return [\n\
                 f('en', { notation: 'scientific' }, 1234),\n\
                 f('en', { notation: 'scientific', maximumFractionDigits: 2 }, 1234),\n\
                 f('en', { notation: 'scientific' }, 0),\n\
                 f('en', { notation: 'engineering' }, 12345),\n\
                 f('en', { notation: 'engineering' }, 0.0001234),\n\
                 f('en', { notation: 'scientific' }, -1234),\n\
                 f('de', { notation: 'scientific' }, 1234),\n\
               ];\n\
             }\n\
             function numberingSystem() {\n\
               const g = (nu, n) => new Intl.NumberFormat('en', { numberingSystem: nu }).format(n);\n\
               return [\n\
                 g('arab', 1234), g('arab', 1234567),\n\
                 g('arabext', 1234567.5),\n\
                 g('beng', 1234567), g('deva', 1234567), g('thai', 1234567), g('hanidec', 1234567),\n\
                 new Intl.NumberFormat('en', { numberingSystem: 'arab' }).resolvedOptions().numberingSystem,\n\
               ];\n\
             }\n\
             function accounting() {\n\
               const nf = new Intl.NumberFormat('en', { style: 'currency', currency: 'USD', currencySign: 'accounting' });\n\
               return [nf.format(-5), nf.format(5), nf.format(-0)];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("notation", "[]"),
        serde_json::to_string(&[
            "1.234E3", "1.23E3", "0E0", "12.345E3", "123.4E-6", "-1.234E3", "1,234E3",
        ])
        .unwrap()
    );
    assert_eq!(
        call("numberingSystem", "[]"),
        serde_json::to_string(&[
            "١٬٢٣٤",
            "١٬٢٣٤٬٥٦٧",
            "۱٬۲۳۴٬۵۶۷٫۵",
            "১,২৩৪,৫৬৭",
            "१,२३४,५६७",
            "๑,๒๓๔,๕๖๗",
            "一,二三四,五六七",
            "arab",
        ])
        .unwrap()
    );
    assert_eq!(
        call("accounting", "[]"),
        serde_json::to_string(&["($5.00)", "$5.00", "($0.00)"]).unwrap()
    );
}

#[cfg(feature = "intl")]
#[test]
fn intl_number_currency_and_percent_match_real_node() {
    assert_eq!(
        load(
            "function all() {\n\
               const f = (locale, options, value) => new Intl.NumberFormat(locale, options).format(value);\n\
               return [\n\
                 f('en-US', { style: 'percent' }, 0.56),\n\
                 f('tr-TR', { style: 'percent' }, 0.56),\n\
                 f('en-US', { style: 'currency', currency: 'USD' }, 1234.5),\n\
                 f('de-DE', { style: 'currency', currency: 'EUR' }, 1234.5),\n\
                 f('fr-FR', { style: 'currency', currency: 'USD', currencyDisplay: 'code' }, 1234.5),\n\
                 f('ja-JP', { style: 'currency', currency: 'JPY' }, 1234.5),\n\
                 f('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name' }, 1234.5),\n\
                 f('en-US', { style: 'currency', currency: 'KRW' }, 1234.5),\n\
                 f('en-US', { style: 'currency', currency: 'BHD' }, 1234.5)\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "56%",
            "%56",
            "$1,234.50",
            "1.234,50 €",
            "1 234,50 USD",
            "￥1,235",
            "1,234.50 US dollars",
            "\u{20a9}1,235",
            "BHD\u{a0}1,234.500",
        ])
        .unwrap()
    );
}

#[cfg(feature = "intl")]
#[test]
fn intl_number_units_match_real_node() {
    assert_eq!(
        load(
            "function all() {\n\
               const f = (locale, unit, unitDisplay, value) => new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay }).format(value);\n\
               return [f('de-DE', 'meter', 'long', 2), f('fr-FR', 'liter', 'long', 1), f('ja-JP', 'kilogram', 'short', 3)];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&["2 Meter", "1 litre", "3 kg"]).unwrap()
    );
}

/// Real multi-locale `Intl.ListFormat` (M7 of docs/design/
/// intl-polyfill.md's "Real CLDR data via icu4x" plan) --
/// `intl_list.rs`'s `__thaw_intl_list_format`, replacing the previous
/// fixed English-only conjunction/disjunction joining for any curated
/// locale. Every case cross-checked against real Node, including
/// Japanese's real `、` separator (not a comma) and its distinct
/// "or"-list connector.
#[cfg(feature = "intl")]
#[test]
fn intl_list_format_matches_real_node_for_curated_non_english_locales() {
    assert_eq!(
        load(
            "function show(locale, opts, items) {\n\
               return new Intl.ListFormat(locale, opts).format(items);\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('ja', {}, ['a', 'b', 'c']),\n\
                 show('ja', { type: 'disjunction' }, ['a', 'b', 'c']),\n\
                 show('fr', {}, ['a', 'b', 'c']),\n\
                 show('de', { style: 'short' }, ['a', 'b', 'c']),\n\
                 show('en-US', {}, ['a', 'b', 'c']),\n\
                 show('en-US', { type: 'disjunction' }, ['a', 'b', 'c']),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "a、b、c",
            "a、b、またはc",
            "a, b et c",
            "a, b und c",
            "a, b, and c",
            "a, b, or c",
        ])
        .unwrap()
    );
}

/// `Intl.PluralRules` (M8, entirely new capability) -- cardinal and
/// ordinal category selection cross-checked against real Node,
/// including Arabic's genuinely 6-category cardinal system (the widest
/// real CLDR plural system) and Japanese's complete absence of any
/// cardinal distinction (always `"other"`).
#[cfg(feature = "intl")]
#[test]
fn intl_plural_rules_matches_real_node() {
    assert_eq!(
        load(
            "function cardinal(locale, n) {\n\
               return new Intl.PluralRules(locale).select(n);\n\
             }\n\
             function ordinal(locale, n) {\n\
               return new Intl.PluralRules(locale, { type: 'ordinal' }).select(n);\n\
             }\n\
             function categories(locale) {\n\
               return new Intl.PluralRules(locale).resolvedOptions().pluralCategories;\n\
             }\n\
             function byOptions(locale, opts, n) {\n\
               return new Intl.PluralRules(locale, opts).select(n);\n\
             }\n\
             function all() {\n\
               return [\n\
                 cardinal('en', 1),\n\
                 cardinal('en', 2),\n\
                 cardinal('en', 1.0),\n\
                 cardinal('en', 1.5),\n\
                 cardinal('ar', 0),\n\
                 cardinal('ar', 1),\n\
                 cardinal('ar', 2),\n\
                 cardinal('ar', 3),\n\
                 cardinal('ar', 11),\n\
                 cardinal('ar', 100),\n\
                 cardinal('ja', 1),\n\
                 ordinal('en', 1),\n\
                 ordinal('en', 2),\n\
                 ordinal('en', 3),\n\
                 ordinal('en', 4),\n\
                 ordinal('en', 11),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "one", "other", "one", "other", "zero", "one", "two", "few", "many", "other", "other",
            "one", "two", "few", "other", "other",
        ])
        .unwrap()
    );
    // `resolvedOptions().pluralCategories` (V8/Node ordering, absent
    // categories skipped) -- previously hardcoded to `["other"]`.
    assert_eq!(call("categories", "[\"en\"]"), r#"["one","other"]"#);
    assert_eq!(
        call("categories", "[\"ru\"]"),
        r#"["few","many","one","other"]"#
    );
    assert_eq!(
        call("categories", "[\"ar\"]"),
        r#"["few","many","one","two","zero","other"]"#
    );
    assert_eq!(call("categories", "[\"ja\"]"), r#"["other"]"#);
    // Digit options are honored by formatting the operand the way
    // `Intl.NumberFormat` would (real Node values).
    assert_eq!(
        call("byOptions", "[\"en\", {}, 1]"),
        serde_json::to_string("one").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"en\", {\"minimumFractionDigits\":1}, 1]"),
        serde_json::to_string("other").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"en\", {\"maximumFractionDigits\":0}, 1.4]"),
        serde_json::to_string("one").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"en\", {\"maximumFractionDigits\":0}, 1.5]"),
        serde_json::to_string("other").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"en\", {\"maximumFractionDigits\":0}, 2]"),
        serde_json::to_string("other").unwrap()
    );
    assert_eq!(
        call(
            "byOptions",
            "[\"en\", {\"maximumSignificantDigits\":1}, 1.4]"
        ),
        serde_json::to_string("one").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"en\", {\"minimumSignificantDigits\":3}, 2]"),
        serde_json::to_string("other").unwrap()
    );
    assert_eq!(
        call("byOptions", "[\"ro\", {\"minimumFractionDigits\":1}, 1]"),
        serde_json::to_string("few").unwrap()
    );
}

/// `Intl.Collator` (M9, entirely new capability) -- real collation
/// order and `sensitivity`/`numeric`/`ignorePunctuation` options, cross-
/// checked against real Node: Swedish's å sorting after z (its real
/// alphabet position), `sensitivity: 'base'` making case/accent
/// differences compare equal, `sensitivity: 'case'` keeping case
/// significant while accents stay insensitive, `numeric: true` making
/// "img2" < "img10" numerically rather than lexicographically, and
/// `ignorePunctuation` making "a,b" and "ab" compare equal.
#[cfg(feature = "intl")]
#[test]
fn intl_collator_matches_real_node() {
    assert_eq!(
        load(
            "function sign(n) { return n < 0 ? -1 : n > 0 ? 1 : 0; }\n\
             function cmp(locale, opts, a, b) {\n\
               return sign(new Intl.Collator(locale, opts).compare(a, b));\n\
             }\n\
             function all() {\n\
               return [\n\
                 cmp('de', {}, 'a', 'ä'),\n\
                 cmp('de', {}, 'ä', 'z'),\n\
                 cmp('sv', {}, 'a', 'å'),\n\
                 cmp('sv', {}, 'å', 'z'),\n\
                 cmp('en', { sensitivity: 'base' }, 'a', 'A'),\n\
                 cmp('en', { sensitivity: 'base' }, 'e', 'é'),\n\
                 cmp('en', { sensitivity: 'case' }, 'a', 'A'),\n\
                 cmp('en', { numeric: true }, 'img2', 'img10'),\n\
                 cmp('en', {}, 'img2', 'img10'),\n\
                 cmp('en', { ignorePunctuation: true }, 'a,b', 'ab'),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[-1, -1, -1, 1, 0, 0, -1, -1, 1, 0]).unwrap()
    );
}

/// `Intl.Segmenter` (M10, entirely new capability) -- real word
/// segmentation cross-checked against real Node, including Thai's real
/// dictionary-based word breaking (no inter-word spaces in the source
/// text at all -- proves the locale/script-aware segmenter is really
/// wired, not a naive whitespace split), an English word case (with
/// `isWordLike` distinguishing words from whitespace/punctuation),
/// grapheme-cluster segmentation across an emoji (a single segment, not
/// split across its surrogate pair), and sentence segmentation.
#[cfg(feature = "intl")]
#[test]
fn intl_segmenter_matches_real_node() {
    assert_eq!(
        load(
            "function segs(locale, granularity, text) {\n\
               const seg = new Intl.Segmenter(locale, { granularity });\n\
               return Array.from(seg.segment(text), s => ({\n\
                 segment: s.segment, index: s.index,\n\
                 isWordLike: s.isWordLike === undefined ? null : s.isWordLike,\n\
               }));\n\
             }\n\
             function thaiWord() { return segs('th', 'word', 'สวัสดีชาวโลก'); }\n\
             function enWord() { return segs('en', 'word', 'Hello world!'); }\n\
             function enGrapheme() { return segs('en', 'grapheme', 'a👍bc'); }\n\
             function enSentence() { return segs('en', 'sentence', 'Hi. Bye!'); }\n\
             function containing(index) {\n\
               const seg = new Intl.Segmenter('en', { granularity: 'word' });\n\
               const s = seg.segment('Hello world').containing(index);\n\
               return s === undefined ? null : {\n\
                 segment: s.segment, index: s.index,\n\
                 isWordLike: s.isWordLike === undefined ? null : s.isWordLike,\n\
               };\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("thaiWord", "[]"),
        r#"[{"segment":"สวัสดี","index":0,"isWordLike":true},{"segment":"ชาว","index":6,"isWordLike":true},{"segment":"โลก","index":9,"isWordLike":true}]"#
    );
    assert_eq!(
        call("enWord", "[]"),
        r#"[{"segment":"Hello","index":0,"isWordLike":true},{"segment":" ","index":5,"isWordLike":false},{"segment":"world","index":6,"isWordLike":true},{"segment":"!","index":11,"isWordLike":false}]"#
    );
    assert_eq!(
        call("enGrapheme", "[]"),
        r#"[{"segment":"a","index":0,"isWordLike":null},{"segment":"👍","index":1,"isWordLike":null},{"segment":"b","index":3,"isWordLike":null},{"segment":"c","index":4,"isWordLike":null}]"#
    );
    assert_eq!(
        call("enSentence", "[]"),
        r#"[{"segment":"Hi. ","index":0,"isWordLike":null},{"segment":"Bye!","index":4,"isWordLike":null}]"#
    );
    // `Segments.prototype.containing(index)` -- previously a documented
    // gap (only the iterable protocol was implemented).
    assert_eq!(
        call("containing", "[7]"),
        r#"{"segment":"world","index":6,"isWordLike":true}"#
    );
    assert_eq!(
        call("containing", "[0]"),
        r#"{"segment":"Hello","index":0,"isWordLike":true}"#
    );
    assert_eq!(call("containing", "[99]"), "null");
    assert_eq!(call("containing", "[-1]"), "null");
}

/// RelativeTimeFormat shares the requested locale's digit preference with
/// its DecimalFormatter and reports the actually supported system.
#[cfg(feature = "intl")]
#[test]
fn intl_relative_time_numbering_system_extension_and_option() {
    assert_eq!(
        load(r#"function intlRelativeNumbering() {
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
        }"#),
        1
    );
    assert_eq!(
        call("intlRelativeNumbering", "[]"),
        "[true,true,true,true,true,true]"
    );
}

#[cfg(feature = "intl")]
#[test]
fn intl_relative_time_rejects_inherited_unit_names() {
    // Unrun regression: invalid unit names must not resolve Object.prototype.
    assert_eq!(load(r#"
        function inheritedRelativeUnits() {
            const formatter = new Intl.RelativeTimeFormat('en');
            return ['toString', 'constructor', '__proto__', 'hasOwnProperty'].map(unit => {
                try { formatter.format(1, unit); return 'no-throw'; }
                catch (error) { return error.name; }
            });
        }
    "#), 1);
    assert_eq!(call("inheritedRelativeUnits", "[]"),
        r#"["RangeError","RangeError","RangeError","RangeError"]"#);
}

/// `Intl.RelativeTimeFormat` (M11, entirely new capability, backed by
/// the one deliberately-unstable icu4x dependency in this whole effort)
/// -- cross-checked against real Node, including `numeric: 'auto'`
/// producing real special-cased words ("yesterday"/"昨日", not
/// a numeric phrase) and Japanese's own real phrasing.
#[cfg(feature = "intl")]
#[test]
fn intl_relative_time_format_matches_real_node() {
    assert_eq!(
        load(
            "function fmt(locale, opts, value, unit) {\n\
               return new Intl.RelativeTimeFormat(locale, opts).format(value, unit);\n\
             }\n\
             function all() {\n\
               return [\n\
                 fmt('en', {}, -1, 'day'),\n\
                 fmt('en', {}, 3, 'days'),\n\
                 fmt('en', { numeric: 'auto' }, -1, 'day'),\n\
                 fmt('en', { style: 'short' }, -3, 'hour'),\n\
                 fmt('ja', {}, -1, 'day'),\n\
                 fmt('ja', { numeric: 'auto' }, -1, 'day'),\n\
                 fmt('de', {}, 2, 'year'),\n\
                 fmt('en', {}, 1.5, 'day'),\n\
                 new Intl.RelativeTimeFormat('en').resolvedOptions().numberingSystem,\n\
                 (() => { try { fmt('en', {}, Infinity, 'day'); return 'no-throw'; } catch (e) { return e.name; } })(),\n\
                 (() => { try { new Intl.RelativeTimeFormat('en', { style: 'bad' }); return 'no-throw'; } catch (e) { return e.name; } })(),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "1 day ago",
            "in 3 days",
            "yesterday",
            "3 hr. ago",
            "1 日前",
            "昨日",
            "in 2 Jahren",
            "in 1.5 days",
            "latn",
            "RangeError",
            "RangeError",
        ])
        .unwrap()
    );
}

/// `node:dns`'s `lookup`/`resolve`/`resolve4`/`resolve6` used to only
/// recognize an IP literal or `"localhost"`, returning `ENOTFOUND` for
/// every real hostname -- unlike `net.connect`/`http.request`, which
/// already resolve real hostnames correctly via `TcpStream::connect`'s
/// own independent OS-level resolution. `dns_lookup_json` closes that
/// gap via `std::net::ToSocketAddrs` (stdlib, the same OS resolver).
/// Resolving `"localhost"` here exercises the real mechanism (the OS
/// resolver, not a hardcoded shortcut -- that shortcut lives one layer
/// up, in the JS `addresses()` helper, and deliberately isn't hit by
/// calling this Rust function directly) while staying fully hermetic:
/// no external network access is needed to resolve the loopback name.
#[test]
fn dns_lookup_json_resolves_localhost_via_the_real_os_resolver() {
    let parsed: serde_json::Value =
        serde_json::from_str(&dns_lookup_json("localhost".into())).unwrap();
    assert_eq!(parsed["ok"], true);
    let addresses = parsed["addresses"].as_array().unwrap();
    assert!(
        addresses
            .iter()
            .any(|entry| entry["address"] == "127.0.0.1" && entry["family"] == 4),
        "{addresses:?}"
    );
    assert!(
        addresses
            .iter()
            .any(|entry| entry["address"] == "::1" && entry["family"] == 6),
        "{addresses:?}"
    );

    // An empty hostname fails resolution locally (`getaddrinfo` rejects
    // the format outright, no DNS query dispatched) -- keeps this test
    // hermetic, unlike a real unresolvable hostname would be.
    let failure: serde_json::Value = serde_json::from_str(&dns_lookup_json(String::new())).unwrap();
    assert_eq!(failure["ok"], false);
}

/// `process.nextTick` used to be a plain alias for `queueMicrotask`
/// (`Promise.resolve().then(callback)`), giving it no priority over an
/// already-scheduled Promise `.then()` -- real Node fully drains its
/// own separate nextTick queue before running any pending Promise
/// microtask, at every checkpoint, so nextTick always wins regardless
/// of scheduling order. Fixed by queuing into a plain JS array instead
/// (`platform_globals/runtime.js`), drained by the Rust host
/// (`drain_next_tick_queue`, thaw-quickjs/src/lib.rs) immediately
/// before every single `execute_pending_job()` check across this
/// crate -- not just once per batch, so a nextTick queued from inside
/// one microtask still runs before the *next* one. Scoped to this
/// crate's own QuickJS engine (this test drives it directly, the same
/// way `thaw_js_run_event_loop`-based tests above do); natively
/// JIT-compiled code has its own separate Promise/timer implementation
/// (thaw-runtime) this fix does not reach -- a natively-compiled
/// program's own `Promise`/`setTimeout` still order correctly against
/// each other (a real, working, separate microtask queue), just not
/// against a `process.nextTick` call, which still bridges into this
/// crate's engine. Left as a known, separately-scoped gap: giving
/// natively-compiled code the same fix needs `process.nextTick` as its
/// own native compiler intrinsic (parallel to `setTimeout`), not a
/// small patch to this file.
#[test]
fn process_next_tick_runs_before_promise_microtasks_scheduled_earlier() {
    assert_eq!(
        load(
            "globalThis.order = []; \
             order.push('sync'); \
             Promise.resolve().then(() => order.push('promise')); \
             process.nextTick(() => order.push('nextTick')); \
             setTimeout(() => order.push('timeout0'), 0); \
             setImmediate(() => order.push('immediate')); \
             function readOrder() { return order; }"
        ),
        1
    );
    assert_eq!(thaw_js_run_event_loop(), 0);
    assert_eq!(
        call("readOrder", "[]"),
        r#"["sync","nextTick","promise","timeout0","immediate"]"#
    );
}

/// Real ECMA-402 constructors reject an out-of-enum option with a
/// `RangeError` at construction time; previously `Collator`/`Segmenter`/
/// `PluralRules`/`ListFormat` silently substituted the default. Also
/// checks `Collator`'s resolved `usage`/`caseFirst`.
#[cfg(feature = "intl")]
#[test]
fn intl_option_validation_matches_real_node() {
    assert_eq!(
        load(
            "function ctorErrors() {\n\
               const cases = [\n\
                 () => new Intl.Collator('en', { sensitivity: 'x' }),\n\
                 () => new Intl.Collator('en', { usage: 'x' }),\n\
                 () => new Intl.Collator('en', { caseFirst: 'x' }),\n\
                 () => new Intl.Segmenter('en', { granularity: 'x' }),\n\
                 () => new Intl.PluralRules('en', { type: 'x' }),\n\
                 () => new Intl.NumberFormat('en', { minimumIntegerDigits: 0 }),\n\
                 () => new Intl.NumberFormat('en', { minimumIntegerDigits: 22 }),\n\
                 () => new Intl.ListFormat('en', { type: 'x' }),\n\
                 () => new Intl.ListFormat('en', { style: 'x' }),\n\
               ];\n\
               return cases.map(fn => { try { fn(); return 'no-throw'; } catch (error) { return error.constructor.name; } });\n\
             }\n\
             function resolved() {\n\
               return [\n\
                 new Intl.Collator('en', { usage: 'search' }).resolvedOptions().usage,\n\
                 new Intl.Collator('en', { caseFirst: false }).resolvedOptions().caseFirst,\n\
                 new Intl.Segmenter('en', { granularity: 'word' }).resolvedOptions().granularity,\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("ctorErrors", "[]"),
        serde_json::to_string(&["RangeError"; 9]).unwrap()
    );
    assert_eq!(
        call("resolved", "[]"),
        serde_json::to_string(&["search", "false", "word"]).unwrap()
    );
}

/// `roundingMode` (all nine ECMA-402 modes) for `Intl.NumberFormat` and
/// `Intl.PluralRules`, previously hardcoded to `halfExpand`.
#[cfg(feature = "intl")]
#[test]
fn intl_number_rounding_mode_matches_real_node() {
    assert_eq!(
        load(
            "function nm(mode) {\n\
               const f = n => new Intl.NumberFormat('en', { useGrouping: false, maximumFractionDigits: 0, roundingMode: mode }).format(n);\n\
               return [f(1.5), f(-1.5), f(2.5)];\n\
             }\n\
             function all() {\n\
               return ['ceil', 'floor', 'expand', 'trunc', 'halfCeil', 'halfFloor', 'halfTrunc', 'halfEven', 'halfExpand'].map(nm);\n\
             }\n\
             function plural(mode, n) {\n\
               return new Intl.PluralRules('en', { maximumFractionDigits: 0, roundingMode: mode }).select(n);\n\
             }\n\
             function pluralModes() {\n\
               return [plural('floor', 1.9), plural('ceil', 1.9), plural('halfEven', 2.5)];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::json!([
            ["2", "-1", "3"],
            ["1", "-2", "2"],
            ["2", "-2", "3"],
            ["1", "-1", "2"],
            ["2", "-1", "3"],
            ["1", "-2", "2"],
            ["1", "-1", "2"],
            ["2", "-2", "2"],
            ["2", "-2", "3"],
        ])
        .to_string()
    );
    assert_eq!(
        call("pluralModes", "[]"),
        serde_json::to_string(&["one", "other", "other"]).unwrap()
    );
}

/// `Intl.DateTimeFormat` time fields are independent: a `minute`/`second`
/// request without `hour` renders only the requested fields (real
/// ECMA-402: `{second:'numeric'}` -> `"45"`), whereas the ICU backend's
/// `TimePrecision` would otherwise render a full `h:mm:ss`. Cross-checked
/// against real Node.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_time_field_independence_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })\n\
                 .format(new Date(Date.UTC(2024, 0, 1, 13, 5, 45, 678)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('en-US', { second: 'numeric' }),\n\
                 show('en-US', { minute: 'numeric' }),\n\
                 show('en-US', { minute: 'numeric', second: 'numeric' }),\n\
                 show('en-US', { minute: '2-digit' }),\n\
                 show('en-US', { second: 'numeric', fractionalSecondDigits: 3 }),\n\
                 show('en-US', { minute: '2-digit', second: '2-digit', fractionalSecondDigits: 2 }),\n\
                 show('en-US', { second: 'numeric', timeZoneName: 'short' }),\n\
                 show('de-DE', { minute: 'numeric', second: 'numeric' }),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&["45", "5", "05:45", "5", "45.678", "05:45.67", "45 UTC", "05:45",])
            .unwrap()
    );
}

/// `Intl.NumberFormat notation: 'compact'` via icu4x's
/// `CompactDecimalFormatter` -- previously unsupported (fell back to
/// standard formatting). Every non-tie value is cross-checked against
/// real Node; the one deliberate, documented divergence is an exact
/// half-way mantissa (`1650`), where icu4x rounds half-to-even
/// (`"1.6K"`) while ECMA-402's default is half-to-even's opposite,
/// `halfExpand` (`Node: "1.7K"`).
#[cfg(feature = "intl")]
#[test]
fn intl_number_compact_notation_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, n) {\n\
               return new Intl.NumberFormat(locale, { notation: 'compact', ...opts }).format(n);\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('en', {}, 999), show('en', {}, 1000), show('en', {}, 1234),\n\
                 show('en', {}, 1750), show('en', {}, 1950), show('en', {}, 15127),\n\
                 show('en', {}, 123456), show('en', {}, 3010349), show('en', {}, 999500),\n\
                 show('en', {}, -13132), show('en', {}, 0.2222),\n\
                 show('de', {}, 15127), show('de', {}, 3010349), show('de', {}, 1234567),\n\
                 show('ja', {}, 15127), show('ja', {}, 123456), show('ja', {}, 1234567),\n\
                 show('en', { compactDisplay: 'long' }, 1000),\n\
                 show('en', { compactDisplay: 'long' }, 1234),\n\
               ];\n\
             }\n\
             function tie() { return show('en', {}, 1650); }\n\
             function options() {\n\
               const r = new Intl.NumberFormat('en', { notation: 'compact', compactDisplay: 'long' }).resolvedOptions();\n\
               return [r.notation, r.compactDisplay];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "999",
            "1K",
            "1.2K",
            "1.8K",
            "2K",
            "15K",
            "123K",
            "3M",
            "1M",
            "-13K",
            "0.22",
            "15.127",
            "3\u{a0}Mio.",
            "1,2\u{a0}Mio.",
            "1.5万",
            "12万",
            "123万",
            "1 thousand",
            "1.2 thousand",
        ])
        .unwrap()
    );
    // Known, documented half-even tie divergence (see this test's own doc
    // comment): pinned deliberately so a future data/rounding change is a
    // visible, intentional update rather than a silent drift.
    assert_eq!(call("tie", "[]"), serde_json::to_string("1.6K").unwrap());
    assert_eq!(
        call("options", "[]"),
        serde_json::to_string(&["compact", "long"]).unwrap()
    );
}

/// A bare `{month}`/`{year}`/`{year, month}` request -- ICU4X classifies
/// these as *calendar-period* field sets (`DateFields::M`/`YM`/`Y`),
/// which `build_date()` rejects, so they previously rendered an empty
/// string. Now routed through `build_calendar_period()`. Cross-checked
/// against real Node.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_calendar_period_fields_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })\n\
                 .format(new Date(Date.UTC(2024, 6, 4)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('en-US', { month: 'numeric' }),\n\
                 show('en-US', { month: 'short' }),\n\
                 show('en-US', { month: 'long' }),\n\
                 show('en-US', { year: 'numeric' }),\n\
                 show('en-US', { year: 'numeric', month: 'long' }),\n\
                 show('en-US', { year: 'numeric', month: 'short' }),\n\
                 show('de-DE', { month: 'long' }),\n\
                 show('ja-JP', { month: 'long' }),\n\
                 show('fr', { month: 'long' }),\n\
                 show('ja-JP', { year: 'numeric', month: 'numeric' }),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "7",
            "Jul",
            "July",
            "2024",
            "July 2024",
            "Jul 2024",
            "Juli",
            "7月",
            "juillet",
            "2024/7",
        ])
        .unwrap()
    );
}

/// `Intl.Collator` with `usage: 'search'` via the opt-in ICU4C backend
/// (feature `icu4c`, default off) -- ICU4X has no search-collation
/// concept. Cross-checked against real Node.
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_collator_search_usage_matches_real_node() {
    assert_eq!(
        load(
            "function cmp(locale, opts, a, b) {\n\
               const sign = n => n < 0 ? -1 : n > 0 ? 1 : 0;\n\
               return sign(new Intl.Collator(locale, { usage: 'search', ...opts }).compare(a, b));\n\
             }\n\
             function all() {\n\
               return [\n\
                 cmp('en', {}, 'a', 'A'), cmp('en', {}, 'a', 'á'), cmp('en', {}, 'æ', 'ae'),\n\
                 cmp('en', {}, 'ß', 'ss'), cmp('en', {}, 'e', 'é'),\n\
                 cmp('de', {}, 'a', 'ä'), cmp('de', {}, 'ss', 'ß'),\n\
                 cmp('en', { sensitivity: 'base' }, 'a', 'á'),\n\
                 cmp('en', {}, '\u{0627}', '\u{0623}'), cmp('ja', {}, '\u{3042}', '\u{30a2}'),\n\
               ];\n\
             }\n\
             function usageOption() {\n\
               return new Intl.Collator('en', { usage: 'search' }).resolvedOptions().usage;\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[-1, -1, 1, 1, -1, -1, -1, 0, -1, -1]).unwrap()
    );
    assert_eq!(
        call("usageOption", "[]"),
        serde_json::to_string("search").unwrap()
    );
}

/// `Intl.PluralRules.prototype.selectRange` via the opt-in ICU4C backend.
/// ICU4X's vendored plural-range data diverges from ICU4C/Node (e.g.
/// `en.selectRange(1,1)`), so this uses ICU4C's own
/// `uplrules_selectForRange`; uncurated locales (here `sl`) work too.
/// Cross-checked against real Node.
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_plural_rules_select_range_icu4c_matches_real_node() {
    assert_eq!(
        load(
            "function range(locale, start, end) {\n\
               return new Intl.PluralRules(locale).selectRange(start, end);\n\
             }\n\
             function all() {\n\
               return [\n\
                 range('en', 1, 1), range('en', 1, 2), range('en', 2, 2),\n\
                 range('ro', 1, 1), range('ro', 1, 2), range('ro', 2, 3), range('ro', 0, 1),\n\
                 range('ar', 0, 1), range('ar', 1, 2), range('ar', 2, 3),\n\
                 range('sl', 1, 1), range('sl', 1, 2), range('sl', 2, 2), range('sl', 0, 1),\n\
               ];\n\
             }\n\
             function withOptions() {\n\
               return new Intl.PluralRules('en', { maximumFractionDigits: 0 }).selectRange(1.5, 2.5);\n\
             }\n\
             function rejectsNaN() {\n\
               try { new Intl.PluralRules('en').selectRange(NaN, 2); return 'no-throw'; }\n\
               catch (error) { return error.constructor.name; }\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "other", "other", "other", "other", "few", "few", "few", "zero", "other", "few", "few",
            "two", "two", "few",
        ])
        .unwrap()
    );
    assert_eq!(
        call("withOptions", "[]"),
        serde_json::to_string("other").unwrap()
    );
    assert_eq!(
        call("rejectsNaN", "[]"),
        serde_json::to_string("RangeError").unwrap()
    );
}

/// `Intl.DateTimeFormat month:'narrow'` / `weekday:'narrow'` via the
/// opt-in ICU4C backend -- ICU4X's field-set builder has no Narrow
/// `Length` and picks the wrong pattern (`ja` narrow is `7月`, not
/// `7/04`). Cross-checked against real Node.
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_datetime_narrow_icu4c_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })\n\
                 .format(new Date(Date.UTC(2024, 6, 4)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 show('en-US', { month: 'narrow' }),\n\
                 show('en-US', { month: 'narrow', day: 'numeric' }),\n\
                 show('en-US', { month: 'narrow', year: 'numeric' }),\n\
                 show('en-US', { weekday: 'narrow' }),\n\
                 show('en-US', { weekday: 'narrow', day: 'numeric' }),\n\
                 show('en-US', { weekday: 'narrow', month: 'narrow', day: 'numeric' }),\n\
                 show('de-DE', { month: 'narrow', day: 'numeric' }),\n\
                 show('de-DE', { weekday: 'narrow', day: 'numeric' }),\n\
                 show('ja-JP', { month: 'narrow' }),\n\
                 show('ja-JP', { month: 'narrow', day: 'numeric' }),\n\
                 show('ja-JP', { weekday: 'narrow' }),\n\
                 show('fr', { month: 'narrow', day: 'numeric' }),\n\
                 show('fr', { weekday: 'narrow', day: 'numeric' }),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "J", "J 4", "J 2024", "T", "4 T", "T, J 4", "4. J", "D, 4.", "7月", "7月4日", "木",
            "4 J", "J 4",
        ])
        .unwrap()
    );
}

/// `Intl.DateTimeFormat.prototype.formatRange` via the opt-in ICU4C
/// interval formatter. Cross-checked against real Node (thin-space/en-dash
/// separators, `de`'s tight dash, `en`'s U+202F before PM, and the
/// identical-endpoints single-value form).
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_datetime_format_range_icu4c_matches_real_node() {
    assert_eq!(
        load(
            "function range(locale, opts, start, end) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts })\n\
                 .formatRange(new Date(Date.UTC(...start)), new Date(Date.UTC(...end)));\n\
             }\n\
             function all() {\n\
               return [\n\
                 range('en-US', {}, [2024, 6, 4], [2024, 6, 5]),\n\
                 range('en-US', { year: 'numeric', month: 'short', day: 'numeric' }, [2024, 6, 4], [2024, 6, 5]),\n\
                 range('en-US', { year: 'numeric', month: 'short', day: 'numeric' }, [2024, 6, 4], [2024, 6, 4]),\n\
                 range('de-DE', { year: 'numeric', month: 'short', day: 'numeric' }, [2024, 6, 4], [2024, 6, 5]),\n\
                 range('en-US', { hour: 'numeric', minute: '2-digit' }, [2024, 6, 4, 13, 0], [2024, 6, 4, 15, 30]),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "7/4/2024\u{2009}\u{2013}\u{2009}7/5/2024",
            "Jul 4\u{2009}\u{2013}\u{2009}5, 2024",
            "Jul 4, 2024",
            "4.\u{2013}5. Juli 2024",
            "1:00\u{2009}\u{2013}\u{2009}3:30\u{202f}PM",
        ])
        .unwrap()
    );
}

/// `Intl.NumberFormat.prototype.formatToParts` (ECMA-402) -- previously
/// absent. Cross-checked against real Node for decimal, negative,
/// percent, currency (incl. `accounting`), unit, and a non-Latin locale.
#[cfg(feature = "intl")]
#[test]
fn intl_number_format_to_parts_matches_real_node() {
    assert_eq!(
        load(
            "function parts(locale, opts, value) {\n\
               return new Intl.NumberFormat(locale, opts).formatToParts(value);\n\
             }\n\
             function all() {\n\
               return [\n\
                 parts('en-US', {}, 1234.5),\n\
                 parts('en-US', {}, -1234.5),\n\
                 parts('en-US', { style: 'percent' }, 0.56),\n\
                 parts('en-US', { style: 'currency', currency: 'USD' }, 1234.5),\n\
                 parts('de-DE', { style: 'currency', currency: 'EUR' }, -1234.5),\n\
                 parts('en-US', { style: 'unit', unit: 'day' }, 3),\n\
                 parts('en-US', { style: 'currency', currency: 'USD', currencySign: 'accounting' }, -5),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        r#"[[{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"}],[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"}],[{"type":"integer","value":"56"},{"type":"percentSign","value":"%"}],[{"type":"currency","value":"$"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"50"}],[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"group","value":"."},{"type":"integer","value":"234"},{"type":"decimal","value":","},{"type":"fraction","value":"50"},{"type":"literal","value":" "},{"type":"currency","value":"€"}],[{"type":"integer","value":"3"},{"type":"literal","value":" "},{"type":"unit","value":"days"}],[{"type":"literal","value":"("},{"type":"currency","value":"$"},{"type":"integer","value":"5"},{"type":"decimal","value":"."},{"type":"fraction","value":"00"},{"type":"literal","value":")"}]]"#
    );
}

/// `Intl.NumberFormat.prototype.formatRange` via the opt-in ICU4C number
/// range formatter. Cross-checked against real Node (locale range
/// separators, per-style spacing, `~n` for equal endpoints).
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_number_format_range_icu4c_matches_real_node() {
    assert_eq!(
        load(
            "function range(locale, opts, start, end) {\n\
               return new Intl.NumberFormat(locale, opts).formatRange(start, end);\n\
             }\n\
             function all() {\n\
               return [\n\
                 range('en-US', {}, 3, 5),\n\
                 range('en-US', {}, 3, 3),\n\
                 range('en-US', { maximumFractionDigits: 2 }, 3.14159, 5.2),\n\
                 range('en-US', { style: 'currency', currency: 'USD' }, 3, 5),\n\
                 range('de-DE', { style: 'currency', currency: 'EUR' }, 3, 5),\n\
                 range('en-US', { style: 'percent' }, 0.03, 0.05),\n\
                 range('en-US', { style: 'unit', unit: 'day' }, 3, 5),\n\
                 range('en-US', { notation: 'compact' }, 3000, 5000),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "3\u{2013}5",
            "~3",
            "3.14\u{2013}5.2",
            "$3.00 \u{2013} $5.00",
            "3,00\u{2013}5,00\u{00a0}\u{20ac}",
            "3% \u{2013} 5%",
            "3\u{2013}5 days",
            "3K \u{2013} 5K",
        ])
        .unwrap()
    );
}

/// `Intl.DateTimeFormat`/`Intl.NumberFormat` `formatRangeToParts` via the
/// opt-in ICU4C span categories, which carry the real
/// shared/startRange/endRange sources. Cross-checked against real Node.
#[cfg(all(feature = "intl", feature = "icu4c"))]
#[test]
fn intl_range_to_parts_icu4c_matches_real_node() {
    assert_eq!(
        load(
            "function dateParts() {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' })\n\
                 .formatRangeToParts(new Date(Date.UTC(2024, 6, 4)), new Date(Date.UTC(2024, 6, 5)));\n\
             }\n\
             function timeParts() {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', hour: 'numeric', minute: '2-digit' })\n\
                 .formatRangeToParts(new Date(Date.UTC(2024, 6, 4, 13, 0)), new Date(Date.UTC(2024, 6, 4, 15, 30)));\n\
             }\n\
             function currencyParts() {\n\
               return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).formatRangeToParts(3, 5);\n\
             }\n\
             function equalParts() {\n\
               return new Intl.NumberFormat('en-US').formatRangeToParts(3, 3);\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("dateParts", "[]"),
        r#"[{"type":"month","value":"Jul","source":"shared"},{"type":"literal","value":" ","source":"shared"},{"type":"day","value":"4","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"day","value":"5","source":"endRange"},{"type":"literal","value":", ","source":"shared"},{"type":"year","value":"2024","source":"shared"}]"#
    );
    assert_eq!(
        call("timeParts", "[]"),
        r#"[{"type":"hour","value":"1","source":"startRange"},{"type":"literal","value":":","source":"startRange"},{"type":"minute","value":"00","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"hour","value":"3","source":"endRange"},{"type":"literal","value":":","source":"endRange"},{"type":"minute","value":"30","source":"endRange"},{"type":"literal","value":" ","source":"shared"},{"type":"dayPeriod","value":"PM","source":"shared"}]"#
    );
    assert_eq!(
        call("currencyParts", "[]"),
        r#"[{"type":"currency","value":"$","source":"startRange"},{"type":"integer","value":"3","source":"startRange"},{"type":"decimal","value":".","source":"startRange"},{"type":"fraction","value":"00","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"currency","value":"$","source":"endRange"},{"type":"integer","value":"5","source":"endRange"},{"type":"decimal","value":".","source":"endRange"},{"type":"fraction","value":"00","source":"endRange"}]"#
    );
    assert_eq!(
        call("equalParts", "[]"),
        r#"[{"type":"approximatelySign","value":"~","source":"shared"},{"type":"integer","value":"3","source":"shared"}]"#
    );
}

/// `Intl.NumberFormat` `formatToParts` for `compact` and
/// `scientific`/`engineering` notation (previously collapsed to a single
/// literal part). Cross-checked against real Node.
#[cfg(feature = "intl")]
#[test]
fn intl_number_format_to_parts_compact_and_scientific_match_real_node() {
    assert_eq!(
        load(
            "function partsFor(locale, opts, value) {\n\
               return new Intl.NumberFormat(locale, opts).formatToParts(value);\n\
             }\n\
             function sci() { return partsFor('en-US', { notation: 'scientific' }, 1234); }\n\
             function eng() { return partsFor('en-US', { notation: 'engineering' }, 12345); }\n\
             function scineg() { return partsFor('en-US', { notation: 'scientific' }, -1234.5); }\n\
             function scinegexp() { return partsFor('en-US', { notation: 'scientific' }, 0.001234); }\n\
             function compact() { return partsFor('en-US', { notation: 'compact' }, 1234); }\n\
             function compactde() { return partsFor('de-DE', { notation: 'compact' }, 1234); }\n\
             function compactde2() { return partsFor('de-DE', { notation: 'compact' }, 123456); }\n\
             function compactja() { return partsFor('ja-JP', { notation: 'compact' }, 15127); }\n\
             function compactlong() { return partsFor('en-US', { notation: 'compact', compactDisplay: 'long' }, 1234); }"
        ),
        1
    );
    assert_eq!(
        call("sci", "[]"),
        r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"234"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#
    );
    assert_eq!(
        call("eng", "[]"),
        r#"[{"type":"integer","value":"12"},{"type":"decimal","value":"."},{"type":"fraction","value":"345"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#
    );
    assert_eq!(
        call("scineg", "[]"),
        r#"[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"235"},{"type":"exponentSeparator","value":"E"},{"type":"exponentInteger","value":"3"}]"#
    );
    assert_eq!(
        call("scinegexp", "[]"),
        r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"234"},{"type":"exponentSeparator","value":"E"},{"type":"exponentMinusSign","value":"-"},{"type":"exponentInteger","value":"3"}]"#
    );
    assert_eq!(
        call("compact", "[]"),
        r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"2"},{"type":"compact","value":"K"}]"#
    );
    assert_eq!(
        call("compactde", "[]"),
        r#"[{"type":"integer","value":"1234"}]"#
    );
    assert_eq!(
        call("compactde2", "[]"),
        r#"[{"type":"integer","value":"123"},{"type":"group","value":"."},{"type":"integer","value":"456"}]"#
    );
    assert_eq!(
        call("compactja", "[]"),
        r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"},{"type":"compact","value":"万"}]"#
    );
    assert_eq!(
        call("compactlong", "[]"),
        r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"2"},{"type":"literal","value":" "},{"type":"compact","value":"thousand"}]"#
    );
}

/// Without the opt-in ICU4C backend, the range methods use a real
/// shared/startRange/endRange partition of the two endpoints rather than
/// collapsing to a single part. Cross-checked against real Node for the
/// cases the locale-independent separator matches (en date, and en
/// numbers).
#[cfg(all(feature = "intl", not(feature = "icu4c")))]
#[test]
fn intl_range_fallback_partitions_endpoints_like_real_node() {
    assert_eq!(
        load(
            "function dateRange() {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' })\n\
                 .formatRange(new Date(Date.UTC(2024, 6, 4)), new Date(Date.UTC(2024, 6, 5)));\n\
             }\n\
             function dateParts() {\n\
               return new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' })\n\
                 .formatRangeToParts(new Date(Date.UTC(2024, 6, 4)), new Date(Date.UTC(2024, 6, 5)));\n\
             }\n\
             function num(locale, opts, start, end) {\n\
               return new Intl.NumberFormat(locale, opts).formatRange(start, end);\n\
             }\n\
             function numParts() {\n\
               return new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day' }).formatRangeToParts(3, 5);\n\
             }\n\
             function currencyParts() {\n\
               return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).formatRangeToParts(3, 5);\n\
             }\n\
             function all() {\n\
               return [\n\
                 num('en-US', {}, 3, 5),\n\
                 num('en-US', {}, 3, 3),\n\
                 num('en-US', { style: 'currency', currency: 'USD' }, 3, 5),\n\
                 num('en-US', { style: 'percent' }, 0.03, 0.05),\n\
                 num('en-US', { style: 'unit', unit: 'day' }, 3, 5),\n\
                 num('en-US', { notation: 'compact' }, 3000, 5000),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("dateRange", "[]"),
        serde_json::to_string("Jul 4\u{2009}\u{2013}\u{2009}5, 2024").unwrap()
    );
    assert_eq!(
        call("dateParts", "[]"),
        r#"[{"type":"month","value":"Jul","source":"shared"},{"type":"literal","value":" ","source":"shared"},{"type":"day","value":"4","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"day","value":"5","source":"endRange"},{"type":"literal","value":", ","source":"shared"},{"type":"year","value":"2024","source":"shared"}]"#
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "3\u{2013}5",
            "~3",
            "$3.00 \u{2013} $5.00",
            "3% \u{2013} 5%",
            "3\u{2013}5 days",
            "3K \u{2013} 5K",
        ])
        .unwrap()
    );
    assert_eq!(
        call("numParts", "[]"),
        r#"[{"type":"integer","value":"3","source":"startRange"},{"type":"literal","value":"–","source":"shared"},{"type":"integer","value":"5","source":"endRange"},{"type":"literal","value":" ","source":"shared"},{"type":"unit","value":"days","source":"shared"}]"#
    );
    assert_eq!(
        call("currencyParts", "[]"),
        r#"[{"type":"currency","value":"$","source":"startRange"},{"type":"integer","value":"3","source":"startRange"},{"type":"decimal","value":".","source":"startRange"},{"type":"fraction","value":"00","source":"startRange"},{"type":"literal","value":" – ","source":"shared"},{"type":"currency","value":"$","source":"endRange"},{"type":"integer","value":"5","source":"endRange"},{"type":"decimal","value":".","source":"endRange"},{"type":"fraction","value":"00","source":"endRange"}]"#
    );
}

/// UTS-35 skeleton matching (`intl_datetime_skeleton.rs`) against real
/// Node: icu4x's own `fieldsets::YMD` derives the requested field widths
/// from the locale's `dateFormats` length patterns, so `de`
/// `{month:'short'}` came out numeric (`4.07.2024`) instead of Node's
/// `4. Juli 2024`. These cases all exercised that divergence.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_skeleton_matching_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4);\n\
               return [\n\
                 show('de-DE', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('de-DE', {year:'numeric',month:'2-digit',day:'numeric'}, t),\n\
                 show('ja-JP', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('ko', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('ar', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('fr', {year:'numeric',month:'2-digit',day:'numeric'}, t),\n\
                 show('ru', {year:'numeric',month:'2-digit',day:'numeric'}, t),\n\
                 show('en-US', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('en-US', {weekday:'long'}, t),\n\
                 show('ca', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('hr', {year:'numeric',month:'long',day:'numeric'}, t),\n\
                 show('sk', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('bg', {year:'numeric',month:'long',day:'numeric'}, t),\n\
                 show('ko', {hour:'numeric',minute:'numeric'}, Date.UTC(2024, 6, 4, 16, 30)),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "4. Juli 2024",
            "4.07.2024",
            "2024年7月4日",
            "2024년 7월 4일",
            "4 يوليو 2024",
            "4/07/2024",
            "4.07.2024",
            "Jul 4, 2024",
            "Thursday",
            "4 de jul. del 2024",
            "4. srpnja 2024.",
            "4. 7. 2024",
            "4 юли 2024 г.",
            "PM 4:30",
        ])
        .unwrap()
    );
}

/// ECMA-402 `ToDateTimeOptions(options, "any", "date")` defaulting
/// (no date *or* time field -> `{year,month,day:'numeric'}`; `era` is not
/// a date field for this check) and ICU's exact-skeleton tie-break
/// (en-GB keeps `yMd`'s `dd/MM/y` for a `{year,month,day}` request).
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_default_fields_and_era_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4);\n\
               return [\n\
                 show('en-US', {}, t),\n\
                 show('en-US', {era:'long'}, t),\n\
                 show('en-GB', {year:'numeric',month:'numeric',day:'numeric'}, t),\n\
                 show('en-GB', {era:'long'}, t),\n\
                 show('fr', {year:'numeric',month:'numeric',day:'numeric'}, t),\n\
                 show('de', {era:'long'}, t),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "7/4/2024",
            "7/4/2024 Anno Domini",
            "04/07/2024",
            "04/07/2024 Anno Domini",
            "04/07/2024",
            "04.07.2024 n. Chr.",
        ])
        .unwrap()
    );
}

/// Format-vs-StandAlone weekday context (ICU gives `c`/`e` a different
/// `dtTypes` value than `E`, so fi's standard full pattern `cccc d. MMMM
/// y` loses to the availableFormats `yMMMMEd` -> `E d. MMMM y` widened to
/// `EEEE` -> the adessive "torstaina"), and the locale `ms` separator for
/// `{minute, second}` (id/da/fi use `.`).
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_weekday_context_and_ms_separator_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4, 16, 30, 45);\n\
               return [\n\
                 show('fi', {weekday:'long',year:'numeric',month:'long',day:'numeric'}, t),\n\
                 show('id', {minute:'numeric',second:'numeric'}, t),\n\
                 show('da', {minute:'numeric',second:'numeric'}, t),\n\
                 show('fi', {minute:'numeric',second:'numeric'}, t),\n\
                 show('en-US', {minute:'2-digit'}, t),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "torstaina 4. heinäkuuta 2024",
            "30.45",
            "30.45",
            "30.45",
            "30",
        ])
        .unwrap()
    );
}

/// `fa` (added to the curated list) defaults to the Persian calendar and
/// `arabext` numbering, exercising the non-Gregorian + non-Latin-digit
/// path end to end.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_fa_persian_defaults_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4);\n\
               return [\n\
                 show('fa', {year:'numeric',month:'long',day:'numeric'}, t),\n\
                 show('fa', {year:'numeric',month:'short',day:'numeric'}, t),\n\
                 show('fa', {year:'numeric',month:'numeric',day:'numeric'}, t),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&["۱۴ تیر ۱۴۰۳", "۱۴ تیر ۱۴۰۳", "۱۴۰۳/۴/۱۴",]).unwrap()
    );
}

/// `icu_datetime` renders a non-Gregorian `relatedYear` (Chinese/Dangi) in
/// Latin digits by design; ICU4C/Node localize it, so the JS path
/// re-localizes the `relatedYear` part with the locale's numbering system.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_chinese_related_year_digits_match_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4);\n\
               const o = { calendar: 'chinese', year: 'numeric', month: 'long', day: 'numeric' };\n\
               return [show('ar-SA', o, t), show('fa', o, t), show('zh', o, t)];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "٢٠٢٤(jia-chen) M05 ٢٩",
            "۲۰۲۴(jia-chen) M05 ۲۹",
            "2024甲辰年五月29",
        ])
        .unwrap()
    );
}

/// ICU's `PatternMap` candidate set + `getDistance` reproduce bg's `Hm`
/// split (numeric keeps the availableFormat `HH:mm 'ч'.`, `2-digit` picks
/// the standard `H:mm`) and cs's standard-medium adjustment, which the
/// earlier CLDR-availableFormats-only reconstruction got wrong.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_patternmap_selection_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const t = Date.UTC(2024, 6, 4, 16, 30, 45);\n\
               return [\n\
                 show('bg', {hour:'numeric',minute:'numeric'}, t),\n\
                 show('bg', {hour:'2-digit',minute:'2-digit'}, t),\n\
                 show('bg', {hour:'numeric',minute:'numeric',second:'numeric'}, t),\n\
                 show('cs', {year:'numeric',month:'2-digit',day:'2-digit'}, t),\n\
                 show('it', {year:'numeric',month:'2-digit',day:'numeric'}, t),\n\
                 show('uk', {year:'numeric',month:'2-digit',day:'numeric'}, t),\n\
                 show('fr', {year:'numeric',month:'numeric',day:'numeric'}, t),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "16:30 ч.",
            "16:30",
            "16:30:45 ч.",
            "04. 07. 2024",
            "04/07/2024",
            "04.07.2024",
            "04/07/2024",
        ])
        .unwrap()
    );
}

/// icu4x renders a Hebrew numeric month with its code number (and a
/// `6a`/`6b` leap suffix), but CLDR/ICU4C use the ordinal month (Adar I
/// counts), so `Intl.DateTimeFormat` swaps in `MonthInfo::ordinal` for a
/// numeric month pattern.
#[cfg(feature = "intl")]
#[test]
fn intl_datetime_hebrew_ordinal_month_matches_real_node() {
    assert_eq!(
        load(
            "function show(locale, opts, ms) {\n\
               return new Intl.DateTimeFormat(locale, { timeZone: 'UTC', ...opts }).format(new Date(ms));\n\
             }\n\
             function all() {\n\
               const opts = { calendar: 'hebrew', year: 'numeric', month: 'long', day: 'numeric' };\n\
               return [\n\
                 show('ja', opts, Date.UTC(2024, 6, 4)),\n\
                 show('ja', opts, Date.UTC(2023, 6, 4)),\n\
                 show('ja', opts, Date.UTC(2024, 1, 15)),\n\
                 show('ja', opts, Date.UTC(2024, 2, 15)),\n\
                 show('ja', { calendar: 'hebrew', year: 'numeric', month: 'numeric', day: 'numeric' }, Date.UTC(2024, 8, 15)),\n\
               ];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("all", "[]"),
        serde_json::to_string(&[
            "AM5784年10月28日",
            "AM5783年10月15日",
            "AM5784年6月6日",
            "AM5784年7月5日",
            "AM5784/13/12",
        ])
        .unwrap()
    );
}

/// Historical Intl remediation cases. Authored for the focused follow-up;
/// execution is left to the user-requested test phase.
#[cfg(feature = "intl")]
#[test]
fn intl_followup_locale_duration_and_exact_digits() {
    assert_eq!(
        load(
            "function intlFollowup() {\n\
               const locale = new Intl.Locale('de-CH-1901-u-hc-h23');\n\
               const merged = new Intl.Locale('en-u-ca-buddhist-hc-h23', { numberingSystem: 'arab' });\n\
               const privateUse = new Intl.Locale('en-x-foo', { numberingSystem: 'arab' });\n\
               let malformedLater = false;\n\
               try { new Intl.NumberFormat(['en-US', 'bad_tag']); }\n\
               catch (error) { malformedLater = error instanceof RangeError; }\n\
               const ameteAlem = new Intl.DateTimeFormat('en-u-ca-ethioaa').resolvedOptions().calendar;\n\
               const digits = new Intl.NumberFormat('en-US').format(9007199254740993n);\n\
               const decimal = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 }).format('9007199254740993.255');\n\
               const parts = new Intl.NumberFormat('en-US').formatToParts(9007199254740993n);\n\
               const digital = new Intl.DurationFormat('en', { style: 'digital' }).format({ hours: 1, minutes: 2, seconds: 3 });\n\
               let mixed = false;\n\
               try { new Intl.DurationFormat('en').format({ hours: 1, minutes: -2 }); }\n\
               catch (error) { mixed = error instanceof RangeError; }\n\
               return [locale.baseName, locale.toString(), merged.toString(), privateUse.toString(), malformedLater, ameteAlem, digits, decimal, parts.map(part => part.value).join(''), digital, mixed];\n\
             }"
        ),
        1
    );
    assert_eq!(
        call("intlFollowup", "[]"),
        r#"["de-CH-1901","de-CH-1901-u-hc-h23","en-u-ca-buddhist-hc-h23-nu-arab","en-u-nu-arab-x-foo",true,"ethioaa","9,007,199,254,740,993","9,007,199,254,740,993.26","9,007,199,254,740,993","1:02:03",true]"#
    );
}

/// CanonicalizeLocaleList reads array-like indices, validates every present
/// element, and does not consult an iterator. Kept unrun for the user test phase.
#[cfg(feature = "intl")]
#[test]
fn intl_followup_locale_list_boundary() {
    assert_eq!(
        load(
            r#"function intlLocaleListBoundary() {
              const locale = value => new Intl.NumberFormat(value).resolvedOptions().locale;
              const rejectsType = value => {
                try { locale(value); return false; }
                catch (error) { return error instanceof TypeError; }
              };
              const inherited = Object.create({ 0: 'ja-JP' });
              inherited.length = 1;
              const noIterator = ['ja-JP'];
              noIterator[Symbol.iterator] = () => { throw new Error('iterator was read'); };
              const instance = new Intl.Locale('ja-JP');
              instance.toString = () => 'bad_tag';
              return [
                locale([, 'ja-JP']) === 'ja-JP',
                locale({ 0: 'ja-JP', length: 1 }) === 'ja-JP',
                locale({ length: 0 }) === 'en-US',
                locale(inherited) === 'ja-JP',
                locale(noIterator) === 'ja-JP',
                locale(instance) === 'ja-JP',
                locale(['ja-JP', 'ja-jp']) === 'ja-JP',
                rejectsType([123]), rejectsType([null]),
                rejectsType([undefined]), rejectsType([Symbol()]),
                rejectsType({ length: 1n }),
              ];
            }"#
        ),
        1
    );
    assert_eq!(
        call("intlLocaleListBoundary", "[]"),
        "[true,true,true,true,true,true,true,true,true,true,true,true]"
    );
}

/// Source-level regression coverage for the four deferred Intl findings.
/// Execution is reserved for the user-requested test phase.
#[cfg(feature = "intl")]
#[test]
fn intl_followup_exact_notation_and_style_grouping() {
    assert_eq!(
        load(r#"function intlDeferredNumberCases() {
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
        }"#),
        1
    );
    assert_eq!(
        call("intlDeferredNumberCases", "[]"),
        r#"["9.007199254740993E15","9.007199254740993E15","9.007199254740993E15","123.456789012345678901E18","1E21","$1234.00","123456%",false,true,true,true]"#
    );
}

/// Large exact inputs must not become Infinity at the JS 10k guard or the
/// native fixed_decimal i16 magnitude boundary. Kept unrun for the requested
/// source-only remediation phase.
#[cfg(feature = "intl")]
#[test]
fn intl_large_exact_digits_share_format_parts_range_and_styles() {
    assert_eq!(
        load(r#"function intlLargeExact() {
          const short = '1' + '0'.repeat(10000);
          const huge = '1' + '0'.repeat(33000);
          const giant = BigInt(huge);
          const plain = new Intl.NumberFormat('en-US', { useGrouping: false });
          const grouped = new Intl.NumberFormat('en-US');
          const scientific = new Intl.NumberFormat('en-US', { notation: 'scientific' });
          const currency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', useGrouping: false });
          const percent = new Intl.NumberFormat('en-US', { style: 'percent', useGrouping: false });
          const scaledScientific = new Intl.NumberFormat('en-US', { style: 'percent', useGrouping: false });
          scaledScientific._notation = 'scientific';
          const scaledEngineering = new Intl.NumberFormat('en-US', { style: 'percent', useGrouping: false });
          scaledEngineering._notation = 'engineering';
          const unit = new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day', unitDisplay: 'long', useGrouping: false });
          const preciseCurrency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name', maximumFractionDigits: 3, useGrouping: false });
          const carriedCurrency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 2, minimumFractionDigits: 2, useGrouping: false });
          const localizedCurrency = new Intl.NumberFormat('en-u-nu-arab', { style: 'currency', currency: 'USD' });
          const signedUnit = new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day', signDisplay: 'always', useGrouping: false });
          const frenchPercent = new Intl.NumberFormat('fr-FR', { style: 'percent', useGrouping: false });
          const frenchUnit = new Intl.NumberFormat('fr-FR', { style: 'unit', unit: 'day', unitDisplay: 'long', useGrouping: false });
          const tinyPercent = new Intl.NumberFormat('fr-FR', { style: 'percent', maximumSignificantDigits: 3, useGrouping: false });
          const tinyPaddedPercent = new Intl.NumberFormat('fr-FR', { style: 'percent', minimumSignificantDigits: 3, maximumSignificantDigits: 3, useGrouping: false });
          const tinyCurrency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumSignificantDigits: 3, useGrouping: false });
          const tinyCurrencyName = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name', maximumSignificantDigits: 3, useGrouping: false });
          const tinyUnit = new Intl.NumberFormat('fr-FR', { style: 'unit', unit: 'day', unitDisplay: 'long', maximumSignificantDigits: 3, useGrouping: false });
          const text = plain.format(giant);
          const range = grouped.formatRange(huge, huge + '1');
          return [
            plain.format(short).length === 10001,
            text.length === 33001 && text[0] === '1' && text.endsWith('000'),
            plain.formatToParts(giant).map(p => p.value).join('') === text,
            grouped.format(giant).includes(',') && !grouped.format(giant).includes('Infinity'),
            scientific.format(giant) === '1E33000',
            scientific.format('1e-20000') === '1E-20000',
            new Intl.NumberFormat('en-US', { notation: 'scientific', signDisplay: 'exceptZero' }).format('-1e-20000').startsWith('-'),
            currency.format(giant).startsWith('$1') && currency.format(giant).endsWith('.00'),
            percent.format(giant).length > 33000 && percent.format(giant).endsWith('%'),
            percent.format(1e308).endsWith('%') && !percent.format(1e308).includes('Infinity'),
            scaledScientific.format('1') === '1E2' && scaledScientific.format('1e308') === '1E310',
            scaledEngineering.format('1') === '100E0' && scaledEngineering.format('1e308') === '10E309',
            unit.format(giant).endsWith(' days'),
            preciseCurrency.format(huge + '.001').includes('.001') && preciseCurrency.format(huge + '.001').includes('US dollars'),
            preciseCurrency.formatToParts(huge + '.001').map(p => p.value).join('') === preciseCurrency.format(huge + '.001'),
            carriedCurrency.format('9'.repeat(33001) + '.999').startsWith('$1') && carriedCurrency.format('9'.repeat(33001) + '.999').endsWith('.00'),
            localizedCurrency.formatToParts(giant).map(p => p.value).join('') === localizedCurrency.format(giant) && !localizedCurrency.format(giant).includes('Infinity'),
            signedUnit.format(giant).startsWith('+1') && signedUnit.format(giant).endsWith(' days'),
            frenchPercent.format(giant).slice(-2) === frenchPercent.format(1).slice(-2),
            frenchUnit.format(giant).includes('jours'),
            tinyPercent.format('1e-40000').length > 32000 && tinyPercent.format('1e-40000').slice(-2) === tinyPercent.format(1).slice(-2),
            tinyPaddedPercent.formatToParts('1e-40000').find(p => p.type === 'fraction').value.endsWith('100'),
            tinyCurrency.format('1e-40000').startsWith('$0.') && tinyCurrency.format('1e-40000').length > 32000,
            tinyCurrencyName.format('1e-40000').includes('US dollars') && tinyCurrencyName.formatToParts('1e-40000').map(p => p.value).join('') === tinyCurrencyName.format('1e-40000'),
            tinyUnit.format('1e-40000').includes('jour') && tinyUnit.format('1e-40000').length > 32000,
            range.includes('–') && !range.includes('Infinity'),
            grouped.formatRangeToParts(huge, huge + '1').map(p => p.value).join('') === range,
          ];
        }"#),
        1
    );
    assert_eq!(call("intlLargeExact", "[]"), "[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]");
}

/// The native Bh pattern supplies flexible-period names and ordering when
/// fractional seconds are the only requested numeric field. The synthetic
/// hour and its unit suffix must not leak into visible parts.
#[cfg(all(feature = "intl", not(feature = "icu4c")))]
#[test]
fn intl_flexible_day_period_with_fractional_seconds_only() {
    assert_eq!(
        load(r#"function intlFlexibleFractionOnly() {
          const date = new Date('2024-07-04T15:05:09.123Z');
          const shapes = [
            ['en-US', true, ' '], ['de', true, ' '], ['vi', true, ' '],
            ['bg', true, ' '], ['zh-Hant', false, ''], ['tr', false, ' '],
            ['ko', false, ' '], ['ja', false, ''], ['zh-Hans', false, ''],
          ];
          const all = shapes.map(([locale, fractionFirst, separator]) => {
            const base = { dayPeriod: 'long', timeZone: 'UTC' };
            const formatter = new Intl.DateTimeFormat(locale, { ...base, fractionalSecondDigits: 3 });
            const parts = formatter.formatToParts(date);
            const fields = parts.filter(part => part.type !== 'literal');
            const expected = fractionFirst
              ? ['fractionalSecond', 'dayPeriod'] : ['dayPeriod', 'fractionalSecond'];
            const bare = new Intl.DateTimeFormat(locale, base).formatToParts(date)
              .find(part => part.type === 'dayPeriod');
            const middle = parts.filter(part => part.type === 'literal').map(part => part.value).join('');
            return fields.length === 2 && fields.every((part, index) => part.type === expected[index]) &&
              fields.find(part => part.type === 'dayPeriod').value === bare.value &&
              fields.find(part => part.type === 'fractionalSecond').value.length > 0 &&
              middle === separator && formatter.format(date) === parts.map(part => part.value).join('');
          });
          const one = new Intl.DateTimeFormat('en-US', {
            dayPeriod: 'short', fractionalSecondDigits: 1, timeZone: 'UTC'
          }).formatToParts(date);
          const standalone = new Intl.DateTimeFormat('en-US', {
            fractionalSecondDigits: 3, timeZone: 'UTC'
          }).formatToParts(date);
          const leadingZero = new Intl.DateTimeFormat('en-US', {
            dayPeriod: 'long', fractionalSecondDigits: 3, timeZone: 'UTC'
          }).formatToParts(new Date('2024-07-04T15:05:09.005Z'));
          return [...all,
            one.filter(part => part.type !== 'literal').map(part => part.type).join(',') === 'fractionalSecond,dayPeriod',
            standalone.filter(part => part.type !== 'literal').map(part => part.type).join(',') === 'fractionalSecond',
            leadingZero.find(part => part.type === 'fractionalSecond').value === '005'];
        }"#),
        1
    );
    assert_eq!(call("intlFlexibleFractionOnly", "[]"), "[true,true,true,true,true,true,true,true,true,true,true,true]");
}

/// Combined flexible day-period requests use the locale skeleton without
/// introducing a synthetic hour in the visible parts.
#[cfg(all(feature = "intl", not(feature = "icu4c")))]
#[test]
fn intl_flexible_day_period_combined_without_hour() {
    assert_eq!(
        load(r#"function intlFlexibleCombined() {
          const date = new Date('2024-07-04T15:05:09Z');
          const base = { dayPeriod: 'long', timeZone: 'UTC' };
          function parts(locale, extra) {
            const formatter = new Intl.DateTimeFormat(locale, { ...base, ...extra });
            const value = formatter.formatToParts(date);
            return { value, text: formatter.format(date) };
          }
          function fields(result, expected) {
            const actual = result.value.filter(part => part.type !== 'literal').map(part => part.type);
            return actual.length === expected.length && expected.every(type => actual.includes(type)) &&
              result.text === result.value.map(part => part.value).join('');
          }
          const only = parts('en-US', {});
          const minute = parts('en-US', { minute: '2-digit' });
          const second = parts('en-US', { second: '2-digit' });
          const both = parts('en-US', { minute: '2-digit', second: '2-digit' });
          const dated = parts('en-US', { year: 'numeric', month: 'numeric', day: 'numeric', minute: '2-digit' });
          const chinese = parts('zh-Hans', { minute: '2-digit' });
          const finnish = parts('fi', { minute: '2-digit', second: '2-digit' });
          const cycle = parts('en-US', { minute: '2-digit', hourCycle: 'h23' });
          const withHour = parts('en-US', { hour: 'numeric', dayPeriod: 'long' });
          const period = only.value.find(part => part.type === 'dayPeriod').value;
          const samePeriod = result => result.value.find(part => part.type === 'dayPeriod').value === period;
          return [
            fields(only, ['dayPeriod']),
            fields(minute, ['minute', 'dayPeriod']) && samePeriod(minute),
            fields(second, ['second', 'dayPeriod']) && samePeriod(second),
            fields(both, ['minute', 'second', 'dayPeriod']) && samePeriod(both),
            fields(dated, ['year', 'month', 'day', 'minute', 'dayPeriod']) && samePeriod(dated),
            fields(chinese, ['dayPeriod', 'minute']) &&
              chinese.value.findIndex(part => part.type === 'dayPeriod') < chinese.value.findIndex(part => part.type === 'minute'),
            fields(finnish, ['minute', 'second', 'dayPeriod']) &&
              finnish.value.some(part => part.type === 'literal' && part.value.includes('.')),
            fields(cycle, ['minute', 'dayPeriod']) && samePeriod(cycle),
            fields(withHour, ['hour', 'dayPeriod']),
          ];
        }"#),
        1
    );
    assert_eq!(
        call("intlFlexibleCombined", "[]"),
        "[true,true,true,true,true,true,true,true,true]"
    );
}

/// Sign display observes the rounded numeric magnitude, while auto/always
/// retain the input's negative-zero sign.
#[cfg(all(feature = "intl", not(feature = "icu4c")))]
#[test]
fn intl_rounded_zero_sign_matches_format_parts_and_ranges() {
    assert_eq!(
        load(r#"function intlRoundedZeroSign() {
          const options = { maximumFractionDigits: 0, useGrouping: false };
          const exceptZero = new Intl.NumberFormat('en-US', { ...options, signDisplay: 'exceptZero' });
          const negative = new Intl.NumberFormat('en-US', { ...options, signDisplay: 'negative' });
          const automatic = new Intl.NumberFormat('en-US', { ...options, signDisplay: 'auto' });
          const always = new Intl.NumberFormat('en-US', { ...options, signDisplay: 'always' });
          const scientific = new Intl.NumberFormat('en-US', { notation: 'scientific', maximumFractionDigits: 0,
            signDisplay: 'negative' });
          const percent = new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 0,
            signDisplay: 'exceptZero' });
          const currency = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD',
            maximumFractionDigits: 2, signDisplay: 'negative' });
          const renderedParts = (formatter, value) => formatter.formatToParts(value).map(part => part.value).join('');
          return [
            exceptZero.format(0.0001) === '0' && exceptZero.format(-0.0001) === '0',
            exceptZero.format('0.0001') === '0' && exceptZero.format('-0.0001') === '0',
            negative.format(-0.0001) === '0' && negative.format('-0.0001') === '0',
            automatic.format(-0.0001) === '-0' && always.format(0.0001) === '+0' && always.format(-0.0001) === '-0',
            renderedParts(exceptZero, -0.0001) === exceptZero.format(-0.0001) &&
              !exceptZero.formatToParts(-0.0001).some(part => part.type === 'minusSign' || part.type === 'plusSign'),
            renderedParts(negative, '-0.0001') === negative.format('-0.0001') &&
              !negative.formatToParts('-0.0001').some(part => part.type === 'minusSign'),
            scientific.format(-0) === '0E0' && renderedParts(scientific, -0) === '0E0',
            percent.format(-0.000001) === '0%' && renderedParts(percent, -0.000001) === '0%',
            currency.format(-0.0001) === '$0.00' && renderedParts(currency, -0.0001) === '$0.00',
            exceptZero.formatRange('-0.0001', '0.0001') === '~0' &&
              exceptZero.formatRangeToParts('-0.0001', '0.0001').map(part => part.value).join('') === '~0',
            new Intl.NumberFormat('en-US', { signDisplay: 'exceptZero', maximumSignificantDigits: 3 })
              .format('-1e-40000').startsWith('-'),
          ];
        }"#),
        1
    );
    assert_eq!(
        call("intlRoundedZeroSign", "[]"),
        "[true,true,true,true,true,true,true,true,true,true,true]"
    );
}

#[cfg(all(feature = "intl", not(feature = "icu4c")))]
#[test]
fn intl_followup_flexible_day_period_without_icu4c() {
    assert_eq!(
        load(r#"function intlFlexibleDayPeriod() {
          const date = new Date('2024-07-04T15:00:00Z');
          const options = { hour: 'numeric', dayPeriod: 'long', timeZone: 'UTC' };
          return new Intl.DateTimeFormat('en-US', options).formatToParts(date)
            .some(part => part.type === 'dayPeriod' && part.value.toLowerCase().includes('afternoon'));
        }"#),
        1
    );
    assert_eq!(call("intlFlexibleDayPeriod", "[]"), "true");
}

#[test]
fn buffer_boundary_and_encoding_regressions() {
    assert_eq!(
        load(r#"function bufferBoundaryRegressions() {
          const written = Buffer.alloc(2);
          const n = written.write('61', 'hex');
          const short = Buffer.alloc(2);
          const limited = short.write('abcd', 0, 4);
          let fractionalLengthFailed = false, infiniteLengthFailed = false;
          try { short.write('ab', 0, 2.5); } catch (e) { fractionalLengthFailed = e instanceof RangeError; }
          try { short.write('ab', 0, Infinity); } catch (e) { infiniteLengthFailed = e instanceof RangeError; }
          const utf8 = Buffer.alloc(2);
          const utf8Count = utf8.write('あ');
          const filled = Buffer.alloc(5, new Uint8Array([1, 2]));
          const fromElements = Buffer.from(new Uint16Array([0x1234]));
          let readFailed = false, writeFailed = false;
          try { Buffer.alloc(1).readUInt16LE(0); } catch (e) { readFailed = e instanceof RangeError; }
          try { Buffer.alloc(1).writeUInt16LE(7, 0); } catch (e) { writeFailed = e instanceof RangeError; }
          return [n, written.toString('hex'), limited, short.toString(), utf8Count,
            utf8.toString('hex'), filled.toString('hex'), fromElements.toString('hex'), readFailed, writeFailed,
            fractionalLengthFailed, infiniteLengthFailed];
        }"#),
        1
    );
    assert_eq!(
        call("bufferBoundaryRegressions", "[]"),
        r#"[1,"6100",2,"ab",0,"0000","0102010201","34",true,true,true,true]"#
    );
}

#[test]
fn stream_source_callbacks_are_captured_once_before_start() {
    assert_eq!(load(r#"
      async function streamSourceCallbackCapture() {
        const reads = [], calls = [];
        const source = {
          get autoAllocateChunkSize() { reads.push('autoAllocateChunkSize'); return undefined; },
          get cancel() { reads.push('cancel'); return function (reason) { calls.push(['cancel', this === source, reason]); }; },
          get pull() { reads.push('pull'); return function (controller) { calls.push(['pull', this === source]); controller.enqueue('chunk'); }; },
          get start() { reads.push('start'); return function () { calls.push(['start', this === source]); Object.defineProperty(this, 'pull', { value: null }); Object.defineProperty(this, 'cancel', { value: 12 }); }; },
          get type() { reads.push('type'); return undefined; }
        };
        const reader = new ReadableStream(source, { highWaterMark: 0 }).getReader();
        const value = (await reader.read()).value;
        await reader.cancel('stop');
        return [reads, calls, value];
      }
    "#), 1);
    assert_eq!(call("streamSourceCallbackCapture", "[]"),
        r#"[["autoAllocateChunkSize","cancel","pull","start","type"],[["start",true],["pull",true],["cancel",true,"stop"]],"chunk"]"#);
}

#[test]
fn stream_sink_callbacks_are_captured_once_before_start() {
    assert_eq!(load(r#"
      async function streamSinkCallbackCapture() {
        const reads = [], calls = [];
        const sink = {
          get abort() { reads.push('abort'); return function (reason) { calls.push(['abort', this === sink, reason]); }; },
          get close() { reads.push('close'); return function () { calls.push(['close', this === sink]); }; },
          get start() { reads.push('start'); return function () { calls.push(['start', this === sink]); Object.defineProperty(this, 'write', { value: null }); Object.defineProperty(this, 'close', { value: 1 }); Object.defineProperty(this, 'abort', { value: null }); }; },
          get type() { reads.push('type'); return undefined; },
          get write() { reads.push('write'); return function (chunk) { calls.push(['write', this === sink, chunk]); }; }
        };
        const writer = new WritableStream(sink).getWriter();
        await writer.write('chunk');
        await writer.close();
        const abortSink = {
          start() { this.abort = 1; },
          abort(reason) { calls.push(['abort-after-start', this === abortSink, reason]); }
        };
        await new WritableStream(abortSink).abort('stop');
        return [reads, calls];
      }
    "#), 1);
    assert_eq!(call("streamSinkCallbackCapture", "[]"),
        r#"[["abort","close","start","type","write"],[["start",true],["write",true,"chunk"],["close",true],["abort-after-start",true,"stop"]]]"#);
}

#[test]
fn stream_transformer_callbacks_are_captured_once_before_start() {
    assert_eq!(load(r#"
      async function streamTransformerCallbackCapture() {
        const reads = [], calls = [];
        const transformer = {
          get cancel() { reads.push('cancel'); return function (reason) { calls.push(['cancel', this === transformer, reason]); }; },
          get flush() { reads.push('flush'); return function () { calls.push(['flush', this === transformer]); }; },
          get readableType() { reads.push('readableType'); return undefined; },
          get start() { reads.push('start'); return function () { calls.push(['start', this === transformer]); Object.defineProperty(this, 'transform', { value: null }); Object.defineProperty(this, 'flush', { value: 1 }); Object.defineProperty(this, 'cancel', { value: null }); }; },
          get transform() { reads.push('transform'); return function (chunk, controller) { calls.push(['transform', this === transformer, chunk]); controller.enqueue(chunk); }; },
          get writableType() { reads.push('writableType'); return undefined; }
        };
        const stream = new TransformStream(transformer, {}, { highWaterMark: 1 });
        const writer = stream.writable.getWriter(), reader = stream.readable.getReader();
        await writer.write('chunk');
        const value = (await reader.read()).value;
        await writer.close();
        const cancelTransformer = { start() { this.cancel = 1; }, cancel(reason) { calls.push(['cancel-after-start', this === cancelTransformer, reason]); } };
        await new TransformStream(cancelTransformer).readable.cancel('stop');
        return [reads, calls, value];
      }
    "#), 1);
    assert_eq!(call("streamTransformerCallbackCapture", "[]"),
        r#"[["cancel","flush","readableType","start","transform","writableType"],[["start",true],["transform",true,"chunk"],["flush",true],["cancel-after-start",true,"stop"]],"chunk"]"#);
}

#[test]
fn stream_dictionary_rejects_noncallable_callbacks_before_start() {
    assert_eq!(load(r#"
      function streamRejectsNoncallableCallbacks() {
        let starts = 0;
        const check = construct => { try { construct(); return false; } catch (error) { return error instanceof TypeError; } };
        return [
          check(() => new ReadableStream({ pull: null, start() { starts++; } })),
          check(() => new WritableStream({ abort: 1, start() { starts++; } })),
          check(() => new TransformStream({ transform: false, start() { starts++; } })),
          starts
        ];
      }
    "#), 1);
    assert_eq!(call("streamRejectsNoncallableCallbacks", "[]"),
        "[true,true,true,0]");
}

#[test]
fn stream_strategy_getters_precede_callback_conversion_once() {
    assert_eq!(load(r#"
      function streamStrategyGetterOrder() {
        const reads = [];
        const strategy = name => ({
          get highWaterMark() { reads.push(name + '.highWaterMark'); return 1; },
          get size() { reads.push(name + '.size'); return () => 1; }
        });
        const source = {
          get autoAllocateChunkSize() { reads.push('source.autoAllocateChunkSize'); return undefined; },
          get cancel() { reads.push('source.cancel'); return undefined; },
          get pull() { reads.push('source.pull'); return undefined; },
          get start() { reads.push('source.start'); return function () { reads.push('source.start()'); }; },
          get type() { reads.push('source.type'); return undefined; }
        };
        new ReadableStream(source, strategy('readable'));
        const sink = {
          get abort() { reads.push('sink.abort'); return undefined; },
          get close() { reads.push('sink.close'); return undefined; },
          get start() { reads.push('sink.start'); return function () { reads.push('sink.start()'); }; },
          get type() { reads.push('sink.type'); return undefined; },
          get write() { reads.push('sink.write'); return undefined; }
        };
        new WritableStream(sink, strategy('writable'));
        const transformer = {
          get cancel() { reads.push('transformer.cancel'); return undefined; },
          get flush() { reads.push('transformer.flush'); return undefined; },
          get readableType() { reads.push('transformer.readableType'); return undefined; },
          get start() { reads.push('transformer.start'); return function () { reads.push('transformer.start()'); }; },
          get transform() { reads.push('transformer.transform'); return undefined; },
          get writableType() { reads.push('transformer.writableType'); return undefined; }
        };
        new TransformStream(transformer, strategy('transformWritable'), strategy('transformReadable'));
        return reads;
      }
    "#), 1);
    assert_eq!(call("streamStrategyGetterOrder", "[]"),
        r#"["readable.highWaterMark","readable.size","source.autoAllocateChunkSize","source.cancel","source.pull","source.start","source.type","source.start()","writable.highWaterMark","writable.size","sink.abort","sink.close","sink.start","sink.type","sink.write","sink.start()","transformWritable.highWaterMark","transformWritable.size","transformReadable.highWaterMark","transformReadable.size","transformer.cancel","transformer.flush","transformer.readableType","transformer.start","transformer.transform","transformer.writableType","transformer.start()"]"#);
}

#[test]
fn stream_start_exceptions_escape_constructors_synchronously() {
    assert_eq!(load(r#"
      function streamStartThrowsSynchronously() {
        const sentinel = new Error('start failure');
        const throwsSame = construct => { try { construct(); return false; } catch (error) { return error === sentinel; } };
        return [
          throwsSame(() => new ReadableStream({ start() { throw sentinel; } })),
          throwsSame(() => new WritableStream({ start() { throw sentinel; } })),
          throwsSame(() => new TransformStream({ start() { throw sentinel; } }))
        ];
      }
    "#), 1);
    assert_eq!(call("streamStartThrowsSynchronously", "[]"), "[true,true,true]");
}

#[test]
fn writable_pending_writes_skip_sink_after_error_or_abort() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writablePendingWriteStops", "[]"),
        r#"[[["second-abort","stop"]],true,true,true,true]"#);
}

#[test]
fn writable_queued_writes_drain_before_close_and_reject_on_failure() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writableCloseAfterQueuedWrites", "[]"),
        r#"[["a","b","close"],true,true,true]"#);
}

#[test]
fn writable_abort_failure_settles_writer_closed_with_original_reason() {
    assert_eq!(load(r#"
      async function writableAbortFailureSettlement() {
        const reason = new Error('abort reason'), failure = new Error('sink failure');
        const events = [];
        const writer = new WritableStream({ abort(value) { events.push(value === reason); throw failure; } }).getWriter();
        const abortFailure = await writer.abort(reason).then(() => false, error => error === failure);
        const closedReason = await writer.closed.then(() => false, error => error === reason);
        const secondAbort = await writer.abort('again').then(() => true, () => false);
        return [events, abortFailure, closedReason, secondAbort];
      }
    "#), 1);
    assert_eq!(call("writableAbortFailureSettlement", "[]"),
        "[[true],true,true,true]");
}

#[test]
fn writable_abort_during_in_flight_close_does_not_call_sink_abort() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writableAbortDuringClose", "[]"), r#"["close"]"#);
}

#[test]
fn writable_close_queued_keeps_controller_desired_size() {
    assert_eq!(load(r#"
      function writableDesiredSizeWhileClosing() {
        const writer = new WritableStream({ write() {} }, { highWaterMark: 3 }).getWriter();
        const pending = writer.write('x');
        const before = writer.desiredSize;
        const closing = writer.close();
        const during = writer.desiredSize;
        pending.catch(() => {}); closing.catch(() => {});
        return [before, during];
      }
    "#), 1);
    assert_eq!(call("writableDesiredSizeWhileClosing", "[]"), "[2,2]");
}

#[test]
fn writable_in_flight_close_success_overrides_controller_error() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writableCloseWinsControllerError", "[]"), "[true,true,0]");
}

#[test]
fn writable_in_flight_close_failure_preserves_stored_error() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writableCloseFailsAfterAbort", "[]"), "[true,true,true,true]");
}

#[test]
fn writable_in_flight_close_failure_after_controller_error() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writableCloseFailsAfterControllerError", "[]"), "[true,true,null]");
}

#[test]
fn writable_abort_listener_reentry_shares_pending_abort() {
    assert_eq!(load(r#"
      async function writableAbortListenerReentry() {
        let controller, nested;
        const calls = [];
        const writer = new WritableStream({
          start(value) { controller = value; },
          abort(reason) { calls.push(reason); }
        }).getWriter();
        controller.signal.addEventListener('abort', () => { nested = writer.abort('other'); });
        const outer = writer.abort('first');
        const same = nested === outer;
        await outer;
        const closed = await writer.closed.then(() => false, reason => reason === 'first');
        return [same, calls, closed];
      }
    "#), 1);
    assert_eq!(call("writableAbortListenerReentry", "[]"), r#"[true,["first"],true]"#);
}

#[test]
fn headers_iterators_observe_mutation_and_resume_after_done() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("headersIteratorsObserveMutation", "[]"),
        r#"[["b","B",["b","B"]],["c","C",["c","C"]],[null,null,null],[true,true,true],["e","E",["e","E"]]]"#);
}

#[test]
fn headers_for_each_refetches_pairs_after_callback() {
    assert_eq!(load(r#"
      function headersForEachRefetchesPairs() {
        const headers = new Headers([['a', 'A'], ['b', 'B']]);
        const receiver = {};
        const seen = [];
        function visit(value, name, target) {
          seen.push([name, value, this === receiver, target === headers]);
          if (name === 'a') { headers.delete('b'); headers.append('c', 'C'); }
          if (name === 'c') headers.append('d', 'D');
        }
        visit.call = () => { throw new Error('callback.call must not be used'); };
        headers.entries = () => { throw new Error('public entries must not be used'); };
        headers.forEach(visit, receiver);
        return [seen, [...headers.keys()], [...headers]];
      }
    "#), 1);
    assert_eq!(call("headersForEachRefetchesPairs", "[]"),
        r#"[[["a","A",true,true],["c","C",true,true],["d","D",true,true]],["a","c","d"],[["a","A"],["c","C"],["d","D"]]]"#);
}

#[test]
fn headers_set_cookie_and_iterator_brands() {
    assert_eq!(load(r#"
      function headersSetCookieAndIteratorBrands() {
        const headers = new Headers([['x', 'one'], ['set-cookie', 'a=1'], ['x', 'two'], ['set-cookie', 'b=2']]);
        const entries = headers.entries();
        const initial = [...entries];
        const done = entries.next().done;
        headers.append('x', 'three');
        headers.append('z', 'Z');
        const resumed = entries.next().value;
        const combined = [...headers.values()];
        const rejectsReceiver = (() => { try { Headers.prototype.keys.call({}); return false; } catch (error) { return error instanceof TypeError; } })();
        const rejectsIterator = (() => { try { entries.next.call({}); return false; } catch (error) { return error instanceof TypeError; } })();
        return [initial, done, resumed, combined, rejectsReceiver, rejectsIterator];
      }
    "#), 1);
    assert_eq!(call("headersSetCookieAndIteratorBrands", "[]"),
        r#"[[["set-cookie","a=1"],["set-cookie","b=2"],["x","one, two"]],true,["z","Z"],["a=1","b=2","one, two, three","Z"],true,true]"#);
}

#[test]
fn form_data_iterators_observe_set_and_resume_after_done() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("formDataIteratorsObserveSet", "[]"),
        r#"[["a","A",["a","A"],["a","A"]],["b","B2",["b","B2"],["b","B2"]],["c","C",["c","C"],["c","C"]],[true,true,true,true],["d","D",["d","D"],["d","D"]]]"#);
}

#[test]
fn form_data_iteration_refetches_after_delete_and_callback() {
    assert_eq!(load(r#"
      function formDataIterationRefetchesAfterDelete() {
        const data = new FormData();
        data.append('a', 'A'); data.append('b', 'B'); data.append('c', 'C');
        const iterator = data.entries();
        const first = iterator.next().value;
        data.delete('b');
        const second = iterator.next().value;
        const receiver = {};
        const seen = [];
        function visit(value, name, target) {
          seen.push([name, value, this === receiver, target === data]);
          if (name === 'a') data.set('c', 'C2');
          if (name === 'c') data.append('d', 'D');
        }
        visit.call = () => { throw new Error('callback.call must not be used'); };
        data.entries = () => { throw new Error('public entries must not be used'); };
        data.forEach(visit, receiver);
        return [first, second, seen, [...data.keys()], [...data]];
      }
    "#), 1);
    assert_eq!(call("formDataIterationRefetchesAfterDelete", "[]"),
        r#"[["a","A"],["c","C"],[["a","A",true,true],["c","C2",true,true],["d","D",true,true]],["a","c","d"],[["a","A"],["c","C2"],["d","D"]]]"#);
}

#[test]
fn form_data_iteration_checks_receiver_at_call_time() {
    assert_eq!(load(r#"
      function formDataIterationChecksReceiver() {
        const data = new FormData();
        const iterator = data.keys();
        const rejects = method => { try { method.call({}); return false; } catch (error) { return error instanceof TypeError; } };
        return [rejects(FormData.prototype.keys), rejects(FormData.prototype.values),
          rejects(FormData.prototype.entries), rejects(FormData.prototype.forEach),
          rejects(FormData.prototype[Symbol.iterator]), rejects(iterator.next)];
      }
    "#), 1);
    assert_eq!(call("formDataIterationChecksReceiver", "[]"), "[true,true,true,true,true,true]");
}

#[test]
fn readable_reader_closed_promises_follow_each_lock() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("readableReaderClosedPromisesFollowLock", "[]"), "[true,true,true,true,true,true]");
}

#[test]
fn byob_reader_closed_promises_follow_each_lock_and_error() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("byobReaderClosedPromisesFollowLock", "[]"), "[true,true,true,true,true]");
}

#[test]
fn writer_ready_and_closed_promises_follow_each_lock() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writerReadyAndClosedPromisesFollowLock", "[]"), "[true,true,true,true,true,true,true]");
}

#[test]
fn writer_backpressure_reassignment_keeps_old_promise_rejected() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writerBackpressureReassignment", "[]"), "[true,true,true,true,true,true,true]");
}

#[test]
fn writer_relock_during_abort_waits_for_closed_settlement() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("writerRelockDuringAbort", "[]"), "[true,true,true]");
}

#[test]
fn tee_preserves_falsy_error_reasons_in_default_and_byte_branches() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("teePreservesFalsyErrors", "[]"),
        "[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]");
}

#[test]
fn tee_closes_normally_and_keeps_cancellation_separate_from_errors() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("teeCloseAndCancelPaths", "[]"),
        "[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]");
}

#[test]
fn pipe_to_detects_destination_error_while_read_is_pending() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("pipeToDetectsDestinationError", "[false]"), "[true,true,false,false]");
    assert_eq!(call("pipeToDetectsDestinationError", "[true]"), "[true,false,false,false]");
}

#[test]
fn pipe_to_waits_for_destination_capacity_before_reading() {
    assert_eq!(load(r#"
      async function pipeToWaitsForDestinationCapacity() {
        let destinationController, canceled;
        const source = new ReadableStream({ cancel(reason) { canceled = reason; } }, { highWaterMark: 0 });
        const destination = new WritableStream({ start(controller) { destinationController = controller; } }, { highWaterMark: 0 });
        const error = new Error('destination');
        const piping = source.pipeTo(destination);
        const hadNoReadRequest = source._reads.length === 0;
        destinationController.error(error);
        const rejected = await piping.then(() => false, reason => reason === error);
        return [hadNoReadRequest, rejected, canceled === error, source.locked, destination.locked];
      }
    "#), 1);
    assert_eq!(call("pipeToWaitsForDestinationCapacity", "[]"), "[true,true,true,false,false]");
}

#[test]
fn pipe_to_observes_source_termination_while_backpressured() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("pipeToObservesSourceTermination", "[false]"), "[\"resolved\",false,1,false,false]");
    assert_eq!(call("pipeToObservesSourceTermination", "[true]"), "[\"source-error\",true,0,false,false]");
}

#[test]
fn pipe_to_rejects_an_already_closed_destination() {
    assert_eq!(load(r#"
      async function pipeToRejectsClosedDestination() {
        let canceled;
        const source = new ReadableStream({ cancel(reason) { canceled = reason; } }, { highWaterMark: 0 });
        const destination = new WritableStream();
        await destination.close();
        const outcome = await source.pipeTo(destination).then(() => false, error => error instanceof TypeError);
        return [outcome, canceled instanceof TypeError, source.locked, destination.locked];
      }
    "#), 1);
    assert_eq!(call("pipeToRejectsClosedDestination", "[]"), "[true,true,false,false]");
}

#[test]
fn pipe_to_respects_ordered_initial_close_and_pending_destination_close() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("pipeToOrderedCloseStates", "[]"), "[true,true,true,false,false,false,false]");
}

#[test]
fn pipe_to_preserves_prevent_flags_and_falsy_destination_reason() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("pipeToPreventFlagsAndFalsyReason", "[]"), "[true,0,true,0,false,true,true,true,false,false]");
}

#[test]
fn pipe_to_distinguishes_equal_signal_and_source_error_reasons() {
    assert_eq!(load(r#"
      async function pipeToEqualErrorOrigins() {
        const reason = new Error('shared');
        let cancelAttempts = 0, abortCount = 0;
        const source = new ReadableStream({ start(controller) { controller.error(reason); } }, { highWaterMark: 0 });
        const originalCancel = source._cancel;
        source._cancel = function(value) { cancelAttempts++; return originalCancel.call(this, value); };
        const destination = new WritableStream({ abort() { abortCount++; } });
        const signal = new AbortController();
        const piping = source.pipeTo(destination, { signal: signal.signal });
        signal.abort(reason);
        const rejected = await piping.then(() => false, error => error === reason);
        return [rejected, cancelAttempts, abortCount, source.locked, destination.locked];
      }
    "#), 1);
    assert_eq!(call("pipeToEqualErrorOrigins", "[]"), "[true,0,1,false,false]");
}

#[test]
fn pipe_to_initial_abort_precedes_source_and_destination_propagation() {
    assert_eq!(load(r#"
      async function pipeToInitialAbortPriority() {
        const signalReason = new Error('signal');
        const sourceReason = new Error('source');
        const signal = new AbortController(); signal.abort(signalReason);
        let erroredCancelCount = 0, erroredAbortReason;
        const erroredSource = new ReadableStream({ start(controller) { controller.error(sourceReason); } });
        const originalCancel = erroredSource._cancel;
        erroredSource._cancel = function(reason) { erroredCancelCount++; return originalCancel.call(this, reason); };
        const erroredDestination = new WritableStream({ abort(reason) { erroredAbortReason = reason; } });
        const erroredOutcome = await erroredSource.pipeTo(erroredDestination, { signal: signal.signal })
          .then(() => false, reason => reason === signalReason);

        let closedAbortReason;
        const closedSource = new ReadableStream({ start(controller) { controller.close(); } });
        const closedDestination = new WritableStream({ abort(reason) { closedAbortReason = reason; } });
        const closedOutcome = await closedSource.pipeTo(closedDestination, { signal: signal.signal })
          .then(() => false, reason => reason === signalReason);

        let readableCancelReason, preventedCancelCount = 0;
        const readableSource = new ReadableStream({ cancel(reason) { readableCancelReason = reason; } });
        const alreadyClosedDestination = new WritableStream();
        await alreadyClosedDestination.close();
        const destinationOutcome = await readableSource.pipeTo(alreadyClosedDestination, { signal: signal.signal })
          .then(() => false, reason => reason === signalReason);

        const preventedSource = new ReadableStream({ cancel() { preventedCancelCount++; } });
        const preventedDestination = new WritableStream();
        await preventedDestination.close();
        const preventedOutcome = await preventedSource.pipeTo(preventedDestination,
          { signal: signal.signal, preventCancel: true, preventAbort: true })
          .then(() => false, reason => reason === signalReason);
        return [erroredOutcome, erroredCancelCount, erroredAbortReason === signalReason,
          closedOutcome, closedAbortReason === signalReason, destinationOutcome,
          readableCancelReason === signalReason, preventedOutcome, preventedCancelCount,
          erroredSource.locked, erroredDestination.locked, closedSource.locked,
          closedDestination.locked, readableSource.locked, alreadyClosedDestination.locked,
          preventedSource.locked, preventedDestination.locked];
      }
    "#), 1);
    assert_eq!(call("pipeToInitialAbortPriority", "[]"),
      "[true,0,true,true,true,true,true,true,0,false,false,false,false,false,false,false,false]");
}

#[test]
fn crypto_buffer_regressions() {
    assert_eq!(
        load(r#"async function cryptoBufferRegressions() {
          const cryptoModule = __thaw_crypto_module;
          const raw = new Uint8Array([1, 2, 3]);
          const key = await crypto.subtle.importKey('raw', raw, {name: 'HMAC', hash: 'SHA-256'}, false, ['sign']);
          raw[0] = 9;
          const keyCopied = key.__thawRaw.toString('hex') === '010203';
          const hash = cryptoModule.createHash('sha256').update('a'); hash.digest();
          const hmac = cryptoModule.createHmac('sha256', 'key').update('a');
          const copiedHmac = hmac.copy().update('b').digest('hex') ===
            cryptoModule.createHmac('sha256', 'key').update('ab').digest('hex');
          const sha384Key = Buffer.alloc(20, 0x0b);
          const sha384Hmac = cryptoModule.createHmac('sha384', sha384Key).update('Hi There').digest('hex') ===
            'afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6';
          let copyFailed = false;
          try { hash.copy(); } catch (e) { copyFailed = true; }
          let typeFailed = false;
          try { crypto.getRandomValues(new Float32Array(1)); } catch (e) { typeFailed = e.name === 'TypeMismatchError'; }
          const original = __thaw_crypto_random_hex;
          let calls = 0, sampled;
          try {
            __thaw_crypto_random_hex = () => (++calls === 1 ? 'ffffffffffff' : '000000000001');
            sampled = cryptoModule.randomInt(0, 3);
          } finally { __thaw_crypto_random_hex = original; }
          const ecdh = cryptoModule.createECDH('P-256');
          const compressed = ecdh.generateKeys(undefined, 'compressed');
          const uncompressed = ecdh.getPublicKey();
          const keylen = cryptoModule.pbkdf2Sync('password', 'salt', 1, 80, 'sha256').length;
          const sha384Length = cryptoModule.pbkdf2Sync('password', 'salt', 1, 80, 'sha384').length;
          const passwordKey = await crypto.subtle.importKey('raw', new Uint8Array([1]), 'PBKDF2', false, ['deriveBits']);
          const derivedLength = (await crypto.subtle.deriveBits({name: 'PBKDF2', salt: new Uint8Array([2]), iterations: 1, hash: 'SHA-256'}, passwordKey, 640)).byteLength;
          const sha384DerivedLength = (await crypto.subtle.deriveBits({name: 'PBKDF2', salt: new Uint8Array([2]), iterations: 1, hash: 'SHA-384'}, passwordKey, 640)).byteLength;
          let invalidIteration = false, fractionalMinFailed = false, exactRangeFailed = false;
          try { cryptoModule.pbkdf2Sync('password', 'salt', 0, 1, 'sha256'); }
          catch (e) { invalidIteration = e instanceof RangeError; }
          try { cryptoModule.randomInt(0.5, 4); } catch (e) { fractionalMinFailed = e instanceof RangeError; }
          try { cryptoModule.randomInt(0, 0x1000000000000); } catch (e) { exactRangeFailed = e instanceof RangeError; }
          const originalQueue = queueMicrotask;
          const originalScrypt = __thaw_crypto_scrypt_hex;
          const originalGenerate = __thaw_crypto_generate_key_pair_json;
          let callbackCount = 0, thrown = false, job;
          try {
            queueMicrotask = next => { job = next; };
            __thaw_crypto_scrypt_hex = () => '00';
            cryptoModule.scrypt('a', 'b', 1, () => { callbackCount++; throw new Error('callback'); });
            try { job(); } catch (e) { thrown = e.message === 'callback'; }
            __thaw_crypto_generate_key_pair_json = () => JSON.stringify({publicPem: 'p', privatePem: 'q'});
            cryptoModule.generateKeyPair('rsa', {publicKeyEncoding: {format: 'pem'}, privateKeyEncoding: {format: 'pem'}},
              () => { callbackCount++; throw new Error('callback'); });
            try { job(); } catch (e) { thrown = thrown && e.message === 'callback'; }
          } finally {
            queueMicrotask = originalQueue;
            __thaw_crypto_scrypt_hex = originalScrypt;
            __thaw_crypto_generate_key_pair_json = originalGenerate;
          }
          return [keyCopied, copyFailed, copiedHmac, sha384Hmac, typeFailed, calls, sampled, compressed.length,
            compressed[0] === (uncompressed[uncompressed.length - 1] & 1) + 2,
            keylen, sha384Length, derivedLength, sha384DerivedLength, invalidIteration, fractionalMinFailed, exactRangeFailed,
            callbackCount, thrown];
        }"#),
        1
    );
    assert_eq!(
        call("cryptoBufferRegressions", "[]"),
        "[true,true,true,true,true,2,1,33,true,80,80,80,80,true,true,true,2,true]"
    );
}

#[test]
fn native_json_graph_replacer_preserves_holder_date_alias_and_wrappers() {
    assert_eq!(load(r#"
      function inspectNativeGraph() {
        const graph = { root: { r: 0 }, nodes: [
          { o: [['left', { r: 1 }], ['right', { r: 1 }], ['map', { r: 2 }],
                ['set', { r: 3 }], ['pattern', { r: 4 }], ['__proto__', { v: 7 }]] },
          { d: 0 },
          { m: { r: 5 } },
          { s: { r: 6 } },
          { re: ['a+', 'gi', 2] },
          { a: [{ r: 7 }] },
          { a: [{ v: 3 }] },
          { a: [{ v: 'self' }, { r: 2 }] },
        ] };
        const value = __thaw_json_graph_decode(graph);
        let same = false;
        const output = __thaw_json_stringify_replacer(value, null, function (key, item) {
          if (key === 'right') same = this.left === value.right && item === '1970-01-01T00:00:00.000Z';
          if (key === 'map' || key === 'set' || key === 'pattern') return undefined;
          return item;
        });
        return [same, value.map.get('self') === value.map,
                value.set.has(3), value.pattern instanceof RegExp,
                Object.prototype.hasOwnProperty.call(value, '__proto__'), output];
      }
    "#), 1);
    assert_eq!(call("inspectNativeGraph", "[]"),
        r#"[true,true,true,true,true,"{\"left\":\"1970-01-01T00:00:00.000Z\",\"right\":\"1970-01-01T00:00:00.000Z\",\"__proto__\":7}"]"#);
}

#[test]
fn live_json_graph_encoding_does_not_read_replacer_children() {
    assert_eq!(load(r#"
      function inspectLiveGraphEncoding() {
        const previousRetain = globalThis.__thaw_retain_dynamic_value;
        let reads = 0;
        const value = { get child() { reads++; throw new Error('early getter'); } };
        try {
          globalThis.__thaw_retain_dynamic_value = () => 1;
          const graph = JSON.parse(__thaw_json_graph_encode_js([value], 0, true));
          return [reads, graph.nodes[1].hdl === 1, graph.leases.length];
        } finally {
          globalThis.__thaw_retain_dynamic_value = previousRetain;
        }
      }
    "#), 1);
    assert_eq!(call("inspectLiveGraphEncoding", "[]"), "[0,true,1]");
}

#[test]
fn graph_array_symbol_property_roundtrips_order_flags_and_cycle() {
    assert_eq!(load(r#"
      function inspectGraphArrayOwnKeys() {
        const key = Symbol('extra');
        const array = [7, ,];
        Object.defineProperty(array, '0', { value: 7, enumerable: true,
          writable: false, configurable: true });
        Object.defineProperty(array, key, { value: array, enumerable: false,
          writable: false, configurable: false });
        array.named = 'saved';
        const graph = JSON.parse(__thaw_json_graph_encode_js([array], 0, false));
        const restored = __thaw_json_graph_decode(graph)[0];
        const own = Reflect.ownKeys(restored);
        const index = Object.getOwnPropertyDescriptor(restored, '0');
        const symbol = Object.getOwnPropertyDescriptor(restored, key);
        return [graph.nodes[1].p.length, restored.length, 1 in restored,
          own.map(item => typeof item === 'symbol' ? 'symbol' : item).join(','),
          index.writable, index.configurable, symbol.value === restored,
          symbol.enumerable, symbol.configurable, restored.named];
      }
    "#), 1);
    assert_eq!(call("inspectGraphArrayOwnKeys", "[]"),
        "[4,2,false,\"0,length,named,symbol\",false,true,true,false,false,\"saved\"]");
}

#[test]
fn live_host_number_query_preserves_negative_zero_and_non_finite_values() {
    assert_eq!(load(r#"
      function inspectLiveHostNumbers() {
        return [__thaw_json_host_query(-0, 10),
                __thaw_json_host_query(NaN, 10),
                __thaw_json_host_query(Infinity, 10)];
      }
    "#), 1);
    assert_eq!(call("inspectLiveHostNumbers", "[]"), r#"["-0","NaN","Infinity"]"#);
}

// Unrun: paired native metadata is never trusted merely because its hdl is
// live. A forged nfn node must fail before exposing an ordinary JS function,
// and the graph decoder still returns the transferred hdl lease.
#[test]
fn paired_native_function_graph_rejects_unproven_handle_and_releases_lease() {
    assert_eq!(load("globalThis.pairedOrdinary = function() { return 1; };"), 1);
    let handle = thaw_js_get_global(c"pairedOrdinary".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_js_retain_handle(handle), 1);
    let source = format!(r#"
      function rejectForgedNativePair() {{
        const graph = JSON.stringify({{
          root: {{r: 0}}, nodes: [{{nfn: '1', hdl: {handle}}}],
          leases: [{handle}], napiLeases: []
        }});
        try {{ __thaw_json_graph_decode_owned(graph); return false; }}
        catch (error) {{ return error.message === 'Mismatched native Function graph node'; }}
      }}
    "#);
    assert_eq!(load(&source), 1);
    assert_eq!(call("rejectForgedNativePair", "[]"), "true");
    assert_eq!(thaw_js_release_handle(handle), 1);
    assert_eq!(thaw_js_release_handle(handle), 0);
}

// Unrun: a live JavaScript Symbol handle is not proof that an unrelated
// native Symbol ID belongs to it. The transferred lease is released even
// when the paired token is rejected before any graph object is populated.
#[test]
fn paired_native_symbol_graph_rejects_unproven_handle_and_releases_lease() {
    assert_eq!(load("globalThis.pairedOrdinarySymbol = Symbol('ordinary');"), 1);
    let handle = thaw_js_get_global(c"pairedOrdinarySymbol".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_js_retain_handle(handle), 1);
    let source = format!(r#"
      function rejectForgedNativeSymbolPair() {{
        const graph = JSON.stringify({{
          root: {{nsy: '1', hdl: {handle}}}, nodes: [],
          leases: [{handle}], napiLeases: []
        }});
        try {{ __thaw_json_graph_decode_owned(graph); return false; }}
        catch (error) {{ return error.message === 'Mismatched native Symbol graph token'; }}
      }}
    "#);
    assert_eq!(load(&source), 1);
    assert_eq!(call("rejectForgedNativeSymbolPair", "[]"), "true");
    assert_eq!(thaw_js_release_handle(handle), 1);
    assert_eq!(thaw_js_release_handle(handle), 0);
}

#[test]
fn private_graph_result_marks_real_date_separately_from_user_shape() {
    assert_eq!(load(r#"
      function graphDateResult() {
        return { real: new Date(0), user: { timestamp: 0 } };
      }
    "#), 1);
    let result = thaw_js_call_graph_result(c"graphDateResult".as_ptr(), c"{\"root\":{\"r\":0},\"nodes\":[{\"a\":[]}],\"leases\":[]}".as_ptr());
    assert!(result.error.is_null());
    let graph: serde_json::Value = serde_json::from_str(
        &unsafe { CStr::from_ptr(result.value) }.to_string_lossy(),
    ).unwrap();
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()); }
    assert_eq!(graph["nodes"][1]["d"], 0);
    assert!(graph["nodes"][2].get("o").is_some());
    assert_eq!(call("graphDateResult", "[]"),
        r#"{"real":{"timestamp":0},"user":{"timestamp":0}}"#);
}

#[test]
fn live_host_buffer_query_uses_native_buffer_identity() {
    assert_eq!(load(r#"
      function inspectLiveHostBuffer() {
        return [__thaw_json_host_query(Buffer.from([1]), 11),
                __thaw_json_host_query({ type: 'Buffer', data: [1] }, 11)];
      }
    "#), 1);
    assert_eq!(call("inspectLiveHostBuffer", "[]"), r#"["1","0"]"#);
}

#[test]
fn live_host_date_query_uses_internal_slot_and_preserves_invalid_date() {
    assert_eq!(load(r#"
      function inspectLiveHostDate() {
        return [__thaw_json_host_query(new Date(0), 12),
                __thaw_json_host_query({ timestamp: 0 }, 12),
                __thaw_json_host_query(new Date(NaN), 12),
                __thaw_json_host_query(new Date(NaN), 13),
                (() => { const p = Proxy.revocable({}, {}); p.revoke();
                  try { __thaw_json_host_query(p.proxy, 12); return 'missed'; }
                  catch (error) { return error instanceof TypeError ? 'throws' : 'wrong'; }
                })()];
      }
    "#), 1);
    assert_eq!(call("inspectLiveHostDate", "[]"), r#"["1","0","1","NaN","throws"]"#);
}

#[test]
fn live_host_date_setter_mutates_internal_slot_seen_by_alias() {
    assert_eq!(load(r#"
      function inspectLiveHostDateSetter() {
        const date = new Date(0);
        const alias = date;
        const result = __thaw_json_host_date_set(date, 2000);
        return [result, alias.getTime(), Object.hasOwn(date, 'timestamp')];
      }
    "#), 1);
    assert_eq!(call("inspectLiveHostDateSetter", "[]"), r#"["2000",2000,false]"#);
}

#[test]
fn user_date_to_json_is_standard_while_internal_wire_keeps_date_identity() {
    assert_eq!(load(r#"
      function userDateSerialization() {
        const date = new Date(0);
        const standard = JSON.stringify({ date });
        date.toJSON = () => ({ timestamp: 42 });
        const custom = JSON.stringify({ date });
        return [standard, custom];
      }
      function internalDateSerialization() {
        return { date: new Date(0) };
      }
    "#), 1);
    assert_eq!(call("userDateSerialization", "[]"),
        r#"["{\"date\":\"1970-01-01T00:00:00.000Z\"}","{\"date\":{\"timestamp\":42}}"]"#);
    assert_eq!(call("internalDateSerialization", "[]"), r#"{"date":{"timestamp":0}}"#);
}

#[test]
fn private_graph_result_preserves_custom_to_json_before_encoding() {
    assert_eq!(load(r#"
      function graphCustomToJsonResult() {
        return { item: { toJSON(key) { return { key, value: 7 }; } } };
      }
    "#), 1);
    let result = thaw_js_call_graph_result(c"graphCustomToJsonResult".as_ptr(), c"{\"root\":{\"r\":0},\"nodes\":[{\"a\":[]}],\"leases\":[]}".as_ptr());
    assert!(result.error.is_null());
    let graph: serde_json::Value = serde_json::from_str(
        &unsafe { CStr::from_ptr(result.value) }.to_string_lossy(),
    ).unwrap();
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()); }
    assert_eq!(graph["nodes"][1]["o"][0][0], "key");
    assert_eq!(graph["nodes"][1]["o"][0][1]["v"], "item");
    assert_eq!(graph["nodes"][1]["o"][1][1]["v"], 7);
}

#[test]
fn private_graph_arguments_do_not_revive_user_marker_shapes() {
    assert_eq!(load(r#"
      function inspectGraphArguments(real, user, absent, fake) {
        return [real instanceof Date, user instanceof Date,
                absent === undefined, fake === undefined,
                user.timestamp, fake.$__thaw_napi_undefined$];
      }
    "#), 1);
    let graph = c"{\"root\":{\"r\":0},\"nodes\":[{\"a\":[{\"r\":1},{\"r\":2},{\"u\":1},{\"r\":3}]},{\"d\":0},{\"o\":[[\"timestamp\",{\"v\":0}]]},{\"o\":[[\"$__thaw_napi_undefined$\",{\"v\":true}]]}],\"leases\":[]}";
    let result = thaw_js_call_graph_result(c"inspectGraphArguments".as_ptr(), graph.as_ptr());
    assert!(result.error.is_null());
    let result: serde_json::Value = serde_json::from_str(
        &unsafe { CStr::from_ptr(result.value) }.to_string_lossy(),
    ).unwrap();
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()); }
    assert_eq!(result["nodes"][0]["a"], serde_json::json!([
        {"v": true}, {"v": false}, {"v": true}, {"v": false}, {"v": 0}, {"v": true}
    ]));
}

#[test]
fn private_graph_arguments_release_input_leases_before_target_lookup_errors() {
    assert_eq!(load("globalThis.leaseInput = {}; globalThis.nonCallableLeaseTarget = 1;"), 1);
    for target in ["missingLeaseTarget", "nonCallableLeaseTarget"] {
        let input = thaw_js_get_global(c"leaseInput".as_ptr());
        assert_eq!(thaw_js_retain_handle(input), 1); // one transferred graph lease
        let graph = CString::new(format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{input}}}],"leases":[{input}]}}"#
        )).unwrap();
        let result = thaw_js_call_graph_result(CString::new(target).unwrap().as_ptr(), graph.as_ptr());
        assert!(!result.error.is_null());
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()); }
        // The ABI entry owns the transferred retain even when the callee is
        // missing or not callable, before the graph decoder is reached.
        assert_eq!(thaw_js_release_handle(input), 1);
        assert_eq!(thaw_js_release_handle(input), 0);
    }
}

#[test]
fn private_graph_property_setter_runs_before_result_encoding() {
    assert_eq!(load(r#"
        globalThis.graphAssignmentOrder = '';
        globalThis.graphAssignmentTarget = {};
        Object.defineProperty(graphAssignmentTarget, 'field', {
          set(value) { graphAssignmentOrder += 'S'; throw new Error('setter failure'); }
        });
        globalThis.graphAssignmentValue = {
          toJSON() { graphAssignmentOrder += 'J'; return { value: 1 }; }
        };
        globalThis.getGraphAssignmentOrder = () => graphAssignmentOrder;
    "#), 1);
    let target = thaw_js_get_global(c"graphAssignmentTarget".as_ptr());
    let value = thaw_js_get_global(c"graphAssignmentValue".as_ptr());
    assert_eq!(thaw_js_retain_handle(value), 1);
    let graph = CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{value}}}],"leases":[{value}]}}"#
    )).unwrap();
    let result = thaw_js_set_property_json_graph_result(target, c"field".as_ptr(), graph.as_ptr());
    assert!(!result.error.is_null());
    unsafe { thaw_arena::destroy_string(result.error.cast_mut()); }
    assert_eq!(call("getGraphAssignmentOrder", "[]"), "\"S\"");
    assert_eq!(thaw_js_release_handle(value), 1);
    assert_eq!(thaw_js_release_handle(value), 0);
    assert_eq!(thaw_js_release_handle(target), 1);
}

#[test]
fn private_graph_arguments_release_input_leases_after_success() {
    assert_eq!(load("globalThis.leaseInput = {}; globalThis.acceptLeaseInput = value => value === leaseInput;"), 1);
    let input = thaw_js_get_global(c"leaseInput".as_ptr());
    assert_eq!(thaw_js_retain_handle(input), 1);
    let graph = CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{input}}}],"leases":[{input}]}}"#
    )).unwrap();
    let result = thaw_js_call_graph_result(c"acceptLeaseInput".as_ptr(), graph.as_ptr());
    assert!(result.error.is_null());
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()); }
    assert_eq!(thaw_js_release_handle(input), 1);
    assert_eq!(thaw_js_release_handle(input), 0);
}


#[test]
fn private_bigint_graph_token_requires_full_decimal_consumption() {
    assert_eq!(load(r#"
      function inspectBigIntGraphToken() {
        const decode = text => __thaw_json_graph_decode({ root: { bi: text }, nodes: [] });
        const exact = String(decode('9007199254740993'));
        const invalid = ['1\n', '1\r', '1\u2028', '01', '-0', '+1'];
        return [exact, ...invalid.map(text => {
          try { decode(text); return false; }
          catch (error) { return error instanceof TypeError; }
        })];
      }
    "#), 1);
    assert_eq!(call("inspectBigIntGraphToken", "[]"),
        r#"["9007199254740993",true,true,true,true,true,true]"#);
}

#[test]
fn dynamic_property_receiver_rejects_null_and_undefined_but_boxes_strings() {
    let null = CString::new("null").unwrap();
    let null_handle = thaw_js_retain_json_result(null.as_ptr());
    assert!(null_handle.error.is_null());
    let property = CString::new("length").unwrap();
    let null_result = thaw_js_get_property_result(null_handle.value, property.as_ptr());
    assert_eq!(null_result.value, 0);
    assert!(!null_result.error.is_null());
    assert!(unsafe { CStr::from_ptr(null_result.error) }.to_string_lossy().starts_with("\u{1}TypeError\u{1}"));
    let key_json = CString::new("\"length\"").unwrap();
    let json_key_result = thaw_js_get_property_json_key_result(null_handle.value, key_json.as_ptr());
    assert!(!json_key_result.error.is_null());
    assert!(unsafe { CStr::from_ptr(json_key_result.error) }.to_string_lossy().starts_with("\u{1}TypeError\u{1}"));

    assert_eq!(load("globalThis.__thaw_undefined_receiver = () => undefined"), 1);
    let function = CString::new("__thaw_undefined_receiver").unwrap();
    let function_handle = thaw_js_get_global(function.as_ptr());
    let empty = CString::new("[]").unwrap();
    let undefined_handle = thaw_js_call_handle_handle_result(function_handle, empty.as_ptr(), true);
    assert!(undefined_handle.error.is_null());
    let method = CString::new("toString").unwrap();
    let undefined_result = thaw_js_call_method_result(undefined_handle.value, method.as_ptr(), empty.as_ptr());
    assert!(!undefined_result.error.is_null());
    assert!(unsafe { CStr::from_ptr(undefined_result.error) }.to_string_lossy().starts_with("\u{1}TypeError\u{1}"));

    let text = CString::new("\"abc\"").unwrap();
    let string_handle = thaw_js_retain_json_result(text.as_ptr());
    let length = thaw_js_get_property_result(string_handle.value, property.as_ptr());
    assert!(length.error.is_null());
    assert_eq!(load("globalThis.__thaw_is_three = value => value === 3"), 1);
    let predicate = CString::new("__thaw_is_three").unwrap();
    let predicate_handle = thaw_js_get_global(predicate.as_ptr());
    let checked = thaw_js_call_handle_value_result(predicate_handle, length.value);
    assert!(checked.error.is_null());
    assert_eq!(unsafe { CStr::from_ptr(checked.value) }.to_str().unwrap(), "true");
    unsafe {
        thaw_arena::destroy_string(null_result.error.cast_mut());
        thaw_arena::destroy_string(json_key_result.error.cast_mut());
        thaw_arena::destroy_string(undefined_result.error.cast_mut());
        thaw_arena::destroy_string(checked.value.cast_mut());
    }
    assert_eq!(thaw_js_release_handle(null_handle.value), 1);
    assert_eq!(thaw_js_release_handle(function_handle), 1);
    assert_eq!(thaw_js_release_handle(undefined_handle.value), 1);
    assert_eq!(thaw_js_release_handle(string_handle.value), 1);
    assert_eq!(thaw_js_release_handle(length.value), 1);
    assert_eq!(thaw_js_release_handle(predicate_handle), 1);
}

#[test]
fn dynamic_reflect_predicates_preserve_thrown_error_class() {
    assert_eq!(load("globalThis.__thaw_predicate_receiver = new Proxy({}, { has() { const error = new TypeError('has marker'); error.code = 'HAS_TRAP'; throw error; }, deleteProperty() { throw new RangeError('delete marker'); } })"), 1);
    let name = CString::new("__thaw_predicate_receiver").unwrap();
    let handle = thaw_js_get_global(name.as_ptr());
    let key = CString::new("key").unwrap();
    let has = thaw_js_has_property_result(handle, key.as_ptr());
    assert_eq!(has.value, 0);
    assert!(!has.error.is_null());
    let has_error = unsafe { CStr::from_ptr(has.error) }.to_string_lossy();
    assert!(has_error.starts_with("\u{1}TypeError\u{1}\u{1e}E1:"));
    assert!(has_error.contains("\u{5}{\"code\":\"HAS_TRAP\"}"));
    let delete = thaw_js_delete_property_result(handle, key.as_ptr());
    assert_eq!(delete.value, 0);
    assert!(!delete.error.is_null());
    assert!(unsafe { CStr::from_ptr(delete.error) }.to_string_lossy().starts_with("\u{1}RangeError\u{1}\u{1e}E1:"));
    unsafe {
        thaw_arena::destroy_string(has.error.cast_mut());
        thaw_arena::destroy_string(delete.error.cast_mut());
    }
    assert_eq!(thaw_js_release_handle(handle), 1);
}

#[test]
fn process_report_result_preserves_listener_error_and_ordered_event_identity() {
    assert_eq!(load("process.on('uncaughtException', globalThis.__thawTestUncaught = function() { const e = new TypeError('listener'); e.code = 'E_LISTENER'; throw e; }); process.on('unhandledRejection', globalThis.__thawTestRejection = function() { throw new RangeError('rejection listener'); }); process.on('rejectionHandled', globalThis.__thawTestHandled = function() { throw new Error('handled listener'); });"), 1);
    let uncaught = thaw_js_emit_uncaught_result(c"original".as_ptr());
    assert_eq!(uncaught.value, 0);
    assert!(!uncaught.error.is_null());
    let error = unsafe { CStr::from_ptr(uncaught.error) }.to_string_lossy();
    assert!(error.starts_with("\u{1}TypeError\u{1}\u{1e}E1:"), "{error}");
    assert!(error.contains("E_LISTENER"), "{error}");
    unsafe { thaw_arena::destroy_string(uncaught.error.cast_mut()) };

    let rejection = thaw_js_emit_unhandled_rejection_result(c"original rejection".as_ptr());
    assert_eq!(rejection.value, 0);
    assert!(!rejection.error.is_null());
    let error = unsafe { CStr::from_ptr(rejection.error) }.to_string_lossy();
    assert!(error.starts_with("\u{1}RangeError\u{1}\u{1e}E1:"), "{error}");
    unsafe { thaw_arena::destroy_string(rejection.error.cast_mut()) };

    let handled = thaw_js_emit_rejection_handled_result();
    assert!(!handled.error.is_null());
    let error = unsafe { CStr::from_ptr(handled.error) }.to_string_lossy();
    assert!(error.contains("handled listener"), "{error}");
    unsafe { thaw_arena::destroy_string(handled.error.cast_mut()) };
    assert_eq!(load("process.off('uncaughtException', globalThis.__thawTestUncaught); process.off('unhandledRejection', globalThis.__thawTestRejection); process.off('rejectionHandled', globalThis.__thawTestHandled); delete globalThis.__thawTestUncaught; delete globalThis.__thawTestRejection; delete globalThis.__thawTestHandled;"), 1);
}

#[test]
fn process_report_result_keeps_embedded_nul_from_thrown_listener() {
    assert_eq!(load("process.on('unhandledRejection', globalThis.__thawNulListener = () => { const error = new TypeError('left\\u0000right'); error.code = 'ERR_NUL'; throw error; });"), 1);
    let result = thaw_js_emit_unhandled_rejection_result(c"original".as_ptr());
    assert!(!result.error.is_null());
    let bytes = unsafe { CStr::from_ptr(result.error) }.to_bytes();
    assert!(bytes.starts_with(b"\x01TypeError\x01"), "{bytes:?}");
    assert!(bytes.windows(b"left\0right".len()).any(|part| part == b"left\0right"), "{bytes:?}");
    assert!(bytes.windows(b"ERR_NUL".len()).any(|part| part == b"ERR_NUL"), "{bytes:?}");
    unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
    assert_eq!(load("process.off('unhandledRejection', globalThis.__thawNulListener); delete globalThis.__thawNulListener;"), 1);
}

#[test]
fn uncaught_result_reads_owned_native_text_past_embedded_nul() {
    assert_eq!(load("globalThis.nativeNulError = ''; process.once('uncaughtException', error => { nativeNulError = error.message; });"), 1);
    let message = thaw_arena::owned_string(b"left\0right");
    let result = thaw_js_emit_uncaught_result(message);
    unsafe { thaw_arena::destroy_string(message) };
    assert_eq!(result.value, 1);
    assert!(result.error.is_null());
    assert_eq!(eval_json("nativeNulError"), Ok(Some("\"left\\u0000right\"".into())));
}

#[cfg(feature = "tls")]
#[test]
fn tls_client_and_accepted_server_handles_remain_independent() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    let dir = std::env::temp_dir().join(format!(
        "thaw_tls_handle_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let cert_pem = dir.join("cert.pem");
    let key_pem = dir.join("key.pem");
    let cert_der_path = dir.join("cert.der");
    let key_der_path = dir.join("key.der");
    assert!(Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:FALSE", "-addext", "keyUsage=critical,digitalSignature,keyEncipherment", "-addext", "extendedKeyUsage=serverAuth", "-keyout"])
        .arg(&key_pem).arg("-out").arg(&cert_pem).output().unwrap().status.success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"]).arg(&cert_pem)
        .args(["-outform", "DER", "-out"]).arg(&cert_der_path)
        .status().unwrap().success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"]).arg(&key_pem)
        .args(["-outform", "DER", "-out"]).arg(&key_der_path)
        .status().unwrap().success());
    let cert_der = std::fs::read(&cert_der_path).unwrap();
    let key_der = std::fs::read(&key_der_path).unwrap();
    let cert_hex = hex_encode(&cert_der);
    let key_hex = hex_encode(&key_der);
    let certificate = CertificateDer::from(cert_der.clone());
    let mut external_server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], PrivatePkcs8KeyDer::from(key_der.clone()).into())
        .unwrap();
    external_server_config.alpn_protocols = vec![b"h2".to_vec()];
    let external_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let external_port = external_listener.local_addr().unwrap().port();
    let external_server = std::thread::spawn(move || {
        let (socket, _) = external_listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(external_server_config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut received = [0];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"C");
        stream.write_all(b"X").unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });
    let client_result = tls_connect(TlsClientOptions {
        host: "127.0.0.1", port: external_port, server_name: "localhost",
        ca_spec: &cert_hex, cert_spec: "", key_spec: "", alpn_spec: "6832",
        report_alpn: true, reject_unauthorized: true,
    });
    assert!(client_result.starts_with("ok:"), "{client_result}");
    let client_handle: u32 = client_result.split(':').nth(1).unwrap().parse().unwrap();

    let listener_result = tls_server_listen(TlsServerOptions {
        host: "127.0.0.1", port: 0, cert_spec: &cert_hex, key_spec: &key_hex,
        ca_spec: "", request_cert: false, reject_unauthorized: true, alpn_spec: "687474702f312e31",
    });
    assert!(listener_result.starts_with("ok:"), "{listener_result}");
    let listener_handle: u32 = listener_result.split(':').nth(1).unwrap().parse().unwrap();
    let listener_port: u16 = listener_result.split(':').nth(2).unwrap().parse().unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let mut external_client_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots).with_no_client_auth();
    external_client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let external_client = std::thread::spawn(move || {
        let socket = TcpStream::connect(("127.0.0.1", listener_port)).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let connection = rustls::ClientConnection::new(
            Arc::new(external_client_config), ServerName::try_from("localhost".to_string()).unwrap()
        ).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut received = [0];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"S");
        stream.write_all(b"Y").unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });
    let accepted = tls_server_accept(listener_handle);
    assert!(accepted.starts_with("ok:"), "{accepted}");
    let server_handle: u32 = accepted.split(':').nth(1).unwrap().parse().unwrap();
    assert!(client_handle > 0 && server_handle > 0 && client_handle != server_handle);
    assert_eq!(client_handle % 2, 1);
    assert_eq!(server_handle % 2, 0);
    assert_eq!(tls_alpn(client_handle), "6832");
    assert_eq!(tls_alpn(server_handle), "687474702f312e31");
    assert_eq!(tls_certificate(client_handle, true), cert_hex);
    assert_eq!(tls_certificate(server_handle, false), cert_hex);
    assert_eq!(tls_certificate(server_handle, true), "");
    assert_eq!(tls_write(client_handle, b"C"), "ok");
    assert_eq!(tls_write(server_handle, b"S"), "ok");
    fn read_byte(handle: u32) -> String {
        for _ in 0..1000 {
            let result = tls_poll_read(handle);
            if result != "pending" { return result; }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("TLS read timed out");
    }
    assert_eq!(read_byte(client_handle), "ok:58");
    assert_eq!(read_byte(server_handle), "ok:59");
    tls_destroy(client_handle);
    assert_eq!(tls_certificate(server_handle, false), cert_hex);
    assert_eq!(tls_alpn(server_handle), "687474702f312e31");
    tls_destroy(server_handle);
    tls_server_close_listener(listener_handle);
    external_server.join().unwrap();
    external_client.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();

    let mut last_client = u32::MAX - 2;
    assert_eq!(take_tls_stream_handle(&mut last_client), Some(u32::MAX - 2));
    assert_eq!(take_tls_stream_handle(&mut last_client), None);
    assert_eq!(last_client, u32::MAX);
    let mut last_server = u32::MAX - 3;
    assert_eq!(take_tls_stream_handle(&mut last_server), Some(u32::MAX - 3));
    assert_eq!(take_tls_stream_handle(&mut last_server), None);
    assert_eq!(last_server, u32::MAX - 1);
}

#[test]
fn worker_transport_codec_preserves_sparse_array_slots_and_graph_references() {
    assert_eq!(
        load(r#"function sparseWorkerTransport() {
            const sparse = new Array(6);
            sparse[1] = undefined;
            sparse[3] = sparse;
            const source = { sparse, map: new Map([['value', sparse]]), set: new Set([sparse]) };
            const copy = __thaw_worker_decode(__thaw_worker_encode(source));
            const empty = __thaw_worker_decode(__thaw_worker_encode(new Array(3)));
            return [copy.sparse.length, 0 in copy.sparse, 1 in copy.sparse,
                2 in copy.sparse, 3 in copy.sparse, 4 in copy.sparse, 5 in copy.sparse,
                copy.sparse[1] === undefined, copy.sparse[3] === copy.sparse,
                copy.map.get('value') === copy.sparse, copy.set.has(copy.sparse),
                empty.length, Object.keys(empty).length];
        }"#),
        1
    );
    assert_eq!(
        call("sparseWorkerTransport", "[]"),
        "[6,false,true,false,true,false,false,true,true,true,true,3,0]"
    );
}

#[test]
fn event_target_skips_listeners_removed_during_dispatch() {
    assert_eq!(
        load(r#"function removedDuringDispatch() {
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
        }"#),
        1
    );
    assert_eq!(
        call("removedDuringDispatch", "[]"),
        r#"["a","a","a,a,b,late",1,"first,second,first"]"#
    );
}

#[test]
fn atomics_notify_nonshared_waitable_arrays_delegate_to_native() {
    assert_eq!(
        load(r#"function notifyNonsharedWaitable() {
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
        }"#),
        1
    );
    assert_eq!(
        call("notifyNonsharedWaitable", "[]"),
        r#"[0,0,1,true,true,true,0,"not-equal"]"#
    );
}

#[test]
fn native_callback_cross_mode_identity_survives_released_handle() {
    unsafe extern "C" fn callback(
        _context: *const c_void,
        _arguments: *const c_char,
    ) -> *const c_char {
        c"null".as_ptr()
    }
    let first_context = &1u8 as *const u8 as *const c_void;
    let other_context = &2u8 as *const u8 as *const c_void;
    let ordinary = thaw_js_register_native_callback(
        callback as *const c_void, first_context, 0, 0, 0,
        std::ptr::null(), 0,
    );
    let graph = thaw_js_register_native_callback_graph(
        callback as *const c_void, first_context, 0, 0, 0,
        std::ptr::null(), 0,
    );
    let unrelated = thaw_js_register_native_callback(
        callback as *const c_void, other_context, 0, 0, 0,
        std::ptr::null(), 0,
    );
    assert!(ordinary.error.is_null() && graph.error.is_null() && unrelated.error.is_null());
    assert_ne!(ordinary.value, graph.value);
    with_context(|ctx| {
        let first = value_for_handle(&ctx, ordinary.value).unwrap();
        let graph_value = value_for_handle(&ctx, graph.value).unwrap();
        let other = value_for_handle(&ctx, unrelated.value).unwrap();
        let query: Function = ctx.globals().get("__thaw_same_native_callback").unwrap();
        assert!(query.call::<_, bool>((first.clone(), graph_value.clone())).unwrap());
        assert!(query.call::<_, bool>((graph_value.clone(), first.clone())).unwrap());
        let strict: Function = ctx.globals().get("__thaw_strict_equal_dynamic").unwrap();
        let loose: Function = ctx.globals().get("__thaw_native_loose_equal").unwrap();
        assert!(strict.call::<_, bool>((first.clone(), graph_value.clone(), 3.0, 0.0)).unwrap());
        assert!(loose.call::<_, bool>((graph_value.clone(), first.clone(), 0.0, 3.0)).unwrap());
        assert!(!query.call::<_, bool>((first.clone(), other)).unwrap());
        let holder = Object::new(ctx.clone()).unwrap();
        holder.set("saved", graph_value).unwrap();
        ctx.globals().set("__thaw_saved_callback_holder", holder).unwrap();
    });
    assert_eq!(thaw_js_release_handle(graph.value), 1);
    let replacement = thaw_js_register_native_callback_graph(
        callback as *const c_void, other_context, 0, 0, 0,
        std::ptr::null(), 0,
    );
    assert!(replacement.error.is_null());
    // The freed numeric slot can be reused for another closure; the saved
    // JS function retains its original private identity independently.
    assert_eq!(replacement.value, graph.value);
    with_context(|ctx| {
        let first = value_for_handle(&ctx, ordinary.value).unwrap();
        let replacement_value = value_for_handle(&ctx, replacement.value).unwrap();
        let holder: Object = ctx.globals().get("__thaw_saved_callback_holder").unwrap();
        let saved: Value = holder.get("saved").unwrap();
        let reacquired = retain_value(&ctx, saved.clone()).unwrap();
        let query: Function = ctx.globals().get("__thaw_same_native_callback").unwrap();
        assert!(query.call::<_, bool>((first.clone(), saved.clone())).unwrap());
        assert!(!query.call::<_, bool>((first, replacement_value)).unwrap());
        assert!(query.call::<_, bool>((saved.clone(), value_for_handle(&ctx, reacquired).unwrap())).unwrap());
        assert!(!query.call::<_, bool>((saved.clone(), Value::new_undefined(ctx.clone()))).unwrap());
        let descriptor: Object = ctx.globals().get::<_, Object>("Object").unwrap()
            .get::<_, Function>("getOwnPropertyDescriptor").unwrap()
            .call((ctx.globals(), "__thaw_same_native_callback")).unwrap();
        assert!(!descriptor.get::<_, bool>("writable").unwrap());
        assert!(!descriptor.get::<_, bool>("configurable").unwrap());
        ctx.globals().set("__thaw_saved_callback_holder", Value::new_undefined(ctx)).unwrap();
    });
}

#[test]
fn result_error_abi_keeps_embedded_nul_in_throw_rejection_and_getter() {
    assert_eq!(load(r#"
        function throwNulResult() {
            const error = new TypeError('left\u0000right');
            error.code = 'ERR_NUL';
            error.status = 418;
            throw error;
        }
        function rejectNulResult() {
            return Promise.reject(new Error('reject\u0000body'));
        }
        globalThis.nulGetterResult = {
            get value() { throw new TypeError('getter\u0000body'); }
        };
    "#), 1);
    let thrown = thaw_js_call_result(c"throwNulResult".as_ptr(), c"[]".as_ptr());
    assert!(thrown.value.is_null());
    let bytes = unsafe { CStr::from_ptr(thrown.error) }.to_bytes();
    let frame = thaw_arena::error_wire::parse_tagged(bytes).unwrap();
    assert_eq!(frame.chain, b"TypeError");
    assert!(frame.display.windows(b"left\0right".len()).any(|part| part == b"left\0right"));
    assert_eq!(frame.original, None);
    assert!(frame.suffix.windows(b"ERR_NUL".len()).any(|part| part == b"ERR_NUL"));
    assert!(frame.suffix.windows(b"418".len()).any(|part| part == b"418"));
    unsafe { thaw_arena::destroy_string(thrown.error.cast_mut()) };

    let rejected = thaw_js_call_result(c"rejectNulResult".as_ptr(), c"[]".as_ptr());
    assert!(rejected.value.is_null());
    let bytes = unsafe { CStr::from_ptr(rejected.error) }.to_bytes();
    assert!(bytes.windows(b"reject\0body".len()).any(|part| part == b"reject\0body"));
    unsafe { thaw_arena::destroy_string(rejected.error.cast_mut()) };

    let handle = thaw_js_get_global(c"nulGetterResult".as_ptr());
    let getter = thaw_js_get_property_result(handle, c"value".as_ptr());
    assert_eq!(getter.value, 0);
    let bytes = unsafe { CStr::from_ptr(getter.error) }.to_bytes();
    assert!(bytes.windows(b"getter\0body".len()).any(|part| part == b"getter\0body"));
    unsafe { thaw_arena::destroy_string(getter.error.cast_mut()) };
    assert_eq!(thaw_js_release_handle(handle), 1);
}

#[test]
fn graph_replacer_uses_its_holder_without_reading_callable_call_property() {
    assert_eq!(load(r#"
      function inspectGraphReplacerCallProperty() {
        const value = { item: 3 };
        let callReads = 0;
        function replacer(key, item) {
          if (key === 'item' && this !== value) throw new Error('wrong holder');
          return item;
        }
        Object.defineProperty(replacer, 'call', {
          get() { callReads++; throw new Error('replacer.call was read'); }
        });
        const output = __thaw_json_stringify_replacer(value, null, replacer);
        return [output, callReads];
      }
    "#), 1);
    assert_eq!(call("inspectGraphReplacerCallProperty", "[]"),
        r#"["{\"item\":3}",0]"#);
}


#[test]
fn tagged_error_reconstruction_reads_versioned_and_legacy_ancestry_names() {
    assert_eq!(load(r#"
      function inspectAncestryNames() {
        const versioned = __thaw_error_from_tagged(
          '\u0001\u001eLeaf\u001fBase$Name\u001fError\u0001failure');
        const legacy = __thaw_error_from_tagged('\u0001Leaf$Base$Error\u0001failure');
        return [versioned.name, versioned.message, legacy.name, legacy.message];
      }
    "#), 1);
    assert_eq!(call("inspectAncestryNames", "[]"),
        r#"["Leaf","failure","Leaf","failure"]"#);
}


#[test]
fn typed_callback_origin_registration_returns_the_original_js_function() {
    std::thread::spawn(|| {
        unsafe extern "C" {
            fn thaw_json_register_callback_origin(closure: *const u8, handle: u64) -> u8;
        }
        unsafe extern "C" fn callback(
            _context: *const c_void, _arguments: *const c_char,
        ) -> *const c_char { c"null".as_ptr() }
        assert_eq!(load("globalThis.__thaw_origin_test = function () { return 42; }"), 1);
        let original = with_context(|ctx| {
            let value: Value = ctx.globals().get("__thaw_origin_test").unwrap();
            retain_value(&ctx, value).unwrap()
        });
        let closure = thaw_arena::thaw_arena_alloc(24, 8);
        assert!(!closure.is_null());
        assert_eq!(unsafe { thaw_json_register_callback_origin(closure, original) }, 1);
        let restored = thaw_js_register_native_callback_graph(
            callback as *const c_void, closure.cast(), 0, 0, 0,
            std::ptr::null(), 0,
        );
        assert!(restored.error.is_null());
        with_context(|ctx| {
            let strict: Function = ctx.globals().get("__thaw_strict_equal_dynamic").unwrap();
            let left = value_for_handle(&ctx, original).unwrap();
            let right = value_for_handle(&ctx, restored.value).unwrap();
            assert!(strict.call::<_, bool>((left, right, 3.0, 3.0)).unwrap());
        });
        assert_eq!(thaw_js_release_handle(restored.value), 1);
        with_context(|ctx| assert!(value_for_handle(&ctx, original).is_ok()));
    }).join().unwrap();
}


#[test]
fn callback_origin_type_check_ignores_mutable_host_query_global() {
    std::thread::spawn(|| {
        assert_eq!(load(r#"
            globalThis.__thaw_origin_object = {};
            globalThis.__thaw_origin_callable = new Proxy(function () {}, {});
            globalThis.__thaw_json_host_query = () => '1';
        "#), 1);
        let (object, callable) = with_context(|ctx| {
            let object: Value = ctx.globals().get("__thaw_origin_object").unwrap();
            let callable: Value = ctx.globals().get("__thaw_origin_callable").unwrap();
            (retain_value(&ctx, object).unwrap(), retain_value(&ctx, callable).unwrap())
        });
        let spoofed = thaw_js_host_query_result(object, 0);
        assert!(spoofed.error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(spoofed.value) }.to_string_lossy(), "1");
        unsafe { thaw_arena::destroy_string(spoofed.value.cast_mut()) };
        let rejected = thaw_js_host_query_result(object, 14);
        let accepted = thaw_js_host_query_result(callable, 14);
        assert!(rejected.error.is_null() && accepted.error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(rejected.value) }.to_string_lossy(), "0");
        assert_eq!(unsafe { CStr::from_ptr(accepted.value) }.to_string_lossy(), "1");
        unsafe {
            thaw_arena::destroy_string(rejected.value.cast_mut());
            thaw_arena::destroy_string(accepted.value.cast_mut());
        }
        assert_eq!(thaw_js_release_handle(object), 1);
        assert_eq!(thaw_js_release_handle(callable), 1);
    }).join().unwrap();
}


#[cfg(feature = "intl")]
#[test]
fn intl_datetime_resolved_hour_cycle_matches_locale_and_midnight() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("cycles", "[]"), serde_json::json!([
        ["h12", true, "12"], ["h23", false, "00"], ["h23", false, "00"],
        ["h12", true, "12"], ["h24", false, "24"]
    ]).to_string());
    assert_eq!(call("localizedMidnight", "[]"), serde_json::json!(["h24", "٢٤"]).to_string());
}

// Unrun: the live-key predicate must use its bootstrap parser and Reflect
// intrinsics even after a script replaces the public JSON/Object/Reflect API.
#[test]
fn live_host_key_predicate_uses_captured_intrinsics() {
    assert_eq!(load(r#"
        globalThis.nativeKeyTarget = Object.create({ inherited: 1 });
        nativeKeyTarget.own = 2;
        globalThis.nativeKeySymbol = Symbol('live');
        nativeKeyTarget[nativeKeySymbol] = 3;
        globalThis.savedKeyIntrinsics = [JSON.parse, Reflect.has,
            Reflect.deleteProperty, Object.hasOwn];
        JSON.parse = () => 'wrong';
        Reflect.has = () => false;
        Reflect.deleteProperty = () => false;
        Object.hasOwn = () => false;
        try {
            Object.defineProperty(globalThis, '__thaw_json_host_key_predicate',
                { value: () => false });
        } catch (_) {}
    "#), 1);
    let object = thaw_js_get_global(c"nativeKeyTarget".as_ptr());
    assert_ne!(object, 0);
    for (key, operation, expected) in [
        (c"\"inherited\"".as_ptr(), 0, 1),
        (c"\"inherited\"".as_ptr(), 1, 0),
        (c"\"own\"".as_ptr(), 1, 1),
        (c"\"own\"".as_ptr(), 3, 1),
        (c"\"own\"".as_ptr(), 2, 1),
        (c"\"own\"".as_ptr(), 1, 0),
    ] {
        let result = thaw_js_property_predicate_json_key_result(object, key, operation);
        assert!(result.error.is_null());
        assert_eq!(result.value, expected);
    }
    let symbol = thaw_js_get_global(c"nativeKeySymbol".as_ptr());
    assert_ne!(symbol, 0);
    for (operation, expected) in [(1, 1), (2, 1), (1, 0)] {
        let result = thaw_js_property_predicate_key_handle_result(object, symbol, operation);
        assert!(result.error.is_null());
        assert_eq!(result.value, expected);
    }
    assert_eq!(load(r#"
        [JSON.parse, Reflect.has, Reflect.deleteProperty, Object.hasOwn] =
            savedKeyIntrinsics;
        delete globalThis.savedKeyIntrinsics;
        delete globalThis.nativeKeyTarget;
        delete globalThis.nativeKeySymbol;
    "#), 1);
    assert_eq!(thaw_js_release_handle(symbol), 1);
    assert_eq!(thaw_js_release_handle(object), 1);
}

#[test]
fn private_iterator_helper_keeps_receiver_and_reads_getters_once() {
    assert_eq!(load(r#"
      globalThis.iteratorIntrinsicObserved = [];
      const source = {
        get [Symbol.iterator]() {
          iteratorIntrinsicObserved.push('iterator');
          return function () {
            iteratorIntrinsicObserved.push(this === source ? 'receiver' : 'wrong');
            return {
              get next() {
                iteratorIntrinsicObserved.push('next');
                return () => ({ value: 7, done: false });
              },
              get return() {
                iteratorIntrinsicObserved.push('return');
                return () => ({ done: true });
              },
            };
          };
        },
      };
      const descriptor = Object.getOwnPropertyDescriptor(globalThis, '__thaw_to_iterator');
      const replaced = Reflect.set(globalThis, '__thaw_to_iterator', () => 0);
      const iterator = descriptor.value(source);
      iterator.next();
      iterator.return();
      globalThis.iteratorIntrinsicControl = descriptor.enumerable
        && !descriptor.writable && !descriptor.configurable && !replaced
        && globalThis.__thaw_to_iterator === descriptor.value
        && iteratorIntrinsicObserved.join(',') === 'iterator,receiver,next,return';
    "#), 1);
    assert_eq!(eval_json("iteratorIntrinsicControl"), Ok(Some("true".into())));
    assert_eq!(load("delete globalThis.iteratorIntrinsicObserved; delete globalThis.iteratorIntrinsicControl;"), 1);
}

#[test]
fn response_headers_preserve_immutable_guard_on_clone() {
    assert_eq!(load(r#"
      function responseHeadersGuard() {
        const response = new Response(null, {headers: {'x-test': 'original'}});
        __thaw_set_response_metadata(response, 'https://example.test/', false);
        const guarded = [response, response.clone(), Response.error(), Response.redirect('https://example.test/')];
        const rejected = guarded.map(value => ['append', 'set', 'delete'].every(method => {
          try { value.headers[method]('x-test', 'changed'); return false; }
          catch (error) { return error instanceof TypeError; }
        }));
        const copy = new Headers(response.headers), ordinary = new Response();
        copy.set('x-test', 'copy'); ordinary.headers.set('x-test', 'ordinary');
        return [rejected, response.headers.get('x-test'), copy.get('x-test'), ordinary.headers.get('x-test')];
      }
    "#), 1);
    assert_eq!(call("responseHeadersGuard", "[]"),
        r#"[[true,true,true,true],"original","copy","ordinary"]"#);
}

#[test]
fn request_body_source_survives_copy_and_clone() {
    assert_eq!(load(r#"
      function requestBodySource() {
        const regular = new Request('https://example.test/', {method: 'POST', body: 'payload'});
        const clone = regular.clone(), copied = new Request(regular);
        const streamed = new Request('https://example.test/', {method: 'POST', body: new ReadableStream(), duplex: 'half'});
        const streamClone = streamed.clone(), streamCopy = new Request(streamed);
        return [clone, copied, streamClone, streamCopy, new Request('https://example.test/')].map(__thaw_request_body_replayable);
      }
    "#), 1);
    assert_eq!(call("requestBodySource", "[]"), "[true,true,false,false,true]");
}

// Unrun regression: aliases and table reads share one exported function identity.
#[test]
fn webassembly_export_aliases_share_function_identity() {
    assert_eq!(load(r#"
        function wasmExportIdentity() {
            const bytes = new TextEncoder().encode(`(module
                (func $f (result i32) i32.const 42)
                (export "a" (func $f)) (export "b" (func $f))
                (table (export "table") 1 funcref)
                (elem (i32.const 0) $f))`);
            const module = new WebAssembly.Module(bytes);
            const first = new WebAssembly.Instance(module);
            const second = new WebAssembly.Instance(module);
            const forwarding = new WebAssembly.Module(new TextEncoder().encode(`(module
                (import "host" "f" (func $f (result i32)))
                (export "a" (func $f)) (export "b" (func $f))
                (table (export "table") 1 funcref)
                (elem (i32.const 0) $f))`));
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
    "#), 1);
    assert_eq!(call("wasmExportIdentity", "[]"), "[true,true,true,42,true,true,true,true,42,42]");
}

// Unrun regression: the same scalar conversion governs local and exported globals.
#[test]
fn webassembly_global_preserves_numeric_type_boundaries() {
    assert_eq!(load(r#"
        function wasmGlobalConversion() {
            const rejects = action => { try { action(); return false; } catch (e) { return e instanceof TypeError; } };
            const types = ['i32', 'i64', 'f32', 'f64'];
            const constructorChecks = types.map(type => rejects(() => new WebAssembly.Global({ value: type }, type === 'i64' ? 1 : 1n)));
            const globals = types.map(type => new WebAssembly.Global({ value: type, mutable: true }));
            const localChecks = globals.map((global, i) => rejects(() => { global.value = types[i] === 'i64' ? { valueOf() { return 1; } } : { valueOf() { return 1n; } }; }));
            const bytes = new TextEncoder().encode(`(module
                (global (export "i32") (mut i32) (i32.const 0))
                (global (export "i64") (mut i64) (i64.const 0))
                (global (export "f32") (mut f32) (f32.const 0))
                (global (export "f64") (mut f64) (f64.const 0))
                (global (export "fixed") i32 (i32.const 0)))`);
            const exported = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports;
            const exportedChecks = types.map(type => rejects(() => { exported[type].value = type === 'i64' ? 1 : 1n; }));
            let conversions = 0;
            const immutable = rejects(() => { exported.fixed.value = { valueOf() { conversions++; return 1; } }; });
            globals[1].value = { valueOf() { return 18446744073709551617n; } };
            exported.i64.value = '7'; exported.i32.value = '9';
            return [constructorChecks, localChecks, exportedChecks, immutable, conversions, globals[1].value.toString(), exported.i64.value.toString(), exported.i32.value,
                rejects(() => new WebAssembly.Global({ value: 'i64' }, undefined)), new WebAssembly.Global({ value: 'externref' }).value === null];
        }
    "#), 1);
    assert_eq!(call("wasmGlobalConversion", "[]"), "[[true,true,true,true],[true,true,true,true],[true,true,true,true],true,0,\"1\",\"7\",9,true,true]");
}

// Unrun regression: Wasm parameter types govern scalar conversion and externref retention.
#[test]
fn webassembly_arguments_use_declared_parameter_types() {
    assert_eq!(load(r#"
        function wasmArgumentTypes() {
            const bytes = new TextEncoder().encode(`(module
                (func (export "ref") (param externref) (result externref) local.get 0)
                (func (export "int") (param i32) (result i32) local.get 0)
                (func (export "wide") (param i64) (result i64) local.get 0)
                (func (export "float") (param f64) (result f64) local.get 0))`);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports;
            const rejects = action => { try { action(); return false; } catch (e) { return e instanceof TypeError; } };
            let coercions = 0;
            const object = { valueOf() { coercions++; return 7; } };
            const refs = [0, 1, 7n, undefined, null, object].map(value => exports.ref(value) === value);
            const values = [exports.int('7'), exports.float(object), exports.int(4294967297), exports.int(), exports.int(9, Symbol('ignored')), exports.wide('18446744073709551617').toString()];
            return [refs, values, coercions, rejects(() => exports.int(1n)), rejects(() => exports.wide(1)), rejects(() => exports.float({ valueOf() { return 1n; } }))];
        }
    "#), 1);
    assert_eq!(call("wasmArgumentTypes", "[]"), "[[true,true,true,true,true,true],[7,7,1,0,9,\"1\"],1,true,true,true]");
}

// Unrun regression: special floats and primitive externrefs cross Global/Table boundaries.
#[test]
fn webassembly_codec_preserves_special_numbers_and_reference_values() {
    assert_eq!(load(r#"
        function wasmCodecBoundaries() {
            const bytes = new TextEncoder().encode(`(module
                (import "host" "global" (global $g (mut externref)))
                (import "host" "table" (table $t 2 externref))
                (export "global" (global $g)) (export "table" (table $t))
                (global (export "floatGlobal") (mut f64) (f64.const 0))
                (func (export "float") (param f64) (result f64) local.get 0)
                (func (export "readGlobal") (result externref) global.get $g))`);
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
    "#), 1);
    assert_eq!(call("wasmCodecBoundaries", "[]"), "[[true,true],[true,true,true],[true,true,true,true]]");
}

// Unrun regression: start writes and growth are visible before the first exported call.
#[test]
fn webassembly_start_refreshes_imported_memory() {
    assert_eq!(load(r#"
        function wasmStartMemory() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const oldBuffer = memory.buffer;
            const bytes = new TextEncoder().encode(`(module
                (import "host" "memory" (memory 1 2))
                (data (i32.const 1) "A")
                (func $start
                    i32.const 0 i32.const 42 i32.store8
                    i32.const 1 memory.grow drop
                    i32.const 65536 i32.const 8 i32.store8)
                (start $start)
                (func (export "read") (result i32)
                    i32.const 0 i32.load8_u i32.const 65536 i32.load8_u i32.add))`);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { memory } });
            const view = new Uint8Array(memory.buffer);
            const before = [view[0], view[1], view[65536], oldBuffer.byteLength, memory.buffer.byteLength];
            return [before, instance.exports.read(), new Uint8Array(memory.buffer)[0]];
        }
    "#), 1);
    assert_eq!(call("wasmStartMemory", "[]"), "[[42,65,8,0,131072],50,42]");
}

// Unrun regression: memory export aliases share the original object and one sync resource.
#[test]
fn webassembly_memory_export_aliases_reuse_original_buffer() {
    assert_eq!(load(r#"
        function wasmMemoryIdentity() {
            const memory = new WebAssembly.Memory({ initial: 1 });
            const bytes = new TextEncoder().encode(`(module
                (import "host" "memory" (memory 1))
                (export "a" (memory 0)) (export "b" (memory 0))
                (func (export "read") (result i32) i32.const 0 i32.load8_u))`);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { memory } }).exports;
            new Uint8Array(memory.buffer)[0] = 73;
            const read = exports.read();
            const own = new WebAssembly.Instance(new WebAssembly.Module(new TextEncoder().encode(`(module
                (memory 0) (export "a" (memory 0)) (export "b" (memory 0)))`))).exports;
            return [exports.a === memory, exports.b === memory, exports.a.buffer === memory.buffer, read, new Uint8Array(memory.buffer)[0], own.a === own.b, own.a !== memory];
        }
    "#), 1);
    assert_eq!(call("wasmMemoryIdentity", "[]"), "[true,true,true,73,73,true,true]");
}

// Unrun regression: duplicate imports share native memory and refresh growth once.
#[test]
fn webassembly_duplicate_memory_imports_share_one_resource() {
    assert_eq!(load(r#"
        function wasmMemoryImportAliases() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const oldBuffer = memory.buffer;
            const bytes = new TextEncoder().encode(`(module
                (import "host" "first" (memory $first 1 2))
                (import "host" "second" (memory $second 1 2))
                (export "a" (memory $first)) (export "b" (memory $second))
                (func $start
                    i32.const 0 i32.const 42 i32.store8 $first
                    i32.const 1 memory.grow $first drop)
                (start $start)
                (func (export "read") (result i32) i32.const 0 i32.load8_u $second))`);
            const exports = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { first: memory, second: memory } }).exports;
            const before = exports.read();
            new Uint8Array(memory.buffer)[0] = 73;
            return [exports.a === memory, exports.b === memory, oldBuffer.byteLength, memory.buffer.byteLength, before, exports.read()];
        }
    "#), 1);
    assert_eq!(call("wasmMemoryImportAliases", "[]"), "[true,true,0,131072,42,73]");
}

// Unrun regression target: an imported JS callback reenters its active instance.
#[test]
fn webassembly_import_callback_reenters_same_instance() {
    assert_eq!(load(r#"
        function wasmReentry() {
            const bytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback (result i32)))
                (memory (export "memory") 1)
                (func (export "inner") (result i32) i32.const 0 i32.load8_u)
                (func (export "outer") (result i32) call $callback i32.const 1 i32.add))`);
            let instance;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { callback() {
                new Uint8Array(instance.exports.memory.buffer)[0] = 41;
                return instance.exports.inner();
            } } });
            return [instance.exports.outer(), instance.exports.inner()];
        }
    "#), 1);
    assert_eq!(call("wasmReentry", "[]"), "[42,41]");
}

// Unrun regression target: result coercion reenters the active Caller.
#[test]
fn webassembly_import_result_coercion_reenters_same_instance() {
    assert_eq!(load(r#"
        function wasmResultReentry() {
            const bytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback (result i32)))
                (func (export "inner") (result i32) i32.const 41)
                (func (export "outer") (result i32) call $callback i32.const 1 i32.add))`);
            let instance, conversions = 0;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: { callback() {
                return { valueOf() { conversions++; return instance.exports.inner(); } };
            } } });
            return [instance.exports.outer(), conversions];
        }
    "#), 1);
    assert_eq!(call("wasmResultReentry", "[]"), "[42,1]");
}

// Unrun regression target: a start import can call another published instance.
#[test]
fn webassembly_start_import_calls_existing_instance() {
    assert_eq!(load(r#"
        function wasmStartReentry() {
            const firstBytes = new TextEncoder().encode(`(module
                (func (export "read") (result i32) i32.const 41))`);
            const first = new WebAssembly.Instance(new WebAssembly.Module(firstBytes));
            const nextBytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback (result i32)))
                (global $value (mut i32) (i32.const 0))
                (func $start call $callback global.set $value)
                (start $start)
                (func (export "read") (result i32) global.get $value))`);
            const next = new WebAssembly.Instance(new WebAssembly.Module(nextBytes), {
                host: { callback() { return first.exports.read(); } }
            });
            return [next.exports.read(), first.exports.read()];
        }
    "#), 1);
    assert_eq!(call("wasmStartReentry", "[]"), "[41,41]");
}

// Unrun regression target: releasing another instance preserves running externrefs.
#[test]
fn webassembly_reentrant_dispose_preserves_externrefs() {
    assert_eq!(load(r#"
        function wasmDisposeDuringCall() {
            const empty = new TextEncoder().encode('(module)');
            const other = new WebAssembly.Instance(new WebAssembly.Module(empty));
            const value = { answer: 42 };
            const global = new WebAssembly.Global({ value: 'externref', mutable: false }, value);
            const bytes = new TextEncoder().encode(`(module
                (import "host" "value" (global $value externref))
                (import "host" "callback" (func $callback))
                (func (export "read") (result externref) call $callback global.get $value))`);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                host: { value: global, callback() { other.dispose(); } }
            });
            return instance.exports.read() === value;
        }
    "#), 1);
    assert_eq!(call("wasmDisposeDuringCall", "[]"), "true");
}

// Unrun regression target: nested imports use the newest Caller for their Store.
#[test]
fn webassembly_nested_import_reentry() {
    assert_eq!(load(r#"
        function wasmNestedReentry() {
            const bytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback (param i32) (result i32)))
                (func (export "recurse") (param i32) (result i32)
                    local.get 0 call $callback i32.const 1 i32.add))`);
            let instance;
            instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { host: {
                callback(depth) { return depth === 0 ? 40 : instance.exports.recurse(depth - 1); }
            } });
            return instance.exports.recurse(2);
        }
    "#), 1);
    assert_eq!(call("wasmNestedReentry", "[]"), "43");
}

// Unrun regression target: special export names are own properties on a null prototype.
#[test]
fn webassembly_special_export_names() {
    assert_eq!(load(r#"
        function wasmSpecialExports() {
            const bytes = new TextEncoder().encode(`(module
                (func $value (result i32) i32.const 42)
                (export "__proto__" (func $value))
                (export "constructor" (func $value)))`);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
            const exports = instance.exports;
            return [Object.getPrototypeOf(exports) === null,
                Object.prototype.hasOwnProperty.call(exports, "__proto__"),
                exports.__proto__(), exports.constructor(),
                exports.__proto__ === exports.constructor, Object.isFrozen(exports)];
        }
    "#), 1);
    assert_eq!(call("wasmSpecialExports", "[]"), "[true,true,42,42,true,true]");
}

// Unrun regression target: shared memory remains coherent across native callbacks.
#[test]
fn webassembly_shared_memory_cross_instance_callback() {
    assert_eq!(load(r#"
        function wasmSharedCallback() {
            const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
            const readerBytes = new TextEncoder().encode(`(module
                (import "host" "memory" (memory 1))
                (func (export "read") (result i32) i32.const 0 i32.load8_u)
                (func (export "size") (result i32) memory.size))`);
            const reader = new WebAssembly.Instance(new WebAssembly.Module(readerBytes), { host: { memory } });
            const writerBytes = new TextEncoder().encode(`(module
                (import "host" "memory" (memory 1))
                (import "host" "callback" (func $callback (result i32)))
                (func (export "writeAndRead") (result i32)
                    i32.const 0 i32.const 42 i32.store8 call $callback))`);
            let observedBuffer;
            const writer = new WebAssembly.Instance(new WebAssembly.Module(writerBytes), {
                host: { memory, callback() { observedBuffer = new Uint8Array(memory.buffer)[0]; const value = reader.exports.read(); new Uint8Array(memory.buffer)[1] = 73; return value; } }
            });
            const fromCallback = writer.exports.writeAndRead();
            const afterCallback = [fromCallback, reader.exports.read(), observedBuffer, new Uint8Array(memory.buffer)[0], new Uint8Array(memory.buffer)[1]];
            const previousPages = memory.grow(1);
            return afterCallback.concat([previousPages, memory.buffer.byteLength, reader.exports.size()]);
        }
    "#), 1);
    assert_eq!(call("wasmSharedCallback", "[]"), "[42,42,42,42,73,1,131072,2]");
}

// Unrun regression target: an exported memory retains the physical memory after dispose.
#[test]
fn webassembly_exported_memory_survives_instance_dispose() {
    assert_eq!(load(r#"
        function wasmRetainedMemory() {
            const bytes = new TextEncoder().encode(`(module
                (memory (export "memory") 1 2)
                (data (i32.const 0) "A"))`);
            const module = new WebAssembly.Module(bytes);
            const instance = new WebAssembly.Instance(module);
            const memory = instance.exports.memory;
            instance.dispose();
            module.dispose();
            const before = new Uint8Array(memory.buffer)[0];
            const oldPages = memory.grow(1);
            return [before, oldPages, memory.buffer.byteLength, new Uint8Array(memory.buffer)[0]];
        }
    "#), 1);
    assert_eq!(call("wasmRetainedMemory", "[]"), "[65,1,131072,65]");
}

// Unrun regression target: successful grow(0) replaces and detaches the buffer.
#[test]
fn webassembly_memory_zero_growth_detaches_buffer() {
    assert_eq!(load(r#"
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
    "#), 1);
    assert_eq!(call("wasmZeroGrowth", "[]"), "[1,0,true,65536,91,true,true]");
}

// Unrun regression target: callbacks refresh memory defined by the calling module.
#[test]
fn webassembly_internal_memory_callback_coherence() {
    assert_eq!(load(r#"
        function wasmInternalCallback() {
            const bytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback))
                (memory (export "memory") 1)
                (func (export "writeAndCall") (result i32)
                    i32.const 0 i32.const 42 i32.store8
                    call $callback
                    i32.const 1 i32.load8_u))`);
            let memory, observed;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                host: { callback() { observed = new Uint8Array(memory.buffer)[0]; new Uint8Array(memory.buffer)[1] = 73; } }
            });
            memory = instance.exports.memory;
            return [instance.exports.writeAndCall(), observed, new Uint8Array(memory.buffer)[1]];
        }
    "#), 1);
    assert_eq!(call("wasmInternalCallback", "[]"), "[73,42,73]");
}

// Unrun regression target: import, coercion and start preserve the exact thrown value.
#[test]
fn webassembly_callback_exception_identity() {
    assert_eq!(load(r#"
        function wasmExceptionIdentity() {
            const marker = { reason: "callback" }, coercion = { reason: "coercion" };
            const bytes = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback (result i32)))
                (func (export "call") (result i32) call $callback))`);
            const results = [];
            for (const thrown of [marker, null, undefined, 17]) {
                const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                    host: { callback() { throw thrown; } }
                });
                let caught = false;
                try { instance.exports.call(); } catch (error) { caught = error === thrown; }
                results.push(caught);
                instance.dispose();
            }
            const converted = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                host: { callback() { return { valueOf() { throw coercion; } }; } }
            });
            try { converted.exports.call(); results.push(false); } catch (error) { results.push(error === coercion); }
            converted.dispose();
            const start = new TextEncoder().encode(`(module
                (import "host" "callback" (func $callback))
                (func $start call $callback) (start $start))`);
            try { new WebAssembly.Instance(new WebAssembly.Module(start), { host: { callback() { throw marker; } } }); results.push(false); }
            catch (error) { results.push(error === marker); }
            return results;
        }
    "#), 1);
    assert_eq!(call("wasmExceptionIdentity", "[]"), "[true,true,true,true,true,true]");
}

// Unrun regression target: a table-derived function synchronizes exported memory.
#[test]
fn webassembly_table_funcref_memory_coherence() {
    assert_eq!(load(r#"
        function wasmTableMemory() {
            const bytes = new TextEncoder().encode(`(module
                (memory (export "memory") 1)
                (table (export "table") 1 funcref)
                (func $write (param i32) i32.const 0 local.get 0 i32.store8)
                (elem (i32.const 0) $write))`);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
            const memory = instance.exports.memory;
            new Uint8Array(memory.buffer)[1] = 91;
            const write = instance.exports.table.get(0);
            write(73);
            return [new Uint8Array(memory.buffer)[0], new Uint8Array(memory.buffer)[1]];
        }
    "#), 1);
    assert_eq!(call("wasmTableMemory", "[]"), "[73,91]");
}
