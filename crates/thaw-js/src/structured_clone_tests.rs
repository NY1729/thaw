//! structuredClone semantics (the D-1 gaps): boxed primitives keep their kind, `options.transfer`
//! detaches the source, and WeakMap/WeakSet/WeakRef/Promise/Proxy throw DataCloneError. Expectations
//! recorded from Node 22 (`node -e`), console lines joined with `|`.

use super::*;

/// Run `src` and return the console.log lines joined with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn structured_clone_keeps_boxed_primitive_kind() {
    assert_eq!(
        one(
            r##"const a=structuredClone(Object(1n)), b=structuredClone(Object("s")), c=structuredClone(Object(true)), d=structuredClone(Object(3)); console.log([a.constructor.name,typeof a,String(a.valueOf()),b.constructor.name,b.valueOf(),b.length,c.constructor.name,c.valueOf(),d.constructor.name,d.valueOf()].join(" "));"##
        ),
        r##"BigInt object 1 String s 1 Boolean true Number 3"##
    );
}

#[test]
fn structured_clone_transfer_detaches_the_source() {
    assert_eq!(
        one(
            r##"const x=new ArrayBuffer(4); new Uint8Array(x)[0]=7; const y=structuredClone(x,{transfer:[x]}); console.log([x.byteLength,x.detached,y.byteLength,new Uint8Array(y)[0]].join(" "));"##
        ),
        r##"0 true 4 7"##
    );
}

#[test]
fn structured_clone_transfer_rejects_bad_lists_without_detaching() {
    assert_eq!(
        one(
            r##"for (const tl of [[{}],[new SharedArrayBuffer(1)]]) { try { structuredClone(new ArrayBuffer(1),{transfer:tl}); console.log("ok"); } catch (e) { console.log(e.name, e.message); } }
const b2=new ArrayBuffer(4); try { structuredClone(b2,{transfer:[b2,b2]}); } catch (e) { console.log(e.name, e.message); }
const b3=new ArrayBuffer(4); b3.transfer(); try { structuredClone(b3,{transfer:[b3]}); } catch (e) { console.log(e.name, e.message); }
const b4=new ArrayBuffer(4); try { structuredClone({f(){},b4},{transfer:[b4]}); } catch (e) {} console.log(b4.byteLength);"##
        ),
        r##"DataCloneError Found invalid value in transferList.|DataCloneError Found invalid value in transferList.|DataCloneError Transfer list contains duplicate ArrayBuffer|DataCloneError Cannot transfer object of unsupported type.|4"##
    );
}

#[test]
fn structured_clone_throws_for_weak_promise_and_proxy_values() {
    assert_eq!(
        one(
            r##"for (const f of [()=>new WeakMap(),()=>new WeakSet(),()=>new WeakRef({}),()=>Promise.resolve(),()=>new Proxy({},{}),()=>new Proxy([],{}),()=>({n:new WeakMap()})]) { try { structuredClone(f()); console.log("no throw"); } catch (e) { console.log(e.name, e.message); } }"##
        ),
        r##"DataCloneError #<WeakMap> could not be cloned.|DataCloneError #<WeakSet> could not be cloned.|DataCloneError #<WeakRef> could not be cloned.|DataCloneError #<Promise> could not be cloned.|DataCloneError #<Object> could not be cloned.|DataCloneError [object Array] could not be cloned.|DataCloneError #<WeakMap> could not be cloned."##
    );
}

// node:worker_threads marks (markAsUntransferable / markAsUncloneable). Expectations recorded from Node 22.23.1.

#[test]
fn worker_threads_marks_match_node_22_exports_and_returns() {
    assert_eq!(
        one(
            r##"const wt=__thaw_worker_threads_exports; const ab=new ArrayBuffer(8);
console.log(typeof wt.markAsUntransferable, wt.markAsUntransferable.length, wt.isMarkedAsUntransferable.length, wt.markAsUncloneable.length);
console.log(wt.isMarkedAsUntransferable(ab), wt.markAsUntransferable(ab), wt.isMarkedAsUntransferable(ab), wt.isMarkedAsUntransferable({}), wt.isMarkedAsUntransferable(1));
const o={a:1}; wt.markAsUntransferable(o); wt.markAsUntransferable(1); wt.markAsUntransferable(); wt.markAsUncloneable(null); wt.markAsUncloneable('s');
console.log(wt.isMarkedAsUntransferable(o), wt.isMarkedAsUntransferable(), wt.markAsUncloneable(o));"##
        ),
        r##"function 1 1 1|false undefined true false false|true false undefined"##
    );
}

#[test]
fn marked_arraybuffer_refuses_transfer_but_still_clones() {
    assert_eq!(
        one(
            r##"const wt=__thaw_worker_threads_exports; const ab=new ArrayBuffer(8); wt.markAsUntransferable(ab);
const {port1}=new wt.MessageChannel();
try { port1.postMessage(ab,[ab]); console.log('sent'); } catch (e) { console.log(e.name, e.code, e.message); }
console.log(ab.byteLength, ab.detached);
try { structuredClone(ab,{transfer:[ab]}); console.log('cloned'); } catch (e) { console.log(e.name, e.code, e.message); }
port1.postMessage(ab);
console.log(ab.byteLength, structuredClone(ab).byteLength, ab.detached);
port1.close();"##
        ),
        r##"DataCloneError 25 Cannot transfer object of unsupported type.|8 false|DataCloneError 25 Cannot transfer object of unsupported type.|8 8 false"##
    );
}

#[test]
fn marked_transfer_entries_are_refused_before_type_checks() {
    assert_eq!(
        one(
            r##"const wt=__thaw_worker_threads_exports; const ab=new ArrayBuffer(2); wt.markAsUntransferable(ab); const x=new ArrayBuffer(2);
for (const tl of [[{}],[ab],[x,ab],[ab,ab],[new Uint8Array(4)]]) { try { structuredClone(1,{transfer:tl}); console.log('ok'); } catch (e) { console.log(e.name, e.code, e.message); } }
console.log(x.byteLength, x.detached);"##
        ),
        r##"DataCloneError 25 Found invalid value in transferList.|DataCloneError 25 Cannot transfer object of unsupported type.|DataCloneError 25 Cannot transfer object of unsupported type.|DataCloneError 25 Cannot transfer object of unsupported type.|DataCloneError 25 Found invalid value in transferList.|2 false"##
    );
}

#[test]
fn mark_as_uncloneable_refuses_objects_and_errors_but_not_builtin_kinds() {
    assert_eq!(
        one(
            r##"const wt=__thaw_worker_threads_exports; const o={a:1}; wt.markAsUncloneable(o);
try { structuredClone(o); console.log('cloned'); } catch (e) { console.log(e.name, e.code, e.message); }
try { structuredClone({n:[o]}); console.log('cloned'); } catch (e) { console.log(e.name, e.code, e.message); }
const {port1}=new wt.MessageChannel();
try { port1.postMessage(o); console.log('sent'); } catch (e) { console.log(e.name, e.code, e.message); }
port1.close();
class A {} const ai=new A(); wt.markAsUncloneable(ai);
try { structuredClone(ai); console.log('cloned'); } catch (e) { console.log(e.name, e.code, e.message); }
const err=new Error('m'); wt.markAsUncloneable(err);
try { structuredClone(err); console.log('cloned'); } catch (e) { console.log(e.name, e.code, e.message); }
const d=new Date(0); const ab=new ArrayBuffer(3); const arr=[1,2]; wt.markAsUncloneable(d); wt.markAsUncloneable(ab); wt.markAsUncloneable(arr);
console.log(structuredClone(d).getTime(), structuredClone(ab).byteLength, structuredClone(arr).length);"##
        ),
        r##"DataCloneError 25 Cannot clone object of unsupported type.|DataCloneError 25 Cannot clone object of unsupported type.|DataCloneError 25 Cannot clone object of unsupported type.|DataCloneError 25 Cannot clone object of unsupported type.|DataCloneError 25 Cannot clone object of unsupported type.|0 3 2"##
    );
}

// node:worker_threads moveMessagePortToContext (Node 22.23.1 expectations, thaw-tmp/we1-scratch/m1-m3.js).
// Runs the event loop after the source, so a started port's delivery is seen. vm_context.js (the real createContext)
// is loaded first, as the vm realm tests do.
const VM_PRELUDE: &str =
    include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js");

fn one_with_jobs(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(&format!("{VM_PRELUDE}\n{src}")) {
        Ok(_) => {
            it.run_event_loop().unwrap();
            it.out.join("|")
        }
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn move_message_port_to_context_argument_errors_match_node() {
    assert_eq!(
        one_with_jobs(
            r##"const wt=__thaw_worker_threads_exports; const {port1,port2}=new MessageChannel(); const sb=createContext({});
const e=f=>{try{f();return 'ok'}catch(x){return x.name+' '+x.code+' '+x.message}};
console.log([e(()=>wt.moveMessagePortToContext()), e(()=>wt.moveMessagePortToContext({},sb)), e(()=>wt.moveMessagePortToContext(port1)), e(()=>wt.moveMessagePortToContext(port1,{})), e(()=>wt.moveMessagePortToContext(port1,null)), e(()=>wt.moveMessagePortToContext(port1,sb))].join(' ; '));
port1.close(); console.log(e(()=>wt.moveMessagePortToContext(port1,sb))); port2.close();"##
        ),
        r##"TypeError ERR_INVALID_ARG_TYPE The "port" argument must be a MessagePort instance ; TypeError ERR_INVALID_ARG_TYPE The "port" argument must be a MessagePort instance ; TypeError ERR_INVALID_ARG_TYPE Invalid context argument ; TypeError ERR_INVALID_ARG_TYPE Invalid context argument ; TypeError ERR_INVALID_ARG_TYPE Invalid context argument ; ok|Error ERR_CLOSED_MESSAGE_PORT Cannot send data on closed MessagePort"##
    );
}

#[test]
fn move_message_port_to_context_delivers_only_after_start_into_the_context_realm() {
    assert_eq!(
        one_with_jobs(
            r##"const wt=__thaw_worker_threads_exports; const sb=createContext({});
const ctxObject=runInContext('Object', sb);
const ctxArray=runInContext('Array', sb);
const c=new MessageChannel(); const m=wt.moveMessagePortToContext(c.port1, sb);
console.log(typeof m, m instanceof MessagePort, 'onmessage' in m, typeof m.start, typeof m.postMessage, typeof m.close, typeof m.hasRef);
m.onmessage=function(ev){ console.log('got', this===m, ev.target===m, ev.type, ev.ports.length, ev.data.constructor===ctxObject, ev.data.arr instanceof ctxArray, Object.getPrototypeOf(ev.data)===ctxObject.prototype, ev.data.arr.constructor===ctxArray); };
c.port2.postMessage({arr:[1,2]});
const quiet=new MessageChannel(); const n=wt.moveMessagePortToContext(quiet.port1, sb);
n.onmessage=()=>console.log('unstarted delivered');
quiet.port2.postMessage(1); console.log('before start');
m.start(); m.start(); wt.moveMessagePortToContext(quiet.port1, sb);
c.port2.postMessage({arr:[3]});
setTimeout(()=>{c.port1.close(); c.port2.close(); quiet.port1.close(); quiet.port2.close();}, 0);"##
        ),
        r##"object false false function function function function|before start|got true true message 0 true true true true|got true true message 0 true true true true"##
    );
}

#[test]
fn structured_clone_views_share_one_buffer_copy_and_brands_ignore_prototypes() {
    assert_eq!(
        one(
            r##"const input = new ArrayBuffer(8); const src = new Uint8Array(input); src.set([10,11,12,13,14,15,16,17]);
const u8 = new Uint8Array(input, 2, 4), u16 = new Uint16Array(input, 2, 2), dv = new DataView(input, 3, 2);
const c = structuredClone({ u8, u16, dv, empty: new Uint8Array(input, 8, 0), buffer: input, buf: Buffer.from('ab') });
console.log([c.u8.buffer === c.buffer, c.u16.buffer === c.buffer, c.dv.buffer === c.buffer, c.empty.buffer === c.buffer, c.buffer !== input].join());
console.log([c.u8.byteOffset, c.u8.length, c.u16.byteOffset, c.dv.byteOffset, c.dv.byteLength, c.empty.byteOffset, c.buf.constructor.name, c.buf.length].join());
c.dv.setUint8(0, 99); console.log(c.u8[1], src[3]);
const b2 = structuredClone({ u8, again: u8 }); console.log(b2.again === b2.u8);
const brands = [Promise.resolve(1), new WeakMap(), new WeakSet(), new WeakRef({})];
let reads = 0;
for (const v of brands) Object.defineProperty(v, 'constructor', { get() { reads++; return 1; } });
const protos = brands.map(v => Object.getPrototypeOf(v));
const res = [];
for (const v of brands) { for (const form of [v, { n: v }, [v]]) { try { structuredClone(form); res.push('ok'); } catch (e) { res.push(e instanceof DOMException && e.name); } } }
console.log(res.join(), reads);
for (const v of brands) Object.setPrototypeOf(v, null);
const res2 = [];
for (const v of brands) { try { structuredClone(v); res2.push('ok'); } catch (e) { res2.push(e.name + ':' + e.message); } }
console.log(res2.join('|'));
let traps = 0; const h = { get() { traps++; throw 1; }, ownKeys() { traps++; throw 1; }, getPrototypeOf() { traps++; throw 1; } };
const rv = Proxy.revocable({}, h); rv.revoke();
const res3 = [];
for (const p of [new Proxy({}, h), rv.proxy]) { try { structuredClone(p); res3.push('ok'); } catch (e) { res3.push(e.name); } }
console.log(res3.join(), traps);"##
        ),
        r##"true,true,true,true,true|2,4,2,3,2,8,Uint8Array,2|99 13|true|DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError 0|DataCloneError:[object Object] could not be cloned.|DataCloneError:[object WeakMap] could not be cloned.|DataCloneError:[object WeakSet] could not be cloned.|DataCloneError:[object Object] could not be cloned.|DataCloneError,DataCloneError 0"##
    );
}
