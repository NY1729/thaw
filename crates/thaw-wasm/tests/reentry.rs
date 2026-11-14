//! Re-entrancy: a host import calls back into the store that is running it (exports, memory,
//! grow). Expected values were checked against node 22 on the same bytes (see the notes on each test).
mod asm;

use std::rc::Rc;

use asm::{
    code, export, func_section, functype, import_func, limits, module, section, vector, I32,
};
use thaw_wasm::{
    Access, Bytes, CallError, Engine, Extern, HostCallbacks, Import, InstanceId, MemoryId, Module,
    Store, Trap, Val,
};

fn int(v: &Val) -> i32 {
    match v {
        Val::I32(x) => *x,
        other => panic!("expected i32, got {other:?}"),
    }
}

/// inner(x) = k(x) * 2 (imports k = func 1); outer(x) = h(x) + 1 (imports h = func 0).
fn chain_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32], &[I32])])),
        section(
            2,
            vector(&[import_func("env", "h", 0), import_func("env", "k", 0)]),
        ),
        func_section(&[0, 0]),
        section(7, vector(&[export("inner", 0, 2), export("outer", 0, 3)])),
        section(
            10,
            vector(&[
                code(&[], &[0x20, 0, 0x10, 1, 0x41, 2, 0x6c]),
                code(&[], &[0x20, 0, 0x10, 0, 0x41, 1, 0x6a]),
            ]),
        ),
    ])
}

/// grow_then_call(n): memory.grow(n), then h(0). Memory: 1..4 pages, exported as "memory".
fn grow_module() -> Vec<u8> {
    module(&[
        section(
            1,
            vector(&[functype(&[I32], &[I32]), functype(&[I32], &[])]),
        ),
        section(2, vector(&[import_func("env", "h", 0)])),
        func_section(&[1]),
        section(5, vector(&[limits(1, Some(4))])),
        section(
            7,
            vector(&[export("grow_then_call", 0, 1), export("memory", 2, 0)]),
        ),
        section(
            10,
            vector(&[code(
                &[],
                &[0x20, 0, 0x40, 0x00, 0x1a, 0x41, 0, 0x10, 0, 0x1a],
            )]),
        ),
    ])
}

/// bad(x) = x / 0 (traps), outer(x) = h(x) + 1, ok(x) = x + 1. h = func 0.
fn trap_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32], &[I32])])),
        section(2, vector(&[import_func("env", "h", 0)])),
        func_section(&[0, 0, 0]),
        section(
            7,
            vector(&[
                export("bad", 0, 1),
                export("outer", 0, 2),
                export("ok", 0, 3),
            ]),
        ),
        section(
            10,
            vector(&[
                code(&[], &[0x20, 0, 0x41, 0, 0x6d]),
                code(&[], &[0x20, 0, 0x10, 0, 0x41, 1, 0x6a]),
                code(&[], &[0x20, 0, 0x41, 1, 0x6a]),
            ]),
        ),
    ])
}

/// recurse(n) = h(n - 1) + 1 when n != 0, else 0. h = func 0 (calls back into recurse).
fn recurse_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32], &[I32])])),
        section(2, vector(&[import_func("env", "h", 0)])),
        func_section(&[0]),
        section(7, vector(&[export("recurse", 0, 1)])),
        section(
            10,
            vector(&[code(
                &[],
                &[
                    0x20, 0, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 0,
                    0x41, 1, 0x6a, 0x0b,
                ],
            )]),
        ),
    ])
}

fn compile(engine: &Engine, bytes: &[u8]) -> Module {
    Module::compile(engine, bytes).unwrap()
}

fn instance_of(store: &Store, instance: InstanceId, name: &str) -> Extern {
    store.export(instance, name).expect("export")
}

/// The chain host: import 0 calls the export `inner` (nested), import 1 is `k`, which throws
/// `9` when x == 7.
struct Chain {
    inst: InstanceId,
}

impl HostCallbacks for Chain {
    fn call_import(
        &mut self,
        store: &mut Access<'_, '_>,
        id: u32,
        args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        let x = int(&args[0]);
        match id {
            0 => match store.call_export(self.inst, "inner", &[Val::I32(x)], self) {
                Ok(v) => Ok(vec![Val::I32(int(&v[0]) + 1000)]),
                Err(CallError::Exception(id)) => Err(id),
                Err(other) => panic!("unexpected {other:?}"),
            },
            _ if x == 7 => Err(9),
            _ => Ok(vec![Val::I32(x + 5)]),
        }
    }
}

#[test]
fn wasm_import_calls_export_and_result_flows_back_through_two_levels() {
    // node 22: h = x => inner(x) + 1000 with inner(x) = (x + 5) * 2, so outer(4) = 1019.
    let engine = Engine::default();
    let module = compile(&engine, &chain_module());
    let mut store = Store::new(&engine);
    let mut host = Chain {
        inst: InstanceId::from_index(0),
    };
    let inst = store
        .instantiate(&module, &[Import::Host(0), Import::Host(1)], &mut host)
        .unwrap();
    assert_eq!(inst, InstanceId::from_index(0));
    let outer = store.call_export(inst, "outer", &[Val::I32(4)], &mut host);
    assert_eq!(outer.unwrap(), vec![Val::I32(1019)]);
    let inner = store.call_export(inst, "inner", &[Val::I32(4)], &mut host);
    assert_eq!(inner.unwrap(), vec![Val::I32(18)]);
}

#[test]
fn nested_import_exception_passes_through_both_wasm_frames_unchanged() {
    // The innermost import throws id 9; it must reach the top-level caller as the same id.
    let engine = Engine::default();
    let module = compile(&engine, &chain_module());
    let mut store = Store::new(&engine);
    let mut host = Chain {
        inst: InstanceId::from_index(0),
    };
    let inst = store
        .instantiate(&module, &[Import::Host(0), Import::Host(1)], &mut host)
        .unwrap();
    let result = store.call_export(inst, "outer", &[Val::I32(7)], &mut host);
    assert!(matches!(result, Err(CallError::Exception(9))), "{result:?}");
    // The store is usable afterwards.
    let again = store.call_export(inst, "outer", &[Val::I32(4)], &mut host);
    assert_eq!(again.unwrap(), vec![Val::I32(1019)]);
}

/// Records what an import sees of the memory, and holds the buffer from before the call.
struct MemView {
    mem: MemoryId,
    old: Bytes,
    seen: Vec<u64>,
}

impl HostCallbacks for MemView {
    fn call_import(
        &mut self,
        store: &mut Access<'_, '_>,
        _id: u32,
        _args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        let now = store.memory_buffer(self.mem).expect("shared memory");
        self.seen = vec![
            now.borrow().len() as u64,
            store.memory_pages(self.mem),
            self.old.borrow().len() as u64,
            Rc::ptr_eq(&now, &self.old) as u64,
        ];
        Ok(vec![Val::I32(0)])
    }
}

#[test]
fn import_sees_the_grown_memory_and_the_old_buffer_is_detached() {
    // node 22: after memory.grow(1) inside wasm, the import sees byteLength 131072 and the
    // buffer captured before the call has byteLength 0 and is not the current buffer.
    let engine = Engine::default();
    let module = compile(&engine, &grow_module());
    let mut store = Store::new(&engine);
    let mut host = MemView {
        mem: MemoryId::from_index(0),
        old: Rc::new(std::cell::RefCell::new(Vec::new())),
        seen: Vec::new(),
    };
    let inst = store
        .instantiate(&module, &[Import::Host(0)], &mut host)
        .unwrap();
    let mem = match instance_of(&store, inst, "memory") {
        Extern::Memory(m) => m,
        other => panic!("{other:?}"),
    };
    host.mem = mem;
    host.old = store.memory_buffer(mem).unwrap();
    let result = store.call_export(inst, "grow_then_call", &[Val::I32(1)], &mut host);
    assert_eq!(result.unwrap(), vec![]);
    assert_eq!(host.seen, vec![131072, 2, 0, 0]);
}

#[test]
fn grow_by_zero_replaces_the_buffer_and_detaches_the_old_one() {
    // node 22: new WebAssembly.Memory({initial: 1, maximum: 2}); m.grow(0) returns 1,
    // m.buffer !== old and old.byteLength === 0.
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let mem = store.new_memory(1, Some(2)).unwrap();
    let old = store.memory_buffer(mem).unwrap();
    assert_eq!(store.memory_grow(mem, 0), Ok(1));
    let new = store.memory_buffer(mem).unwrap();
    assert!(!Rc::ptr_eq(&old, &new));
    assert_eq!(old.borrow().len(), 0);
    assert_eq!(new.borrow().len(), 65536);
}

/// Calls the export `bad` and turns its trap into exception 5 (the trap itself is kept).
struct TrapHost {
    inst: InstanceId,
    err: Option<CallError>,
}

impl HostCallbacks for TrapHost {
    fn call_import(
        &mut self,
        store: &mut Access<'_, '_>,
        _id: u32,
        args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        match store.call_export(self.inst, "bad", &args[..1], self) {
            Ok(v) => Ok(v),
            Err(error) => {
                self.err = Some(error);
                Err(5)
            }
        }
    }
}

#[test]
fn nested_trap_surfaces_at_the_outer_call_and_the_outer_call_unwinds_cleanly() {
    // node 22: h = x => bad(x) with bad(x) = x / 0 gives RuntimeError "divide by zero".
    let engine = Engine::default();
    let module = compile(&engine, &trap_module());
    let mut store = Store::new(&engine);
    let mut host = TrapHost {
        inst: InstanceId::from_index(0),
        err: None,
    };
    let inst = store
        .instantiate(&module, &[Import::Host(0)], &mut host)
        .unwrap();
    let result = store.call_export(inst, "outer", &[Val::I32(3)], &mut host);
    assert!(matches!(result, Err(CallError::Exception(5))), "{result:?}");
    assert!(matches!(
        host.err,
        Some(CallError::Trap(Trap::DivideByZero))
    ));
    let ok = store.call_export(inst, "ok", &[Val::I32(41)], &mut host);
    assert_eq!(ok.unwrap(), vec![Val::I32(42)]);
}

/// recurse through the import: h(x) = recurse(x), keeping the first nested error (if any).
struct Recurse {
    inst: InstanceId,
    err: Option<CallError>,
}

impl HostCallbacks for Recurse {
    fn call_import(
        &mut self,
        store: &mut Access<'_, '_>,
        _id: u32,
        args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        match store.call_export(self.inst, "recurse", &args[..1], self) {
            Ok(v) => Ok(v),
            Err(CallError::Exception(id)) => Err(id),
            Err(error) => {
                self.err = Some(error);
                Err(0)
            }
        }
    }
}

#[test]
fn deep_reentry_past_the_guard_is_a_stack_overflow_not_a_rust_one() {
    // node 22: recurse(3) = 3, recurse(50) = 50 (within the default limit of 100 levels).
    let engine = Engine::default();
    let module = compile(&engine, &recurse_module());
    let mut store = Store::new(&engine);
    let mut host = Recurse {
        inst: InstanceId::from_index(0),
        err: None,
    };
    let inst = store
        .instantiate(&module, &[Import::Host(0)], &mut host)
        .unwrap();
    assert_eq!(
        store
            .call_export(inst, "recurse", &[Val::I32(50)], &mut host)
            .unwrap(),
        vec![Val::I32(50)]
    );

    store.set_reentry_limit(5);
    assert_eq!(
        store
            .call_export(inst, "recurse", &[Val::I32(3)], &mut host)
            .unwrap(),
        vec![Val::I32(3)]
    );
    let deep = store.call_export(inst, "recurse", &[Val::I32(10)], &mut host);
    assert!(matches!(deep, Err(CallError::Exception(0))), "{deep:?}");
    assert!(matches!(
        host.err,
        Some(CallError::Trap(Trap::StackOverflow))
    ));
}

/// Dumps the re-entrancy fixtures as hex JSON for node (ignored by default).
#[test]
#[ignore]
fn dump_reentry_fixtures() {
    let path = "/home/fedora/work/thaw-tmp/scratch-wasm/reentry_fixtures.json";
    let fixtures = [
        ("chain", chain_module()),
        ("grow", grow_module()),
        ("trap", trap_module()),
        ("recurse", recurse_module()),
    ];
    let entries: Vec<String> = fixtures
        .iter()
        .map(|(name, bytes)| {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("\"{name}\":\"{hex}\"")
        })
        .collect();
    std::fs::write(path, format!("{{{}}}", entries.join(","))).unwrap();
}
