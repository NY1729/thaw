//! Cycle collector on a real package: the yup registry bundle (`thaw_modules/yup/bundle.js.gz`, gunzipped to
//! `GC_YUP_BUNDLE`), driven by `gc_yup_workload.js`. A manual safety gate, not part of the default test run: every
//! test is `#[ignore]`d and needs the environment variable `GC_YUP_BUNDLE` (and optionally `GC_YUP_OUT`, a directory
//! for the two transcripts). Run it with `cargo test -p thaw-js --lib gc_yup -- --ignored --test-threads=1` after any
//! change to `gc.rs`, and before a collection trigger is wired in. The transcript was also checked against real
//! Node 22 with yup 1.7.1 (line for line, apart from a trailing newline).
//!
//! - `gc_yup_transcript_equal`: the 3000-iteration validation transcript with `__gc()` a no-op equals the one with
//!   `__gc()` running `collect_cycles` (every 100 iterations, at random points, and inside custom validation tests).
//!   Every collection also takes a full walk of the heap reachable from the global object before and after, and
//!   asserts that no reachable node changed shape (a freed-but-reachable node would have its contents emptied).
//! - `gc_yup_freed_fraction`: live object/scope/closure counts before and after one collection at N=1000.
//! - `gc_yup_collect_cost` (ignored): time and peak-RSS delta of one collection on 1e5 and 1e6 live nodes.
use super::*;
use crate::eventloop::PState;
use crate::gc::force_enabled;
use crate::gc_registry::{check_invariants, counts};
use crate::generators::GStatus;
use crate::host::Package;
use crate::mem_trace::{census_for_tests, census_snapshot, live_counts};
use crate::{Kind, Obj, Scope, Slot};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

const WORKLOAD: &str = include_str!("gc_yup_workload.js");
const ASYNC_WORKLOAD: &str = include_str!("gc_yup_async_workload.js");
const SCHEMA: &str = "const yup = require('yup'); \
    globalThis.__s = yup.object({ name: yup.string().required().min(2), \
    age: yup.number().integer().min(0).required(), tags: yup.array(yup.string()).default([]) });";

/// The gunzipped yup registry bundle. There is no default: these tests are manual safety gates (see the module doc),
/// so a missing variable is reported instead of reading a path that only exists on one machine.
fn bundle_path() -> String {
    std::env::var("GC_YUP_BUNDLE").unwrap_or_else(|_| {
        panic!("set GC_YUP_BUNDLE to the gunzipped thaw_modules/yup/bundle.js.gz (yup 1.7.1)")
    })
}

/// Where to save the two transcripts for diffing; unset means nothing is written.
fn out_dir() -> Option<String> {
    std::env::var("GC_YUP_OUT").ok()
}

/// Loads the yup bundle the way `thaw-js-node --pkg yup=...` does, and serves it as `require('yup')`.
fn load_yup() -> Package {
    let text = std::fs::read_to_string(bundle_path()).expect("read yup bundle");
    let mut pkg = Package::load(&text).unwrap_or_else(|_| panic!("load yup bundle"));
    let exports = pkg.exports();
    pkg.interp()
        .install_require(vec![("yup".to_string(), exports)]);
    pkg
}

// ---- heap walk: shape of every node reachable from the global object and the global scope ----

enum N {
    Obj(Obj),
    Scope(Rc<Scope>),
}

fn ptr_of<T>(r: &Rc<T>) -> usize {
    Rc::as_ptr(r) as usize
}

fn vrepr(v: &Value, stack: &mut Vec<N>) -> String {
    match v {
        Value::Undefined => "u".into(),
        Value::Null => "null".into(),
        Value::Bool(b) => format!("b{b}"),
        Value::Number(n) => format!("n{:x}", n.to_bits()),
        Value::Str(s) => format!("s{:?}", &**s),
        Value::BigInt(b) => format!("big{b}"),
        Value::Object(o) => {
            stack.push(N::Obj(o.clone()));
            format!("o{:x}", ptr_of(o))
        }
        Value::Hole => "hole".into(),
    }
}

fn obj_shape(o: &Obj, stack: &mut Vec<N>) -> String {
    let mut s = format!("O{:x}", ptr_of(o));
    match o.raw_props.try_borrow() {
        Ok(props) => {
            for p in props.iter() {
                s += &format!(
                    "|{}:{}{}{}",
                    p.key, p.enumerable, p.writable, p.configurable
                );
                match &p.slot {
                    Slot::Data(v) => s += &format!("d{}", vrepr(v, stack)),
                    Slot::Accessor { get, set } => {
                        let g = get.as_ref().map(|v| vrepr(v, stack)).unwrap_or_default();
                        let t = set.as_ref().map(|v| vrepr(v, stack)).unwrap_or_default();
                        s += &format!("a{g},{t}");
                    }
                    Slot::LazyProto { .. } => s += "lazy",
                }
            }
        }
        Err(_) => s += "BORROWED",
    }
    match o.proto.try_borrow() {
        Ok(p) => match p.as_ref() {
            Some(p) => {
                stack.push(N::Obj(p.clone()));
                s += &format!("#o{:x}", ptr_of(p));
            }
            None => s += "#none",
        },
        Err(_) => s += "#BORROWED",
    }
    match o.kind.try_borrow() {
        Ok(kind) => match &*kind {
            Kind::Array(items) => {
                s += "[";
                for v in items {
                    s += &vrepr(v, stack);
                    s += ",";
                }
                s += "]";
            }
            Kind::Closure { def, scope } => {
                stack.push(N::Scope(scope.clone()));
                s += &format!("F{:x}:{:x}", ptr_of(def), ptr_of(scope));
            }
            Kind::Wrapper(v) => s += &format!("W{}", vrepr(v, stack)),
            Kind::Native(_) => s += "native",
            Kind::Promise(d) => {
                s += "P";
                match &d.state {
                    PState::Pending => s += "pend",
                    PState::Fulfilled(v) => s += &format!("ful{}", vrepr(v, stack)),
                    PState::Rejected(v) => s += &format!("rej{}", vrepr(v, stack)),
                }
                for r in &d.reactions {
                    s += &format!("R{},{}", vrepr(&r.on_ful, stack), vrepr(&r.on_rej, stack));
                    if let Some(o) = &r.result {
                        stack.push(N::Obj(o.clone()));
                        s += &format!("r{:x}", ptr_of(o));
                    }
                    if let Some(f) = &r.frame {
                        s += &format!("f{}", vrepr(f, stack));
                    }
                }
            }
            Kind::Generator(g) => {
                s += "G";
                s += match g.status.get() {
                    GStatus::Start => "start",
                    GStatus::Suspended => "susp",
                    GStatus::Running => "run",
                    GStatus::Done => "done",
                };
                match g.co.try_borrow() {
                    Ok(co) => s += if co.is_some() { "co" } else { "noco" },
                    Err(_) => s += "coBORROWED",
                }
                match g.start.try_borrow() {
                    Ok(st) => {
                        if let Some(st) = st.as_ref() {
                            s += &format!("f{}", vrepr(&st.f, stack));
                            stack.push(N::Scope(st.frame.clone()));
                            s += &format!("fr{:x}", ptr_of(&st.frame));
                        }
                    }
                    Err(_) => s += "startBORROWED",
                }
                match g.queue.try_borrow() {
                    Ok(q) => {
                        for (_, v, p) in q.iter() {
                            s += &vrepr(v, stack);
                            stack.push(N::Obj(p.clone()));
                            s += &format!("q{:x}", ptr_of(p));
                        }
                    }
                    Err(_) => s += "queueBORROWED",
                }
            }
            other => s += &format!("K{:?}", std::mem::discriminant(other)),
        },
        Err(_) => s += "KBORROWED",
    }
    s
}

fn scope_shape(sc: &Rc<Scope>, stack: &mut Vec<N>) -> String {
    let mut s = format!("S{:x}", ptr_of(sc));
    if let Some(p) = &sc.parent {
        stack.push(N::Scope(p.clone()));
        s += &format!("p{:x}", ptr_of(p));
    }
    if let Some(w) = &sc.with {
        stack.push(N::Obj(w.clone()));
        s += &format!("with{:x}", ptr_of(w));
    }
    match sc.this_val.try_borrow() {
        Ok(t) => {
            if let Some(v) = t.as_ref() {
                s += &format!("this{}", vrepr(v, stack));
            }
        }
        Err(_) => s += "thisBORROWED",
    }
    match sc.vars.try_borrow() {
        Ok(vars) => {
            for (name, b) in vars.list.iter() {
                s += &format!("|{}={}", name, vrepr(&b.value, stack));
            }
        }
        Err(_) => s += "varsBORROWED",
    }
    s
}

/// Shape of every node reachable from `it.global_object` and the global scope's bindings, keyed by address.
/// Holds no strong handles once it returns.
fn walk_shapes(it: &Interp) -> BTreeMap<usize, String> {
    let mut stack: Vec<N> = vec![N::Obj(it.global_object.clone())];
    {
        let vars = it.global.vars.borrow();
        for (_, b) in vars.list.iter() {
            if let Value::Object(o) = &b.value {
                stack.push(N::Obj(o.clone()));
            }
        }
    }
    let mut shapes = BTreeMap::new();
    while let Some(n) = stack.pop() {
        let (key, shape) = match n {
            N::Obj(o) => {
                let k = ptr_of(&o);
                if shapes.contains_key(&k) {
                    continue;
                }
                (k, obj_shape(&o, &mut stack))
            }
            N::Scope(sc) => {
                let k = ptr_of(&sc);
                if shapes.contains_key(&k) {
                    continue;
                }
                (k, scope_shape(&sc, &mut stack))
            }
        };
        shapes.insert(key, shape);
    }
    shapes
}

/// Ok when the reachable heap is unchanged by a collection; otherwise the first node whose shape changed.
fn compare_walks(
    before: &BTreeMap<usize, String>,
    after: &BTreeMap<usize, String>,
) -> Result<(), String> {
    if before.len() != after.len() {
        return Err(format!(
            "reachable node count {} -> {}",
            before.len(),
            after.len()
        ));
    }
    for (k, b) in before {
        match after.get(k) {
            None => return Err(format!("node {k:x} unreachable after collect: {b}")),
            Some(a) if a != b => {
                return Err(format!("node {k:x} changed:\n  before {b}\n  after  {a}"))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

// ---- the __gc native and its bookkeeping ----

thread_local! {
    static CALLS: Cell<usize> = const { Cell::new(0) };
    static CLEARED: Cell<usize> = const { Cell::new(0) };
    static MAX_REACHABLE: Cell<usize> = const { Cell::new(0) };
    static VIOLATIONS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Installs `__gc`: a no-op returning 0 when `collect` is false; otherwise the poison-checked collection.
fn install_gc(it: &mut Interp, collect: bool) {
    let f = it.native(move |it, _, _| {
        CALLS.with(|c| c.set(c.get() + 1));
        if !collect {
            return Ok(Value::Number(0.0));
        }
        let before = walk_shapes(it);
        MAX_REACHABLE.with(|m| m.set(m.get().max(before.len())));
        let cleared = it.collect_cycles();
        let after = walk_shapes(it);
        if let Err(e) = compare_walks(&before, &after) {
            VIOLATIONS.with(|v| v.borrow_mut().push(e));
        }
        CLEARED.with(|c| c.set(c.get() + cleared));
        Ok(Value::Number(cleared as f64))
    });
    it.global.declare("__gc", f, false);
}

fn reset_bookkeeping() {
    CALLS.with(|c| c.set(0));
    CLEARED.with(|c| c.set(0));
    MAX_REACHABLE.with(|c| c.set(0));
    VIOLATIONS.with(|v| v.borrow_mut().clear());
}

/// Runs the workload; returns the transcript.
fn run_workload(collect: bool) -> String {
    reset_bookkeeping();
    let mut pkg = load_yup();
    let it = pkg.interp();
    install_gc(it, collect);
    let v = match it.run(WORKLOAD) {
        Ok(v) => v,
        Err(_) => panic!("workload threw"),
    };
    let text = it.to_str(&v).to_string();
    drop(pkg);
    text
}

#[test]
#[ignore = "manual safety gate (about 3.5 minutes in a debug build): needs GC_YUP_BUNDLE, run with --ignored before wiring any collection trigger"]
fn gc_yup_transcript_equal() {
    force_enabled();
    let plain = run_workload(false);
    let plain_calls = CALLS.with(|c| c.get());
    let plain_collections = crate::gc::collections();
    let plain_peak = crate::gc::peak_live();
    let collected = run_workload(true);
    let calls = CALLS.with(|c| c.get());
    let cleared = CLEARED.with(|c| c.get());
    let max_reach = MAX_REACHABLE.with(|c| c.get());
    let violations = VIOLATIONS.with(|v| v.borrow().clone());
    eprintln!(
        "gc_yup: no-explicit-__gc run: automatic collections {plain_collections}, peak registry live count at a safe point {plain_peak}"
    );
    if let Some(dir) = out_dir() {
        std::fs::write(format!("{dir}/transcript-nogc.txt"), &plain).ok();
        std::fs::write(format!("{dir}/transcript-gc.txt"), &collected).ok();
    }
    eprintln!(
        "gc_yup: transcript lines {}, __gc calls {} (no-op run {}), nodes cleared {}, reachable nodes (max) {}, poison violations {}",
        plain.lines().count(),
        calls,
        plain_calls,
        cleared,
        max_reach,
        violations.len()
    );
    assert!(
        violations.is_empty(),
        "poison check failed: {}",
        violations[0]
    );
    assert!(
        calls >= 60 && cleared > 0,
        "the collector did not run (calls {calls}, cleared {cleared})"
    );
    if plain != collected {
        let (i, (a, b)) = plain
            .lines()
            .zip(collected.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .unwrap_or((usize::MAX, ("", "")));
        panic!("transcripts differ at line {i}:\n  nogc: {a}\n  gc:   {b}");
    }
    assert_eq!(plain.len(), collected.len());
    check_invariants().unwrap_or_else(|e| panic!("registry invariants: {e}"));
}

/// The async validation transcript with `__gc()` a no-op, or running `collect_cycles` (the workload calls `__gc` every
/// 100 iterations, at random points, and inside its async custom tests). The event loop drives the run to completion.
fn run_async_workload(collect: bool) -> String {
    reset_bookkeeping();
    let mut pkg = load_yup();
    let it = pkg.interp();
    install_gc(it, collect);
    if it.run(ASYNC_WORKLOAD).is_err() {
        panic!("async workload threw");
    }
    if it.run_event_loop().is_err() {
        panic!("async workload: the event loop reported an error");
    }
    let global = Value::Object(it.global_object.clone());
    let v = match it.get(&global, "__transcript") {
        Ok(v) => v,
        Err(_) => panic!("async workload left no transcript"),
    };
    let text = it.to_str(&v).to_string();
    drop(pkg);
    text
}

#[test]
#[ignore = "manual safety gate (async, N=3000): needs GC_YUP_BUNDLE; with GC_YUP_ASYNC_ORACLE set, also compares with the Node 22 transcript"]
fn gc_yup_async_transcript_equal() {
    force_enabled();
    let plain = run_async_workload(false);
    let plain_collections = crate::gc::collections();
    let plain_peak = crate::gc::peak_live();
    let collected = run_async_workload(true);
    let calls = CALLS.with(|c| c.get());
    let cleared = CLEARED.with(|c| c.get());
    let max_reach = MAX_REACHABLE.with(|c| c.get());
    let violations = VIOLATIONS.with(|v| v.borrow().clone());
    if let Some(dir) = out_dir() {
        std::fs::write(format!("{dir}/async-transcript-nogc.txt"), &plain).ok();
        std::fs::write(format!("{dir}/async-transcript-gc.txt"), &collected).ok();
    }
    eprintln!(
        "gc_yup async: transcript lines {}, automatic collections in the no-op run {plain_collections}, peak live at a safe point {plain_peak}; __gc calls {calls}, nodes cleared {cleared}, reachable nodes (max) {max_reach}, poison violations {}",
        plain.lines().count(),
        violations.len()
    );
    assert!(
        violations.is_empty(),
        "poison check failed: {}",
        violations[0]
    );
    assert!(
        calls >= 60 && cleared > 0,
        "the collector did not run (calls {calls}, cleared {cleared})"
    );
    assert!(
        plain == collected,
        "async transcripts differ (first differing line {:?})",
        plain
            .lines()
            .zip(collected.lines())
            .position(|(a, b)| a != b)
    );
    if let Ok(oracle) = std::env::var("GC_YUP_ASYNC_ORACLE") {
        let node = std::fs::read_to_string(&oracle).unwrap_or_else(|_| panic!("read {oracle}"));
        assert!(
            node.trim_end() == collected.trim_end(),
            "the thaw-js transcript differs from the Node 22 transcript in {oracle} (first differing line {:?})",
            node.lines()
                .zip(collected.lines())
                .position(|(a, b)| a != b)
        );
    }
    check_invariants().unwrap_or_else(|e| panic!("registry invariants: {e}"));
}

/// Live counts before and after one collection of 1000 async validations (`await validate`), driven by the event loop.
#[test]
#[ignore = "manual safety gate: needs GC_YUP_BUNDLE (the gunzipped yup registry bundle)"]
fn gc_yup_async_freed_fraction() {
    force_enabled();
    let mut pkg = load_yup();
    let it = pkg.interp();
    install_gc(it, false);
    let defs = |it: &mut Interp, src: &str| match it.run(src) {
        Ok(v) => v,
        Err(_) => panic!("script threw: {src}"),
    };
    defs(it, SCHEMA);
    // A realm runs its event loop once (a later call returns at once), so the loop is driven by a single call.
    it.collect_cycles();
    let base = live_counts();
    let loop_src = "globalThis.done=false; (async function(){ for (let i = 0; i < 1000; i++) { \
        await __s.validate({ name: 'n'.repeat(i % 5), age: (i % 9) - 2 }, { abortEarly: false }).catch(() => 0); } \
        globalThis.done=true; })();";
    defs(it, loop_src);
    assert!(it.run_event_loop().is_ok(), "run_event_loop");
    let grown = live_counts();
    let done = defs(it, "globalThis.done");
    let cleared = it.collect_cycles();
    let after = live_counts();
    let f = (grown.0 - after.0, grown.1 - after.1, grown.2 - after.2);
    eprintln!(
        "gc_yup async N=1000 (done={}): base {base:?}, growth obj/scope/closure {} / {} / {}, freed {} / {} / {} (cleared nodes {cleared}), fraction obj {:.1}%",
        matches!(done, Value::Bool(true)),
        grown.0 - base.0,
        grown.1 - base.1,
        grown.2 - base.2,
        f.0,
        f.1,
        f.2,
        if grown.0 > base.0 {
            100.0 * f.0 as f64 / (grown.0 - base.0) as f64
        } else {
            0.0
        }
    );
    check_invariants().unwrap_or_else(|e| panic!("registry invariants: {e}"));
}

/// Negative control for the poison check: clearing a reachable node's props (what a wrong collection would do) must
/// be reported by `compare_walks`.
#[test]
#[ignore = "manual safety gate: needs GC_YUP_BUNDLE (the gunzipped yup registry bundle)"]
fn gc_yup_poison_check_detects_cleared_reachable_node() {
    force_enabled();
    let mut pkg = load_yup();
    let it = pkg.interp();
    let inner = match it.run("globalThis.keep = { a: { b: 1 } }; globalThis.keep.a") {
        Ok(Value::Object(o)) => o,
        _ => panic!("setup threw"),
    };
    let before = walk_shapes(it);
    assert!(compare_walks(&before, &before).is_ok());
    inner.raw_props.borrow_mut().clear();
    let after = walk_shapes(it);
    let err = compare_walks(&before, &after);
    assert!(err.is_err(), "poison check missed a cleared reachable node");
    eprintln!(
        "gc_yup control: {}",
        err.unwrap_err().lines().next().unwrap_or("")
    );
}

// ---- freed fraction at N=1000 ----

#[test]
#[ignore = "manual safety gate: needs GC_YUP_BUNDLE (the gunzipped yup registry bundle)"]
fn gc_yup_freed_fraction() {
    force_enabled();
    let mut pkg = load_yup();
    let it = pkg.interp();
    install_gc(it, false);
    let defs = |it: &mut Interp, src: &str| match it.run(src) {
        Ok(v) => v,
        Err(_) => panic!("script threw: {src}"),
    };
    defs(it, SCHEMA);
    defs(it, "__s.isValidSync({name:'warm', age:1})");
    it.collect_cycles();
    let base = live_counts();
    census_for_tests(true);
    let c0 = census_snapshot();

    let loop_src = "for (let i = 0; i < 1000; i++) { __s.isValidSync({ name: 'n'.repeat(i % 5), age: (i % 9) - 2 }); }";
    defs(it, loop_src);
    let grown = live_counts();
    let c1 = census_snapshot();
    let cleared = it.collect_cycles();
    let after = live_counts();
    let census: Vec<usize> = (0..5).map(|i| c1[i] - c0[i]).collect();

    let freed =
        |g: (isize, isize, isize), a: (isize, isize, isize)| (g.0 - a.0, g.1 - a.1, g.2 - a.2);
    let f = freed(grown, after);
    eprintln!("gc_yup N=1000 base {base:?}");
    eprintln!(
        "gc_yup N=1000 grown {grown:?} (per iter obj/scope/closure {:.1}/{:.1}/{:.1})",
        (grown.0 - base.0) as f64 / 1000.0,
        (grown.1 - base.1) as f64 / 1000.0,
        (grown.2 - base.2) as f64 / 1000.0
    );
    eprintln!("gc_yup N=1000 after one collect {after:?} cleared nodes {cleared}");
    eprintln!(
        "gc_yup N=1000 freed obj {} scope {} closure {} of growth obj {} scope {} closure {}",
        f.0,
        f.1,
        f.2,
        grown.0 - base.0,
        grown.1 - base.1,
        grown.2 - base.2
    );
    eprintln!(
        "gc_yup census(this loop): closures {} chain_empty {} used_only {} unused_only {} mixed {}",
        census[0], census[1], census[2], census[3], census[4]
    );
    eprintln!(
        "gc_yup used-name closures (used_only+mixed) = {}",
        census[2] + census[4]
    );

    // Residual growth per iteration with collection at every 1000 (second loop, measured against `after`).
    defs(it, loop_src);
    let grown2 = live_counts();
    it.collect_cycles();
    let after2 = live_counts();
    eprintln!(
        "gc_yup second N=1000 (collect at end): grown {:?} after {after2:?}",
        (grown2.0 - after.0, grown2.1 - after.1, grown2.2 - after.2)
    );
    check_invariants().unwrap_or_else(|e| panic!("registry invariants: {e}"));
    let _ = counts();
}

// ---- cost of one collection on a large live heap (ignored: slow, run with --ignored) ----

fn status_kb(field: &str) -> usize {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with(field))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
#[ignore = "cost measurement: run with --ignored, one size per run"]
fn gc_yup_collect_cost() {
    force_enabled();
    let n: usize = std::env::var("GC_COST_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let mut pkg = load_yup();
    let it = pkg.interp();
    install_gc(it, false);
    let src = format!(
        "globalThis.keep = []; for (let i = 0; i < {n}; i++) {{ keep.push({{ a: i, b: [i] }}); }}"
    );
    if it.run(&src).is_err() {
        panic!("build threw");
    }
    let (o, s) = counts();
    let before_rss = status_kb("VmRSS:");
    std::fs::write("/proc/self/clear_refs", "5").ok();
    let t = std::time::Instant::now();
    let cleared = it.collect_cycles();
    let first = t.elapsed();
    let peak = status_kb("VmHWM:");
    let t = std::time::Instant::now();
    let cleared2 = it.collect_cycles();
    let second = t.elapsed();
    eprintln!(
        "gc_yup cost n={n} live registry objs {o} scopes {s}: first collect {:.1} ms (cleared {cleared}), second {:.1} ms (cleared {cleared2}); peak RSS delta during first collect {} KB (VmRSS before {} KB)",
        first.as_secs_f64() * 1e3,
        second.as_secs_f64() * 1e3,
        peak.saturating_sub(before_rss),
        before_rss
    );
}
