// Opt-in (THAW_JS_INFER=1): type-inference census of every registry package bundle used by a build,
// recorded in the artifact manifest as `js_infer` and printed by `thaw inspect`.

fn js_infer_entries(registry_dir: &Path, packages: &[String]) -> serde_json::Value {
    let entries = packages.iter().filter_map(|package| {
        let resolved = thaw_registry::resolve(registry_dir, package).ok()?;
        let bundle = resolved.bundle_js?;
        Some(
            match thaw_registry::infer::analyze(Some(&resolved.dts_source), &bundle, false) {
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
