//! Writes every fixture as hex JSON (ignored by default; used to verify against node 22).
//! Output: $THAW_WASM_DUMP or /home/fedora/work/thaw-tmp/scratch-wasm/fixtures.json.
mod asm;

#[test]
#[ignore]
fn dump_fixtures() {
    let path = std::env::var("THAW_WASM_DUMP")
        .unwrap_or_else(|_| "/home/fedora/work/thaw-tmp/scratch-wasm/fixtures.json".to_string());
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        ("add", asm::add_module()),
        ("memory", asm::memory_module()),
        ("poke", asm::poke_module()),
        ("table", asm::table_module()),
        ("global", asm::global_module()),
        ("import", asm::import_module("env", "cb")),
        ("missing_field", asm::import_module("env", "missing")),
        ("trap", asm::trap_module()),
        ("multi", asm::multi_module()),
        ("simd", asm::simd_module()),
        ("externref", asm::externref_module()),
        ("wasi_exit", asm::wasi_exit_module()),
        ("wasi_write", asm::wasi_write_module()),
        ("defined_memory", asm::defined_memory_module()),
    ];
    let entries: Vec<String> = fixtures
        .iter()
        .map(|(name, bytes)| {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("\"{name}\":\"{hex}\"")
        })
        .collect();
    let out = format!("{{{}}}", entries.join(","));
    std::fs::write(path, out).unwrap();
}
