//! Stack-overflow guard for compiled code that runs on a thread other than the main one.
//!
//! Compiled functions compare their stack address with one process-wide limit (`__thaw_stack_limit`,
//! set at entry from the main thread's stack). A coroutine thread's stack lies in a different
//! address range, so every compiled call made there would look like an overflow. The host calls
//! [`with_thread_stack_guard`] around each compiled call: when the current stack is below the
//! registered limit, the limit is swapped to this thread's own bottom for the call and restored after.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    /// Bounds `(lo, hi)` of the stack this thread runs on when that is not the thread's own stack (a
    /// stackful coroutine, see thaw-js `costack`). `None` = use `pthread_getattr_np` as usual.
    static BOUNDS_OVERRIDE: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

/// Makes `bounds` (lowest and highest usable address) the stack this thread reports to
/// `thread_stack_limit` while a foreign stack runs on it; `None` restores the thread's own stack.
/// Returns the previous setting so the caller can restore it.
pub fn set_stack_bounds_override(bounds: Option<(usize, usize)>) -> Option<(usize, usize)> {
    BOUNDS_OVERRIDE.with(|c| c.replace(bounds))
}

/// Address of the compiled module's `__thaw_stack_limit` slot (0 = none registered).
static SLOT: AtomicUsize = AtomicUsize::new(0);

/// Safety margin kept above a thread's stack bottom, matching the main-thread limit.
const MARGIN: u64 = 1 << 20;

/// Called by compiled entry code with the address of its `__thaw_stack_limit` slot.
#[no_mangle]
pub extern "C" fn thaw_stack_guard_register(slot: *mut u64) {
    SLOT.store(slot as usize, Ordering::Release);
}

/// Runs `f` with the registered stack limit valid for the current thread.
pub fn with_thread_stack_guard<T>(f: impl FnOnce() -> T) -> T {
    with_guard_slot(SLOT.load(Ordering::Acquire) as *mut u64, f)
}

fn with_guard_slot<T>(slot: *mut u64, f: impl FnOnce() -> T) -> T {
    if slot.is_null() {
        return f();
    }
    // SAFETY: the registered slot is the compiled module's static limit, live for the process.
    let current = unsafe { slot.read_volatile() };
    if stack_address() >= current {
        return f();
    }
    let limit = thread_stack_limit();
    struct Restore(*mut u64, u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: as above; restores the value read before the swap.
            unsafe { self.0.write_volatile(self.1) };
        }
    }
    // SAFETY: as above.
    unsafe { slot.write_volatile(limit) };
    let _restore = Restore(slot, current);
    f()
}

#[inline(never)]
fn stack_address() -> u64 {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as u64
}

/// Lowest address this thread's compiled code may use (0 = no guard), by the same rule as the
/// main thread: its stack bottom plus a margin of one eighth of the stack or 1 MiB, whichever is larger.
/// A stack override (see [`set_stack_bounds_override`]) takes precedence over the thread's own stack.
fn thread_stack_limit() -> u64 {
    match BOUNDS_OVERRIDE.with(|c| c.get()) {
        Some((lo, hi)) => limit_for_stack(lo as u64, (hi - lo) as u64),
        None => own_thread_stack_limit(),
    }
}

/// The limit for a stack of `size` bytes starting at `bottom`; 0 when it is not below the current sp.
fn limit_for_stack(bottom: u64, size: u64) -> u64 {
    let limit = bottom + MARGIN.max(size / 8);
    if limit < stack_address() {
        limit
    } else {
        0
    }
}

#[cfg(target_os = "linux")]
fn own_thread_stack_limit() -> u64 {
    // SAFETY: plain pthread queries on the current thread with a zeroed attribute object.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return 0;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        if !ok || size == 0 {
            return 0;
        }
        limit_for_stack(addr as u64, size as u64)
    }
}

#[cfg(not(target_os = "linux"))]
fn own_thread_stack_limit() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foreign_thread_gets_its_own_limit_and_the_slot_is_restored() {
        let mut slot: u64 = u64::MAX;
        let address = &mut slot as *mut u64 as usize;
        let seen = std::thread::spawn(move || {
            let slot = address as *mut u64;
            with_guard_slot(slot, || {
                // SAFETY: the owning test frame is blocked in `join` while this runs.
                let inside = unsafe { slot.read_volatile() };
                assert!(
                    inside != u64::MAX && inside != 0,
                    "limit not swapped: {inside:#x}"
                );
                assert!(
                    stack_address() >= inside,
                    "this thread's sp is below its own limit"
                );
                inside
            })
        })
        .join()
        .unwrap();
        assert_ne!(seen, u64::MAX);
        assert_eq!(slot, u64::MAX, "the slot is restored after the call");
    }

    #[test]
    fn an_override_is_the_stack_the_limit_is_taken_from() {
        let sp = stack_address();
        let size = 64u64 << 20;
        let bottom = sp - (16 << 20);
        let prev = set_stack_bounds_override(Some((bottom as usize, (bottom + size) as usize)));
        assert_eq!(prev, None);
        // lo + max(1 MiB, size / 8): 8 MiB above the bottom for a 64 MiB stack
        assert_eq!(thread_stack_limit(), bottom + (8 << 20));
        assert!(set_stack_bounds_override(None).is_some());
        assert_ne!(thread_stack_limit(), bottom + (8 << 20), "override cleared");
    }

    #[test]
    fn an_override_above_the_sp_reports_no_limit() {
        let sp = stack_address();
        let bottom = sp + (1 << 20);
        set_stack_bounds_override(Some((bottom as usize, (bottom + (64 << 20)) as usize)));
        let limit = thread_stack_limit();
        set_stack_bounds_override(None);
        assert_eq!(limit, 0);
    }

    #[test]
    fn override_setter_returns_the_previous_value_for_nesting() {
        let a = Some((1usize << 20, 2usize << 20));
        let b = Some((3usize << 20, 4usize << 20));
        assert_eq!(set_stack_bounds_override(a), None);
        assert_eq!(set_stack_bounds_override(b), a);
        assert_eq!(set_stack_bounds_override(a), b);
        assert_eq!(set_stack_bounds_override(None), a);
    }

    #[test]
    fn the_slot_is_swapped_to_the_override_limit_while_the_override_is_set() {
        let sp = stack_address();
        let bottom = sp - (16 << 20);
        set_stack_bounds_override(Some((bottom as usize, (bottom + (64 << 20)) as usize)));
        let mut slot: u64 = u64::MAX;
        let pointer = &mut slot as *mut u64;
        let inside = with_guard_slot(pointer, || {
            // SAFETY: `slot` outlives the call and nothing else touches it meanwhile.
            unsafe { pointer.read_volatile() }
        });
        set_stack_bounds_override(None);
        assert_eq!(inside, bottom + (8 << 20));
        assert_eq!(slot, u64::MAX, "the slot is restored after the call");
    }

    #[test]
    fn a_stack_above_the_limit_leaves_the_slot_untouched() {
        let mut slot: u64 = 1;
        let pointer = &mut slot as *mut u64;
        // SAFETY: `slot` outlives the call and nothing else touches it meanwhile.
        with_guard_slot(pointer, || {
            assert_eq!(unsafe { pointer.read_volatile() }, 1)
        });
        assert_eq!(slot, 1);
        with_guard_slot(std::ptr::null_mut(), || {});
    }
}
