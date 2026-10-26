// Opt-in (THAW_JS_INFER=1): type-inference census of every registry package bundle used by a build,
// recorded in the artifact manifest as `js_infer` and printed by `thaw inspect`.

fn js_infer_entries(registry_dir: &Path, packages: &[String], user_source: &str) -> serde_json::Value {
    let entries = packages.iter().filter_map(|package| {
        let resolved = thaw_registry::resolve(registry_dir, package).ok()?;
        let bundle = resolved.bundle_js?;
        Some(
            match thaw_registry::infer::analyze_with_program(Some(&resolved.dts_source), &bundle, false, Some((package.as_str(), user_source))) {
                Ok((json, _)) => {
                    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
                    serde_json::json!({
                        "package": package,
                        "summary": value["summary"],
                        "typed_functions": value["typed_functions"].as_array().map_or(0, Vec::len),
                    })
                }
                Err(error) => serde_json::json!({ "package": package, "error": error }),
            },
        )
    });
    serde_json::Value::Array(entries.collect())
}

fn js_infer_inspect_lines(manifest: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(aot) = manifest["js_infer_aot"].as_object() {
        for entry in aot["patched"].as_array().into_iter().flatten() {
            lines.push(format!(
                "js-infer-aot: package={} function={} clone={}",
                entry["package"].as_str().unwrap_or("?"),
                entry["function"].as_str().unwrap_or("?"),
                entry["clone"].as_str().unwrap_or("?"),
            ));
        }
        for entry in aot["retargeted"].as_array().into_iter().flatten() {
            lines.push(format!(
                "js-infer-aot-direct: package={} function={} clone={}",
                entry["package"].as_str().unwrap_or("?"),
                entry["function"].as_str().unwrap_or("?"),
                entry["clone"].as_str().unwrap_or("?"),
            ));
        }
        for entry in aot["guarded"].as_array().into_iter().flatten() {
            lines.push(format!(
                "js-infer-aot-guarded: package={} function={} wrapper={}",
                entry["package"].as_str().unwrap_or("?"),
                entry["function"].as_str().unwrap_or("?"),
                entry["wrapper"].as_str().unwrap_or("?"),
            ));
        }
        for entry in aot["rejected"].as_array().into_iter().flatten() {
            lines.push(format!(
                "js-infer-aot-rejected: package={} function={} reason={}",
                entry["package"].as_str().unwrap_or("?"),
                entry["function"].as_str().unwrap_or("?"),
                entry["error"].as_str().unwrap_or("?"),
            ));
        }
    }
    let Some(entries) = manifest["js_infer"].as_array() else {
        return lines;
    };
    for entry in entries {
        let package = entry["package"].as_str().unwrap_or("?");
        if let Some(error) = entry["error"].as_str() {
            lines.push(format!("js-infer: package={package} error={error}"));
            continue;
        }
        let s = &entry["summary"];
        let n = |key: &str| s[key].as_u64().unwrap_or(0);
        let typed = n("concrete") + n("nullable") + n("union");
        lines.push(format!(
            "js-infer: package={package} functions={} typed={typed} (concrete={} nullable={} union={}) any={} closed={} sound={} guarded={}",
            n("functions"),
            n("concrete"),
            n("nullable"),
            n("union"),
            n("any"),
            n("closed"),
            n("sound_typed"),
            n("guarded"),
        ));
        if let Some(reasons) = s["any_reasons"].as_object() {
            let mut sorted: Vec<(&String, u64)> =
                reasons.iter().map(|(k, v)| (k, v.as_u64().unwrap_or(0))).collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let top = sorted
                .iter()
                .take(6)
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
            if !top.is_empty() {
                lines.push(format!("js-infer-any: package={package} {top}"));
            }
        }
    }
    lines
}

#[cfg(test)]
mod js_infer_report_tests {
    use super::*;

    fn sample_clone() -> thaw_js_infer::clone::AotClone {
        use thaw_js_infer::clone::Prim;
        thaw_js_infer::clone::AotClone {
            exports: vec!["twice".into()],
            fn_name: "__thaw_aot_pkg_0_twice".into(),
            params: vec![Prim::Num.into()],
            ret: Prim::Num,
            ts: "function __thaw_aot_pkg_0_twice(x: number): number { return x * 2; }".into(),
        }
    }

    #[test]
    fn wrappers_become_guarded_dispatchers_and_the_signature_match_retargets() {
        let shim = "declare function __thaw_typed_js_706b673a3a7477696365(x: number): number;\n\
                    // Fallback\n\
                    function twice(argsArray: Json): Json {\n    return callDynamic(\"twice\", argsArray);\n}\n\
                    function other(argsArray: Json): Json {\n    return callDynamic(\"other\", argsArray);\n}\n"
            .to_string();
        let mut report = JsInferAot::default();
        let out = js_infer_apply(shim, &[("pkg".to_string(), sample_clone())], &mut report);
        assert!(out.contains("if (argsArray.length === 1) {"), "{out}");
        assert!(out.contains("typeof __a0 === \"number\""), "{out}");
        assert!(out.contains("return __thaw_aot_pkg_0_twice(__a0);"), "{out}");
        assert!(out.contains("return callDynamic(\"twice\", argsArray);"), "{out}");
        // untouched neighbour, clone appended once
        assert!(out.contains("function other(argsArray: Json): Json {\n    return callDynamic(\"other\", argsArray);\n}"));
        assert_eq!(out.matches("function __thaw_aot_pkg_0_twice(x: number)").count(), 1);
        // the typed declaration has the clone's signature: exports bound to it move
        assert_eq!(report.symbol_moves.len(), 1);
        assert_eq!(report.symbol_moves[0].1, "__thaw_typed_js_706b673a3a7477696365");
        // a different declared signature is not retargeted
        let shim = "declare function __thaw_typed_js_706b673a3a7477696365(x: string): number;\n".to_string();
        let mut report = JsInferAot::default();
        js_infer_apply(shim, &[("pkg".to_string(), sample_clone())], &mut report);
        assert!(report.symbol_moves.is_empty());
    }

    fn array_clone() -> thaw_js_infer::clone::AotClone {
        use thaw_js_infer::clone::{CTy, Prim};
        thaw_js_infer::clone::AotClone {
            exports: vec!["total".into()],
            fn_name: "__thaw_aot_pkg_1_total".into(),
            params: vec![CTy::Arr(Prim::Num), CTy::Prim(Prim::Num)],
            ret: Prim::Num,
            ts: "function __thaw_aot_pkg_1_total(xs: number[], k: number): number { return xs.length * k; }".into(),
        }
    }

    fn array_shim() -> String {
        "declare function __thaw_typed_js_706b673a3a746f74616c(xs: number[], k: number): number;\n".to_string()
    }

    #[test]
    fn the_programs_static_types_decide_between_a_direct_call_and_a_guard() {
        let run = |program: &str| {
            let mut report = JsInferAot::default();
            let mut evidence = thaw_js_infer::evidence::Evidence::default();
            evidence.add_program("pkg", program);
            report.evidence.insert("pkg".to_string(), evidence);
            let out = js_infer_apply(array_shim(), &[("pkg".to_string(), array_clone())], &mut report);
            (out, report)
        };
        // arguments of evident types: the clone is called directly
        let (out, report) = run(
            "import { total } from \"pkg\";\nconst xs: number[] = [1, 2];\nconsole.log(total(xs, 3), total([4], 2));",
        );
        assert_eq!(report.symbol_moves[0].2, "__thaw_aot_pkg_1_total", "{out}");
        assert!(report.guarded.is_empty());
        // an argument whose type is not evident (a call result, `Json`): a run-time guard
        let (out, report) = run(
            "import { total } from \"pkg\";\nconst j: Json = JSON.parse(\"[1]\");\nconsole.log(total(j, 3));",
        );
        assert_eq!(report.symbol_moves[0].2, "__thaw_aot_pkg_1_total__g_total", "{out}");
        assert!(out.contains("function __thaw_aot_pkg_1_total__g_total(a0: Json, a1: Json): number {"), "{out}");
        assert!(out.contains("__thaw_aot_pkg_1_total_p0_ok(a0) && typeof a1 === \"number\""), "{out}");
        assert!(out.contains("return __thaw_aot_pkg_1_total(__thaw_aot_pkg_1_total_p0_cv(a0), a1);"), "{out}");
        assert!(out.contains("return __thaw_typed_js_706b673a3a746f74616c(a0, a1);"), "{out}");
        assert!(out.contains("function __thaw_aot_pkg_1_total_p0_cv(v: Json): number[]"), "{out}");
        // the function used as a value, or several files: never proven
        for program in [
            "import { total } from \"pkg\";\nconst f = total;\nconsole.log(f([1], 2), total([1], 2));",
            "import { total } from \"pkg\";\nimport { x } from \"./other\";\nconsole.log(total([1], 2));",
            "import * as p from \"pkg\";\nconsole.log(p.total([1], 2), p);",
        ] {
            let (_, report) = run(program);
            assert_eq!(report.guarded.len(), 1, "{program}");
        }
    }

    #[test]
    fn helper_names_that_the_program_declares_are_collected() {
        assert_eq!(
            js_infer_top_level_names("function sum(a) {}\nconst f = function g() {};\nasync function h() {}"),
            vec!["sum".to_string(), "g".to_string(), "h".to_string()]
        );
    }

    #[test]
    fn retargeted_functions_stop_being_quickjs_fallback_reasons() {
        let aot = JsInferAot {
            retargets: vec![("pkg".into(), "twice".into(), "f".into())],
            ..Default::default()
        };
        let reasons = vec![
            serde_json::json!({"kind": "registry-fallback", "package": "pkg", "operation": "twice"}),
            serde_json::json!({"kind": "registry-fallback", "package": "pkg", "operation": "other"}),
        ];
        let kept = js_infer_retain_reasons(reasons, &aot);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["operation"], "other");
    }

    #[test]
    fn inspect_lines_summarise_the_census() {
        let manifest = serde_json::json!({ "js_infer": [
            { "package": "ms", "typed_functions": 2, "summary": {
                "functions": 70, "concrete": 5, "nullable": 1, "union": 0, "any": 60, "closed": 3,
                "sound_typed": 4, "guarded": 2, "any_reasons": { "Escaped": 40, "This": 9 } } },
            { "package": "bad", "error": "boom" }
        ]});
        let lines = js_infer_inspect_lines(&manifest);
        assert_eq!(
            lines,
            vec![
                "js-infer: package=ms functions=70 typed=6 (concrete=5 nullable=1 union=0) any=60 closed=3 sound=4 guarded=2",
                "js-infer-any: package=ms Escaped=40 This=9",
                "js-infer: package=bad error=boom",
            ]
        );
        assert!(js_infer_inspect_lines(&serde_json::json!({})).is_empty());
    }
}

// ---- AOT clones (THAW_JS_INFER=1) -------------------------------------------------------------
//
// An exported package function whose signature is a plain number/string/boolean contract from the
// package's `.d.ts` and whose body is self-contained (see thaw-js-infer `clone`) is compiled
// ahead of time by the TS pipeline next to the program. The generated registry shim wraps the call
//
//     function f(argsArray: Json): Json { return callDynamic("f", argsArray); }
//
// into a guarded dispatcher: when the arguments have the contracted types the typed clone runs,
// otherwise the original QuickJS call runs unchanged.

/// What was compiled ahead of time, for the manifest / `thaw inspect`.
#[derive(Default)]
struct JsInferAot {
    /// (package, export name, clone function)
    patched: Vec<(String, String, String)>,
    /// clones that were extracted but rejected by the TS pipeline
    rejected: Vec<(String, String, String)>,
    /// (package, export name, clone function): the program's imports are re-pointed at the clone
    retargets: Vec<(String, String, String)>,
    /// (package, typed fallback symbol, clone function): every export bound to the symbol moves
    symbol_moves: Vec<(String, String, String)>,
    /// what the program evidently passes to each package (empty = no information, e.g. unit tests)
    evidence: std::collections::HashMap<String, thaw_js_infer::evidence::Evidence>,
    /// guard wrappers that were generated instead of a direct re-point (static types did not prove the contract)
    guarded: Vec<(String, String, String)>,
}

fn js_infer_clones(
    registry_dir: &Path,
    packages: &[String],
    user_source: &str,
) -> Vec<(String, thaw_js_infer::clone::AotClone, Result<(), String>)> {
    let mut out = Vec::new();
    for package in packages {
        let Ok(resolved) = thaw_registry::resolve(registry_dir, package) else { continue };
        let Some(bundle) = resolved.bundle_js else { continue };
        let dts = thaw_js_infer::dts::DtsIndex::parse(&resolved.dts_source).ok();
        let cfg = thaw_js_infer::Config {
            dts: dts.map(std::sync::Arc::new),
            reserved_names: js_infer_top_level_names(user_source),
            ..Default::default()
        };
        let prefix = format!("__thaw_aot_{}_", package.replace(|c: char| !c.is_alphanumeric(), "_"));
        let clones = std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(move || thaw_js_infer::clone::extract_clones(&bundle, cfg, &prefix))
            .ok()
            .and_then(|handle| handle.join().ok())
            .and_then(Result::ok)
            .unwrap_or_default();
        for clone in clones {
            // debugging aid: THAW_JS_INFER_SKIP=name,name leaves those functions out
            if std::env::var("THAW_JS_INFER_SKIP").is_ok_and(|skip| {
                skip.split(',').any(|n| clone.exports.iter().any(|e| e == n))
            }) {
                continue;
            }
            // only functions the program mentions are worth compiling
            if !clone.exports.iter().any(|name| user_source.contains(name.as_str())) {
                continue;
            }
            // lowering is lazy: call the clone so that its body is really checked
            let probe = format!(
                "{}\nfunction __thaw_aot_probe(): void {{ const r = {}({}); }}\n{}",
                clone.ts,
                clone.fn_name,
                clone.params.iter().map(thaw_js_infer::clone::CTy::dummy).collect::<Vec<_>>().join(", "),
                clone.helpers()
            );
            let verdict = thaw_parser::parse_typescript(&probe)
                .and_then(|module| thaw_hir::lower_module(&module).map(|_| ()));
            out.push((package.clone(), clone, verdict));
        }
    }
    out
}

/// Names declared with `function NAME` in the program (a clone's helper must not share one).
fn js_infer_top_level_names(source: &str) -> Vec<String> {
    source
        .split("function ")
        .skip(1)
        .map(|rest| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect::<String>())
        .filter(|name| !name.is_empty())
        .collect()
}

fn js_infer_dispatcher(
    name: &str,
    key: &str,
    clone: &thaw_js_infer::clone::AotClone,
) -> String {
    let n = clone.params.len();
    let loads: String = (0..n).map(|i| format!("        const __a{i} = argsArray[{i}];\n")).collect();
    // primitives are checked inline; arrays / records by the generated `_ok` helper
    let helper = |i: usize| format!("{}_p{i}", clone.fn_name);
    let guard = clone
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| p.guard(&format!("__a{i}")).unwrap_or_else(|| format!("{}_ok(__a{i})", helper(i))))
        .collect::<Vec<_>>()
        .join(" && ");
    let args = clone
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| match p {
            thaw_js_infer::clone::CTy::Prim(_) => format!("__a{i}"),
            _ => format!("{}_cv(__a{i})", helper(i)),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let trace = if std::env::var("THAW_JS_INFER_TRACE").is_ok() {
        format!("    console.log(\"aot-dispatch {name}\", argsArray.length);\n")
    } else {
        String::new()
    };
    format!(
        "function {name}(argsArray: Json): Json {{\n{trace}    if (argsArray.length === {n}) {{\n{loads}        if ({guard}) {{ return {}({args}); }}\n    }}\n    return callDynamic(\"{key}\", argsArray);\n}}",
        clone.fn_name
    )
}

/// Rewrite the registry shim: guarded dispatchers for the verified clones, plus the clones.
fn js_infer_patch_shim(
    registry_shim: String,
    registry_dir: &Path,
    packages: &[String],
    user_source: &str,
) -> (String, JsInferAot) {
    let mut report = JsInferAot::default();
    let clones = js_infer_clones(registry_dir, packages, user_source);
    for package in packages {
        let mut evidence = thaw_js_infer::evidence::Evidence {
            returns: js_infer_typed_returns(&registry_shim, package),
            ..Default::default()
        };
        // `import clamp from "pkg"` binds the default export, which is the clone's other name
        for (p, clone, _) in &clones {
            if p == package && clone.exports.iter().any(|e| e == thaw_js_infer::evidence::DEFAULT) {
                if let Some(t) = clone.exports.iter().find_map(|e| evidence.returns.get(e)).cloned() {
                    evidence.returns.insert(thaw_js_infer::evidence::DEFAULT.to_string(), t);
                }
            }
        }
        evidence.add_program(package, user_source);
        report.evidence.insert(package.clone(), evidence);
    }
    let mut accepted = Vec::new();
    for (package, clone, verdict) in clones {
        match verdict {
            Ok(()) => accepted.push((package, clone)),
            Err(error) => report.rejected.push((package, clone.exports[0].clone(), error)),
        }
    }
    let shim = js_infer_apply(registry_shim, &accepted, &mut report);
    (shim, report)
}

/// The pure part of [`js_infer_patch_shim`]: rewrite `shim` for the accepted clones.
fn js_infer_apply(
    registry_shim: String,
    accepted: &[(String, thaw_js_infer::clone::AotClone)],
    report: &mut JsInferAot,
) -> String {
    if accepted.is_empty() {
        return registry_shim;
    }
    let lines: Vec<&str> = registry_shim.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 8);
    let mut used = std::collections::BTreeSet::new();
    let mut guards: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let wrapper = (i + 2 < lines.len())
            .then(|| {
                let head = lines[i].strip_prefix("function ")?;
                let name = head.strip_suffix("(argsArray: Json): Json {")?;
                let key = lines[i + 1]
                    .strip_prefix("    return callDynamic(\"")?
                    .strip_suffix("\", argsArray);")?;
                (lines[i + 2] == "}").then_some((name, key))
            })
            .flatten();
        if let Some((name, key)) = wrapper {
            let hit = accepted.iter().find(|(package, clone)| {
                clone
                    .exports
                    .iter()
                    .any(|e| key == e.as_str() || key == format!("{package}::{e}"))
            });
            if let Some((package, clone)) = hit {
                out.push(js_infer_dispatcher(name, key, clone));
                used.insert(clone.fn_name.clone());
                report
                    .patched
                    .push((package.clone(), key.to_string(), clone.fn_name.clone()));
                i += 3;
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut shim = out.join("\n");
    // typed fallback declaration with exactly the clone's signature: the program's import is
    // re-pointed at the clone (the compiler's static types are the guard); the declaration stays
    // for every other path that still names it
    for (package, clone) in accepted {
        for export in &clone.exports {
            let key = format!("{package}::{export}");
            let symbol = format!("__thaw_typed_js_{}", key.bytes().map(|b| format!("{b:02x}")).collect::<String>());
            let prefix = format!("declare function {symbol}(");
            let Some(start) = shim.find(&prefix) else { continue };
            let end = shim[start..].find('\n').map_or(shim.len(), |e| start + e);
            let line = shim[start..end].to_string();
            let signature = &line[prefix.len()..];
            if js_infer_signature_matches(signature, clone) {
                // The program's static types prove the contract: call the clone directly. Otherwise
                // (an argument of unknown static type, an escaping reference, several files) the
                // symbol moves to a wrapper that takes `Json`, checks the contract at run time and
                // falls back to the original typed QuickJS call - the dynamic path - when it fails.
                let proven = js_infer_proven(report, package, clone);
                let target = if proven {
                    report.retargets.push((package.clone(), export.clone(), clone.fn_name.clone()));
                    clone.fn_name.clone()
                } else {
                    let name = format!(
                        "{}__g_{}",
                        clone.fn_name,
                        export.replace(|c: char| !c.is_alphanumeric(), "_")
                    );
                    guards.push(js_infer_guard_wrapper(&name, &symbol, clone));
                    report.guarded.push((package.clone(), export.clone(), name.clone()));
                    name
                };
                report.symbol_moves.push((package.clone(), symbol.clone(), target));
                used.insert(clone.fn_name.clone());
            }
        }
    }
    for guard in &guards {
        shim.push('\n');
        shim.push_str(guard);
    }
    for (_, clone) in accepted {
        if used.contains(&clone.fn_name) {
            shim.push('\n');
            shim.push_str(&clone.ts);
            shim.push('\n');
            shim.push_str(&clone.helpers());
        }
    }
    shim
}

/// `(a: number, b: string[]): ret;` (the declaration text after its name) against the clone's types.
fn js_infer_signature_matches(signature: &str, clone: &thaw_js_infer::clone::AotClone) -> bool {
    use thaw_js_infer::clone::CTy;
    let Some(close) = signature.rfind("):") else { return false };
    let params = &signature[..close];
    let ret = signature[close + 2..].trim().trim_end_matches(';');
    let declared: Vec<&str> = if params.trim().is_empty() { vec![] } else { js_infer_split_top(params) };
    declared.len() == clone.params.len()
        && CTy::parse(ret) == Some(CTy::from(clone.ret))
        && declared.iter().zip(&clone.params).all(|(d, p)| {
            d.split_once(':').and_then(|(_, t)| CTy::parse(t)).as_ref() == Some(p)
        })
}

/// Split at commas that are not inside `{}` / `[]` / `()` / `<>`.
fn js_infer_split_top(text: &str) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
    for (i, ch) in text.char_indices() {
        match ch {
            '{' | '[' | '(' | '<' => depth += 1,
            '}' | ']' | ')' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// Result types of the package's typed fallback declarations in the generated shim (export -> type):
/// `declare function __thaw_typed_js_<hex(pkg::name)>(..): number;`.
fn js_infer_typed_returns(
    shim: &str,
    package: &str,
) -> std::collections::HashMap<String, thaw_js_infer::dts::DT> {
    // arity / overload variants (`..__arity_2`) share a base name: the result type must agree
    let mut seen: std::collections::HashMap<String, Option<thaw_js_infer::dts::DT>> = Default::default();
    for line in shim.lines() {
        let Some(rest) = line.strip_prefix("declare function __thaw_typed_js_") else { continue };
        let Some((symbol, signature)) = rest.split_once('(') else { continue };
        let hex = symbol.split("__").next().unwrap_or(symbol);
        let bytes: Option<Vec<u8>> = (0..hex.len() / 2)
            .map(|i| hex.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
            .collect();
        let Some(key) = bytes.and_then(|b| String::from_utf8(b).ok()) else { continue };
        let Some(name) = key.strip_prefix(&format!("{package}::")) else { continue };
        let ret = signature
            .rsplit_once("):")
            .map(|(_, r)| r.trim().trim_end_matches(';'))
            .and_then(thaw_js_infer::clone::CTy::parse)
            .map(|t| t.to_dt());
        match seen.get(name) {
            Some(prev) if *prev != ret => {
                seen.insert(name.to_string(), None);
            }
            Some(_) => {}
            None => {
                seen.insert(name.to_string(), ret);
            }
        }
    }
    seen.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect()
}

/// The program's static types prove the clone's contract for every reference to the function.
fn js_infer_proven(
    report: &JsInferAot,
    package: &str,
    clone: &thaw_js_infer::clone::AotClone,
) -> bool {
    if report.evidence.is_empty() {
        return true;
    }
    let Some(evidence) = report.evidence.get(package) else { return false };
    let mut names: Vec<&str> = clone.exports.iter().map(String::as_str).collect();
    names.push(thaw_js_infer::evidence::DEFAULT);
    let want: Vec<_> = clone.params.iter().map(thaw_js_infer::clone::CTy::to_dt).collect();
    evidence.proves(&names, &want)
}

/// `function <name>(a0: Json, ..): R`: the clone when every argument is covered by the contract,
/// the original typed call (`symbol`, which marshals `Json` as it always did) otherwise.
fn js_infer_guard_wrapper(
    name: &str,
    symbol: &str,
    clone: &thaw_js_infer::clone::AotClone,
) -> String {
    use thaw_js_infer::clone::CTy;
    let n = clone.params.len();
    let helper = |i: usize| format!("{}_p{i}", clone.fn_name);
    let params = (0..n).map(|i| format!("a{i}: Json")).collect::<Vec<_>>().join(", ");
    let guard = clone
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| p.guard(&format!("a{i}")).unwrap_or_else(|| format!("{}_ok(a{i})", helper(i))))
        .collect::<Vec<_>>()
        .join(" && ");
    let args = clone
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| match p {
            CTy::Prim(_) => format!("a{i}"),
            _ => format!("{}_cv(a{i})", helper(i)),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let fallback = (0..n).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", ");
    format!(
        "function {name}({params}): {} {{\n    if ({guard}) {{ return {}({args}); }}\n    return {symbol}({fallback});\n}}\n",
        clone.ret.ts(),
        clone.fn_name
    )
}

fn js_infer_aot_manifest(report: &JsInferAot) -> serde_json::Value {
    serde_json::json!({
        "patched": report.patched.iter().map(|(p, k, f)| serde_json::json!({"package": p, "function": k, "clone": f})).collect::<Vec<_>>(),
        "guarded": report.guarded.iter().map(|(p, k, f)| serde_json::json!({"package": p, "function": k, "wrapper": f})).collect::<Vec<_>>(),
        "retargeted": report.retargets.iter().map(|(p, k, f)| serde_json::json!({"package": p, "function": k, "clone": f})).collect::<Vec<_>>(),
        "rejected": report.rejected.iter().map(|(p, k, e)| serde_json::json!({"package": p, "function": k, "error": e})).collect::<Vec<_>>(),
    })
}

/// Drop the `registry-fallback` reasons of functions that were retargeted to an AOT clone.
fn js_infer_retain_reasons(
    reasons: Vec<serde_json::Value>,
    aot: &JsInferAot,
) -> Vec<serde_json::Value> {
    if aot.retargets.is_empty() {
        return reasons;
    }
    reasons
        .into_iter()
        .filter(|reason| {
            let package = reason["package"].as_str().unwrap_or("");
            let operation = reason["function"]
                .as_str()
                .or_else(|| reason["operation"].as_str())
                .unwrap_or("");
            !(reason["kind"].as_str() == Some("registry-fallback")
                && aot.retargets.iter().any(|(p, n, _)| p == package && n == operation))
        })
        .collect()
}
