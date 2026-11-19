//! Worker `resourceLimits` (`maxOldGenerationSizeMb`): an interpreter byte count of what a worker thread's JS keeps
//! live, checked against the limit. It is not V8's heap.
//!
//! ponytail: approximation. The threshold is this estimate, not V8's old-generation size (a `Value` is larger than a
//! V8 slot, so the same program trips at a different point). Counted: each object record and its array and property
//! slot tables (`ObjData::charged`, growth at `push_prop`, the array mutators and indexed stores), and ArrayBuffer and
//! SharedArrayBuffer bytes (`Store::charged`). Not counted: strings, closures and scopes, Map and Set tables, array
//! growth made by other natives (Reflect, process events, napi), and data held by natives (zlib). A runaway made of
//! those reaches the system OOM killer as before. `maxYoungGenerationSizeMb` and `codeRangeSizeMb` are accepted with
//! no effect.
//!
//! The state is per OS thread. A coroutine thread (generator, async function) takes it over with the other host
//! tables on each control handoff (`host::park_host_tls` / `adopt_host_tls`), so one worker's count follows its JS.
//! With no limit the state stays off and every charge and credit is one thread-local bool read.

use std::cell::Cell;

/// The `Ctl::Exit` code of a worker that tripped its limit (worker.rs maps it to `ERR_WORKER_OUT_OF_MEMORY`).
pub(crate) const OOM_EXIT: i32 = 134;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static USED: Cell<usize> = const { Cell::new(0) };
    static LIMIT: Cell<usize> = const { Cell::new(usize::MAX) };
    static TRIPPED: Cell<bool> = const { Cell::new(false) };
}

/// The accounting of one thread, moved between threads at a coroutine handoff.
#[derive(Default)]
pub(crate) struct State {
    on: bool,
    used: usize,
    limit: usize,
    tripped: bool,
}

/// Turns the accounting on for this thread with `limit_bytes` (a worker thread, once, at its start).
pub(crate) fn enable(limit_bytes: usize) {
    ON.with(|c| c.set(true));
    LIMIT.with(|c| c.set(limit_bytes));
}

/// Turns the accounting off and clears its state for this thread.
pub(crate) fn reset() {
    ON.with(|c| c.set(false));
    USED.with(|c| c.set(0));
    LIMIT.with(|c| c.set(usize::MAX));
    TRIPPED.with(|c| c.set(false));
}

pub(crate) fn enabled() -> bool {
    ON.with(Cell::get)
}

/// The limit in bytes this thread was enabled with (`usize::MAX` when off).
pub(crate) fn limit() -> usize {
    LIMIT.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn used() -> usize {
    USED.with(Cell::get)
}

/// True once the count went over the limit on this thread; the interpreter stops at its next interrupt check.
#[inline]
pub(crate) fn tripped() -> bool {
    TRIPPED.with(Cell::get)
}

/// Adds `n` bytes to the count (a no-op unless enabled). Returns the bytes counted: `n`, or 0 when off, so a caller
/// that keeps the figure to credit later credits exactly what was added.
#[inline]
pub(crate) fn charge(n: usize) -> usize {
    if !ON.with(Cell::get) {
        return 0;
    }
    let used = USED.with(Cell::get).saturating_add(n);
    USED.with(|c| c.set(used));
    if used > LIMIT.with(Cell::get) {
        TRIPPED.with(|c| c.set(true));
    }
    n
}

/// Removes `n` bytes that a charge added (a no-op unless enabled).
#[inline]
pub(crate) fn credit(n: usize) {
    if ON.with(Cell::get) {
        USED.with(|c| c.set(c.get().saturating_sub(n)));
    }
}

/// Takes this thread's accounting away (it is reset here), for the thread that takes over the JS.
pub(crate) fn take() -> State {
    let state = State {
        on: ON.with(Cell::get),
        used: USED.with(Cell::get),
        limit: LIMIT.with(Cell::get),
        tripped: TRIPPED.with(Cell::get),
    };
    reset();
    state
}

/// Gives this thread the accounting `state` was taken with.
pub(crate) fn restore(state: State) {
    ON.with(|c| c.set(state.on));
    USED.with(|c| c.set(state.used));
    LIMIT.with(|c| c.set(state.limit));
    TRIPPED.with(|c| c.set(state.tripped));
}
