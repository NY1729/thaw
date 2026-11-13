//! The JS job barrier hooks the compiled runtime calls around native continuations: a barrier drains
//! exactly the jobs queued before it (Node 22 order: `await` on a settled promise runs the pending
//! thenable and microtask jobs first). Expectations recorded from `node` 22.23.1.

use super::*;
use crate::host::{default_realm, drain_js_job_barrier, queue_js_job_barrier, with_package};

fn log_of(it: &mut Interp) -> String {
    let v = it.run("log.join(',')").unwrap();
    it.to_str(&v).to_string()
}

#[test]
fn job_barrier_drains_only_jobs_queued_before_it() {
    let mut it = Interp::new();
    it.run(
        "var log = []; \
         Promise.resolve({ then(r) { log.push('then'); r(1); } }); \
         Promise.resolve().then(() => log.push('micro'));",
    )
    .unwrap();
    it.push_job_barrier(7);
    it.run("Promise.resolve().then(() => log.push('later'));")
        .unwrap();
    it.drain_to_job_barrier(7).unwrap();
    assert_eq!(log_of(&mut it), "then,micro");
    // the later job is still queued behind the barrier
    it.run_jobs().unwrap();
    assert_eq!(log_of(&mut it), "then,micro,later");
}

#[test]
fn job_barrier_with_no_marker_drains_nothing() {
    let mut it = Interp::new();
    it.run("var log = []; Promise.resolve().then(() => log.push('x'));")
        .unwrap();
    it.drain_to_job_barrier(9).unwrap();
    assert_eq!(log_of(&mut it), "");
}

#[test]
fn host_barrier_is_zero_when_idle_and_orders_pending_jobs() {
    let h = default_realm();
    // idle realm: nothing to order against
    assert_eq!(queue_js_job_barrier(), 0);
    with_package(h, |p| {
        p.interp
            .run(
                "var log = []; \
                 Promise.resolve({ then(r) { log.push('then'); r(1); } }); \
                 Promise.resolve().then(() => log.push('micro'));",
            )
            .map_err(|c| crate::host::classify(&mut p.interp, c))
    })
    .unwrap();
    let id = queue_js_job_barrier();
    assert_ne!(id, 0);
    with_package(h, |p| {
        p.interp
            .run("Promise.resolve().then(() => log.push('later'));")
            .map_err(|c| crate::host::classify(&mut p.interp, c))
    })
    .unwrap();
    drain_js_job_barrier(id);
    let seen = with_package(h, |p| {
        let v = p.interp.run("log.join(',')").unwrap();
        Ok(p.interp.to_str(&v).to_string())
    })
    .unwrap();
    assert_eq!(seen, "then,micro");
}
