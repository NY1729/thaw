//! Cycle collector: "subtract the internal references" (CPython's gc). `collect_cycles` is the collection; the trigger
//! (`gc_safe_point`, PR-2) runs it when a registry list's live count reaches its threshold.
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
//! Opaque nodes are never cleared and always count as roots: a node whose cell is borrowed, a `Proxy` whose data is
//! shared, a started and unfinished generator (its coroutine frames hold strong `Rc`s), and the global object. Their
//! own edges are not subtracted, so anything they hold stays externally referenced.

use crate::eventloop::PState;
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
/// the prototype, and the kind's objects: array items, wrapped value, closure scope (plus the class's field scope and
/// parent when the constructor sole-owns its `FuncDef`), Map keys and values, Set items, a collection iterator's
/// collection, a typed array's buffer, a proxy's target and handler (when its `ProxyData` is sole-owned). A
/// `Slot::LazyProto` owner is a `Weak` and is not an edge; its parent is. Returns false for an opaque object (a cell
/// is borrowed, a shared proxy, or a kind whose edges are not enumerated yet): then `f` was not called at all.
pub(crate) fn obj_edges(o: &ObjData, f: &mut dyn FnMut(Edge)) -> bool {
    // Read `raw_props`, never `props()`: materialising a lazy `prototype` is a mutation.
    let (Ok(props), Ok(proto), Ok(kind), Ok(sparse)) = (
        o.raw_props.try_borrow(),
        o.proto.try_borrow(),
        o.kind.try_borrow(),
        o.sparse
            .get()
            .map_or(Ok(None), |c| c.try_borrow().map(Some)),
    ) else {
        return false;
    };
    // A proxy's handler sits in its own cell: borrow it here so the edges it yields live as long as `buf`.
    let proxy_handler = match &*kind {
        Kind::Proxy(p) => match p.handler.try_borrow() {
            Ok(h) => Some(h),
            Err(_) => return false,
        },
        _ => None,
    };
    // A generator's cells are read only while it is not started or already finished. A started, unfinished generator
    // owns a coroutine whose frames hold strong `Rc`s (roots by deduction): it is opaque, so it is never cleared.
    // The guards live here, before `buf`, so the edges borrowed from them stay valid.
    let gen_cells = match &*kind {
        Kind::Generator(g) => {
            if Rc::strong_count(g) != 1 {
                return false;
            }
            let (Ok(start), Ok(co), Ok(queue)) = (
                g.start.try_borrow(),
                g.co.try_borrow(),
                g.queue.try_borrow(),
            ) else {
                return false;
            };
            if co.is_some() {
                return false;
            }
            Some((start, queue))
        }
        _ => None,
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
            if let Some(s) = sparse.as_ref() {
                for e in s.elems.values() {
                    match &e.slot {
                        Slot::Data(v) => value_edge(&mut buf, v),
                        Slot::Accessor { get, set } => {
                            for v in [get, set].into_iter().flatten() {
                                value_edge(&mut buf, v);
                            }
                        }
                        Slot::LazyProto { .. } => {}
                    }
                }
            }
        }
        Kind::Closure { def, scope } => {
            buf.push(Edge::Scope(scope));
            // The class's field scope and parent live in `FuncDef.class`. Only the constructor holds that `FuncDef` when
            // its count is 1; any other holder (a call in flight, `call_frames`, a generator's `def`) makes it 2 or more,
            // and then these two edges are not counted, so their targets keep an external count (the safe direction).
            if let Some(ci) = def.class.as_ref().filter(|_| Rc::strong_count(def) == 1) {
                buf.push(Edge::Scope(&ci.field_scope));
                if let Some(p) = &ci.parent {
                    value_edge(&mut buf, p);
                }
            }
        }
        Kind::Wrapper(v) => value_edge(&mut buf, v),
        // Entries are held by the `MapData`/`SetData` inline in the kind (no other clone): one edge per Object key and
        // per Object value. The lookup index holds addresses only, and cursors are `Weak`.
        Kind::Map(m) => {
            for (k, v) in &m.entries {
                value_edge(&mut buf, k);
                value_edge(&mut buf, v);
            }
        }
        Kind::Set(s) => {
            for v in &s.items {
                value_edge(&mut buf, v);
            }
        }
        // A live iterator's collection (`cursor` is a bare `Rc<Cell>`, not a node).
        Kind::CollIter(c) => buf.push(Edge::Obj(&c.src)),
        Kind::Typed(t) => buf.push(Edge::Obj(&t.abuf)),
        Kind::Proxy(p) => {
            // A `ProxyData` with another owner (a trap in flight) is opaque: its target and handler keep their counts.
            if Rc::strong_count(p) != 1 {
                return false;
            }
            value_edge(&mut buf, &p.target);
            if let Some(h) = proxy_handler.as_deref() {
                value_edge(&mut buf, h);
            }
        }
        // A promise's state and each reaction's callbacks, derived promise and async-context frame: one clone each
        // (the reaction and the state are moved in once, `settle` moves them out to jobs without cloning).
        // Reactions held by a queued job or a timer are not here: those holders are strong `Rc`s, roots by deduction.
        Kind::Promise(d) => {
            if let PState::Fulfilled(v) | PState::Rejected(v) = &d.state {
                value_edge(&mut buf, v);
            }
            for r in &d.reactions {
                value_edge(&mut buf, &r.on_ful);
                value_edge(&mut buf, &r.on_rej);
                if let Some(o) = &r.result {
                    buf.push(Edge::Obj(o));
                }
                if let Some(f) = &r.frame {
                    value_edge(&mut buf, f);
                }
            }
        }
        // Only the cells of `gen_cells` (a sole-owned, not-running generator): its unstarted function and its bound
        // frame (`this`, `arguments` and the parameters are its bindings; `def` is no node) and its queued requests
        // (the value and the promise).
        Kind::Generator(_) => {
            if let Some((start, queue)) = gen_cells.as_ref() {
                if let Some(st) = start.as_ref() {
                    value_edge(&mut buf, &st.f);
                    buf.push(Edge::Scope(&st.frame));
                }
                for (_, v, p) in queue.iter() {
                    value_edge(&mut buf, v);
                    buf.push(Edge::Obj(p));
                }
            }
        }
        Kind::Global => return false,
    }
    for e in buf {
        f(e);
    }
    true
}

/// Calls `f` once per `Rc` clone that `s` owns: the parent, the `with` object, the `this` value, and each binding's
/// value. Returns false, calling `f` for nothing, when the scope cannot be read. The collector clears only `vars` and
/// `this_val` (both mutable): a `with` scope is cleared like any other, because every cycle through one also runs
/// through a mutable edge (the closure or property that holds the scope).
pub(crate) fn scope_edges(s: &Scope, f: &mut dyn FnMut(Edge)) -> bool {
    let (Ok(vars), Ok(this)) = (s.vars.try_borrow(), s.this_val.try_borrow()) else {
        return false;
    };
    let mut buf = Vec::new();
    if let Some(p) = &s.parent {
        buf.push(Edge::Scope(p));
    }
    if let Some(w) = &s.with {
        buf.push(Edge::Obj(w));
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
#[allow(dead_code)] // the fields are never read: they are only dropped, after every borrow is released
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
                    .then(|| self.objs.len() + j)
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
                let (Ok(mut props), Ok(mut index), Ok(mut proto), Ok(mut kind), Ok(mut sparse)) = (
                    o.raw_props.try_borrow_mut(),
                    o.raw_props.index.try_borrow_mut(),
                    o.proto.try_borrow_mut(),
                    o.kind.try_borrow_mut(),
                    o.sparse
                        .get()
                        .map_or(Ok(None), |c| c.try_borrow_mut().map(Some)),
                ) else {
                    continue;
                };
                taken.push(Taken::Props(std::mem::take(&mut *props)));
                taken.push(Taken::Index(index.take()));
                if let Some(p) = proto.take() {
                    taken.push(Taken::Obj(p));
                }
                if let Some(s) = sparse.as_mut() {
                    taken.push(Taken::Values(
                        std::mem::take(&mut s.elems)
                            .into_values()
                            .flat_map(crate::Elem::into_values)
                            .collect(),
                    ));
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
    COLLECTIONS.with(|c| c.set(c.get() + 1));
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

/// The automatic trigger's floor: a collection is due once a registry list holds this many live nodes (then at
/// `max(floor, 2 x live)` after each attempt, so a growing live set collects O(log n) times).
const AUTO_FLOOR: usize = 15_000;

/// `THAW_JS_GC` and `THAW_JS_GC_STRESS`, read once per process.
fn env_policy() -> (bool, Option<u32>) {
    static ENV: std::sync::OnceLock<(bool, Option<u32>)> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| {
        parse_env(
            std::env::var("THAW_JS_GC").ok().as_deref(),
            std::env::var("THAW_JS_GC_STRESS").ok().as_deref(),
        )
    })
}

/// The environment rule, pure for the table test. Returns `(trigger on, stress every n)`. `THAW_JS_GC=0` turns the
/// trigger off. `THAW_JS_GC_STRESS=n` (n >= 1; anything else is ignored) turns it on regardless, and collects at
/// every n-th safe point that follows a registration.
pub(crate) fn parse_env(gc: Option<&str>, stress: Option<&str>) -> (bool, Option<u32>) {
    let stress = stress
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1);
    (stress.is_some() || gc != Some("0"), stress)
}

/// Whether the trigger arms on new realms: the collector is enabled and the environment does not turn it off.
pub(crate) fn trigger_on() -> bool {
    enabled() && env_policy().0
}

/// The stress interval, if `THAW_JS_GC_STRESS` set one. Tests override it in-process (`stress_for_test`).
fn stress_every() -> Option<u32> {
    #[cfg(test)]
    {
        if let Some(n) = STRESS_OVERRIDE.with(Cell::get) {
            return n;
        }
    }
    env_policy().1
}

/// Arms this thread's trigger. Called at the end of `Interp::build` on every thread (workers and vm realms included).
pub(crate) fn arm_for_thread() {
    if trigger_on() {
        let floor = if stress_every().is_some() {
            0
        } else {
            AUTO_FLOOR
        };
        crate::gc_registry::arm_default(floor);
    }
}

thread_local! {
    /// Set by `no_auto`: the safe points on this thread do not collect (explicit `collect_cycles` still does).
    static NO_AUTO: Cell<bool> = const { Cell::new(false) };
    /// Safe points reached since the last stress collection.
    static STRESS_TICK: Cell<u32> = const { Cell::new(0) };
    /// Collections that ran on this thread.
    static COLLECTIONS: Cell<usize> = const { Cell::new(0) };
    /// Largest registry live count (objects + scopes) seen at a safe point on this thread.
    static PEAK_LIVE: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Test override of the stress interval (`Some(None)` = off); `None` reads the environment.
    static STRESS_OVERRIDE: Cell<Option<Option<u32>>> = const { Cell::new(None) };
}

/// Opts this thread out of automatic collections. For tests that assert exact node counts; the explicit
/// `collect_cycles` (and `__gc`) still runs.
#[cfg(test)]
pub(crate) fn no_auto() {
    NO_AUTO.with(|c| c.set(true));
}

/// Undoes `no_auto` and sets the stress interval for this thread, then re-arms it (tests of the trigger itself).
#[cfg(test)]
pub(crate) fn stress_for_test(every: Option<u32>) {
    NO_AUTO.with(|c| c.set(false));
    STRESS_OVERRIDE.with(|c| c.set(Some(every)));
    crate::gc_registry::arm(if every.is_some() { 0 } else { AUTO_FLOOR });
}

/// Collections that ran on this thread.
#[cfg(test)]
pub(crate) fn collections() -> usize {
    COLLECTIONS.with(Cell::get)
}

/// The largest registry live count seen at a safe point on this thread.
#[cfg(test)]
pub(crate) fn peak_live() -> usize {
    PEAK_LIVE.with(Cell::get)
}

impl Interp {
    /// The trigger's safe point. Runs only when `gc_registry::due()` (a registration crossed the threshold); the
    /// caller is `check_deadline` (every call and loop back-edge) and the event loop, both at the interpreter's
    /// statement level (see `collect_cycles`). Re-arms the thresholds after every attempt. The collection itself
    /// is `collect_cycles`, which is a no-op when the collector is disabled.
    #[cold]
    #[inline(never)]
    pub(crate) fn gc_safe_point(&mut self) {
        crate::gc_registry::clear_due();
        let (objs, scopes) = crate::gc_registry::counts();
        PEAK_LIVE.with(|p| p.set(p.get().max(objs + scopes)));
        let collect = match stress_every() {
            Some(n) => STRESS_TICK.with(|c| {
                let tick = c.get().wrapping_add(1);
                c.set(tick);
                tick % n == 0
            }),
            None => true,
        };
        if collect && !NO_AUTO.with(Cell::get) {
            self.collect_cycles();
        }
        crate::gc_registry::rearm();
    }
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
