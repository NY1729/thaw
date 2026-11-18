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
