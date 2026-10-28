//! Opt-in type inference over installed package bundles (`THAW_JS_INFER=1`).
//!
//! Next to every `bundle.js.gz` that `add` writes, also write
//!   * `inferred.json`        -- census, per-function inferred signatures, trust sources, entry guards
//!   * `bundle.annotated.ts`  -- the bundle with the inferred TypeScript annotations
//!
//! Nothing reads these files unless `thaw build` is itself run with `THAW_JS_INFER=1` (it then reports
//! them in `thaw inspect`). Failures here never fail an install.

use std::path::Path;
use std::sync::Arc;

pub fn enabled() -> bool {
    std::env::var("THAW_JS_INFER").as_deref() == Ok("1")
}

/// Analyse `bundle` (JavaScript source) with `dts` (a `package.d.ts`) as declared-type hints.
/// Returns the JSON report and, when asked, the annotated TypeScript.
pub fn analyze(
    dts: Option<&str>,
    bundle: &str,
    want_annotated: bool,
) -> Result<(String, Option<String>), String> {
    analyze_with_program(dts, bundle, want_annotated, None)
}

/// Like [`analyze`], additionally using the calls the compiled program itself makes into the package
/// (`program` = package name and the program's TypeScript source) as trusted call-site evidence.
pub fn analyze_with_program(
    dts: Option<&str>,
    bundle: &str,
    want_annotated: bool,
    program: Option<(&str, &str)>,
) -> Result<(String, Option<String>), String> {
    let evidence = program.map(|(package, source)| {
        let mut ev = thaw_js_infer::evidence::Evidence::default();
        ev.add_program(package, source);
        Arc::new(ev)
    });
    let dts = dts
        .and_then(|text| thaw_js_infer::dts::DtsIndex::parse(text).ok())
        .map(Arc::new);
    let cfg = thaw_js_infer::Config {
        dts,
        evidence,
        returns: true,
        jsdoc: true,
        default_contract: true,
        ..Default::default()
    };
    let source = bundle.to_string();
    // bundles nest deeply: analyse on a thread with a large stack
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || {
            let (fns, summary) = thaw_js_infer::analyze_source(&source, cfg.clone())?;
            let annotated = if want_annotated {
                Some(thaw_js_infer::annotate(&source, cfg)?)
            } else {
                None
            };
            Ok((thaw_js_infer::report_json(&fns, &summary), annotated))
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "type inference panicked".to_string())?
}

/// Called after a bundle was written into `dir`.
pub fn emit(dir: &Path, bundle: &str) {
    if !enabled() {
        return;
    }
    let dts = std::fs::read_to_string(dir.join("package.d.ts")).ok();
    if let Ok((report, Some(annotated))) = analyze(dts.as_deref(), bundle, true) {
        let _ = std::fs::write(dir.join("inferred.json"), report);
        let _ = std::fs::write(dir.join("bundle.annotated.ts"), annotated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_reports_typed_exports_with_guards() {
        let (json, annotated) = analyze(
            Some("declare function twice(x: number): number;"),
            "exports.twice = function (x) { return x * 2; };",
            true,
        )
        .unwrap();
        assert!(
            json.contains("\"typed_functions\":[{\"name\":\"twice\""),
            "{json}"
        );
        assert!(
            json.contains("\"guard\":\"typeof x === 'number'\""),
            "{json}"
        );
        assert!(annotated.unwrap().contains("function (x: number)"));
    }

    #[test]
    fn nothing_is_written_unless_enabled() {
        // THAW_JS_INFER is not set in the test environment
        if enabled() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("thaw-infer-off-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        emit(&dir, "exports.f = function (a) { return a; };");
        assert!(!dir.join("inferred.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
