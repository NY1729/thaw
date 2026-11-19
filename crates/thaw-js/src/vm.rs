//! `node:vm` contexts: the `__thaw_vm_*` natives behind `registry/builtins/system/vm_context.js`.
//! A context is a separate `Interp` (its own intrinsics and global scope), built on its first run
//! so that contexts which are only created cost nothing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

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
    "navigator",
    "Navigator",
    "Console",
];

/// Globals every context shares with the host instead of building its own.
const SHARED: &[&str] = &["Intl", "WebAssembly", "SharedArrayBuffer", "Atomics"];

/// A fresh context realm: its own ECMAScript intrinsics, with the host-only globals hidden.
fn new_realm(host: &Interp) -> Box<Interp> {
    // symbols are thread-wide (symbol_table.rs), so this realm's symbol ids never collide with another realm's
    let mut sub = Box::new(Interp::new_vm_realm());
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
            if let Value::Object(o) = &v {
                sub.vm_shared.push(o.clone());
            }
            sub.global.declare(name, v, true);
        }
    }
    let global = Value::Object(sub.global_object.clone());
    sub.global.set_this(global);
    sub
}
/// A context's realm plus what persists between runs of a sandboxed context.
pub(crate) struct VmRealm {
    pub(crate) interp: Box<Interp>,
    /// Scope `scope -> with(sandbox) -> global` holding the context's var/let/const/function
    /// bindings; built on the first run with a sandbox.
    pub(crate) script: Option<Rc<Scope>>,
    /// Global bindings at index >= `n0` were made by sandboxed runs (implicit globals).
    n0: usize,
    /// Var and function names of earlier runs, and implicit globals already copied to the sandbox.
    vars: Vec<Rc<str>>,
}

/// One context: `realm` is `None` until the first run, and taken out while the context runs.
pub(crate) struct VmSlot {
    realm: Option<Box<VmRealm>>,
    busy: bool,
    /// `realm_id` of the Interp that created the context; dropping that Interp drops the slot.
    owner: usize,
}

thread_local! {
    /// Every context on this thread, by id, shared by all realms. A context can be run from inside another
    /// realm (a host closure called by a context calls `vm.runInContext`), and that run executes on the caller's
    /// Interp, so a per-Interp table would not find it. Not shared between threads: an Interp holds `Rc`s, so the
    /// table moves with the control at a coroutine hand-off (`take_contexts`), never used by two threads at once.
    /// `ManuallyDrop`: at thread exit the table is not dropped, so no realm is torn down
    /// while other thread-locals are already gone; a context outlives its thread only if its owner was leaked.
    static CONTEXTS: RefCell<ManuallyDrop<HashMap<u32, VmSlot>>> =
        RefCell::new(ManuallyDrop::new(HashMap::new()));
}

/// Process-wide, not thread-local: a context made on a coroutine thread must not reuse an id of another thread's table.
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// This thread's context table, moved out with the other host tables at a coroutine hand-off
/// (`host::park_host_tls`); the table left behind is empty.
pub(crate) fn take_contexts() -> ManuallyDrop<HashMap<u32, VmSlot>> {
    CONTEXTS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Puts back a table from `take_contexts` (`host::adopt_host_tls`); the table it replaces is empty.
pub(crate) fn restore_contexts(
    table: ManuallyDrop<HashMap<u32, VmSlot>>,
) -> ManuallyDrop<HashMap<u32, VmSlot>> {
    CONTEXTS.with(|c| c.replace(table))
}

/// Removes the slots `owner` created and returns them. The caller drops them after the borrow ends:
/// dropping a realm drops its Interp, which comes back here.
pub(crate) fn take_owned(owner: usize) -> Vec<VmSlot> {
    let ids: Vec<u32> = CONTEXTS.with(|c| {
        c.borrow()
            .iter()
            .filter(|(_, s)| s.owner == owner)
            .map(|(id, _)| *id)
            .collect()
    });
    CONTEXTS.with(|c| {
        let mut c = c.borrow_mut();
        ids.iter().filter_map(|id| c.remove(id)).collect()
    })
}

/// The global object of the first context realm built on this thread (tests).
#[cfg(test)]
pub(crate) fn first_realm_global_object() -> Option<Obj> {
    CONTEXTS.with(|c| {
        c.borrow()
            .values()
            .find_map(|s| s.realm.as_ref().map(|r| r.interp.global_object.clone()))
    })
}

impl VmSlot {
    pub(crate) fn realm_mut(&mut self) -> Option<&mut VmRealm> {
        self.realm.as_deref_mut()
    }
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

/// The filename a vm call was given: `None` when it was not a value (no filename, no stack frame).
fn script_file(it: &mut Interp, v: &Value) -> Option<Rc<str>> {
    match v {
        Value::Undefined => None,
        v => Some(Rc::from(it.to_str(v).to_string())),
    }
}

/// The SyntaxError of `code` for `new vm.Script(code, { filename })`, which compiles it without running it.
fn syntax_check(it: &mut Interp, code: &str, file: Option<Rc<str>>) -> R<()> {
    if crate::too_nested(code) {
        return it.throw_error("RangeError", crate::STACK_OVERFLOW);
    }
    match thaw_parser::parse_javascript_script_located(code) {
        Ok(_) => Ok(()),
        Err(f) => it.throw_syntax(file.as_deref(), code, &f),
    }
}

/// The filename of the vm script running in this realm, for errors' `stack` (`undefined` when none).
pub(crate) fn install_script_file(it: &mut Interp) {
    let get = it.native(|it, _, _| {
        Ok(match &it.script_filename {
            Some(file) => Value::str(file),
            None => Value::Undefined,
        })
    });
    it.global.declare("__thaw_vm_script_file", get, true);
}

/// Starts the timeout of a run (`0` is none); `disarm` ends it.
fn arm(ip: &mut Interp, ms: u64) {
    ip.deadline =
        (ms > 0).then(|| std::time::Instant::now() + std::time::Duration::from_millis(ms));
    ip.dl_ms = ms;
    ip.dl_hit = false;
}

fn disarm(ip: &mut Interp) {
    ip.deadline = None;
    ip.dl_hit = false;
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    let create = it.native(|it, _, _| {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1;
        let owner = it.realm_id;
        CONTEXTS.with(|c| {
            c.borrow_mut().insert(
                id,
                VmSlot {
                    realm: None,
                    busy: false,
                    owner,
                },
            )
        });
        Ok(Value::Number(f64::from(id)))
    });
    g.declare("__thaw_vm_create", create, true);
    let free = it.native(|it, _, a| {
        let id = it.num(&arg(a, 0)) as u32;
        // the slot (and its realm) drops after the borrow ends
        let slot = CONTEXTS.with(|c| c.borrow_mut().remove(&id));
        drop(slot);
        Ok(Value::Undefined)
    });
    g.declare("__thaw_vm_free", free, true);
    // (id, code, filename, timeoutMs, sandbox, stop). The filename names the script in the stack of an error the
    // run makes (`script_filename`); the previous one is put back when the run ends, on every path.
    let run = it.native(|it, _, a| {
        let id = it.num(&arg(a, 0)) as i64;
        let code = it.to_str(&arg(a, 1)).to_string();
        let file = script_file(it, &arg(a, 2));
        let ms = it.num(&arg(a, 3)) as u64;
        if id < 0 {
            // runInThisContext: indirect-eval semantics in this realm
            let prev = std::mem::replace(&mut it.script_filename, file);
            arm(it, ms);
            let r = it.run_vm_plain(&code);
            disarm(it);
            it.script_filename = prev;
            return r;
        }
        let sandbox = match arg(a, 4) {
            Value::Object(sb) => Some(sb),
            Value::Undefined => None,
            _ => return it.throw_error("TypeError", "vm sandbox must be an object"),
        };
        let id = id as u32;
        // the realm is taken out of its slot while it runs, so a nested run of this context is refused
        enum Claim {
            Released,
            Busy,
            Taken(Option<Box<VmRealm>>),
        }
        let claim = CONTEXTS.with(|c| match c.borrow_mut().get_mut(&id) {
            None => Claim::Released,
            Some(slot) if slot.busy => Claim::Busy,
            Some(slot) => {
                slot.busy = true;
                Claim::Taken(slot.realm.take())
            }
        });
        let mut realm = match claim {
            Claim::Released => return it.throw_error("Error", "the vm context was released"),
            Claim::Busy => return it.throw_error("Error", "the vm context is busy"),
            Claim::Taken(Some(realm)) => realm,
            Claim::Taken(None) => Box::new(VmRealm {
                interp: new_realm(it),
                script: None,
                n0: 0,
                vars: Vec::new(),
            }),
        };
        // stack limits and depth follow the calling thread
        realm.interp.restore(it.snapshot());
        realm.interp.vm_sandbox = sandbox.clone();
        let prev = std::mem::replace(&mut realm.interp.script_filename, file);
        arm(&mut realm.interp, ms);
        let r = match &sandbox {
            Some(sb) => run_in_sandbox(it, &mut realm, sb, &code),
            None => realm.interp.run_vm_plain(&code),
        };
        disarm(&mut realm.interp);
        realm.interp.vm_sandbox = None;
        realm.interp.script_filename = prev;
        // put the realm back unless the context was freed while it ran (then it drops here, after the borrow)
        CONTEXTS.with(|c| {
            if let Some(slot) = c.borrow_mut().get_mut(&id) {
                slot.realm = Some(realm);
                slot.busy = false;
            }
        });
        r
    });
    g.declare("__thaw_vm_run", run, true);
    // (source, filename, syntaxOnly): `new vm.Script` checks the syntax with `syntaxOnly`, and a SyntaxError names
    // the filename
    let eval = it.native(|it, _, a| {
        let src = it.to_str(&arg(a, 0)).to_string();
        let file = script_file(it, &arg(a, 1));
        syntax_check(it, &src, file)?;
        if arg(a, 2).truthy() {
            Ok(Value::Undefined)
        } else {
            it.run_sloppy(&src)
        }
    });
    g.declare("__thaw_vm_eval", eval, true);
}
