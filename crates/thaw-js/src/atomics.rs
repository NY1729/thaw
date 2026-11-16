//! Atomics on SharedArrayBuffer memory (plan A3). The JS prelude (extras.rs) sends a SharedArrayBuffer here: `__thaw_atomic`
//! runs one read-modify-write as a real atomic op on the shared block, and `__thaw_atomics_wait` / `__thaw_atomics_notify`
//! park and wake threads by absolute address. A non-shared buffer never reaches these natives.

use crate::typed::{Ty, TypedArr};
use crate::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicI32, AtomicU16, AtomicU32, AtomicU8, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How often a parked thread checks its interrupt flag (so `worker.terminate()` ends an infinite wait).
const INTERRUPT_POLL: Duration = Duration::from_millis(10);

/// A thread parked in `Atomics.wait`. `woken` is set by the notify that takes it off the queue (under `PARK`).
struct Waiter {
    woken: Mutex<bool>,
    cv: Condvar,
}

/// Parked waiters by absolute element address, FIFO per address. A wait checks its value and queues itself under this
/// lock; a notify pops and wakes under it. So a store that a notify follows cannot slip between a waiter's check and
/// its park: either the waiter sees the new value or it is queued when the notify runs.
static PARK: Mutex<BTreeMap<usize, VecDeque<Arc<Waiter>>>> = Mutex::new(BTreeMap::new());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// ToInt32-style wrap of a number to 32 bits; the narrower element types keep the low bits of this.
fn wrap_bits(v: f64) -> u32 {
    if v.is_finite() {
        v.trunc().rem_euclid(4_294_967_296.0) as u32
    } else {
        0
    }
}

/// The number a script reads from an element of type `ty` whose bits are `bits` (sign-extended for signed types).
fn elem_value(ty: Ty, bits: u32) -> f64 {
    match ty {
        Ty::I8 => bits as u8 as i8 as f64,
        Ty::I16 => bits as u16 as i16 as f64,
        Ty::I32 => bits as i32 as f64,
        Ty::U16 => bits as u16 as f64,
        Ty::U32 => bits as f64,
        _ => bits as u8 as f64,
    }
}

/// The absolute address of element `k` of an integer typed array, or a RangeError when it is not an element of a
/// live view (the caller has already checked the Atomics index).
fn element_addr(it: &mut Interp, ta: &TypedArr, k: usize) -> R<usize> {
    if !ta.live() || k >= ta.len {
        return it.throw_error("RangeError", "Invalid atomic access index");
    }
    let base = ta.data.borrow().as_ptr() as usize;
    let addr = base + ta.off + k * ta.ty.size();
    // element widths are powers of two
    if addr & (ta.ty.size() - 1) != 0 {
        return it.throw_error("RangeError", "Atomics element is not aligned");
    }
    Ok(addr)
}

fn integer_view(it: &mut Interp, v: &Value) -> R<TypedArr> {
    match Interp::typed_of(v) {
        Some(ta)
            if matches!(
                ta.ty,
                Ty::I8 | Ty::U8 | Ty::I16 | Ty::U16 | Ty::I32 | Ty::U32
            ) =>
        {
            Ok(ta)
        }
        _ => it.throw_error("TypeError", "Atomics requires an integer typed array"),
    }
}

/// One op on the element at `addr`, with the element's own width. Returns the old bits (the stored bits for `store`);
/// `None` for an unknown op. Each arm is one std atomic instruction, so no other thread can interleave inside it.
macro_rules! atomic_op {
    ($name:ident, $atomic:ty, $int:ty) => {
        fn $name(addr: usize, op: &str, v1: $int, v2: $int) -> Option<$int> {
            // SAFETY: `addr` is an element inside a live view's bytes (`element_addr`), which the view's store keeps
            // alive for this call. The address is aligned for the element width (checked there), and the bytes are
            // only ever touched through atomics or the plain paths of a non-shared buffer.
            let a = unsafe { <$atomic>::from_ptr(addr as *mut $int) };
            Some(match op {
                "add" => a.fetch_add(v1, SeqCst),
                "sub" => a.fetch_sub(v1, SeqCst),
                "and" => a.fetch_and(v1, SeqCst),
                "or" => a.fetch_or(v1, SeqCst),
                "xor" => a.fetch_xor(v1, SeqCst),
                "exchange" => a.swap(v1, SeqCst),
                "compareExchange" => match a.compare_exchange(v1, v2, SeqCst, SeqCst) {
                    Ok(old) | Err(old) => old,
                },
                "load" => a.load(SeqCst),
                "store" => {
                    a.store(v1, SeqCst);
                    v1
                }
                _ => return None,
            })
        }
    };
}
atomic_op!(op8, AtomicU8, u8);
atomic_op!(op16, AtomicU16, u16);
atomic_op!(op32, AtomicU32, u32);

/// `__thaw_atomic(op, ta, index, v1, v2)`: the element's old value (the stored value for `store`), operands wrapped to
/// the element's width. The JS prelude has checked the op name's caller, the index and the integer kind.
pub(crate) fn atomic(it: &mut Interp, a: &[Value]) -> R<Value> {
    let op = it.to_str(&arg(a, 0));
    let ta = integer_view(it, &arg(a, 1))?;
    let k = it.num(&arg(a, 2)) as usize;
    let (w1, w2) = (wrap_bits(it.num(&arg(a, 3))), wrap_bits(it.num(&arg(a, 4))));
    let addr = element_addr(it, &ta, k)?;
    let old = match ta.ty.size() {
        1 => op8(addr, &op, w1 as u8, w2 as u8).map(u32::from),
        2 => op16(addr, &op, w1 as u16, w2 as u16).map(u32::from),
        _ => op32(addr, &op, w1, w2),
    };
    match old {
        Some(bits) => Ok(Value::Number(elem_value(ta.ty, bits))),
        None => it.throw_error("TypeError", "unknown Atomics operation"),
    }
}

/// Takes `waiter` off `addr`'s queue. True when it was still queued (its timeout or interrupt won); false when a notify
/// already took it, which sets `woken` under `PARK` before the lock is released.
fn unpark(addr: usize, waiter: &Arc<Waiter>) -> bool {
    let mut park = lock(&PARK);
    let Some(queue) = park.get_mut(&addr) else {
        return false;
    };
    let before = queue.len();
    queue.retain(|w| !Arc::ptr_eq(w, waiter));
    let removed = queue.len() != before;
    if queue.is_empty() {
        park.remove(&addr);
    }
    removed
}

/// `__thaw_atomics_wait(ta, index, value, timeoutMs)` on a shared Int32Array: 'not-equal' when the element is not
/// `value` (no park), 'ok' when a notify wakes it, 'timed-out' after `timeoutMs` (Infinity waits without a deadline).
/// An interrupt (worker terminate) ends the wait as an exit, as it ends any other loop.
pub(crate) fn wait(it: &mut Interp, a: &[Value]) -> R<Value> {
    let ta = integer_view(it, &arg(a, 0))?;
    if ta.ty != Ty::I32 {
        return it.throw_error("TypeError", "Atomics.wait requires a shared Int32Array");
    }
    let k = it.num(&arg(a, 1)) as usize;
    let value = wrap_bits(it.num(&arg(a, 2))) as i32;
    let ms = it.num(&arg(a, 3));
    let addr = element_addr(it, &ta, k)?;
    let deadline = if ms.is_finite() {
        Duration::try_from_secs_f64(ms.max(0.0) / 1000.0)
            .ok()
            .and_then(|d| Instant::now().checked_add(d))
    } else {
        None
    };
    let waiter = Arc::new(Waiter {
        woken: Mutex::new(false),
        cv: Condvar::new(),
    });
    {
        let mut park = lock(&PARK);
        // SAFETY: an aligned i32 element inside the live view (`element_addr`); the load is atomic.
        let now = unsafe { AtomicI32::from_ptr(addr as *mut i32) }.load(SeqCst);
        if now != value {
            return Ok(Value::str("not-equal"));
        }
        park.entry(addr).or_default().push_back(waiter.clone());
    }
    let mut woken = lock(&waiter.woken);
    loop {
        if *woken {
            return Ok(Value::str("ok"));
        }
        if let Err(e) = it.check_interrupt() {
            drop(woken);
            unpark(addr, &waiter);
            return Err(e);
        }
        let slice = match deadline {
            Some(d) => {
                let now = Instant::now();
                if now >= d {
                    drop(woken);
                    return Ok(Value::str(if unpark(addr, &waiter) {
                        "timed-out"
                    } else {
                        "ok"
                    }));
                }
                (d - now).min(INTERRUPT_POLL)
            }
            None => INTERRUPT_POLL,
        };
        woken = waiter
            .cv
            .wait_timeout(woken, slice)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// `__thaw_atomics_notify(ta, index, count)` on a shared Int32Array: wakes up to `count` threads parked on the element,
/// in arrival order, and returns how many it woke.
pub(crate) fn notify(it: &mut Interp, a: &[Value]) -> R<Value> {
    let ta = integer_view(it, &arg(a, 0))?;
    let k = it.num(&arg(a, 1)) as usize;
    let count = it.num(&arg(a, 2)).max(0.0);
    let addr = element_addr(it, &ta, k)?;
    let mut park = lock(&PARK);
    let Some(queue) = park.get_mut(&addr) else {
        return Ok(Value::Number(0.0));
    };
    let n = (count.min(queue.len() as f64)) as usize;
    let taken: Vec<Arc<Waiter>> = queue.drain(..n).collect();
    if queue.is_empty() {
        park.remove(&addr);
    }
    for w in &taken {
        *lock(&w.woken) = true;
        w.cv.notify_one();
    }
    Ok(Value::Number(n as f64))
}
