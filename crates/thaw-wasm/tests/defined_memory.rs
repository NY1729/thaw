//! Module-defined memory: the module's own memory is rewritten to an import of an
//! embedder-created memory, so its bytes are shared (see src/defined_memory.rs).
//! Expectations were checked against node 22 on the same bytes (see `dump_fixtures`).
mod asm;

use std::rc::Rc;

use asm::*;
use thaw_wasm::{
    CallError, Engine, Extern, ExternKind, FuncId, HostCallbacks, Import, InstanceId, MemoryId,
    Module, Store, Val, ValType,
};

struct NoHost;

impl HostCallbacks for NoHost {
    fn call_import(&mut self, id: u32, _args: &[Val]) -> Result<Vec<Val>, u32> {
        panic!("unexpected host call {id}")
    }
}

/// Shape of the fixture module. Functions: store(addr, val), load(addr),
/// grow(pages) -> old pages, size() -> pages, and init() (start function, unexported).
struct Opts {
    min: u32,
    max: Option<u32>,
    export: bool,
    data: Option<(i32, Vec<u8>)>,
    start: bool,
}

impl Opts {
    fn new(min: u32, max: Option<u32>) -> Self {
        Self {
            min,
            max,
            export: true,
            data: None,
            start: false,
        }
    }
}

fn defined_module(o: &Opts) -> Vec<u8> {
    let mut exports = vec![
        export("store", 0, 0),
        export("load", 0, 1),
        export("grow", 0, 2),
        export("size", 0, 3),
    ];
    if o.export {
        exports.insert(0, export("memory", 2, 0));
    }
    let mut sections = vec![
        types(&[
            functype(&[I32, I32], &[]),
            functype(&[I32], &[I32]),
            functype(&[], &[I32]),
            functype(&[], &[]),
        ]),
        func_section(&[0, 1, 1, 2, 3]),
        section(5, vector(&[limits(o.min, o.max)])),
        export_section(&exports),
    ];
    if o.start {
        sections.push(section(8, uleb(4)));
    }
    sections.push(code_section(&[
        code(&[], &[0x20, 0, 0x20, 1, 0x3a, 0x00, 0x00]),
        code(&[], &[0x20, 0, 0x2d, 0x00, 0x00]),
        code(&[], &[0x20, 0, 0x40, 0x00]),
        code(&[], &[0x3f, 0x00]),
        code(&[], &[i32c(0), i32c(99), vec![0x3a, 0x00, 0x00]].concat()),
    ]));
    if let Some((offset, bytes)) = &o.data {
        let segment = [
            vec![0x00],
            i32c(*offset),
            vec![0x0b],
            uleb(bytes.len() as u64),
            bytes.clone(),
        ]
        .concat();
        sections.push(section(11, vector(&[segment])));
    }
    module(&sections)
}

fn instantiate(
    store: &mut Store,
    bytes: &[u8],
    engine: &Engine,
) -> Result<(Module, InstanceId), thaw_wasm::InstantiateError> {
    let module = Module::compile(engine, bytes).unwrap();
    let instance = store.instantiate(&module, &[], &mut NoHost)?;
    Ok((module, instance))
}

fn export_func(store: &Store, instance: InstanceId, name: &str) -> FuncId {
    match store.export(instance, name) {
        Some(Extern::Func(id)) => id,
        other => panic!("export {name}: {other:?}"),
    }
}

fn call(store: &mut Store, instance: InstanceId, name: &str, args: &[Val]) -> Vec<Val> {
    let f = export_func(store, instance, name);
    store
        .call_func(f, args, &mut NoHost)
        .map_err(|e: CallError| e.to_string())
        .unwrap()
}

fn exported_memory(store: &Store, instance: InstanceId) -> MemoryId {
    match store.export(instance, "memory") {
        Some(Extern::Memory(id)) => id,
        other => panic!("export memory: {other:?}"),
    }
}

#[test]
fn defined_memory_is_shared_with_the_embedder() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let (_, instance) =
        instantiate(&mut store, &defined_module(&Opts::new(1, Some(2))), &engine).unwrap();
    let mem = exported_memory(&store, instance);
    let buffer = store.memory_buffer(mem).expect("defined memory is shared");
    assert_eq!(buffer.borrow().len(), 65536);

    call(
        &mut store,
        instance,
        "store",
        &[Val::I32(10), Val::I32(171)],
    );
    assert_eq!(
        buffer.borrow()[10],
        171,
        "wasm store visible to the embedder"
    );
    buffer.borrow_mut()[20] = 205;
    assert_eq!(
        call(&mut store, instance, "load", &[Val::I32(20)]),
        vec![Val::I32(205)],
        "embedder write visible to wasm"
    );
    assert!(
        Rc::ptr_eq(&buffer, &store.memory_buffer(mem).unwrap()),
        "no detach without a grow"
    );
}

#[test]
fn data_segment_is_visible_right_after_instantiate() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let mut opts = Opts::new(1, Some(2));
    opts.data = Some((16, vec![1, 2, 3]));
    let (_, instance) = instantiate(&mut store, &defined_module(&opts), &engine).unwrap();
    let mem = exported_memory(&store, instance);
    let buffer = store.memory_buffer(mem).unwrap();
    assert_eq!(&buffer.borrow()[16..19], &[1, 2, 3]);
    assert_eq!(buffer.borrow()[19], 0);
    assert_eq!(
        call(&mut store, instance, "load", &[Val::I32(17)]),
        vec![Val::I32(2)]
    );
}

#[test]
fn start_function_runs_after_data_and_before_return() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let mut opts = Opts::new(1, Some(2));
    opts.data = Some((16, vec![7]));
    opts.start = true;
    let (_, instance) = instantiate(&mut store, &defined_module(&opts), &engine).unwrap();
    let buffer = store
        .memory_buffer(exported_memory(&store, instance))
        .unwrap();
    assert_eq!(buffer.borrow()[0], 99, "start wrote to memory");
    assert_eq!(buffer.borrow()[16], 7, "data segment still applied");
}

#[test]
fn grow_from_wasm_detaches_the_old_owner_and_keeps_bytes() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let (_, instance) =
        instantiate(&mut store, &defined_module(&Opts::new(1, Some(2))), &engine).unwrap();
    let mem = exported_memory(&store, instance);
    call(
        &mut store,
        instance,
        "store",
        &[Val::I32(10), Val::I32(171)],
    );
    let old = store.memory_buffer(mem).unwrap();
    let base = old.borrow().as_ptr();

    assert_eq!(
        call(&mut store, instance, "grow", &[Val::I32(1)]),
        vec![Val::I32(1)]
    );
    assert_eq!(old.borrow().len(), 0, "old buffer detached");
    let new = store.memory_buffer(mem).unwrap();
    assert!(!Rc::ptr_eq(&old, &new));
    assert_eq!(new.borrow().len(), 2 * 65536);
    assert_eq!(new.borrow().as_ptr(), base, "same allocation after grow");
    assert_eq!(new.borrow()[10], 171, "contents preserved");
    assert_eq!(store.memory_pages(mem), 2);
    assert_eq!(call(&mut store, instance, "size", &[]), vec![Val::I32(2)]);

    // Over the declared max: wasm sees -1 and nothing detaches.
    assert_eq!(
        call(&mut store, instance, "grow", &[Val::I32(1)]),
        vec![Val::I32(-1)]
    );
    assert!(Rc::ptr_eq(&new, &store.memory_buffer(mem).unwrap()));
}

#[test]
fn unexported_defined_memory_still_runs() {
    let engine = Engine::default();
    let mut store = Store::new(&engine);
    let mut opts = Opts::new(1, None);
    opts.export = false;
    let (_, instance) = instantiate(&mut store, &defined_module(&opts), &engine).unwrap();
    assert!(store.export(instance, "memory").is_none());
    let names: Vec<&str> = store
        .exports(instance)
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(names, ["store", "load", "grow", "size"]);
    call(&mut store, instance, "store", &[Val::I32(3), Val::I32(9)]);
    assert_eq!(
        call(&mut store, instance, "load", &[Val::I32(3)]),
        vec![Val::I32(9)]
    );
    assert_eq!(
        call(&mut store, instance, "grow", &[Val::I32(2)]),
        vec![Val::I32(1)]
    );
    assert_eq!(call(&mut store, instance, "size", &[]), vec![Val::I32(3)]);
}

#[test]
fn descriptors_are_unchanged() {
    let engine = Engine::default();
    let module = Module::compile(&engine, &defined_module(&Opts::new(1, Some(2)))).unwrap();
    assert!(module.imports().is_empty(), "hidden import is not listed");
    let kinds: Vec<(&str, &ExternKind)> = module
        .exports()
        .iter()
        .map(|e| (e.name.as_str(), &e.kind))
        .collect();
    assert_eq!(
        kinds[0],
        (
            "memory",
            &ExternKind::Memory {
                min: 1,
                max: Some(2)
            }
        )
    );
    assert_eq!(kinds.len(), 5);
    assert_eq!(
        kinds[1].1,
        &ExternKind::Func {
            params: vec![ValType::I32, ValType::I32],
            results: vec![]
        }
    );
}

#[test]
fn module_that_imports_a_reserved_name_is_not_rewritten() {
    // An import from the hidden module name keeps the module as it was: the
    // import is listed and the defined memory stays wasmi-owned.
    let engine = Engine::default();
    let bytes = module(&[
        types(&[functype(&[], &[])]),
        import_section(&[import_func("\u{0}thaw", "memory0", 0)]),
        section(5, vector(&[limits(1, None)])),
        export_section(&[export("memory", 2, 0)]),
    ]);
    let module = Module::compile(&engine, &bytes).unwrap();
    assert_eq!(module.imports().len(), 1);
    assert_eq!(module.imports()[0].module, "\u{0}thaw");
    let mut store = Store::new(&engine);
    struct Ok0;
    impl HostCallbacks for Ok0 {
        fn call_import(&mut self, _: u32, _: &[Val]) -> Result<Vec<Val>, u32> {
            Ok(vec![])
        }
    }
    let instance = store
        .instantiate(&module, &[Import::Host(0)], &mut Ok0)
        .unwrap();
    let mem = exported_memory(&store, instance);
    assert!(store.memory_buffer(mem).is_none(), "wasmi-owned memory");
}

#[test]
fn multi_memory_module_keeps_wasmi_owned_memory() {
    // Imported memory (index 0) plus a defined memory (index 1): not rewritten.
    let engine = Engine::default();
    let bytes = module(&[
        import_section(&[import_memory("env", "memory", 1, Some(2))]),
        section(5, vector(&[limits(1, None)])),
        export_section(&[export("m1", 2, 1)]),
    ]);
    let module = Module::compile(&engine, &bytes).unwrap();
    let mut store = Store::new(&engine);
    let imported = store.new_memory(1, Some(2)).unwrap();
    let instance = store
        .instantiate(&module, &[Import::Memory(imported)], &mut NoHost)
        .unwrap();
    let defined = exported_memory_named(&store, instance, "m1");
    assert!(store.memory_buffer(defined).is_none());
    assert_eq!(store.memory_pages(defined), 1);
    assert!(store.memory_buffer(imported).is_some());
}

fn exported_memory_named(store: &Store, instance: InstanceId, name: &str) -> MemoryId {
    match store.export(instance, name) {
        Some(Extern::Memory(id)) => id,
        other => panic!("export {name}: {other:?}"),
    }
}

/// Writes the fixtures as hex JSON for node 22 (ignored by default).
/// Output: $THAW_WASM_DEFINED_DUMP or /home/fedora/work/thaw-tmp/scratch-wasm/defined.json.
#[test]
#[ignore]
fn dump_fixtures() {
    let path = std::env::var("THAW_WASM_DEFINED_DUMP")
        .unwrap_or_else(|_| "/home/fedora/work/thaw-tmp/scratch-wasm/defined.json".to_string());
    let mut with_data = Opts::new(1, Some(2));
    with_data.data = Some((16, vec![1, 2, 3]));
    let mut start = Opts::new(1, Some(2));
    start.data = Some((16, vec![7]));
    start.start = true;
    let mut hidden = Opts::new(1, None);
    hidden.export = false;
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        ("shared", defined_module(&Opts::new(1, Some(2)))),
        ("data", defined_module(&with_data)),
        ("start", defined_module(&start)),
        ("unexported", defined_module(&hidden)),
        (
            "reserved",
            module(&[
                types(&[functype(&[], &[])]),
                import_section(&[import_func("\u{0}thaw", "memory0", 0)]),
                section(5, vector(&[limits(1, None)])),
                export_section(&[export("memory", 2, 0)]),
            ]),
        ),
        (
            "multi",
            module(&[
                import_section(&[import_memory("env", "memory", 1, Some(2))]),
                section(5, vector(&[limits(1, None)])),
                export_section(&[export("m1", 2, 1)]),
            ]),
        ),
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
