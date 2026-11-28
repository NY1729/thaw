//! ArrayBuffer and the typed arrays (`Uint8Array` & co); the methods live in a JS prelude on top of
//! the native storage, `Buffer` / `TextEncoder` / `TextDecoder` in `buffer.rs`.

use crate::*;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// Float16Array (ES2025): elements are IEEE half floats, read and written through `f16_to_f64` / `f64_to_f16`.
    F16,
    F32,
    F64,
    // BigInt64Array / BigUint64Array: elements are BigInts, so they have no f64 form (see `TypedArr::get`)
    I64,
    U64,
}

/// IEEE 754 binary16 bits as a number.
pub(crate) fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f64::from(bits & 0x3ff);
    match exp {
        0 => sign * mant * 2f64.powi(-24),
        31 if mant == 0.0 => sign * f64::INFINITY,
        31 => f64::NAN,
        _ => sign * (1.0 + mant / 1024.0) * 2f64.powi(exp - 15),
    }
}

/// A number as IEEE 754 binary16 bits, rounded to nearest, ties to even (the value goes straight from f64, no
/// double rounding through f32).
pub(crate) fn f64_to_f16(v: f64) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let sign: u16 = if v.is_sign_negative() { 0x8000 } else { 0 };
    let abs = v.abs();
    if abs == 0.0 {
        return sign;
    }
    // 65520 is the midpoint between the largest half (65504) and the next power step: it rounds to infinity
    if abs >= 65520.0 {
        return sign | 0x7c00;
    }
    // below the smallest normal (2^-14) the unit is 2^-24; a result of 1024 is the smallest normal's bits
    if abs < 2f64.powi(-14) {
        return sign | (abs * 2f64.powi(24)).round_ties_even() as u16;
    }
    let mut exp = ((abs.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let mut mant = ((abs / 2f64.powi(exp) - 1.0) * 1024.0).round_ties_even() as u32;
    if mant == 1024 {
        mant = 0;
        exp += 1;
    }
    if exp > 15 {
        return sign | 0x7c00;
    }
    sign | (((exp + 15) as u16) << 10) | mant as u16
}

impl Ty {
    pub(crate) fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 | Ty::U8C => 1,
            Ty::I16 | Ty::U16 | Ty::F16 => 2,
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
            Ty::F16 => "[object Float16Array]",
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
            Ty::F16 => "Float16Array",
            Ty::F32 => "Float32Array",
            Ty::F64 => "Float64Array",
            Ty::I64 => "BigInt64Array",
            Ty::U64 => "BigUint64Array",
        }
    }
}

const ALL: [Ty; 12] = [
    Ty::I8,
    Ty::U8,
    Ty::U8C,
    Ty::I16,
    Ty::U16,
    Ty::I32,
    Ty::U32,
    Ty::F16,
    Ty::F32,
    Ty::F64,
    Ty::I64,
    Ty::U64,
];

/// The memory of a SharedArrayBuffer. Every store over it (`sab_store`) and every in-flight wire that names it
/// (worker.rs) holds an `Arc`, so the memory lives while any of them does.
///
/// SAFETY invariants (the unsafe code is `new_sab_block`, `sab_store`, `Store::deref`, `Store::drop` and
/// `SabBlock::drop`):
/// 1. A SAB store's `cell` is a `Vec<u8>` aliasing `[ptr, ptr + len)` with capacity `cap`. That Vec is never
///    dropped, reallocated, resized, shrunk or replaced: `Store::drop` forgets it, `detach_bytes` leaves a shared
///    store alone, and no code calls a length-changing method on a store's Vec (test
///    `grep_guard_no_length_change_on_a_sab_backed_vec`). The one exception is `Store::deref`, which moves a
///    growable block's alias length up to the block's current length with `set_len` (never past `cap`).
/// 2. The allocation is freed only by `SabBlock::drop`, when the last `Arc` goes.
/// 3. `ptr` comes from a `u64` allocation, so it is 8-byte aligned; element offsets that are multiples of the
///    element size are aligned for atomic access.
/// 4. All `cap` bytes are reserved and zeroed when the block is created, and `cap` never changes. The length is
///    fixed unless the block is growable (`max` is Some): `grow` then raises the shared `len` (never lowers it,
///    never past `cap == max`), which every store picks up the next time it is dereferenced.
/// 5. Two threads may touch the bytes at once through their own stores, so racing plain accesses are a data race in
///    Rust's model. ponytail: accepted because JS permits racy SAB access and the hot path has no atomic branch;
///    upgrade path: relaxed atomic bytes in `TypedArr::{get,set}` when `store.sab` is set.
pub(crate) struct SabBlock {
    ptr: *mut u64,
    words: usize,
    /// Reserved bytes (`maxByteLength` of a growable block, else the length).
    cap: usize,
    /// The current byte length, shared by every holder.
    len: AtomicUsize,
    /// `maxByteLength` of a growable SharedArrayBuffer; None when fixed.
    max: Option<usize>,
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

impl SabBlock {
    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
}

/// A zeroed fixed-length block of `len` bytes.
#[cfg(test)]
pub(crate) fn new_sab_block(len: usize) -> Arc<SabBlock> {
    new_sab_block_max(len, None)
}

/// A zeroed block of `len` bytes; growable up to `max` bytes (all reserved now) when given.
pub(crate) fn new_sab_block_max(len: usize, max: Option<usize>) -> Arc<SabBlock> {
    let cap = max.unwrap_or(len);
    let words = cap.div_ceil(8);
    let ptr = Box::into_raw(vec![0u64; words].into_boxed_slice()) as *mut u64;
    Arc::new(SabBlock {
        ptr,
        words,
        cap,
        len: AtomicUsize::new(len),
        max,
    })
}

/// A zeroed SharedArrayBuffer storage of `len` bytes.
pub(crate) fn new_sab_bytes(len: usize) -> Bytes {
    new_sab_bytes_max(len, None)
}

/// `new_sab_bytes`, growable up to `max` bytes when given.
pub(crate) fn new_sab_bytes_max(len: usize, max: Option<usize>) -> Bytes {
    // counted once, on the creating thread: an importing store (`sab_store` alone) charges nothing
    let bytes = sab_store(new_sab_block_max(len, max));
    bytes.charged.set(heap_limit::charge(len));
    bytes
}

/// An attached storage over `block`: the same bytes as every other store of the block.
pub(crate) fn sab_store(block: Arc<SabBlock>) -> Bytes {
    // SAFETY: the Vec aliases the block's allocation (`words * 8 >= cap >= len` bytes, so `len` bytes are live). It
    // is never dropped (invariant 1), which is why its allocation layout (align 8) never has to match a `Vec<u8>`'s.
    let alias = unsafe { Vec::from_raw_parts(block.ptr.cast::<u8>(), block.len(), block.cap) };
    let max = block.max;
    Rc::new(Store {
        cell: RefCell::new(alias),
        shared: None,
        detached: Cell::new(false),
        sab: Some(block),
        charged: Cell::new(0),
        max,
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
            None => {
                // a growable SharedArrayBuffer another store (maybe another thread's) grew: follow its length
                if let Some(block) = self.sab.as_ref().filter(|b| b.max.is_some()) {
                    let len = block.len();
                    // SAFETY: `len <= cap` (grow checks), the bytes below `cap` are reserved and zeroed, and
                    // this thread is the only writer of this store's Vec header (invariant 1).
                    unsafe {
                        let alias = &mut *self.cell.as_ptr();
                        if alias.len() != len {
                            alias.set_len(len);
                        }
                    }
                }
                &self.cell
            }
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

/// `SharedArrayBuffer.prototype.grow`: raises the shared length to `n`; false when `n` is below the current length
/// or above the maximum (or the block is fixed). Atomic, so concurrent growers cannot lower the length.
pub(crate) fn grow_sab(b: &Bytes, n: usize) -> bool {
    let Some(block) = b.sab.as_ref() else {
        return false;
    };
    let Some(max) = block.max else {
        return false;
    };
    if n > max {
        return false;
    }
    let mut cur = block.len.load(Ordering::Acquire);
    loop {
        if n < cur {
            return false;
        }
        match block
            .len
            .compare_exchange(cur, n, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
    b.charged.set(b.charged.get() + heap_limit::charge(n - cur));
    drop(b.borrow());
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
            Ty::F16 => f16_to_f64(u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?)),
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
            let n = it.tonum(v)?;
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
            Ty::F16 => d[p..p + 2].copy_from_slice(&f64_to_f16(v).to_le_bytes()),
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
        let max = self.ab_max_option(&arg(args, 1), n as usize)?;
        let obj = if shared {
            // a growable block reserves its whole maximum now
            if max.is_some_and(|m| m as f64 > 2f64.powi(32)) {
                return self.throw_error("RangeError", "Array buffer allocation failed");
            }
            self.new_shared_buffer(new_sab_bytes_max(n as usize, max))
        } else {
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
                    let n = if n.is_nan() { 0.0 } else { n };
                    // the element buffer is allocated before any element is read: past the ceiling, RangeError
                    if n >= crate::MAX_DENSE_ARRAY_LEN as f64 {
                        return self.throw_error("RangeError", "Array buffer allocation failed");
                    }
                    let mut v = Vec::new();
                    for i in 0..n as usize {
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
        // a SharedArrayBuffer receiver's storage, or V8's TypeError (a plain ArrayBuffer is not one)
        fn sab_receiver(it: &mut Interp, this: &Value, method: &str) -> R<Bytes> {
            let shared = match this {
                Value::Object(o) => match (&*o.proto.borrow(), &it.rt.sharedarraybuffer_proto) {
                    (Some(op), Some(sp)) => Rc::ptr_eq(op, sp),
                    _ => false,
                },
                _ => false,
            };
            match Interp::bytes_of_buffer(this) {
                Some(b) if shared && b.sab.is_some() => Ok(b),
                _ => {
                    let desc = if matches!(this, Value::Object(_)) {
                        "#<Object>"
                    } else {
                        "undefined"
                    };
                    it.throw_error(
                        "TypeError",
                        &format!("Method {method} called on incompatible receiver {desc}"),
                    )
                }
            }
        }
        let growable_get = self.native(|it, this, _| {
            let data = sab_receiver(it, this, "get SharedArrayBuffer.prototype.growable")?;
            Ok(Value::Bool(data.max.is_some()))
        });
        define_accessor(&sab_proto, "growable", Some(growable_get), None, false);
        let sab_max_get = self.native(|it, this, _| {
            let data = sab_receiver(it, this, "get SharedArrayBuffer.prototype.maxByteLength")?;
            let n = data.max.unwrap_or_else(|| data.borrow().len());
            Ok(Value::Number(n as f64))
        });
        define_accessor(&sab_proto, "maxByteLength", Some(sab_max_get), None, false);
        // spec order: receiver (growable), ToIndex, then the range against the current length and the maximum
        self.set_native(&sab_proto, "grow", |it, this, a| {
            let data = sab_receiver(it, this, "SharedArrayBuffer.prototype.grow")?;
            if data.max.is_none() {
                return it.throw_error(
                    "TypeError",
                    "Method SharedArrayBuffer.prototype.grow called on incompatible receiver #<SharedArrayBuffer>",
                );
            }
            let n = it.num(&arg(a, 0));
            let n = if n.is_nan() { 0.0 } else { n.trunc() };
            if !(0.0..=9007199254740991.0).contains(&n) || !grow_sab(&data, n as usize) {
                return it.throw_error(
                    "RangeError",
                    "SharedArrayBuffer.prototype.grow: Invalid length parameter",
                );
            }
            it.check_interrupt()?;
            Ok(Value::Undefined)
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
            let off = match arg(a, 1) {
                Value::Undefined => 0,
                v => {
                    // ToIntegerOrInfinity: a negative or infinite offset is a RangeError, not a clamp to 0
                    let n = it.tonum(&v)?;
                    let n = if n.is_nan() { 0.0 } else { n.trunc() };
                    if n < 0.0 || n.is_infinite() {
                        return it.throw_error("RangeError", "offset is out of bounds");
                    }
                    n as usize
                }
            };
            // the offset's conversion may have detached the target (or the source): that is a TypeError
            if t.gone() {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer",
                );
            }
            // spec: a typed array source is copied after its checks; an array-like's length is checked before any element
            // is read, then each element is read and written in turn (a getter sees the earlier writes)
            let src: Vec<Value> = if let Some(s) = Self::typed_of(&arg(a, 0)) {
                if s.gone() {
                    return it.throw_error(
                        "TypeError",
                        "Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer",
                    );
                }
                it.check_mix(s.ty, t.ty)?;
                // same element type: the bytes copy as they are (no f64 round trip per element)
                if s.ty == t.ty {
                    let n = s.cur_len();
                    if off + n > t.cur_len() {
                        return it.throw_error("RangeError", "offset is out of bounds");
                    }
                    let bytes = s.window();
                    let at = t.off + off * t.ty.size();
                    t.data.borrow_mut()[at..at + bytes.len()].copy_from_slice(&bytes);
                    return Ok(Value::Undefined);
                }
                let v = s.to_values();
                if off + v.len() > t.cur_len() {
                    return it.throw_error("RangeError", "offset is out of bounds");
                }
                v
            } else {
                // a plain dense array is read in one go; everything else is an array-like read element by element
                let items = match &arg(a, 0) {
                    v @ Value::Object(o)
                        if matches!(&*o.kind.borrow(), Kind::Array(_)) && !o.is_exotic() =>
                    {
                        it.iter_items(v)
                    }
                    _ => None,
                };
                match items {
                    Some(i) => {
                        if off + i.len() > t.cur_len() {
                            return it.throw_error("RangeError", "offset is out of bounds");
                        }
                        i
                    }
                    None => {
                        let n = it.get(&arg(a, 0), "length")?;
                        let n = it.tonum(&n)?;
                        let n = if n.is_nan() {
                            0
                        } else {
                            n.trunc().max(0.0) as usize
                        };
                        if off + n > t.cur_len() {
                            return it.throw_error("RangeError", "offset is out of bounds");
                        }
                        for i in 0..n {
                            let v = it.get(&arg(a, 0), &i.to_string())?;
                            t.set_value(it, off + i, &v)?;
                        }
                        return Ok(Value::Undefined);
                    }
                }
            };
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
            let rel = |it: &mut Interp, v: Value, d: f64| -> R<f64> {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    let x = it.tonum(&v)?;
                    if x.is_nan() {
                        0.0
                    } else {
                        x.trunc()
                    }
                };
                Ok(if x < 0.0 { (n + x).max(0.0) } else { x.min(n) })
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0)?, rel(it, arg(a, 1), n)?);
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
            let rel = |it: &mut Interp, v: Value, d: f64| -> R<f64> {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    let x = it.tonum(&v)?;
                    if x.is_nan() {
                        0.0
                    } else {
                        x.trunc()
                    }
                };
                Ok(if x < 0.0 { (n + x).max(0.0) } else { x.min(n) })
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0)?, rel(it, arg(a, 1), n)?);
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
        // Same kind as the receiver (TypedArrayCreateSameType: the intrinsic prototype, so a Buffer
        // gives a Uint8Array). The value converts before the index is range-checked; `set_value` at
        // an out-of-range index is that conversion alone.
        self.set_native(&ta_proto, "with", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            if t.gone() {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform %TypedArray%.prototype.with on a detached ArrayBuffer",
                );
            }
            // the relative index, then the value's conversion (which may detach or shrink), then the bounds
            let rel = match it.tonum(&arg(a, 0))?.trunc() {
                x if x.is_nan() => 0.0,
                x => x,
            };
            let scratch = it.typed_with_len(t.ty, 1);
            let sc = Self::typed_of(&scratch).unwrap();
            sc.set_value(it, 0, &arg(a, 1))?;
            let value = sc.get_value(0).unwrap_or(Value::Undefined);
            if t.gone() {
                return it.throw_error(
                    "TypeError",
                    "Cannot perform %TypedArray%.prototype.with on a detached ArrayBuffer",
                );
            }
            let n = t.cur_len();
            let idx = if rel < 0.0 { n as f64 + rel } else { rel };
            let in_range = idx >= 0.0 && idx < n as f64;
            let at = if in_range { idx as usize } else { n };
            let out = it.typed_with_len(t.ty, n);
            let o = Self::typed_of(&out).unwrap();
            let size = t.ty.size();
            let bytes = n * size;
            o.data.borrow_mut()[..bytes].copy_from_slice(&t.data.borrow()[t.off..t.off + bytes]);
            o.set_value(it, at, &value)?;
            if !in_range {
                return it.throw_error("RangeError", "Invalid typed array index");
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
  Object.defineProperty(SharedArrayBuffer.prototype, TAG, { value: 'SharedArrayBuffer', configurable: true });
  ['at', 'toString', 'keys', 'values', 'entries'].forEach(function (n) {
    if (AP[n]) def(TP, n, AP[n]);
  });
  // Spec: %TypedArray%.prototype[Symbol.iterator] is the `values` function. Compiled `Array.from(view)` and `for..of` use the protocol.
  def(TP, Symbol.iterator, TP.values);
  var rApply = Reflect.apply, rConstruct = Reflect.construct;
  var SPECIES = Symbol.species, ITER = Symbol.iterator;
  function toInt(v) { var x = +v; return x !== x ? 0 : (x === Infinity || x === -Infinity ? x : Math.trunc(x) + 0); }
  function relIndex(v, len, d) {
    if (v === undefined) return d;
    var x = toInt(v);
    return x < 0 ? Math.max(len + x, 0) : Math.min(x, len);
  }
  function needFn(f) { if (typeof f !== 'function') throw new TypeError('callback is not a function'); }
  def(TP, 'toLocaleString', function () {
    var len = this.length, out = '';
    for (var i = 0; i < len; i++) {
      if (i) out += ',';
      var v = this[i];
      if (v !== undefined && v !== null) out += `${v.toLocaleString()}`;
    }
    return out;
  });
  def(TP, 'indexOf', function (v, from) {
    var len = this.length;
    if (len === 0) return -1;
    var n = toInt(from);
    if (n === Infinity) return -1;
    if (n === -Infinity) n = 0;
    for (var k = n >= 0 ? n : Math.max(len + n, 0); k < len; k++) if (k in this && this[k] === v) return k;
    return -1;
  });
  def(TP, 'lastIndexOf', function (v, from) {
    var len = this.length;
    if (len === 0) return -1;
    var n = arguments.length > 1 ? toInt(from) : len - 1;
    if (n === -Infinity) return -1;
    for (var k = n >= 0 ? Math.min(n, len - 1) : len + n; k >= 0; k--) if (k in this && this[k] === v) return k;
    return -1;
  });
  def(TP, 'join', function (sep) {
    var len = this.length;
    var s = sep === undefined ? ',' : `${sep}`;
    var out = '';
    for (var k = 0; k < len; k++) {
      if (k > 0) out += s;
      var e = this[k];
      if (e !== undefined) out += e;
    }
    return out;
  });
  def(TP, 'includes', function (v, from) {
    var len = this.length;
    if (len === 0) return false;
    var n = toInt(from);
    if (n === Infinity) return false;
    if (n === -Infinity) n = 0;
    for (var k = n >= 0 ? n : Math.max(len + n, 0); k < len; k++) {
      var e = this[k];
      if (e === v || (e !== e && v !== v)) return true;
    }
    return false;
  });
  def(TP, 'reverse', function () {
    for (var i = 0, j = this.length - 1; i < j; i++, j--) { var t = this[i]; this[i] = this[j]; this[j] = t; }
    return this;
  });
  def(TP, 'forEach', function (f, t) {
    var len = this.length; needFn(f);
    for (var k = 0; k < len; k++) rApply(f, t, [this[k], k, this]);
  });
  def(TP, 'some', function (f, t) {
    var len = this.length; needFn(f);
    for (var k = 0; k < len; k++) if (rApply(f, t, [this[k], k, this])) return true;
    return false;
  });
  def(TP, 'every', function (f, t) {
    var len = this.length; needFn(f);
    for (var k = 0; k < len; k++) if (!rApply(f, t, [this[k], k, this])) return false;
    return true;
  });
  [['find', false, false], ['findIndex', false, true], ['findLast', true, false], ['findLastIndex', true, true]].forEach(function (d) {
    var back = d[1], wantIndex = d[2];
    def(TP, d[0], { [d[0]]: function (f, t) {
      var len = this.length; needFn(f);
      for (var i = 0; i < len; i++) {
        var k = back ? len - 1 - i : i, v = this[k];
        if (rApply(f, t, [v, k, this])) return wantIndex ? k : v;
      }
      return wantIndex ? -1 : undefined;
    } }[d[0]]);
  });
  [['reduce', false], ['reduceRight', true]].forEach(function (d) {
    var back = d[1];
    def(TP, d[0], { [d[0]]: function (f, init) {
      var len = this.length; needFn(f);
      var i = 0, acc;
      if (arguments.length >= 2) acc = init;
      else {
        if (len === 0) throw new TypeError('Reduce of empty array with no initial value');
        acc = this[back ? len - 1 : 0]; i = 1;
      }
      for (; i < len; i++) { var k = back ? len - 1 - i : i; acc = rApply(f, undefined, [acc, this[k], k, this]); }
      return acc;
    } }[d[0]]);
  });
  // TypedArraySpeciesCreate: the default constructor of each kind is captured here, before user code runs
  var DEFAULTS = {}, SCRATCH = {};
  ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array', 'Int32Array', 'Uint32Array', 'Float16Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array'].forEach(function (n) {
    if (typeof globalThis[n] === 'function') { DEFAULTS[n] = globalThis[n]; SCRATCH[n] = new DEFAULTS[n](1); }
  });
  // ToNumber / ToBigInt of `v` as `o`'s element type, done once (the engine's own element conversion)
  function coerce(o, v) { var s = SCRATCH[o[TAG]]; s[0] = v; return s[0]; }
  var isBig = function (o) { return /^Big/.test(o[TAG]); };
  function speciesCtor(o) {
    var dflt = DEFAULTS[o[TAG]], C = o.constructor;
    if (C === undefined) return dflt;
    if ((typeof C !== 'object' && typeof C !== 'function') || C === null) throw new TypeError('object.constructor is not an object');
    var S = C[SPECIES];
    if (S === undefined || S === null) return dflt;
    if (typeof S !== 'function') throw new TypeError('object.constructor[Symbol.species] is not a constructor');
    return S;
  }
  // TypedArrayCreateFromConstructor: construct, validate, and check the length of a single-length request
  function createFrom(C, args) {
    var r = rConstruct(C, args);
    if (!isTA(r) || gone(r)) throw new TypeError('constructor did not return a TypedArray');
    if (args.length === 1 && typeof args[0] === 'number' && r.length < args[0]) throw new TypeError('constructor returned a too short TypedArray');
    return r;
  }
  function speciesCreate(o, args, count) {
    var r = createFrom(speciesCtor(o), args);
    if (isBig(r) !== isBig(o)) throw new TypeError('Content type mismatch');
    if (count !== undefined && r.length < count) throw new TypeError('species constructor returned a too short TypedArray');
    return r;
  }
  def(TP, 'fill', function (value, start, end) {
    var len = this.length;
    value = coerce(this, value);
    var k = relIndex(start, len, 0), fin = relIndex(end, len, len);
    if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.fill on a detached ArrayBuffer');
    fin = Math.min(fin, this.length);
    for (; k < fin; k++) this[k] = value;
    return this;
  });
  def(TP, 'map', function (f, t) {
    var len = this.length;
    needFn(f);
    var r = speciesCreate(this, [len], len);
    for (var i = 0; i < len; i++) r[i] = rApply(f, t, [this[i], i, this]);
    return r;
  });
  def(TP, 'filter', function (f, t) {
    var kept = [], len = this.length;
    needFn(f);
    for (var i = 0; i < len; i++) { var v = this[i]; if (rApply(f, t, [v, i, this])) kept.push(v); }
    var r = speciesCreate(this, [kept.length], kept.length);
    for (var j = 0; j < kept.length; j++) r[j] = kept[j];
    return r;
  });
  function sortValues(vals, cmp) {
    if (cmp !== undefined) return vals.sort(function (x, y) { var v = +cmp(x, y); return v !== v ? 0 : v; });
    return vals.sort(function (x, y) {
      if (x < y) return -1;
      if (x > y) return 1;
      if (x === 0 && y === 0) { var nx = Object.is(x, -0), ny = Object.is(y, -0); return nx && !ny ? -1 : !nx && ny ? 1 : 0; }
      if (x !== x) return y !== y ? 0 : 1;
      if (y !== y) return -1;
      return 0;
    });
  }
  function vals(o, len) { var a = []; for (var i = 0; i < len; i++) a.push(o[i]); return a; }
  function validate(o, name) {
    if (!isTA(o)) throw new TypeError('this is not a typed array.');
    if (gone(o)) throw new TypeError('Cannot perform %TypedArray%.prototype.' + name + ' on a detached ArrayBuffer');
  }
  def(TP, 'sort', function (cmp) {
    if (cmp !== undefined && typeof cmp !== 'function') throw new TypeError('The comparison function must be either a function or undefined');
    validate(this, 'sort');
    var len = this.length, a = sortValues(vals(this, len), cmp);
    for (var i = 0; i < len; i++) this[i] = a[i];
    return this;
  });
  def(TP, 'toSorted', function (cmp) {
    if (cmp !== undefined && typeof cmp !== 'function') throw new TypeError('The comparison function must be either a function or undefined');
    validate(this, 'toSorted');
    var len = this.length, r = new DEFAULTS[this[TAG]](len), a = sortValues(vals(this, len), cmp);
    for (var i = 0; i < len; i++) r[i] = a[i];
    return r;
  });
  def(TP, 'toReversed', function () { return nSlice.call(this, 0, this.length).reverse(); });
  def(TP, 'copyWithin', function (target, start, end) {
    var len = this.length;
    var to = relIndex(target, len, 0), from = relIndex(start, len, 0), fin = relIndex(end, len, len);
    var count = Math.min(fin - from, len - to);
    if (count > 0) {
      if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.copyWithin on a detached ArrayBuffer');
      var now = this.length;
      if (from < to && to < from + count) for (var i = count - 1; i >= 0; i--) { if (from + i < now && to + i < now) this[to + i] = this[from + i]; }
      else for (var j = 0; j < count; j++) { if (from + j < now && to + j < now) this[to + j] = this[from + j]; }
    }
    return this;
  });
  // %TypedArray%.from / .of: own on %TypedArray% only; the concrete constructors inherit them.
  var TAC = Object.getPrototypeOf(Uint8Array);
  function isCtor(C) { try { rConstruct(function () {}, [], C); return true; } catch (e) { return false; } }
  def(TAC, 'from', function from(src) {
    var C = this, mapFn = arguments[1], thisArg = arguments[2];
    if (!isCtor(C)) throw new TypeError(String(C) + ' is not a constructor');
    if (mapFn !== undefined && typeof mapFn !== 'function') throw new TypeError(String(mapFn) + ' is not a function');
    var using = src === undefined || src === null ? src[ITER] : Object(src)[ITER];
    if (using !== undefined && using !== null && typeof using !== 'function') throw new TypeError('source[Symbol.iterator] is not a function');
    var items;
    if (using !== undefined && using !== null) items = Array.from({ [ITER]: function () { return rApply(using, src, []); } });
    else { var o = Object(src), n = Math.min(Math.max(toInt(o.length), 0), 9007199254740991); items = []; for (var i = 0; i < n; i++) items.push(o[i]); }
    var len = items.length, r = createFrom(C, [len]);
    for (var k = 0; k < len; k++) {
      var v = items[k];
      if (mapFn !== undefined) v = rApply(mapFn, thisArg, [v, k]);
      if (!Reflect.set(r, k, v)) throw new TypeError('Cannot set element ' + k);
    }
    return r;
  });
  def(TAC, 'of', function of() {
    var C = this, len = arguments.length;
    if (!isCtor(C)) throw new TypeError(String(C) + ' is not a constructor');
    var r = createFrom(C, [len]);
    for (var k = 0; k < len; k++) if (!Reflect.set(r, k, arguments[k])) throw new TypeError('Cannot set element ' + k);
    return r;
  });
  [TAC, ArrayBuffer].forEach(function (C) {
    if (!Object.prototype.hasOwnProperty.call(C, SPECIES))
      Object.defineProperty(C, SPECIES, { get: Object.getOwnPropertyDescriptor({ get [SPECIES]() { return this; } }, SPECIES).get, configurable: true });
  });
  // Detached-buffer guard for the methods V8 checks (message names as V8 prints them).
  var detachedNames = { forEach: 'forEach', some: 'some', every: 'every', find: 'find', findIndex: 'findIndex',
    findLast: 'findLast', findLastIndex: 'findIndexLast', reduce: 'reduce', reduceRight: 'reduceRight', fill: 'fill',
    at: 'at', keys: 'keys', values: 'values', entries: 'entries', join: 'join', includes: 'includes',
    indexOf: 'indexOf', lastIndexOf: 'lastIndexOf', map: 'map', filter: 'filter',
    toReversed: 'toReversed', reverse: 'reverse', copyWithin: 'copyWithin',
    toString: 'join', toLocaleString: 'toLocaleString' };
  Object.keys(detachedNames).forEach(function (n) {
    var orig = TP[n];
    if (typeof orig !== 'function') return;
    def(TP, n, { [n]: function () {
      if (n !== 'toString' && !isTA(this)) throw new TypeError('this is not a typed array.');
      if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.' + detachedNames[n] + ' on a detached ArrayBuffer');
      return orig.apply(this, arguments);
    } }[n]);
  });
  // slice / subarray: a user species constructor builds the result; the default (and Buffer) keep the native path
  function relIdx(v, len, d) {
    if (v === undefined) return d;
    var x = +v;
    x = x !== x ? 0 : Math.trunc(x);
    return x < 0 ? Math.max(len + x, 0) : Math.min(x, len);
  }
  function plainCtor(C, o) { return C === DEFAULTS[o[TAG]] || (typeof Buffer === 'function' && C === Buffer); }
  var nSlice = TP.slice, nSub = TP.subarray;
  def(TP, 'slice', function (start, end) {
    if (!isTA(this)) throw new TypeError('this is not a typed array.');
    if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.slice on a detached ArrayBuffer');
    var len = this.length, s = relIdx(start, len, 0), e = relIdx(end, len, len), C = speciesCtor(this);
    if (plainCtor(C, this)) return nSlice.call(this, s, e);
    var cnt = Math.max(e - s, 0), r = speciesCreate(this, [cnt], cnt);
    if (cnt > 0) {
      if (gone(this)) throw new TypeError('Cannot perform %TypedArray%.prototype.slice on a detached ArrayBuffer');
      e = Math.min(e, this.length);
      for (var k = s, n = 0; k < e; k++, n++) r[n] = this[k];
    }
    return r;
  });
  def(TP, 'subarray', function (begin, end) {
    if (!isTA(this)) throw new TypeError('this is not a typed array.');
    var len = this.length, off = this.byteOffset, buf = this.buffer, tracking = __thaw_ta_track(this);
    var s = relIdx(begin, len, 0), e = relIdx(end, len, len), C = speciesCtor(this);
    if (plainCtor(C, this)) return nSub.call(this, s, end === undefined ? undefined : e);
    var args = [buf, off + s * this.BYTES_PER_ELEMENT];
    if (!(end === undefined && tracking)) args.push(Math.max(e - s, 0));
    return speciesCreate(this, args);
  });
  // ArrayBuffer.prototype.slice: relative indices, then SpeciesConstructor and the checks on what it builds
  var abSlice = ArrayBuffer.prototype.slice, AB = ArrayBuffer;
  def(ArrayBuffer.prototype, 'slice', function slice(start, end) {
    var isDet;
    try { isDet = detGet.call(this); } catch (e) { return rApply(abSlice, this, [start, end]); } // the native names the bad receiver
    if (isDet) throw new TypeError('Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer');
    var len = this.byteLength, first = relIdx(start, len, 0), fin = relIdx(end, len, len), n = Math.max(fin - first, 0);
    var C = this.constructor, S = AB;
    if (C !== undefined) {
      if ((typeof C !== 'object' && typeof C !== 'function') || C === null) throw new TypeError('object.constructor is not an object');
      var sp = C[SPECIES];
      if (sp !== undefined && sp !== null) { if (!isCtor(sp)) throw new TypeError('object.constructor[Symbol.species] is not a constructor'); S = sp; }
    }
    if (S === AB) return rApply(abSlice, this, [first, fin]);
    var r = rConstruct(S, [n]);
    if (detGet.call(r)) throw new TypeError('species constructor returned a detached ArrayBuffer');
    if (r === this) throw new TypeError('species constructor returned the same ArrayBuffer');
    if (r.byteLength < n) throw new TypeError('species constructor returned a too small ArrayBuffer');
    if (detGet.call(this)) throw new TypeError('Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer');
    var avail = Math.max(Math.min(n, this.byteLength - first), 0);
    if (avail > 0) new Uint8Array(r).set(new Uint8Array(this, first, avail));
    return r;
  });
  // Spec: %TypedArray%.prototype[Symbol.iterator] is the `values` function (the guarded one now).
  def(TP, Symbol.iterator, TP.values);
})();
"#;
