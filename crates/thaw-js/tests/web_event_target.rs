//! Differential tests for the `Event` / `EventTarget` port (`src/web/event_target.js`).
mod web_harness;

#[test]
fn event_target_cases_match_recorded_node_output() {
    web_harness::run_dir(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/differential/web/event_target"
        ),
        &[],
    );
}
