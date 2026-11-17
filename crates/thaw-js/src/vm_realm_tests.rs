//! `node:vm` context realms are lean (`Interp::new_vm_realm`): cheap to build, ES globals only,
//! and they leave no stdin or event-loop work behind.

use std::rc::Weak;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

const VM_PRELUDE: &str =
    include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js");

/// 300 distinct contexts, each created and run once; prints the sum of i*i for i < 300.
const MANY_CONTEXTS: &str = "let total = 0; for (let i = 0; i < 300; i++) total += runInNewContext('n * n', { n: i }); console.log(total);";

fn run_lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    let _ = it.run(src);
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
        let mut it = Interp::new();
        let _ = it.run(&format!("{VM_PRELUDE}\n{MANY_CONTEXTS}"));
        let pending = it.work_pending();
        let stdin = crate::sysio::stdin_active();
        let loop_ok = it.run_event_loop().is_ok();
        let _ = tx.send((it.out.clone(), pending, stdin, loop_ok));
    });
    let (out, pending, stdin, loop_ok) = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("contexts run and the event loop returns within 30 s");
    worker.join().expect("worker thread");
    assert_eq!(out, vec!["8955050".to_string()]);
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
    let slot = it.vm.values_mut().next().expect("one context");
    let realm = weak(
        &slot
            .realm_mut()
            .expect("context realm built")
            .interp
            .global_object,
    );
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
