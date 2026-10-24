//! Differential check of real registry bundles against recorded Node output.
//!
//! `tests/differential/cases.json` holds `{pkg, path, args, expect}` cases whose `expect` was produced by
//! real Node (`tests/differential/run_diff.py --write`). The bundles themselves are derived from npm
//! (`thaw_modules/<pkg>/bundle.js.gz` after a `thaw build`), so they are not checked in: point
//! `THAW_JS_CORPUS` at a directory holding `<pkg>.js` files to run; without it the test is skipped.

use std::collections::HashMap;
use thaw_js::host::{Native, Package};

fn field<'a>(o: &'a [(String, Native)], k: &str) -> &'a Native {
    &o.iter()
        .find(|(n, _)| n == k)
        .unwrap_or_else(|| panic!("missing {k}"))
        .1
}

#[test]
fn real_bundles_match_recorded_node_output() {
    let Ok(dir) = std::env::var("THAW_JS_CORPUS") else {
        eprintln!("THAW_JS_CORPUS not set: skipping real-bundle differential test");
        return;
    };
    let cases = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/differential/cases.json"
    ))
    .unwrap();
    // the interpreter recurses natively; give it a big stack like a production host thread would
    let handle = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let Native::Array(cases) = Native::from_json(&cases).unwrap() else {
                panic!("cases must be an array")
            };
            let mut packages: HashMap<String, Option<Package>> = HashMap::new();
            let (mut checked, mut failures) = (0, Vec::new());
            for case in &cases {
                let Native::Object(c) = case else {
                    panic!("case must be an object")
                };
                let (Native::Str(pkg), Native::Str(path), Native::Str(expect)) =
                    (field(c, "pkg"), field(c, "path"), field(c, "expect"))
                else {
                    panic!("bad case")
                };
                let bundle = format!("{dir}/{pkg}.js");
                let entry = packages.entry(pkg.clone()).or_insert_with(|| {
                    std::fs::read_to_string(&bundle)
                        .ok()
                        .map(|s| Package::load(&s).unwrap_or_else(|e| panic!("{pkg}: {e}")))
                });
                let Some(package) = entry else { continue };
                let got = match field(c, "args") {
                    Native::Array(args) => package.call(path, args),
                    _ => package.get(path),
                };
                // Node's JSON output is the oracle; an exception on both sides counts as agreement
                let got = match got {
                    Ok(v) => v.to_json(),
                    Err(_) => "ERROR".to_string(),
                };
                checked += 1;
                if &got != expect {
                    failures.push(format!("{pkg}.{path}: thaw-js {got} != node {expect}"));
                }
            }
            assert!(
                failures.is_empty(),
                "{} of {checked} cases differ:\n{}",
                failures.len(),
                failures.join("\n")
            );
            eprintln!("{checked} real-bundle cases match Node");
        })
        .unwrap();
    handle.join().unwrap();
}
