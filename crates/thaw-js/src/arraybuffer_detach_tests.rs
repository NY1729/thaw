//! ArrayBuffer detach: every view access site clamps to the (now empty) storage instead of panicking,
//! and the JS-visible results match Node 22 (expectations recorded from `node -e`, joined with `|`).

use super::*;
use crate::typed::{detach_bytes, is_detached, new_bytes, Ty};

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
fn detach_clamps_a_u8_view_on_every_rust_read_and_write_path() {
    let it = Interp::new();
    let v = it.typed_with_len(Ty::U8, 4);
    let t = Interp::typed_of(&v).unwrap();
    t.set(0, 9.0);
    assert!(!is_detached(&t.data));
    detach_bytes(&t.data);
    assert!(is_detached(&t.data));
    assert_eq!(t.cur_len(), 0);
    assert_eq!(t.cur_off(), 0);
    assert_eq!(t.window(), Vec::<u8>::new());
    assert_eq!(t.get(0), None);
    assert_eq!(t.to_vec(), Vec::<f64>::new());
    t.set(0, 1.0); // no panic: a write into a detached view is dropped
    assert_eq!(Interp::view_bytes(&v), Some(Vec::new()));
}

#[test]
fn detach_clamps_a_view_whose_storage_shrank_without_detaching() {
    // a view that outlives a shrink of its storage (not a detach) must clamp too, never index past the end
    let it = Interp::new();
    let v = it.typed_with_len(Ty::I32, 2);
    let t = Interp::typed_of(&v).unwrap();
    t.data.borrow_mut().truncate(3);
    assert!(!t.live());
    assert_eq!(t.cur_len(), 0);
    assert_eq!(t.window(), Vec::<u8>::new());
    t.set(1, 5.0);
    assert!(t.get_value(0).is_none());
}

#[test]
fn detach_clamps_bigint_element_writes_and_reads() {
    let mut it = Interp::new();
    let v = it.typed_with_len(Ty::I64, 2);
    let t = Interp::typed_of(&v).unwrap();
    detach_bytes(&t.data);
    t.set_value(&mut it, 1, &crate::bigint::big(num_bigint::BigInt::from(5)))
        .unwrap();
    assert!(t.get_value(1).is_none());
}

#[test]
fn arraybuffer_over_existing_storage_shares_bytes_and_detaches_together() {
    let it = Interp::new();
    let store = new_bytes(vec![1, 2, 3]);
    let ab = it.new_array_buffer(store.clone());
    let v = Value::Object(ab);
    assert!(Interp::bytes_of_buffer(&v).is_some_and(|b| std::rc::Rc::ptr_eq(&b, &store)));
    assert!(!is_detached(&store));
    detach_bytes(&store);
    detach_bytes(&store); // idempotent
    assert!(is_detached(&store));
    assert_eq!(store.borrow().len(), 0);
}

#[test]
fn detach_fast_path_length_offset() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint16Array(ab,0,2); ab.transfer(); console.log(u.length, u.byteLength, u.byteOffset, u[0], u[1]);"##
        ),
        r##"0 0 0 undefined undefined"##
    );
}

#[test]
fn detach_element_write_ignored() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab,1,2); ab.transfer(); u[0]=5; u[1]=6; console.log(u[0], u[1], u.length);"##
        ),
        r##"undefined undefined 0"##
    );
}

#[test]
fn detach_bigint_element_write_ignored() {
    assert_eq!(
        one(
            r##"const ab=new BigInt64Array(2).buffer; const b=new BigInt64Array(ab); ab.transfer(); b[0]=5n; console.log(b.length, b[0]);"##
        ),
        r##"0 undefined"##
    );
}

#[test]
fn detach_keys_of_detached_view() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab); ab.transfer(); console.log(Object.keys(u).length, [...Object.keys(u)].join());"##
        ),
        r##"0 "##
    );
}

#[test]
fn detach_set_on_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab); ab.transfer(); try { u.set([1]); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_subarray_on_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab,2); ab.transfer(); try { u.subarray(0); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform Construct on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_slice_on_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab,2); ab.transfer(); try { u.slice(0); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform %TypedArray%.prototype.slice on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_construct_from_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); ab.transfer(); try { new Uint8Array(ab); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform Construct on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_spread_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab); ab.transfer(); try { console.log([...u].length); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform %TypedArray%.prototype.values on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_array_from_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab); ab.transfer(); try { Array.from(u); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform %TypedArray%.prototype.values on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_utf8_decode_on_detached() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const u=new Uint8Array(ab); ab.transfer(); console.log(JSON.stringify(new TextDecoder().decode(u)));"##
        ),
        r##""""##
    );
}

#[test]
fn detach_buffer_from_detached_throws() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); ab.transfer(); try { Buffer.from(ab); } catch (e) { console.log(e.name, e.message); }"##
        ),
        r##"TypeError Cannot perform Construct on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_buffer_view_detached_length() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const b=Buffer.from(ab); const c=Buffer.alloc(3); ab.transfer(); console.log(b.length, c.length, c.toString('hex').length);"##
        ),
        r##"0 3 6"##
    );
}

#[test]
fn detach_dataview_accessors_throw() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const d=new DataView(ab,1,2); ab.transfer(); for (const f of [()=>d.byteLength, ()=>d.byteOffset, ()=>d.getUint8(0), ()=>d.setInt16(0,1,true)]) { try { console.log(f()); } catch (e) { console.log(e.name, e.message); } }"##
        ),
        r##"TypeError Cannot perform get DataView.prototype.byteLength on a detached ArrayBuffer|TypeError Cannot perform get DataView.prototype.byteOffset on a detached ArrayBuffer|TypeError Cannot perform DataView.prototype.getUint8 on a detached ArrayBuffer|TypeError Cannot perform DataView.prototype.setInt16 on a detached ArrayBuffer"##
    );
}

#[test]
fn detach_arraybuffer_structured_clone() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); const d=new DataView(ab); ab.transfer(); for (const v of [ab, new Uint8Array(4), d]) { try { structuredClone(v); console.log('ok'); } catch (e) { console.log(e.name, e.code, e.message); } }"##
        ),
        r##"DataCloneError 25 An ArrayBuffer is detached and could not be cloned.|ok|DataCloneError 25 An ArrayBuffer is detached and could not be cloned."##
    );
}

#[test]
fn detach_inspect_detached() {
    assert_eq!(
        one(
            r##"const ab=new ArrayBuffer(4); ab.transfer(); console.log(ab, ab.byteLength, ab.detached);"##
        ),
        r##"ArrayBuffer { (detached), [byteLength]: 0 } 0 true"##
    );
}

#[test]
fn detach_transfer_grows_and_shrinks() {
    assert_eq!(
        one(
            r##"const a=new ArrayBuffer(4); new Uint8Array(a)[0]=7; const b=a.transfer(6); const c=b.transferToFixedLength(2); console.log(a.detached, b.detached, c.byteLength, c.detached, new Uint8Array(c).join(), c.resizable);"##
        ),
        r##"true true 2 false 7,0 false"##
    );
}

#[test]
fn detach_transfer_argument_coercion() {
    assert_eq!(
        one(
            r##"console.log(new ArrayBuffer(2).transfer(NaN).byteLength, new ArrayBuffer(2).transfer('3').byteLength, new ArrayBuffer(2).transferToFixedLength(undefined).byteLength);"##
        ),
        r##"0 3 2"##
    );
}

#[test]
fn detach_transfer_argument_errors() {
    assert_eq!(
        one(
            r##"for (const f of [()=>new ArrayBuffer(2).transfer(-1), ()=>new ArrayBuffer(2).transfer(2**53), ()=>new ArrayBuffer(2).transferToFixedLength(-1), ()=>ArrayBuffer.prototype.transfer.call({})]) { try { f(); } catch (e) { console.log(e.name, e.message); } }"##
        ),
        r##"RangeError Invalid array buffer length|RangeError ArrayBuffer.prototype.transfer: Invalid length parameter|RangeError Invalid array buffer length|TypeError Method ArrayBuffer.prototype.transfer called on incompatible receiver #<Object>"##
    );
}

#[test]
fn detach_arraybuffer_detached_getter_brand() {
    assert_eq!(
        one(
            r##"const d=Object.getOwnPropertyDescriptor(ArrayBuffer.prototype,'detached'); console.log(typeof d.get, d.set, new ArrayBuffer(1).detached, d.get.call(new ArrayBuffer(1)));"##
        ),
        r##"function undefined false false"##
    );
}
