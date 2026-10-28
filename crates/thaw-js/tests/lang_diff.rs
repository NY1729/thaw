//! Differential tests for language features and builtins (generators, classes, BigInt, Proxy, ...).
//!
//! `tests/differential/lang/<case>.js` are run by the `thaw-js-node` binary; stdout must equal
//! `<case>.out`, recorded from real Node (v22):
//! `for f in *.js; do LANG=en_US.UTF-8 TZ=America/New_York node $f > ${f%.js}.out; done`

use std::process::Command;

#[test]
fn lang_cases_match_recorded_node_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/lang");
    let mut cases: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.path().to_str().map(str::to_string))
        .filter(|p| p.ends_with(".js"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 15, "cases missing");
    let mut failures = Vec::new();
    for js in &cases {
        let want = std::fs::read_to_string(js.replace(".js", ".out")).unwrap();
        let got = Command::new(env!("CARGO_BIN_EXE_thaw-js-node"))
            .arg(js)
            .env("TZ", "America/New_York")
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&got.stdout);
        if out != want || got.status.code() != Some(0) {
            failures.push(format!(
                "{js}\n--- node\n{want}\n--- thaw-js (exit {:?})\n{out}\n{}",
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
