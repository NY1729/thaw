//! `node:vm` contexts: the `__thaw_vm_*` natives behind `registry/builtins/system/vm_context.js`.
//! A context is a separate `Interp` (its own intrinsics and global scope), built on its first run
//! so that contexts which are only created cost nothing.

use std::rc::Rc;

use crate::{arg, find_own, Interp, Obj, Scope, Slot, Value, R};

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
/// A context's realm plus what persists between runs of a sandboxed context.
struct VmRealm {
    interp: Box<Interp>,
    /// Scope `scope -> with(sandbox) -> global` holding the context's var/let/const/function
    /// bindings; built on the first run with a sandbox.
    script: Option<Rc<Scope>>,
    /// Global bindings at index >= `n0` were made by sandboxed runs (implicit globals).
    n0: usize,
    /// Var and function names of earlier runs, and implicit globals already copied to the sandbox.
    vars: Vec<Rc<str>>,
}

/// One context: `realm` is `None` until the first run, and taken out while the context runs.
pub(crate) struct VmSlot {
    realm: Option<Box<VmRealm>>,
    busy: bool,
}

/// Runs `code` with `sb` as the sandbox: before the run, vars the host changed on `sb` are read
/// back in; after it (also after a throw), vars, functions and implicit globals are copied out to `sb`.
fn run_in_sandbox(it: &mut Interp, realm: &mut VmRealm, sb: &Obj, code: &str) -> R<Value> {
    let script = match &realm.script {
        Some(script) => script.clone(),
        None => {
            let with = Scope::new_with(realm.interp.global.clone(), sb.clone());
            let script = Scope::new(Some(with));
            realm.n0 = realm.interp.global.vars.borrow().list.len();
            realm.script = Some(script.clone());
            script
        }
    };
    for name in &realm.vars {
        if let Some((Slot::Data(v), _)) = find_own(sb, name) {
            if let Some(b) = script.vars.borrow_mut().get_mut(name) {
                b.value = v;
            }
        }
    }
    let (r, names) = realm.interp.run_vm_script(code, &script, sb);
    let mut out: Vec<(Rc<str>, Value)> = Vec::new();
    {
        let vars = script.vars.borrow();
        for name in names.iter().chain(&realm.vars) {
            if let Some(b) = vars.get(name) {
                out.push((name.clone(), b.value.clone()));
            }
        }
    }
    let implicit: Vec<(Rc<str>, Value)> = realm.interp.global.vars.borrow().list[realm.n0..]
        .iter()
        .filter(|(name, _)| !realm.vars.contains(name) && !names.contains(name))
        .map(|(name, b)| (name.clone(), b.value.clone()))
        .collect();
    for name in names.iter() {
        if !realm.vars.contains(name) {
            realm.vars.push(name.clone());
        }
    }
    for (name, _) in &implicit {
        realm.vars.push(name.clone());
    }
    let target = Value::Object(sb.clone());
    let mut synced: R<()> = Ok(());
    for (name, value) in out.into_iter().chain(implicit) {
        if synced.is_ok() {
            synced = it.set(&target, &name, value);
        }
    }
    r.and_then(|v| synced.map(|()| v))
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
        let sandbox = match arg(a, 4) {
            Value::Object(sb) => Some(sb),
            Value::Undefined => None,
            _ => return it.throw_error("TypeError", "vm sandbox must be an object"),
        };
        let id = id as u32;
        let Some(slot) = it.vm.get_mut(&id) else {
            return it.throw_error("Error", "the vm context was released");
        };
        if slot.busy {
            return it.throw_error("Error", "the vm context is busy");
        }
        slot.busy = true;
        let taken = slot.realm.take();
        let mut realm = match taken {
            Some(realm) => realm,
            None => Box::new(VmRealm {
                interp: new_realm(id, it),
                script: None,
                n0: 0,
                vars: Vec::new(),
            }),
        };
        // stack limits and depth follow the calling thread
        realm.interp.restore(it.snapshot());
        let r = match &sandbox {
            Some(sb) => run_in_sandbox(it, &mut realm, sb, &code),
            None => realm.interp.run_sloppy(&code),
        };
        if let Some(slot) = it.vm.get_mut(&id) {
            slot.realm = Some(realm);
            slot.busy = false;
        }
        r
    });
    g.declare("__thaw_vm_run", run, true);
}
