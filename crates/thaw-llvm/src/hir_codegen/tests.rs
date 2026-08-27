use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn napi_constructor_arity_symbols_share_the_same_export() {
    assert_eq!(
        napi_constructor_export_name("$new$Database$arity0"),
        Some("Database")
    );
    assert_eq!(
        napi_constructor_export_name("$new$Database$arity2"),
        Some("Database")
    );
    assert_eq!(
        napi_constructor_export_name("$new$Database$arity1$overload0"),
        Some("Database")
    );
    assert_eq!(napi_constructor_export_name("ordinary"), None);
}

#[test]
fn compiles_typed_napi_collection_arguments_and_results() {
    let module = thaw_parser::parse_typescript(
        r#"declare function __thaw_typed_napi_737761705475706c65(value: [number, string]): [string, number];
           declare function __thaw_typed_napi_737472696e6773(value: string[]): string[];
           declare function __thaw_typed_napi_626f6f6c73(value: boolean[]): boolean[];
           declare function __thaw_typed_napi_6e6573746564(value: string[][]): string[][];
           declare function __thaw_typed_napi_6f626a65637473(value: { name: string }[]): { name: string }[];
           declare function __thaw_typed_napi_6e756c6c61626c65(value: string | null): string | null;
           declare function __thaw_typed_napi_6e756c6c61626c656172726179(value: (string | null)[]): (string | null)[];
           function main(): void {
               const value: [number, string] = [7, "value"];
               const swapped: [string, number] = __thaw_typed_napi_737761705475706c65(value);
               const strings: string[] = __thaw_typed_napi_737472696e6773(["a", "b"]);
               const bools: boolean[] = __thaw_typed_napi_626f6f6c73([true, false]);
               const nested: string[][] = __thaw_typed_napi_6e6573746564([["a"]]);
               const objects: { name: string }[] = __thaw_typed_napi_6f626a65637473([{ name: "a" }]);
               const nullable: string | null = __thaw_typed_napi_6e756c6c61626c65(null);
               const nullableArray: (string | null)[] = __thaw_typed_napi_6e756c6c61626c656172726179(["a", null]);
               console.log(swapped[0]);
               console.log(strings[0]);
               console.log(bools[0]);
               console.log(nested[0][0]);
               console.log(objects[0].name);
               console.log(nullable);
               console.log(nullableArray[1]);
           }"#,
    )
    .unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "typed_napi_tuple");
    compiler.compile_program(&program).unwrap();
}

/// Builds the given TS source into a standalone native binary (linking
/// thaw-arena's staticlib too, since array-using programs call into
/// it), runs it with the given extra environment variables, and
/// returns its captured stdout. This is the same pipeline thaw-cli
/// drives, just inlined for testing.
fn compile_and_run(source: &str, test_name: &str) -> String {
    compile_and_run_with_env(source, test_name, &[])
}

fn compile_and_run_with_env(source: &str, test_name: &str, envs: &[(&str, &str)]) -> String {
    compile_and_run_output_with_env(source, test_name, envs).0
}

fn compile_and_run_output(source: &str, test_name: &str) -> (String, String) {
    compile_and_run_output_with_env(source, test_name, &[])
}

fn compile_and_run_output_with_env(
    source: &str,
    test_name: &str,
    envs: &[(&str, &str)],
) -> (String, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, test_name);
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-{test_name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");

    compiler.write_object_file(&obj_path).unwrap();

    // Match thaw-cli and always link every runtime archive -- an
    // unreferenced static archive member is
    // simply never pulled in, same reasoning as thaw-cli's build().
    let arena_lib = build_staticlib("thaw-arena");
    let runtime_lib = build_staticlib("thaw-runtime");
    let std_lib = build_staticlib("thaw-std");
    let quickjs_lib = build_staticlib("thaw-quickjs");

    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&quickjs_lib)
        // QuickJS-NG's C code calls libm math functions directly;
        // `rustc` normally adds `-lm` automatically when it does the
        // final link, but this is a manual `cc` invocation instead.
        .arg("-lm")
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .envs(envs.iter().copied())
        .output()
        .expect("failed to execute compiled binary");
    assert!(
        output.status.success(),
        "binary exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Builds `pkg` as a staticlib (if not already built) and returns the
/// path to the resulting `.a` file, by parsing `cargo build`'s JSON
/// artifact output -- robust to `CARGO_TARGET_DIR` overrides, unlike
/// guessing a relative path.
fn build_staticlib(pkg: &str) -> std::path::PathBuf {
    let output = Command::new("cargo")
        .args(["build", "--release", "-p", pkg, "--message-format=json"])
        .output()
        .unwrap_or_else(|e| panic!("failed to invoke `cargo build -p {pkg}`: {e}"));
    assert!(output.status.success(), "building {pkg} failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let target_name = pkg.replace('-', "_");
    for line in stdout.lines() {
        if !line.contains(&format!("\"name\":\"{target_name}\"")) {
            continue;
        }
        if let Some(idx) = line.find("\"filenames\":[\"") {
            let rest = &line[idx + "\"filenames\":[\"".len()..];
            if let Some(end) = rest.find(".a\"") {
                let path = &rest[..end + 2];
                if path.ends_with(".a") {
                    return std::path::PathBuf::from(path);
                }
            }
        }
    }
    panic!("could not find a staticlib for `{pkg}` in `cargo build` output:\n{stdout}");
}

fn compile_and_invoke_lambda(source: &str, test_name: &str, event_body: &str) -> (String, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, test_name);
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-{test_name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    compiler.write_object_file(&obj_path).unwrap();

    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(build_staticlib("thaw-arena"))
        .arg(build_staticlib("thaw-std"))
        .arg(build_staticlib("thaw-runtime"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let event_body = event_body.to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: test-req-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            event_body.len(),
            event_body
        );
        conn.write_all(response.as_bytes()).unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    let mut child = Command::new(&exe_path)
        .env("AWS_LAMBDA_RUNTIME_API", &addr)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn compiled Lambda handler binary");
    let post_request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("handler never posted to the mock runtime API");
    server.join().unwrap();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    (
        post_request,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

include!("tests/async.rs");
include!("tests/classes.rs");
include!("tests/control_flow.rs");
include!("tests/core.rs");
include!("tests/ffi.rs");
include!("tests/types.rs");
include!("tests/values.rs");
