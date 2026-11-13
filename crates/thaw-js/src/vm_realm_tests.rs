//! `node:vm` context realms are lean (`Interp::new_vm_realm`): cheap to build, ES globals only,
//! and they leave no stdin or event-loop work behind.

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
    it.out
}

#[test]
fn vm_300_contexts_run_once_each_finish_fast() {
    let t = Instant::now();
    let out = run_lines(&format!("{VM_PRELUDE}\n{MANY_CONTEXTS}"));
    assert_eq!(out, vec!["8955050".to_string()]);
    assert!(
        t.elapsed() < Duration::from_secs(20),
        "300 contexts took {:?}",
        t.elapsed()
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
