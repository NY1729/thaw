//! Memory probe: live and total counts of the object records, scopes and closures this thread created.
//! The counts are always kept (a thread-local `Cell` bump per allocation and free); `mem_trace_report`
//! prints them to stderr only when `THAW_JS_MEM_TRACE` is set.
//!
//! Thread-local rather than process-wide: a realm is confined to the thread that built it, and a
//! process-wide counter would race with every other realm in the same test binary. A value dropped
//! on another thread makes that thread's live count wrap below zero; the sum over threads still balances.
use std::cell::Cell;

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
    format!(
        "thaw-js mem-trace {label}: obj live={} total={} scope live={} total={} closure live={} total={} rss_kb={} heap_in_use={}",
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

/// Thread-local counts `(obj live, scope live, closure live)`, for tests.
#[cfg(test)]
pub(crate) fn live_counts() -> (isize, isize, isize) {
    (
        OBJ.with(|t| t.live.get() as isize),
        SCOPE.with(|t| t.live.get() as isize),
        CLOSURE.with(|t| t.live.get() as isize),
    )
}

/// Thread-local total closures created so far, for tests.
#[cfg(test)]
pub(crate) fn closure_total() -> usize {
    CLOSURE.with(|t| t.total.get())
}
