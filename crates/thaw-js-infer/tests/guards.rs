//! Entry guards: the guarded TypeScript output must behave exactly like the original JavaScript for
//! callers that honour the contract AND for callers that violate it. Executed with Node's type
//! stripping (the annotations are all erasable syntax), so no Thaw build is needed.

use std::process::Command;
use thaw_js_infer::{annotate, dts::DtsIndex, Config};

/// Node >= 22.6 with `--experimental-strip-types` (and `require` of a `.ts` file) is needed; skip otherwise.
fn node_can_strip_types() -> bool {
    let dir = std::env::temp_dir().join(format!("thaw-js-infer-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("m.ts"),
        "exports.x = (1 as number);\nexports.f = function (a: number): number { return a; };",
    )
    .unwrap();
    std::fs::write(
        dir.join("d.js"),
        "console.log(require(require('path').resolve(process.argv[2])).f(7));",
    )
    .unwrap();
    let ok = Command::new("node")
        .args(["--experimental-strip-types", "--no-warnings"])
        .arg(dir.join("d.js"))
        .arg(dir.join("m.ts"))
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "7");
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

fn check(js: &str, driver: &str, dts: &str, must_contain: &[&str]) {
    if !node_can_strip_types() {
        eprintln!("skipped: node with --experimental-strip-types not available");
        return;
    }
    let cfg = Config {
        dts: Some(std::sync::Arc::new(DtsIndex::parse(dts).unwrap())),
        returns: true,
        guards: true,
        ..Default::default()
    };
    let ts = annotate(js, cfg).unwrap();
    for m in must_contain {
        assert!(ts.contains(m), "missing `{m}` in:\n{ts}");
    }
    let dir = std::env::temp_dir().join(format!("thaw-js-infer-guards-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (jsf, tsf, drv) = (
        dir.join("orig.js"),
        dir.join("guarded.ts"),
        dir.join("driver.js"),
    );
    std::fs::write(&jsf, js).unwrap();
    std::fs::write(&tsf, &ts).unwrap();
    std::fs::write(
        &drv,
        format!("const m = require(require('path').resolve(process.argv[2]));\n{driver}"),
    )
    .unwrap();
    let run = |args: &[&str], module: &std::path::Path| {
        let out = Command::new("node")
            .args(args)
            .arg(&drv)
            .arg(module)
            .output()
            .expect("node");
        assert!(
            out.status.success(),
            "node failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let want = run(&[], &jsf);
    let got = run(&["--experimental-strip-types", "--no-warnings"], &tsf);
    assert_eq!(got, want, "guarded output differs\n{ts}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn guarded_functions_match_the_original_for_valid_and_invalid_callers() {
    check(
        "exports.add = function (a, b) { return a + b; };\n\
         exports.greet = function (name) { return 'hi ' + name; };\n\
         exports.total = function (xs) { var s = 0; for (var i = 0; i < xs.length; i++) { s += xs[i]; } return s; };\n\
         exports.opt = function (o) { return o.n + (o.s || ''); };\n",
        "console.log(m.add(1, 2), m.add('1', 2), m.add(undefined, 1));\n\
         console.log(m.greet('x'), m.greet(5), m.greet(null));\n\
         console.log(m.total([1, 2, 3]), m.total(['a', 'b']), m.total([1, 'x']));\n\
         console.log(m.opt({ n: 1, s: 'a' }), m.opt({ n: 'q' }), m.opt({ n: 2, s: 5 }));\n",
        "declare function add(a: number, b: number): number;\n\
         declare function greet(name: string): string;\n\
         declare function total(xs: number[]): number;\n\
         declare function opt(o: { n: number; s?: string }): string;\n",
        &["typeof a === 'number'", "Array.isArray(xs)", "typeof o === 'object'", ".call(this, a, b)"],
    );
}
