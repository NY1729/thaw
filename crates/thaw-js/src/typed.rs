//! ArrayBuffer and the typed arrays (`Uint8Array` & co); the methods live in a JS prelude on top of
//! the native storage, `Buffer` / `TextEncoder` / `TextDecoder` in `buffer.rs`.

use crate::*;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Ty {
    I8,
    U8,
    U8C,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    // BigInt64Array / BigUint64Array: elements are BigInts, so they have no f64 form (see `TypedArr::get`)
    I64,
    U64,
}

impl Ty {
    pub(crate) fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 | Ty::U8C => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 | Ty::I64 | Ty::U64 => 8,
        }
    }

    /// The BigInt element types.
    pub(crate) fn is_big(self) -> bool {
        matches!(self, Ty::I64 | Ty::U64)
    }

    pub(crate) fn tag(self) -> &'static str {
        match self {
            Ty::I8 => "[object Int8Array]",
            Ty::U8 => "[object Uint8Array]",
            Ty::U8C => "[object Uint8ClampedArray]",
            Ty::I16 => "[object Int16Array]",
            Ty::U16 => "[object Uint16Array]",
            Ty::I32 => "[object Int32Array]",
            Ty::U32 => "[object Uint32Array]",
            Ty::F32 => "[object Float32Array]",
            Ty::F64 => "[object Float64Array]",
            Ty::I64 => "[object BigInt64Array]",
            Ty::U64 => "[object BigUint64Array]",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Ty::I8 => "Int8Array",
            Ty::U8 => "Uint8Array",
            Ty::U8C => "Uint8ClampedArray",
            Ty::I16 => "Int16Array",
            Ty::U16 => "Uint16Array",
            Ty::I32 => "Int32Array",
            Ty::U32 => "Uint32Array",
            Ty::F32 => "Float32Array",
            Ty::F64 => "Float64Array",
            Ty::I64 => "BigInt64Array",
            Ty::U64 => "BigUint64Array",
        }
    }
}

const ALL: [Ty; 11] = [
    Ty::I8,
    Ty::U8,
    Ty::U8C,
    Ty::I16,
    Ty::U16,
    Ty::I32,
    Ty::U32,
    Ty::F32,
    Ty::F64,
    Ty::I64,
    Ty::U64,
];

/// The memory of a SharedArrayBuffer. Every store over it (`sab_store`) and every in-flight wire that names it
/// (worker.rs) holds an `Arc`, so the memory lives while any of them does.
///
/// SAFETY invariants (the unsafe code is `new_sab_block`, `sab_store`, `Store::drop` and `SabBlock::drop`):
/// 1. A SAB store's `cell` is a `Vec<u8>` aliasing `[ptr, ptr + len)` with capacity `len`. That Vec is never
///    dropped, reallocated, resized, shrunk or replaced: `Store::drop` forgets it, `detach_bytes` leaves a shared
///    store alone, and no code calls a length-changing method on a store's Vec (test
///    `grep_guard_no_length_change_on_a_sab_backed_vec`).
/// 2. The allocation is freed only by `SabBlock::drop`, when the last `Arc` goes.
/// 3. `ptr` comes from a `u64` allocation, so it is 8-byte aligned; element offsets that are multiples of the
///    element size are aligned for atomic access.
/// 4. The length is fixed for the block's lifetime (SharedArrayBuffer growth is not supported).
/// 5. Two threads may touch the bytes at once through their own stores, so racing plain accesses are a data race in
///    Rust's model. ponytail: accepted because JS permits racy SAB access and the hot path has no atomic branch;
///    upgrade path: relaxed atomic bytes in `TypedArr::{get,set}` when `store.sab` is set.
pub(crate) struct SabBlock {
    ptr: *mut u64,
    words: usize,
    len: usize,
}

// SAFETY: raw memory that every holder reaches only through its store, under the invariants above.
unsafe impl Send for SabBlock {}
unsafe impl Sync for SabBlock {}

impl Drop for SabBlock {
    fn drop(&mut self) {
        // SAFETY: `ptr` and `words` are the `Box::into_raw` of `new_sab_block`, and the Vec aliases were forgotten
        // (invariant 1), so this is the only free.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.ptr, self.words)) });
    }
}

/// A zeroed block of `len` bytes.
pub(crate) fn new_sab_block(len: usize) -> Arc<SabBlock> {
    let words = len.div_ceil(8);
    let ptr = Box::into_raw(vec![0u64; words].into_boxed_slice()) as *mut u64;
    Arc::new(SabBlock { ptr, words, len })
}

/// A zeroed SharedArrayBuffer storage of `len` bytes.
pub(crate) fn new_sab_bytes(len: usize) -> Bytes {
    // counted once, on the creating thread: an importing store (`sab_store` alone) charges nothing
    let bytes = sab_store(new_sab_block(len));
    bytes.charged.set(heap_limit::charge(len));
    bytes
}

/// An attached storage over `block`: the same bytes as every other store of the block.
pub(crate) fn sab_store(block: Arc<SabBlock>) -> Bytes {
    // SAFETY: the Vec aliases the block's allocation (`words * 8 >= len` bytes, so `len` bytes are live). It is
    // never dropped (invariant 1), which is why its allocation layout (align 8) never has to match a `Vec<u8>`'s.
    let alias = unsafe { Vec::from_raw_parts(block.ptr.cast::<u8>(), block.len, block.len) };
    Rc::new(Store {
        cell: RefCell::new(alias),
        shared: None,
        detached: Cell::new(false),
        sab: Some(block),
        charged: Cell::new(0),
        max: None,
    })
}

/// An ArrayBuffer's storage. `detached` is set by `detach_bytes` (ArrayBuffer.prototype.transfer and
/// the wasm memory grow path); a detached store is empty and its views clamp to length 0.
#[derive(Default)]
pub(crate) struct Store {
    cell: RefCell<Vec<u8>>,
    /// Storage owned by the wasm engine (a wasm memory's bytes, see `thaw-wasm`); `cell` is unused then.
    /// The owner empties it when the memory is replaced, which is how a stale view sees length 0.
    shared: Option<Rc<RefCell<Vec<u8>>>>,
    detached: Cell<bool>,
    /// The block of a SharedArrayBuffer; `cell` then aliases it (see `SabBlock`).
    sab: Option<Arc<SabBlock>>,
    /// Bytes this storage charged to the heap limit (`heap_limit`); 0 once detached.
    charged: Cell<usize>,
    /// `maxByteLength` of a resizable ArrayBuffer (`resize_bytes` moves its length within it); None when fixed.
    max: Option<usize>,
}

impl Drop for Store {
    fn drop(&mut self) {
        heap_limit::credit(self.charged.get());
        if self.sab.is_some() {
            // invariant 1: the alias must not free the block; `sab` then drops its Arc
            std::mem::forget(std::mem::take(self.cell.get_mut()));
        }
    }
}

impl std::ops::Deref for Store {
    type Target = RefCell<Vec<u8>>;
    fn deref(&self) -> &RefCell<Vec<u8>> {
        match &self.shared {
            Some(shared) => shared,
            None => &self.cell,
        }
    }
}

pub(crate) type Bytes = Rc<Store>;

/// A fresh, attached storage owning `v`.
pub(crate) fn new_bytes(v: Vec<u8>) -> Bytes {
    new_bytes_max(v, None)
}

/// A fresh, attached storage owning `v`, resizable up to `max` when given (a resizable ArrayBuffer).
pub(crate) fn new_bytes_max(v: Vec<u8>, max: Option<usize>) -> Bytes {
    let charged = heap_limit::charge(v.len());
    Rc::new(Store {
        cell: RefCell::new(v),
        shared: None,
        detached: Cell::new(false),
        sab: None,
        charged: Cell::new(charged),
        max,
    })
}

/// Sets a resizable storage's length to `n` (the caller has checked `n <= max`): new bytes are zero, a shrink drops
/// the tail. The heap-limit charge follows the length. False, with the length unchanged, when the memory cannot be
/// had (the caller throws a RangeError).
pub(crate) fn resize_bytes(b: &Bytes, n: usize) -> bool {
    debug_assert!(
        b.sab.is_none() && b.max.is_some(),
        "resize_bytes on a fixed or shared storage"
    );
    let mut v = b.cell.borrow_mut();
    let cur = v.len();
    if n > cur {
        if v.try_reserve_exact(n - cur).is_err() {
            return false;
        }
        b.charged.set(b.charged.get() + heap_limit::charge(n - cur));
    } else {
        let credit = (cur - n).min(b.charged.get());
        heap_limit::credit(credit);
        b.charged.set(b.charged.get() - credit);
    }
    v.resize(n, 0);
    true
}

/// Whether an ArrayBuffer's storage is a SharedArrayBuffer block (`util.types.isSharedArrayBuffer`).
pub(crate) fn bytes_is_shared(b: &Bytes) -> bool {
    b.sab.is_some()
}

#[cfg(feature = "wasm")]
/// An attached storage over bytes the wasm engine owns (no copy; the same allocation as the engine's memory).
pub(crate) fn shared_bytes(shared: Rc<RefCell<Vec<u8>>>) -> Bytes {
    Rc::new(Store {
        cell: RefCell::new(Vec::new()),
        shared: Some(shared),
        detached: Cell::new(false),
        sab: None,
        charged: Cell::new(0),
        max: None,
    })
}

/// Detach a storage: its bytes are released (length 0) and it reports `detached`. Idempotent.
/// A shared storage keeps its bytes (the engine owns them); only the flag changes.
pub(crate) fn detach_bytes(b: &Bytes) {
    // a SharedArrayBuffer is never detached: its Vec aliases a shared block (invariant 1)
    if b.sab.is_some() {
        debug_assert!(false, "detach_bytes on a SharedArrayBuffer");
        return;
    }
    if b.shared.is_none() {
        *b.cell.borrow_mut() = Vec::new();
    }
    heap_limit::credit(b.charged.replace(0));
    b.detached.set(true);
}

#[cfg(feature = "wasm")]
/// True when `b` is the storage over exactly this engine-owned allocation (see `shared_bytes`).
pub(crate) fn same_shared(b: &Bytes, shared: &Rc<RefCell<Vec<u8>>>) -> bool {
    b.shared.as_ref().is_some_and(|s| Rc::ptr_eq(s, shared))
}

/// True once `detach_bytes` has run on this storage, or a shared storage was emptied by its engine owner.
pub(crate) fn is_detached(b: &Bytes) -> bool {
    b.detached.get() || b.shared.as_ref().is_some_and(|s| s.borrow().is_empty())
}

#[derive(Clone)]
pub(crate) struct TypedArr {
    pub(crate) abuf: Obj,
    pub(crate) data: Bytes,
    pub(crate) ty: Ty,
    pub(crate) off: usize,
    /// The element count of a fixed-length view (meaningless for a length-tracking one: use `view_len`).
    pub(crate) fixed_len: usize,
    /// Length-tracking (`new Uint8Array(rab)`): the view runs from `off` to the buffer's current end.
    pub(crate) track: bool,
    /// The buffer is resizable, so the length is re-derived on each access (`view_len`); false keeps the fixed path.
    pub(crate) rz: bool,
}

/// What an integer-indexed exotic object does with a property key (see `TypedArr::key`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum TypedKey {
    Index(usize),
    Invalid,
    Ordinary,
}

/// The own-property answer for a numeric key of a typed array: `Some(true)` for a valid element, `Some(false)`
/// for any other canonical numeric key, `None` for an ordinary key or any non-typed object.
pub(crate) fn typed_own_key(o: &Obj, key: &str) -> Option<bool> {
    match &*o.kind.borrow() {
        Kind::Typed(t) => match t.key(key) {
            TypedKey::Index(_) => Some(true),
            TypedKey::Invalid => Some(false),
            TypedKey::Ordinary => None,
        },
        _ => None,
    }
}

impl TypedArr {
    /// The element count right now, or None when the view is out of bounds: its buffer is detached or shrank below
    /// the window (a fixed view), or below its offset (a length-tracking one). Whole elements only.
    #[inline]
    pub(crate) fn view_len(&self) -> Option<usize> {
        let bytes = self.data.borrow().len();
        if self.track {
            bytes.checked_sub(self.off).map(|t| t / self.ty.size())
        } else if self.off + self.fixed_len * self.ty.size() <= bytes {
            Some(self.fixed_len)
        } else {
            None
        }
    }

    /// The view is in bounds (false once the buffer is detached or the window no longer fits).
    #[inline]
    pub(crate) fn live(&self) -> bool {
        self.view_len().is_some()
    }

    /// Detached or out of bounds: the methods that throw a TypeError on it.
    pub(crate) fn gone(&self) -> bool {
        is_detached(&self.data) || !self.live()
    }

    /// Element `i` is inside the view now. A fixed view over a fixed buffer is just the index check: the one branch
    /// the hot path pays (the buffer cannot shrink without detaching, and the byte access is bounds-checked anyway).
    #[inline]
    fn has(&self, i: usize) -> bool {
        if self.rz {
            self.view_len().is_some_and(|n| i < n)
        } else {
            i < self.fixed_len
        }
    }

    /// The length a script sees: 0 when out of bounds (detached, or a window that no longer fits), the live length
    /// otherwise.
    pub(crate) fn cur_len(&self) -> usize {
        self.view_len().unwrap_or(0)
    }

    /// The offset a script sees: 0 for a detached buffer, `off` otherwise.
    pub(crate) fn cur_off(&self) -> usize {
        if self.live() {
            self.off
        } else {
            0
        }
    }

    /// A copy of the bytes the view covers (empty when detached).
    pub(crate) fn window(&self) -> Vec<u8> {
        let Some(n) = self.view_len() else {
            return Vec::new();
        };
        let d = self.data.borrow();
        d[self.off..self.off + n * self.ty.size()].to_vec()
    }

    #[inline]
    pub(crate) fn get(&self, i: usize) -> Option<f64> {
        if !self.has(i) {
            return None;
        }
        let d = self.data.borrow();
        let p = self.off + i * self.ty.size();
        // one bounds check per read: a fixed-size copy of the element bytes
        Some(match self.ty {
            Ty::I8 => d.get(p).copied()? as i8 as f64,
            Ty::U8 | Ty::U8C => d.get(p).copied()? as f64,
            Ty::I16 => i16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?) as f64,
            Ty::U16 => u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?) as f64,
            Ty::I32 => i32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as f64,
            Ty::U32 => u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as f64,
            Ty::F32 => f32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as f64,
            Ty::F64 => f64::from_le_bytes(d.get(p..p + 8)?.try_into().ok()?),
            // no f64 form: the caller takes its slow path (`get_value`)
            Ty::I64 | Ty::U64 => return None,
        })
    }

    /// The element as a JS value (a Number, or a BigInt for the 64-bit types); None when out of range.
    pub(crate) fn get_value(&self, i: usize) -> Option<Value> {
        if !self.ty.is_big() {
            return self.get(i).map(Value::Number);
        }
        if !self.has(i) {
            return None;
        }
        let d = self.data.borrow();
        let p = self.off + i * 8;
        let raw: [u8; 8] = d.get(p..p + 8)?.try_into().ok()?;
        let v = if self.ty == Ty::I64 {
            BigInt::from(i64::from_le_bytes(raw))
        } else {
            BigInt::from(u64::from_le_bytes(raw))
        };
        Some(crate::bigint::big(v))
    }

    /// Store a JS value at `i` (ToNumber for the numeric types, ToBigInt for the 64-bit ones, which throws
    /// for a Number). An out-of-range index is a no-op after the conversion, as the spec orders it.
    pub(crate) fn set_value(&self, it: &mut Interp, i: usize, v: &Value) -> R<()> {
        if self.ty.is_big() {
            let x = it.bigint_element_of(v)?;
            self.set_big(i, &x);
        } else {
            let n = it.num(v);
            self.set(i, n);
        }
        Ok(())
    }

    /// How this array treats property key `k` (the integer-indexed exotic object rules): a valid index is an
    /// element; any other CanonicalNumericIndexString (`-1`, `1.5`, `-0`, `NaN`, `Infinity`, `4294967296`) is
    /// `Invalid` and never reaches the ordinary properties; every other key is `Ordinary`.
    pub(crate) fn key(&self, k: &str) -> TypedKey {
        match array_index(k) {
            Some(i) if i < self.cur_len() => TypedKey::Index(i),
            Some(_) => TypedKey::Invalid,
            None if canonical_numeric_index(k).is_some() => TypedKey::Invalid,
            None => TypedKey::Ordinary,
        }
    }

    /// Wraps modulo 2^64 like the spec's BigInt64 / BigUint64 stores.
    fn set_big(&self, i: usize, x: &BigInt) {
        if !self.has(i) || !self.live() {
            return;
        }
        let low = (x & BigInt::from(u64::MAX)).to_u64().unwrap_or(0);
        let p = self.off + i * 8;
        self.data.borrow_mut()[p..p + 8].copy_from_slice(&low.to_le_bytes());
    }

    /// Numeric store. The 64-bit types never reach it: `set_value` takes the BigInt path.
    pub(crate) fn set(&self, i: usize, v: f64) {
        if !self.has(i) || !self.live() {
            return;
        }
        if self.ty.is_big() {
            debug_assert!(false, "f64 store into a BigInt typed array");
            return;
        }
        let mut d = self.data.borrow_mut();
        let p = self.off + i * self.ty.size();
        let wrap = |bits: u32| -> u64 {
            if !v.is_finite() {
                0
            } else {
                (v.trunc().rem_euclid(2f64.powi(bits as i32))) as u64
            }
        };
        match self.ty {
            Ty::I8 | Ty::U8 => d[p] = wrap(8) as u8,
            Ty::U8C => {
                d[p] = if v.is_nan() {
                    0
                } else {
                    // round half to even, clamped
                    let c = v.clamp(0.0, 255.0);
                    let f = c.floor();
                    let r = if c - f > 0.5 || (c - f == 0.5 && f % 2.0 != 0.0) {
                        f + 1.0
                    } else {
                        f
                    };
                    r as u8
                }
            }
            Ty::I16 | Ty::U16 => d[p..p + 2].copy_from_slice(&(wrap(16) as u16).to_le_bytes()),
            Ty::I32 | Ty::U32 => d[p..p + 4].copy_from_slice(&(wrap(32) as u32).to_le_bytes()),
            Ty::F32 => d[p..p + 4].copy_from_slice(&(v as f32).to_le_bytes()),
            Ty::F64 => d[p..p + 8].copy_from_slice(&v.to_le_bytes()),
            Ty::I64 | Ty::U64 => unreachable!("guarded above"),
        }
    }

    /// The numeric elements. Numeric types only: a 64-bit array has no f64 form, use `to_values`.
    pub(crate) fn to_vec(&self) -> Vec<f64> {
        debug_assert!(!self.ty.is_big(), "to_vec on a BigInt typed array");
        (0..self.cur_len()).filter_map(|i| self.get(i)).collect()
    }

    /// Every element as a JS value.
    pub(crate) fn to_values(&self) -> Vec<Value> {
        (0..self.cur_len())
            .filter_map(|i| self.get_value(i))
            .collect()
    }
}

impl Interp {
    pub(crate) fn typed_of(v: &Value) -> Option<TypedArr> {
        match v {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::Typed(t) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn bytes_of_buffer(v: &Value) -> Option<Bytes> {
        match v {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::ArrayBuffer(b) => Some(b.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// An `ArrayBuffer` object over an existing storage (shares its bytes; the wasm memory path uses it).
    pub(crate) fn new_array_buffer(&self, data: Bytes) -> Obj {
        new_obj(self.rt.arraybuffer_proto.clone(), Kind::ArrayBuffer(data))
    }

    /// A SharedArrayBuffer: the same storage kind as an ArrayBuffer under its own prototype, over a shared block
    /// (`new_sab_bytes` / `sab_store`); the prototype and the block are the two faces of one fact.
    pub(crate) fn new_shared_buffer(&self, data: Bytes) -> Obj {
        debug_assert!(
            data.sab.is_some(),
            "a SharedArrayBuffer is over a shared block"
        );
        new_obj(
            self.rt.sharedarraybuffer_proto.clone(),
            Kind::ArrayBuffer(data),
        )
    }

    /// The storage behind an ArrayBuffer receiver (a SharedArrayBuffer is not one: V8 rejects it too).
    fn ab_receiver(&mut self, this: &Value, method: &str) -> R<Bytes> {
        let shared = match this {
            Value::Object(o) => match (&*o.proto.borrow(), &self.rt.sharedarraybuffer_proto) {
                (Some(op), Some(sp)) => Rc::ptr_eq(op, sp),
                _ => false,
            },
            _ => false,
        };
        match Self::bytes_of_buffer(this) {
            Some(b) if !shared => Ok(b),
            _ => {
                let desc = match this {
                    Value::Object(_) if shared => "#<SharedArrayBuffer>".to_string(),
                    _ => "#<Object>".to_string(),
                };
                self.throw_error(
                    "TypeError",
                    &format!("Method {method} called on incompatible receiver {desc}"),
                )
            }
        }
    }

    /// `new ArrayBuffer(len, options)`'s `maxByteLength`: None (a fixed buffer) unless options is an object whose
    /// member is not undefined. `len` must not exceed it.
    fn ab_max_option(&mut self, options: &Value, len: usize) -> R<Option<usize>> {
        if !matches!(options, Value::Object(_)) {
            return Ok(None);
        }
        let m = self.get(options, "maxByteLength")?;
        if matches!(m, Value::Undefined) {
            return Ok(None);
        }
        let m = self.num(&m);
        let m = if m.is_nan() { 0.0 } else { m.trunc() };
        if !(0.0..=9007199254740991.0).contains(&m) || len as f64 > m {
            return self.throw_error("RangeError", "Invalid array buffer max length");
        }
        Ok(Some(m as usize))
    }

    /// `ArrayBuffer.prototype.resize(newLength)`: a resizable buffer's length moves within its max. The order is the
    /// spec's: receiver, then ToIndex (which may run user code), then detached, then the max.
    fn ab_resize(&mut self, this: &Value, a: &[Value]) -> R<Value> {
        let data = self.ab_receiver(this, "ArrayBuffer.prototype.resize")?;
        let Some(max) = data.max else {
            return self.throw_error(
                "TypeError",
                "Method ArrayBuffer.prototype.resize called on incompatible receiver #<ArrayBuffer>",
            );
        };
        let n = self.num(&arg(a, 0));
        let n = if n.is_nan() { 0.0 } else { n.trunc() };
        if !(0.0..=9007199254740991.0).contains(&n) {
            return self.throw_error(
                "RangeError",
                "ArrayBuffer.prototype.resize: Invalid length parameter",
            );
        }
        if is_detached(&data) {
            return self.throw_error(
                "TypeError",
                "Cannot perform ArrayBuffer.prototype.resize on a detached ArrayBuffer",
            );
        }
        if n > max as f64 {
            return self.throw_error(
                "RangeError",
                "ArrayBuffer.prototype.resize: Invalid length parameter",
            );
        }
        if n > 2f64.powi(32) || !resize_bytes(&data, n as usize) {
            return self.throw_error("RangeError", "Array buffer allocation failed");
        }
        self.check_interrupt()?;
        Ok(Value::Undefined)
    }

    /// `transfer(newLength?)` / `transferToFixedLength(newLength?)`: a new buffer with the same bytes
    /// (zero-extended or truncated) and the source detached. `keep_resizable` (transfer, not
    /// transferToFixedLength) keeps a resizable buffer's max, and the new length must fit it.
    fn ab_transfer(
        &mut self,
        this: &Value,
        a: &[Value],
        method: &str,
        keep_resizable: bool,
    ) -> R<Value> {
        let data = self.ab_receiver(this, method)?;
        if is_detached(&data) {
            return self.throw_error(
                "TypeError",
                &format!("Cannot perform {method} on a detached ArrayBuffer"),
            );
        }
        let cur = data.borrow().len();
        let max = if keep_resizable { data.max } else { None };
        let new_len = match arg(a, 0) {
            Value::Undefined => cur,
            v => {
                let n = self.num(&v);
                let n = if n.is_nan() { 0.0 } else { n.trunc() };
                if n < 0.0 {
                    return self.throw_error("RangeError", "Invalid array buffer length");
                }
                if n > 9007199254740991.0 {
                    return self
                        .throw_error("RangeError", &format!("{method}: Invalid length parameter"));
                }
                if n > 2f64.powi(32) {
                    return self.throw_error("RangeError", "Array buffer allocation failed");
                }
                n as usize
            }
        };
        if max.is_some_and(|m| new_len > m) {
            return self.throw_error("RangeError", "Invalid array buffer length");
        }
        let mut out = data.borrow().clone();
        if out.try_reserve(new_len.saturating_sub(cur)).is_err() {
            return self.throw_error("RangeError", "Array buffer allocation failed");
        }
        out.resize(new_len, 0);
        detach_bytes(&data);
        Ok(Value::Object(
            self.new_array_buffer(new_bytes_max(out, max)),
        ))
    }

    /// `new ArrayBuffer(n)` / `new SharedArrayBuffer(n)`: zero-filled, `n` truncated, NaN = 0.
    fn alloc_buffer(&mut self, args: &[Value], shared: bool) -> R<Value> {
        let n = self.num(&arg(args, 0));
        let n = if n.is_nan() { 0.0 } else { n };
        if !(0.0..=2f64.powi(32)).contains(&n) {
            return self.throw_error("RangeError", "Array buffer allocation failed");
        }
        let obj = if shared {
            self.new_shared_buffer(new_sab_bytes(n as usize))
        } else {
            let max = self.ab_max_option(&arg(args, 1), n as usize)?;
            self.new_array_buffer(new_bytes_max(vec![0; n as usize], max))
        };
        // the bytes are charged above: stop here if that tripped the worker's heap limit
        self.check_interrupt()?;
        Ok(Value::Object(obj))
    }

    /// The bytes `[start, end)` of a buffer for `slice(start, end)` (negative = from the end).
    fn slice_bytes(it: &mut Interp, data: &Bytes, a: &[Value]) -> Vec<u8> {
        let n = data.borrow().len() as f64;
        let rel = |it: &mut Interp, v: Value, d: f64| {
            let x = if matches!(v, Value::Undefined) {
                d
            } else {
                it.num(&v).trunc()
            };
            if x < 0.0 {
                (n + x).max(0.0)
            } else {
                x.min(n)
            }
        };
        let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
        if e > s {
            data.borrow()[s as usize..e as usize].to_vec()
        } else {
            Vec::new()
        }
    }

    /// A view over `data` from `off`: `len` elements, or length-tracking when `track` (a tracking view covers the
    /// buffer to its current end, so `len` is then only a placeholder).
    pub(crate) fn new_typed(
        &self,
        ty: Ty,
        abuf: Obj,
        data: Bytes,
        off: usize,
        len: usize,
        track: bool,
    ) -> Value {
        let proto = self.rt.typed_protos.get(ty as usize).cloned();
        let rz = data.max.is_some();
        Value::Object(new_obj(
            proto,
            Kind::Typed(TypedArr {
                abuf,
                data,
                ty,
                off,
                fixed_len: len,
                track,
                rz,
            }),
        ))
    }

    /// A fresh zero-filled typed array.
    pub(crate) fn typed_with_len(&self, ty: Ty, len: usize) -> Value {
        let data: Bytes = new_bytes(vec![0; len * ty.size()]);
        let abuf = self.new_array_buffer(data.clone());
        self.new_typed(ty, abuf, data, 0, len, false)
    }

    /// The bytes a typed array views (its window, not the whole buffer) or an `ArrayBuffer` holds; None for anything else.
    pub(crate) fn view_bytes(v: &Value) -> Option<Vec<u8>> {
        if let Some(t) = Self::typed_of(v) {
            return Some(t.window());
        }
        Self::bytes_of_buffer(v).map(|b| b.borrow().clone())
    }

    /// An `ArrayBuffer` owning `bytes`.
    pub(crate) fn array_buffer_from_vec(&self, bytes: Vec<u8>) -> Value {
        Value::Object(self.new_array_buffer(new_bytes(bytes)))
    }

    /// A `Uint8Array` over a copy of `bytes`.
    pub(crate) fn u8_from_vec(&self, bytes: Vec<u8>) -> Value {
        let len = bytes.len();
        let data: Bytes = new_bytes(bytes);
        let abuf = self.new_array_buffer(data.clone());
        self.new_typed(Ty::U8, abuf, data, 0, len, false)
    }

    /// A typed array source copies element-wise only between two BigInt types or two numeric ones (spec).
    fn check_mix(&mut self, src: Ty, dst: Ty) -> R<()> {
        if src.is_big() != dst.is_big() {
            return self.throw_error(
                "TypeError",
                "Cannot mix BigInt and other types, use explicit conversions",
            );
        }
        Ok(())
    }

    fn construct_typed(&mut self, ty: Ty, args: &[Value]) -> R<Value> {
        let a0 = arg(args, 0);
        match &a0 {
            Value::Undefined | Value::Null => Ok(self.typed_with_len(ty, 0)),
            Value::Number(n) => {
                if !(*n >= 0.0 && n.is_finite()) || *n > 2f64.powi(32) {
                    return self.throw_error(
                        "RangeError",
                        &format!("Invalid typed array length: {}", number_to_string(*n)),
                    );
                }
                Ok(self.typed_with_len(ty, *n as usize))
            }
            Value::Object(o) => {
                if let Some(data) = Self::bytes_of_buffer(&a0) {
                    if is_detached(&data) {
                        return self.throw_error(
                            "TypeError",
                            "Cannot perform Construct on a detached ArrayBuffer",
                        );
                    }
                    let total = data.borrow().len();
                    let off = match arg(args, 1) {
                        Value::Undefined => 0,
                        v => self.num(&v) as usize,
                    };
                    if off % ty.size() != 0 {
                        return self.throw_error(
                            "RangeError",
                            &format!(
                                "start offset of {} should be a multiple of {}",
                                ty.name(),
                                ty.size()
                            ),
                        );
                    }
                    if off > total {
                        return self.throw_error(
                            "RangeError",
                            &format!("Start offset {off} is outside the bounds of the buffer"),
                        );
                    }
                    // a resizable buffer with no length gives a length-tracking view (spec: IsFixedLength false)
                    let track = data.max.is_some() && matches!(arg(args, 2), Value::Undefined);
                    let len = match arg(args, 2) {
                        Value::Undefined if track => 0,
                        Value::Undefined => {
                            if (total - off) % ty.size() != 0 {
                                return self.throw_error(
                                    "RangeError",
                                    &format!(
                                        "byte length of {} should be a multiple of {}",
                                        ty.name(),
                                        ty.size()
                                    ),
                                );
                            }
                            (total - off) / ty.size()
                        }
                        v => self.num(&v) as usize,
                    };
                    if !track && off + len * ty.size() > total {
                        return self.throw_error(
                            "RangeError",
                            &format!("Invalid typed array length: {len}"),
                        );
                    }
                    return Ok(self.new_typed(ty, o.clone(), data, off, len, track));
                }
                let items: Vec<Value> = if let Some(t) = Self::typed_of(&a0) {
                    self.check_mix(t.ty, ty)?;
                    t.to_values()
                } else if let Some(items) = self.iter_protocol(&a0)? {
                    items
                } else {
                    let n = self.get(&a0, "length")?;
                    let n = self.num(&n);
                    let mut v = Vec::new();
                    for i in 0..(if n.is_nan() { 0 } else { n as usize }) {
                        v.push(self.get(&a0, &i.to_string())?);
                    }
                    v
                };
                let out = self.typed_with_len(ty, items.len());
                let t = Self::typed_of(&out).unwrap();
                for (i, v) in items.iter().enumerate() {
                    t.set_value(self, i, v)?;
                }
                Ok(out)
            }
            other => {
                let n = self.num(other);
                Ok(self.typed_with_len(ty, if n.is_nan() { 0 } else { n as usize }))
            }
        }
    }

    pub(crate) fn install_typed(&mut self) {
        let g = self.global.clone();
        // ----- ArrayBuffer -----
        let ab_proto = self.plain();
        self.rt.arraybuffer_proto = Some(ab_proto.clone());
        let ab = self.native(|it, _, args| it.alloc_buffer(args, false));
        if let Value::Object(c) = &ab {
            put_hidden(c, "prototype", Value::Object(ab_proto.clone()));
            put_hidden(&ab_proto, "constructor", ab.clone());
            self.set_native(c, "isView", |_, _, a| {
                Ok(Value::Bool(Self::typed_of(&arg(a, 0)).is_some()))
            });
        }
        self.set_native(&ab_proto, "slice", |it, this, a| {
            let data = it.ab_receiver(this, "ArrayBuffer.prototype.slice")?;
            if is_detached(&data) {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer",
                );
            }
            let out = Self::slice_bytes(it, &data, a);
            Ok(Value::Object(it.new_array_buffer(new_bytes(out))))
        });
        self.set_native(&ab_proto, "transfer", |it, this, a| {
            it.ab_transfer(this, a, "ArrayBuffer.prototype.transfer", true)
        });
        self.set_native(&ab_proto, "transferToFixedLength", |it, this, a| {
            it.ab_transfer(
                this,
                a,
                "ArrayBuffer.prototype.transferToFixedLength",
                false,
            )
        });
        self.set_native(&ab_proto, "resize", |it, this, a| it.ab_resize(this, a));
        let detached_get = self.native(|it, this, _| {
            let data = it.ab_receiver(this, "get ArrayBuffer.prototype.detached")?;
            Ok(Value::Bool(is_detached(&data)))
        });
        define_accessor(&ab_proto, "detached", Some(detached_get), None, false);
        let resizable_get = self.native(|it, this, _| {
            let data = it.ab_receiver(this, "get ArrayBuffer.prototype.resizable")?;
            Ok(Value::Bool(data.max.is_some()))
        });
        define_accessor(&ab_proto, "resizable", Some(resizable_get), None, false);
        // a detached buffer reports 0 (its max is kept for `resizable`); a fixed one reports its length
        let max_get = self.native(|it, this, _| {
            let data = it.ab_receiver(this, "get ArrayBuffer.prototype.maxByteLength")?;
            let n = match data.max {
                _ if is_detached(&data) => 0,
                Some(m) => m,
                None => data.borrow().len(),
            };
            Ok(Value::Number(n as f64))
        });
        define_accessor(&ab_proto, "maxByteLength", Some(max_get), None, false);
        g.declare("ArrayBuffer", ab, true);

        // ----- SharedArrayBuffer: same storage, own prototype (not an ArrayBuffer); byteLength is served by
        // the ArrayBuffer fast path in lib.rs, the prelude adds the brand-checked accessors -----
        let sab_proto = self.plain();
        self.rt.sharedarraybuffer_proto = Some(sab_proto.clone());
        let sab = self.native(|it, _, args| it.alloc_buffer(args, true));
        if let Value::Object(c) = &sab {
            put_hidden(c, "prototype", Value::Object(sab_proto.clone()));
            put_hidden(&sab_proto, "constructor", sab.clone());
        }
        self.set_native(&sab_proto, "slice", |it, this, a| {
            let Some(data) = Self::bytes_of_buffer(this) else {
                return it.throw_error("TypeError", "Receiver is not a SharedArrayBuffer");
            };
            let out = Self::slice_bytes(it, &data, a);
            let copy = new_sab_bytes(out.len());
            copy.borrow_mut().copy_from_slice(&out);
            Ok(Value::Object(it.new_shared_buffer(copy)))
        });
        g.declare("SharedArrayBuffer", sab, true);
        // length-tracking and out-of-bounds views (the prelude's brand checks and DataView / structuredClone use them)
        let oob = self.native(|_, _, a| {
            Ok(Value::Bool(
                Self::typed_of(&arg(a, 0)).is_some_and(|t| !t.live()),
            ))
        });
        g.declare("__thaw_ta_oob", oob, true);
        let track = self.native(|_, _, a| {
            Ok(Value::Bool(
                Self::typed_of(&arg(a, 0)).is_some_and(|t| t.track),
            ))
        });
        g.declare("__thaw_ta_track", track, true);
        // the wire and structuredClone move a block between Interps as an id (the table is in worker.rs)
        let export = self.native(|it, _, a| {
            let Some(block) = Self::bytes_of_buffer(&arg(a, 0)).and_then(|b| b.sab.clone()) else {
                return it.throw_error("TypeError", "value is not a SharedArrayBuffer");
            };
            Ok(Value::Number(crate::worker::sab_export(block) as f64))
        });
        g.declare("__thaw_sab_export", export, true);
        let import = self.native(|it, _, a| {
            let id = it.num(&arg(a, 0)) as u64;
            match crate::worker::sab_import(id) {
                Some(block) => Ok(Value::Object(it.new_shared_buffer(sab_store(block)))),
                None => it.throw_error("TypeError", "unknown SharedArrayBuffer id"),
            }
        });
        g.declare("__thaw_sab_import", import, true);
        // a transferred ArrayBuffer moves its bytes to the in-flight table (worker.rs, AB_IN_FLIGHT) and is detached;
        // the decode that reads the wire gives them to a new buffer (no copy)
        let take = self.native(|it, _, a| {
            let Some(data) = Self::bytes_of_buffer(&arg(a, 0)) else {
                return it.throw_error("TypeError", "value is not an ArrayBuffer");
            };
            if data.sab.is_some() || is_detached(&data) {
                return it.throw_error("TypeError", "Cannot transfer object of unsupported type.");
            }
            // a wasm memory's bytes belong to its engine: those are copied, the storage is detached all the same
            let bytes = if data.shared.is_some() {
                data.borrow().clone()
            } else {
                std::mem::take(&mut *data.borrow_mut())
            };
            let max = data.max;
            detach_bytes(&data);
            Ok(Value::Number(crate::worker::ab_export(bytes, max) as f64))
        });
        g.declare("__thaw_ab_take", take, true);
        let put = self.native(|it, _, a| {
            let id = it.num(&arg(a, 0)) as u64;
            let Some((bytes, max)) = crate::worker::ab_import(id) else {
                return it.throw_error("TypeError", "unknown ArrayBuffer id");
            };
            let buffer = Value::Object(it.new_array_buffer(new_bytes_max(bytes, max)));
            it.check_interrupt()?;
            Ok(buffer)
        });
        g.declare("__thaw_ab_put", put, true);
        // Atomics over a SharedArrayBuffer (extras.rs sends only shared buffers here; the logic is atomics.rs)
        let atomic = self.native(|it, _, a| crate::atomics::atomic(it, a));
        g.declare("__thaw_atomic", atomic, true);
        let wait = self.native(|it, _, a| crate::atomics::wait(it, a));
        g.declare("__thaw_atomics_wait", wait, true);
        let notify = self.native(|it, _, a| crate::atomics::notify(it, a));
        g.declare("__thaw_atomics_notify", notify, true);
        let wait_async = self.native(|it, _, a| crate::atomics::wait_async(it, a));
        g.declare("__thaw_atomics_wait_async", wait_async, true);
        let cancel = self.native(|it, _, a| crate::atomics::cancel(it, a));
        g.declare("__thaw_atomics_cancel", cancel, true);

        // ----- typed arrays -----
        let ta_proto = self.plain(); // %TypedArray%.prototype: methods come from the JS prelude
        self.rt.typed_base_proto = Some(ta_proto.clone());
        // %TypedArray%.prototype[@@toStringTag]: the [[TypedArrayName]] of the receiver (undefined for others)
        let tag_get = self.native(|_, this, _| {
            Ok(match Self::typed_of(this) {
                Some(t) => Value::str(t.ty.name()),
                None => Value::Undefined,
            })
        });
        let tag_key = self.well_known_key("toStringTag");
        crate::define_accessor(&ta_proto, &tag_key, Some(tag_get), None, false);
        // %TypedArray% itself: abstract, and the [[Prototype]] of every concrete constructor (so
        // `from`/`of` and `Function.prototype` are inherited, as in V8).
        let Value::Object(ta_ctor) = self.native(|it, _, _| {
            it.throw_error(
                "TypeError",
                "Abstract class TypedArray not directly constructable",
            )
        }) else {
            unreachable!("a native is an object")
        };
        put_hidden(&ta_ctor, "prototype", Value::Object(ta_proto.clone()));
        put_hidden(&ta_proto, "constructor", Value::Object(ta_ctor.clone()));
        for ty in ALL {
            let proto = new_obj(Some(ta_proto.clone()), Kind::Plain);
            self.rt.typed_protos.push(proto.clone());
            let ctor = self.native(move |it, _, args| it.construct_typed(ty, args));
            if let Value::Object(c) = &ctor {
                *c.proto.borrow_mut() = Some(ta_ctor.clone());
                put_hidden(c, "prototype", Value::Object(proto.clone()));
                put_hidden(&proto, "constructor", ctor.clone());
                put_hidden(c, "name", Value::str(ty.name()));
                put_hidden(c, "length", Value::Number(3.0));
                put_hidden(c, "BYTES_PER_ELEMENT", Value::Number(ty.size() as f64));
                put_hidden(&proto, "BYTES_PER_ELEMENT", Value::Number(ty.size() as f64));
            }
            g.declare(ty.name(), ctor, true);
        }
        self.set_native(&ta_proto, "set", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            if t.gone() {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer",
                );
            }
            let off = match arg(a, 1) {
                Value::Undefined => 0,
                v => it.num(&v) as usize,
            };
            // spec: a typed array source is copied after its checks; an array-like's length is checked before any element
            // is read, and a resize made by a getter in between just drops the writes past the new end
            let mut array_like = false;
            let src: Vec<Value> = if let Some(s) = Self::typed_of(&arg(a, 0)) {
                it.check_mix(s.ty, t.ty)?;
                s.to_values()
            } else {
                match it.iter_items(&arg(a, 0)) {
                    Some(i) => i,
                    None => {
                        let n = it.get(&arg(a, 0), "length")?;
                        let n = it.num(&n) as usize;
                        if off + n > t.cur_len() {
                            return it.throw_error("RangeError", "offset is out of bounds");
                        }
                        array_like = true;
                        let mut v = Vec::new();
                        for i in 0..n {
                            v.push(it.get(&arg(a, 0), &i.to_string())?);
                        }
                        v
                    }
                }
            };
            if !array_like && off + src.len() > t.cur_len() {
                return it.throw_error("RangeError", "offset is out of bounds");
            }
            for (i, v) in src.iter().enumerate() {
                t.set_value(it, off + i, v)?;
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ta_proto, "subarray", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            if is_detached(&t.data) {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform Construct on a detached ArrayBuffer",
                );
            }
            let n = t.cur_len() as f64;
            let rel = |it: &mut Interp, v: Value, d: f64| {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    it.num(&v).trunc()
                };
                if x < 0.0 {
                    (n + x).max(0.0)
                } else {
                    x.min(n)
                }
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
            let len = if e > s { (e - s) as usize } else { 0 };
            // a length-tracking view with no end stays tracking (spec: the result's length is auto too)
            let track = t.track && matches!(arg(a, 1), Value::Undefined);
            let view = it.new_typed(
                t.ty,
                t.abuf.clone(),
                t.data.clone(),
                t.off + s as usize * t.ty.size(),
                len,
                track,
            );
            // keep the receiver's prototype (Buffer subclasses)
            if let (Value::Object(v), Value::Object(r)) = (&view, this) {
                *v.proto.borrow_mut() = r.proto.borrow().clone();
            }
            Ok(view)
        });
        self.set_native(&ta_proto, "slice", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            if t.gone() {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform %TypedArray%.prototype.slice on a detached ArrayBuffer",
                );
            }
            let n = t.cur_len() as f64;
            let rel = |it: &mut Interp, v: Value, d: f64| {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    it.num(&v).trunc()
                };
                if x < 0.0 {
                    (n + x).max(0.0)
                } else {
                    x.min(n)
                }
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
            let (s, e) = (s as usize, (e as usize).max(s as usize));
            // same element type on both sides: the bytes copy as they are (no f64 round trip)
            let out = it.typed_with_len(t.ty, e - s);
            let o = Self::typed_of(&out).unwrap();
            let size = t.ty.size();
            let src = t.data.borrow();
            o.data.borrow_mut()[..(e - s) * size]
                .copy_from_slice(&src[t.off + s * size..t.off + e * size]);
            if let (Value::Object(v), Value::Object(r)) = (&out, this) {
                *v.proto.borrow_mut() = r.proto.borrow().clone();
            }
            Ok(out)
        });
        self.run_builtin(TYPED_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js typed array prelude must evaluate"));
    }
}

/// Generic methods are the Array ones (they only use `length` and indexing); the ones that must
/// build a typed result are written out.
const TYPED_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var AP = Array.prototype;
  var TP = Object.getPrototypeOf(Uint8Array.prototype);
  // Real accessors, so `Reflect.apply(getter, view, [])` brand-checks like V8 (the evaluator also
  // serves these keys directly on the object, so plain reads never reach them).
  var TAG = Symbol.toStringTag;
  function acc(o, k, f) { Object.defineProperty(o, k, { get: f, enumerable: false, configurable: true }); }
  var isTA = function (v) { return ArrayBuffer.isView(v) && !(typeof DataView === 'function' && v instanceof DataView); };
  ['buffer', 'byteOffset', 'byteLength', 'length'].forEach(function (k) {
    acc(TP, k, { [k]: function () {
      if (!isTA(this)) throw new TypeError('Receiver is not a typed array');
      return this[k];
    } }[k]);
  });
  acc(ArrayBuffer.prototype, 'byteLength', function () {
    if (!(this instanceof ArrayBuffer)) throw new TypeError('Receiver is not an ArrayBuffer');
    return this.byteLength;
  });
  acc(SharedArrayBuffer.prototype, 'byteLength', function () {
    if (!(this instanceof SharedArrayBuffer)) throw new TypeError('Receiver is not a SharedArrayBuffer');
    return this.byteLength;
  });
  // The native getter, captured before any script can replace it; views over a detached (or shrunk, see
  // `__thaw_ta_oob`) buffer clamp to length 0 and the methods below throw V8's messages (recorded from Node 22).
  var detGet = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'detached').get;
  function gone(t) { return isTA(t) && (detGet.call(t.buffer) || __thaw_ta_oob(t)); }
  acc(SharedArrayBuffer.prototype, 'growable', function () { return false; });
  acc(SharedArrayBuffer.prototype, 'maxByteLength', function () {
    if (!(this instanceof SharedArrayBuffer)) throw new TypeError('Receiver is not a SharedArrayBuffer');
    return this.byteLength;
  });
  Object.defineProperty(SharedArrayBuffer.prototype, TAG, { value: 'SharedArrayBuffer', configurable: true });
  ['forEach', 'some', 'every', 'find', 'findIndex', 'findLast', 'findLastIndex', 'reduce', 'reduceRight', 'fill', 'at', 'toString', 'keys', 'values', 'entries'].forEach(function (n) {
    if (AP[n]) def(TP, n, AP[n]);
  });
  // Spec: %TypedArray%.prototype[Symbol.iterator] is the `values` function. Compiled `Array.from(view)` and `for..of` use the protocol.
  def(TP, Symbol.iterator, TP.values);
  def(TP, 'toLocaleString', AP.toString);
  def(TP, 'indexOf', function (v, from) {
    from = from === undefined ? 0 : (from < 0 ? Math.max(this.length + from, 0) : from);
    for (var i = from; i < this.length; i++) if (this[i] === v) return i;
    return -1;
  });
  def(TP, 'lastIndexOf', function (v, from) {
    from = from === undefined ? this.length - 1 : (from < 0 ? this.length + from : Math.min(from, this.length - 1));
    for (var i = from; i >= 0; i--) if (this[i] === v) return i;
    return -1;
  });
  def(TP, 'join', function (sep) {
    sep = sep === undefined ? ',' : String(sep);
    var out = '';
    for (var i = 0; i < this.length; i++) out += (i ? sep : '') + this[i];
    return out;
  });
  def(TP, 'includes', function (v, from) {
    for (var i = from || 0; i < this.length; i++) if (this[i] === v || (v !== v && this[i] !== this[i])) return true;
    return false;
  });
  def(TP, 'reverse', function () {
    for (var i = 0, j = this.length - 1; i < j; i++, j--) { var t = this[i]; this[i] = this[j]; this[j] = t; }
    return this;
  });
  def(TP, 'map', function (f, t) {
    var r = new this.constructor(this.length);
    for (var i = 0; i < this.length; i++) r[i] = f.call(t, this[i], i, this);
    return r;
  });
  def(TP, 'filter', function (f, t) {
    var kept = [];
    for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) kept.push(this[i]);
    return new this.constructor(kept);
  });
  def(TP, 'sort', function (cmp) {
    var a = Array.from(this);
    a.sort(cmp || function (x, y) { return x < y ? -1 : x > y ? 1 : 0; });
    for (var i = 0; i < a.length; i++) this[i] = a[i];
    return this;
  });
  def(TP, 'toSorted', function (cmp) { return this.slice().sort(cmp); });
  def(TP, 'toReversed', function () { return this.slice().reverse(); });
  def(TP, 'copyWithin', function (target, start, end) {
    var n = this.length, vals = Array.from(this);
    target = target < 0 ? Math.max(n + target, 0) : Math.min(target, n);
    start = start === undefined ? 0 : (start < 0 ? Math.max(n + start, 0) : Math.min(start, n));
    end = end === undefined ? n : (end < 0 ? Math.max(n + end, 0) : Math.min(end, n));
    for (var i = 0; i < end - start && target + i < n; i++) this[target + i] = vals[start + i];
    return this;
  });
  // %TypedArray%.from / .of: own on %TypedArray% only; the concrete constructors inherit them.
  var TAC = Object.getPrototypeOf(Uint8Array);
  def(TAC, 'from', function from(src) {
    var mapFn = arguments[1], thisArg = arguments[2];
    var items = Array.from(src);
    if (mapFn) items = items.map(function (v, i) { return mapFn.call(thisArg, v, i); });
    return new this(items);
  });
  def(TAC, 'of', function of() { return new this(Array.from(arguments)); });
  // Detached-buffer guard for the methods V8 checks (message names as V8 prints them).
  var detachedNames = { forEach: 'forEach', some: 'some', every: 'every', find: 'find', findIndex: 'findIndex',
    findLast: 'findLast', findLastIndex: 'findIndexLast', reduce: 'reduce', reduceRight: 'reduceRight', fill: 'fill',
    at: 'at', keys: 'keys', values: 'values', entries: 'entries', join: 'join', includes: 'includes',
    indexOf: 'indexOf', lastIndexOf: 'lastIndexOf', map: 'map', filter: 'filter', sort: 'sort',
    toSorted: 'toSorted', toReversed: 'toReversed', reverse: 'reverse', copyWithin: 'copyWithin',
    toString: 'join' };
  Object.keys(detachedNames).forEach(function (n) {
    var orig = TP[n];
    if (typeof orig !== 'function') return;
    def(TP, n, function () {
      if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.' + detachedNames[n] + ' on a detached ArrayBuffer');
      return orig.apply(this, arguments);
    });
  });
  // Spec: %TypedArray%.prototype[Symbol.iterator] is the `values` function (the guarded one now).
  def(TP, Symbol.iterator, TP.values);
})();
"#;
