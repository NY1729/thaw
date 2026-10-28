//! Opt-in end-to-end test of the AOT clones: real `thaw add` / `thaw build` of npm packages with and
//! without `THAW_JS_INFER=1`, outputs compared with Node. Needs network, node and a built `thaw`:
//!
//!     THAW_JS_INFER_THAW_BIN=/path/to/thaw cargo test -p thaw-js-infer --test e2e_registry -- --nocapture

use std::process::{Command, Stdio};
use std::time::Instant;

struct Case {
    /// directory under tests/e2e
    name: &'static str,
    /// the npm package it uses (`name` when empty)
    package: &'static str,
    /// a package shipped with the tests (tests/e2e_local/<package>) instead of one fetched from npm
    local: bool,
    /// functions that must have been compiled ahead of time and called directly
    direct: &'static [&'static str],
    /// functions whose clone must sit behind a run-time guard (the static types prove nothing)
    guarded: &'static [&'static str],
}

impl Case {
    fn package_name(&self) -> &str {
        if self.package.is_empty() {
            self.name
        } else {
            self.package
        }
    }
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
    let dir = std::env::temp_dir().join(format!("thaw-js-infer-e2e-{}", c.name));
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

    if c.local {
        let target = dir.join("node_modules").join(c.package_name());
        std::fs::create_dir_all(&target).unwrap();
        let source = format!("{manifest}/tests/e2e_local/{}", c.package_name());
        for entry in std::fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    } else {
        let (ok, log, _) = run(Command::new(thaw).current_dir(&dir).args([
            "add",
            c.package_name(),
            "--prefix",
            ".",
        ]));
        assert!(ok, "{}: thaw add failed\n{log}", c.name);
    }
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
    for f in c.guarded {
        assert!(
            inspect.contains(&format!(
                "js-infer-aot-guarded: package={} function={f} ",
                c.package_name()
            )),
            "{}: `{f}` is not behind a guard\n{inspect}",
            c.name
        );
    }
    for f in c.direct {
        assert!(
            inspect.contains(&format!(
                "js-infer-aot-direct: package={} function={f} ",
                c.package_name()
            )),
            "{}: `{f}` was not compiled ahead of time\n{inspect}",
            c.name
        );
    }
    assert!(dir
        .join("thaw_modules")
        .join(c.package_name())
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
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipped: node is not installed");
        return;
    }
    for c in [
        Case {
            name: "date-fns",
            package: "",
            local: false,
            direct: &["yearsToMonths", "minutesToSeconds"],
            guarded: &[],
        },
        Case {
            name: "bcryptjs",
            package: "",
            local: false,
            direct: &["getRounds", "getSalt", "truncates"],
            guarded: &[],
        },
        Case {
            name: "just-clamp",
            package: "",
            local: false,
            direct: &["clamp"],
            guarded: &[],
        },
        // array parameters (copied at the typed-call boundary, in-bounds element reads proven)
        Case {
            name: "simple-statistics",
            package: "",
            local: false,
            direct: &[
                "sum",
                "mean",
                "max",
                "min",
                "product",
                "geometricMean",
                "zScore",
            ],
            guarded: &[],
        },
        // the same package with `Json` arguments: guarded, and a contract violation matches Node
        Case {
            name: "simple-statistics-guard",
            package: "simple-statistics",
            local: false,
            direct: &[],
            guarded: &["sum", "max"],
        },
        // records, string / boolean arrays; the mutating `grow` stays a QuickJS call
        Case {
            name: "aot-shapes",
            package: "aot-shapes",
            local: true,
            direct: &["area", "label", "longest", "allTrue"],
            guarded: &[],
        },
        Case {
            name: "aot-shapes-guard",
            package: "aot-shapes",
            local: true,
            direct: &[],
            guarded: &["area", "label", "longest"],
        },
        Case {
            name: "just-is-prime",
            package: "",
            local: false,
            direct: &["isPrime"],
            guarded: &[],
        },
        // an array result (a fresh `number[]` built by the clone), a different package family
        Case {
            name: "simple-statistics-array",
            package: "simple-statistics",
            local: false,
            direct: &["numericSort"],
            guarded: &[],
        },
        // arrays built by the clone (a result array is fresh); `same` (returns its argument) is not cloned
        Case {
            name: "aot-arrays",
            package: "aot-arrays",
            local: true,
            direct: &["evens", "scale", "words"],
            guarded: &[],
        },
        Case {
            name: "more-rounding",
            package: "",
            local: false,
            direct: &["round", "roundUp", "roundDown"],
            guarded: &[],
        },
        Case {
            name: "leap-year-checker",
            package: "",
            local: false,
            direct: &["isLeapYear"],
            guarded: &[],
        },
        // an anonymous arrow exported as `module.exports`, typed by `export = lerp`
        Case {
            name: "lerpjs",
            package: "",
            local: false,
            direct: &["lerp"],
            guarded: &[],
        },
        // nothing is cloned here: the opt-in build must still produce the baseline's output
        Case {
            name: "uuid",
            package: "",
            local: false,
            direct: &[],
            guarded: &[],
        },
        Case {
            name: "marked",
            package: "",
            local: false,
            direct: &[],
            guarded: &[],
        },
        Case {
            name: "dayjs",
            package: "",
            local: false,
            direct: &[],
            guarded: &[],
        },
    ] {
        case(&thaw, &c);
    }
}
