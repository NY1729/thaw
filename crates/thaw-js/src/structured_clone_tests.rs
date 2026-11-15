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
