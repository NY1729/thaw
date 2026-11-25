//! The automatic trigger (PR-2): the environment rule, the threshold policy, the safe point, bounded growth with no
//! manual collection, and the negative scenarios of `leak_audit_tests` re-run with the trigger and no explicit
//! `collect_cycles` call. The stress interval is set in-process (`stress_for_test`), so these run in the default
//! `cargo test` without any environment variable.
use super::*;
use crate::gc::{collections, force_enabled, no_auto, parse_env, stress_for_test};
use crate::gc_registry::{arm, check_invariants, counts, gc_at_for_test, threshold};
use crate::mem_trace::live_counts;

fn run(it: &mut Interp, src: &str) -> Value {
    match it.run(src) {
        Ok(v) => v,
        Err(Ctl::Throw(v)) => {
            let m = it.get(&v, "message").unwrap_or(Value::Undefined);
            panic!("script threw {}: {src}", it.to_str(&m))
        }
        Err(_) => panic!("stray completion: {src}"),
    }
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        _ => panic!("expected a number"),
    }
}

fn ok<T>(r: R<T>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(_) => panic!("{what} threw"),
    }
}

#[test]
fn gc_trigger_parse_env_table() {
    assert_eq!(parse_env(None, None), (true, None));
    assert_eq!(parse_env(Some("1"), None), (true, None));
    assert_eq!(parse_env(Some("0"), None), (false, None));
    // Stress implies the trigger on, even with the kill switch set.
    assert_eq!(parse_env(Some("0"), Some("7")), (true, Some(7)));
    assert_eq!(parse_env(None, Some("200")), (true, Some(200)));
    assert_eq!(parse_env(None, Some(" 3 ")), (true, Some(3)));
    // Only n >= 1 counts; anything unparsable is ignored.
    assert_eq!(parse_env(None, Some("0")), (true, None));
    assert_eq!(parse_env(None, Some("x")), (true, None));
}

#[test]
fn gc_trigger_threshold_policy_table() {
    assert_eq!(threshold(0, 1_234), 0, "stress: due at every registration");
    assert_eq!(
        threshold(50_000, 10),
        50_000,
        "the floor holds for a small heap"
    );
    assert_eq!(
        threshold(50_000, 40_000),
        80_000,
        "2 x live above the floor"
    );
    assert_eq!(threshold(50_000, 300_000), 600_000);
    assert_eq!(
        threshold(usize::MAX, 5),
        usize::MAX,
        "a disarmed list stays disarmed"
    );
}

#[test]
fn gc_trigger_safe_point_rearms_when_nothing_is_freed() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(None);
    arm(1_000);
    run(
        &mut it,
        "var keep=[]; for(let i=0;i<100;i++) keep.push({i:i});",
    );
    let c0 = collections();
    it.gc_safe_point();
    assert_eq!(collections(), c0 + 1, "the safe point collects");
    let (objs, _) = counts();
    // Every node is still reachable, so the thresholds are re-set from the live counts after the attempt.
    assert_eq!(gc_at_for_test().0, threshold(1_000, objs));
    assert!(
        !crate::gc_registry::due(),
        "the due flag is cleared by the safe point"
    );
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_auto_flat_cycles_without_manual_collect() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(None);
    it.collect_cycles();
    let base = counts().0;
    arm(5_000);
    let c0 = collections();
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<200000;i++) mk();",
    );
    assert!(collections() > c0, "the trigger ran at least once");
    let live = counts().0;
    assert!(
        live < 3 * (base + 5_000),
        "live {live} is not bounded by the threshold (base {base})"
    );
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_infinite_cycle_loop_is_bounded() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(None);
    run(
        &mut it,
        "var n=0; for(;;){ var o={}; o.f=function(){return o}; if(++n>300000) break }",
    );
    let (objs, scopes) = counts();
    let (gc_objs, gc_scopes) = gc_at_for_test();
    assert!(
        objs <= gc_objs + 16,
        "objects {objs} exceed the threshold {gc_objs}"
    );
    assert!(
        scopes <= gc_scopes + 16,
        "scopes {scopes} exceed the threshold {gc_scopes}"
    );
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_no_storm_on_a_growing_live_set() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(None);
    let c0 = collections();
    run(&mut it, "var a=[]; for(let i=0;i<500000;i++) a.push({});");
    let runs = collections() - c0;
    // Thresholds 50k, 100k, 200k, 400k: about log2(500k / 50k) collections, none of them freeing anything.
    assert!(runs <= 8, "{runs} collections for a live set of 500k");
    assert!(runs >= 1, "the trigger never ran");
    assert_eq!(num(&run(&mut it, "a.length")), 500_000.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_off_restores_unbounded_growth() {
    force_enabled();
    let mut it = Interp::new();
    // The threshold is never reached: the same state the kill switch (`THAW_JS_GC=0`) leaves the thread in.
    arm(usize::MAX);
    let c0 = collections();
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<200000;i++) mk();",
    );
    assert_eq!(collections(), c0, "no collection while the trigger is off");
    assert!(
        counts().0 >= 200_000,
        "cycles accumulate without the trigger"
    );
    stress_for_test(None);
    it.collect_cycles();
}

#[test]
fn gc_trigger_stress_seven_keeps_results() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(7));
    let c0 = collections();
    let v = run(
        &mut it,
        "var sum=0; for(let i=0;i<3000;i++){ var o={a:i}; o.f=()=>o; sum+=o.f().a } sum",
    );
    assert_eq!(num(&v), (0..3000).sum::<i32>() as f64);
    assert!(
        collections() - c0 >= 400,
        "stress 7 collects every 7th safe point"
    );
    check_invariants().unwrap();
}

// ---- the negative scenarios of leak_audit_tests, with the trigger collecting at every safe point (n = 1) and no
// explicit `collect_cycles` call ----

#[test]
fn gc_trigger_global_holder_survives() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<100;i++) mk(); globalThis.keep=mk();",
    );
    assert!(collections() > 0);
    assert!(matches!(run(&mut it, "keep.f()===keep"), Value::Bool(true)));
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_pending_timer_keeps_cycle() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; o.a=1; setTimeout(function(){ globalThis.out=o.a },0) } for(let i=0;i<50;i++) mk();",
    );
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(num(&run(&mut it, "out")), 1.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_native_held_values_survive() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    let held = run(
        &mut it,
        "(function(){ var o={}; o.f=()=>o; o.a=9; return [o].map(x=>x)[0] })()",
    );
    let stored = run(
        &mut it,
        "(function(){ var o={}; o.f=()=>o; o.a=4; return [o].map(x=>x)[0] })()",
    );
    it.callback_values.insert(usize::MAX, stored);
    run(
        &mut it,
        "function mk(){ var p={}; p.f=()=>p; return p.f() } for(let i=0;i<200;i++) mk();",
    );
    assert!(collections() > 0);
    let f = ok(it.get(&held, "f"), "get f");
    let back = ok(it.call(&f, &Value::Undefined, &[]), "call f");
    assert_eq!(num(&ok(it.get(&back, "a"), "get a")), 9.0);
    let stored = it.callback_values[&usize::MAX].clone();
    assert_eq!(num(&ok(it.get(&stored, "a"), "get stored a")), 4.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_suspended_generator_frame_survives() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    run(
        &mut it,
        "function* g(){ var o={}; o.f=()=>o; o.a=7; yield 1; yield o.a } globalThis.gi=g(); gi.next();",
    );
    run(
        &mut it,
        "function mk(){ var p={}; p.f=()=>p; return p.f() } for(let i=0;i<100;i++) mk();",
    );
    assert_eq!(num(&run(&mut it, "gi.next().value")), 7.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_pending_promise_reaction_keeps_cycle() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; o.a=5; new Promise(r=>setTimeout(r,0)).then(()=>{ globalThis.out=o.a }) } for(let i=0;i<20;i++) mk();",
    );
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(num(&run(&mut it, "out")), 5.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_executing_closure_frame_survives() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    let v = run(
        &mut it,
        "function f(){ var o={}; o.f=()=>o; o.a=7; for(let i=0;i<50;i++){ var p={}; p.g=()=>p } return o.f().a } f()",
    );
    assert_eq!(num(&v), 7.0);
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_map_cycle_is_kept_and_usable() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    run(
        &mut it,
        "function mk(){ var o={}; var m=new Map(); m.set(1,o); o.m=m; return o } \
         globalThis.k=mk(); for(let i=0;i<100;i++){ mk(); }",
    );
    assert!(matches!(run(&mut it, "k.m.get(1)===k"), Value::Bool(true)));
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_borrowed_cell_survives_safe_point() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    let proto = it.object_proto.clone();
    let a = new_obj(Some(proto.clone()), Kind::Plain);
    let b = new_obj(Some(proto.clone()), Kind::Plain);
    crate::put(&a, "p", Value::Object(b.clone()));
    crate::put(&b, "q", Value::Object(a.clone()));
    it.gc_safe_point();
    let before = live_counts();
    {
        // `b` is opaque while its kind is borrowed: it is a root, and so is `a` through it.
        let _guard = b.kind.borrow_mut();
        drop(a);
        it.gc_safe_point();
        assert_eq!(live_counts(), before, "a borrowed node is not cleared");
    }
    drop(b);
    it.gc_safe_point();
    assert_eq!(live_counts().0, before.0 - 2, "the released cycle is freed");
    check_invariants().unwrap();
}

#[test]
fn gc_trigger_disabled_by_no_auto_for_exact_count_tests() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(Some(1));
    no_auto();
    let c0 = collections();
    run(
        &mut it,
        "function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<100;i++) mk();",
    );
    assert_eq!(collections(), c0, "no_auto: safe points do not collect");
    it.collect_cycles();
    stress_for_test(None);
}

/// The `gc_flat_*` probes of leak_audit_tests (copied: their strings are local to that module), run with the trigger
/// collecting at every 7th safe point. Their garbage is freed by the trigger during the loop; an explicit collection
/// at the end must find nothing left, and the live counts must equal the base taken after the definitions.
#[test]
fn gc_trigger_stress_seven_flat_probes() {
    const LOOP: &str = "for(let i=0;i<2000;i++){ mk(); }";
    let probes: [&str; 4] = [
        "const N=2000; function mk(){ var o={a:1}; o.f=function(){return o}; return o }",
        "const N=2000; function mk(){ let o={a:1}; o.f=function(){return o}; return 0 }",
        "const N=2000; function mk(){ var o={a:1}; o.m=function(){ this.f=()=>this; }; o.m(); return 0 }",
        "const N=2000; function mk(){ var o={}; o.o=o; return 0 }",
    ];
    for defs in probes {
        force_enabled();
        let mut it = Interp::new();
        run(&mut it, defs);
        it.collect_cycles();
        let base = live_counts();
        stress_for_test(Some(7));
        let c0 = collections();
        run(&mut it, LOOP);
        assert!(
            collections() > c0,
            "{defs}: the trigger ran during the loop"
        );
        stress_for_test(None);
        it.collect_cycles();
        assert_eq!(live_counts(), base, "{defs}: live counts after the loop");
        check_invariants().unwrap();
    }
}

#[test]
fn gc_trigger_default_floor_collects_cycles_and_keeps_live_data() {
    force_enabled();
    let mut it = Interp::new();
    stress_for_test(None);
    // Re-arm at the production floor so a lower floor is exercised, not a test-set threshold.
    arm(15_000);
    let c0 = collections();
    run(
        &mut it,
        "var keep=[]; for(let i=0;i<20;i++) keep.push({i, self:null}); keep.forEach(o=>o.self=o);
         function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<100000;i++) mk();",
    );
    assert!(collections() > c0, "the lowered floor never collected");
    let (objs, _) = counts();
    assert!(objs < 60_000, "cycles were not freed: {objs} live objects");
    assert_eq!(num(&run(&mut it, "keep.length")), 20.0);
    assert_eq!(
        num(&run(&mut it, "keep.every((o,i)=>o.i===i&&o.self===o)?1:0")),
        1.0
    );
    check_invariants().unwrap();
}
