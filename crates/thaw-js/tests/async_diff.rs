//! Differential tests for Promise / async / timers / event-loop ordering.
//!
//! `tests/differential/async/<case>.js` are run by the `thaw-js-node` binary; stdout and the exit
//! code must equal `<case>.out` / `<case>.code`, which were recorded from real Node (v22):
//! `for f in *.js; do TZ=America/New_York node $f > ${f%.js}.out; echo $? > ${f%.js}.code; done`
//! (the harness pins `TZ` so the recorded local-time output is reproducible).

use std::process::Command;

#[test]
fn async_cases_match_recorded_node_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/async");
    let mut cases: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.path().to_str().map(str::to_string))
        .filter(|p| p.ends_with(".js"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 20, "cases missing");
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

/// Package scripts (`tests/differential/pkgs/<name>.js`, first line `// pkg: <npm name>`) run against the
/// real registry bundles; point `THAW_JS_CORPUS` at the directory holding `<npm name>.js` bundles.
#[test]
fn package_scripts_match_recorded_node_output() {
    let Ok(corpus) = std::env::var("THAW_JS_CORPUS") else {
        eprintln!("THAW_JS_CORPUS not set: skipping package script tests");
        return;
    };
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential/pkgs");
    let mut scripts: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.path().to_str().map(str::to_string))
        .filter(|p| p.ends_with(".js"))
        .collect();
    scripts.sort();
    let mut failures = Vec::new();
    for js in &scripts {
        let source = std::fs::read_to_string(js).unwrap();
        let pkg = source
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("// pkg: "))
            .unwrap()
            .trim();
        let bundle = format!("{corpus}/{pkg}.js");
        if !std::path::Path::new(&bundle).exists() {
            continue;
        }
        let want = std::fs::read_to_string(js.replace(".js", ".out")).unwrap();
        let got = Command::new(env!("CARGO_BIN_EXE_thaw-js-node"))
            .arg("--pkg")
            .arg(format!("{pkg}={bundle}"))
            .arg(js)
            .env("TZ", "America/New_York")
            .output()
            .unwrap();
        if String::from_utf8_lossy(&got.stdout) != want {
            failures.push(js.clone());
        }
    }
    assert!(failures.is_empty(), "differ from Node: {failures:?}");
}
