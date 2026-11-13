//! Tiny wasm assembler for test fixtures: just enough sections and opcodes.
#![allow(dead_code)]

pub const I32: u8 = 0x7f;
pub const I64: u8 = 0x7e;
pub const F32: u8 = 0x7d;
pub const F64: u8 = 0x7c;
pub const V128: u8 = 0x7b;
pub const FUNCREF: u8 = 0x70;
pub const EXTERNREF: u8 = 0x6f;

pub fn uleb(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

pub fn sleb(mut value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        if done {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// `i32.const v`.
pub fn i32c(value: i32) -> Vec<u8> {
    [vec![0x41], sleb(value as i64)].concat()
}

/// Length-prefixed UTF-8 name.
pub fn name(text: &str) -> Vec<u8> {
    [uleb(text.len() as u64), text.as_bytes().to_vec()].concat()
}

/// Count-prefixed vector of already-encoded items.
pub fn vector(items: &[Vec<u8>]) -> Vec<u8> {
    [uleb(items.len() as u64), items.concat()].concat()
}

pub fn section(id: u8, body: Vec<u8>) -> Vec<u8> {
    [vec![id], uleb(body.len() as u64), body].concat()
}

pub fn module(sections: &[Vec<u8>]) -> Vec<u8> {
    [b"\0asm".to_vec(), vec![1, 0, 0, 0], sections.concat()].concat()
}

pub fn functype(params: &[u8], results: &[u8]) -> Vec<u8> {
    [
        vec![0x60],
        uleb(params.len() as u64),
        params.to_vec(),
        uleb(results.len() as u64),
        results.to_vec(),
    ]
    .concat()
}

pub fn types(items: &[Vec<u8>]) -> Vec<u8> {
    section(1, vector(items))
}

pub fn limits(min: u32, max: Option<u32>) -> Vec<u8> {
    match max {
        None => [vec![0x00], uleb(min as u64)].concat(),
        Some(max) => [vec![0x01], uleb(min as u64), uleb(max as u64)].concat(),
    }
}

pub fn import_func(module: &str, field: &str, type_index: u32) -> Vec<u8> {
    [
        name(module),
        name(field),
        vec![0x00],
        uleb(type_index as u64),
    ]
    .concat()
}

pub fn import_memory(module: &str, field: &str, min: u32, max: Option<u32>) -> Vec<u8> {
    [name(module), name(field), vec![0x02], limits(min, max)].concat()
}

/// Export kinds: 0 func, 1 table, 2 memory, 3 global.
pub fn export(text: &str, kind: u8, index: u32) -> Vec<u8> {
    [name(text), vec![kind], uleb(index as u64)].concat()
}

/// Function body. `locals` are (count, type) pairs.
pub fn code(locals: &[(u32, u8)], body: &[u8]) -> Vec<u8> {
    let mut inner = uleb(locals.len() as u64);
    for (count, ty) in locals {
        inner.extend(uleb(*count as u64));
        inner.push(*ty);
    }
    inner.extend_from_slice(body);
    inner.push(0x0b);
    [uleb(inner.len() as u64), inner].concat()
}

pub fn func_section(type_indices: &[u32]) -> Vec<u8> {
    let items: Vec<Vec<u8>> = type_indices.iter().map(|t| uleb(*t as u64)).collect();
    section(3, vector(&items))
}

pub fn export_section(items: &[Vec<u8>]) -> Vec<u8> {
    section(7, vector(items))
}

pub fn code_section(bodies: &[Vec<u8>]) -> Vec<u8> {
    section(10, vector(bodies))
}

pub fn import_section(items: &[Vec<u8>]) -> Vec<u8> {
    section(2, vector(items))
}

// ---- fixtures -------------------------------------------------------------

/// `(func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)`
pub fn add_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32, I32], &[I32])]),
        func_section(&[0]),
        export_section(&[export("add", 0, 0)]),
        code_section(&[code(&[], &[0x20, 0, 0x20, 1, 0x6a])]),
    ])
}

/// Imports `env.memory` (1 page, max 2) and exports: `memory`, `store(addr, val)`, `load(addr)`,
/// `grow(pages) -> old_pages`, `copy(dst, src, n)` (bulk memory).
pub fn memory_module() -> Vec<u8> {
    module(&[
        types(&[
            functype(&[I32, I32], &[]),
            functype(&[I32], &[I32]),
            functype(&[I32, I32, I32], &[]),
        ]),
        import_section(&[import_memory("env", "memory", 1, Some(2))]),
        func_section(&[0, 1, 1, 2]),
        export_section(&[
            export("memory", 2, 0),
            export("store", 0, 0),
            export("load", 0, 1),
            export("grow", 0, 2),
            export("copy", 0, 3),
        ]),
        code_section(&[
            code(&[], &[0x20, 0, 0x20, 1, 0x36, 0x02, 0x00]),
            code(&[], &[0x20, 0, 0x28, 0x02, 0x00]),
            code(&[], &[0x20, 0, 0x40, 0x00]),
            code(&[], &[0x20, 0, 0x20, 1, 0x20, 2, 0xfc, 0x0a, 0x00, 0x00]),
        ]),
    ])
}

/// Like `memory_module` but imports the memory as "memory" from the embedder and
/// the type of func 2 is grow(i32)->i32, plus a host-import `env.poke(addr, val)`
/// that the module calls before `viahost(addr, val) -> load8_u(addr)`.
pub fn poke_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32, I32], &[]), functype(&[I32, I32], &[I32])]),
        import_section(&[
            import_func("env", "poke", 0),
            import_memory("env", "memory", 1, Some(2)),
        ]),
        func_section(&[1]),
        export_section(&[export("viahost", 0, 1), export("memory", 2, 0)]),
        code_section(&[code(
            &[],
            &[0x20, 0, 0x20, 1, 0x10, 0x00, 0x20, 0, 0x2d, 0x00, 0x00],
        )]),
    ])
}

/// Two exported functions with 0 and 1 params whose types are (i32,i32)->i32 (add) and
/// call_indirect through a 2-slot funcref table (slot 0 = add, slot 1 = null):
/// `ci(idx, a, b) = call_indirect(a, b)[idx]`.
pub fn table_module() -> Vec<u8> {
    let elem = [vec![0x00], vec![0x41, 0x00, 0x0b], vector(&[uleb(0)])].concat();
    module(&[
        types(&[
            functype(&[I32, I32], &[I32]),
            functype(&[I32, I32, I32], &[I32]),
        ]),
        func_section(&[0, 1]),
        section(4, vector(&[[vec![FUNCREF], limits(2, Some(4))].concat()])),
        export_section(&[
            export("table", 1, 0),
            export("add", 0, 0),
            export("ci", 0, 1),
        ]),
        section(9, vector(&[elem])),
        code_section(&[
            code(&[], &[0x20, 0, 0x20, 1, 0x6a]),
            code(&[], &[0x20, 1, 0x20, 2, 0x20, 0, 0x11, 0x00, 0x00]),
        ]),
    ])
}

/// Mutable i32 global (init 7) exported as "g", plus `get() -> i32`.
pub fn global_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[], &[I32])]),
        func_section(&[0]),
        section(6, vector(&[vec![I32, 0x01, 0x41, 0x07, 0x0b]])),
        export_section(&[export("g", 3, 0), export("get", 0, 0)]),
        code_section(&[code(&[], &[0x23, 0x00])]),
    ])
}

/// Imports `module.field : (i32) -> i32` and exports `run(x) = field(x) + 1`.
pub fn import_module(module_name: &str, field: &str) -> Vec<u8> {
    module(&[
        types(&[functype(&[I32], &[I32])]),
        import_section(&[import_func(module_name, field, 0)]),
        func_section(&[0]),
        export_section(&[export("run", 0, 1)]),
        code_section(&[code(&[], &[0x20, 0, 0x10, 0x00, 0x41, 0x01, 0x6a])]),
    ])
}

/// div(a, b) = a / b (signed), boom() = unreachable, rec(x) = rec(x) (infinite recursion).
pub fn trap_module() -> Vec<u8> {
    module(&[
        types(&[
            functype(&[I32, I32], &[I32]),
            functype(&[], &[]),
            functype(&[I32], &[I32]),
        ]),
        func_section(&[0, 1, 2]),
        export_section(&[
            export("div", 0, 0),
            export("boom", 0, 1),
            export("rec", 0, 2),
        ]),
        code_section(&[
            code(&[], &[0x20, 0, 0x20, 1, 0x6d]),
            code(&[], &[0x00]),
            code(&[], &[0x20, 0, 0x10, 0x02]),
        ]),
    ])
}

/// swap(a, b) = (b, a): multi-value return.
pub fn multi_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32, I32], &[I32, I32])]),
        func_section(&[0]),
        export_section(&[export("swap", 0, 0)]),
        code_section(&[code(&[], &[0x20, 1, 0x20, 0])]),
    ])
}

/// simd_add(a, b) = extract_lane0(splat(a) + splat(b)) on i32x4.
pub fn simd_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32, I32], &[I32])]),
        func_section(&[0]),
        export_section(&[export("simd_add", 0, 0)]),
        code_section(&[code(
            &[],
            &[
                0x20, 0, 0xfd, 0x11, 0x20, 1, 0xfd, 0x11, 0xfd, 0xae, 0x01, 0xfd, 0x1b, 0x00,
            ],
        )]),
    ])
}

/// id(x: externref) -> externref.
pub fn externref_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[EXTERNREF], &[EXTERNREF])]),
        func_section(&[0]),
        export_section(&[export("id", 0, 0)]),
        code_section(&[code(&[], &[0x20, 0])]),
    ])
}

/// Imports `wasi_snapshot_preview1.proc_exit(i32)`; `_start` calls `proc_exit(3)`. Exports memory (Node requires it).
pub fn wasi_exit_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32], &[]), functype(&[], &[])]),
        import_section(&[import_func("wasi_snapshot_preview1", "proc_exit", 0)]),
        func_section(&[1]),
        section(5, vector(&[limits(1, None)])),
        export_section(&[export("_start", 0, 1), export("memory", 2, 0)]),
        code_section(&[code(&[], &[0x41, 0x03, 0x10, 0x00])]),
    ])
}

/// Imports `wasi_snapshot_preview1.fd_write` and the memory "env.memory"; re-exports the
/// memory as "memory". `_start` writes the iovec at 16 (buf 0, len 3) to stdout (fd 1),
/// with nwritten at 32.
pub fn wasi_write_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32, I32, I32, I32], &[I32]), functype(&[], &[])]),
        import_section(&[
            import_func("wasi_snapshot_preview1", "fd_write", 0),
            import_memory("env", "memory", 1, Some(2)),
        ]),
        func_section(&[1]),
        export_section(&[export("_start", 0, 1), export("memory", 2, 0)]),
        code_section(&[code(
            &[],
            &[
                0x41, 0x01, 0x41, 0x10, 0x41, 0x01, 0x41, 0x20, 0x10, 0x00, 0x1a,
            ],
        )]),
    ])
}

/// A module that defines (does not import) its own memory, exporting it as "memory".
pub fn defined_memory_module() -> Vec<u8> {
    module(&[
        types(&[functype(&[I32], &[I32])]),
        func_section(&[0]),
        section(5, vector(&[limits(1, Some(2))])),
        export_section(&[export("memory", 2, 0), export("load", 0, 0)]),
        code_section(&[code(&[], &[0x20, 0, 0x28, 0x02, 0x00])]),
    ])
}
