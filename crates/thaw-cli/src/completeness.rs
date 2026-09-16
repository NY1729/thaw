fn run_completeness(args: &[String]) -> Result<(), String> {
    let mut as_json = false;
    let mut with_node = false;
    for arg in args {
        match arg.as_str() {
            "--json" => as_json = true,
            "--with-node" => with_node = true,
            other => {
                return Err(format!(
                    "unknown completeness option `{other}`\n\nusage: thaw completeness [--json] [--with-node]"
                ))
            }
        }
    }
    let report = completeness_report(with_node)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
    } else {
        print_completeness(&report);
    }
    Ok(())
}

/// A rough, machine-readable snapshot of how much of the project's stated
/// target surface is currently covered.
///
/// Two measured axes feed the composite `indexPercent`:
///
/// - **TypeScript subset** (`tests/typescript-compat.json`): the share of
///   in-scope cases the compiler can lower. `out-of-scope` cases are
///   deliberately excluded from the denominator -- they are documented
///   non-goals, not gaps.
/// - **Node runtime** (`tests/typescript-runtime-compat.json` and
///   `tests/node-compat.json`, only with `--with-node`): the share of cases
///   whose observable behavior matches real Node. These *build and execute*
///   every case against both runtimes, so they are opt-in.
///
/// The remaining fields are static repository counts, reported for trend
/// tracking rather than scoring: they have no defensible "100%" target.
///
/// `--with-node` is intentionally not the default: those manifests compile
/// and run hundreds of programs and take minutes.
fn completeness_report(with_node: bool) -> Result<serde_json::Value, String> {
    const TYPESCRIPT_MANIFEST: &str = "tests/typescript-compat.json";

    let root = repository_root();
    let typescript = compatibility_report(&root.join(TYPESCRIPT_MANIFEST))?;
    let supported = typescript["counts"]["supported"].as_u64().unwrap_or(0);
    let unsupported = typescript["counts"]["unsupported"].as_u64().unwrap_or(0);
    let out_of_scope = typescript["counts"]["out-of-scope"].as_u64().unwrap_or(0);
    let typescript_bugs = typescript["bugs"].as_u64().unwrap_or(0);
    let typescript_coverage = percentage(supported, supported + unsupported);

    let node = if with_node {
        let mut total = 0;
        let mut matched = 0;
        let mut unsupported = 0;
        let mut failed = 0;
        let mut bugs = 0;
        let mut reference_errors = 0;
        for manifest in ["tests/typescript-runtime-compat.json", "tests/node-compat.json"] {
            let report = node_compatibility_report(&root.join(manifest))?;
            for result in report["results"].as_array().into_iter().flatten() {
                total += 1;
                match result["classification"].as_str().unwrap_or("unknown") {
                    "matched" => matched += 1,
                    "unsupported" => unsupported += 1,
                    "failed" => failed += 1,
                    _ => bugs += 1,
                }
            }
            reference_errors += report["referenceErrors"].as_u64().unwrap_or(0);
        }
        Some(serde_json::json!({
            "total": total,
            "matched": matched,
            "unsupported": unsupported,
            "failed": failed,
            "bugs": bugs,
            "referenceErrors": reference_errors,
            "coveragePercent": percentage(matched, matched + unsupported),
        }))
    } else {
        None
    };

    let mut axes = vec![typescript_coverage];
    if let Some(node) = &node {
        axes.push(node["coveragePercent"].as_f64().unwrap_or(0.0));
    }
    let index = (axes.iter().sum::<f64>() / axes.len() as f64 * 10.0).round() / 10.0;

    Ok(serde_json::json!({
        "typescript": {
            "total": typescript["total"],
            "supported": supported,
            "unsupported": unsupported,
            "outOfScope": out_of_scope,
            "bugs": typescript_bugs,
            "coveragePercent": typescript_coverage,
        },
        "node": node,
        "repository": {
            "pinnedNpmIntegrationTests": count_marker_lines(
                &root.join("crates"),
                "THAW_RUN_NPM_INTEGRATION",
            ),
            "examples": directory_count(&root.join("examples")),
            "designDocs": design_document_count(&root.join("docs/design")),
        },
        // Mean of the measured axes only -- a rough trend indicator, not a
        // precise statement of completeness. See the doc comment above.
        "indexPercent": index,
    }))
}

fn percentage(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 1000.0).round() / 10.0
}

fn count_marker_lines(directory: &Path, marker: &str) -> u64 {
    fn walk(path: &Path, marker: &str, count: &mut u64) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, marker, count);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    *count += source
                        .lines()
                        .filter(|line| line.contains(marker))
                        .count() as u64;
                }
            }
        }
    }
    let mut count = 0;
    walk(directory, marker, &mut count);
    count
}

fn directory_count(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .map(|entries| entries.flatten().filter(|entry| entry.path().is_dir()).count() as u64)
        .unwrap_or(0)
}

fn design_document_count(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|extension| extension.to_str()) == Some("md")
                })
                .count() as u64
        })
        .unwrap_or(0)
}

/// The repository root, found by walking up from the current directory
/// until the default TypeScript manifest appears. Lets `thaw completeness`
/// work from either the repo root or a crate subdirectory.
fn repository_root() -> std::path::PathBuf {
    let start = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut directory = start.clone();
    loop {
        if directory.join("tests/typescript-compat.json").is_file() {
            return directory;
        }
        match directory.parent() {
            Some(parent) => directory = parent.to_path_buf(),
            None => return start,
        }
    }
}

fn print_completeness(report: &serde_json::Value) {
    let typescript = &report["typescript"];
    println!("Thaw completeness snapshot");
    println!();
    println!(
        "TypeScript subset   {supported}/{in_scope} in scope ({coverage}%)  bugs={bugs}  out-of-scope={out_of_scope}",
        supported = typescript["supported"],
        in_scope = typescript["supported"].as_u64().unwrap_or(0)
            + typescript["unsupported"].as_u64().unwrap_or(0),
        coverage = typescript["coveragePercent"],
        bugs = typescript["bugs"],
        out_of_scope = typescript["outOfScope"],
    );
    match report["node"].as_object() {
        Some(node) => println!(
            "Node runtime         {matched}/{in_scope} compared ({coverage}%)  unsupported={unsupported} failed={failed} bugs={bugs}",
            matched = node["matched"],
            in_scope = node["matched"].as_u64().unwrap_or(0)
                + node["unsupported"].as_u64().unwrap_or(0),
            coverage = node["coveragePercent"],
            unsupported = node["unsupported"],
            failed = node["failed"],
            bugs = node["bugs"],
        ),
        None => println!("Node runtime         (skipped; pass --with-node)"),
    }
    let repository = &report["repository"];
    println!(
        "Pinned npm tests     {}",
        repository["pinnedNpmIntegrationTests"]
    );
    println!("Examples             {}", repository["examples"]);
    println!("Design docs          {}", repository["designDocs"]);
    println!();
    println!("Index (mean of measured axes)  {}%", report["indexPercent"]);
    println!();
    println!("Note: the index is a rough trend indicator over the measured axes only;");
    println!("the repository counts above are not scored.");
}
