// tj35 (class A): `Bytes` crosses the thaw-js boundary as plain data. Compile-only contracts;
// the host-side wire is pinned in crates/thaw-js/src/tests.rs (`tj35_bytes_cross_the_typed_abi`).

fn tj35_symbol(name: &str) -> String {
    format!("__thaw_typed_tjs_{}", tj6_hex(name))
}

/// Whether some `call` instruction in `ir` calls `@callee` (declarations do not count).
fn tj35_calls(ir: &str, callee: &str) -> bool {
    let needle = format!("@{callee}(");
    ir.lines()
        .any(|line| line.contains("call ") && line.contains(&needle))
}

#[test]
fn tj35_buffer_result_is_a_routed_thaw_js_call_not_a_stub() {
    let f = tj35_symbol("m::bytes");
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(): Buffer;
         function main(): void {{ console.log({f}().length); }}"
    ));
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("thaw_tjs_call_typed_result"), "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj35_promise_of_buffer_is_a_routed_thaw_js_call_not_a_stub() {
    let f = tj35_symbol("m::bytesAsync");
    let (uses_thaw_js, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(): Promise<Buffer>;
         async function main(): Promise<void> {{ const b = await {f}(); console.log(b.length); }}"
    ));
    assert!(uses_thaw_js, "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj35_bare_buffer_argument_is_sent_as_the_tagged_base64_node() {
    let f = tj35_symbol("m::shape");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(value: Buffer): Json;
         function main(): void {{ console.log({f}(Buffer.from([1]))); }}"
    ));
    assert!(ir.contains("__thaw_tjs_bytes__"), "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn tj35_bare_buffer_argument_selects_its_kind_at_runtime() {
    // `k` is picked from the runtime kind (`thaw_bytes_console_info`), not hard-coded to "Buffer"
    let f = tj35_symbol("m::shape");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(value: Buffer): Json;
         function main(): void {{ console.log({f}(Buffer.from([1]))); }}"
    ));
    assert!(ir.contains("thaw_js_bytes_kind_u8"), "{ir}");
    assert!(tj35_calls(&ir, "thaw_bytes_console_info"), "{ir}");
}

#[test]
fn tj35_nested_buffer_argument_still_stubs() {
    // `Buffer[]` is not encoded per element yet (stage 2), so it keeps the stub
    let f = tj35_symbol("m::shapes");
    let (uses_thaw_js, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(value: Buffer[]): Json;
         function main(): void {{ const xs: Buffer[] = []; if (xs.length > 5) {{ console.log({f}(xs)); }} }}"
    ));
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn tj35_random_fill_sync_copies_the_result_back_into_the_argument() {
    let fill = tj35_symbol("m::randomFillSync");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {fill}(buf: Buffer): Buffer;
         function main(): void {{ const b = Buffer.alloc(4); if (b.length > 5) {{ console.log({fill}(b).length); }} }}"
    ));
    assert!(tj35_calls(&ir, "thaw_bytes_copy"), "{ir}");
    // any other thaw-js function returning a Buffer from its argument keeps its own copy
    let other = tj35_symbol("m::randomBytes");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {other}(buf: Buffer): Buffer;
         function main(): void {{ const b = Buffer.alloc(4); if (b.length > 5) {{ console.log({other}(b).length); }} }}"
    ));
    assert!(!tj35_calls(&ir, "thaw_bytes_copy"), "{ir}");
}

#[test]
fn tj35_random_fill_sync_copy_back_passes_array_data_not_handles() {
    // `thaw_bytes_copy` reads ByteView data pointers, so both operands must be unwrapped
    // with `compile_array_data` first; the raw 16-byte array handles would be misread.
    let fill = tj35_symbol("m::randomFillSync");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {fill}(buf: Buffer): Buffer;
         function main(): void {{ const b = Buffer.alloc(4); if (b.length > 5) {{ console.log({fill}(b).length); }} }}"
    ));
    let call = ir
        .lines()
        .find(|line| line.contains("call ") && line.contains("@thaw_bytes_copy("))
        .unwrap_or_else(|| panic!("no thaw_bytes_copy call:\n{ir}"));
    assert!(call.contains("%array_data"), "{call}\n{ir}");
}

#[test]
fn tj93_bare_buffer_argument_selects_array_buffer_and_data_view_kinds() {
    // ArrayBuffer (code 1) and DataView (code 2) get their own `k` names, not "Buffer"
    let f = tj35_symbol("m::shape");
    let (_, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(value: ArrayBuffer): Json;
         function main(): void {{ console.log({f}(new ArrayBuffer(1))); }}"
    ));
    assert!(ir.contains("thaw_js_bytes_kind_ab"), "{ir}");
    assert!(ir.contains("thaw_js_bytes_kind_dv"), "{ir}");
}
