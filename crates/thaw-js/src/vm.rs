//! `node:vm` contexts: the `__thaw_vm_*` natives behind `registry/builtins/system/vm_context.js`.
//! A context is a separate `Interp` (its own intrinsics and global scope), built on its first run
//! so that contexts which are only created cost nothing.

use std::rc::Rc;

use crate::{arg, Interp, Value};

/// Host globals a context must not see (`typeof` gives `undefined`; a bare read is `undefined` too).
const HOST_ONLY: &[&str] = &[
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "clearImmediate",
    "queueMicrotask",
    "process",
    "Buffer",
    "require",
    "module",
    "exports",
    "fetch",
    "URL",
    "URLSearchParams",
    "TextEncoder",
    "TextDecoder",
    "structuredClone",
    "AbortController",
    "EventTarget",
    "Event",
    "performance",
];

/// Globals every context shares with the host instead of building its own.
const SHARED: &[&str] = &["Intl", "WebAssembly", "SharedArrayBuffer", "Atomics"];

/// A fresh context realm: its own ECMAScript intrinsics, with the host-only globals hidden.
fn new_realm(id: u32, host: &Interp) -> Box<Interp> {
    let mut sub = Box::new(Interp::new());
    // symbols made in this realm get keys no other realm produces
    sub.next_symbol += u64::from(id) << 32;
    let internal: Vec<Rc<str>> = sub
        .global
        .vars
        .borrow()
        .keys()
        .filter(|k| k.starts_with("__thaw_"))
        .cloned()
        .collect();
    for name in internal
        .iter()
        .map(|k| &**k)
        .chain(HOST_ONLY.iter().copied())
    {
        sub.global.declare(name, Value::Undefined, true);
    }
    for name in SHARED {
        if let Some(v) = host.lookup(&host.global, name) {
            sub.global.declare(name, v, true);
        }
    }
    let global = Value::Object(sub.global_object.clone());
    sub.global.declare("this", global, false);
    sub
}
/// One context: `realm` is `None` until the first run, and taken out while the context runs.
pub(crate) struct VmSlot {
    realm: Option<Box<Interp>>,
    busy: bool,
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    let create = it.native(|it, _, _| {
        it.vm_next += 1;
        let id = it.vm_next;
        it.vm.insert(
            id,
            VmSlot {
                realm: None,
                busy: false,
            },
        );
        Ok(Value::Number(f64::from(id)))
    });
    g.declare("__thaw_vm_create", create, true);
    let free = it.native(|it, _, a| {
        let id = it.num(&arg(a, 0)) as u32;
        it.vm.remove(&id);
        Ok(Value::Undefined)
    });
    g.declare("__thaw_vm_free", free, true);
    // (id, code, filename, timeoutMs, sandbox, stop); the filename is only for stack traces (ponytail)
    let run = it.native(|it, _, a| {
        let id = it.num(&arg(a, 0)) as i64;
        let code = it.to_str(&arg(a, 1)).to_string();
        if id < 0 {
            // runInThisContext: indirect-eval semantics in this realm
            return it.run_sloppy(&code);
        }
        if !matches!(arg(a, 4), Value::Undefined) {
            return it.throw_error("Error", "vm sandboxes are not supported yet");
        }
        let id = id as u32;
        let Some(slot) = it.vm.get_mut(&id) else {
            return it.throw_error("Error", "the vm context was released");
        };
        if slot.busy {
            return it.throw_error("Error", "the vm context is busy");
        }
        slot.busy = true;
        let mut sub = match slot.realm.take() {
            Some(sub) => sub,
            None => new_realm(id, it),
        };
        // stack limits and depth follow the calling thread
        sub.restore(it.snapshot());
        let r = sub.run_sloppy(&code);
        if let Some(slot) = it.vm.get_mut(&id) {
            slot.realm = Some(sub);
            slot.busy = false;
        }
        r
    });
    g.declare("__thaw_vm_run", run, true);
}
