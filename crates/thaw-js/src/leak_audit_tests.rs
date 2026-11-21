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
    assert_flat_with(name, defs, call, run_script);
}

/// Sloppy-mode script (`with` is only legal here; `Interp::run` parses strict module text).
fn run_sloppy_script(it: &mut Interp, name: &str, src: &str) -> Value {
    match it.run_sloppy(src) {
        Ok(v) => v,
        Err(_) => panic!("{name}: sloppy script failed: {src}"),
    }
}

fn assert_flat_with(name: &str, defs: &str, call: &str, run: fn(&mut Interp, &str, &str) -> Value) {
    crate::gc::force_enabled();
    // Counts the loop's garbage before the explicit collect: the automatic trigger must not run first.
    crate::gc::no_auto();
    let mut it = Interp::new();
    run(&mut it, name, defs);
    // The realm's own start-up garbage (about 8 closures, 8 objects, 1 scope) is freed by the first collection; the
    // base is taken after it, so the check measures the loop alone.
    it.collect_cycles();
    let base = live_counts();
    run(&mut it, name, call);
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

// PR-3a data kinds: each garbage cycle must go flat after one collection (2000 iterations, back to the base).

#[test]
fn gc_flat_map_self_holder() {
    assert_flat_after_cycles(
        "map_self_holder",
        "const N=2000; function mk(){ var o={}; var m=new Map(); m.set(o,1); o.m=m; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_set_and_iterator_cycle() {
    assert_flat_after_cycles(
        "set_and_iterator",
        "const N=2000; function mk(){ var o={}; var s=new Set(); s.add(o); o.s=s; var m=new Map(); m.set(1,o); o.it=m.keys(); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_weakmap_cycle() {
    // WeakMap is JS over a plain `Map` in `__store` (extras.rs:181): its entries are strong Map edges, so this cycle
    // is collected like any Map cycle.
    assert_flat_after_cycles(
        "weakmap",
        "const N=2000; function mk(){ var o={}; var w=new WeakMap(); w.set(o,w); o.w=w; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_proxy_target_cycle() {
    assert_flat_after_cycles(
        "proxy_target",
        "const N=2000; function mk(){ var o={}; var p=new Proxy(o,{get:()=>o}); o.p=p; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_typed_view_cycle() {
    assert_flat_after_cycles(
        "typed_view",
        "const N=2000; function mk(){ var u=new Uint8Array(8); u.self=u; return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_with_scope_cycle() {
    // The arrow captures the `with` scope, so that scope is registered; o -> arrow -> with-scope -> o is the cycle.
    assert_flat_with(
        "with_scope",
        "const N=2000; function mk(){ var o={a:1}; with(o){ o.f=()=>a } return 0 }",
        PROBE_LOOP,
        run_sloppy_script,
    );
}

#[test]
fn gc_flat_class_field_arrow() {
    assert_flat_after_cycles(
        "class_field_arrow",
        "const N=2000; function mk(){ class A{ f=()=>this; static s=mk } return new A() }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_class_static_block() {
    assert_flat_after_cycles(
        "class_static_block",
        "const N=2000; function mk(){ class A{ static t=1; static { this.u=A } } return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_class_derived() {
    assert_flat_after_cycles(
        "class_derived",
        "const N=2000; function mk(){ class A{ x=()=>this } class B extends A{ y=()=>B } return new B() }",
        PROBE_LOOP,
    );
}

/// Counts the edges of `visit` that name the scope `s` (its `Rc` clones held by the visited node).
fn count_scope_edges_to(visit: impl FnOnce(&mut dyn FnMut(Edge)) -> bool, s: &Rc<Scope>) -> usize {
    let mut n = 0;
    let ok = visit(&mut |e| {
        if let Edge::Scope(x) = e {
            if Rc::ptr_eq(x, s) {
                n += 1;
            }
        }
    });
    assert!(ok, "node is opaque");
    n
}

// PR-3a negative cases: each keeps a live object usable after a collection (the automatic trigger runs too under
// THAW_JS_GC_STRESS, so these do not opt out).

#[test]
fn gc_map_held_by_global_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "map_global",
        "globalThis.m = new Map(); \
         (function(){ var o={v:9}; o.self=o; m.set('k', o) })(); \
         for(let i=0;i<300;i++){ var g={}; g.m=new Map(); g.m.set(g,g.m); }",
    );
    it.collect_cycles();
    assert!(matches!(
        run_script(
            &mut it,
            "map_global",
            "m.get('k').self===m.get('k') && m.get('k').v===9"
        ),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_map_iterator_survives_native_collect() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    let gc_fn = it.native(|it, _, _| Ok(Value::Number(it.collect_cycles() as f64)));
    it.global.declare("__gc", gc_fn, false);
    // The iterator is live (a global) and mid-walk when the native collect runs.
    let v = run_script(
        &mut it,
        "iter",
        "globalThis.m = new Map([[1,{a:1}],[2,{a:2}],[3,{a:3}]]); \
         globalThis.it = m.values(); var first = it.next().value.a; \
         function mk(){ var o={}; var mm=new Map(); mm.set(o,mm); o.mm=mm; return 0 } \
         for(let i=0;i<200;i++) mk(); \
         __gc(); \
         first*100 + it.next().value.a*10 + it.next().value.a",
    );
    assert_eq!(number(&v), 123.0);
    check_invariants().unwrap();
}

#[test]
fn gc_proxy_handler_held_by_live_object_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "proxy",
        "globalThis.keep = {}; \
         (function(){ var h={get:function(t,k){return 'h:'+k}}; keep.h=h; keep.p=new Proxy({}, h); })(); \
         for(let i=0;i<300;i++){ var q={}; var p2=new Proxy(q,{get:()=>q}); q.p=p2; }",
    );
    it.collect_cycles();
    assert!(matches!(
        run_script(&mut it, "proxy", "keep.p.zz==='h:zz'"),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_class_held_by_global_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "class_global",
        "globalThis.C = (function(){ class A { static s = () => A; x = () => this } return A })(); \
         globalThis.inst = new C(); \
         function mk(){ class B { static t=()=>B; y=()=>this } return new B() } \
         for(let i=0;i<300;i++) mk();",
    );
    it.collect_cycles();
    assert!(matches!(
        run_script(
            &mut it,
            "class_global",
            "C.s()===C && inst.x()===inst && new C().x() instanceof C"
        ),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_with_scope_closure_held_by_global_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_sloppy_script(
        &mut it,
        "with_global",
        "globalThis.g = (function(){ var o={a:5}; with(o){ return ()=>a } })(); \
         function mk(){ var o={a:1}; with(o){ o.f=()=>a } return 0 } \
         for(let i=0;i<300;i++) mk();",
    );
    it.collect_cycles();
    assert!(matches!(
        run_sloppy_script(&mut it, "with_global", "g()===5"),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
}

#[test]
fn gc_typed_view_held_by_global_survives() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "typed_global",
        "globalThis.u = new Uint8Array(4); u.self=u; u[0]=7; \
         for(let i=0;i<300;i++){ var t=new Uint8Array(2); t.t=t; }",
    );
    it.collect_cycles();
    assert!(matches!(
        run_script(
            &mut it,
            "typed_global",
            "u.self===u && u[0]===7 && u.buffer===u.buffer && u.self.buffer===u.buffer"
        ),
        Value::Bool(true)
    ));
    check_invariants().unwrap();
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
fn gc_map_cycle_with_global_holder_collects_garbage_only() {
    crate::gc::force_enabled();
    // Exact live counts below: the automatic trigger must not run first.
    crate::gc::no_auto();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "map",
        "function mk(){ var o={}; var m=new Map(); m.set(1,o); o.m=m; return o } \
         globalThis.k=mk(); for(let i=0;i<100;i++){ mk(); }",
    );
    it.collect_cycles();
    let before = live_counts();
    run_script(&mut it, "map", "for(let i=0;i<100;i++){ mk(); }");
    assert!(live_counts().0 > before.0, "the loop made garbage");
    let cleared = it.collect_cycles();
    assert!(cleared >= 200, "collect cleared {cleared} Map-cycle nodes");
    assert_eq!(
        live_counts(),
        before,
        "Map cycles are freed; the held one is not"
    );
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

/// PR-3a edge audit: each data kind reports exactly one edge per `Rc` clone it owns (strong-count delta), and the
/// shared-owner cases are opaque or uncounted, never over-counted.
#[test]
fn gc_edges_match_rc_clones_data_kinds() {
    crate::gc::force_enabled();
    crate::gc::no_auto();
    let mut it = Interp::new();
    let t = new_obj(None, Kind::Plain);
    let base = Rc::strong_count(&t);

    // Map: key and value of two entries are four clones of `t`.
    let mut md = crate::collections::MapData::default();
    for _ in 0..2 {
        md.entries
            .push((Value::Object(t.clone()), Value::Object(t.clone())));
    }
    let m = new_obj(None, Kind::Map(md));
    assert_eq!(Rc::strong_count(&t) - base, 4, "map handles");
    assert_eq!(count_edges_to(|f| obj_edges(&m, f), &t), 4, "map edges");

    // Set: two items, two clones.
    let before = Rc::strong_count(&t);
    let mut sd = crate::collections::SetData::default();
    sd.items.push(Value::Object(t.clone()));
    sd.items.push(Value::Object(t.clone()));
    let s = new_obj(None, Kind::Set(sd));
    assert_eq!(Rc::strong_count(&t) - before, 2, "set handles");
    assert_eq!(count_edges_to(|f| obj_edges(&s, f), &t), 2, "set edges");

    // Proxy: target and handler, two clones.
    let before = Rc::strong_count(&t);
    let pd = crate::proxy::ProxyData {
        target: Value::Object(t.clone()),
        handler: std::cell::RefCell::new(Value::Object(t.clone())),
        callable: false,
        env_view: false,
    };
    let p = new_obj(None, Kind::Proxy(Rc::new(pd)));
    assert_eq!(Rc::strong_count(&t) - before, 2, "proxy handles");
    assert_eq!(count_edges_to(|f| obj_edges(&p, f), &t), 2, "proxy edges");
    // A second owner of the ProxyData (a trap in flight) makes the node opaque: no edges, so `t` keeps its count.
    let shared = match &*p.kind.borrow() {
        Kind::Proxy(pd) => pd.clone(),
        _ => unreachable!(),
    };
    assert!(!obj_edges(&p, &mut |_| {}), "a shared ProxyData is opaque");
    drop(shared);

    // Typed view: one edge, to its buffer object.
    let ab = new_obj(None, Kind::ArrayBuffer(crate::typed::new_bytes(vec![0; 8])));
    let ta = crate::typed::TypedArr {
        abuf: ab.clone(),
        data: crate::typed::new_bytes(vec![0; 8]),
        ty: crate::typed::Ty::U8,
        off: 0,
        fixed_len: 8,
        track: false,
        rz: false,
    };
    let view = new_obj(None, Kind::Typed(ta));
    assert_eq!(
        count_edges_to(|f| obj_edges(&view, f), &ab),
        1,
        "typed edge"
    );

    // `with` scope: one object edge, one clone.
    let before = Rc::strong_count(&t);
    let ws = Scope::new_with(Scope::new(None), t.clone());
    assert_eq!(Rc::strong_count(&t) - before, 1, "with handle");
    let mut with_edges = 0;
    assert!(scope_edges(&ws, &mut |e| {
        if let Edge::Obj(o) = e {
            if Rc::ptr_eq(o, &t) {
                with_edges += 1;
            }
        }
    }));
    assert_eq!(with_edges, 1, "with edge");

    // Class: the constructor's scope and the class field scope are two edges to one scope; the parent is two edges
    // to `A` (the static-inheritance proto and `ClassInfo.parent`). Both need the FuncDef's sole owner.
    let g = run_script(
        &mut it,
        "cls",
        "globalThis.A = class A { x = () => this }; globalThis.B = class B extends A {}; globalThis",
    );
    let Value::Object(b) = ok(it.get(&g, "B"), "get B") else {
        panic!("B is not an object")
    };
    let Value::Object(a) = ok(it.get(&g, "A"), "get A") else {
        panic!("A is not an object")
    };
    let (def_rc, scope) = match &*b.kind.borrow() {
        Kind::Closure { def, scope } => (def.clone(), scope.clone()),
        _ => panic!("B is not a closure"),
    };
    assert_eq!(
        Rc::strong_count(&def_rc),
        2,
        "the kind and this clone share the FuncDef"
    );
    drop(def_rc);
    assert_eq!(
        count_scope_edges_to(|f| obj_edges(&b, f), &scope),
        2,
        "class: closure scope + field scope"
    );
    assert_eq!(
        count_edges_to(|f| obj_edges(&b, f), &a),
        2,
        "class: proto + parent"
    );
    // A second FuncDef owner (a call in flight) makes the class edges uncounted: the safe direction.
    let hold = match &*b.kind.borrow() {
        Kind::Closure { def, .. } => def.clone(),
        _ => unreachable!(),
    };
    assert_eq!(
        count_scope_edges_to(|f| obj_edges(&b, f), &scope),
        1,
        "shared FuncDef: field-scope edge not counted"
    );
    assert_eq!(
        count_edges_to(|f| obj_edges(&b, f), &a),
        1,
        "shared FuncDef: parent edge not counted"
    );
    drop(hold);
    check_invariants().unwrap();
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
        // A realm runs its event loop once: a later `run_event_loop` returns at once (`exit_emitted`), so the async
        // rounds drain their jobs in one call after the loop.
        if !async_rounds {
            ok(it.run_event_loop(), "run_event_loop");
        }
        if r % 2 == 1 {
            run_script(&mut it, "stress", "drop_half()");
            let good = run_script(&mut it, "stress", "checksum()");
            assert!(
                matches!(good, Value::Bool(true)),
                "round {r}: checksum after drop"
            );
        }
    }
    if async_rounds {
        ok(it.run_event_loop(), "run_event_loop");
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
fn gc_stress_async_rounds() {
    stress(true);
}

// PR-3b control kinds (Promise, Generator): garbage cycles go flat after one collection.

#[test]
fn gc_flat_promise_then_closes_over_promise() {
    assert_flat_after_cycles(
        "promise_then_closes",
        "const N=2000; function mk(){ var p=new Promise(()=>{}); p.then(function(){ p; }); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_pending_promise_chain() {
    assert_flat_after_cycles(
        "promise_chain",
        "const N=2000; function mk(){ var p1=new Promise(()=>{}); var p2=p1.then(function(){ return p2 }); var p3=p2.then(function(){ return p3 }); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_promise_all_pending_cycle() {
    assert_flat_after_cycles(
        "promise_all_cycle",
        "const N=2000; function mk(){ var a=new Promise(()=>{}); var b=new Promise(()=>{}); a.o=[a,b]; b.o=a; Promise.all([a,b]); return 0 }",
        PROBE_LOOP,
    );
}

#[test]
fn gc_flat_never_started_generator_cycle() {
    assert_flat_after_cycles(
        "gen_unstarted_cycle",
        "const N=2000; function mk(){ var it=(function*(){ yield it })(); return 0 }",
        PROBE_LOOP,
    );
}

/// Boundary, not a flat case: an async function suspended forever on a promise that closes over itself. Its frame
/// (the coroutine's Rust stack) holds strong `Rc`s to its scope, so the cycle is a root and stays until PR-4
/// (cancel-on-drop). The test pins that it survives collection and the program stays usable.
#[test]
fn gc_async_never_settles_is_kept_until_pr4() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    crate::gc::no_auto();
    run_script(
        &mut it,
        "async_kept",
        "async function af(){ var p=new Promise(()=>{}); p.self=p; await p; } globalThis.mk=function(){ af(); return 0 };",
    );
    it.collect_cycles();
    let base = live_counts();
    run_script(&mut it, "async_kept", "for(let i=0;i<200;i++){ mk(); }");
    it.collect_cycles();
    assert!(
        live_counts().0 > base.0,
        "a suspended async frame's cycle is a root (PR-4 frees it)"
    );
    ok(it.run_event_loop(), "run_event_loop");
    check_invariants().unwrap();
}

/// A started, unfinished generator is opaque: its frame holds the scope, so the collector never clears it. Dropping
/// the garbage generator leaks its frame exactly as before; the test only pins that collection keeps it usable.
#[test]
fn gc_started_generator_garbage_is_a_root() {
    crate::gc::force_enabled();
    crate::gc::no_auto();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "started_gen",
        "globalThis.keepit=null; globalThis.mk=function(){ var it=(function*(){ var o={}; o.f=()=>o; yield 1; yield o.f().self }).call(null); it.next(); it.self=it; return 0 };",
    );
    for _ in 0..50 {
        run_script(&mut it, "started_gen", "mk()");
        it.collect_cycles();
    }
    check_invariants().unwrap();
}

#[test]
fn gc_flat_finished_generator_cycle() {
    // The collector is off with the thread backend: a finished coroutine thread drops its frames only when it
    // exits, so the live counts are not flat there by design. The stack backend is the supported one.
    if crate::co::backend() == crate::co::Backend::Thread {
        return;
    }
    assert_flat_after_cycles(
        "gen_finished_cycle",
        "const N=2000; function mk(){ var it=(function*(){ yield 1 })(); it.self=it; for(var x of it){} return 0 }",
        PROBE_LOOP,
    );
}

// Negative: each must survive collection and stay usable.

#[test]
fn gc_timer_resolves_promise_held_by_queue() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "timer_promise",
        "function mk(){ var o={a:3}; o.f=()=>o; var p=new Promise(function(r){ o.r=r }); \
         setTimeout(function(){ o.r(o.a+1) },0); return p.then(function(x){ globalThis.out=x }) } mk();",
    );
    it.collect_cycles();
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(number(&run_script(&mut it, "timer_promise", "out")), 4.0);
    check_invariants().unwrap();
}

#[test]
fn gc_async_suspended_on_timer_resumes() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "async_timer",
        "async function af(){ var o={a:5}; o.f=()=>o; await new Promise(function(r){ setTimeout(r,0) }); globalThis.out=o.f().a; } af();",
    );
    it.collect_cycles();
    it.collect_cycles();
    it.collect_cycles();
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(number(&run_script(&mut it, "async_timer", "out")), 5.0);
    check_invariants().unwrap();
}

#[test]
fn gc_suspended_generator_global_keeps_yielding() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "gen_global",
        "function* g(){ var o={n:1}; o.f=()=>o; yield 1; yield o.f().n+1; yield o.f().n+2 } globalThis.gi=g(); gi.next();",
    );
    it.collect_cycles();
    it.collect_cycles();
    assert_eq!(
        number(&run_script(&mut it, "gen_global", "gi.next().value")),
        2.0
    );
    it.collect_cycles();
    assert_eq!(
        number(&run_script(&mut it, "gen_global", "gi.next().value")),
        3.0
    );
    check_invariants().unwrap();
}

#[test]
fn gc_global_promise_pending_reaction_runs() {
    crate::gc::force_enabled();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "global_promise",
        "globalThis.P=new Promise(function(r){ globalThis.R=r }); \
         (function(){ var o={a:6}; o.f=()=>o; P.then(function(){ globalThis.out=o.f().a }) })();",
    );
    it.collect_cycles();
    run_script(&mut it, "global_promise", "R(1)");
    it.collect_cycles();
    ok(it.run_event_loop(), "run_event_loop");
    assert_eq!(number(&run_script(&mut it, "global_promise", "out")), 6.0);
    check_invariants().unwrap();
}

/// The unhandled-rejection report of a run with a collection in front of it must match the run without one.
#[test]
fn gc_unhandled_rejection_unchanged() {
    crate::gc::force_enabled();
    let src = "function mk(){ var o={}; o.f=()=>o; Promise.reject(o); o.self=o; } mk();";
    let mut plain = Interp::new();
    run_script(&mut plain, "unhandled", src);
    let plain_failed = plain.run_event_loop().is_err();
    let mut collected = Interp::new();
    run_script(&mut collected, "unhandled", src);
    collected.collect_cycles();
    let collected_failed = collected.run_event_loop().is_err();
    assert!(
        plain_failed,
        "the plain run reports the unhandled rejection"
    );
    assert_eq!(
        collected_failed, plain_failed,
        "collection changed the report"
    );
    check_invariants().unwrap();
}

/// Promise and generator edges against `Rc` clones: each reaction callback, derived promise, unstarted argument and
/// queued request is one clone the node owns. A started generator is opaque (its coroutine holds clones).
#[test]
fn gc_edges_match_rc_clones_control_kinds() {
    crate::gc::force_enabled();
    crate::gc::no_auto();
    let mut it = Interp::new();
    run_script(
        &mut it,
        "ctl",
        "globalThis.t=function(){}; globalThis.P=new Promise(function(){}); globalThis.Q=P.then(t,t); \
         globalThis.G=(function*(a,b){ void this; }).call(t,t,t);",
    );
    let g = it.global_object.clone();
    let get = |it: &mut Interp, k: &str| match ok(it.get(&Value::Object(g.clone()), k), k) {
        Value::Object(o) => o,
        _ => panic!("{k} is not an object"),
    };
    let t = get(&mut it, "t");
    let p = get(&mut it, "P");
    let q = get(&mut it, "Q");
    let gen = get(&mut it, "G");

    // Holders of `t`: the global property, this test's handle, two reactions, and the generator's frame, which
    // binds `this` and the two parameters `a`, `b`. The generator has no direct edge to `t`: its bindings are
    // reached through its bound frame (`GenStart::frame`), a registered scope.
    let p_edges = count_edges_to(|f| obj_edges(&p, f), &t);
    assert_eq!(p_edges, 2, "promise: on_ful and on_rej");
    let g_edges = count_edges_to(|f| obj_edges(&gen, f), &t);
    assert_eq!(
        g_edges, 0,
        "unstarted generator: no direct edge (was 3: this and two args)"
    );
    let frame = match &*gen.kind.borrow() {
        Kind::Generator(gd) => gd.start.borrow().as_ref().expect("unstarted").frame.clone(),
        _ => panic!("G is not a generator"),
    };
    let frame_edges = count_edges_to(|f| scope_edges(&frame, f), &t);
    assert_eq!(frame_edges, 3, "bound frame: this, a, b");
    assert_eq!(
        Rc::strong_count(&t),
        2 + p_edges + frame_edges,
        "t: global + test handle + every edge"
    );

    // The derived promise is the reaction's `result`: one clone, held by `P`.
    assert_eq!(count_edges_to(|f| obj_edges(&p, f), &q), 1, "result edge");
    assert_eq!(
        Rc::strong_count(&q),
        3,
        "Q: global + handle + reaction result"
    );

    // A queued request (value and promise) of a generator with no coroutine: the queue arm.
    let base = Rc::strong_count(&t);
    let pending = it.new_promise();
    let qd = crate::generators::GenData {
        status: std::cell::Cell::new(crate::generators::GStatus::Done),
        start: std::cell::RefCell::new(None),
        co: std::cell::RefCell::new(None),
        queue: std::cell::RefCell::new(std::collections::VecDeque::from([(
            crate::generators::Req::Next,
            Value::Object(t.clone()),
            pending.clone(),
        )])),
    };
    let qobj = new_obj(None, Kind::Generator(Rc::new(qd)));
    assert_eq!(Rc::strong_count(&t) - base, 1, "queue value handle");
    assert_eq!(
        count_edges_to(|f| obj_edges(&qobj, f), &t),
        1,
        "queue value edge"
    );
    assert_eq!(
        count_edges_to(|f| obj_edges(&qobj, f), &pending),
        1,
        "queue promise edge"
    );
    assert_eq!(
        Rc::strong_count(&pending),
        2,
        "pending: this handle and the queue's clone"
    );

    // A started, unfinished generator holds a coroutine: opaque.
    run_script(
        &mut it,
        "ctl",
        "globalThis.S=(function*(){ yield 1 })(); S.next();",
    );
    let s = get(&mut it, "S");
    assert!(!obj_edges(&s, &mut |_| {}), "a started generator is opaque");
    check_invariants().unwrap();
}

// ---- arena reclamation at event-loop turns: the thaw-js counterpart of the QuickJS turn reset ----

thread_local! {
    static TURN_PTR: Cell<usize> = const { Cell::new(0) };
    static TURN_SEEN: Cell<bool> = const { Cell::new(false) };
    static NESTED_PTR: Cell<usize> = const { Cell::new(0) };
    static NESTED_SEEN: Cell<bool> = const { Cell::new(false) };
    static OUTER_SEEN: Cell<bool> = const { Cell::new(false) };
}

/// Native that makes one unrooted arena block per call and remembers the first. `was_reclaimed` only reports the
/// most recent reset, and a turn's reset runs before that turn's timers, so the latch catches the reclaim in time.
fn turn_alloc_native(it: &Interp) -> Value {
    it.native(|_, _, _| {
        let p = thaw_arena::thaw_arena_alloc(8192, 8) as usize;
        TURN_PTR.with(|first| {
            if first.get() == 0 {
                first.set(p);
            }
        });
        if thaw_arena::was_reclaimed(TURN_PTR.with(Cell::get)) {
            TURN_SEEN.with(|seen| seen.set(true));
        }
        Ok(Value::Undefined)
    })
}

#[test]
fn loop_turns_reclaim_unrooted_arena_allocations() {
    std::thread::spawn(|| {
        thaw_arena::thaw_arena_enable_tracing();
        let mut it = Interp::new();
        let alloc = turn_alloc_native(&it);
        it.global.declare("__alloc", alloc, false);
        ok(
            it.run("var n = 0; function step() { __alloc(); if (++n < 300) setTimeout(step, 0); } setTimeout(step, 0);"),
            "chain",
        );
        ok(it.run_event_loop(), "run_event_loop");
        assert!(
            TURN_SEEN.with(Cell::get),
            "a loop turn reclaimed the unrooted block a previous turn made"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn loop_turns_without_tracing_reclaim_nothing() {
    std::thread::spawn(|| {
        assert!(!thaw_arena::is_tracing());
        let mut it = Interp::new();
        let alloc = turn_alloc_native(&it);
        it.global.declare("__alloc", alloc, false);
        ok(
            it.run("var n = 0; function step() { __alloc(); if (++n < 300) setTimeout(step, 0); } setTimeout(step, 0);"),
            "chain",
        );
        ok(it.run_event_loop(), "run_event_loop");
        assert!(!TURN_SEEN.with(Cell::get), "untraced arena reclaims nothing");
        assert_eq!(number(&run_script(&mut it, "n", "n")), 300.0);
    })
    .join()
    .unwrap();
}

#[test]
fn nested_loop_does_not_reclaim_an_outer_callback_allocation() {
    std::thread::spawn(|| {
        thaw_arena::thaw_arena_enable_tracing();
        let mut it = Interp::new();
        // An outer timer callback makes a block, then runs a nested loop (as a native pump would) that makes
        // enough garbage to cross the auto-reset threshold. The block must survive the nested turns.
        let outer_alloc = it.native(|_, _, _| {
            let p = thaw_arena::thaw_arena_alloc(8192, 8) as usize;
            NESTED_PTR.with(|first| {
                if first.get() == 0 {
                    first.set(p);
                }
            });
            if thaw_arena::was_reclaimed(NESTED_PTR.with(Cell::get)) {
                OUTER_SEEN.with(|seen| seen.set(true));
            }
            Ok(Value::Undefined)
        });
        let inner_alloc = it.native(|_, _, _| {
            thaw_arena::thaw_arena_alloc(8192, 8);
            if thaw_arena::was_reclaimed(NESTED_PTR.with(Cell::get)) {
                NESTED_SEEN.with(|seen| seen.set(true));
            }
            Ok(Value::Undefined)
        });
        let nested = it.native(|it, _, _| {
            it.run_loop_only()?;
            Ok(Value::Undefined)
        });
        it.global.declare("__outer_alloc", outer_alloc, false);
        it.global.declare("__inner_alloc", inner_alloc, false);
        it.global.declare("__nested", nested, false);
        ok(
            it.run(
                "var on = 0, m = 0; \
                 function inner() { __inner_alloc(); if (++m < 300) setTimeout(inner, 0); } \
                 function outer() { __outer_alloc(); on++; \
                   if (on === 1) { setTimeout(inner, 0); __nested(); } \
                   if (on < 4) setTimeout(outer, 0); } \
                 setTimeout(outer, 0);",
            ),
            "chain",
        );
        ok(it.run_event_loop(), "run_event_loop");
        assert!(
            !NESTED_SEEN.with(Cell::get),
            "the nested loop reclaimed an allocation its outer callback still holds"
        );
        assert!(
            OUTER_SEEN.with(Cell::get),
            "the outermost loop reclaims it once the nested loop has returned"
        );
    })
    .join()
    .unwrap();
}
