/// `thaw x`'s argument grammar: a package spec, optional `-y`/`--yes`, an
/// optional `--` separator, then everything else forwarded verbatim.
#[test]
fn parses_x_invocations() {
    let invocation = parse_x_args(&["cowsay".into(), "hello".into(), "--loud".into()]).unwrap();
    assert_eq!(invocation.spec, "cowsay");
    assert_eq!(invocation.arguments, ["hello", "--loud"]);

    let invocation = parse_x_args(&["-y".into(), "prisma@5".into(), "--".into(), "generate".into()])
        .unwrap();
    assert_eq!(invocation.spec, "prisma@5");
    assert_eq!(invocation.arguments, ["generate"]);

    // A `--` *inside* the forwarded arguments is preserved.
    let invocation = parse_x_args(&["tsx".into(), "--".into(), "a".into()]).unwrap();
    assert_eq!(invocation.arguments, ["a"]);

    assert!(parse_x_args(&[]).is_err());
    assert!(parse_x_args(&["--unknown".into(), "pkg".into()]).is_err());
}

#[test]
fn resolves_a_string_bin_as_the_package_command() {
    let dir = temp_dir("string-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":"cli.js"}"#);

    let (command, path) = resolve_package_bin(&dir, "hello-cli").unwrap();
    assert_eq!(command, "hello-cli");
    assert_eq!(path, dir.join("cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_the_bin_key_named_after_the_package() {
    let dir = temp_dir("named-bin");
    write_manifest(
        &dir,
        r#"{"name":"hello","bin":{"hello":"cli.js","other":"other.js"}}"#,
    );

    let (command, path) = resolve_package_bin(&dir, "hello").unwrap();
    assert_eq!(command, "hello");
    assert_eq!(path, dir.join("cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_a_scoped_package_by_its_basename() {
    let dir = temp_dir("scoped-bin");
    write_manifest(
        &dir,
        r#"{"name":"@scope/hello","bin":{"hello":"cli.js"}}"#,
    );

    let (command, path) = resolve_package_bin(&dir, "@scope/hello").unwrap();
    assert_eq!(command, "hello");
    assert_eq!(path, dir.join("cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_a_single_entry_bin_object_with_a_different_name() {
    let dir = temp_dir("single-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":{"hello":"cli.js"}}"#);

    let (command, path) = resolve_package_bin(&dir, "hello-cli").unwrap();
    assert_eq!(command, "hello");
    assert_eq!(path, dir.join("cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn strips_a_leading_dot_slash_from_the_declared_bin() {
    let dir = temp_dir("dot-slash-bin");
    write_manifest(&dir, r#"{"name":"hello-cli","bin":"./dist/cli.js"}"#);

    let (_, path) = resolve_package_bin(&dir, "hello-cli").unwrap();
    assert_eq!(path, dir.join("dist/cli.js"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_a_package_without_a_bin() {
    let dir = temp_dir("no-bin");
    write_manifest(&dir, r#"{"name":"library","main":"index.js"}"#);
    let error = resolve_package_bin(&dir, "library").unwrap_err();
    assert!(error.contains("does not declare a `bin`"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_an_ambiguous_multi_bin_package() {
    let dir = temp_dir("ambiguous-bin");
    write_manifest(
        &dir,
        r#"{"name":"hello-cli","bin":{"a":"a.js","b":"b.js"}}"#,
    );
    let error = resolve_package_bin(&dir, "hello-cli").unwrap_err();
    assert!(error.contains("multiple commands"), "{error}");
    assert!(error.contains("a, b"), "{error}");
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

    assert_eq!(find_local_package_dir("hello-cli", &nested), Some(package));
    assert_eq!(find_local_package_dir("missing", &nested), None);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn runs_an_executable_bin_directly() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("exec-bin");
    let bin = dir.join("cli.sh");
    std::fs::write(&bin, "#!/bin/sh\nprintf 'run:%s\\n' \"$1\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = bin_command(&bin)
        .unwrap()
        .arg("world")
        .output()
        .unwrap();
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

    let output = bin_command(&bin)
        .unwrap()
        .arg("world")
        .output()
        .unwrap();
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
