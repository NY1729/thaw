//! The arena allocator from the design doc's memory model (section 3.2,
//! "Layer 2: bump allocation, bulk-freed at request end, no GC pause").
//!
//! This crate is linked into *compiled Thaw programs* as a plain static
//! archive (see `thaw-cli`'s link step), not used as a normal Rust
//! dependency for generated code -- generated LLVM IR calls
//! `thaw_arena_alloc` directly by symbol name. `thaw-runtime` also depends
//! on this crate so it can reset all request-owned allocations after every
//! Lambda invocation.

use std::alloc::Layout;
use std::cell::RefCell;

use bumpalo::Bump;

thread_local! {
    static ARENA: RefCell<Bump> = RefCell::new(Bump::new());
}

/// Allocates `size` bytes aligned to `align` from the thread-local arena.
/// Returns a null pointer if `align` isn't a valid alignment (a power of
/// two) or the underlying allocation fails.
///
/// The returned pointer stays valid until the next `thaw_arena_reset` call
/// on the same thread; compiled Thaw code must not keep it alive past that.
#[no_mangle]
pub extern "C" fn thaw_arena_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return std::ptr::null_mut();
    };
    ARENA.with(|arena| arena.borrow_mut().alloc_layout(layout).as_ptr())
}

/// Reclaims all memory allocated from the thread-local arena in one shot.
/// This is the "bulk-free, no GC pause" step from the design doc.
/// `thaw-runtime` runs it once at every Lambda invocation boundary.
#[no_mangle]
pub extern "C" fn thaw_arena_reset() {
    ARENA.with(|arena| arena.borrow_mut().reset());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_distinct_writable_regions() {
        let a = thaw_arena_alloc(8, 8);
        let b = thaw_arena_alloc(8, 8);
        assert!(!a.is_null());
        assert!(!b.is_null());
        assert_ne!(a, b);

        unsafe {
            (a as *mut u64).write(0x1122_3344_5566_7788);
            (b as *mut u64).write(0x99);
            assert_eq!((a as *const u64).read(), 0x1122_3344_5566_7788);
            assert_eq!((b as *const u64).read(), 0x99);
        }
    }

    #[test]
    fn reset_reclaims_and_stays_usable() {
        for _ in 0..1000 {
            assert!(!thaw_arena_alloc(64, 8).is_null());
        }
        thaw_arena_reset();
        assert!(!thaw_arena_alloc(64, 8).is_null());
    }

    #[test]
    fn rejects_non_power_of_two_alignment() {
        assert!(thaw_arena_alloc(8, 3).is_null());
    }
}
