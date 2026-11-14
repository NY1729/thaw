//! Externally owned memory: sharing, detach on grow, host writes, two instances, WASI.
mod asm;

use std::rc::Rc;

use thaw_wasm::{
    Bytes, CallError, Engine, Extern, FuncId, HostCallbacks, Import, InstanceId, MemoryId, Module,
    Store, Val,
};

struct NoHost;

impl HostCallbacks for NoHost {
    fn call_import(
        &mut self,
        _store: &mut thaw_wasm::Access<'_, '_>,
        id: u32,
        _args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        panic!("unexpected host call {id}")
    }
}

/// Host import `env.poke(addr, val)`: writes straight into the shared buffer mid-call.
struct Poke(Bytes);

impl HostCallbacks for Poke {
    fn call_import(
        &mut self,
        _store: &mut thaw_wasm::Access<'_, '_>,
        _id: u32,
        args: &[Val],
    ) -> Result<Vec<Val>, u32> {
        let (Val::I32(addr), Val::I32(value)) = (&args[0], &args[1]) else {
            return Err(1);
        };
        self.0.borrow_mut()[*addr as usize] = *value as u8;
        Ok(vec![])
    }
}

fn func(store: &Store, instance: InstanceId, name: &str) -> FuncId {
    match store.export(instance, name) {
        Some(Extern::Func(id)) => id,
        other => panic!("export {name}: {other:?}"),
    }
}

fn call(store: &mut Store, f: FuncId, args: &[Val]) -> Result<Vec<Val>, CallError> {
    store.call_func(f, args, &mut NoHost)
}

fn invoke(
    store: &mut Store,
    instance: InstanceId,
    name: &str,
    args: &[Val],
) -> Result<Vec<Val>, CallError> {
    let f = func(store, instance, name);
    call(store, f, args)
}

fn memory_store() -> (Engine, Store, Module, MemoryId) {
    let engine = Engine::default();
    let module = Module::compile(&engine, &asm::memory_module()).unwrap();
    let mut store = Store::new(&engine);
    let mem = store.new_memory(1, Some(2)).unwrap();
    (engine, store, module, mem)
}

#[test]
fn wasm_and_host_share_bytes_without_copies() {
    let (_engine, mut store, module, mem) = memory_store();
    let buffer = store
        .memory_buffer(mem)
        .expect("embedder-owned memory has a buffer");
    let base = buffer.borrow().as_ptr();
    let instance = store
        .instantiate(&module, &[Import::Memory(mem)], &mut NoHost)
        .unwrap();
    let store_fn = func(&store, instance, "store");
    let load = func(&store, instance, "load");

    call(&mut store, store_fn, &[Val::I32(10), Val::I32(171)]).unwrap();
    assert_eq!(buffer.borrow()[10], 171, "wasm store visible to host");
    buffer.borrow_mut()[20] = 205;
    assert_eq!(
        call(&mut store, load, &[Val::I32(20)]).unwrap(),
        vec![Val::I32(205)],
        "host write visible to wasm"
    );
    assert_eq!(buffer.borrow().as_ptr(), base, "no reallocation");
}

#[test]
fn grow_detaches_old_buffer_and_keeps_bytes() {
    let (_engine, mut store, module, mem) = memory_store();
    let old = store.memory_buffer(mem).unwrap();
    let base = old.borrow().as_ptr();
    let instance = store
        .instantiate(&module, &[Import::Memory(mem)], &mut NoHost)
        .unwrap();
    invoke(
        &mut store,
        instance,
        "store",
        &[Val::I32(10), Val::I32(171)],
    )
    .unwrap();
    let generation = store.memory_generation(mem);

    let grow = func(&store, instance, "grow");
    assert_eq!(
        call(&mut store, grow, &[Val::I32(1)]).unwrap(),
        vec![Val::I32(1)],
        "grow returns old pages"
    );
    assert_eq!(old.borrow().len(), 0, "old owner detached");
    let new = store.memory_buffer(mem).unwrap();
    assert!(!Rc::ptr_eq(&old, &new));
    assert_eq!(new.borrow().len(), 2 * 65536);
    assert_eq!(new.borrow().as_ptr(), base, "same allocation after grow");
    assert_eq!(new.borrow()[10], 171, "contents preserved");
    assert_eq!(store.memory_generation(mem), generation + 1);
    assert_eq!(store.memory_pages(mem), 2);

    // Over max: wasm sees -1, memory unchanged.
    assert_eq!(
        call(&mut store, grow, &[Val::I32(1)]).unwrap(),
        vec![Val::I32(-1)]
    );
    assert_eq!(store.memory_pages(mem), 2);
    assert!(
        Rc::ptr_eq(&new, &store.memory_buffer(mem).unwrap()),
        "no detach when grow fails"
    );
    // Later grows still work (grow by 0 reports the current size).
    assert_eq!(
        call(&mut store, grow, &[Val::I32(0)]).unwrap(),
        vec![Val::I32(2)]
    );
}

#[test]
fn copy_and_bounds_checks() {
    let (_engine, mut store, module, mem) = memory_store();
    let instance = store
        .instantiate(&module, &[Import::Memory(mem)], &mut NoHost)
        .unwrap();
    let (store_fn, load, grow, copy) = (
        func(&store, instance, "store"),
        func(&store, instance, "load"),
        func(&store, instance, "grow"),
        func(&store, instance, "copy"),
    );
    call(&mut store, store_fn, &[Val::I32(8), Val::I32(42)]).unwrap();
    call(&mut store, store_fn, &[Val::I32(4), Val::I32(43)]).unwrap();
    call(&mut store, copy, &[Val::I32(8), Val::I32(0), Val::I32(8)]).unwrap();
    assert_eq!(
        call(&mut store, load, &[Val::I32(8)]).unwrap(),
        vec![Val::I32(0)]
    );
    assert_eq!(
        call(&mut store, load, &[Val::I32(12)]).unwrap(),
        vec![Val::I32(43)]
    );

    call(&mut store, grow, &[Val::I32(1)]).unwrap();
    match call(&mut store, load, &[Val::I32(131072)]) {
        Err(CallError::Trap(trap)) => assert_eq!(trap.message(), "memory access out of bounds"),
        other => panic!("expected out-of-bounds trap, got {other:?}"),
    }
}

#[test]
fn host_import_write_is_seen_mid_call() {
    let engine = Engine::default();
    let module = Module::compile(&engine, &asm::poke_module()).unwrap();
    let mut store = Store::new(&engine);
    let mem = store.new_memory(1, Some(2)).unwrap();
    let buffer = store.memory_buffer(mem).unwrap();
    let mut host = Poke(buffer.clone());
    let instance = store
        .instantiate(&module, &[Import::Host(1), Import::Memory(mem)], &mut host)
        .unwrap();
    let viahost = func(&store, instance, "viahost");
    let result = store
        .call_func(viahost, &[Val::I32(30), Val::I32(90)], &mut host)
        .unwrap();
    assert_eq!(
        result,
        vec![Val::I32(90)],
        "wasm reads the byte the host wrote during the same call"
    );
}

#[test]
fn two_instances_share_one_memory() {
    let (_engine, mut store, module, mem) = memory_store();
    let a = store
        .instantiate(&module, &[Import::Memory(mem)], &mut NoHost)
        .unwrap();
    let b = store
        .instantiate(&module, &[Import::Memory(mem)], &mut NoHost)
        .unwrap();
    invoke(&mut store, a, "store", &[Val::I32(5), Val::I32(99)]).unwrap();
    assert_eq!(
        invoke(&mut store, b, "load", &[Val::I32(5)]).unwrap(),
        vec![Val::I32(99)]
    );
    invoke(&mut store, a, "grow", &[Val::I32(1)]).unwrap();
    assert_eq!(store.memory_pages(mem), 2);
    assert_eq!(
        invoke(&mut store, b, "load", &[Val::I32(5)]).unwrap(),
        vec![Val::I32(99)]
    );
}

#[cfg(feature = "wasi")]
#[test]
fn wasi_fd_write_lands_in_shared_memory() {
    use thaw_wasm::wasi::WasiOptions;
    let engine = Engine::default();
    let module = Module::compile(&engine, &asm::wasi_write_module()).unwrap();
    let mut store = Store::new(&engine);
    let mem = store.new_memory(1, Some(2)).unwrap();
    let buffer = store.memory_buffer(mem).unwrap();
    {
        let mut bytes = buffer.borrow_mut();
        bytes[..3].copy_from_slice(b"abc");
        // iovec at 16: buf = 0, len = 3
        bytes[16..20].copy_from_slice(&0u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&3u32.to_le_bytes());
    }
    let instance = store
        .instantiate_with_wasi(
            &module,
            &[
                Import::Missing {
                    module_exists: true,
                },
                Import::Memory(mem),
            ],
            &mut NoHost,
            &WasiOptions::default(),
        )
        .unwrap();
    invoke(&mut store, instance, "_start", &[]).unwrap();
    assert_eq!(
        u32::from_le_bytes(buffer.borrow()[32..36].try_into().unwrap()),
        3,
        "nwritten"
    );
}

#[cfg(feature = "wasi")]
#[test]
fn wasi_proc_exit_reports_the_code() {
    use thaw_wasm::wasi::WasiOptions;
    let engine = Engine::default();
    let module = Module::compile(&engine, &asm::wasi_exit_module()).unwrap();
    let mut store = Store::new(&engine);
    let instance = store
        .instantiate_with_wasi(
            &module,
            &[Import::Missing {
                module_exists: true,
            }],
            &mut NoHost,
            &WasiOptions::default(),
        )
        .unwrap();
    assert_eq!(
        invoke(&mut store, instance, "_start", &[]),
        Err(CallError::Exit(3))
    );
}
