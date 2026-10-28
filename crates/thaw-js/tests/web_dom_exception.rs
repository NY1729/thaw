//! Differential tests for the `dom_exception` port (`src/web/dom_exception.js`).
mod web_harness;

#[test]
fn dom_exception_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/dom_exception"
        ),
        &[],
    );
}
