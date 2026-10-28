//! Differential tests for the `abort` port (`src/web/abort.js`).
mod web_harness;

#[test]
fn abort_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/web/abort"),
        &[],
    );
}
