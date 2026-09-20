/// `thaw x`'s argument grammar: a package spec, optional `-y`/`--yes`, an
/// optional `--` separator, then everything else forwarded verbatim.
#[test]
fn parses_x_invocations() {
    let invocation = parse_x_args(&["cowsay".into(), "hello".into(), "--loud".into()]).unwrap();
    assert_eq!(invocation.spec.as_deref(), Some("cowsay"));
    assert!(invocation.command.is_none());
    assert_eq!(invocation.arguments, ["hello", "--loud"]);

    let invocation =
        parse_x_args(&["-y".into(), "prisma@5".into(), "--".into(), "generate".into()]).unwrap();
    assert_eq!(invocation.spec.as_deref(), Some("prisma@5"));
    assert_eq!(invocation.arguments, ["generate"]);

    // A `--` *inside* the forwarded arguments is preserved.
    let invocation = parse_x_args(&["tsx".into(), "a".into(), "--".into(), "b".into()]).unwrap();
    assert_eq!(invocation.arguments, ["a", "--", "b"]);

    assert!(parse_x_args(&[]).is_err());
    assert!(parse_x_args(&["--unknown".into(), "pkg".into()]).is_err());
}

/// `-h`/`--help` is a request for usage, not a forwarded argument, and an
/// empty package/command name is rejected before it reaches `npm`.
#[test]
fn parses_x_help_and_rejects_empty_specs() {
    assert!(parse_x_args(&["-h".into()]).unwrap().help);
    assert!(parse_x_args(&["--help".into()]).unwrap().help);
    // After the spec, `-h` is forwarded to the bin, not treated as help.
    assert!(!parse_x_args(&["pkg".into(), "-h".into()]).unwrap().help);
    assert!(parse_x_args(&["--package=".into()]).is_err());
    assert!(parse_x_args(&["-p".into(), "".into(), "cmd".into()]).is_err());
    assert!(parse_x_args(&["".into()]).is_err());
    assert!(parse_x_args(&["-p".into(), "pkg".into(), "".into()]).is_err());
}

#[test]
fn parses_no_install_before_the_package() {
    let invocation = parse_x_args(&[
        "--no-install".into(),
        "prettier@3".into(),
        "--check".into(),
        "app.ts".into(),
    ])
    .unwrap();
    assert!(invocation.no_install);
    assert_eq!(invocation.spec.as_deref(), Some("prettier@3"));
    assert_eq!(invocation.arguments, ["--check", "app.ts"]);

    // Options after the package belong to the package executable.
    assert!(!parse_x_args(&["pkg".into(), "--no-install".into()])
        .unwrap()
        .no_install);
}

#[test]
fn no_install_rejects_an_absent_package_without_fetching() {
    let error = run_x(&[
        "--no-install".into(),
        "thaw-definitely-absent-package".into(),
    ])
    .unwrap_err();
    assert!(error.contains("not installed locally"), "{error}");
}

#[test]
fn x_fetches_and_runs_a_real_package_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        run_x(&["cowsay@1.6.0".into(), "hello from thaw".into()]).unwrap(),
        0
    );
}

#[test]
fn x_runs_a_real_vitest_suite_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = temp_dir("vitest");
    std::fs::write(
        root.join("basic.test.js"),
        "import { expect, test } from 'vitest'; test('works', () => expect(40 + 2).toBe(42));\n",
    )
    .unwrap();

    let status = run_x(&[
        "vitest@3.2.4".into(),
        "run".into(),
        "--root".into(),
        root.to_string_lossy().into_owned(),
        "basic.test.js".into(),
    ])
    .unwrap();

    assert_eq!(status, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_runs_a_real_jest_suite_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = temp_dir("jest");
    std::fs::write(
        root.join("basic.test.js"),
        "test('works', () => expect(40 + 2).toBe(42));\n",
    )
    .unwrap();
    let config = format!(r#"{{"rootDir":"{}"}}"#, root.display());

    let status = run_x(&[
        "jest@30.2.0".into(),
        "--runInBand".into(),
        "--config".into(),
        config,
        "basic.test.js".into(),
    ])
    .unwrap();

    assert_eq!(status, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_runs_a_real_typescript_file_with_tsx_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = temp_dir("tsx");
    let script = root.join("main.ts");
    std::fs::write(
        &script,
        "const value: number = 40 + 2; if (value !== 42 || process.argv[2] !== 'tail') process.exit(1);\n",
    )
    .unwrap();

    let status = run_x(&[
        "tsx@4.20.6".into(),
        script.to_string_lossy().into_owned(),
        "tail".into(),
    ])
    .unwrap();

    assert_eq!(status, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_runs_tsc_from_an_explicit_package_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = temp_dir("tsc");
    let script = root.join("typecheck.ts");
    std::fs::write(&script, "const answer: number = 40 + 2; void answer;\n").unwrap();

    let status = run_x(&[
        "-p".into(),
        "typescript@5.9.3".into(),
        "tsc".into(),
        "--noEmit".into(),
        "--skipLibCheck".into(),
        "--target".into(),
        "es2022".into(),
        script.to_string_lossy().into_owned(),
    ])
    .unwrap();

    assert_eq!(status, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_formats_a_file_with_real_prettier_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = temp_dir("prettier");
    let input = root.join("input.ts");
    std::fs::write(&input, "const answer={value:40+2}\n").unwrap();

    let status = run_x(&[
        "prettier@3.6.2".into(),
        "--write".into(),
        input.to_string_lossy().into_owned(),
    ])
    .unwrap();

    assert_eq!(status, 0);
    assert_eq!(
        std::fs::read_to_string(&input).unwrap(),
        "const answer = { value: 40 + 2 };\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_preserves_real_eslint_failure_status_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("thaw-cli-x-eslint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("eslint.config.mjs");
    let input = root.join("input.js");
    std::fs::write(
        &config,
        "export default [{ rules: { 'no-unused-vars': 'error' } }];\n",
    )
    .unwrap();
    std::fs::write(&input, "const unused = 42;\n").unwrap();

    let status = run_x(&[
        "eslint@9.36.0".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        input.to_string_lossy().into_owned(),
    ])
    .unwrap();

    assert_eq!(status, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_exposes_multiple_real_packages_on_path_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        run_x(&[
            "-p".into(),
            "cross-env@7.0.3".into(),
            "-p".into(),
            "cowsay@1.6.0".into(),
            "cross-env".into(),
            "cowsay".into(),
            "hello from thaw".into(),
        ])
        .unwrap(),
        0
    );
}

/// `-p/--package <spec>` runs the explicitly named command from that
/// package, `npx -p`-style.
#[test]
fn parses_package_option_invocations() {
    let invocation = parse_x_args(&[
        "-p".into(),
        "prisma".into(),
        "prisma".into(),
        "generate".into(),
    ])
    .unwrap();
    assert_eq!(invocation.packages, ["prisma"]);
    assert!(invocation.spec.is_none());
    assert_eq!(invocation.command.as_deref(), Some("prisma"));
    assert_eq!(invocation.arguments, ["generate"]);

    let invocation =
        parse_x_args(&["--package=typescript".into(), "tsc".into(), "--noEmit".into()]).unwrap();
    assert_eq!(invocation.packages, ["typescript"]);
    assert_eq!(invocation.command.as_deref(), Some("tsc"));
    assert_eq!(invocation.arguments, ["--noEmit"]);

    // `-p` requires both a package and a command.
    assert!(parse_x_args(&["-p".into()]).is_err());
    assert!(parse_x_args(&["-p".into(), "pkg".into()]).is_err());
}

#[test]
fn resolves_a_string_bin_as_the_package_command() {
    let dir = temp_dir("string-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":"cli.js"}"#);

    let bins = package_bins(&dir, "hello-cli").unwrap();
    assert_eq!(bins, [("hello-cli".to_string(), dir.join("cli.js"))]);
    assert_eq!(
        default_bin(&bins, "hello-cli").unwrap(),
        dir.join("cli.js")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_the_bin_key_named_after_the_package() {
    let dir = temp_dir("named-bin");
    write_manifest(
        &dir,
        r#"{"name":"hello","bin":{"hello":"cli.js","other":"other.js"}}"#,
    );

    let bins = package_bins(&dir, "hello").unwrap();
    assert_eq!(default_bin(&bins, "hello").unwrap(), dir.join("cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_a_scoped_package_by_its_basename() {
    let dir = temp_dir("scoped-bin");
    write_manifest(&dir, r#"{"name":"@scope/hello","bin":{"hello":"cli.js"}}"#);

    let bins = package_bins(&dir, "@scope/hello").unwrap();
    assert_eq!(bins, [("hello".to_string(), dir.join("cli.js"))]);
    assert_eq!(
        default_bin(&bins, "@scope/hello").unwrap(),
        dir.join("cli.js")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_a_single_entry_bin_object_with_a_different_name() {
    let dir = temp_dir("single-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":{"hello":"cli.js"}}"#);

    let bins = package_bins(&dir, "hello-cli").unwrap();
    assert_eq!(
        default_bin(&bins, "hello-cli").unwrap(),
        dir.join("cli.js")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn strips_a_leading_dot_slash_from_the_declared_bin() {
    let dir = temp_dir("dot-slash-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":"./dist/cli.js"}"#);

    let bins = package_bins(&dir, "hello-cli").unwrap();
    assert_eq!(bins[0].1, dir.join("dist/cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_a_package_without_a_bin() {
    let dir = temp_dir("no-bin");
    write_manifest(&dir, r#"{"name":"library","main":"index.js"}"#);

    let bins = package_bins(&dir, "library").unwrap();
    assert!(bins.is_empty());
    let error = default_bin(&bins, "library").unwrap_err();
    assert!(error.contains("does not declare a `bin`"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_an_ambiguous_multi_bin_package() {
    let dir = temp_dir("ambiguous-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":{"a":"a.js","b":"b.js"}}"#);

    let bins = package_bins(&dir, "hello-cli").unwrap();
    let error = default_bin(&bins, "hello-cli").unwrap_err();
    assert!(error.contains("multiple commands"), "{error}");
    assert!(error.contains("a, b"), "{error}");
    assert!(error.contains("-p hello-cli"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn selects_an_explicit_command_from_multiple_bins() {
    let dir = temp_dir("select-bin");
    write_manifest(
        &dir,
        r#"{"name":"toolkit","bin":{"alpha":"a.js","beta":"b.js"}}"#,
    );

    let bins = package_bins(&dir, "toolkit").unwrap();
    assert_eq!(
        select_bin(&bins, "beta", &["toolkit".into()]).unwrap(),
        dir.join("b.js")
    );
    let error = select_bin(&bins, "gamma", &["toolkit".into()]).unwrap_err();
    assert!(error.contains("no command `gamma`"), "{error}");
    assert!(error.contains("alpha, beta"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn finds_a_package_by_walking_up_node_modules() {
    let root = temp_dir("walk-up");
    let package = root.join("node_modules/hello-cli");
    std::fs::create_dir_all(&package).unwrap();
    write_manifest(&package, r#"{"name":"hello-cli","bin":"cli.js"}"#);
    let nested = root.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();

    assert_eq!(
        find_local_package_dir("hello-cli", "hello-cli", &nested),
        Some(package)
    );
    assert_eq!(
        find_local_package_dir("missing", "missing", &nested),
        None
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn versioned_x_only_reuses_an_exact_local_version() {
    let root = temp_dir("local-version");
    let package = root.join("node_modules/hello-cli");
    std::fs::create_dir_all(&package).unwrap();
    write_manifest(
        &package,
        r#"{"name":"hello-cli","version":"2.1.0","bin":"cli.js"}"#,
    );

    assert_eq!(
        find_local_package_dir("hello-cli@2.1.0", "hello-cli", &root),
        Some(package)
    );
    assert_eq!(
        find_local_package_dir("hello-cli@1.0.0", "hello-cli", &root),
        None
    );
    assert_eq!(
        find_local_package_dir("hello-cli@latest", "hello-cli", &root),
        None
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn x_cache_only_skips_resolution_for_an_exact_version() {
    let root = temp_dir("cache-version");
    write_manifest(&root, r#"{"name":"hello-cli","version":"2.1.0"}"#);

    assert!(cached_package_matches("hello-cli@2.1.0", "hello-cli", &root));
    assert!(!cached_package_matches("hello-cli", "hello-cli", &root));
    assert!(!cached_package_matches("hello-cli@latest", "hello-cli", &root));
    assert!(!cached_package_matches("hello-cli@^2", "hello-cli", &root));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multiple_packages_share_one_safe_cache_key() {
    let specs = vec!["@scope/tool@1.2.3".to_string(), "helper@latest".to_string()];
    let key = x_cache_key(&specs);
    assert_eq!(key.len(), 16);
    assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(key, x_cache_key(&[specs[1].clone(), specs[0].clone()]));
}

#[test]
fn package_bin_directories_precede_the_inherited_path() {
    let first = std::path::PathBuf::from("/tmp/thaw-first-bin");
    let second = std::path::PathBuf::from("/tmp/thaw-second-bin");
    let path = executable_path(&[first.clone(), second.clone()]).unwrap();
    let paths = std::env::split_paths(&path).collect::<Vec<_>>();
    assert_eq!(&paths[..2], &[first, second]);
}

#[cfg(unix)]
#[test]
fn runs_an_executable_bin_directly() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("exec-bin");
    let bin = dir.join("cli.sh");
    std::fs::write(&bin, "#!/bin/sh\nprintf 'run:%s\\n' \"$1\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = bin_command(&bin).unwrap().arg("world").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "run:world\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn falls_back_to_the_shebang_when_the_bin_is_not_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("shebang-bin");
    let bin = dir.join("cli.sh");
    std::fs::write(&bin, "#!/bin/sh\nprintf 'fallback:%s\\n' \"$1\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

    let output = bin_command(&bin).unwrap().arg("world").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "fallback:world\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn rejects_a_non_executable_bin_without_a_shebang() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("no-shebang-bin");
    let bin = dir.join("cli.js");
    std::fs::write(&bin, "console.log('hi');\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

    let error = bin_command(&bin).unwrap_err();
    assert!(error.contains("no shebang interpreter"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("thaw-cli-x-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_manifest(dir: &std::path::Path, manifest: &str) {
    std::fs::write(dir.join("package.json"), manifest).unwrap();
}
