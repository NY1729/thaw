//! Registry of the live object records and captured scopes of this thread: the node set a cycle collector will
//! walk. Measurement only for now (`mem_trace` prints the counts); nothing is collected here.
//!
//! Thread-local like `mem_trace`. A realm's JS can run on other OS threads (the thread coroutine backend, the
//! default unless a native addon is registered): those threads have their own registries, and a node dropped on
//! a thread other than the one that created it cannot be removed from its registry.
//!
//! Entries are `Weak`, so the registry never keeps a node alive. A node's `Drop` removes its entry first
//! (`swap_remove`, then the moved entry's index is fixed up), so on the creating thread the registry holds exactly
//! the live nodes. The one exception is a node dropped on another thread: its entry stays as a `Weak` to a dead
//! node. That keeps the node's allocation (not its contents) until the next sweep, so the registry is bounded by
//! a sweep (`List::sweep`: when the list reaches twice its live size, and at realm teardown). No `unsafe` is
//! needed, which is why this is not a `*const` list: a stale `Weak` is harmless, a stale raw pointer would be UB.
//!
//! Objects are registered eagerly (`new_obj`). Scopes are registered lazily: only the scopes a closure captures,
//! and their ancestors up to the first one already registered (`capture_scope`). A frame or block scope that no
//! closure captures never touches the registry; its `Drop` only reads one `Cell`.
use crate::{Obj, ObjData, Scope};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

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
    entries: Vec<Weak<T>>,
    /// The length at which the next `add` sweeps first.
    sweep_at: usize,
}

impl<T: Node> List<T> {
    const fn new() -> Self {
        List {
            entries: Vec::new(),
            sweep_at: SWEEP_MIN,
        }
    }

    fn add(&mut self, node: &Rc<T>) {
        if self.entries.len() >= self.sweep_at {
            self.sweep();
        }
        node.reg_idx().set(self.entries.len());
        self.entries.push(Rc::downgrade(node));
    }

    /// Removes `node` (its index is `reg_idx`), if this list holds it: a node dropped on another thread is not
    /// here, and its stale entry is left for `sweep`.
    fn remove(&mut self, node: &T) {
        let idx = node.reg_idx().get();
        if idx == NONE {
            return;
        }
        if self
            .entries
            .get(idx)
            .is_some_and(|w| std::ptr::eq(w.as_ptr(), node))
        {
            self.entries.swap_remove(idx);
            // The moved entry's node is alive (each node's `Drop` removed its entry before it dropped anything),
            // so `upgrade` succeeds. The temporary `Rc` is dropped at once.
            if let Some(moved) = self.entries.get(idx).and_then(Weak::upgrade) {
                moved.reg_idx().set(idx);
            }
        }
    }

    /// Drops the entries whose node is gone and renumbers the rest.
    fn sweep(&mut self) {
        self.entries.retain(|w| w.strong_count() > 0);
        for (i, w) in self.entries.iter().enumerate() {
            if let Some(n) = w.upgrade() {
                n.reg_idx().set(i);
            }
        }
        self.sweep_at = (2 * self.entries.len()).max(SWEEP_MIN);
    }
}

thread_local! {
    static OBJS: RefCell<List<ObjData>> = const { RefCell::new(List::new()) };
    static SCOPES: RefCell<List<Scope>> = const { RefCell::new(List::new()) };
}

/// Adds a new object record. Called once, from `new_obj`.
pub(crate) fn register_obj(obj: &Obj) {
    let _ = OBJS.try_with(|r| r.borrow_mut().add(obj));
}

/// Removes `node` from the object list; called from its `Drop`, before its fields drop.
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
    let _ = OBJS.try_with(|r| r.borrow_mut().sweep());
    let _ = SCOPES.try_with(|r| r.borrow_mut().sweep());
}

/// Registered `(objects, scopes)` on this thread, stale entries included.
pub(crate) fn counts() -> (usize, usize) {
    let o = OBJS.try_with(|r| r.borrow().entries.len()).unwrap_or(0);
    let s = SCOPES.try_with(|r| r.borrow().entries.len()).unwrap_or(0);
    (o, s)
}

/// The live registered objects, as strong handles (the census walks them; the caller drops them).
pub(crate) fn live_objs() -> Vec<Obj> {
    OBJS.try_with(|r| {
        r.borrow()
            .entries
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    })
    .unwrap_or_default()
}

/// Checks that every entry is live, points back at its node, and that each node's index names its entry.
#[cfg(test)]
pub(crate) fn check_invariants() -> Result<(), String> {
    OBJS.with(|r| {
        for (i, w) in r.borrow().entries.iter().enumerate() {
            let o = w.upgrade().ok_or(format!("obj entry {i} is dead"))?;
            if o.reg_idx.get() != i {
                return Err(format!("obj entry {i} has reg_idx {}", o.reg_idx.get()));
            }
        }
        Ok(())
    })?;
    SCOPES.with(|r| {
        for (i, w) in r.borrow().entries.iter().enumerate() {
            let s = w.upgrade().ok_or(format!("scope entry {i} is dead"))?;
            if s.reg_idx.get() != i {
                return Err(format!("scope entry {i} has reg_idx {}", s.reg_idx.get()));
            }
        }
        Ok(())
    })
}
