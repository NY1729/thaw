//! Differential tests for `Performance` / `performance` and the user-timing classes (`src/web/performance.js`).
mod web_harness;

#[test]
fn performance_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/performance"
        ),
        &[],
    );
}
