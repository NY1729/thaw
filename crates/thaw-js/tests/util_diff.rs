//! Differential tests for `util.types` (the `globalThis.__thaw_util_types` object the node:util polyfill exports).
//!
//! `tests/differential/util/<case>.js` are run by the `thaw-js-node` binary; stdout and the exit code must equal
//! `<case>.out` / `<case>.code`, recorded from real Node (v22):
//! `for f in *.js; do TZ=America/New_York node $f > ${f%.js}.out; echo $? > ${f%.js}.code; done`

use std::process::Command;

#[test]
fn util_cases_match_recorded_node_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/util");
    let mut cases: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.path().to_str().map(str::to_string))
        .filter(|p| p.ends_with(".js"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "cases missing");
    let mut failures = Vec::new();
    for js in &cases {
        let base = js.trim_end_matches(".js");
        let want_out = std::fs::read_to_string(format!("{base}.out")).unwrap();
        let want_code: i32 = std::fs::read_to_string(format!("{base}.code"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let got = Command::new(env!("CARGO_BIN_EXE_thaw-js-node"))
            .arg(js)
            .env("TZ", "America/New_York")
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&got.stdout);
        if out != want_out || got.status.code() != Some(want_code) {
            failures.push(format!(
                "{js}\n--- node (exit {want_code})\n{want_out}\n--- thaw-js (exit {:?})\n{out}\n{}",
                got.status.code(),
                String::from_utf8_lossy(&got.stderr)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n=====\n")
    );
}
