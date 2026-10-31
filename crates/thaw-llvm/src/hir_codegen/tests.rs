use super::*;
use thaw_hir::HirOptionalMask;

#[test]
fn overloaded_callable_uses_its_physical_layout() {
    let physical = HirType::CallableFunction(
        vec![HirType::F64],
        HirOptionalMask::default(),
        Some(Box::new(HirType::Str)),
        Box::new(HirType::Bool),
    );
    let metadata = HirType::OverloadedFunction(
        vec![HirType::Function(
            vec![HirType::Str],
            Box::new(HirType::Str),
        )],
        Box::new(physical.clone()),
    );
    assert_eq!(raw_value_layout(&metadata), raw_value_layout(&physical));
}

#[test]
fn overloaded_calls_use_selected_returns_and_evaluate_once() {
    let output = compile_and_run(
        r#"
        let targetReads = 0;
        let argumentReads = 0;
        let implementations = 0;
        function pick(value: number): number;
        function pick(value: string): string;
        function pick(value: number | string | boolean): number | string | boolean {
            implementations++;
            return value;
        }
        function target() { targetReads++; return pick; }
        function argument(): number { argumentReads++; return 7; }
        function main(): void {
            const numberResult: number = target().call(undefined, argument());
            const stringResult: string = pick.apply(undefined, ["ok"]);
            console.log(numberResult, stringResult, targetReads, argumentReads, implementations);
        }
        "#,
        "overloaded_calls_use_selected_returns_and_evaluate_once",
    );
    assert_eq!(output, "7 ok 1 1 2\n");
}

#[test]
fn overloaded_rest_skips_implementation_fallback_slot() {
    let output = compile_and_run(
        r#"
        function choose(value: number): number;
        function choose(value: string, ...tail: number[]): string;
        function choose(value: number | string, fallback?: string | number[], ...tail: number[]): number | string {
            return typeof value === "number" ? value : value + tail[0];
        }
        function main(): void {
            const result: string = choose("x", 7);
            console.log(result);
        }
        "#,
        "overloaded_rest_skips_implementation_fallback_slot",
    );
    assert_eq!(output, "x7\n");
}

#[test]
fn overloaded_object_member_rest_skips_implementation_fallback_slot() {
    let output = compile_and_run(
        r#"
        function choose(value: number): number;
        function choose(value: string, ...tail: number[]): string;
        function choose(value: number | string, fallback?: string | number[], ...tail: number[]): number | string {
            return typeof value === "number" ? value : value + tail[0];
        }
        function main(): void {
            const object = { choose };
            const result: string = object.choose("x", 7);
            console.log(result);
        }
        "#,
        "overloaded_object_member_rest_skips_implementation_fallback_slot",
    );
    assert_eq!(output, "x7\n");
}

#[test]
fn overloaded_callback_uses_selected_context_and_adapts_physical_abi() {
    let output = compile_and_run(
        r#"
        function transform(kind: number, callback: (value: number) => number): number;
        function transform(kind: string, callback: (value: string) => string): string;
        function transform(kind: number | string, callback: (value: number | string) => number | string): number | string {
            if (typeof kind === "number") return callback(kind);
            return callback(kind);
        }
        function main(): void {
            const numberResult: number = transform(4, value => value + 1);
            const stringResult: string = transform("selected", value => value.toUpperCase());
            console.log(numberResult, stringResult);
        }
        "#,
        "overloaded_callback_uses_selected_context_and_adapts_physical_abi",
    );
    assert_eq!(output, "5 SELECTED\n");
}

fn cc_command() -> Command {
    let mut command = Command::new("cc");
    if cfg!(target_os = "linux") {
        command.arg("-no-pie");
        // thaw-runtime's num-bigint pulls in libm (log/exp/log2); the real `thaw build` link adds -lm too.
        // Listed first with --no-as-needed so the DSO is kept even though the archives come later.
        command.args(["-Wl,--no-as-needed", "-lm"]);
    }
    command
}
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
           declare function __thaw_typed_napi_6f7074696f6e616c(value: string | undefined): string | undefined;
           declare function __thaw_typed_napi_6e756c6c697368(value: string | null | undefined): string | null | undefined;
           declare function __thaw_typed_napi_6f7074696f6e616c6172726179(value: (string | undefined)[]): (string | undefined)[];
           function main(): void {
               const value: [number, string] = [7, "value"];
               const swapped: [string, number] = __thaw_typed_napi_737761705475706c65(value);
               const strings: string[] = __thaw_typed_napi_737472696e6773(["a", "b"]);
               const bools: boolean[] = __thaw_typed_napi_626f6f6c73([true, false]);
               const nested: string[][] = __thaw_typed_napi_6e6573746564([["a"]]);
               const objects: { name: string }[] = __thaw_typed_napi_6f626a65637473([{ name: "a" }]);
               const nullable: string | null = __thaw_typed_napi_6e756c6c61626c65(null);
               const nullableArray: (string | null)[] = __thaw_typed_napi_6e756c6c61626c656172726179(["a", null]);
               const optional: string | undefined = __thaw_typed_napi_6f7074696f6e616c(undefined);
               const nullish: string | null | undefined = __thaw_typed_napi_6e756c6c697368(undefined);
               const optionalArray: (string | undefined)[] = __thaw_typed_napi_6f7074696f6e616c6172726179(["a", undefined]);
               console.log(swapped[0]);
               console.log(strings[0]);
               console.log(bools[0]);
               console.log(nested[0][0]);
               console.log(objects[0].name);
               console.log(nullable);
               console.log(nullableArray[1]);
               console.log(optional);
               console.log(nullish);
               console.log(optionalArray[1]);
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
    let output = compile_and_run_raw(source, test_name, envs);
    assert!(
        output.status.success(),
        "binary exited {:?} (terminated by signal {:?}): stdout: {} stderr: {}",
        output.status.code(),
        std::os::unix::process::ExitStatusExt::signal(&output.status),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// For programs that must end unsuccessfully (an unhandled rejection of
/// `main` exits 1 in Node as well): `(stdout, stderr, exit code)`.
fn compile_and_run_failing(source: &str, test_name: &str) -> (String, String, Option<i32>) {
    let output = compile_and_run_raw(source, test_name, &[]);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn compile_and_run_raw(
    source: &str,
    test_name: &str,
    envs: &[(&str, &str)],
) -> std::process::Output {
    compile_and_run_raw_as(source, test_name, envs, false)
}

/// `commonjs` selects the entry's module kind (what thaw-cli derives from the source): a
/// CommonJS entry drains `process.nextTick` right after the main script, an ES module's does not.
fn compile_and_run_raw_as(
    source: &str,
    test_name: &str,
    envs: &[(&str, &str)],
    commonjs: bool,
) -> std::process::Output {
    compile_and_run_raw_with_opt(
        source,
        test_name,
        envs,
        commonjs,
        inkwell::OptimizationLevel::Default,
    )
}

/// `compile_and_run_raw_as` with the backend optimisation level chosen by the caller.
fn compile_and_run_raw_with_opt(
    source: &str,
    test_name: &str,
    envs: &[(&str, &str)],
    commonjs: bool,
    codegen_opt: inkwell::OptimizationLevel,
) -> std::process::Output {
    // The shared source map keeps the spans resolvable, so `Error.stack` frames get `file:line:col`.
    let (module, _) = thaw_parser::parse_typescript_with_shared_source_map_named(
        source,
        thaw_parser::common::FileName::Custom("input.ts".into()),
    )
    .unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, test_name);
    compiler.set_commonjs_entry(commonjs);
    compiler.set_codegen_opt(codegen_opt);
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
    // Like thaw-cli: the locale runtime is linked only when the program calls it.
    let intl_lib = compiler.uses_intl().then(|| build_staticlib("thaw-intl"));

    let mut linker = cc_command();
    linker.arg(&obj_path).arg(&arena_lib);
    if let Some(intl_lib) = &intl_lib {
        linker.arg(intl_lib).arg(&arena_lib);
    }
    let link_status = linker
        // `std_lib`/`runtime_lib` have a genuine two-way symbol
        // dependency now (see the sibling `compile_and_invoke_lambda`'s
        // own comment on this same pattern) -- listed twice each so a
        // single left-to-right archive scan resolves both directions.
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&quickjs_lib)
        // QuickJS-NG's C code calls libm math functions directly;
        // `rustc` normally adds `-lm` automatically when it does the
        // final link, but this is a manual `cc` invocation instead.
        .arg("-lm")
        // Matches thaw-cli's own real build (`quickjs_callback_export_
        // args`, `build.rs`): thaw-quickjs's `dlsym(RTLD_DEFAULT, ...)`
        // soft lookup of these thaw-runtime symbols (no hard crate
        // dependency between the two) needs them kept in the dynamic
        // symbol table -- without this, a program whose *only* async
        // suspension point is driven by `thaw_js_run_event_loop` (a
        // bare, unawaited top-level async call, as opposed to `main()`'s
        // own tracked completion) never finds `thaw_runtime_poll_one`
        // and its native continuation queue is never drained.
        .arg("-Wl,--export-dynamic-symbol=thaw_runtime_poll_one")
        .arg("-Wl,--export-dynamic-symbol=thaw_runtime_set_js_job_hooks")
        .arg("-Wl,--export-dynamic-symbol=thaw_promise_state")
        .arg("-Wl,--export-dynamic-symbol=thaw_promise_mark_handled")
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .envs(envs.iter().copied())
        .output()
        .expect("failed to execute compiled binary");
    if std::os::unix::process::ExitStatusExt::signal(&output.status).is_some() {
        diagnose_crashed_binary(&exe_path, envs);
    }
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// A compiled test program killed by a signal (heap corruption showed up this
/// way intermittently in CI and never locally): collect evidence for the
/// failure report instead of just the exit status. Re-runs the same binary a
/// bounded number of times with glibc's checking allocator, which names the
/// corruption kind, and, when `gdb` is installed, once under it for a
/// backtrace. Only runs on the failure path; the test still fails.
fn diagnose_crashed_binary(exe_path: &std::path::Path, envs: &[(&str, &str)]) {
    eprintln!("--- crash diagnostics for {} ---", exe_path.display());
    for command in [("uname", &["-srm"][..]), ("ldd", &["--version"][..])] {
        if let Ok(out) = Command::new(command.0).args(command.1).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            eprintln!("{}: {}", command.0, text.lines().next().unwrap_or(""));
        }
    }
    let mut crashes = 0;
    for attempt in 0..40 {
        let out = Command::new(exe_path)
            .envs(envs.iter().copied())
            .env("MALLOC_CHECK_", "3")
            .env("MALLOC_PERTURB_", ((attempt % 254) + 1).to_string())
            .output();
        let Ok(out) = out else { continue };
        if std::os::unix::process::ExitStatusExt::signal(&out.status).is_some() {
            crashes += 1;
            if crashes <= 3 {
                eprintln!(
                    "checked rerun {attempt}: signal {:?}: stderr: {}",
                    std::os::unix::process::ExitStatusExt::signal(&out.status),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
    }
    eprintln!("checked reruns that crashed: {crashes}/40");
    let gdb = Command::new("gdb").arg("--version").output().is_ok();
    eprintln!("gdb available: {gdb}");
    if gdb {
        for attempt in 0..20 {
            let Ok(out) = Command::new("gdb")
                .args([
                    "-batch",
                    "-ex",
                    "run",
                    "-ex",
                    "bt 25",
                    "-ex",
                    "info registers rip",
                ])
                .arg(exe_path)
                .output()
            else {
                break;
            };
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("SIGABRT")
                || text.contains("SIGSEGV")
                || text.contains("received signal")
            {
                eprintln!("gdb attempt {attempt}:\n{text}");
                break;
            }
        }
    }
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
    let (mut requests, stdout) = compile_and_invoke_lambda_events(source, test_name, &[event_body]);
    (requests.remove(0), stdout)
}

fn compile_and_invoke_lambda_events(
    source: &str,
    test_name: &str,
    events: &[&str],
) -> (Vec<String>, String) {
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

    let std_lib = build_staticlib("thaw-std");
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(build_staticlib("thaw-arena"))
        // `std_lib`/`runtime_lib` now have a genuine two-way symbol
        // dependency (thaw-std's `json.rs` calls a couple of
        // thaw-runtime date/string helpers; thaw-runtime's `AnyKey`/
        // any-array search call thaw-std's `Json` comparison helpers,
        // see thaw-std's own doc comment on `thaw_json_strict_equal`) --
        // a single left-to-right archive scan only resolves the
        // *forward* direction, so each is listed twice (a portable fix
        // that needs no `--start-group`/`--end-group` linker-specific
        // support): the second `std_lib` pass picks up what the first
        // `runtime_lib` pass left outstanding.
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let events = events
        .iter()
        .map(|event| event.to_string())
        .collect::<Vec<_>>();
    let invocation_count = events.len();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for event_body in events {
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
            conn.write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        }
    });

    let mut child = Command::new(&exe_path)
        .env("AWS_LAMBDA_RUNTIME_API", &addr)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn compiled Lambda handler binary");
    let post_requests = (0..invocation_count)
        .map(|_| {
            rx.recv_timeout(Duration::from_secs(10))
                .expect("handler never posted to the mock runtime API")
        })
        .collect();
    server.join().unwrap();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    (
        post_requests,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

include!("tests/async.rs");
include!("tests/classes.rs");
include!("tests/control_flow.rs");
include!("tests/core.rs");
include!("tests/collections.rs");
include!("tests/native_builtins.rs");
include!("tests/ffi.rs");
include!("tests/types.rs");
include!("tests/values.rs");
include!("tests/f_contracts.rs");
include!("tests/i_contracts.rs");
include!("tests/s_contracts.rs");
include!("tests/p_contracts.rs");
include!("tests/r_contracts.rs");
include!("tests/t_contracts.rs");
include!("tests/tick_contracts.rs");
include!("tests/z_contracts.rs");
include!("tests/thaw_js_contracts.rs");
include!("tests/thaw_js_handle_intrinsics_contracts.rs");
include!("tests/thaw_js_value_export_contracts.rs");
include!("tests/evloop_contracts.rs");
include!("tests/an_contracts.rs");
include!("tests/aa_contracts.rs");
include!("tests/ci_contracts.rs");
include!("tests/uc_contracts.rs");
include!("tests/function_length_contracts.rs");
include!("tests/newtarget_ctor_contracts.rs");
include!("tests/errfam_contracts.rs");
include!("tests/lv_contracts.rs");
include!("tests/rm_contracts.rs");
include!("tests/hs_contracts.rs");
include!("tests/fa_contracts.rs");
include!("tests/fc_contracts.rs");
include!("tests/fe_contracts.rs");
include!("tests/ga_contracts.rs");
include!("tests/e1_contracts.rs");
include!("tests/gc_contracts.rs");
include!("tests/gb_contracts.rs");
include!("tests/hb_contracts.rs");
include!("tests/ha_contracts.rs");
include!("tests/y1_contracts.rs");
include!("tests/h2_contracts.rs");
include!("tests/ia_contracts.rs");
include!("tests/jb_contracts.rs");
include!("tests/la_contracts.rs");
include!("tests/ja_contracts.rs");
include!("tests/h1_contracts.rs");
include!("tests/k3_contracts.rs");
include!("tests/r1o1_contracts.rs");
include!("tests/r1gap_contracts.rs");
include!("tests/r8f_contracts.rs");
include!("tests/na_contracts.rs");
include!("tests/nb3_contracts.rs");
include!("tests/nb4_contracts.rs");
include!("tests/d4_contracts.rs");
include!("tests/nb_contracts.rs");
include!("tests/kb_contracts.rs");
include!("tests/codegen_opt_contracts.rs");

#[test]
fn immediate_jsvalue_identity_lambda_reads_capture_without_retaining_environment() {
    let value = HirParam {
        name: "value".into(),
        ty: HirType::JsValue,
    };
    let program = HirProgram {
        functions: vec![HirFunction {
            name: "immediate_capture".into(),
            params: vec![value.clone()],
            ret: HirType::JsValue,
            is_async: false,
            body: vec![HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    vec![value.clone()],
                    Vec::new(),
                    HirType::JsValue,
                    Box::new(HirExpr::Var("value".into())),
                )),
                Vec::new(),
            )))],
        }],
        ..HirProgram::default()
    };
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "immediate_capture");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let symbol = ir.find("@immediate_capture(").unwrap();
    let start = ir[..symbol].rfind("define ").unwrap();
    let end = ir[symbol..]
        .find("\n}")
        .map(|offset| symbol + offset)
        .unwrap();
    let function_ir = &ir[start..end];
    assert!(
        function_ir.contains("load i64"),
        "expected the original JsValue parameter to be read: {function_ir}"
    );
    assert!(
        !function_ir.contains("thaw_js_retain_handle"),
        "identity capture must not retain an environment: {function_ir}"
    );
    assert!(
        !function_ir.contains("__thaw_lambda_"),
        "identity capture must not construct a closure: {function_ir}"
    );
}

#[test]
fn typed_quickjs_wrapper_lease_requires_all_paths_to_return_native_jsvalue() {
    let signature = thaw_hir::DynamicSignature {
        backend: thaw_hir::DynamicBackend::QuickJs,
        symbol: "__thaw_typed_js_test".into(),
        params: Vec::new(),
        ret: HirType::JsValue,
    };
    let dynamic_return =
        || HirStmt::Return(Some(HirExpr::DynamicCall(signature.clone(), Vec::new())));
    let proven = HirFunction {
        name: "__thaw_typed_wrapper_js_proven".into(),
        params: Vec::new(),
        ret: HirType::JsValue,
        is_async: false,
        body: vec![
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::OptionalIsNone(
                        Box::new(HirExpr::Var("options".into())),
                        HirType::JsValue,
                    )),
                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                ),
                vec![dynamic_return()],
                Vec::new(),
            ),
            dynamic_return(),
        ],
    };
    assert!(is_proven_typed_quickjs_wrapper(&proven));

    let alias = HirFunction {
        name: "__thaw_typed_wrapper_js_alias".into(),
        params: vec![HirParam {
            name: "value".into(),
            ty: HirType::JsValue,
        }],
        ret: HirType::JsValue,
        is_async: false,
        body: vec![HirStmt::Return(Some(HirExpr::Var("value".into())))],
    };
    assert!(!is_proven_typed_quickjs_wrapper(&alias));

    let json_alias = HirFunction {
        name: "__thaw_typed_wrapper_js_json_alias".into(),
        params: vec![HirParam {
            name: "value".into(),
            ty: HirType::Json,
        }],
        ret: HirType::Json,
        is_async: false,
        body: vec![HirStmt::Return(Some(HirExpr::Var("value".into())))],
    };
    assert!(!is_proven_typed_quickjs_wrapper(&json_alias));

    let escaping_argument = HirFunction {
        name: "__thaw_typed_wrapper_js_escaping_argument".into(),
        params: Vec::new(),
        ret: HirType::JsValue,
        is_async: false,
        body: vec![HirStmt::Return(Some(HirExpr::DynamicCall(
            signature.clone(),
            vec![HirExpr::Call(
                Box::new(HirExpr::Var("store_and_return".into())),
                vec![HirExpr::Var("borrowed".into())],
            )],
        )))],
    };
    assert!(!is_proven_typed_quickjs_wrapper(&escaping_argument));

    let side_effecting_body = HirFunction {
        name: "__thaw_typed_wrapper_js_side_effect".into(),
        params: Vec::new(),
        ret: HirType::JsValue,
        is_async: false,
        body: vec![
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("store_borrowed".into())),
                vec![HirExpr::Var("borrowed".into())],
            )),
            dynamic_return(),
        ],
    };
    assert!(!is_proven_typed_quickjs_wrapper(&side_effecting_body));
}
include!("tests/fb_contracts.rs");

include!("tests/ka_contracts.rs");
include!("tests/mapalias_contracts.rs");
include!("tests/lb_contracts.rs");
include!("tests/looseeq_contracts.rs");
include!("tests/construct_any_plain_contracts.rs");
include!("tests/anyprim_contracts.rs");
include!("tests/symkeys_contracts.rs");

include!("tests/privfield_contracts.rs");
include!("tests/hirdiv_contracts.rs");
include!("tests/conlog_contracts.rs");
include!("tests/bytes_range_contracts.rs");
include!("tests/bufferlit_contracts.rs");
include!("tests/buf_erase_contracts.rs");
include!("tests/newtarget_contracts.rs");

include!("tests/fnlen_contracts.rs");
include!("tests/langcore_contracts.rs");
include!("tests/exitfix_contracts.rs");
include!("tests/oobidx_contracts.rs");
include!("tests/fatalexit_contracts.rs");
include!("tests/rejects_contracts.rs");
include!("tests/declgaps_contracts.rs");
include!("tests/syntaxgaps_contracts.rs");
include!("tests/enumvals_contracts.rs");
include!("tests/builtingaps_contracts.rs");
include!("tests/typinggaps_contracts.rs");
include!("tests/classgaps_contracts.rs");
include!("tests/anyctor_contracts.rs");
include!("tests/anyclasses_contracts.rs");
include!("tests/classfields_contracts.rs");
include!("tests/clsfld_contracts.rs");
include!("tests/errobj_contracts.rs");
include!("tests/errstack_contracts.rs");
include!("tests/mapsub_contracts.rs");
include!("tests/arrsub_contracts.rs");
include!("tests/colrest_contracts.rs");
include!("tests/collim_contracts.rs");
include!("tests/bufview_contracts.rs");
include!("tests/bugfix_contracts.rs");
include!("tests/fldrest_contracts.rs");
include!("tests/fldlim_contracts.rs");
include!("tests/errrest_contracts.rs");
include!("tests/errrest2_contracts.rs");

include!("tests/oobread_contracts.rs");
include!("tests/bufu8_contracts.rs");
include!("tests/arraybuffer_contracts.rs");
include!("tests/lim3_contracts.rs");
include!("tests/library_mode.rs");
include!("tests/lim4_contracts.rs");
include!("tests/errframes_contracts.rs");
include!("tests/stackloc_contracts.rs");
include!("tests/anon_fn_contracts.rs");
include!("tests/audit_silent1_contracts.rs");
include!("tests/audit2_class_contracts.rs");
include!("tests/ce1_contracts.rs");
include!("tests/audit_inspect_contracts.rs");
include!("tests/audit_perf_contracts.rs");
include!("tests/inference_audit_contracts.rs");
include!("tests/generators_g034_contracts.rs");
include!("tests/arguments_object_contracts.rs");
include!("tests/accessor_inspect_contracts.rs");
include!("tests/audit2_error_async_contracts.rs");
include!("tests/audit2_recursive_contracts.rs");
include!("tests/bigint_arbitrary_contracts.rs");
include!("tests/error_inspect_contracts.rs");
include!("tests/audit2_inspect_format_contracts.rs");
include!("tests/perf_audit2_contracts.rs");
include!("tests/perf_audit2_leftovers_contracts.rs");
include!("tests/audit2_async_streams_contracts.rs");
include!("tests/audit2_node_api_leftover_contracts.rs");
include!("tests/function_identity_contracts.rs");
include!("tests/compile_speed_contracts.rs");
include!("tests/object_create_contracts.rs");
include!("tests/audit2_leftovers_contracts.rs");
include!("tests/sort_nan_contracts.rs");
