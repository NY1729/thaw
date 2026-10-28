//! Differential tests for the `structured_clone` port (the native `structuredClone` in `src/extras.rs`).
mod web_harness;

#[test]
fn structured_clone_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/structured_clone"
        ),
        &[],
    );
}
