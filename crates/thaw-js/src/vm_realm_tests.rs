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
