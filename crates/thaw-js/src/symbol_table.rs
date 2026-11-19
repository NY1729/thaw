//! Symbol identity for one OS thread. Every Interp on the thread shares these tables: a registry package
//! or subpath is its own Interp, and objects cross between them as the same Rc. A symbol-keyed property is
//! stored under `@@sym:<id>`, so an id must name the same symbol in every Interp that reads the property.
//! Objects cross OS threads only by structured clone, which ignores symbols, so each thread keeps its own
//! tables.
//!
//! Symbols are never freed on this thread: `BY_ID` holds each one strongly, since `getOwnPropertySymbols`
//! must still return a symbol whose key an object keeps. That is one record per `Symbol()` call for the life
//! of the thread. ponytail: weak entries would drop a symbol an object still keys by, add a key-side table
//! when this record growth matters.

use crate::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// The well-known symbol names, in the order `Symbol` exposes them.
pub(crate) const WELL_KNOWN_NAMES: [&str; 15] = [
    "iterator",
    "asyncIterator",
    "hasInstance",
    "toPrimitive",
    "toStringTag",
    "species",
    "isConcatSpreadable",
    "match",
    "matchAll",
    "replace",
    "search",
    "split",
    "unscopables",
    "dispose",
    "asyncDispose",
];

thread_local! {
    static LAST_ID: Cell<u64> = const { Cell::new(0) };
    /// Every symbol by id, so an id from any Interp of this thread resolves to its symbol.
    static BY_ID: RefCell<HashMap<u64, Obj>> = RefCell::new(HashMap::new());
    /// The well-known symbols, made once per thread on first use.
    static WELL_KNOWN: RefCell<Vec<Obj>> = const { RefCell::new(Vec::new()) };
    /// Prototype the well-known symbols were made with (the first realm's `Symbol.prototype`).
    static WELL_KNOWN_PROTO: RefCell<Option<Obj>> = const { RefCell::new(None) };
    /// The `Symbol.for` registry of the host realms on this thread.
    static REGISTRY: Obj = new_obj(None, Kind::Plain);
}

/// A new symbol with `desc`, under a fresh id of this thread: `(id, symbol)`.
pub(crate) fn new_symbol(proto: Obj, desc: Option<Rc<str>>) -> (u64, Obj) {
    let id = LAST_ID.with(|last| {
        let id = last.get() + 1;
        last.set(id);
        id
    });
    let o = new_obj(
        Some(proto),
        Kind::Symbol {
            id,
            desc: desc.clone(),
        },
    );
    put_hidden(
        &o,
        "description",
        desc.map_or(Value::Undefined, |d| Value::Str(d.into())),
    );
    BY_ID.with(|m| m.borrow_mut().insert(id, o.clone()));
    (id, o)
}

/// The symbol an id names, from any Interp of this thread.
/// `try_with`: teardown may run while the thread's locals are destroyed (see `shared_proto`).
pub(crate) fn by_id(id: u64) -> Option<Obj> {
    BY_ID
        .try_with(|m| m.borrow().get(&id).cloned())
        .ok()
        .flatten()
}

/// The well-known symbols in `WELL_KNOWN_NAMES` order, made on first call with `proto` as their prototype.
pub(crate) fn well_known(proto: &Obj) -> Vec<Obj> {
    WELL_KNOWN.with(|w| {
        let mut w = w.borrow_mut();
        if w.is_empty() {
            WELL_KNOWN_PROTO.with(|p| *p.borrow_mut() = Some(proto.clone()));
            for name in WELL_KNOWN_NAMES {
                let (_, o) = new_symbol(proto.clone(), Some(format!("Symbol.{name}").into()));
                w.push(o);
            }
        }
        w.clone()
    })
}

/// Whether `sym` is one of the well-known symbols of this thread.
pub(crate) fn is_well_known(sym: &Obj) -> bool {
    WELL_KNOWN.with(|w| w.borrow().iter().any(|s| Rc::ptr_eq(s, sym)))
}

/// The prototype the shared symbols inherit from. Teardown leaves it alone, since symbols use it.
/// `try_with`: an Interp dropped while the thread's locals are being destroyed must not panic here.
pub(crate) fn shared_proto() -> Option<Obj> {
    WELL_KNOWN_PROTO
        .try_with(|p| p.borrow().clone())
        .ok()
        .flatten()
}

/// The `Symbol.for` registry of this thread's host realms.
pub(crate) fn registry() -> Obj {
    REGISTRY.with(|r| r.clone())
}

/// The key `sym` is registered under in `registry`, if any.
pub(crate) fn key_in_registry(registry: &Obj, sym: &Obj) -> Option<Rc<str>> {
    registry
        .raw_props
        .borrow()
        .iter()
        .find_map(|p| match &p.slot {
            Slot::Data(Value::Object(o)) if Rc::ptr_eq(o, sym) => Some(p.key.clone()),
            _ => None,
        })
}
