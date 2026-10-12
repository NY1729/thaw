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
use std::collections::{BTreeMap, HashMap, HashSet};

use bumpalo::Bump;
mod strings;
pub use strings::*;
pub mod error_wire;
pub mod exception;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AllocationId {
    pub base: usize,
    pub serial: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationSnapshot {
    pub id: AllocationId,
    pub size: usize,
    pub generation: usize,
    pub pointer_words: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGraphSnapshot {
    pub epoch: u64,
    pub allocations: BTreeMap<usize, AllocationSnapshot>,
    pub references: BTreeMap<usize, Vec<usize>>,
    pub root_values: Vec<usize>,
    /// Addresses of registered root slots, which may themselves be side-edge owners.
    pub root_slots: Vec<usize>,
    pub pinned_counts: BTreeMap<usize, usize>,
    _thread_local: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NativeMark {
    pub allocation_starts: HashSet<usize>,
    pub owner_nodes: HashSet<usize>,
    pub retained_generations: HashSet<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    BorrowConflict,
    IdentityUnavailable,
}

thread_local! {
    static ARENA: RefCell<Bump> = RefCell::new(Bump::new());
    static RETAINED: RefCell<Vec<Bump>> = const { RefCell::new(Vec::new()) };
    static ALLOCATIONS: RefCell<BTreeMap<usize, (usize, usize)>> = const { RefCell::new(BTreeMap::new()) };
    static ALLOCATION_IDS: RefCell<BTreeMap<usize, u64>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_ALLOCATION_ID: Cell<u64> = const { Cell::new(0) };
    static GRAPH_EPOCH: Cell<Option<u64>> = const { Cell::new(Some(0)) };
    static ROOTS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static RECLAIMED: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
    static REFERENCES: RefCell<BTreeMap<usize, Vec<usize>>> = const { RefCell::new(BTreeMap::new()) };
    /// Side edges owned by a specific allocation incarnation. Keys include
    /// the byte-address node so multiple cell fields in one allocation remain
    /// independent when a bump address is reused.
    static ALLOCATION_REFERENCES: RefCell<HashMap<(AllocationId, usize), Vec<usize>>> =
        RefCell::new(HashMap::new());
    static TRACING: Cell<bool> = const { Cell::new(false) };
    static PINNED: RefCell<BTreeMap<usize, usize>> = const { RefCell::new(BTreeMap::new()) };
    static RESET_HOOKS: RefCell<Vec<fn(bool)>> = const { RefCell::new(Vec::new()) };
    static METADATA_RESET_HOOKS: RefCell<Vec<fn(bool)>> = const { RefCell::new(Vec::new()) };
    static RESET_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

struct ResetGuard;

impl ResetGuard {
    fn acquire() -> Option<Self> {
        RESET_ACTIVE
            .try_with(|active| {
                if active.replace(true) {
                    None
                } else {
                    Some(Self)
                }
            })
            .ok()
            .flatten()
    }
}

impl Drop for ResetGuard {
    fn drop(&mut self) {
        let _ = RESET_ACTIVE.try_with(|active| active.set(false));
    }
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

/// Replaces one side-table edge owned by an exact live allocation generation.
/// Returns false when the owner address no longer belongs to `identity`.
pub fn replace_allocation_reference(
    identity: AllocationId,
    owner: usize,
    previous: usize,
    next: usize,
) -> bool {
    if allocation_identity(owner) != Some(identity) { return false; }
    ALLOCATION_REFERENCES.with(|references| {
        let mut references = references.borrow_mut();
        let children = references.entry((identity, owner)).or_default();
        if let Some(index) = children.iter().position(|&child| child == previous) {
            children.remove(index);
        }
        if next != 0 { children.push(next); }
        let empty = children.is_empty();
        if empty { references.remove(&(identity, owner)); }
    });
    true
}

/// Removes one edge from a retired allocation generation without touching
/// edges that may now occupy the same raw owner address.
pub fn remove_allocation_reference(identity: AllocationId, owner: usize, child: usize) {
    ALLOCATION_REFERENCES.with(|references| {
        let mut references = references.borrow_mut();
        let key = (identity, owner);
        if let Some(children) = references.get_mut(&key) {
            if let Some(index) = children.iter().position(|&value| value == child) {
                children.remove(index);
            }
            let empty = children.is_empty();
            if empty { references.remove(&key); }
        }
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

/// Removes a process-lifetime root slot at module/JIT teardown.
///
/// # Safety
/// `slot` must have been registered with `thaw_arena_register_root` and must
/// remain readable for this call. The caller clears or releases its value
/// before unregistering so the slot cannot publish a stale arena pointer.
#[no_mangle]
pub unsafe extern "C" fn thaw_arena_unregister_root(slot: *const usize) -> u8 {
    let address = slot as usize;
    let removed = ROOTS.with(|roots| {
        let mut roots = roots.borrow_mut();
        let Some(index) = roots.iter().position(|root| *root == address) else {
            return false;
        };
        roots.swap_remove(index);
        true
    });
    if removed {
        forget_references(address);
    }
    u8::from(removed)
}

/// Whether the last reset reclaimed the allocation containing this identity.
pub fn was_reclaimed(pointer: usize) -> bool {
    RECLAIMED.with(|ranges| {
        let ranges = ranges.borrow();
        let index = ranges.partition_point(|&(start, _)| start <= pointer);
        index != 0 && pointer < ranges[index - 1].1
    })
}

/// A foreign FFI pointer cannot be kept alive by an ArenaRoot. Check that a
/// prospective live-object owner still belongs to a tracked allocation
/// before attaching a retained JS wrapper or callback to it.
pub fn contains_allocation(pointer: usize) -> bool {
    if pointer == 0 || !is_tracing() {
        return false;
    }
    ALLOCATIONS.with(|allocations| {
        let allocations = allocations.borrow();
        allocations
            .range(..=pointer)
            .next_back()
            .is_some_and(|(&start, &(size, _))| pointer - start < size)
    })
}

/// Returns the stable identity of a tracked allocation containing `pointer`.
pub fn allocation_identity(pointer: usize) -> Option<AllocationId> {
    if pointer == 0 || !is_tracing() {
        return None;
    }
    ALLOCATIONS.with(|allocations| {
        let allocations = allocations.try_borrow().ok()?;
        let (&base, &(size, _)) = allocations.range(..=pointer).next_back()?;
        if pointer - base >= size {
            return None;
        }
        let serial = ALLOCATION_IDS.with(|ids| ids.try_borrow().ok()?.get(&base).copied())?;
        Some(AllocationId { base, serial })
    })
}

/// Validates a complete, aligned byte range inside one tracked allocation and
/// returns that allocation's generation-safe identity plus the range offset.
/// Empty or overflowing ranges are rejected; callers must compare the returned
/// identity with the expected owner before writing through the range.
pub fn allocation_span_identity(
    pointer: usize,
    byte_len: usize,
    alignment: usize,
) -> Option<(AllocationId, usize)> {
    if pointer == 0 || byte_len == 0 || alignment == 0 || !alignment.is_power_of_two()
        || pointer % alignment != 0 || !is_tracing()
    {
        return None;
    }
    ALLOCATIONS.with(|allocations| {
        let allocations = allocations.try_borrow().ok()?;
        let (&base, &(size, _)) = allocations.range(..=pointer).next_back()?;
        let offset = pointer.checked_sub(base)?;
        let end = offset.checked_add(byte_len)?;
        if end > size { return None; }
        let serial = ALLOCATION_IDS.with(|ids| ids.try_borrow().ok()?.get(&base).copied())?;
        Some((AllocationId { base, serial }, offset))
    })
}

/// Copies native tracing metadata and initialized pointer words without changing
/// roots, references, allocation metadata, or bump generations.
pub fn snapshot_native_graph() -> Result<NativeGraphSnapshot, SnapshotError> {
    let allocations = ALLOCATIONS.with(|allocations| {
        let allocations = allocations.try_borrow().map_err(|_| SnapshotError::BorrowConflict)?;
        ALLOCATION_IDS.with(|ids| {
            let ids = ids.try_borrow().map_err(|_| SnapshotError::BorrowConflict)?;
            let mut copied = BTreeMap::new();
            for (&base, &(size, generation)) in allocations.iter() {
                let serial = ids.get(&base).copied().ok_or(SnapshotError::IdentityUnavailable)?;
                let mut pointer_words = Vec::new();
                for offset in (0..size.saturating_sub(std::mem::size_of::<usize>() - 1))
                    .step_by(std::mem::size_of::<usize>())
                {
                    pointer_words.push(unsafe { ((base + offset) as *const usize).read_unaligned() });
                }
                copied.insert(base, AllocationSnapshot {
                    id: AllocationId { base, serial }, size, generation, pointer_words,
                });
            }
            Ok(copied)
        })
    })?;
    let mut references = REFERENCES.with(|references| references.try_borrow()
        .map(|references| references.clone()).map_err(|_| SnapshotError::BorrowConflict))?;
    ALLOCATION_REFERENCES.with(|edges| {
        let edges = edges.try_borrow().map_err(|_| SnapshotError::BorrowConflict)?;
        for (&(identity, owner), children) in edges.iter() {
            if allocations.get(&identity.base).is_some_and(|allocation| allocation.id == identity) {
                references.entry(owner).or_default().extend(children.iter().copied());
            }
        }
        Ok::<(), SnapshotError>(())
    })?;
    let root_values = ROOTS.with(|roots| {
        let roots = roots.try_borrow().map_err(|_| SnapshotError::BorrowConflict)?;
        Ok(roots.iter().map(|&slot| unsafe { (slot as *const usize).read_unaligned() }).collect())
    })?;
    let root_slots = ROOTS.with(|roots| roots.try_borrow()
        .map(|roots| roots.clone()).map_err(|_| SnapshotError::BorrowConflict))?;
    let pinned_counts = PINNED.with(|pinned| pinned.try_borrow()
        .map(|pinned| pinned.clone()).map_err(|_| SnapshotError::BorrowConflict))?;
    let epoch = GRAPH_EPOCH.with(Cell::get).ok_or(SnapshotError::IdentityUnavailable)?;
    Ok(NativeGraphSnapshot { epoch, allocations, references, root_values, root_slots, pinned_counts,
        _thread_local: std::marker::PhantomData })
}

/// Computes native reachability using only copied snapshot data.
pub fn mark_native_graph(snapshot: &NativeGraphSnapshot, roots: &[usize]) -> NativeMark {
    let mut mark = NativeMark::default();
    let mut pending = snapshot.root_values.iter().copied()
        .chain(snapshot.root_slots.iter().copied())
        .chain(snapshot.pinned_counts.keys().copied())
        .chain(roots.iter().copied()).collect::<Vec<_>>();
    while let Some(pointer) = pending.pop() {
        if !mark.owner_nodes.insert(pointer) {
            continue;
        }
        if let Some(children) = snapshot.references.get(&pointer) {
            pending.extend(children.iter().copied());
        }
        let Some((&base, allocation)) = snapshot.allocations.range(..=pointer).next_back() else { continue; };
        if pointer - base >= allocation.size || !mark.allocation_starts.insert(base) { continue; }
        mark.retained_generations.insert(allocation.generation);
        pending.extend(allocation.pointer_words.iter().copied());
    }
    mark
}

#[no_mangle]
pub extern "C" fn thaw_arena_contains_allocation(pointer: *const u8) -> u8 {
    u8::from(contains_allocation(pointer as usize))
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

/// Registers cleanup of Rust-only side-table metadata before release callbacks.
/// Hooks must not call Host/user code, allocate arena payloads, or reset recursively.
pub fn register_metadata_reset_hook(hook: fn(bool)) {
    METADATA_RESET_HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        if !hooks.contains(&hook) {
            hooks.push(hook);
        }
    });
}

fn run_metadata_reset_hooks(tracing: bool) {
    let hooks = METADATA_RESET_HOOKS.with(|hooks| hooks.borrow().clone());
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
            let base = pointer as usize;
            allocations.borrow_mut().insert(base, (size.max(1), generation));
            ALLOCATION_IDS.with(|ids| {
                let serial = NEXT_ALLOCATION_ID.with(|next| next.get().checked_add(1));
                if let Some(serial) = serial {
                    NEXT_ALLOCATION_ID.with(|next| next.set(serial));
                    ids.borrow_mut().insert(base, serial);
                } else {
                    ids.borrow_mut().remove(&base);
                }
            });
            GRAPH_EPOCH.with(|epoch| epoch.set(epoch.get().and_then(|value| value.checked_add(1))));
        });
    }
    pointer
}

/// Reclaims generations with no values reachable from registered global roots.
/// With tracing disabled this is a plain bulk reset.
/// A same-thread reset called from a reset hook is ignored; callback allocations
/// are eligible for reclamation at the next independent reset.
/// `thaw-runtime` runs it once at every Lambda invocation boundary.
#[no_mangle]
pub extern "C" fn thaw_arena_reset() {
    let Some(_reset_guard) = ResetGuard::acquire() else {
        return;
    };
    let roots = ROOTS.with(|roots| roots.borrow().clone());
    if !is_tracing() {
        strings::reset_lengths(false);
        ARENA.with(|arena| arena.borrow_mut().reset());
        ALLOCATION_REFERENCES.with(|references| references.borrow_mut().clear());
        run_metadata_reset_hooks(false);
        run_reset_hooks(false);
        return;
    }
    ALLOCATIONS.with(|allocations| {
        let mut allocations = allocations.borrow_mut();
        let mut pending = roots
            .iter()
            .map(|&slot| unsafe { (slot as *const usize).read_unaligned() })
            .collect::<Vec<_>>();
        // A registered global slot can own side-table references (for
        // example, a Json binding Box plus its root token). Visit the slot
        // address as an owner node as well as visiting its current contents.
        pending.extend(roots.iter().copied());
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
                if let Some((&base, &(size, _))) = allocations.range(..=pointer).next_back() {
                    if pointer - base < size {
                        ALLOCATION_IDS.with(|ids| {
                            if let Some(serial) = ids.borrow().get(&base).copied() {
                                let identity = AllocationId { base, serial };
                                ALLOCATION_REFERENCES.with(|references| {
                                    if let Some(children) = references.borrow().get(&(identity, pointer)) {
                                        pending.extend(children);
                                    }
                                });
                            }
                        });
                    }
                }
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
        ALLOCATION_REFERENCES.with(|references| {
            ALLOCATION_IDS.with(|ids| {
                let ids = ids.borrow();
                references.borrow_mut().retain(|(identity, owner), _| {
                    live.contains(&identity.base)
                        && ids.get(&identity.base) == Some(&identity.serial)
                        && *owner >= identity.base
                        && allocations.get(&identity.base)
                            .is_some_and(|(size, _)| *owner - identity.base < *size)
                });
            });
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
            ALLOCATION_IDS.with(|ids| ids.borrow_mut().retain(|base, _| allocations.contains_key(base)));
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
    run_metadata_reset_hooks(true);
    run_reset_hooks(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static RESET_TEST_EVENTS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static RESET_TEST_LATE_ARMED: Cell<bool> = const { Cell::new(false) };
        static RESET_TEST_LATE_STATE: Cell<bool> = const { Cell::new(false) };
        static NESTED_RESET_ARMED: Cell<bool> = const { Cell::new(false) };
        static NESTED_RESET_FRESH: Cell<usize> = const { Cell::new(0) };
        static NESTED_RESET_OLD_AFTER_NESTED: Cell<bool> = const { Cell::new(false) };
        static NESTED_RESET_OLD_IN_B: Cell<bool> = const { Cell::new(false) };
        static NESTED_RESET_HOOK_COUNTS: RefCell<[usize; 2]> = const { RefCell::new([0; 2]) };
        static NESTED_RESET_METADATA_COUNT: Cell<usize> = const { Cell::new(0) };
        static NESTED_RESET_OLD: Cell<usize> = const { Cell::new(0) };
    }

    #[test]
    fn allocation_span_requires_full_aligned_pointer_width_within_one_generation() {
        std::thread::spawn(|| {
            thaw_arena_enable_tracing();
            let width = std::mem::size_of::<usize>();
            let align = std::mem::align_of::<usize>();
            let short = thaw_arena_alloc(width - 1, align) as usize;
            assert!(allocation_span_identity(short, width, align).is_none());

            let base = thaw_arena_alloc(width + 1, align) as usize;
            let identity = allocation_identity(base).unwrap();
            assert_eq!(allocation_span_identity(base, width, align), Some((identity, 0)));
            assert!(allocation_span_identity(base + 1, width, align).is_none(),
                "an unaligned pointer field cannot be published");
            assert!(allocation_span_identity(base + width, width, 1).is_none(),
                "a pointer-sized field cannot extend past the allocation end");
            assert_ne!(allocation_span_identity(base, width, align).unwrap().0.serial, 0);
        }).join().unwrap();
    }

    #[test]
    fn registered_global_slot_side_edge_survives_reset() {
        std::thread::spawn(|| {
            let mut global_slot = 0usize;
            let slot = &mut global_slot as *mut usize;
            unsafe { thaw_arena_register_root(slot) };
            unsafe { thaw_arena_register_root(slot) };

            let child = thaw_arena_alloc(8, 8) as usize;
            unsafe { (child as *mut usize).write(0); }
            replace_reference(slot as usize, 0, child);
            let snapshot = snapshot_native_graph().unwrap();
            assert!(snapshot.root_slots.contains(&(slot as usize)));
            assert!(mark_native_graph(&snapshot, &[]).owner_nodes.contains(&child));
            thaw_arena_reset();
            assert!(!was_reclaimed(child), "the global slot's side edge retains its child");

            replace_reference(slot as usize, child, 0);
            thaw_arena_reset();
            assert!(was_reclaimed(child), "removing the global edge releases its child");
            assert_eq!(unsafe { thaw_arena_unregister_root(slot) }, 1);
            assert_eq!(unsafe { thaw_arena_unregister_root(slot) }, 0);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn copied_mark_keeps_owner_nodes_distinct_from_allocation_starts() {
        let snapshot = NativeGraphSnapshot {
            epoch: 0,
            allocations: BTreeMap::from([
                (100, AllocationSnapshot { id: AllocationId { base: 100, serial: 1 }, size: 16,
                    generation: 3, pointer_words: vec![205, 9999] }),
                (200, AllocationSnapshot { id: AllocationId { base: 200, serial: 2 }, size: 8,
                    generation: 4, pointer_words: vec![] }),
            ]),
            references: BTreeMap::from([(205, vec![200, 200])]),
            root_values: vec![104],
            root_slots: vec![],
            pinned_counts: BTreeMap::new(),
            _thread_local: std::marker::PhantomData,
        };
        let mark = mark_native_graph(&snapshot, &[]);
        assert_eq!(mark.owner_nodes, HashSet::from([104, 205, 200, 9999]));
        assert_eq!(mark.allocation_starts, HashSet::from([100, 200]));
        assert_eq!(mark.retained_generations, HashSet::from([3, 4]));
        assert_eq!(snapshot.references[&205], vec![200, 200]);
    }

    fn reset_test_metadata_hook(_: bool) {
        assert!(METADATA_RESET_HOOKS.with(|hooks| hooks.try_borrow_mut().is_ok()));
        assert!(RESET_HOOKS.with(|hooks| hooks.try_borrow_mut().is_ok()));
        RESET_TEST_EVENTS.with(|events| events.borrow_mut().push(2));
    }

    fn reset_test_release_hook(_: bool) {
        assert!(METADATA_RESET_HOOKS.with(|hooks| hooks.try_borrow_mut().is_ok()));
        assert!(RESET_HOOKS.with(|hooks| hooks.try_borrow_mut().is_ok()));
        RESET_TEST_EVENTS.with(|events| events.borrow_mut().push(1));
    }

    fn reset_test_late_metadata_hook(_: bool) {
        RESET_TEST_LATE_STATE.with(|state| state.set(false));
    }

    fn reset_test_late_release_hook(_: bool) {
        if RESET_TEST_LATE_ARMED.with(|armed| armed.replace(false)) {
            RESET_TEST_LATE_STATE.with(|state| state.set(true));
            register_metadata_reset_hook(reset_test_late_metadata_hook);
        }
    }

    fn nested_reset_metadata_hook(_: bool) {
        NESTED_RESET_METADATA_COUNT.with(|count| count.set(count.get() + 1));
    }

    fn nested_reset_release_a(_: bool) {
        NESTED_RESET_HOOK_COUNTS.with(|counts| counts.borrow_mut()[0] += 1);
        if NESTED_RESET_ARMED.with(|armed| armed.replace(false)) {
            let fresh = thaw_arena_alloc(8, 8) as usize;
            NESTED_RESET_FRESH.with(|pointer| pointer.set(fresh));
            thaw_arena_reset();
            thaw_arena_reset();
            let old = NESTED_RESET_OLD.with(Cell::get);
            NESTED_RESET_OLD_AFTER_NESTED.with(|reclaimed| reclaimed.set(was_reclaimed(old)));
        }
    }

    fn nested_reset_release_b(_: bool) {
        NESTED_RESET_HOOK_COUNTS.with(|counts| counts.borrow_mut()[1] += 1);
        let old = NESTED_RESET_OLD.with(Cell::get);
        NESTED_RESET_OLD_IN_B.with(|reclaimed| reclaimed.set(was_reclaimed(old)));
    }

    #[test]
    fn nested_reset_preserves_outer_classification_and_fresh_allocations() {
        std::thread::spawn(|| {
            thaw_arena_enable_tracing();
            let old = thaw_arena_alloc(8, 8) as usize;
            let survivor = thaw_arena_alloc(8, 8) as usize;
            let _survivor_pin = ArenaRoot::new(survivor);
            NESTED_RESET_OLD.with(|pointer| pointer.set(old));
            register_reset_hook(nested_reset_release_a);
            register_reset_hook(nested_reset_release_b);
            NESTED_RESET_ARMED.with(|armed| armed.set(true));

            thaw_arena_reset();

            assert!(NESTED_RESET_OLD_AFTER_NESTED.with(Cell::get));
            assert!(NESTED_RESET_OLD_IN_B.with(Cell::get));
            let fresh = NESTED_RESET_FRESH.with(Cell::get);
            assert!(contains_allocation(fresh));
            assert_eq!(
                NESTED_RESET_HOOK_COUNTS.with(|counts| counts.borrow()[1]),
                1
            );

            let fresh_pin = ArenaRoot::new(fresh);
            thaw_arena_reset();
            assert!(contains_allocation(fresh));
            drop(fresh_pin);
            thaw_arena_reset();
            assert!(was_reclaimed(fresh));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn nested_reset_is_ignored_without_tracing_and_later_reset_runs() {
        std::thread::spawn(|| {
            register_reset_hook(nested_reset_release_a);
            register_reset_hook(nested_reset_release_b);
            register_metadata_reset_hook(nested_reset_metadata_hook);
            NESTED_RESET_ARMED.with(|armed| armed.set(true));
            thaw_arena_reset();

            assert_eq!(
                NESTED_RESET_HOOK_COUNTS.with(|counts| *counts.borrow()),
                [1, 1]
            );
            assert_eq!(NESTED_RESET_METADATA_COUNT.with(Cell::get), 1);
            thaw_arena_reset();
            assert_eq!(
                NESTED_RESET_HOOK_COUNTS.with(|counts| *counts.borrow()),
                [2, 2]
            );
            assert_eq!(NESTED_RESET_METADATA_COUNT.with(Cell::get), 2);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn metadata_hooks_run_once_before_release_hooks_in_both_reset_modes() {
        for tracing in [false, true] {
            std::thread::spawn(move || {
                if tracing {
                    thaw_arena_enable_tracing();
                }
                register_reset_hook(reset_test_release_hook);
                register_metadata_reset_hook(reset_test_metadata_hook);
                register_metadata_reset_hook(reset_test_metadata_hook);
                thaw_arena_reset();
                assert_eq!(
                    RESET_TEST_EVENTS.with(|events| events.borrow().clone()),
                    vec![2, 1]
                );
            })
            .join()
            .unwrap();
        }
    }

    #[test]
    fn metadata_hook_registered_during_release_starts_next_reset() {
        std::thread::spawn(|| {
            register_reset_hook(reset_test_late_release_hook);
            RESET_TEST_LATE_ARMED.with(|armed| armed.set(true));
            thaw_arena_reset();
            assert!(RESET_TEST_LATE_STATE.with(Cell::get));
            thaw_arena_reset();
            assert!(!RESET_TEST_LATE_STATE.with(Cell::get));
        })
        .join()
        .unwrap();
    }

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
