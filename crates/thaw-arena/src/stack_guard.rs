//! Stack-overflow guard for compiled code that runs on a thread other than the main one.
//!
//! Compiled functions compare their stack address with one process-wide limit (`__thaw_stack_limit`,
//! set at entry from the main thread's stack). A coroutine thread's stack lies in a different
//! address range, so every compiled call made there would look like an overflow. The host calls
//! [`with_thread_stack_guard`] around each compiled call: when the current stack is below the
//! registered limit, the limit is swapped to this thread's own bottom for the call and restored after.

use std::sync::atomic::{AtomicUsize, Ordering};

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
#[cfg(target_os = "linux")]
fn thread_stack_limit() -> u64 {
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
        let limit = addr as u64 + MARGIN.max(size as u64 / 8);
        if limit < stack_address() {
            limit
        } else {
            0
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn thread_stack_limit() -> u64 {
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
