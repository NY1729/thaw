//! Cycle collector: "subtract the internal references" (CPython's gc). PR-1 only adds the collector; nothing calls
//! `collect_cycles` outside the tests (PR-2 adds the trigger).
//!
//! Every registered node (`gc_registry`: each `ObjData`, each captured `Scope`) is walked once with `obj_edges` /
//! `scope_edges`, which report each `Rc` clone the node owns. An edge whose target is a registered node in this
//! snapshot is subtracted from that target's strong count. What is left over is referenced from outside the
//! registered graph (a Rust stack value, a native closure, a timer, a suspended coroutine, a root), so those nodes
//! are roots, and everything reachable from a root is live. The rest is garbage: its mutable contents are taken and
//! dropped, which breaks the cycles and lets plain reference counting free the nodes.
//!
//! Failure direction: a missed edge only leaves a cycle uncollected (no worse than before). An edge counted that is
//! not a real `Rc` clone would subtract a reference that is not there and could free a live node, so every edge
//! kind is audited against exactly one `Rc` clone (`obj_edges` documents each), and a negative count aborts the run.
//!
//! Opaque nodes are never cleared and always count as roots: a node whose cell is borrowed, a `with` scope, and
//! every kind whose edges are not enumerated (`Global`, `Map`, `Set`, `CollIter`, `Promise`, `Generator`, `Proxy`,
//! `Typed`). Their own edges are not subtracted, so anything they hold stays externally referenced.
#![allow(dead_code)] // PR-1: `collect_cycles` has no caller outside the tests

use crate::gc_registry;
use crate::{account, FxMap, Interp, Kind, Obj, ObjData, Prop, Scope, Slot, Value, Vars};
use std::cell::Cell;
use std::rc::Rc;

/// One `Rc` clone a node owns: an object handle or a scope handle.
#[derive(Clone, Copy)]
pub(crate) enum Edge<'a> {
    Obj(&'a Obj),
    Scope(&'a Rc<Scope>),
}

fn value_edge<'a>(buf: &mut Vec<Edge<'a>>, v: &'a Value) {
    if let Value::Object(o) = v {
        buf.push(Edge::Obj(o));
    }
}

/// Calls `f` once per `Rc` clone that `o` owns: each property value (data, getter, setter, one per occurrence),
/// the prototype, and the kind's objects (array items, wrapped value, closure scope). A `Slot::LazyProto` owner is a
/// `Weak` and is not an edge; its parent is. Returns false for an opaque object (a cell is borrowed, or the kind's
/// edges are not enumerated): then `f` was not called at all.
pub(crate) fn obj_edges(o: &ObjData, f: &mut dyn FnMut(Edge)) -> bool {
    // Read `raw_props`, never `props()`: materialising a lazy `prototype` is a mutation.
    let (Ok(props), Ok(proto), Ok(kind)) = (
        o.raw_props.try_borrow(),
        o.proto.try_borrow(),
        o.kind.try_borrow(),
    ) else {
        return false;
    };
    let mut buf = Vec::new();
    for p in props.iter() {
        match &p.slot {
            Slot::Data(v) => value_edge(&mut buf, v),
            Slot::Accessor { get, set } => {
                for v in [get, set].into_iter().flatten() {
                    value_edge(&mut buf, v);
                }
            }
            Slot::LazyProto { parent, .. } => buf.push(Edge::Obj(parent)),
        }
    }
    if let Some(p) = proto.as_ref() {
        buf.push(Edge::Obj(p));
    }
    // Every variant is named (no `_ =>`): a new `Kind` must be classified here.
    match &*kind {
        // Strong captures inside a native closure are not visible here; they count as external.
        Kind::Plain
        | Kind::Native(_)
        | Kind::RegExp(_)
        | Kind::Symbol { .. }
        | Kind::ArrayBuffer(_)
        | Kind::RawJson(_) => {}
        Kind::Array(items) => {
            for v in items {
                value_edge(&mut buf, v);
            }
        }
        // `def` (its class's parent value and field scope) is cached per process: an external count, not an edge.
        Kind::Closure { scope, .. } => buf.push(Edge::Scope(scope)),
        Kind::Wrapper(v) => value_edge(&mut buf, v),
        Kind::Global
        | Kind::Map(_)
        | Kind::Set(_)
        | Kind::CollIter(_)
        | Kind::Promise(_)
        | Kind::Generator(_)
        | Kind::Proxy(_)
        | Kind::Typed(_) => return false,
    }
    for e in buf {
        f(e);
    }
    true
}

/// Calls `f` once per `Rc` clone that `s` owns: the parent, the `this` value, and each binding's value. A scope with
/// a `with` record is opaque (its immutable field cannot be cleared). Returns false, calling `f` for nothing, when
/// the scope cannot be read.
pub(crate) fn scope_edges(s: &Scope, f: &mut dyn FnMut(Edge)) -> bool {
    if s.with.is_some() {
        return false;
    }
    let (Ok(vars), Ok(this)) = (s.vars.try_borrow(), s.this_val.try_borrow()) else {
        return false;
    };
    let mut buf = Vec::new();
    if let Some(p) = &s.parent {
        buf.push(Edge::Scope(p));
    }
    if let Some(v) = this.as_ref() {
        value_edge(&mut buf, v);
    }
    for (_, b) in &vars.list {
        value_edge(&mut buf, &b.value);
    }
    for e in buf {
        f(e);
    }
    true
}

/// A registered node held by the snapshot.
#[derive(Clone, Copy)]
enum Node<'a> {
    Obj(&'a Obj),
    Scope(&'a Rc<Scope>),
}

fn node_edges(n: Node, f: &mut dyn FnMut(Edge)) -> bool {
    match n {
        Node::Obj(o) => obj_edges(o, f),
        Node::Scope(s) => scope_edges(s, f),
    }
}

/// Contents taken out of a garbage node. Dropped only after every cell borrow is released.
enum Taken {
    Props(Vec<Prop>),
    Index(Option<FxMap<Rc<str>, usize>>),
    Obj(Obj),
    Kind(Kind),
    Values(Vec<Value>),
    Vars(Vars),
    Value(Value),
}

/// The snapshot, by node id: objects are `0..objs.len()`, then scopes. A free or dead slot is `None`.
struct Graph {
    objs: Vec<Option<Obj>>,
    scopes: Vec<Option<Rc<Scope>>>,
}

impl Graph {
    fn len(&self) -> usize {
        self.objs.len() + self.scopes.len()
    }

    fn node(&self, id: usize) -> Option<Node<'_>> {
        match id.checked_sub(self.objs.len()) {
            None => self.objs[id].as_ref().map(Node::Obj),
            Some(j) => self.scopes[j].as_ref().map(Node::Scope),
        }
    }

    /// The id of the node `e` names, if it is registered in this snapshot. `reg_idx` is checked against the slot's
    /// own pointer, so a foreign-thread index or a stale one never matches.
    fn id_of(&self, e: Edge) -> Option<usize> {
        match e {
            Edge::Obj(o) => {
                let i = o.reg_idx.get();
                (i < self.objs.len() && self.objs[i].as_ref().is_some_and(|s| Rc::ptr_eq(s, o)))
                    .then_some(i)
            }
            Edge::Scope(s) => {
                let j = s.reg_idx.get();
                (j < self.scopes.len() && self.scopes[j].as_ref().is_some_and(|x| Rc::ptr_eq(x, s)))
                    .then_some(self.objs.len() + j)
            }
        }
    }
}

/// One collection over the registry. Returns the number of nodes cleared, or 0 if the registry was busy or a count
/// went negative (the collection then frees nothing).
fn collect_nodes() -> usize {
    let (objs, scopes) = gc_registry::snapshot();
    let g = Graph { objs, scopes };
    let n = g.len();

    // External count per node: strong count, minus the snapshot's own clone.
    let mut ext: Vec<isize> = (0..n)
        .map(|id| match g.node(id) {
            Some(Node::Obj(o)) => Rc::strong_count(o) as isize - 1,
            Some(Node::Scope(s)) => Rc::strong_count(s) as isize - 1,
            None => 0,
        })
        .collect();

    // Subtract every edge whose target is registered in this snapshot.
    let mut opaque = vec![false; n];
    for (id, op) in opaque.iter_mut().enumerate() {
        let Some(node) = g.node(id) else { continue };
        let ok = node_edges(node, &mut |e| {
            if let Some(t) = g.id_of(e) {
                ext[t] -= 1;
            }
        });
        *op = !ok;
    }

    let bad = (0..n).find(|&id| ext[id] < 0);
    #[cfg(test)]
    assert!(
        bad.is_none(),
        "gc: node {bad:?} has more edges than references"
    );
    if bad.is_some() {
        debug_assert!(false, "gc: a node has more edges than references");
        return 0;
    }

    // Roots: externally referenced or opaque. Mark what they reach (an explicit worklist, so a long chain cannot
    // overflow the stack).
    let mut marked = vec![false; n];
    let mut work = Vec::new();
    for id in 0..n {
        if g.node(id).is_some() && (ext[id] > 0 || opaque[id]) {
            marked[id] = true;
            work.push(id);
        }
    }
    while let Some(id) = work.pop() {
        let Some(node) = g.node(id) else { continue };
        // An opaque node yields no edges, so its targets keep their external count and are roots themselves.
        node_edges(node, &mut |e| {
            if let Some(t) = g.id_of(e) {
                if !marked[t] {
                    marked[t] = true;
                    work.push(t);
                }
            }
        });
    }

    // Garbage: registered, not marked. Take the mutable contents of each; a node that cannot be borrowed is left
    // alone (its holders then keep it, which is conservative).
    let mut taken: Vec<Taken> = Vec::new();
    let mut cleared_objs = Vec::new();
    let mut cleared = 0;
    for (id, &is_marked) in marked.iter().enumerate() {
        if is_marked {
            continue;
        }
        match g.node(id) {
            Some(Node::Obj(o)) => {
                let (Ok(mut props), Ok(mut index), Ok(mut proto), Ok(mut kind)) = (
                    o.raw_props.try_borrow_mut(),
                    o.index.try_borrow_mut(),
                    o.proto.try_borrow_mut(),
                    o.kind.try_borrow_mut(),
                ) else {
                    continue;
                };
                taken.push(Taken::Props(std::mem::take(&mut *props)));
                taken.push(Taken::Index(index.take()));
                if let Some(p) = proto.take() {
                    taken.push(Taken::Obj(p));
                }
                match &mut *kind {
                    Kind::Array(items) => taken.push(Taken::Values(std::mem::take(items))),
                    _ => taken.push(Taken::Kind(std::mem::replace(&mut *kind, Kind::Plain))),
                }
                cleared_objs.push(id);
                cleared += 1;
            }
            Some(Node::Scope(s)) => {
                let (Ok(mut vars), Ok(mut this)) =
                    (s.vars.try_borrow_mut(), s.this_val.try_borrow_mut())
                else {
                    continue;
                };
                taken.push(Taken::Vars(std::mem::take(&mut *vars)));
                if let Some(v) = this.take() {
                    taken.push(Taken::Value(v));
                }
                cleared += 1;
            }
            None => {}
        }
    }

    // Every borrow above is released. Heap-limit accounting for the emptied objects, then the drops (which
    // unregister nodes, so the registry must not be borrowed) and the snapshot.
    for &id in &cleared_objs {
        if let Some(Node::Obj(o)) = g.node(id) {
            account(o);
        }
    }
    drop(taken);
    drop(g);
    cleared
}

thread_local! {
    /// Set while a collection runs; a nested call (from a `Drop` or a re-entrant path) returns 0.
    static IN_GC: Cell<bool> = const { Cell::new(false) };
}

/// Clears `IN_GC` when the collection ends, even by unwinding.
struct InGc;

impl Drop for InGc {
    fn drop(&mut self) {
        let _ = IN_GC.try_with(|c| c.set(false));
    }
}

fn collect() -> usize {
    if IN_GC.try_with(|c| c.replace(true)).unwrap_or(true) {
        return 0;
    }
    let _guard = InGc;
    let cleared = collect_nodes();
    drop(_guard);
    crate::mem_trace::mem_trace_report("collect");
    cleared
}

#[cfg(test)]
static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Test override: enables the collector regardless of the backend.
#[cfg(test)]
pub(crate) fn force_enabled() {
    FORCED.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
fn forced() -> bool {
    FORCED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(test))]
fn forced() -> bool {
    false
}

/// Whether `collect_cycles` may run. Decided once per process: the costack backend is in use (see
/// `co::stack_default`); with the thread backend the collector is off.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    forced() || *ON.get_or_init(crate::co::stack_default)
}

impl Interp {
    /// Clears the garbage cycles among the registered nodes and returns how many nodes were cleared. A node that is
    /// reachable from outside the registered graph (any Rust-held value, native closure, timer, suspended coroutine,
    /// or root) is kept, so this is safe whenever no `ObjData`/`Scope` cell borrow is held by the caller. The
    /// caller's borrows only decide how much is collected: a borrowed node counts as a root.
    ///
    /// Safe point (for the trigger in PR-2): call it only from interpreter methods at a statement, loop back-edge or
    /// call-entry level, never from `Drop` for `ObjData`/`Scope`, never while `gc_registry` is borrowed, and never
    /// inside a `json`/`inspect` address-set walk. A call during a collection returns 0.
    ///
    /// Returns 0 when the collector is disabled (`THAW_JS_CO=thread`, or no costack support).
    pub(crate) fn collect_cycles(&mut self) -> usize {
        if !enabled() {
            return 0;
        }
        collect()
    }
}
