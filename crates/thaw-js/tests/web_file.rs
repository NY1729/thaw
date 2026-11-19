//! Differential tests for the `File` global (`src/web/blob.js`), recorded from Node v22.
mod web_harness;

#[test]
fn file_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/web/file"),
        &[],
    );
}
