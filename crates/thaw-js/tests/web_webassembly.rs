//! Differential tests for the real `WebAssembly` namespace (`src/web/webassembly.js` over the
//! `__thaw_wasm_*` natives). Needs the `wasm` feature; without it the target is empty.
#![cfg(feature = "wasm")]
mod web_harness;

#[test]
fn webassembly_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/webassembly"
        ),
        &[],
    );
}
