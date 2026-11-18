//! Memory probe: live and total counts of the object records, scopes and closures this thread created.
//! The counts are always kept (a thread-local `Cell` bump per allocation and free); `mem_trace_report`
//! prints them to stderr only when `THAW_JS_MEM_TRACE` is set.
//!
//! Thread-local rather than process-wide: a realm is confined to the thread that built it, and a
//! process-wide counter would race with every other realm in the same test binary. A value dropped
//! on another thread makes that thread's live count wrap below zero; the sum over threads still balances.
//!
//! The census (`census_closure`, `mem_trace_census_report`) is the closure-capture measurement behind the
//! leak note: it runs only when `THAW_JS_MEM_TRACE` is set. Its counters are cumulative; the cycle figure is
//! taken from the registry at report time.
//!
//! `reg_obj`/`reg_scope` in the report line are the `gc_registry` counts (see that module).
use crate::{Kind, Obj, ObjData, Scope, Slot, Value};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

struct Tally {
    live: Cell<usize>,
    total: Cell<usize>,
}

impl Tally {
    const fn new() -> Tally {
        Tally {
            live: Cell::new(0),
            total: Cell::new(0),
        }
    }

    fn created(&self) {
        self.live.set(self.live.get().wrapping_add(1));
        self.total.set(self.total.get().wrapping_add(1));
    }

    fn dropped(&self) {
        self.live.set(self.live.get().wrapping_sub(1));
    }
}

thread_local! {
    static OBJ: Tally = const { Tally::new() };
    static SCOPE: Tally = const { Tally::new() };
    static CLOSURE: Tally = const { Tally::new() };
}

/// An `ObjData` was created; `closure` when it is a function object (`Kind::Closure`).
pub(crate) fn obj_created(closure: bool) {
    OBJ.with(Tally::created);
    if closure {
        CLOSURE.with(Tally::created);
    }
}

/// An `ObjData` was dropped; pass the `closure` flag it was created with.
pub(crate) fn obj_dropped(closure: bool) {
    OBJ.with(Tally::dropped);
    if closure {
        CLOSURE.with(Tally::dropped);
    }
}

pub(crate) fn scope_created() {
    SCOPE.with(Tally::created);
}

pub(crate) fn scope_dropped() {
    SCOPE.with(Tally::dropped);
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn heap_in_use() -> usize {
    // SAFETY: mallinfo2 only reads the allocator's counters.
    unsafe { libc::mallinfo2() }.uordblks
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn heap_in_use() -> usize {
    0
}

/// The report line for `label`, without printing it.
pub fn mem_trace_line(label: &str) -> String {
    let (o, s, c) = (
        OBJ.with(|t| (t.live.get(), t.total.get())),
        SCOPE.with(|t| (t.live.get(), t.total.get())),
        CLOSURE.with(|t| (t.live.get(), t.total.get())),
    );
    let (reg_obj, reg_scope) = crate::gc_registry::counts();
    format!(
        "thaw-js mem-trace {label}: obj live={} total={} scope live={} total={} closure live={} total={} rss_kb={} heap_in_use={} reg_obj={reg_obj} reg_scope={reg_scope}",
        o.0 as isize,
        o.1,
        s.0 as isize,
        s.1,
        c.0 as isize,
        c.1,
        crate::eventloop::process_rss_bytes() / 1024,
        heap_in_use(),
    )
}

/// Prints `mem_trace_line(label)` to stderr when `THAW_JS_MEM_TRACE` is set; does nothing otherwise.
pub fn mem_trace_report(label: &str) {
    if std::env::var_os("THAW_JS_MEM_TRACE").is_some() {
        eprintln!("{}", mem_trace_line(label));
    }
}

/// Census state: `on` is 0 until the first closure is created on this thread, then 1 (off) or 2 (on).
struct Census {
    on: Cell<u8>,
    closures: Cell<usize>,
    /// Closures whose captured scopes (root excluded) bind nothing.
    chain_empty: Cell<usize>,
    /// ... whose captured bindings are all named by the closure's free names.
    used_only: Cell<usize>,
    /// ... whose captured bindings are none of the free names.
    unused_only: Cell<usize>,
    /// ... with both kinds.
    mixed: Cell<usize>,
    chain_bindings: Cell<usize>,
    chain_unused: Cell<usize>,
}

thread_local! {
    static CENSUS: Census = const {
        Census {
            on: Cell::new(0),
            closures: Cell::new(0),
            chain_empty: Cell::new(0),
            used_only: Cell::new(0),
            unused_only: Cell::new(0),
            mixed: Cell::new(0),
            chain_bindings: Cell::new(0),
            chain_unused: Cell::new(0),
        }
    };
}

/// Whether the census is on (`THAW_JS_MEM_TRACE` set). The environment is read once per thread.
fn census_on() -> bool {
    CENSUS.with(|c| match c.on.get() {
        0 => {
            let on = std::env::var_os("THAW_JS_MEM_TRACE").is_some();
            c.on.set(if on { 2 } else { 1 });
            on
        }
        v => v == 2,
    })
}

/// Census at closure creation: how the captured scope chain relates to the names the closure's body uses.
/// The chain is every scope from `scope` up to, not including, the root (the global scope is not a leak
/// source). `free` is the analyzer's free-name list for `def`, which is sorted.
///
/// ponytail: the analysis runs again for every closure created while the census is on (debug-only cost);
/// cache the free names per `FuncDef` if a corpus run is too slow with it.
pub(crate) fn census_closure(def: &crate::FuncDef, scope: &Rc<Scope>) {
    if !census_on() {
        return;
    }
    let free = crate::compile::analyze(def).free;
    let (mut bindings, mut unused) = (0usize, 0usize);
    let mut cur = Some(scope);
    while let Some(s) = cur {
        let Some(parent) = s.parent.as_ref() else {
            break;
        };
        if let Ok(vars) = s.vars.try_borrow() {
            for (name, _) in vars.list.iter() {
                bindings += 1;
                if free.binary_search_by(|f| f.as_str().cmp(&**name)).is_err() {
                    unused += 1;
                }
            }
        }
        cur = Some(parent);
    }
    CENSUS.with(|c| {
        c.closures.set(c.closures.get() + 1);
        c.chain_bindings.set(c.chain_bindings.get() + bindings);
        c.chain_unused.set(c.chain_unused.get() + unused);
        let bucket = if bindings == 0 {
            &c.chain_empty
        } else if unused == 0 {
            &c.used_only
        } else if unused == bindings {
            &c.unused_only
        } else {
            &c.mixed
        };
        bucket.set(bucket.get() + 1);
    });
}

/// The objects `o` points at through its own data properties (accessors and lazy prototypes are not followed).
/// The objects `o` points at: its own data properties, its array elements, and, for a closure, the object
/// bindings of its captured scope chain (root excluded, as in `census_closure`). Those last edges are what
/// make `o.f = function () { return o }` a cycle.
fn children(o: &ObjData) -> Vec<Obj> {
    let mut out = Vec::new();
    if let Ok(props) = o.raw_props.try_borrow() {
        out.extend(props.iter().filter_map(|p| match &p.slot {
            Slot::Data(Value::Object(c)) => Some(c.clone()),
            _ => None,
        }));
    }
    if let Ok(kind) = o.kind.try_borrow() {
        match &*kind {
            Kind::Array(items) => out.extend(items.iter().filter_map(|v| match v {
                Value::Object(c) => Some(c.clone()),
                _ => None,
            })),
            Kind::Closure { scope, .. } => {
                let mut cur = Some(scope);
                while let Some(s) = cur {
                    let Some(parent) = s.parent.as_ref() else {
                        break;
                    };
                    if let Ok(vars) = s.vars.try_borrow() {
                        for (_, b) in vars.list.iter() {
                            if let Value::Object(h) = &b.value {
                                out.push(h.clone());
                            }
                        }
                    }
                    cur = Some(parent);
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `h` reaches itself through `children`.
fn reaches_itself(h: &Obj) -> bool {
    let target = Rc::as_ptr(h);
    let mut stack = children(h);
    let mut seen = HashSet::new();
    while let Some(c) = stack.pop() {
        if Rc::as_ptr(&c) == target {
            return true;
        }
        if seen.insert(Rc::as_ptr(&c)) {
            stack.extend(children(&c));
        }
    }
    false
}

/// Report-time census line (printed when `THAW_JS_MEM_TRACE` is set): the cumulative closure-creation counters,
/// then over the closures live now (from the registry): how many capture a scope binding an object that reaches
/// itself through `children` (`live_closures_holder_in_cycle`), split by whether that binding is a name
/// the closure's body uses (`..._via_used`) or only an unused one (`..._via_unused_only`).
pub(crate) fn census_line(label: &str) -> String {
    let [closures, empty, used, unused, mixed] = census_snapshot();
    let (bindings, unused_bindings) =
        CENSUS.with(|c| (c.chain_bindings.get(), c.chain_unused.get()));
    let (mut live_closures, mut in_cycle, mut via_used, mut via_unused_only) = (0usize, 0, 0, 0);
    let mut cycle_memo: HashMap<*const ObjData, bool> = HashMap::new();
    let mut free_memo: HashMap<*const crate::FuncDef, Vec<String>> = HashMap::new();
    for o in crate::gc_registry::live_objs() {
        let kind = o.kind.borrow();
        let Kind::Closure { def, scope } = &*kind else {
            continue;
        };
        live_closures += 1;
        let free = free_memo
            .entry(Rc::as_ptr(def))
            .or_insert_with(|| crate::compile::analyze(def).free);
        let (mut hit_used, mut hit_unused) = (false, false);
        let mut cur = Some(scope);
        while let Some(s) = cur {
            let Some(parent) = s.parent.as_ref() else {
                break;
            };
            if let Ok(vars) = s.vars.try_borrow() {
                for (name, b) in vars.list.iter() {
                    if let Value::Object(h) = &b.value {
                        let cyc = *cycle_memo
                            .entry(Rc::as_ptr(h))
                            .or_insert_with(|| reaches_itself(h));
                        if cyc {
                            if free.binary_search_by(|f| f.as_str().cmp(&**name)).is_ok() {
                                hit_used = true;
                            } else {
                                hit_unused = true;
                            }
                        }
                    }
                }
            }
            cur = Some(parent);
        }
        in_cycle += usize::from(hit_used || hit_unused);
        via_used += usize::from(hit_used);
        via_unused_only += usize::from(hit_unused && !hit_used);
    }
    format!(
        "thaw-js mem-trace {label}: closures_created={closures} chain_empty={empty} used_only={used} unused_only={unused} mixed={mixed} chain_bindings={bindings} chain_bindings_unused={unused_bindings} live_closures={live_closures} live_closures_holder_in_cycle={in_cycle} live_closures_cycle_via_used={via_used} live_closures_cycle_via_unused_only={via_unused_only}"
    )
}

/// Prints `census_line(label)` to stderr when `THAW_JS_MEM_TRACE` is set; does nothing otherwise.
pub(crate) fn mem_trace_census_report(label: &str) {
    if census_on() {
        eprintln!("{}", census_line(label));
    }
}

/// Thread-local counts `(obj live, scope live, closure live)`, for tests.
#[cfg(test)]
pub(crate) fn live_counts() -> (isize, isize, isize) {
    (
        OBJ.with(|t| t.live.get() as isize),
        SCOPE.with(|t| t.live.get() as isize),
        CLOSURE.with(|t| t.live.get() as isize),
    )
}

/// Turns the census on or off for this thread (the environment is not read), for tests.
#[cfg(test)]
pub(crate) fn census_for_tests(on: bool) {
    CENSUS.with(|c| c.on.set(if on { 2 } else { 1 }));
}

/// Census closure counters `[closures, chain_empty, used_only, unused_only, mixed]`.
pub(crate) fn census_snapshot() -> [usize; 5] {
    CENSUS.with(|c| {
        [
            c.closures.get(),
            c.chain_empty.get(),
            c.used_only.get(),
            c.unused_only.get(),
            c.mixed.get(),
        ]
    })
}

/// Thread-local total closures created so far, for tests.
#[cfg(test)]
pub(crate) fn closure_total() -> usize {
    CLOSURE.with(|t| t.total.get())
}
