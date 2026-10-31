//! Differential tests for `Intl` (cargo feature `intl`): `tests/differential/intl/<case>.js` run by
//! `thaw-js-node`; stdout must equal `<case>.out`, recorded from real Node v22.23 (ICU 78.2):
//! `for f in *.js; do LANG=en_US.UTF-8 TZ=America/New_York node $f > ${f%.js}.out; done`
//! The baked CLDR data (icu4x) and Node's ICU can differ by release; a diff in a locale string is
//! first suspected to be a CLDR version difference.
#![cfg(feature = "intl")]

use std::process::Command;

#[test]
fn intl_cases_match_recorded_node_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/intl");
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let js = entry.unwrap().path().to_str().unwrap().to_string();
        if !js.ends_with(".js") {
            continue;
        }
        count += 1;
        let want = std::fs::read_to_string(js.replace(".js", ".out")).unwrap();
        let got = Command::new(env!("CARGO_BIN_EXE_thaw-js-node"))
            .arg(&js)
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
    assert!(count >= 2, "cases missing");
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n=====\n")
    );
}
