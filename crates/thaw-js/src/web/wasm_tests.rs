//! Interp-level tests of the `__thaw_wasm_*` natives. The modules are hand-assembled (a minimal copy
//! of the `thaw-wasm` test assembler); each test runs JS that console.logs its results.

use super::*;

// ---- minimal assembler (copied from crates/thaw-wasm/tests/asm) -----------

const I32: u8 = 0x7f;

fn uleb(mut value: u64) -> Vec<u8> {
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

fn name(text: &str) -> Vec<u8> {
    [uleb(text.len() as u64), text.as_bytes().to_vec()].concat()
}

fn vector(items: &[Vec<u8>]) -> Vec<u8> {
    [uleb(items.len() as u64), items.concat()].concat()
}

fn section(id: u8, body: Vec<u8>) -> Vec<u8> {
    [vec![id], uleb(body.len() as u64), body].concat()
}

fn module(sections: &[Vec<u8>]) -> Vec<u8> {
    [b"\0asm".to_vec(), vec![1, 0, 0, 0], sections.concat()].concat()
}

fn functype(params: &[u8], results: &[u8]) -> Vec<u8> {
    [
        vec![0x60],
        uleb(params.len() as u64),
        params.to_vec(),
        uleb(results.len() as u64),
        results.to_vec(),
    ]
    .concat()
}

fn limits(min: u32, max: Option<u32>) -> Vec<u8> {
    match max {
        None => [vec![0x00], uleb(min as u64)].concat(),
        Some(max) => [vec![0x01], uleb(min as u64), uleb(max as u64)].concat(),
    }
}

fn import_func(module: &str, field: &str, type_index: u32) -> Vec<u8> {
    [
        name(module),
        name(field),
        vec![0x00],
        uleb(type_index as u64),
    ]
    .concat()
}

/// Export kinds: 0 func, 1 table, 2 memory, 3 global.
fn export(text: &str, kind: u8, index: u32) -> Vec<u8> {
    [name(text), vec![kind], uleb(index as u64)].concat()
}

fn code(body: &[u8]) -> Vec<u8> {
    let inner = [vec![0x00], body.to_vec(), vec![0x0b]].concat();
    [uleb(inner.len() as u64), inner].concat()
}

fn func_section(type_indices: &[u32]) -> Vec<u8> {
    let items: Vec<Vec<u8>> = type_indices.iter().map(|t| uleb(*t as u64)).collect();
    section(3, vector(&items))
}

// ---- fixtures -------------------------------------------------------------

/// add(a, b) = a + b.
fn add_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32, I32], &[I32])])),
        func_section(&[0]),
        section(7, vector(&[export("add", 0, 0)])),
        section(10, vector(&[code(&[0x20, 0, 0x20, 1, 0x6a])])),
    ])
}

/// run(x) = g(x) + 1, with `g` imported from env.
fn import_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32], &[I32])])),
        section(2, vector(&[import_func("env", "g", 0)])),
        func_section(&[0]),
        section(7, vector(&[export("run", 0, 1)])),
        section(
            10,
            vector(&[code(&[0x20, 0, 0x10, 0x00, 0x41, 0x01, 0x6a])]),
        ),
    ])
}

/// div(a, b) = a / b (signed).
fn div_module() -> Vec<u8> {
    module(&[
        section(1, vector(&[functype(&[I32, I32], &[I32])])),
        func_section(&[0]),
        section(7, vector(&[export("div", 0, 0)])),
        section(10, vector(&[code(&[0x20, 0, 0x20, 1, 0x6d])])),
    ])
}

/// A defined memory (1..4 pages) exported as "memory", with poke(addr, v), peek(addr) and grow(n).
fn memory_module() -> Vec<u8> {
    module(&[
        section(
            1,
            vector(&[functype(&[I32, I32], &[]), functype(&[I32], &[I32])]),
        ),
        func_section(&[0, 1, 1]),
        section(5, vector(&[limits(1, Some(4))])),
        section(
            7,
            vector(&[
                export("poke", 0, 0),
                export("peek", 0, 1),
                export("grow", 0, 2),
                export("memory", 2, 0),
            ]),
        ),
        section(
            10,
            vector(&[
                code(&[0x20, 0, 0x20, 1, 0x3a, 0, 0]),
                code(&[0x20, 0, 0x2d, 0, 0]),
                code(&[0x20, 0, 0x40, 0]),
            ]),
        ),
    ])
}

/// `new Uint8Array([...])` source for `bytes`.
fn js_bytes(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
    format!("new Uint8Array([{}])", items.join(","))
}

/// Runs `src` and returns the console.log lines joined with `|` (or the uncaught error).
fn run(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn compile_validate_imports_and_exports_describe_the_module() {
    let src = format!(
        "const m = __thaw_wasm_compile({});
         console.log(__thaw_wasm_validate({}), __thaw_wasm_validate(new Uint8Array([0,97,115,109,2,0,0,0])));
         const imps = __thaw_wasm_module_imports(__thaw_wasm_compile({}));
         console.log(imps.length, imps[0].module, imps[0].name, imps[0].kind);
         const exps = __thaw_wasm_module_exports(m);
         console.log(exps.length, exps[0].name, exps[0].kind);",
        js_bytes(&add_module()),
        js_bytes(&add_module()),
        js_bytes(&import_module()),
    );
    assert_eq!(run(&src), "true false|1 env g function|1 add function");
}

#[test]
fn bad_module_throws_v8_compile_error_text() {
    let src = "try { __thaw_wasm_compile(new Uint8Array([0,97,115,109,2,0,0,0])); }
               catch (e) { console.log(e.__wasmErrorKind, e.message); }";
    assert_eq!(
        run(src),
        "CompileError WebAssembly.Module(): expected version 01 00 00 00, found 02 00 00 00 @+4"
    );
}

#[test]
fn add_call_coerces_arguments_to_i32() {
    let src = format!(
        "const inst = __thaw_wasm_instantiate(__thaw_wasm_compile({}), []);
         console.log(__thaw_wasm_call(inst, 'add', [2, 3]), __thaw_wasm_call(inst, 'add', ['7', 1]));
         console.log(__thaw_wasm_call(inst, 'add', [2147483648, 1]));",
        js_bytes(&add_module())
    );
    assert_eq!(run(&src), "5 8|-2147483647");
}

#[test]
fn trap_maps_to_runtime_error_with_v8_text() {
    let src = format!(
        "const inst = __thaw_wasm_instantiate(__thaw_wasm_compile({}), []);
         try {{ __thaw_wasm_call(inst, 'div', [7, 0]); }}
         catch (e) {{ console.log(e.__wasmErrorKind, e.message); }}",
        js_bytes(&div_module())
    );
    assert_eq!(run(&src), "RuntimeError divide by zero");
}

#[test]
fn host_import_is_called_and_its_result_flows_back_into_wasm() {
    let src = format!(
        "const mod = __thaw_wasm_compile({});
         const inst = __thaw_wasm_instantiate(mod, [['env', 'g', 'function', (x) => x * 10]]);
         console.log(__thaw_wasm_call(inst, 'run', [4]));",
        js_bytes(&import_module())
    );
    assert_eq!(run(&src), "41");
}

#[test]
fn host_import_exception_propagates_as_the_same_js_value() {
    let src = format!(
        "const err = new TypeError('boom');
         const mod = __thaw_wasm_compile({});
         const inst = __thaw_wasm_instantiate(mod, [['env', 'g', 'function', () => {{ throw err; }}]]);
         try {{ __thaw_wasm_call(inst, 'run', [1]); }} catch (e) {{ console.log(e === err, e.name, e.message); }}",
        js_bytes(&import_module())
    );
    assert_eq!(run(&src), "true TypeError boom");
}

#[test]
fn missing_host_import_is_a_link_error_and_missing_module_a_type_error() {
    let src = format!(
        "const mod = __thaw_wasm_compile({});
         try {{ __thaw_wasm_instantiate(mod, [['env', 'g', 'missing', undefined]]); }}
         catch (e) {{ console.log(e.__wasmErrorKind, e.message); }}
         try {{ __thaw_wasm_instantiate(mod, [['env', 'g', 'missingModule', undefined]]); }}
         catch (e) {{ console.log(e.name, e.message); }}",
        js_bytes(&import_module())
    );
    assert_eq!(
        run(&src),
        "LinkError WebAssembly.Instance(): Import #0 module=\"env\" function=\"g\": function import requires a callable|\
         TypeError WebAssembly.Instance(): Import #0 module=\"env\": module is not an object or function"
    );
}

#[test]
fn memory_buffer_is_stable_until_grow_and_shares_bytes_with_wasm() {
    let src = format!(
        "const inst = __thaw_wasm_instantiate(__thaw_wasm_compile({}), []);
         const memh = __thaw_wasm_instance_exports(inst).find((e) => e.name === 'memory').handle;
         const buf = __thaw_wasm_memory_buffer(memh);
         console.log(buf === __thaw_wasm_memory_buffer(memh), buf.byteLength);
         new Uint8Array(buf)[10] = 42;
         console.log(__thaw_wasm_call(inst, 'peek', [10]));
         __thaw_wasm_call(inst, 'poke', [11, 7]);
         console.log(new Uint8Array(buf)[11]);
         console.log(__thaw_wasm_call(inst, 'grow', [1]), __thaw_wasm_memory_size(memh));
         const after = __thaw_wasm_memory_buffer(memh);
         console.log(after === buf, buf.byteLength, after.byteLength, new Uint8Array(after)[10]);
         new Uint8Array(after)[70000] = 5;
         console.log(__thaw_wasm_call(inst, 'peek', [70000]));",
        js_bytes(&memory_module())
    );
    assert_eq!(run(&src), "true 65536|42|7|1 2|false 0 131072 42|5");
}

#[test]
fn memory_grow_past_the_maximum_is_a_range_error() {
    let src = format!(
        "const inst = __thaw_wasm_instantiate(__thaw_wasm_compile({}), []);
         const memh = __thaw_wasm_instance_exports(inst).find((e) => e.name === 'memory').handle;
         try {{ __thaw_wasm_memory_grow(memh, 10); }}
         catch (e) {{ console.log(e.name, e.message); }}
         console.log(__thaw_wasm_memory_grow(memh, 1));",
        js_bytes(&memory_module())
    );
    assert_eq!(
        run(&src),
        "RangeError WebAssembly.Memory.grow(): Maximum memory size exceeded|1"
    );
}

#[test]
fn table_ops_grow_and_range_errors() {
    let src = "const t = __thaw_wasm_table_create('anyfunc', 2, undefined, null);
         console.log(__thaw_wasm_table_length(t), __thaw_wasm_table_get(t, 0));
         console.log(__thaw_wasm_table_grow(t, 1, null), __thaw_wasm_table_length(t));
         __thaw_wasm_table_set(t, 2, null);
         try { __thaw_wasm_table_get(t, 5); } catch (e) { console.log(e.name, e.message); }";
    assert_eq!(
        run(src),
        "2 null|2 3|RangeError WebAssembly.Table.get(): invalid index 5 into funcref table of size 3"
    );
}

#[test]
fn global_ops_coerce_and_enforce_mutability() {
    let src = "const g = __thaw_wasm_global_create('i32', true, 5);
         __thaw_wasm_global_set(g, 9);
         console.log(__thaw_wasm_global_get(g));
         const c = __thaw_wasm_global_create('i64', false, 5n);
         console.log(String(__thaw_wasm_global_get(c)));
         try { __thaw_wasm_global_set(c, 1n); } catch (e) { console.log(e.name, e.message); }
         try { __thaw_wasm_global_create('i64', true, 5); } catch (e) { console.log(e.name, e.message); }";
    assert_eq!(
        run(src),
        "9|5|TypeError set WebAssembly.Global.value: Can't set the value of an immutable global.|TypeError Cannot convert 5 to a BigInt"
    );
}
