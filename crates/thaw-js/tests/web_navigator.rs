//! Differential tests for the `Navigator` / `navigator` port (`src/web/navigator.js`).
mod web_harness;

#[test]
fn navigator_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/navigator"
        ),
        &[],
    );
}
