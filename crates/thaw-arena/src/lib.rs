//! Bump allocation for generated values. Lambda invocation boundaries reclaim
//! request-only generations while preserving allocations reachable from
//! registered globals, including references stored in runtime side tables.
//!
//! This crate is linked into *compiled Thaw programs* as a plain static
//! archive (see `thaw-cli`'s link step), not used as a normal Rust
//! dependency for generated code -- generated LLVM IR calls
//! `thaw_arena_alloc` directly by symbol name. `thaw-runtime` also depends
//! on this crate so it can reset all request-owned allocations after every
//! Lambda invocation.

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};

use bumpalo::Bump;
mod strings;
pub use strings::*;

thread_local! {
    static ARENA: RefCell<Bump> = RefCell::new(Bump::new());
    static RETAINED: RefCell<Vec<Bump>> = const { RefCell::new(Vec::new()) };
    static ALLOCATIONS: RefCell<BTreeMap<usize, (usize, usize)>> = const { RefCell::new(BTreeMap::new()) };
    static ROOTS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static RECLAIMED: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
    static REFERENCES: RefCell<BTreeMap<usize, Vec<usize>>> = const { RefCell::new(BTreeMap::new()) };
    static TRACING: Cell<bool> = const { Cell::new(false) };
    static PINNED: RefCell<BTreeMap<usize, usize>> = const { RefCell::new(BTreeMap::new()) };
    static RESET_HOOKS: RefCell<Vec<fn(bool)>> = const { RefCell::new(Vec::new()) };
}

/// A runtime-owned reference, released when its callback or watcher is dropped.
pub struct ArenaRoot(usize, std::marker::PhantomData<std::rc::Rc<()>>);

impl ArenaRoot {
    pub fn new(pointer: usize) -> Self {
        let pointer = if is_tracing() { pointer } else { 0 };
        if pointer != 0 {
            PINNED.with(|pinned| *pinned.borrow_mut().entry(pointer).or_default() += 1);
        }
        Self(pointer, std::marker::PhantomData)
    }
}
impl Clone for ArenaRoot {
    fn clone(&self) -> Self {
        Self::new(self.0)
    }
}
impl Drop for ArenaRoot {
    fn drop(&mut self) {
        let _ = PINNED.try_with(|pinned| {
            let mut pinned = pinned.borrow_mut();
            if let Some(count) = pinned.get_mut(&self.0) {
                *count -= 1;
                if *count == 0 {
                    pinned.remove(&self.0);
                }
            }
        });
    }
}

/// Enables invocation tracing before module initialization allocates values.
#[no_mangle]
pub extern "C" fn thaw_arena_enable_tracing() {
    TRACING.with(|tracing| tracing.set(true));
}

pub fn is_tracing() -> bool {
    TRACING.with(Cell::get)
}

/// Tracks references held in runtime side tables rather than native fields.
pub fn replace_reference(owner: usize, previous: usize, next: usize) {
    if !is_tracing() {
        return;
    }
    REFERENCES.with(|references| {
        let mut references = references.borrow_mut();
        let children = references.entry(owner).or_default();
        if let Some(index) = children.iter().position(|&child| child == previous) {
            children.remove(index);
        }
        if next != 0 {
            children.push(next);
        }
    });
}

pub fn forget_references(owner: usize) {
    REFERENCES.with(|references| {
        references.borrow_mut().remove(&owner);
    });
}

/// Registers a process-lifetime pointer slot in generated module storage.
/// # Safety
/// `slot` must stay readable until process exit and contain an initialized
/// pointer-sized value whenever the arena is reset.
#[no_mangle]
pub unsafe extern "C" fn thaw_arena_register_root(slot: *const usize) {
    thaw_arena_enable_tracing();
    ROOTS.with(|roots| {
        let mut roots = roots.borrow_mut();
        if !roots.contains(&(slot as usize)) {
            roots.push(slot as usize);
        }
    });
}

/// Whether the last reset reclaimed the allocation containing this identity.
pub fn was_reclaimed(pointer: usize) -> bool {
    RECLAIMED.with(|ranges| {
        let ranges = ranges.borrow();
        let index = ranges.partition_point(|&(start, _)| start <= pointer);
        index != 0 && pointer < ranges[index - 1].1
    })
}

/// Registers thread-local side-table cleanup after each arena reset.
/// The hook receives whether tracing preserved reachable allocations; when
/// true, it may use `was_reclaimed` to discard only dead identities.
pub fn register_reset_hook(hook: fn(bool)) {
    RESET_HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        if !hooks.contains(&hook) {
            hooks.push(hook);
        }
    });
}

fn run_reset_hooks(tracing: bool) {
    let hooks = RESET_HOOKS.with(|hooks| hooks.borrow().clone());
    for hook in hooks {
        hook(tracing);
    }
}

/// Allocates `size` bytes aligned to `align` from the thread-local arena.
/// Returns a null pointer if `align` isn't a valid alignment (a power of
/// two) or the underlying allocation fails.
///
/// The pointer stays valid until reset, or longer when reachable from a
/// registered process-lifetime root on the same thread.
#[no_mangle]
pub extern "C" fn thaw_arena_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return std::ptr::null_mut();
    };
    let pointer = ARENA.with(|arena| {
        arena
            .borrow_mut()
            .try_alloc_layout(layout)
            .map(|allocation| allocation.as_ptr())
    });
    let Ok(pointer) = pointer else {
        return std::ptr::null_mut();
    };
    if is_tracing() {
        unsafe { pointer.write_bytes(0, size) };
        let generation = RETAINED.with(|retained| retained.borrow().len());
        ALLOCATIONS.with(|allocations| {
            allocations
                .borrow_mut()
                .insert(pointer as usize, (size.max(1), generation));
        });
    }
    pointer
}

/// Reclaims generations with no values reachable from registered global roots.
/// With tracing disabled this is a plain bulk reset.
/// `thaw-runtime` runs it once at every Lambda invocation boundary.
#[no_mangle]
pub extern "C" fn thaw_arena_reset() {
    let roots = ROOTS.with(|roots| roots.borrow().clone());
    if !is_tracing() {
        strings::reset_lengths(false);
        ARENA.with(|arena| arena.borrow_mut().reset());
        run_reset_hooks(false);
        return;
    }
    ALLOCATIONS.with(|allocations| {
        let mut allocations = allocations.borrow_mut();
        let mut pending = roots
            .iter()
            .map(|&slot| unsafe { (slot as *const usize).read_unaligned() })
            .collect::<Vec<_>>();
        PINNED.with(|pinned| pending.extend(pinned.borrow().keys().copied()));
        let mut live = HashSet::new();
        let mut owners = HashSet::new();
        while let Some(pointer) = pending.pop() {
            if owners.insert(pointer) {
                REFERENCES.with(|references| {
                    if let Some(children) = references.borrow().get(&pointer) {
                        pending.extend(children);
                    }
                });
            }
            let Some((&start, &(size, _))) = allocations.range(..=pointer).next_back() else {
                continue;
            };
            if pointer - start >= size || !live.insert(start) {
                continue;
            }
            for offset in (0..size.saturating_sub(std::mem::size_of::<usize>() - 1))
                .step_by(std::mem::size_of::<usize>())
            {
                pending.push(unsafe { ((start + offset) as *const usize).read_unaligned() });
            }
        }
        let generations = live
            .iter()
            .map(|start| allocations[start].1)
            .collect::<HashSet<_>>();
        REFERENCES.with(|references| {
            references
                .borrow_mut()
                .retain(|owner, _| live.contains(owner) || owners.contains(owner))
        });
        RECLAIMED.with(|ranges| {
            let mut ranges = ranges.borrow_mut();
            ranges.clear();
            allocations.retain(|&start, (size, _)| {
                if live.contains(&start) {
                    true
                } else {
                    ranges.push((start, start + *size));
                    false
                }
            });
        });
        RETAINED.with(|retained| {
            let mut retained = retained.borrow_mut();
            let current = retained.len();
            if generations.contains(&current) {
                ARENA.with(|arena| retained.push(std::mem::take(&mut *arena.borrow_mut())));
            } else {
                ARENA.with(|arena| arena.borrow_mut().reset());
            }
            // ponytail: retain whole bump generations containing live values;
            // per-allocation reclamation is needed only if retained slack matters.
            let mut mapping = BTreeMap::new();
            let mut index = 0;
            let mut old_index = 0;
            retained.retain(|_| {
                let keep = generations.contains(&old_index);
                if keep {
                    mapping.insert(old_index, index);
                    index += 1;
                }
                old_index += 1;
                keep
            });
            for (_, generation) in allocations.values_mut() {
                *generation = mapping[generation];
            }
        });
    });
    strings::reset_lengths(true);
    run_reset_hooks(true);
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

    #[test]
    fn roots_preserve_transitive_values_and_release_replaced_values() {
        let mut root = 0_usize;
        unsafe { thaw_arena_register_root(&raw const root) };
        let object = thaw_arena_alloc(8, 8).cast::<usize>();
        let child = thaw_arena_alloc(8, 8).cast::<usize>();
        unsafe {
            object.write(child as usize);
            child.write(42);
        }
        root = object as usize;
        assert_eq!(root, object as usize);
        thaw_arena_reset();
        assert_eq!(unsafe { child.read() }, 42);
        let replacement = thaw_arena_alloc(8, 8).cast::<usize>();
        unsafe {
            replacement.write(99);
            object.write(replacement as usize);
        }
        thaw_arena_reset();
        assert!(was_reclaimed(child as usize));
        assert_eq!(unsafe { replacement.read() }, 99);
        root = 0;
        assert_eq!(root, 0);
        let pinned = ArenaRoot::new(object as usize);
        let copy = pinned.clone();
        drop(pinned);
        thaw_arena_reset();
        assert!(!was_reclaimed(object as usize));
        assert_eq!(unsafe { replacement.read() }, 99);
        drop(copy);
        thaw_arena_reset();
        assert!(was_reclaimed(object as usize));
        ROOTS.with(|roots| roots.borrow_mut().clear());
        TRACING.with(|tracing| tracing.set(false));
    }
}
