//! Differential tests for the fetch classes, full web streams and `crypto.subtle.digest`
//! (`src/web/fetch.js`, `src/web/crypto.js`).
mod web_harness;

#[test]
fn fetch_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/web/fetch"),
        &[],
    );
}
