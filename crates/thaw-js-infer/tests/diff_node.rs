//! Differential test: annotate JS -> TypeScript, compile it with the existing `thaw` TS pipeline and
//! compare stdout with Node. Opt-in: set `THAW_JS_INFER_THAW_BIN=/path/to/thaw` (and have `node` on
//! PATH); otherwise the test is skipped. Scratch goes under `$TMPDIR`.

use std::process::Command;

fn run_case(thaw: &str, name: &str) {
    let src_path = format!("{}/tests/cases/{name}.js", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&src_path).unwrap();
    let mut cfg = thaw_js_infer::Config {
        returns: true,
        ..Default::default()
    };
    if let Ok(d) = std::fs::read_to_string(src_path.replace(".js", ".d.ts")) {
        cfg.dts = Some(std::sync::Arc::new(
            thaw_js_infer::dts::DtsIndex::parse(&d).unwrap(),
        ));
    }
    let ts = thaw_js_infer::annotate(&src, cfg).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| ".".into()))
        .join(format!("thaw-js-infer-diff-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("package.json"),
        r#"{"name":"t","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("prog.ts"), &ts).unwrap();
    let out = Command::new(thaw)
        .current_dir(&dir)
        .args(["build", "prog.ts", "-o", "prog.bin"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{name}: thaw build failed:\n{}\n--- annotated ---\n{ts}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ours = Command::new(dir.join("prog.bin"))
        .current_dir(&dir)
        .output()
        .unwrap();
    let node = Command::new("node").arg(&src_path).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ours.stdout),
        String::from_utf8_lossy(&node.stdout),
        "{name}: thaw output differs from node\n--- annotated ---\n{ts}"
    );
}

#[test]
fn annotated_js_matches_node() {
    let Ok(thaw) = std::env::var("THAW_JS_INFER_THAW_BIN") else {
        eprintln!("skipped: set THAW_JS_INFER_THAW_BIN to run");
        return;
    };
    for case in [
        "numeric_string",
        "union_uninit",
        "array_callbacks",
        "dts_api",
        "classes",
        "narrowing",
        "hierarchy",
    ] {
        run_case(&thaw, case);
    }
}
