//! Opt-in end-to-end test of the AOT clones: real `thaw add` / `thaw build` of npm packages with and
//! without `THAW_JS_INFER=1`, outputs compared with Node. Needs network, node and a built `thaw`:
//!
//!     THAW_JS_INFER_THAW_BIN=/path/to/thaw cargo test -p thaw-js-infer --test e2e_registry -- --nocapture

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

struct Case {
    /// directory under tests/e2e and the npm package it uses
    name: &'static str,
    /// functions that must have been compiled ahead of time
    direct: &'static [&'static str],
}

fn run(cmd: &mut Command) -> (bool, String, f64) {
    let start = Instant::now();
    let out = cmd.stdin(Stdio::null()).output().expect("spawn");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
        start.elapsed().as_secs_f64(),
    )
}

fn case(thaw: &str, c: &Case) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let dir = Path::new(&std::env::var("TMPDIR").unwrap_or_else(|_| ".".into()))
        .join(format!("thaw-js-infer-e2e-{}", c.name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program =
        std::fs::read_to_string(format!("{manifest}/tests/e2e/{}/main.ts", c.name)).unwrap();
    std::fs::write(dir.join("main.ts"), &program).unwrap();
    std::fs::write(dir.join("ref.mts"), format!("{program}\nmain();\n")).unwrap();
    std::fs::write(
        dir.join("package.json"),
        r#"{"name":"t","version":"1.0.0"}"#,
    )
    .unwrap();

    let (ok, log, _) = run(Command::new(thaw)
        .current_dir(&dir)
        .args(["add", c.name, "--prefix", "."]));
    assert!(ok, "{}: thaw add failed\n{log}", c.name);
    let (ok, want, node_s) = run(Command::new("node").current_dir(&dir).args([
        "--experimental-strip-types",
        "--no-warnings",
        "ref.mts",
    ]));
    assert!(ok, "{}: node failed\n{want}", c.name);

    let build = |out: &str, infer: bool| {
        let _ = std::fs::remove_dir_all(dir.join("thaw_modules"));
        let mut cmd = Command::new(thaw);
        cmd.current_dir(&dir).args(["build", "main.ts", "-o", out]);
        if infer {
            cmd.env("THAW_JS_INFER", "1");
        }
        run(&mut cmd)
    };
    let (ok, log, _) = build("base.bin", false);
    assert!(ok, "{}: baseline build failed\n{log}", c.name);
    let (ok, log, _) = build("aot.bin", true);
    assert!(ok, "{}: THAW_JS_INFER build failed\n{log}", c.name);

    let (ok, base_out, base_s) = run(&mut Command::new(dir.join("base.bin")));
    assert!(
        ok && base_out == want,
        "{}: baseline differs from node\n{base_out}\n--- node\n{want}",
        c.name
    );
    let (ok, aot_out, aot_s) = run(&mut Command::new(dir.join("aot.bin")));
    assert!(
        ok && aot_out == want,
        "{}: AOT binary differs from node\n{aot_out}\n--- node\n{want}",
        c.name
    );

    let (_, inspect, _) = run(Command::new(thaw).arg("inspect").arg(dir.join("aot.bin")));
    for f in c.direct {
        assert!(
            inspect.contains(&format!(
                "js-infer-aot-direct: package={} function={f} ",
                c.name
            )),
            "{}: `{f}` was not compiled ahead of time\n{inspect}",
            c.name
        );
    }
    assert!(dir
        .join("thaw_modules")
        .join(c.name)
        .join("inferred.json")
        .is_file());
    let size = |n: &str| std::fs::metadata(dir.join(n)).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "{}: node {node_s:.2}s | thaw baseline {base_s:.2}s {} B | thaw AOT {aot_s:.2}s {} B | {:.1}x",
        c.name,
        size("base.bin"),
        size("aot.bin"),
        base_s / aot_s.max(1e-3)
    );
}

#[test]
fn aot_clones_match_node_and_beat_the_quickjs_calls() {
    let Ok(thaw) = std::env::var("THAW_JS_INFER_THAW_BIN") else {
        eprintln!("skipped: set THAW_JS_INFER_THAW_BIN to run");
        return;
    };
    for c in [
        Case {
            name: "date-fns",
            direct: &["yearsToMonths", "minutesToSeconds"],
        },
        Case {
            name: "bcryptjs",
            direct: &["getRounds", "getSalt", "truncates"],
        },
        Case {
            name: "just-clamp",
            direct: &["clamp"],
        },
    ] {
        case(&thaw, &c);
    }
}
