fn record_acceptance_metrics(name: &str, build_time: Duration, output: &Path) {
    let executable_bytes = std::fs::metadata(output).unwrap().len();
    let metrics = serde_json::json!({
        "scenario": name,
        "build_ms": build_time.as_millis(),
        "executable_bytes": executable_bytes,
    });
    println!("thaw acceptance: {metrics}");
    if let Ok(directory) = std::env::var("THAW_ACCEPTANCE_OUTPUT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            Path::new(&directory).join(format!("{name}.json")),
            format!("{metrics}\n"),
        )
        .unwrap();
    }
    assert!(build_time < Duration::from_secs(300), "{metrics}");
    assert!(executable_bytes < 100 * 1024 * 1024, "{metrics}");
}

#[test]
fn builds_and_runs_a_constructor_overload() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-constructor-overload-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "class Value { value: number; constructor(value: string); constructor(value: number); constructor(value: string | number) { this.value = typeof value === 'number' ? value + 1 : value.length; } } function main(): void { console.log(new Value(41).value); }\n",
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn installs_builds_and_serves_the_react_prisma_board_when_enabled() {
    let measure_performance = std::env::var("THAW_RUN_PERFORMANCE").as_deref() == Ok("1");
    if !measure_performance && std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/hono-react-prisma-board")
        .canonicalize()
        .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-react-prisma-board-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let performance_thaw = measure_performance.then(|| {
        let source = std::env::var_os("THAW_PERF_THAW")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/thaw")
            });
        assert!(
            source.is_file(),
            "build target/release/thaw first or set THAW_PERF_THAW"
        );
        let copied = dir.join("thaw-release");
        std::fs::copy(source, &copied).unwrap();
        copied
    });
    let hello_metrics = measure_performance.then(|| {
        let thaw = performance_thaw.as_ref().unwrap();
        let source = dir.join("hello.ts");
        let output = dir.join("hello");
        std::fs::write(
            &source,
            "function main(): void { console.log(\"hello\"); }\n",
        )
        .unwrap();
        let cold_started = Instant::now();
        run_thaw_build(thaw, &source, &output, &dir.join("hello-registry"), None);
        let cold_ms = cold_started.elapsed().as_millis();
        let prepare_started = Instant::now();
        let status = Command::new(thaw).arg("prepare").status().unwrap();
        assert!(status.success(), "thaw prepare failed");
        let prepare_ms = prepare_started.elapsed().as_millis();
        let prepared_started = Instant::now();
        run_thaw_build(thaw, &source, &output, &dir.join("hello-registry"), None);
        let prepared_ms = prepared_started.elapsed().as_millis();
        let cached_started = Instant::now();
        run_thaw_build(thaw, &source, &output, &dir.join("hello-registry"), None);
        (
            cold_ms,
            prepare_ms,
            prepared_ms,
            cached_started.elapsed().as_millis(),
            std::fs::metadata(output).unwrap().len(),
        )
    });
    let database = dir.join("board.db");
    std::fs::File::create(&database).unwrap();
    let database_url = format!("file:{}", database.display());

    run_install(&[project.display().to_string()]).unwrap();
    let status = npm_run_command("db:push", &project)
        .env("DATABASE_URL", &database_url)
        .status()
        .unwrap();
    assert!(status.success());
    let vite_started = Instant::now();
    let assets = build_vite_project(&project).unwrap();
    let vite_ms = vite_started.elapsed().as_millis();
    let package = dir.join("package");
    std::fs::create_dir_all(&package).unwrap();
    let executable = package.join("board");
    let build_started = Instant::now();
    if let Some(thaw) = &performance_thaw {
        run_thaw_build(
            thaw,
            &project.join("server.ts"),
            &executable,
            &dir.join("registry"),
            Some(&assets),
        );
    } else {
        build_with_native_mode(
            &project.join("server.ts"),
            &executable,
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            false,
            Some(&assets),
            false,
        )
        .unwrap();
    }
    let initial_build_ms = build_started.elapsed().as_millis();
    let cached_build_ms = measure_performance.then(|| {
        let started = Instant::now();
        run_thaw_build(
            performance_thaw.as_ref().unwrap(),
            &project.join("server.ts"),
            &executable,
            &dir.join("registry"),
            Some(&assets),
        );
        started.elapsed().as_millis()
    });
    let executable_bytes = std::fs::metadata(&executable).unwrap().len();
    let sidecar_bytes = directory_size(&package.join("board.native"));
    let artifact_manifest =
        artifact_manifest_from_bytes(&std::fs::read(&executable).unwrap()).unwrap();
    assert!(package
        .join("board.native/_prisma_client/native.node")
        .is_file());
    let moved = dir.join("moved");
    std::fs::rename(package, &moved).unwrap();
    let executable = moved.join("board");

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(executable)
        .env("DATABASE_URL", database_url)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = |method: &str, path: &str, body: &str| {
        let mut stream = (0..200)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("compiled bulletin board did not start");
        stream
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    assert!(request("GET", "/api/posts", "").contains(r#"{"posts":[]}"#));
    let created = request(
        "POST",
        "/api/posts",
        r#"{"author":"Yuu","message":"compiled"}"#,
    );
    assert!(created.contains(r#""author":"Yuu""#));
    assert!(created.contains(r#""message":"compiled""#));
    let listed = request("GET", "/api/posts", "");
    assert!(listed.contains(r#""author":"Yuu""#));
    assert!(listed.contains(r#""message":"compiled""#));
    assert!(request("GET", "/", "").contains("<title>Thaw掲示板</title>"));
    child.kill().unwrap();
    child.wait().unwrap();
    if measure_performance {
        let (hello_cold_ms, prepare_ms, hello_prepared_ms, hello_cached_ms, hello_bytes) =
            hello_metrics.unwrap();
        let cached_build_ms = cached_build_ms.unwrap();
        let quickjs_reasons = artifact_manifest["quickjs_reasons"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut quickjs_reason_counts = std::collections::BTreeMap::<&str, usize>::new();
        for reason in &quickjs_reasons {
            *quickjs_reason_counts
                .entry(reason["kind"].as_str().unwrap_or("unknown"))
                .or_default() += 1;
        }
        let metrics = serde_json::json!({
            "hello": {
                "cold_build_ms": hello_cold_ms,
                "prepare_ms": prepare_ms,
                "prepared_build_ms": hello_prepared_ms,
                "cached_build_ms": hello_cached_ms,
                "executable_bytes": hello_bytes,
            },
            "board": {
                "vite_ms": vite_ms,
                "prepared_build_ms": initial_build_ms,
                "cached_build_ms": cached_build_ms,
                "executable_bytes": executable_bytes,
                "sidecar_bytes": sidecar_bytes,
                "quickjs_reason_counts": quickjs_reason_counts,
                "quickjs_reasons": quickjs_reasons,
            },
        });
        println!("thaw performance: {metrics}");
        if let Ok(path) = std::env::var("THAW_PERF_OUTPUT") {
            let path = Path::new(&path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, format!("{metrics}\n")).unwrap();
        }
        assert!(
            hello_cached_ms < 30_000,
            "cached Hello build took {hello_cached_ms} ms"
        );
        assert!(
            hello_bytes < 20 * 1024 * 1024,
            "Hello executable grew to {hello_bytes} bytes"
        );
        assert!(
            cached_build_ms < 60_000,
            "cached board build took {cached_build_ms} ms"
        );
        assert!(
            executable_bytes < 50 * 1024 * 1024,
            "board executable grew to {executable_bytes} bytes"
        );
        assert!(
            sidecar_bytes < 50 * 1024 * 1024,
            "board native sidecar grew to {sidecar_bytes} bytes"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn directory_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                directory_size(&path)
            } else {
                entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            }
        })
        .sum()
}

fn run_thaw_build(
    thaw: &Path,
    input: &Path,
    output: &Path,
    registry: &Path,
    assets: Option<&Path>,
) {
    let mut command = Command::new(thaw);
    command
        .arg("build")
        .arg(input)
        .arg("--registry")
        .arg(registry)
        .arg("--external-native")
        .arg("-o")
        .arg(output);
    if let Some(assets) = assets {
        command.arg("--assets").arg(assets);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
