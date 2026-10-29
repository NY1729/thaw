fn run_node_compat(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: thaw node-compat [manifest.json]".into());
    }
    let path = Path::new(
        args.first()
            .map(String::as_str)
            .unwrap_or("tests/node-compat.json"),
    );
    let started = std::time::Instant::now();
    let report = node_compatibility_report(path)?;
    eprintln!(
        "node-compat: {} case(s), {} job(s), wall {:.1}s",
        report["total"],
        node_compat_jobs()?,
        started.elapsed().as_secs_f64()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    let bugs = report["bugs"].as_u64().unwrap_or(0);
    let reference_errors = report["referenceErrors"].as_u64().unwrap_or(0);
    if bugs == 0 && reference_errors == 0 {
        Ok(())
    } else {
        Err(format!(
            "{bugs} Node compatibility expectation(s) changed; {reference_errors} reference case(s) failed"
        ))
    }
}

/// One case's share of the `node-compat` report: its JSON entry plus its counter contribution.
struct NodeCompatCase {
    result: serde_json::Value,
    bug: bool,
    reference_error: bool,
}

impl NodeCompatCase {
    fn reference_error(result: serde_json::Value) -> Self {
        Self {
            result,
            bug: false,
            reference_error: true,
        }
    }
}

/// Worker count for `thaw node-compat`: `THAW_NODE_COMPAT_JOBS`, default `min(8, cores / 2)`.
fn node_compat_jobs() -> Result<usize, String> {
    match std::env::var("THAW_NODE_COMPAT_JOBS") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|jobs| *jobs > 0)
            .ok_or_else(|| format!("THAW_NODE_COMPAT_JOBS must be a positive integer, not `{value}`")),
        Err(_) => Ok(std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .div_euclid(2)
            .clamp(1, 8)),
    }
}

/// Runs a Node compatibility manifest (building and executing each case
/// against both `node` and a compiled Thaw binary) and returns its JSON
/// report -- the same document `thaw node-compat` prints, shared with
/// `thaw completeness`. Cases run on `node_compat_jobs()` threads; results are
/// stored by index, so the report is the same as a serial run.
fn node_compatibility_report(path: &Path) -> Result<serde_json::Value, String> {
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
    let jobs = node_compat_jobs()?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::atomic::AtomicBool::new(false);
    let mut outcomes: Vec<Option<Result<NodeCompatCase, String>>> =
        (0..cases.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                scope.spawn(|| {
                    // Fast compile for the per-case builds: the reference check is about
                    // behaviour, and the object backend dominates build time.
                    set_codegen_opt_override(Some(inkwell::OptimizationLevel::None));
                    let mut done = Vec::new();
                    while !failed.load(std::sync::atomic::Ordering::Relaxed) {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(case) = cases.get(index) else {
                            break;
                        };
                        let outcome = run_node_compat_case(index, case, &root);
                        if outcome.is_err() {
                            failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        done.push((index, outcome));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            for (index, outcome) in worker.join().expect("node-compat worker panicked") {
                outcomes[index] = Some(outcome);
            }
        }
    });
    let mut results = Vec::with_capacity(cases.len());
    let mut bugs = 0;
    let mut reference_errors = 0;
    for outcome in outcomes.into_iter().flatten() {
        // The first failing case in manifest order is reported, as a serial run would.
        let case = outcome?;
        bugs += usize::from(case.bug);
        reference_errors += usize::from(case.reference_error);
        results.push(case.result);
    }
    let _ = std::fs::remove_dir_all(root);
    Ok(serde_json::json!({
        "total": results.len(),
        "bugs": bugs,
        "referenceErrors": reference_errors,
        "results": results,
    }))
}

/// Runs one manifest case (its Node reference, then the Thaw build and execution).
fn run_node_compat_case(
    index: usize,
    case: &serde_json::Value,
    root: &Path,
) -> Result<NodeCompatCase, String> {
    const EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    // Node reference runs only: a slow `node` start (CPU-starved runner) gets one fresh
    // process with a longer limit. The Thaw binary is never retried.
    const NODE_REFERENCE_TIMEOUTS: [std::time::Duration; 2] = [
        std::time::Duration::from_secs(10),
        std::time::Duration::from_secs(30),
    ];
    let name = case["name"].as_str().ok_or("case is missing `name`")?;
    let expected = case["expect"]
        .as_str()
        .ok_or_else(|| format!("case `{name}` is missing `expect`"))?;
    if !matches!(expected, "matched" | "unsupported" | "failed") {
        return Err(format!("case `{name}` has invalid expectation `{expected}`"));
    }
    let expected_exit_code = case.get("exitCode").and_then(serde_json::Value::as_i64);
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
    // Node runs a file without import/export as CommonJS and the rest as an ES module,
    // and `process.nextTick` vs promise ordering differs between the two, so the reference
    // run mirrors the module kind Thaw derives from the same source.
    // Strict mode is kept for the CommonJS run: ES modules (what every case used to run
    // as) are strict, and Thaw follows strict semantics.
    let (node_name, node_head, node_tail) =
        if module_graph::entry_is_commonjs(&case_dir.join("main.ts"), source)? {
            ("main.cts", "'use strict';\n", "main();")
        } else {
            ("main.mts", "", "await main();")
        };
    let node_source = case_dir.join(node_name);
    std::fs::write(&node_source, format!("{node_head}{source}\n{node_tail}\n"))
        .map_err(|error| error.to_string())?;
    let node = match run_node_reference(
        name,
        || {
            let mut command = Command::new("node");
            command.arg(&node_source).args(&arguments);
            command
        },
        &NODE_REFERENCE_TIMEOUTS,
    ) {
        Ok(output) => output,
        Err(NodeReferenceError::TimedOut(detail)) => {
            return Ok(NodeCompatCase::reference_error(serde_json::json!({
                "name": name,
                "expected": expected,
                "actual": "reference-error",
                "classification": "reference-error",
                "detail": detail,
            })));
        }
        Err(NodeReferenceError::Failed(error)) => return Err(error),
    };
    if expected != "failed"
        && ((!node.status.success()
            && node.status.code().map(i64::from) != expected_exit_code)
            || String::from_utf8_lossy(&node.stderr).contains("SyntaxError:"))
    {
        return Ok(NodeCompatCase::reference_error(serde_json::json!({
            "name": name,
            "expected": expected,
            "actual": "reference-error",
            "classification": "reference-error",
            "detail": String::from_utf8_lossy(&node.stderr).trim(),
        })));
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
        false,
        false,
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
        Ok(thaw)
            if thaw.status.code() == node.status.code() && thaw.stdout == node.stdout =>
        {
            ("matched", None)
        }
        Ok(thaw) if !node.status.success() && !thaw.status.success() => (
            "failed",
            Some(String::from_utf8_lossy(&thaw.stderr).trim().to_string()),
        ),
        Ok(thaw) if thaw.status.success() => (
            "unsupported",
            Some(format!(
                "output differs: node={}, thaw={}",
                String::from_utf8_lossy(&node.stdout).trim(),
                String::from_utf8_lossy(&thaw.stdout).trim(),
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
    let exit_code_matches = expected_exit_code.is_none_or(|expected| {
        node.status.code().map(i64::from) == Some(expected) && thaw_exit_code == Some(expected)
    });
    let classification = if actual == expected && detail_matches && exit_code_matches {
        actual
    } else {
        "bug"
    };
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
    Ok(NodeCompatCase {
        result,
        bug: classification == "bug",
        reference_error: false,
    })
}

enum NodeReferenceError {
    /// Every attempt timed out; recorded as a `reference-error` for the case.
    TimedOut(String),
    /// Spawn or I/O failure; aborts the manifest as before.
    Failed(String),
}

/// Runs the Node reference for `case_name` with one fresh process per timeout in `timeouts`.
/// Only a timeout is retried; any other failure is returned immediately.
fn run_node_reference(
    case_name: &str,
    mut command: impl FnMut() -> Command,
    timeouts: &[std::time::Duration],
) -> Result<std::process::Output, NodeReferenceError> {
    let mut last_timeout = String::new();
    for &timeout in timeouts {
        match command_output_with_timeout(&mut command(), timeout) {
            Ok(output) => return Ok(output),
            Err(error) if error.starts_with("timed out") => last_timeout = error,
            Err(error) => {
                return Err(NodeReferenceError::Failed(format!(
                    "case `{case_name}`: failed to run Node.js: {error}"
                )));
            }
        }
    }
    Err(NodeReferenceError::TimedOut(format!(
        "case `{case_name}`: failed to run Node.js: {last_timeout}"
    )))
}

fn command_output_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |pipe: Box<dyn std::io::Read + Send>, capture: Arc<Mutex<Vec<u8>>>| {
        std::thread::spawn(move || capture_pipe(pipe, capture))
    };
    let stdout_capture = Arc::new(Mutex::new(Vec::new()));
    let stderr_capture = Arc::new(Mutex::new(Vec::new()));
    let stdout = read(Box::new(stdout), Arc::clone(&stdout_capture));
    let stderr = read(Box::new(stderr), Arc::clone(&stderr_capture));
    let timeout_error = |stdout: &Arc<Mutex<Vec<u8>>>, stderr: &Arc<Mutex<Vec<u8>>>| {
        const TAIL: usize = 8192;
        let format_tail = |label: &str, capture: &Arc<Mutex<Vec<u8>>>| {
            match capture.try_lock() {
                Ok(bytes) => {
                    let start = bytes.len().saturating_sub(TAIL);
                    let truncated = if start > 0 {
                        " (truncated to last 8192 bytes)"
                    } else {
                        ""
                    };
                    format!(
                        "\n{label}{truncated}: {}",
                        String::from_utf8_lossy(&bytes[start..])
                    )
                }
                Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                    let bytes = poisoned.into_inner();
                    let start = bytes.len().saturating_sub(TAIL);
                    let truncated = if start > 0 {
                        " (truncated to last 8192 bytes)"
                    } else {
                        ""
                    };
                    format!(
                        "\n{label}{truncated}: {}",
                        String::from_utf8_lossy(&bytes[start..])
                    )
                }
                Err(std::sync::TryLockError::WouldBlock) => format!("\n{label}: <capture busy>"),
            }
        };
        format!(
            "timed out after {} seconds{}{}",
            timeout.as_secs(),
            format_tail("stdout", stdout),
            format_tail("stderr", stderr)
        )
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            if !stdout.is_finished() || !stderr.is_finished() {
                if std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
            } else {
                let collect = |reader: std::thread::JoinHandle<std::io::Result<()>>,
                               capture: Arc<Mutex<Vec<u8>>>| {
                    reader
                        .join()
                        .map_err(|_| "output reader panicked".to_string())?
                        .map_err(|error| error.to_string())?;
                    capture
                        .lock()
                        .map(|mut bytes| std::mem::take(&mut *bytes))
                        .map_err(|_| "output capture lock poisoned".to_string())
                };
                return Ok(std::process::Output {
                    status,
                    stdout: collect(stdout, stdout_capture)?,
                    stderr: collect(stderr, stderr_capture)?,
                });
            }
        }
        if std::time::Instant::now() >= deadline {
            #[cfg(unix)]
            unsafe {
                unsafe extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
                kill(-(child.id() as i32), 9);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(timeout_error(&stdout_capture, &stderr_capture));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn capture_pipe(
    mut pipe: impl std::io::Read,
    capture: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 8192];
    loop {
        let count = match pipe.read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            return Ok(());
        }
        capture
            .lock()
            .map_err(|_| std::io::Error::other("output capture lock poisoned"))?
            .extend_from_slice(&chunk[..count]);
    }
}

#[cfg(test)]
mod node_reference_tests {
    use super::{NodeReferenceError, run_node_reference};
    use std::cell::Cell;
    use std::process::Command;
    use std::time::Duration;

    #[test]
    fn slow_reference_names_case_and_retries_exactly_once() {
        let attempts = Cell::new(0);
        let outcome = run_node_reference(
            "slow case",
            || {
                attempts.set(attempts.get() + 1);
                let mut command = Command::new("sleep");
                command.arg("2");
                command
            },
            &[Duration::from_millis(100), Duration::from_millis(100)],
        );
        assert_eq!(attempts.get(), 2);
        match outcome {
            Err(NodeReferenceError::TimedOut(detail)) => assert!(
                detail.starts_with("case `slow case`: failed to run Node.js: timed out"),
                "{detail}"
            ),
            _ => panic!("expected a timeout reference error"),
        }
    }

    #[test]
    fn successful_reference_is_not_retried() {
        let attempts = Cell::new(0);
        let outcome = run_node_reference(
            "fast case",
            || {
                attempts.set(attempts.get() + 1);
                Command::new("true")
            },
            &[Duration::from_secs(5), Duration::from_secs(5)],
        );
        assert_eq!(attempts.get(), 1);
        assert!(outcome.is_ok());
    }
}

#[cfg(test)]
mod capture_pipe_tests {
    use super::capture_pipe;
    use std::io::{self, Read};
    use std::sync::{Arc, Mutex};

    struct InterruptedOnce(u8);

    impl Read for InterruptedOnce {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                self.0 = 1;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.0 == 2 || output.is_empty() {
                return Ok(0);
            }
            self.0 = 2;
            output[..6].copy_from_slice(b"marker");
            Ok(6)
        }
    }

    #[test]
    fn capture_retries_interrupted_reads_before_eof() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        capture_pipe(InterruptedOnce(0), Arc::clone(&captured)).unwrap();
        assert_eq!(*captured.lock().unwrap(), b"marker");
    }
}
