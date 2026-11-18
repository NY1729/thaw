//! Leak audit: build an `Interp`, run a scenario, drop it, and check that what the scenario allocated
//! is freed. The gate is deterministic: the scenario creates `globalThis.__probe` and holds it only
//! through the feature under test. After the harness deletes the global and drops the Interp, the
//! probe must be dead (a survivor is an unwalked holder or a cycle through that feature).
//!
//! RSS (`/proc/self/statm` after `malloc_trim(0)`) is a backstop only: it is noisy (floor about
//! 0.1-0.3 MB per Interp), so those tests are `#[ignore]`d and run with `-- --ignored`.
//!
//! Scenarios that leak today are `#[ignore = "leak: <cause>, fixed by PR <n>"]`; the PR named there
//! removes its ignore. Run filtered only, never the full suite (see the plan in designs/).
use super::*;
use crate::gc::{obj_edges, scope_edges, Edge};
use crate::gc_registry::{check_invariants, counts, snapshot};
use crate::mem_trace::live_counts;
use crate::{Kind, Prop, Scope, Slot};
use std::rc::Weak;

const VM_PRELUDE: &str =
    include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js");
/// Measured Interp drops per counter test (after one warm-up drop, so lazy statics are not counted).
const ITERS: usize = 3;
const RSS_WARM: usize = 4;
const RSS_ITERS: usize = 16;
/// Backstop threshold, MB per Interp above the empty-Interp baseline in the same test.
const RSS_LIMIT_MB: f64 = 0.4;

/// One Interp: run `src`, optionally drain the event loop, delete the probe, drop. True if the probe
/// survived the drop.
fn probe_survives_once(name: &str, src: &str, run_loop: bool) -> bool {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => {
            let m = it.get(&v, "message").unwrap_or(Value::Undefined);
            panic!("{name}: scenario threw {}", it.to_str(&m));
        }
        Err(_) => panic!("{name}: scenario ended with a stray completion"),
    }
    if run_loop {
        it.run_event_loop()
            .unwrap_or_else(|_| panic!("{name}: event loop failed"));
    }
    let gv = Value::Object(it.global_object.clone());
    let probe: Weak<_> = match it.get(&gv, "__probe") {
        Ok(Value::Object(o)) => Rc::downgrade(&o),
        _ => panic!("{name}: scenario did not set globalThis.__probe"),
    };
    let _ = it.run("delete globalThis.__probe");
    drop(it);
    probe.upgrade().is_some()
}

/// Number of measured drops (out of `ITERS`) whose probe survived.
fn probe_survivors(name: &str, src: &str, run_loop: bool) -> usize {
    let _ = probe_survives_once(name, src, run_loop);
    (0..ITERS)
        .filter(|_| probe_survives_once(name, src, run_loop))
        .count()
}

/// Gate: the probe must not survive any measured drop.
fn assert_freed(name: &str, src: &str) {
    let s = probe_survivors(name, src, true);
    assert_eq!(s, 0, "{name}: probe survived {s}/{ITERS} Interp drops");
}

fn assert_freed_without_loop(name: &str, src: &str) {
    let s = probe_survivors(name, src, false);
    assert_eq!(s, 0, "{name}: probe survived {s}/{ITERS} Interp drops");
}

fn rss_mb() -> f64 {
    unsafe { libc::malloc_trim(0) };
    let p = std::fs::read_to_string("/proc/self/statm").unwrap();
    p.split_whitespace().nth(1).unwrap().parse::<f64>().unwrap() * 4096.0 / 1048576.0
}

/// Resident MB added per Interp iteration of `src` (warm-up excluded).
fn rss_per_iter_mb(name: &str, src: &str) -> f64 {
    for _ in 0..RSS_WARM {
        probe_survives_once(name, src, true);
    }
    let start = rss_mb();
    for _ in 0..RSS_ITERS {
        probe_survives_once(name, src, true);
    }
    (rss_mb() - start) / RSS_ITERS as f64
}

/// Backstop: `src` must not cost more than `RSS_LIMIT_MB` per Interp over an empty one.
fn assert_rss_flat(name: &str, src: &str) {
    let base = rss_per_iter_mb("baseline", "globalThis.__probe={}");
    let per = rss_per_iter_mb(name, src);
    assert!(
        per - base < RSS_LIMIT_MB,
        "{name}: {per:.2} MB/iter vs baseline {base:.2} MB/iter"
    );
}

// ---- scenarios that are clean today: run in the default test run ----

#[test]
fn leak_baseline_console_log() {
    assert_freed("baseline", "globalThis.__probe={}; console.log(1)");
}

#[test]
fn leak_timers_cleared() {
    assert_freed(
        "timers_cleared",
        "globalThis.__probe={}; var P=__probe; var t=setTimeout(()=>P,1000); clearTimeout(t); \
         var i=setInterval(()=>P,1000); clearInterval(i); var m=setImmediate(()=>P); clearImmediate(m);",
    );
}

#[test]
fn leak_timers_fired() {
    assert_freed(
        "timers_fired",
        "globalThis.__probe={}; var P=__probe; var n=0; setTimeout(()=>{n+=P?1:0},1); \
         var i=setInterval(()=>{if(++n>3)clearInterval(i)},1); setImmediate(()=>{n++});",
    );
}

#[test]
fn leak_timers_uncleared_dropped_without_loop() {
    // Never drain the loop: an uncleared 100 s timer would block run_event_loop until it fires.
    assert_freed_without_loop(
        "timers_uncleared",
        "globalThis.__probe={}; var P=__probe; setTimeout(()=>P,100000); setInterval(()=>P,100000); \
         setImmediate(()=>P);",
    );
}

#[test]
fn leak_promise_async_await() {
    assert_freed(
        "promise_async",
        "globalThis.__probe={}; var P=__probe; new Promise(()=>{}).then(()=>P); var pr=Promise.resolve(P); \
         pr.self=pr; async function f(n){ for(let i=0;i<n;i++) await Promise.resolve(i); return P } \
         f(50).then(()=>{}); Promise.all([1,2,3].map(async x=>x)).then(()=>{}); \
         Promise.reject(new Error('x')).catch(()=>{});",
    );
}

#[test]
fn leak_generator_run_to_completion() {
    assert_freed(
        "gen_done",
        "globalThis.__probe={}; var P=__probe; function* g(){ var x=P; yield 1; yield 2 } \
         for (const x of g()); var d=g(); d.next(); d.next(); d.next();",
    );
}

#[test]
fn leak_collections_with_cycles() {
    assert_freed(
        "collections",
        "globalThis.__probe={}; var P=__probe; var m=new Map(),s=new Set(),w=new WeakMap(),k={}; \
         m.set('p',P); s.add(P); w.set(k,P); m.set(m,m); m.set(k,m); s.add(s); w.set(k,{k,m}); \
         m.set('f',()=>m); s.add(()=>s); globalThis.keep={m,s,w};",
    );
}

#[test]
fn leak_proxy_reflect() {
    assert_freed(
        "proxy_reflect",
        "globalThis.__probe={}; var P=__probe; var t={a:1,P}; \
         var p=new Proxy(t,{get(o,k,r){return Reflect.get(o,k,r)},has(o,k){return Reflect.has(o,k)}}); \
         p.a; 'a' in p; t.self=p; globalThis.p=p; Reflect.ownKeys(p);",
    );
}

#[test]
fn leak_buffer_typedarray() {
    assert_freed(
        "buffer_typedarray",
        "globalThis.__probe={}; var P=__probe; var b=Buffer.alloc(1<<16); b.p=P; \
         var u=new Uint8Array(b.buffer); var f=new Float64Array(1024); \
         var ab=new ArrayBuffer(1<<16); var dv=new DataView(ab); \
         globalThis.keep=[b,u,f,ab,dv,Buffer.from('abc').toString('base64')];",
    );
}

#[test]
fn leak_url_searchparams() {
    assert_freed(
        "url",
        "globalThis.__probe={}; var P=__probe; var u=new URL('http://a.b/c?d=1#e'); u.p=P; \
         var q=new URLSearchParams('a=1&b=2'); q.append('c','3'); u.searchParams.set('x','y'); \
         globalThis.keep=[u,q,u.href];",
    );
}

#[test]
fn leak_error_cause_and_stack() {
    assert_freed(
        "error_cause",
        "globalThis.__probe={}; var P=__probe; function f(){ throw new Error('a',{cause:new TypeError('b')}) } \
         try{f()}catch(e){ globalThis.keep=[e.stack,e.cause,e.cause.stack]; } \
         var e2=new Error('x'); e2.cause=e2; globalThis.e2=e2; new AggregateError([new Error('1')],'m'); \
         globalThis.keep.push(P);",
    );
}

#[test]
fn leak_error_property_holds_probe() {
    assert_freed(
        "set_error_prop",
        "globalThis.__probe={}; var e=new Error('x'); e.p=__probe; e.stack;",
    );
}

#[test]
fn leak_process_events_and_event_target_once() {
    assert_freed(
        "events_once",
        "globalThis.__probe={}; var P=__probe; process.once('foo',()=>P); process.once('bar',()=>P); \
         process.on('exit',()=>P); process.once('foo',()=>{}); process.emit('foo'); \
         const et=new EventTarget(); et.addEventListener('x',()=>P,{once:true}); \
         et.addEventListener('y',()=>P);",
    );
}

#[test]
fn leak_process_hooks_and_microtasks() {
    assert_freed(
        "process_hooks",
        "globalThis.__probe={}; var P=__probe; process.once('beforeExit',()=>P); \
         process.on('unhandledRejection',()=>P); process.nextTick(()=>P); queueMicrotask(()=>P);",
    );
}

#[test]
fn leak_zlib_native_roundtrip() {
    assert_freed(
        "zlib",
        "globalThis.__probe={}; var P=__probe; var h=__thaw_zlib_hex('compress','gzip',\
         Buffer.from('hello hello hello').toString('hex')); \
         __thaw_zlib_hex('decompress','gzip',h); globalThis.keep=[P];",
    );
}

#[test]
fn leak_crypto_random_and_digest() {
    assert_freed(
        "crypto",
        "globalThis.__probe={}; var P=__probe; crypto.randomUUID(); \
         crypto.getRandomValues(new Uint8Array(16)); \
         crypto.subtle.digest('SHA-256', new Uint8Array(4)).then(()=>P); globalThis.keep=[P];",
    );
}

#[test]
fn leak_net_listen_close_native() {
    assert_freed(
        "net_listen",
        "globalThis.__probe={}; var P=__probe; var l=__thaw_net_listen('127.0.0.1',0).split(':'); \
         __thaw_net_close_listener(Number(l[1])); globalThis.keep=[P];",
    );
}

#[test]
fn leak_worker_threads_message_channel() {
    assert_freed(
        "worker_channel",
        "globalThis.__probe={}; var P=__probe; const { MessageChannel } = __thaw_worker_threads_exports; \
         const c=new MessageChannel(); c.port2.on('message',()=>{c.port1.close();c.port2.close()}); \
         c.port1.postMessage({a:1}); const d=new MessageChannel(); globalThis.keep=[d,P];",
    );
}

#[test]
fn leak_web_streams() {
    assert_freed(
        "web_streams",
        "globalThis.__probe={}; var P=__probe; const rs=new ReadableStream({start(c){c.enqueue(P);c.close()}}); \
         rs.getReader().read().then(()=>{}); new TransformStream(); new WritableStream();",
    );
}

#[test]
fn leak_abort_controller_event_target_blob() {
    assert_freed(
        "abort_event_blob",
        "globalThis.__probe={}; var P=__probe; const ac=new AbortController(); \
         ac.signal.addEventListener('abort',()=>P); const et=new EventTarget(); \
         et.addEventListener('x',()=>P); ac.abort(); new TextEncoder().encode('x'); \
         new Blob(['a']).text().then(()=>{});",
    );
}

#[test]
fn leak_closures_and_class_cycles() {
    assert_freed(
        "closures_cycles",
        "globalThis.__probe={}; var P=__probe; function mk(){ var o={P}; o.f=function(){return o}; return o } \
         globalThis.keep=[mk(),mk()]; class A{ static s=new A(); m(){return A} } globalThis.A=A; \
         var tpl=(s)=>s; tpl`a${P}b`;",
    );
}

// ---- scenarios that leak today: ignored until the named PR lands ----

#[test]
#[ignore = "leak: suspended generator frame not walked by Interp teardown, fixed by PR 2"]
fn leak_generator_suspended() {
    assert_freed(
        "gen_suspended",
        "globalThis.__probe={}; var P=__probe; function* h(){ var x=P; yield 1; yield 2 } var g=h(); g.next();",
    );
}

#[test]
fn leak_generator_unstarted_dropped() {
    assert_freed(
        "gen_unstarted",
        "globalThis.__probe={}; var P=__probe; function* h(){ var x=P; yield 1 } var g=h();",
    );
}

#[test]
#[ignore = "leak: suspended async generator frame not walked by Interp teardown, fixed by PR 2"]
fn leak_async_generator_suspended() {
    assert_freed(
        "asyncgen_suspended",
        "globalThis.__probe={}; var P=__probe; async function* h(){ var x=P; yield 1 } var g=h(); g.next();",
    );
}

#[test]
#[ignore = "leak: structuredClone retains its input graph after drop (holder not yet identified), fixed by PR 4"]
fn leak_structured_clone_graph() {
    assert_freed(
        "structured_clone",
        "globalThis.__probe={}; var P=__probe; var o={a:[1,2,{b:new Map([[1,2]])}],P,d:new Date(),s:new Set([1])}; \
         o.self=o; globalThis.keep=structuredClone(o);",
    );
}

#[test]
#[ignore = "leak: vm realms (10 contexts per Interp) survive host drop, ~0.1 MB per context, fixed by PR 3"]
fn leak_vm_contexts_rss() {
    let src = format!(
        "{VM_PRELUDE}\n for(let i=0;i<10;i++) runInNewContext('n*n',{{n:i}}); \
         globalThis.__probe={{}}; globalThis.keep=runInNewContext('globalThis',{{}});"
    );
    assert_rss_flat("vm_contexts", &src);
}

#[test]
#[ignore = "leak: global scope, global_object and Object.prototype survive Interp drop, fixed by PR 5"]
fn leak_global_residue_baseline() {
    let mut survivors = 0;
    for i in 0..ITERS + 1 {
        let mut it = Interp::new();
        it.run("console.log(1)").ok();
        let scope: Weak<_> = Rc::downgrade(&it.global);
        let global: Weak<_> = Rc::downgrade(&it.global_object);
        let proto: Weak<_> = Rc::downgrade(&it.object_proto);
        drop(it);
        if i > 0 {
            survivors += scope.upgrade().is_some() as usize
                + global.upgrade().is_some() as usize
                + proto.upgrade().is_some() as usize;
        }
    }
    assert_eq!(
        survivors, 0,
        "global residue survived {survivors} Weak checks"
    );
}

// ---- cycle collector (gc.rs): flat after one collection, negative cases, edge audit ----

/// Runs `src` and returns its value; a script error fails the test.
fn run_script(it: &mut Interp, name: &str, src: &str) -> Value {
    match it.run(src) {
        Ok(v) => v,
        Err(_) => panic!("{name}: script failed: {src}"),
    }
}

fn ok<T>(r: R<T>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(_) => panic!("{what} threw"),
    }
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        _ => panic!("expected a number"),
    }
}

/// `defs` runs first; `call` (a loop of garbage cycles) is measured. One collection must free all of it: the live
/// counts return to the base taken after `defs`, and the registry is consistent.
fn assert_flat_after_cycles(name: &str, defs: &str, call: &str) {
    crate::gc::force_enabled();
    // Counts the loop's garbage before the explicit collect: the automatic trigger must not run first.
    crate::gc::no_auto();
    let mut it = Interp::new();
    run_script(&mut it, name, defs);
    // The realm's own start-up garbage (about 8 closures, 8 objects, 1 scope) is freed by the first collection; the
    // base is taken after it, so the check measures the loop alone.
    it.collect_cycles();
    let base = live_counts();
    run_script(&mut it, name, call);
    let grown = live_counts();
    assert!(
        grown.0 > base.0,
        "{name}: the loop made no garbage ({base:?} -> {grown:?})"
    );
    let cleared = it.collect_cycles();
    assert!(
        cleared >= 2000,
        "{name}: collect cleared {cleared} nodes, expected at least 2000"
    );
    assert_eq!(live_counts(), base, "{name}: live counts after collect");
    check_invariants().unwrap_or_else(|e| panic!("{name}: {e}"));
}

const PROBE_LOOP: &str = "for(let i=0;i<2000;i++){ mk(); }";

#[test]
fn gc_flat_v1_used() {
    assert_flat_after_cycles(
        "v1_used",
        "const N=2000; function mk(){ var o={a:1}; o.f=function(){return o}; return o }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_w2_let_used() {
    assert_flat_after_cycles(
        "w2_let_used",
        "const N=2000; function mk(){ let o={a:1}; o.f=function(){return o}; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
#[ignore = "class evaluated per call: its constructor, prototype and field scope are reached through the uncounted \
            FuncDef.class.field_scope edge (classes.rs:12), so each evaluation leaves 2 objects, 3 scopes and 1 closure \
            uncollected (measured: class-only loop cleared 0 of 2000). Instances are freed. PR-1 limit, spec section 7 risk 4"]
fn gc_flat_w4_class() {
    assert_flat_after_cycles(
        "w4_class",
        "const N=2000; function mk(){ class A{ constructor(){ this.self=this; this.f=()=>1 } m(){return 1} } new A(); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_w5_thisarrow() {
    assert_flat_after_cycles(
        "w5_thisarrow",
        "const N=2000; function mk(){ var o={a:1}; o.m=function(){ this.f=()=>this; }; o.m(); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_w6_selfcycle() {
    assert_flat_after_cycles(
        "w6_selfcycle",
        "const N=2000; function mk(){ var o={}; o.o=o; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_global_holder_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "global",
        "function mk(){ var o={}; o.f=()=>o; return o } for(let i=0;i<100;i++) mk(); globalThis.keep=mk();",
    );
    it.collect_cycles();
    assert!(matches!(
        run_script(&mut it, "global", "keep.f()===keep"),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_pending_timer_keeps_cycle() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "timer",
        "function mk(){ var o={}; o.f=()=>o; o.a=1; setTimeout(function(){ globalThis.out=o.a },0) } mk();",
    );
    it.collect_cycles();
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(number(&run_script(&mut it, "timer", "out")), 1.0);
    check_invariants().unwrap();
}

#[test]
fn gc_native_held_values_survive() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    let held = run_script(
        &mut it,
        "held",
        "(function(){ var o={}; o.f=()=>o; o.a=9; return [o].map(x=>x)[0] })()",
    );
    let stored = run_script(
        &mut it,
        "stored",
        "(function(){ var o={}; o.f=()=>o; o.a=4; return [o].map(x=>x)[0] })()",
    );
    it.callback_values.insert(usize::MAX, stored);
    it.collect_cycles();
    let f = ok(it.get(&held, "f"), "get f");
    let back = ok(it.call(&f, &Value::Undefined, &[]), "call f");
    assert_eq!(number(&ok(it.get(&back, "a"), "get a")), 9.0);
    let stored = it.callback_values[&usize::MAX].clone();
    assert_eq!(number(&ok(it.get(&stored, "a"), "get stored a")), 4.0);
    check_invariants().unwrap();
}

#[test]
fn gc_suspended_generator_frame_is_external() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "gen",
        "function* g(){ var o={}; o.f=()=>o; o.a=7; yield 1; yield o.a } globalThis.gi=g(); gi.next();",
    );
    it.collect_cycles();
    assert_eq!(number(&run_script(&mut it, "gen", "gi.next().value")), 7.0);
    check_invariants().unwrap();
}

#[test]
fn gc_pending_promise_reaction_keeps_cycle() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "promise",
        "function mk(){ var o={}; o.f=()=>o; o.a=5; new Promise(r=>setTimeout(r,0)).then(()=>{ globalThis.out=o.a }) } mk();",
    );
    it.collect_cycles();
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(number(&run_script(&mut it, "promise", "out")), 5.0);
    check_invariants().unwrap();
}

#[test]
fn gc_executing_closure_frame_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    let gc_fn = it.native(|it, _, _| Ok(Value::Number(it.collect_cycles() as f64)));
    it.global.declare("__gc", gc_fn, false);
    let v = run_script(
        &mut it,
        "executing",
        "function f(){ var o={}; o.f=()=>o; o.a=7; __gc(); return o.f().a } f()",
    );
    assert_eq!(number(&v), 7.0);
    check_invariants().unwrap();
}

#[test]
fn gc_map_cycle_is_kept_and_usable() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "map",
        "function mk(){ var o={}; var m=new Map(); m.set(1,o); o.m=m; return o } \
         globalThis.k=mk(); for(let i=0;i<100;i++){ mk(); }",
    );
    it.collect_cycles();
    let before = live_counts();
    it.collect_cycles();
    assert_eq!(live_counts(), before, "a Map is opaque: its cycles stay");
    assert!(matches!(
        run_script(&mut it, "map", "k.m.get(1)===k"),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_borrowed_cell_survives_collection() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    let proto = it.object_proto.clone();
    let a = new_obj(Some(proto.clone()), Kind::Plain);
    let b = new_obj(Some(proto.clone()), Kind::Plain);
    crate::put(&a, "p", Value::Object(b.clone()));
    crate::put(&b, "q", Value::Object(a.clone()));
    it.collect_cycles();
    let before = live_counts();
    {
        // `b` is opaque while its kind is borrowed: it is a root, and so is `a` through it.
        let _guard = b.kind.borrow_mut();
        drop(a);
        it.collect_cycles();
        assert_eq!(live_counts(), before, "a borrowed node is not cleared");
    }
    drop(b);
    // The cycle is garbage once the borrow and the handles are gone.
    it.collect_cycles();
    assert_eq!(live_counts().0, before.0 - 2, "the released cycle is freed");
    check_invariants().unwrap();
}

/// Edges of a node that point at `t`, counted through `visit`. The node must enumerate (not be opaque).
fn count_edges_to(visit: impl FnOnce(&mut dyn FnMut(Edge)) -> bool, t: &Obj) -> usize {
    let mut n = 0;
    let ok = visit(&mut |e| {
        if let Edge::Obj(o) = e {
            if Rc::ptr_eq(o, t) {
                n += 1;
            }
        }
    });
    assert!(ok, "node is opaque");
    n
}

#[test]
fn gc_edges_match_rc_clones() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    let t = new_obj(None, Kind::Plain);
    let base = Rc::strong_count(&t);
    // Object: proto (1), two array items, two data props, one accessor's getter and setter: 7 edges to `t`.
    let holder = new_obj(
        Some(t.clone()),
        Kind::Array(vec![
            Value::Object(t.clone()),
            Value::Object(t.clone()),
            Value::Number(1.0),
        ]),
    );
    crate::put(&holder, "a", Value::Object(t.clone()));
    crate::put(&holder, "b", Value::Object(t.clone()));
    holder.raw_props.borrow_mut().push(Prop {
        key: "g".into(),
        slot: Slot::Accessor {
            get: Some(Value::Object(t.clone())),
            set: Some(Value::Object(t.clone())),
        },
        enumerable: false,
        writable: false,
        configurable: true,
    });
    let edges = count_edges_to(|f| obj_edges(&holder, f), &t);
    assert_eq!(edges, 7);
    assert_eq!(Rc::strong_count(&t) - base, edges, "object edges");

    // Scope: two bindings and `this` (3 edges). A child scope's parent is one more scope edge.
    let before = Rc::strong_count(&t);
    let s = Scope::new(None);
    s.declare("x", Value::Object(t.clone()), true);
    s.declare("y", Value::Object(t.clone()), true);
    *s.this_val.borrow_mut() = Some(Value::Object(t.clone()));
    assert_eq!(Rc::strong_count(&t) - before, 3, "scope handles");
    let mut scope_obj_edges = 0;
    let ok = scope_edges(&s, &mut |e| {
        if let Edge::Obj(o) = e {
            if Rc::ptr_eq(o, &t) {
                scope_obj_edges += 1;
            }
        }
    });
    assert!(ok);
    assert_eq!(scope_obj_edges, 3, "scope edges to the object");
    let child = Scope::new(Some(s.clone()));
    let mut parent_edges = 0;
    assert!(scope_edges(&child, &mut |e| {
        if let Edge::Scope(p) = e {
            if Rc::ptr_eq(p, &s) {
                parent_edges += 1;
            }
        }
    }));
    assert_eq!(parent_edges, 1, "one parent edge");

    // A function's lazily made `prototype` is a `Weak` owner: no self edge.
    let f = run_script(&mut it, "lazy", "(function gfn(){})");
    let Value::Object(fo) = &f else {
        panic!("not a function")
    };
    assert_eq!(
        count_edges_to(|cb| obj_edges(fo, cb), fo),
        0,
        "no self edge"
    );

    // The registry snapshot holds every live node, once.
    let (objs, scopes) = snapshot();
    let (n_objs, n_scopes) = counts();
    assert_eq!(objs.iter().flatten().count(), n_objs);
    assert_eq!(scopes.iter().flatten().count(), n_scopes);
}

/// A mixed heap: objects, arrays, classes, closures, prototype links, half of them kept alive. The per-node count
/// must stay non-negative (a negative one is an over-counted edge, which `collect_nodes` asserts against).
const MIXED_JS: &str = "globalThis.keep = []; \
    class K { constructor(i){ this.i=i; this.self=this; } m(){ return this.i } } \
    function mk(i){ var o={i:i}; var a=[o,o]; \
      switch(i%5){ case 0: o.self=o; break; case 1: o.a=a; a.push(o); break; \
        case 2: o.f=()=>o.i; break; case 3: o.k=new K(i); o.k.o=o; break; \
        default: o.p=Object.create(o); } return o; } \
    for (let i=0;i<3000;i++){ var r = mk(i); if (i%2===0) keep.push(r); }";

#[test]
fn gc_mixed_heap_counts_stay_nonnegative() {
    crate::gc::force_enabled();
    crate::gc::no_auto(); // the explicit collections below must find the garbage themselves
    let mut it = Interp::new();
    run_script(&mut it, "mixed", MIXED_JS);
    let first = it.collect_cycles();
    assert!(first >= 1000, "collected {first} from a mixed heap");
    assert_eq!(it.collect_cycles(), 0, "a second collection finds nothing");
    check_invariants().unwrap();
}

const STRESS_JS: &str = "globalThis.keep = []; globalThis.vals = []; var seed = 7; \
    function rnd(n){ seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; } \
    function mkobj(v){ var o={v:v}; o.arr=[o,o]; if (v%2) o.self=o; return o; } \
    function mkfn(v){ var o={v:v}; o.self=o; o.f=()=>o.v; return o.f; } \
    function checksum(){ var s=0, t=0; for (var k=0;k<keep.length;k++){ s+=keep[k](); t+=vals[k]; } return s===t; } \
    async function ping(x){ await null; return x } \
    function* gen(n){ var o={n:n}; o.self=o; yield o.n; yield o.self.n; } \
    function round(r){ \
      for (var i=0;i<200;i++){ mkobj(r*200+i); mkfn(r*200+i); } \
      for (var j=0;j<20;j++){ var v=r*1000+rnd(1000); keep.push(mkfn(v)); vals.push(v); } \
      var g=[...gen(r)]; \
      return checksum(); } \
    function round_async(r){ var ok=round(r); ping(r).then(function(x){ globalThis.pinged=x }); return ok; } \
    function drop_half(){ keep.splice(0, keep.length >> 1); vals.splice(0, vals.length >> 1); }";

/// Interleaves allocation, collection, calls into kept closures, generator work, and drops. Every round checks the
/// kept closures, and the end state returns to the base. `async_rounds` adds an async call per round.
fn stress(async_rounds: bool) {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(&mut it, "stress", STRESS_JS);
    it.collect_cycles();
    let base = live_counts();
    let round_fn = if async_rounds { "round_async" } else { "round" };
    for r in 0..50 {
        let good = run_script(&mut it, "stress", &format!("{round_fn}({r})"));
        assert!(
            matches!(good, Value::Bool(true)),
            "round {r}: checksum mismatch"
        );
        it.collect_cycles();
        ok(it.run_event_loop(), "run_event_loop");
        if r % 2 == 1 {
            run_script(&mut it, "stress", "drop_half()");
            let good = run_script(&mut it, "stress", "checksum()");
            assert!(
                matches!(good, Value::Bool(true)),
                "round {r}: checksum after drop"
            );
        }
    }
    run_script(&mut it, "stress", "keep.length=0; vals.length=0");
    it.collect_cycles();
    assert_eq!(live_counts(), base, "live counts after the stress run");
    check_invariants().unwrap();
}

#[test]
fn gc_stress_interleaved() {
    stress(false);
}

#[test]
#[ignore = "Promise is an opaque kind (not enumerated in PR-1): after 50 async rounds 98 Promise objects remain, and \
            they remain with no collection run at all (only event-loop turns), so that part is not a collector \
            regression; the .then closures and their round scopes (49 each) stay with their promises"]
fn gc_stress_async_rounds() {
    stress(true);
}
