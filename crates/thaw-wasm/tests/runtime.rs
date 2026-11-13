//! Behaviour checks on hand-assembled modules. Expected values were recorded from node 22.23.1
//! on the same bytes (scratch-wasm/verify.js); the V8 texts are copied from that run.
mod asm;

use thaw_wasm::{
    CallError, CompileError, Engine, Extern, FuncId, HostCallbacks, Import, InstanceId,
    InstantiateError, LinkError, Module, Store, Trap, Val,
};

struct NoHost;

impl HostCallbacks for NoHost {
    fn call_import(&mut self, id: u32, _args: &[Val]) -> Result<Vec<Val>, u32> {
        panic!("unexpected host call {id}")
    }
}

/// Host that doubles its argument (for `env.cb`) and records the ids it was called with.
#[derive(Default)]
struct Doubler {
    seen: Vec<u32>,
}

impl HostCallbacks for Doubler {
    fn call_import(&mut self, id: u32, args: &[Val]) -> Result<Vec<Val>, u32> {
        self.seen.push(id);
        match args {
            [Val::I32(x)] => Ok(vec![Val::I32(x * 2)]),
            _ => Err(99),
        }
    }
}

fn compile(engine: &Engine, bytes: &[u8]) -> Module {
    Module::compile(engine, bytes).expect("module compiles")
}

fn instantiate(store: &mut Store, module: &Module, imports: &[Import]) -> InstanceId {
    store
        .instantiate(module, imports, &mut NoHost)
        .expect("instantiates")
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

fn trap_of(result: Result<Vec<Val>, CallError>) -> Trap {
    match result {
        Err(CallError::Trap(trap)) => trap,
        other => panic!("expected a trap, got {other:?}"),
    }
}

#[test]
fn add_wraps_like_i32() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::add_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let add = func(&store, instance, "add");
    assert_eq!(
        call(&mut store, add, &[Val::I32(2), Val::I32(3)]).unwrap(),
        vec![Val::I32(5)]
    );
    assert_eq!(
        call(&mut store, add, &[Val::I32(i32::MAX), Val::I32(1)]).unwrap(),
        vec![Val::I32(i32::MIN)]
    );
}

#[test]
fn imports_and_exports_describe_the_module() {
    use thaw_wasm::{ExportDesc, ExternKind, ImportDesc, ValType};
    let engine = Engine::default();
    let module = compile(&engine, &asm::memory_module());
    assert_eq!(
        module.imports(),
        &[ImportDesc {
            module: "env".into(),
            name: "memory".into(),
            kind: ExternKind::Memory {
                min: 1,
                max: Some(2)
            },
        }]
    );
    let names: Vec<&str> = module
        .exports()
        .iter()
        .map(|e: &ExportDesc| e.name.as_str())
        .collect();
    assert_eq!(names, ["memory", "store", "load", "grow", "copy"]);
    assert_eq!(
        module.exports()[1].kind,
        ExternKind::Func {
            params: vec![ValType::I32, ValType::I32],
            results: vec![]
        }
    );
}

#[test]
fn invalid_bytes_give_v8_compile_errors() {
    let engine = Engine::default();
    let error = |bytes: &[u8]| {
        Module::compile(&engine, bytes)
            .err()
            .map(|e: CompileError| e.message)
    };
    assert_eq!(
        error(b"(module)").as_deref(),
        Some("expected magic word 00 61 73 6d, found 28 6d 6f 64 @+0")
    );
    assert_eq!(
        error(&[]).as_deref(),
        Some("BufferSource argument is empty")
    );
    assert_eq!(
        error(&[0, 0x61, 0x73, 0x6d, 2, 0, 0, 0]).as_deref(),
        Some("expected version 01 00 00 00, found 02 00 00 00 @+4")
    );
    assert_eq!(
        error(&[1, 2, 3, 4, 1, 0, 0, 0]).as_deref(),
        Some("expected magic word 00 61 73 6d, found 01 02 03 04 @+0")
    );
    assert_eq!(
        error(&[0, 0x61]).as_deref(),
        Some("expected 4 bytes, fell off end @+0")
    );
    assert_eq!(
        error(&[0, 0x61, 0x73, 0x6d, 1, 0]).as_deref(),
        Some("expected 4 bytes, fell off end @+4")
    );
    assert!(Module::validate(&engine, &asm::add_module()).is_ok());
}

#[test]
fn multi_value_returns_both_results() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::multi_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let swap = func(&store, instance, "swap");
    assert_eq!(
        call(&mut store, swap, &[Val::I32(1), Val::I32(2)]).unwrap(),
        vec![Val::I32(2), Val::I32(1)]
    );
}

#[test]
fn traps_map_to_v8_classes() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::trap_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let div = func(&store, instance, "div");
    assert_eq!(
        call(&mut store, div, &[Val::I32(7), Val::I32(2)]).unwrap(),
        vec![Val::I32(3)]
    );
    let by_zero = trap_of(call(&mut store, div, &[Val::I32(1), Val::I32(0)]));
    assert_eq!(by_zero.message(), "divide by zero");
    let overflow = trap_of(call(&mut store, div, &[Val::I32(i32::MIN), Val::I32(-1)]));
    assert_eq!(overflow.message(), "divide result unrepresentable");
    let boom = func(&store, instance, "boom");
    assert_eq!(
        trap_of(call(&mut store, boom, &[])).message(),
        "unreachable"
    );
    let rec = func(&store, instance, "rec");
    let overflow = trap_of(call(&mut store, rec, &[Val::I32(0)]));
    assert_eq!(overflow.message(), "Maximum call stack size exceeded");
    assert!(
        overflow.is_range_error(),
        "node raises stack exhaustion as RangeError"
    );
}

#[test]
fn call_indirect_through_table() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::table_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let Some(Extern::Table(table)) = store.export(instance, "table") else {
        panic!("table export")
    };
    assert_eq!(store.table_size(table), 2);
    let slot0 = store.table_get(table, 0).expect("slot 0 in range");
    let Val::FuncRef(Some(add)) = slot0 else {
        panic!("slot 0 is a function: {slot0:?}")
    };
    assert_eq!(
        call(&mut store, add, &[Val::I32(2), Val::I32(3)]).unwrap(),
        vec![Val::I32(5)]
    );
    assert_eq!(store.table_get(table, 1), Some(Val::FuncRef(None)));
    let ci = func(&store, instance, "ci");
    let args = |idx: i32| [Val::I32(idx), Val::I32(2), Val::I32(3)];
    assert_eq!(call(&mut store, ci, &args(0)).unwrap(), vec![Val::I32(5)]);
    assert_eq!(
        trap_of(call(&mut store, ci, &args(1))).message(),
        "null function or function signature mismatch"
    );
    assert_eq!(
        trap_of(call(&mut store, ci, &args(5))).message(),
        "table index is out of bounds"
    );
    assert_eq!(store.table_grow(table, 1, &Val::FuncRef(None)), Ok(2));
    assert_eq!(store.table_size(table), 3);
}

#[test]
fn global_get_set_and_immutable_error() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::global_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let Some(Extern::Global(g)) = store.export(instance, "g") else {
        panic!("global export")
    };
    let get = func(&store, instance, "get");
    assert_eq!(store.global_get(g), Val::I32(7));
    assert_eq!(call(&mut store, get, &[]).unwrap(), vec![Val::I32(7)]);
    store.global_set(g, &Val::I32(9)).unwrap();
    assert_eq!(call(&mut store, get, &[]).unwrap(), vec![Val::I32(9)]);
    let fixed = store.new_global(false, &Val::I32(1));
    assert_eq!(
        store.global_set(fixed, &Val::I32(2)).unwrap_err(),
        "Can't set the value of an immutable global."
    );
}

#[test]
fn host_import_is_called_and_returns_values() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::import_module("env", "cb"));
    let mut store = Store::new(&engine);
    let instance = store
        .instantiate(&module, &[Import::Host(7)], &mut NoHost)
        .map_or_else(|_| panic!("import instantiates"), |i| i);
    let run = func(&store, instance, "run");
    let mut host = Doubler::default();
    assert_eq!(
        store.call_func(run, &[Val::I32(5)], &mut host).unwrap(),
        vec![Val::I32(11)]
    );
    assert_eq!(host.seen, vec![7]);
}

#[test]
fn host_exception_is_passed_through() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::import_module("env", "cb"));
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[Import::Host(1)]);
    let run = func(&store, instance, "run");
    // The host gets a non-i32 argument path only through a bad call; use run with an i32 and
    // make Doubler reject by calling a 0-arg-shaped host: here we simply return Err from a custom host.
    struct Throws;
    impl HostCallbacks for Throws {
        fn call_import(&mut self, _id: u32, _args: &[Val]) -> Result<Vec<Val>, u32> {
            Err(42)
        }
    }
    let result = store.call_func(run, &[Val::I32(1)], &mut Throws);
    assert_eq!(result, Err(CallError::Exception(42)));
}

#[test]
fn link_errors_use_node_text() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);

    let missing_module = compile(&engine, &asm::import_module("env", "cb"));
    let err = store
        .instantiate(
            &missing_module,
            &[Import::Missing {
                module_exists: false,
            }],
            &mut NoHost,
        )
        .unwrap_err();
    assert_eq!(
        err,
        InstantiateError::Link(LinkError {
            type_error: true,
            message: "Import #0 module=\"env\": module is not an object or function".into(),
        })
    );

    let missing_field = compile(&engine, &asm::import_module("env", "missing"));
    let err = store
        .instantiate(
            &missing_field,
            &[Import::Missing {
                module_exists: true,
            }],
            &mut NoHost,
        )
        .unwrap_err();
    assert_eq!(
        err,
        InstantiateError::Link(LinkError {
            type_error: false,
            message:
                "Import #0 module=\"env\" function=\"missing\": function import requires a callable"
                    .into(),
        })
    );

    let add_instance = instantiate(&mut store, &compile(&engine, &asm::add_module()), &[]);
    let add = func(&store, add_instance, "add");
    let cb = compile(&engine, &asm::import_module("env", "cb"));
    let err = store
        .instantiate(&cb, &[Import::Func(add)], &mut NoHost)
        .unwrap_err();
    assert_eq!(
        err,
        InstantiateError::Link(LinkError {
            type_error: false,
            message: "Import #0 module=\"env\" function=\"cb\": imported function does not match the expected type".into(),
        })
    );
}

#[test]
fn externref_handles_round_trip() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::externref_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let id = func(&store, instance, "id");
    assert_eq!(
        call(&mut store, id, &[Val::ExternRef(Some(42))]).unwrap(),
        vec![Val::ExternRef(Some(42))]
    );
    assert_eq!(
        call(&mut store, id, &[Val::ExternRef(None)]).unwrap(),
        vec![Val::ExternRef(None)]
    );
}

#[test]
fn module_defined_memory_is_shared() {
    let engine = Engine::default();
    let module = compile(&engine, &asm::defined_memory_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let Some(Extern::Memory(mem)) = store.export(instance, "memory") else {
        panic!("memory export")
    };
    assert_eq!(store.memory_pages(mem), 1);
    assert_eq!(
        store.memory_buffer(mem).map(|b| b.borrow().len()),
        Some(65536),
        "module-defined memory is shared (see defined_memory.rs)"
    );
    assert_eq!(store.with_memory_bytes(mem, |bytes| bytes.len()), 65536);
    let load = func(&store, instance, "load");
    assert_eq!(
        call(&mut store, load, &[Val::I32(0)]).unwrap(),
        vec![Val::I32(0)]
    );
}

#[cfg(feature = "simd")]
#[test]
fn simd_add_when_built_with_simd() {
    let engine = Engine::default();
    assert!(Module::validate(&engine, &asm::simd_module()).is_ok());
    let module = compile(&engine, &asm::simd_module());
    let mut store = Store::new(&engine);
    let instance = instantiate(&mut store, &module, &[]);
    let add = func(&store, instance, "simd_add");
    assert_eq!(
        call(&mut store, add, &[Val::I32(3), Val::I32(4)]).unwrap(),
        vec![Val::I32(7)]
    );
}

#[cfg(not(feature = "simd"))]
#[test]
fn simd_is_rejected_without_the_feature() {
    assert!(Module::validate(&Engine::default(), &asm::simd_module()).is_err());
}
