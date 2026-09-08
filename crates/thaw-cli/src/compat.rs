fn run_compat(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: thaw compat [manifest.json]".into());
    }
    let path = Path::new(
        args.first()
            .map(String::as_str)
            .unwrap_or("tests/typescript-compat.json"),
    );
    let document: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?,
    )
    .map_err(|error| format!("invalid compatibility manifest: {error}"))?;
    let cases = document["cases"]
        .as_array()
        .ok_or("compatibility manifest must contain a `cases` array")?;
    let mut results = Vec::with_capacity(cases.len());
    let mut bugs = 0;
    for case in cases {
        let name = case["name"].as_str().ok_or("case is missing `name`")?;
        let expected = case["expect"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `expect`"))?;
        if expected == "out-of-scope" {
            results.push(serde_json::json!({ "name": name, "classification": expected }));
            continue;
        }
        if expected != "supported" && expected != "unsupported" {
            return Err(format!("case `{name}` has invalid expectation `{expected}`"));
        }
        let source = case["source"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `source`"))?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            thaw_parser::parse_typescript_with_source_map(source).and_then(
                |(module, source_map)| {
                    thaw_hir::lower_module_with_source_map(&module, &source_map, name)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                },
            )
        }))
        .unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            Err(format!("compiler panic: {message}"))
        });
        let actual = if outcome.is_ok() {
            "supported"
        } else {
            "unsupported"
        };
        let classification = if actual == expected { actual } else { "bug" };
        bugs += usize::from(classification == "bug");
        let mut result = serde_json::json!({
            "name": name,
            "expected": expected,
            "actual": actual,
            "classification": classification,
        });
        if let Err(error) = outcome {
            result["error"] = error.into();
        }
        results.push(result);
    }
    let count = |classification: &str| {
        results
            .iter()
            .filter(|result| result["classification"] == classification)
            .count()
    };
    let report = serde_json::json!({
        "total": results.len(),
        "bugs": bugs,
        "counts": {
            "supported": count("supported"),
            "unsupported": count("unsupported"),
            "out-of-scope": count("out-of-scope"),
        },
        "results": results,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    if bugs == 0 {
        Ok(())
    } else {
        Err(format!("{bugs} compatibility expectation(s) changed"))
    }
}
