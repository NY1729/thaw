fn run_node_compat(args: &[String]) -> Result<(), String> {
    const EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
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
    let mut reference_errors = 0;

    for (index, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().ok_or("case is missing `name`")?;
        let expected = case["expect"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `expect`"))?;
        if !matches!(expected, "matched" | "unsupported" | "failed") {
            return Err(format!("case `{name}` has invalid expectation `{expected}`"));
        }
        let source = case["source"]
            .as_str()
            .ok_or_else(|| format!("case `{name}` is missing `source`"))?;
        let arguments = case
            .get("args")
            .and_then(serde_json::Value::as_array)
            .map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| {
                        argument
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| format!("case `{name}` has a non-string argument"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let case_dir = root.join(index.to_string());
        std::fs::create_dir_all(&case_dir).map_err(|error| error.to_string())?;
        let node_source = case_dir.join("main.mts");
        std::fs::write(&node_source, format!("{source}\nawait main();\n"))
            .map_err(|error| error.to_string())?;
        let node = command_output_with_timeout(
            Command::new("node").arg(&node_source).args(&arguments),
            EXECUTION_TIMEOUT,
        )
            .map_err(|error| format!("failed to run Node.js: {error}"))?;
        if !node.status.success() && expected != "failed" {
            reference_errors += 1;
            results.push(serde_json::json!({
                "name": name,
                "expected": expected,
                "actual": "reference-error",
                "classification": "reference-error",
                "detail": String::from_utf8_lossy(&node.stderr).trim(),
            }));
            continue;
        }

        let thaw_source = case_dir.join("main.ts");
        let output = case_dir.join("app");
        std::fs::write(&thaw_source, source).map_err(|error| error.to_string())?;
        let build = build_with_native_mode(
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
        );
        let outcome = build.and_then(|_| {
            command_output_with_timeout(
                Command::new(&output).args(&arguments),
                EXECUTION_TIMEOUT,
            )
        });
        let thaw_exit_code = outcome
            .as_ref()
            .ok()
            .and_then(|output| output.status.code())
            .map(i64::from);
        let (actual, detail) = match outcome {
            Ok(thaw) if !node.status.success() && !thaw.status.success() => (
                "failed",
                Some(String::from_utf8_lossy(&thaw.stderr).trim().to_string()),
            ),
            Ok(thaw) if node.status.success() && thaw.status.success() && thaw.stdout == node.stdout => {
                ("matched", None)
            }
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
        let expected_detail = case.get("detailContains").and_then(serde_json::Value::as_str);
        let detail_matches = expected_detail.is_none_or(|expected| {
            detail
                .as_deref()
                .is_some_and(|detail| detail.contains(expected))
        });
        let expected_exit_code = case.get("exitCode").and_then(serde_json::Value::as_i64);
        let exit_code_matches = expected_exit_code.is_none_or(|expected| {
            node.status.code().map(i64::from) == Some(expected) && thaw_exit_code == Some(expected)
        });
        let classification = if actual == expected && detail_matches && exit_code_matches {
            actual
        } else {
            "bug"
        };
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
        if let Some(expected_detail) = expected_detail {
            result["detailContains"] = expected_detail.into();
        }
        if let Some(expected_exit_code) = expected_exit_code {
            result["exitCode"] = expected_exit_code.into();
            result["actualExitCode"] = thaw_exit_code.into();
        }
        results.push(result);
    }
    let _ = std::fs::remove_dir_all(root);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "total": results.len(),
            "bugs": bugs,
            "referenceErrors": reference_errors,
            "results": results,
        }))
        .map_err(|error| error.to_string())?
    );
    if bugs == 0 && reference_errors == 0 {
        Ok(())
    } else {
        Err(format!(
            "{bugs} Node compatibility expectation(s) changed; {reference_errors} reference case(s) failed"
        ))
    }
}

fn command_output_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    use std::process::Stdio;

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if child.try_wait().map_err(|error| error.to_string())?.is_some() {
            return child.wait_with_output().map_err(|error| error.to_string());
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("timed out after {} seconds", timeout.as_secs()));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
