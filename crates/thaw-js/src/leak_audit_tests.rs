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
