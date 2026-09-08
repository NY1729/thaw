fn run_node_compat(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: thaw node-compat [manifest.json]".into());
    }
    let path = Path::new(
        args.first()
            .map(String::as_str)
            .unwrap_or("tests/node-compat.json"),
    );
    let document: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?,
    )
    .map_err(|error| format!("invalid Node compatibility manifest: {error}"))?;
    let cases = document["cases"]
        .as_array()
        .ok_or("Node compatibility manifest must contain a `cases` array")?;
    let root = std::env::temp_dir().join(format!("thaw-node-compat-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(cases.len());
    let mut bugs = 0;

    for (index, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().ok_or("case is missing `name`")?;
        let expected = case["expect"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `expect`"))?;
        if !matches!(expected, "matched" | "unsupported") {
            return Err(format!("case `{name}` has invalid expectation `{expected}`"));
        }
        let source = case["source"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `source`"))?;
        let case_dir = root.join(index.to_string());
        std::fs::create_dir_all(&case_dir).map_err(|error| error.to_string())?;
        let node_source = case_dir.join("main.mts");
        std::fs::write(&node_source, format!("{source}\nawait main();\n"))
            .map_err(|error| error.to_string())?;
        let node = Command::new("node")
            .arg(&node_source)
            .output()
            .map_err(|error| format!("failed to run Node.js: {error}"))?;
        if !node.status.success() {
            return Err(format!(
                "Node.js reference case `{name}` failed: {}",
                String::from_utf8_lossy(&node.stderr).trim()
            ));
        }

        let thaw_source = case_dir.join("main.ts");
        let output = case_dir.join("app");
        std::fs::write(&thaw_source, source).map_err(|error| error.to_string())?;
        let outcome = build_with_native_mode(
            &thaw_source,
            &output,
            &[],
            &[],
            &[],
            &case_dir.join("thaw_modules"),
            &[],
            false,
            None,
            true,
        )
        .and_then(|_| {
            Command::new(&output)
                .output()
                .map_err(|error| error.to_string())
        });
        let (actual, detail) = match outcome {
            Ok(thaw) if thaw.status.success() && thaw.stdout == node.stdout => ("matched", None),
            Ok(thaw) if thaw.status.success() => (
                "unsupported",
                Some(format!(
                    "output differs: node={}, thaw={}",
                    String::from_utf8_lossy(&node.stdout).trim(),
                    String::from_utf8_lossy(&thaw.stdout).trim()
                )),
            ),
            Ok(thaw) => (
                "unsupported",
                Some(String::from_utf8_lossy(&thaw.stderr).trim().to_string()),
            ),
            Err(error) => ("unsupported", Some(error)),
        };
        let classification = if actual == expected { actual } else { "bug" };
        bugs += usize::from(classification == "bug");
        let mut result = serde_json::json!({
            "name": name,
            "expected": expected,
            "actual": actual,
            "classification": classification,
        });
        if let Some(detail) = detail {
            result["detail"] = detail.into();
        }
        results.push(result);
    }
    let _ = std::fs::remove_dir_all(root);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "total": results.len(),
            "bugs": bugs,
            "results": results,
        }))
        .map_err(|error| error.to_string())?
    );
    if bugs == 0 {
        Ok(())
    } else {
        Err(format!("{bugs} Node compatibility expectation(s) changed"))
    }
}
