//! Differential tests for the `Console` port (`src/web/console.js`).
mod web_harness;

#[test]
fn console_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/console"
        ),
        &[],
    );
}
