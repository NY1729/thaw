//! Registry of the live object records and captured scopes of this thread: the node set a cycle collector will
//! walk. Measurement only for now (`mem_trace` prints the counts); nothing is collected here.
//!
//! Thread-local like `mem_trace`. A realm's JS can run on other OS threads (the thread coroutine backend, the
//! default unless a native addon is registered): those threads have their own registries, and a node dropped on
//! a thread other than the one that created it cannot be removed from its registry.
//!
//! Entries are `Weak`, so the registry never keeps a node alive. A node's `Drop` frees its slot first (the slot is
//! `None` and goes on a free list; no other node is touched), so on the creating thread the registry holds exactly
//! the live nodes. The one exception is a node dropped on another thread: its entry stays as a `Weak` to a dead
//! node. That keeps the node's allocation (not its contents) until a sweep. `STALE` counts those removals; a sweep
//! runs only when `STALE` is non-zero (when the list must grow past its threshold, and at realm teardown), so a
//! single-threaded run never scans the lists. No `unsafe` is needed, which is why this is not a `*const` list:
//! a stale `Weak` is harmless, a stale raw pointer would be UB.
//!
//! Objects are registered eagerly (`new_obj`). Scopes are registered lazily: only the scopes a closure captures,
//! and their ancestors up to the first one already registered (`capture_scope`). A frame or block scope that no
//! closure captures never touches the registry; its `Drop` only reads one `Cell`.
use crate::{Obj, ObjData, Scope};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Removals that found their node's entry in another thread's list: those entries are stale until a sweep.
static STALE: AtomicUsize = AtomicUsize::new(0);

/// `reg_idx` of a node that is not in the registry.
pub(crate) const NONE: usize = usize::MAX;

/// The first sweep happens at this many entries.
const SWEEP_MIN: usize = 1024;

/// A node the registry lists; it keeps its own index in the list.
pub(crate) trait Node {
    fn reg_idx(&self) -> &Cell<usize>;
}

impl Node for ObjData {
    fn reg_idx(&self) -> &Cell<usize> {
        &self.reg_idx
    }
}

impl Node for Scope {
    fn reg_idx(&self) -> &Cell<usize> {
        &self.reg_idx
    }
}

struct List<T: Node> {
    /// Slot `i` holds the node whose `reg_idx` is `i`; a `None` slot is listed in `free`.
    entries: Vec<Option<Weak<T>>>,
    /// Empty slots, reused before the list grows.
    free: Vec<usize>,
    /// The length at which growing the list sweeps first.
    sweep_at: usize,
}

impl<T: Node> List<T> {
    const fn new() -> Self {
        List {
            entries: Vec::new(),
            free: Vec::new(),
            sweep_at: SWEEP_MIN,
        }
    }

    fn live(&self) -> usize {
        self.entries.len() - self.free.len()
    }

    /// Slots are stable: no other node is touched, so an add or a remove costs the node itself plus one slot.
    /// Growing past `sweep_at` sweeps first, but only when some node died on another thread (`STALE`).
    fn add(&mut self, node: &Rc<T>) {
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                if self.entries.len() >= self.sweep_at && STALE.load(Ordering::Relaxed) > 0 {
                    self.sweep();
                }
                match self.free.pop() {
                    Some(i) => i,
                    None => {
                        self.entries.push(None);
                        self.entries.len() - 1
                    }
                }
            }
        };
        node.reg_idx().set(idx);
        self.entries[idx] = Some(Rc::downgrade(node));
    }

    /// Frees `node`'s slot (its index is `reg_idx`), if this list holds it: a node dropped on another thread is
    /// not here, and its stale entry is left for `sweep`.
    fn remove(&mut self, node: &T) {
        let idx = node.reg_idx().get();
        if idx == NONE {
            return;
        }
        let Some(slot) = self.entries.get_mut(idx) else {
            return;
        };
        if slot
            .as_ref()
            .is_some_and(|w| std::ptr::eq(w.as_ptr(), node))
        {
            // Dropping the `Weak` here touches only `node`, which is being dropped anyway.
            *slot = None;
            self.free.push(idx);
        } else {
            // Dropped on another thread: its entry is in the creating thread's list and goes stale there.
            STALE.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Frees the slots whose node is gone (dead on another thread) and sets the next growth threshold.
    fn sweep(&mut self) {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|w| w.strong_count() == 0) {
                *slot = None;
                self.free.push(i);
            }
        }
        self.sweep_at = (2 * self.live()).max(SWEEP_MIN);
    }
}

thread_local! {
    static OBJS: RefCell<List<ObjData>> = const { RefCell::new(List::new()) };
    static SCOPES: RefCell<List<Scope>> = const { RefCell::new(List::new()) };
}

/// Adds a new object record. Called once, from `new_obj`.
#[inline]
pub(crate) fn register_obj(obj: &Obj) {
    let _ = OBJS.try_with(|r| r.borrow_mut().add(obj));
}

/// Removes `node` from the object list; called from its `Drop`, before its fields drop.
#[inline]
pub(crate) fn unregister_obj(node: &ObjData) {
    let _ = OBJS.try_with(|r| r.borrow_mut().remove(node));
}

/// Registers `scope` and each ancestor not yet registered. Called for every scope a closure captures.
pub(crate) fn capture_scope(scope: &Rc<Scope>) {
    let mut cur = scope;
    while cur.reg_idx.get() == NONE {
        let _ = SCOPES.try_with(|r| r.borrow_mut().add(cur));
        match &cur.parent {
            Some(p) => cur = p,
            None => break,
        }
    }
}

/// Removes `node` from the scope list; called from `Scope::drop`.
pub(crate) fn unregister_scope(node: &Scope) {
    // The common case (a frame no closure captured) returns before touching the thread-local.
    if node.reg_idx.get() == NONE {
        return;
    }
    let _ = SCOPES.try_with(|r| r.borrow_mut().remove(node));
}

/// Drops the stale entries of both lists (nodes that died on another thread). Called at realm teardown.
pub(crate) fn sweep_stale() {
    if STALE.load(Ordering::Relaxed) == 0 {
        return;
    }
    let _ = OBJS.try_with(|r| r.borrow_mut().sweep());
    let _ = SCOPES.try_with(|r| r.borrow_mut().sweep());
}

/// Registered `(objects, scopes)` on this thread, stale entries included.
pub(crate) fn counts() -> (usize, usize) {
    let o = OBJS.try_with(|r| r.borrow().live()).unwrap_or(0);
    let s = SCOPES.try_with(|r| r.borrow().live()).unwrap_or(0);
    (o, s)
}

/// The live registered objects, as strong handles (the census walks them; the caller drops them).
pub(crate) fn live_objs() -> Vec<Obj> {
    OBJS.try_with(|r| {
        r.borrow()
            .entries
            .iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .collect()
    })
    .unwrap_or_default()
}

/// Strong handles to every registered node, by slot (`reg_idx` is the index): `None` for a free slot or a node that
/// died on another thread. The list is borrowed only while the handles are cloned, so the caller may drop them.
/// Empty if the list is busy (`try_borrow`), which makes a collection a no-op.
pub(crate) fn snapshot() -> (Vec<Option<Obj>>, Vec<Option<Rc<Scope>>>) {
    let objs = OBJS
        .try_with(|r| {
            r.try_borrow()
                .map(|l| {
                    l.entries
                        .iter()
                        .map(|w| w.as_ref().and_then(Weak::upgrade))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let scopes = SCOPES
        .try_with(|r| {
            r.try_borrow()
                .map(|l| {
                    l.entries
                        .iter()
                        .map(|w| w.as_ref().and_then(Weak::upgrade))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    (objs, scopes)
}

/// Checks that every entry is live, points back at its node, and that each node's index names its entry.
#[cfg(test)]
pub(crate) fn check_invariants() -> Result<(), String> {
    OBJS.with(|r| {
        for (i, w) in r.borrow().entries.iter().enumerate() {
            let Some(w) = w else { continue };
            let o = w.upgrade().ok_or(format!("obj entry {i} is dead"))?;
            if o.reg_idx.get() != i {
                return Err(format!("obj entry {i} has reg_idx {}", o.reg_idx.get()));
            }
        }
        Ok(())
    })?;
    SCOPES.with(|r| {
        for (i, w) in r.borrow().entries.iter().enumerate() {
            let Some(w) = w else { continue };
            let s = w.upgrade().ok_or(format!("scope entry {i} is dead"))?;
            if s.reg_idx.get() != i {
                return Err(format!("scope entry {i} has reg_idx {}", s.reg_idx.get()));
            }
        }
        Ok(())
    })
}
