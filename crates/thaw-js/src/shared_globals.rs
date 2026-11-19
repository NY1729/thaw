//! Process-wide user writes to the global object. Each registry package runs in its own realm with its own
//! global scope, but Node runs one process: `Object.defineProperty(globalThis, 'Response', ...)` in one
//! package must be the `Response` every other package sees. The write-time mirror: after a successful user
//! write to a global name, `publish_global` stores the binding's value in a thread-local table and sets the
//! binding in every other live realm; a realm loaded later takes the table as its seed. Reads are untouched.
//! Only registry-package realms publish (`share_globals`); bare, vm and worker interpreters stay isolated.
//!
//! ponytail: accessor properties on `globalThis` are not mirrored (their closures are realm-local).
//! ponytail: the table holds values strongly for the thread, so a cycle through it is never collected.
//! ponytail: the table is scanned linearly per write; a hash map if a package publishes hundreds of names.

use std::cell::RefCell;
use std::rc::Rc;

use crate::host::for_other_realms;
use crate::{Interp, Origin, Value};

thread_local! {
    /// The published global bindings in first-write order.
    static SHARED: RefCell<Vec<(Rc<str>, Value)>> = const { RefCell::new(Vec::new()) };
}

/// Internal helpers (`__thaw_*`) are per realm: never published.
fn publishable(name: &str) -> bool {
    !name.starts_with("__thaw_")
}

/// Sets `name` in `it`'s global scope to `value`: an existing binding keeps its origin and flags, an absent
/// one becomes an implicit (configurable) global. A getter-only global and a lexical binding are left alone.
fn apply_shared(it: &mut Interp, name: &str, value: &Value) {
    if it.global_refuses_writes(name) {
        return;
    }
    let g = it.global.clone();
    let present = g.vars.borrow().get(name).map(|b| (b.origin, b.mutable));
    match present {
        Some((Origin::Lexical, _)) | Some((_, false)) => {}
        Some(_) => {
            if let Some(b) = g.vars.borrow_mut().get_mut(name) {
                b.value = value.clone();
            }
        }
        None => g.declare_with(name.into(), value.clone(), true, Origin::Prop),
    }
}

/// Removes `name` from `it`'s global scope unless it is a lexical binding (which `delete` never reaches).
fn remove_shared(it: &mut Interp, name: &str) {
    let g = it.global.clone();
    let lexical = g
        .vars
        .borrow()
        .get(name)
        .is_some_and(|b| b.origin == Origin::Lexical);
    if !lexical {
        g.vars.borrow_mut().remove(name);
    }
}

impl Interp {
    /// Makes this realm a registry-package realm: it publishes its global writes from now on, and the names
    /// already published by other realms are applied to it (call after its own prelude init).
    pub(crate) fn enable_shared_globals(&mut self) {
        self.share_globals = true;
        let entries = SHARED.with(|s| s.borrow().clone());
        for (name, value) in &entries {
            apply_shared(self, name, value);
        }
    }

    /// True when a write to the global `name` in this realm must be published, so an in-place fast path
    /// (which skips `assign_var`) must not handle it.
    pub(crate) fn global_write_publishes(&self, name: &str) -> bool {
        self.share_globals && self.global.vars.borrow().contains_key(name)
    }

    /// After a successful user write to the global `name` in this realm: mirror its value to every realm.
    pub(crate) fn publish_global(&mut self, name: &str) {
        if !self.share_globals || !publishable(name) {
            return;
        }
        let value = match self.global.vars.borrow().get(name) {
            Some(b) if b.origin != Origin::Lexical => b.value.clone(),
            _ => return,
        };
        SHARED.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(i) = s.iter().position(|(n, _)| &**n == name) {
                s[i].1 = value.clone();
            } else {
                s.push((name.into(), value.clone()));
            }
        });
        for_other_realms(self, |other| apply_shared(other, name, &value));
    }

    /// After `delete` removed the global `name` from this realm: remove it from every realm.
    pub(crate) fn unpublish_global(&mut self, name: &str) {
        if !self.share_globals || !publishable(name) {
            return;
        }
        SHARED.with(|s| s.borrow_mut().retain(|(n, _)| &**n != name));
        for_other_realms(self, |other| remove_shared(other, name));
    }
}
