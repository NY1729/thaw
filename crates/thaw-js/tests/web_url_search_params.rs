//! Differential tests for the `url_search_params` port (the native `URLSearchParams` in `src/url.rs`; no port needed).
mod web_harness;

#[test]
fn url_search_params_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/url_search_params"
        ),
        &[],
    );
}
