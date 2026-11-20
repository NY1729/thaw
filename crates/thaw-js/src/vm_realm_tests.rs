//! `node:vm` context realms are lean (`Interp::new_vm_realm`): cheap to build, ES globals only,
//! and they leave no stdin or event-loop work behind.

use std::rc::Weak;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

const VM_PRELUDE: &str =
    include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js");

/// 30 distinct contexts, each created and run once; prints the sum of i*i for i < 30. Scale (300 contexts) is
/// covered by `vm_300_contexts_run_once_each_finish_fast`; this test only needs several contexts to check that
/// none of them leaves stdin or event-loop work behind.
const MANY_CONTEXTS: &str = "let total = 0; for (let i = 0; i < 30; i++) total += runInNewContext('n * n', { n: i }); console.log(total);";

fn run_lines(src: &str) -> Vec<String> {
    // Timing and linearity are measured without the automatic trigger: under the stress interval every collection
    // walks all live contexts, which is the cost being stress-tested, not the cost of creating a context.
    crate::gc::no_auto();
    let mut it = Interp::new();
    let _ = it.run(src);
    std::mem::take(&mut it.out)
}

/// Like `run_lines`, then drains the event loop, so the `await` continuations of async bodies run too.
fn run_lines_async(src: &str) -> Vec<String> {
    crate::gc::no_auto();
    let mut it = Interp::new();
    assert!(it.run(src).is_ok());
    assert!(it.run_event_loop().is_ok());
    std::mem::take(&mut it.out)
}

/// Builds `n` contexts, each created and run once; returns the sum of i*i and the elapsed time.
fn run_many_contexts(n: usize) -> (Vec<String>, Duration) {
    let t = Instant::now();
    let src = format!(
        "let total = 0; for (let i = 0; i < {n}; i++) total += runInNewContext('n * n', {{ n: i }}); console.log(total);"
    );
    let out = run_lines(&format!("{VM_PRELUDE}\n{src}"));
    (out, t.elapsed())
}

/// Creating contexts must stay linear: a context is lean (`new_vm_realm`), so 300 of them must not
/// cost more than a small multiple of 30 (a leak or a per-context rebuild of shared work would be
/// super-linear). A wall-clock threshold alone flaked on a loaded CI runner (20.2 s against 20 s);
/// the absolute ceiling is only a hang guard.
#[test]
fn vm_300_contexts_run_once_each_finish_fast() {
    let (small_out, small) = run_many_contexts(30);
    assert_eq!(small_out, vec!["8555".to_string()]);
    let (out, large) = run_many_contexts(300);
    assert_eq!(out, vec!["8955050".to_string()]);
    assert!(
        large < Duration::from_secs(120),
        "300 contexts took {large:?} (hang guard)"
    );
    // 10x the contexts, a fixed per-run setup cost on top: allow 15x the 30-context time
    assert!(
        large <= small * 15 + Duration::from_secs(1),
        "300 contexts took {large:?}, more than 15x the 30-context time {small:?}: creation is no longer linear"
    );
}

#[test]
fn vm_contexts_leave_no_stdin_or_loop_work() {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::gc::no_auto();
        let mut it = Interp::new();
        let _ = it.run(&format!("{VM_PRELUDE}\n{MANY_CONTEXTS}"));
        let pending = it.work_pending();
        let stdin = crate::sysio::stdin_active();
        let loop_ok = it.run_event_loop().is_ok();
        let _ = tx.send((it.out.clone(), pending, stdin, loop_ok));
    });
    // Hang bound only: the run takes about 1.5 s in a debug build; 120 s cannot trip on a loaded machine.
    let (out, pending, stdin, loop_ok) = rx
        .recv_timeout(Duration::from_secs(120))
        .expect("contexts run and the event loop returns within 120 s (hang bound)");
    worker.join().expect("worker thread");
    assert_eq!(out, vec!["8555".to_string()]);
    assert!(!pending, "contexts left pending work");
    assert!(!stdin, "contexts started stdin");
    assert!(loop_ok, "event loop failed after contexts");
}

#[test]
fn vm_realm_has_only_es_globals() {
    let host_intl = run_lines("console.log(typeof Intl)");
    let src = format!(
        "{VM_PRELUDE}\n{}",
        "console.log(runInNewContext(\"[typeof process, typeof Buffer, typeof URL, typeof setTimeout, typeof __thaw_vm_run, typeof require, typeof fetch, typeof structuredClone, typeof Array, typeof Map, typeof Promise, typeof Uint8Array, typeof Intl].join(',')\", {}));"
    );
    let out = run_lines(&src);
    let expected = format!(
        "undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined,function,function,function,function,{}",
        host_intl[0]
    );
    assert_eq!(out, vec![expected]);
    #[cfg(feature = "intl")]
    assert_eq!(
        run_lines(&format!(
            "{VM_PRELUDE}\nconsole.log(runInNewContext('Intl === h', {{ h: Intl }}));"
        )),
        vec!["true".to_string()]
    );
}

#[test]
fn vm_realm_construct_cheaper_than_host() {
    // warm up both paths once so first-use costs do not count
    drop(Interp::new());
    drop(Interp::new_vm_realm());
    let (mut vm, mut host) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..10 {
        let t = Instant::now();
        let it = Interp::new_vm_realm();
        vm += t.elapsed();
        drop(it);
        let t = Instant::now();
        let it = Interp::new();
        host += t.elapsed();
        drop(it);
    }
    assert!(
        vm < host,
        "vm realms {vm:?} not cheaper than host realms {host:?}"
    );
}

#[test]
fn vm_realm_hides_navigator() {
    let out = run_lines(&format!(
        "{VM_PRELUDE}\nconsole.log(runInNewContext(\"[typeof navigator, typeof Navigator].join(',')\", {{}}));"
    ));
    assert_eq!(out, vec!["undefined,undefined".to_string()]);
}

#[test]
fn vm_realm_has_bigint_typed_arrays() {
    // Node 22: the BigInt64Array / BigUint64Array constructors are ES globals, so a new context has them too.
    let src = format!(
        "{VM_PRELUDE}\n{}",
        "console.log(runInNewContext(\"[typeof BigInt64Array, typeof BigUint64Array, BigInt64Array.BYTES_PER_ELEMENT, new BigUint64Array([-1n])[0]].join(',')\", {}));"
    );
    assert_eq!(
        run_lines(&src),
        vec!["function,function,8,18446744073709551615".to_string()]
    );
}

#[test]
fn util_types_know_the_bigint_typed_arrays() {
    // Node 22 `util.types` (checked with node 22.23.1): the 64-bit checks match only their own array kind.
    let src = "const t = globalThis.__thaw_util_types;\nconsole.log([t.isBigInt64Array(new BigInt64Array(1)), t.isBigUint64Array(new BigInt64Array(1)), t.isBigUint64Array(new BigUint64Array(1)), t.isBigInt64Array(new Int8Array(1))].join(','));";
    assert_eq!(run_lines(src), vec!["true,false,true,false".to_string()]);
}

#[test]
fn vm_realm_hides_performance_globals() {
    // Node 22 `vm.runInNewContext`: performance and every user-timing class are undefined
    let src = format!(
        "{VM_PRELUDE}\n{}",
        "console.log(runInNewContext(\"[typeof performance, typeof Performance, typeof PerformanceEntry, typeof PerformanceMark, typeof PerformanceMeasure, typeof PerformanceObserver, typeof PerformanceObserverEntryList, typeof PerformanceResourceTiming].join(',')\", {}));"
    );
    assert_eq!(
        run_lines(&src),
        vec![
            "undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined"
                .to_string()
        ]
    );
}

fn prelude_parses() -> usize {
    crate::PRELUDE_PARSES.with(|n| n.get())
}

#[test]
fn shared_prelude_parse_count_is_one_for_n_vm_realms() {
    // on a fresh thread: the shared cache is thread-local and the counter is per thread
    std::thread::spawn(|| {
        let _first = Interp::new_vm_realm();
        let after_first = prelude_parses();
        assert!(after_first > 0, "the first vm realm parsed no prelude");
        for _ in 0..20 {
            drop(Interp::new_vm_realm());
        }
        assert_eq!(
            prelude_parses(),
            after_first,
            "later vm realms re-parsed a prelude"
        );
    })
    .join()
    .expect("worker thread");
}

#[test]
fn host_realm_does_not_populate_shared_cache() {
    std::thread::spawn(|| {
        let _host = Interp::new();
        assert_eq!(
            crate::shared_prelude_entries(),
            0,
            "host realm filled the shared cache"
        );
        assert_eq!(
            prelude_parses(),
            0,
            "host realm used the shared prelude path"
        );
    })
    .join()
    .expect("worker thread");
}

#[test]
fn vm_realm_calls_prelude_closure_from_another_realm() {
    // `Object.groupBy` is a JS prelude closure; realm A's copy runs with realm B's values and hints
    let mut a = Interp::new_vm_realm();
    let group = a.run("Object.groupBy").unwrap();
    let mut b = Interp::new_vm_realm();
    let items = b.run("[1, 2, 3, 4]").unwrap();
    let key = b.run("(x) => (x % 2 ? 'odd' : 'even')").unwrap();
    let grouped = b.call(&group, &Value::Undefined, &[items, key]).unwrap();
    let show = b.run("(v) => console.log(JSON.stringify(v))").unwrap();
    b.call(&show, &Value::Undefined, &[grouped]).unwrap();
    assert_eq!(b.out, vec![r#"{"odd":[1,3],"even":[2,4]}"#.to_string()]);
}

#[test]
fn vm_realms_share_prelude_source_text() {
    let mut a = Interp::new_vm_realm();
    let mut b = Interp::new_vm_realm();
    let fa = a.run("Object.groupBy.toString()").unwrap();
    let fb = b.run("Object.groupBy.toString()").unwrap();
    let text = a.to_str(&fa).to_string();
    assert!(
        text.contains("groupBy") || text.contains("items"),
        "no source text: {text}"
    );
    assert_eq!(text, b.to_str(&fb).to_string());
}

#[test]
fn vm_context_global_has_no_global_to_string_tag() {
    let out = run_lines(&format!(
        "{VM_PRELUDE}\n{}",
        "console.log(runInNewContext(\"[Object.prototype.toString.call(this), typeof globalThis[Symbol.toStringTag]].join(' ')\", {}));"
    ));
    assert_eq!(out, vec!["[object Object] undefined".to_string()]);
}

/// Dropping an `Interp` breaks the Rc cycles reachable from its roots, so the global objects of the host
/// and of a vm realm it made are freed (they are cycle nodes: global object -> props -> closures -> scope).
/// Known residue, not asserted: the global scope and Object.prototype stay alive through a few Rust-held
/// closures the walk cannot reach (measured: ~0.1 MB per dropped Interp, was ~5.9 MB).
#[test]
fn interp_drop_frees_host_and_realm_global_objects() {
    use std::any::Any;
    fn weak<T: Any>(r: &Rc<T>) -> Weak<dyn Any> {
        let w: Weak<T> = Rc::downgrade(r);
        w
    }
    let mut it = Interp::new();
    it.run("function f() {}; globalThis.o = { f };")
        .expect("script runs");
    it.run(&format!(
        "{VM_PRELUDE}\nconsole.log(runInNewContext('1', {{}}));"
    ))
    .expect("context script runs");
    let host = weak(&it.global_object);
    let realm = weak(&vm::first_realm_global_object().expect("context realm built"));
    drop(it);
    assert!(host.upgrade().is_none(), "host global object survived drop");
    assert!(
        realm.upgrade().is_none(),
        "realm global object survived drop"
    );
}

/// Freeing a context drops its realm, which must leave the host's shared globals (Intl) intact.
#[test]
fn vm_free_keeps_host_shared_globals_usable() {
    let mut it = Interp::new();
    let r = it.run(
        "const id = __thaw_vm_create(); __thaw_vm_run(id, '1', 'f', 0, undefined, undefined); \
         __thaw_vm_free(id); console.log(typeof Atomics.add, new SharedArrayBuffer(8).byteLength);",
    );
    if let Err(Ctl::Throw(v)) = &r {
        let m = it.get(v, "message").unwrap_or(Value::Undefined);
        panic!("script threw: {}", it.to_str(&m));
    }
    assert!(r.is_ok(), "script failed");
    assert_eq!(it.out, vec!["function 8".to_string()]);
}

/// A closure of the host, called from inside context `outer`, runs `vm.runInContext` on `inner`: the nested run
/// must find `inner` in the table even though it executes on `outer`'s realm.
#[test]
fn vm_nested_run_in_context_from_another_context() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"const inner = createContext({});
const outer = createContext({ nest: (code) => runInContext(code, inner) });
console.log(runInContext('[nest("var deep = 1; deep + 1"), typeof deep, nest("deep")].join(",")', outer));
console.log(runInContext("1 + 1", outer));"#
    );
    let mut it = Interp::new();
    if let Err(Ctl::Throw(v)) = it.run(&src) {
        let m = it.get(&v, "message").unwrap_or(Value::Undefined);
        panic!("script threw: {}", it.to_str(&m));
    }
    assert_eq!(it.out, vec!["2,undefined,1".to_string(), "2".to_string()]);
}

/// A script threw: panics with its message (the tests below run several scripts and drains in turn).
fn report_ctl(it: &mut Interp, e: Ctl) -> ! {
    match e {
        Ctl::Throw(v) => {
            let m = it.get(&v, "message").unwrap_or(Value::Undefined);
            panic!("script threw: {}", it.to_str(&m))
        }
        _ => panic!("script aborted"),
    }
}

fn run_script(it: &mut Interp, src: &str) {
    if let Err(e) = it.run(src) {
        report_ctl(it, e);
    }
}

/// Runs the host's promise jobs, then prints `globalThis.out` (the array the contexts push into).
fn drain_and_report(it: &mut Interp) -> Vec<String> {
    if let Err(e) = it.run_jobs() {
        report_ctl(it, e);
    }
    run_script(it, "console.log(globalThis.out.join(','))");
    std::mem::take(&mut it.out)
}

/// A context's promise reactions run: `then` on a context promise, an async function inside the context, and
/// a context promise the host awaits all settle with the host draining its jobs.
#[test]
fn vm_context_promise_jobs_run_on_host_drain() {
    let mut it = Interp::new();
    run_script(
        &mut it,
        &format!(
            "{VM_PRELUDE}\n{}",
            r#"globalThis.out = []; const ctx = createContext({ out });
runInContext('Promise.resolve(1).then(v => { out.push("then" + v) })', ctx);
runInContext('(async () => { await null; out.push("a"); return 5 })().then(v => out.push("r" + v))', ctx);
(async () => { const v = await runInContext('(async () => { await null; return 7 })()', ctx); out.push("host" + v); })();"#
        ),
    );
    assert_eq!(
        drain_and_report(&mut it),
        vec!["then1,a,r5,host7".to_string()]
    );
}

/// A host promise's `then`, called from inside a context, runs its handler on that context's queue.
#[test]
fn vm_host_promise_then_called_from_a_context_runs() {
    let mut it = Interp::new();
    run_script(
        &mut it,
        &format!(
            "{VM_PRELUDE}\n{}",
            r#"globalThis.out = []; const ctx = createContext({ out, hp: Promise.resolve(3) });
runInContext('hp.then(v => out.push("hp" + v))', ctx);"#
        ),
    );
    assert_eq!(drain_and_report(&mut it), vec!["hp3".to_string()]);
}

/// Host and context reactions queued in one tick: the host's checkpoint runs first, then the context's queue
/// (the accepted ordering: per-realm FIFOs, realm queues after the host's). Node interleaves them in queue order
/// (`host1,ctx,host2`); pinned here so that a change of model shows up.
#[test]
fn vm_host_and_context_then_same_tick_order_is_pinned() {
    let mut it = Interp::new();
    run_script(
        &mut it,
        &format!(
            "{VM_PRELUDE}\n{}",
            r#"globalThis.out = []; const ctx = createContext({ out });
Promise.resolve().then(() => out.push("host1"));
runInContext('Promise.resolve().then(() => out.push("ctx"))', ctx);
Promise.resolve().then(() => out.push("host2"));"#
        ),
    );
    assert_eq!(
        drain_and_report(&mut it),
        vec!["host1,host2,ctx".to_string()]
    );
}

/// A context's jobs wait for the host's checkpoint: the host code after a `runInContext` finishes first, as in Node.
#[test]
fn vm_context_jobs_wait_for_the_host_checkpoint() {
    let mut it = Interp::new();
    run_script(
        &mut it,
        &format!(
            "{VM_PRELUDE}\n{}",
            r#"globalThis.out = []; const ctx = createContext({ out });
runInContext('Promise.resolve().then(() => out.push("ctx"))', ctx);
out.push("sync");"#
        ),
    );
    assert_eq!(drain_and_report(&mut it), vec!["sync,ctx".to_string()]);
}

/// A due timer (pending work) does not stop the loop from draining a context's promise jobs first.
#[test]
fn vm_timer_pending_run_still_drains_context_jobs() {
    let mut it = Interp::new();
    run_script(
        &mut it,
        &format!(
            "{VM_PRELUDE}\n{}",
            r#"globalThis.out = []; const ctx = createContext({ out });
setTimeout(() => out.push("timer"), 5);
runInContext('Promise.resolve(1).then(v => out.push("ctx" + v))', ctx);"#
        ),
    );
    if let Err(e) = it.run_event_loop() {
        report_ctl(&mut it, e);
    }
    run_script(&mut it, "console.log(globalThis.out.join(','))");
    assert_eq!(std::mem::take(&mut it.out), vec!["ctx1,timer".to_string()]);
}

/// `Object.prototype.toString` tags an error made in a context as `[object Error]`, whatever its subclass.
#[test]
fn vm_context_errors_keep_the_error_tag() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"const ctx = createContext({});
const tag = (code) => Object.prototype.toString.call(runInContext(code, ctx));
console.log([tag('new Error("x")'), tag('new TypeError("y")'), tag('class E2 extends Error {}; new E2("z")'), tag('({ a: 1 })')].join(' '));"#
    );
    assert_eq!(
        run_lines(&src),
        vec!["[object Error] [object Error] [object Error] [object Object]".to_string()]
    );
}

/// An error made while a named script runs carries the filename in `stack`, after the `Name: message` header.
#[test]
fn vm_error_stack_names_the_script_file() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"const sb = createContext({});
try { runInContext('throw new Error("boom")', sb, { filename: 'ctx-file.js' }); } catch (e) {
  const lines = String(e.stack).split('\n');
  console.log([lines[0], String(e.stack).includes('ctx-file.js')].join(' | '));
}
try { runInContext('null.x', sb, { filename: 'ctx-null.js' }); } catch (e) { console.log(String(e.stack).includes('ctx-null.js')); }
console.log(new Error('host').stack.includes('ctx-file.js'));"#
    );
    assert_eq!(
        run_lines(&src),
        vec![
            "Error: boom | true".to_string(),
            "true".to_string(),
            "false".to_string()
        ]
    );
}

/// A run nested inside another (a host closure called by one context runs a second, named script) restores the
/// outer filename once it returns: an error the outer script makes afterwards still names the outer file.
#[test]
fn vm_nested_script_run_restores_the_outer_filename() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"globalThis.inner = () => runInThisContext('new Error("in").stack', { filename: 'inner.js' });
console.log(runInThisContext('[inner().includes("inner.js"), new Error("o").stack.includes("outer.js"), new Error("o").stack.includes("inner.js")].join(",")', { filename: 'outer.js' }));
console.log(new Error("after").stack.includes("outer.js"));"#
    );
    assert_eq!(
        run_lines(&src),
        vec!["true,true,false".to_string(), "false".to_string()]
    );
}

/// A SyntaxError from a named script keeps `SyntaxError` as its name, has a message, and its stack names the file
/// (V8 puts the `file:line` header and the source line above `SyntaxError: message`).
#[test]
fn vm_syntax_error_names_the_script_file() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"try { __thaw_vm_eval('(', 'bad.js', true); } catch (e) {
  console.log([e.name, e.message.length > 0, String(e.stack).includes('bad.js'), String(e.stack).split('\n').some((l) => l.startsWith('SyntaxError: '))].join(','));
}
const sb = createContext({});
try { runInContext('let a = ;', sb, { filename: 'bad-syntax.js' }); } catch (e) {
  console.log([e.name, String(e.stack).includes('bad-syntax.js')].join(','));
}"#
    );
    assert_eq!(
        run_lines(&src),
        vec![
            "SyntaxError,true,true,true".to_string(),
            "SyntaxError,true".to_string()
        ]
    );
}

/// Without a filename the stack is the plain header, as before.
#[test]
fn vm_script_without_filename_keeps_the_plain_stack() {
    let src = r#"console.log(__thaw_vm_run(-1, 'new Error("plain").stack', undefined, 0));"#;
    assert_eq!(run_lines(src), vec!["Error: plain".to_string()]);
}

/// A context made at top level and run inside async bodies, before and after an `await`. Under the thread
/// coroutine backend each async body runs on its own thread, so the context table has to follow the control.
#[test]
fn vm_context_made_at_top_level_runs_in_async_bodies() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"const ctx = createContext({ x: 1 });
(async () => {
  console.log(`before ${runInContext('x + 1', ctx)}`);
  await null;
  console.log(`after ${runInContext('x + 2', ctx)}`);
})().catch((e) => console.log(`E:${e.message}`));"#
    );
    assert_eq!(
        run_lines_async(&src),
        vec!["before 2".to_string(), "after 3".to_string()]
    );
}

/// A context made inside an async body (before its `await`) is found afterwards at top level and in another
/// async body that runs after an `await`.
#[test]
fn vm_context_made_in_async_body_runs_later() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"let ctx;
(async () => {
  ctx = createContext({ y: 5 });
  await null;
  console.log(`inner ${runInContext('y * 3', ctx)}`);
})().catch((e) => console.log(`E:${e.message}`));
console.log(`top ${runInContext('y * 2', ctx)}`);
(async () => {
  await null;
  console.log(`later ${runInContext('y * 4', ctx)}`);
})().catch((e) => console.log(`E:${e.message}`));"#
    );
    assert_eq!(
        run_lines_async(&src),
        vec![
            "top 10".to_string(),
            "inner 15".to_string(),
            "later 20".to_string()
        ]
    );
}

/// Two contexts made on two different coroutine threads (and one at top level) each keep their own slot:
/// every context runs its own script, so no id is reused.
#[test]
fn vm_contexts_made_on_different_coroutine_threads_stay_distinct() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r#"let a, b;
(async () => {
  a = createContext({ v: 'a' });
  await null;
})().catch((e) => console.log(`E:${e.message}`));
(async () => {
  b = createContext({ v: 'b' });
  await null;
})().catch((e) => console.log(`E:${e.message}`));
const c = createContext({ v: 'c' });
console.log(runInContext('v', a) + runInContext('v', b) + runInContext('v', c));"#
    );
    assert_eq!(run_lines_async(&src), vec!["abc".to_string()]);
}

/// A bare call of a function the sandbox holds has no `this` of its own: a sloppy callee sees the context's global
/// object, a strict one `undefined`. The sandbox is not the `this` (the `with` of the vm scope is not a user `with`).
#[test]
fn vm_bare_call_this_is_not_the_sandbox() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({ f() { return this === sb; }, sf: function () { "use strict"; return typeof this; } });
console.log([runInContext('f()', sb), runInContext('sf()', sb), runInContext('(function () { "use strict"; return typeof this; })()', sb)].join(','));"##
    );
    assert_eq!(
        run_lines(&src),
        vec!["false,undefined,undefined".to_string()]
    );
}

/// A user `with` keeps its object as the `this` of a call it resolves, and a method call keeps its receiver.
#[test]
fn vm_user_with_and_method_call_keep_their_this() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({});
console.log([runInContext('var wo = { k() { return this === wo; } }; var wr; with (wo) { wr = k(); } wr', sb), runInContext('const holder = { m() { return this === holder; } }; holder.m()', sb)].join(','));"##
    );
    assert_eq!(run_lines(&src), vec!["true,true".to_string()]);
}

/// `let`/`const`/`class` persist in a context's script scope. A later run that declares the same name with
/// let/class throws a SyntaxError before any of its code runs; `var` redeclaration stays legal; a `let` conflicts with
/// an earlier `var` or function of the name; the same name in two contexts is fine.
#[test]
fn vm_let_redeclaration_throws_before_the_script_runs() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const attempt = (code, c) => { try { runInContext(code, c); return 'ok'; } catch (e) { return e.name + ':' + e.message; } };
const sb = createContext({ hits: 0 });
console.log(attempt('hits = hits + 1; let dup = 1', sb), attempt('hits = hits + 1; let dup = 2', sb), sb.hits);
console.log(attempt('var dup = 3', sb));
const v = createContext({});
console.log(attempt('var lv = 1; function fn1() { return 1 }', v), attempt('let lv = 2', v), attempt('let fn1 = 2', v));
console.log(attempt('var vv = 1; function fv() { return 1 }', v), attempt('var vv = 2; function fv() { return 2 }', v), runInContext('vv + fv()', v));
const a = createContext({}), b = createContext({});
console.log(attempt('let same = 1', a), attempt('let same = 2', b), runInContext('same', b));
console.log(attempt('class K1 {}', a), attempt('class K1 {}', a));
console.log(attempt('const k = 5', a), attempt('k = 6', a));"##
    );
    assert_eq!(
        run_lines(&src),
        vec![
            "ok SyntaxError:Identifier 'dup' has already been declared 1".to_string(),
            "SyntaxError:Identifier 'dup' has already been declared".to_string(),
            "ok SyntaxError:Identifier 'lv' has already been declared SyntaxError:Identifier 'fn1' has already been declared".to_string(),
            "ok ok 4".to_string(),
            "ok ok 2".to_string(),
            "ok SyntaxError:Identifier 'K1' has already been declared".to_string(),
            "ok TypeError:Assignment to constant variable.".to_string(),
        ]
    );
}

/// l_vm_objects `hostMutatesDeclared`: the var of a context is a property of its sandbox, so a host write is seen by
/// the context's function, and that function's write is seen by the host.
#[test]
fn vm_host_write_to_declared_var_is_seen_by_context_function() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const c = createContext({});
runInContext('var counter = 0; function bump() { return ++counter }', c);
c.counter = 10;
const r = [runInContext('counter', c)];
c.bump();
r.push(c.counter);
console.log(JSON.stringify(r));"##
    );
    assert_eq!(run_lines(&src), vec!["[10,11]".to_string()]);
}

/// l_vm_async `bump`: a context function called from the host writes its sandbox's var.
#[test]
fn vm_function_called_from_host_updates_sandbox_var() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({});
runInContext('var fnCalls = 0; function bump() { return ++fnCalls }', sb);
console.log(JSON.stringify([sb.bump(), sb.bump(), sb.fnCalls]));"##
    );
    assert_eq!(run_lines(&src), vec!["[1,2,2]".to_string()]);
}

/// An assignment to an undeclared name in a run creates a property of the sandbox, seen by the host and by the next run.
#[test]
fn vm_implicit_global_lands_on_sandbox() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({});
runInContext('x = 5', sb);
console.log(JSON.stringify([sb.x, runInContext('x + 1', sb)]));"##
    );
    assert_eq!(run_lines(&src), vec!["[5,6]".to_string()]);
}

/// A second `var d;` (no initializer) keeps the one binding of the sandbox, value and function included.
#[test]
fn vm_var_redeclaration_keeps_one_binding() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({});
const r = [runInContext('var d = 1; function dget() { return d }', sb), runInContext('var d; d', sb), runInContext('dget()', sb), Object.keys(sb).filter((k) => k === 'd').length];
console.log(JSON.stringify(r));"##
    );
    assert_eq!(run_lines(&src), vec!["[null,1,1,1]".to_string()]);
}

/// A frozen sandbox refuses the var, so the var stays local to the context's script scope: the run does not throw, the
/// value is readable in the context, and the sandbox gets no property.
#[test]
fn vm_frozen_sandbox_var_is_script_local() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const f = createContext(Object.freeze({ fz: 1 }));
const attempt = (code) => { try { runInContext(code, f); return 'ok'; } catch (e) { return e.name + ':' + e.message; } };
console.log(JSON.stringify([attempt('var y = 1; function fy() { return y + 1 }'), runInContext('y', f), runInContext('fy()', f), 'y' in f]));"##
    );
    assert_eq!(run_lines(&src), vec![r#"["ok",1,2,false]"#.to_string()]);
}

/// A sandbox accessor is read through its getter and written through its setter, from inside a run.
#[test]
fn vm_sandbox_accessor_goes_through_get_and_set() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"let stored = 0;
const c = createContext({ get acc() { return stored * 2 }, set acc(v) { stored = v } });
console.log(JSON.stringify([runInContext('acc = 5; acc', c), stored]));"##
    );
    assert_eq!(run_lines(&src), vec!["[10,5]".to_string()]);
}

/// The own-key operations on a context's `globalThis` read the sandbox: `Object.keys` lists its enumerable names,
/// var and function names included (they are sandbox properties).
#[test]
fn vm_globalthis_own_keys_forward_to_sandbox() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({ a: 1 });
runInContext('var b = 2; function fb() {}', sb);
console.log(JSON.stringify([runInContext('Object.keys(globalThis).sort().join()', sb), runInContext('Object.getOwnPropertyNames(globalThis).includes("a") && Object.getOwnPropertyNames(globalThis).includes("fb")', sb)]));"##
    );
    assert_eq!(run_lines(&src), vec![r#"["a,b,fb",true]"#.to_string()]);
}

/// `in` and hasOwnProperty on `globalThis` answer for sandbox names and var/function names; a missing name is false, and
/// a realm builtin (`Math`) is still an own property of the global.
#[test]
fn vm_globalthis_in_and_has_own_see_sandbox_and_vars() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({ a: 1 });
runInContext('var b = 2; function fb() {}', sb);
console.log(JSON.stringify(runInContext('[ "a" in globalThis, "b" in globalThis, "fb" in globalThis, globalThis.hasOwnProperty("b"), globalThis.hasOwnProperty("a"), globalThis.hasOwnProperty("fb"), "zz" in globalThis, globalThis.hasOwnProperty("zz"), globalThis.hasOwnProperty("Math") ]', sb)));"##
    );
    assert_eq!(
        run_lines(&src),
        vec!["[true,true,true,true,true,true,false,false,true]".to_string()]
    );
}

/// A descriptor of `globalThis` reads the sandbox, and `delete globalThis.x` deletes the sandbox property. A var or
/// function name is non-configurable: its delete returns false and it stays bound.
#[test]
fn vm_globalthis_descriptor_and_delete_forward_to_sandbox() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({ a: 1 });
console.log(JSON.stringify([runInContext('var b = 2; function fb() {} [Object.getOwnPropertyDescriptor(globalThis, "b").value, Object.getOwnPropertyDescriptor(globalThis, "a").value, delete globalThis.b, typeof b, delete globalThis.fb, typeof fb, delete globalThis.a, typeof a]', sb), 'a' in sb]));"##
    );
    assert_eq!(
        run_lines(&src),
        vec![r#"[[2,1,false,"number",false,"function",true,"undefined"],false]"#.to_string()]
    );
}

/// A sandbox property named like a realm builtin shadows it for `globalThis.X`, as it does for a bare `X`.
#[test]
fn vm_globalthis_sandbox_overrides_realm_builtin() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({ Math: { random: () => 4 } });
console.log(JSON.stringify(runInContext('[globalThis.Math.random(), Math.random(), globalThis.hasOwnProperty("Math")]', sb)));"##
    );
    assert_eq!(run_lines(&src), vec!["[4,4,true]".to_string()]);
}

/// Node 22 pins this: a `var` named like an inherited property (`toString`) is not defined again, so the name still
/// reads the inherited value and the sandbox gets no own property.
#[test]
fn vm_var_named_like_inherited_property_is_not_redefined() {
    let src = format!(
        "{VM_PRELUDE}\n{}",
        r##"const sb = createContext({});
console.log(JSON.stringify([runInContext('var toString; typeof toString', sb), Object.prototype.hasOwnProperty.call(sb, 'toString')]));"##
    );
    assert_eq!(run_lines(&src), vec![r#"["function",false]"#.to_string()]);
}
