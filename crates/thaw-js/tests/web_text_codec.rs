//! Differential tests for the `TextEncoder` / `TextDecoder` port (`src/web/text_codec.js`).
mod web_harness;

#[test]
fn text_codec_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/text_codec"
        ),
        &[],
    );
}
