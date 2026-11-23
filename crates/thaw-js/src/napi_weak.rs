//! Real weakness for the JS objects handed to a native addon (the N-API finalizer lane).
//!
//! The proxy layer gives each non-function object it sends to native code a reference id. The native side copies
//! the object and may attach `napi_add_finalizer` records to the copy; they must run when the JS object is
//! collected (the retired-engine lane does this with a FinalizationRegistry). Only these objects are weak: `watch` flags
//! the object, `ObjData::drop` queues its id, and `drain` (at `__thaw_gc` and at an event-loop turn) tells the
//! bridge to `release` it. `WeakRef`/`FinalizationRegistry` stay strong for ordinary programs.
//!
//! Thread safety: the queue is thread-local and `Drop` only pushes an id (no JS, no bridge call). The collector and
//! the weak mode need the costack backend (one OS thread per realm); with the thread coroutine backend an object
//! may be dropped on another OS thread, so there `watch` keeps the object strongly, as before.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::{Obj, ObjData, Value};

enum Held {
    Weak(Weak<ObjData>),
    Strong(Obj),
}

#[derive(Default)]
struct State {
    /// Object address -> reference id, for the flagged objects.
    ids: HashMap<usize, u64>,
    held: HashMap<u64, Held>,
    dead: Vec<u64>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn key(o: &Obj) -> usize {
    Rc::as_ptr(o) as usize
}

/// Flags `o` as the object of reference `id`.
pub(crate) fn watch(o: &Obj, id: u64) {
    let weak = crate::gc::enabled();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.ids.insert(key(o), id);
        s.held.insert(
            id,
            if weak {
                Held::Weak(Rc::downgrade(o))
            } else {
                Held::Strong(o.clone())
            },
        );
    });
    if weak {
        o.set_watched();
    }
}

/// The reference id of `o`, if flagged.
pub(crate) fn id_of(o: &Obj) -> Option<u64> {
    STATE.with(|s| s.borrow().ids.get(&key(o)).copied())
}

/// The live object of reference `id`.
pub(crate) fn value_of(id: u64) -> Value {
    STATE.with(|s| match s.borrow().held.get(&id) {
        Some(Held::Strong(o)) => Value::Object(o.clone()),
        Some(Held::Weak(w)) => w.upgrade().map_or(Value::Undefined, Value::Object),
        None => Value::Undefined,
    })
}

/// Called from `ObjData::drop` of a flagged object (`addr` is its address): queues its id. Never runs JS.
pub(crate) fn died(addr: usize) {
    let _ = STATE.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(id) = s.ids.remove(&addr) {
                s.held.remove(&id);
                s.dead.push(id);
            }
        }
    });
}

/// Releases every collected object's reference on the bridge, so its native finalizers run.
pub(crate) fn drain() {
    let dead = STATE
        .try_with(|s| s.try_borrow_mut().map(|mut s| std::mem::take(&mut s.dead)))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    for id in dead {
        let _ = crate::napi_bridge::handle_op("release", &id.to_string(), "", "[]");
    }
}
