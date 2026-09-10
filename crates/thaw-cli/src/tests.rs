use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn compatibility_manifest_detects_expectation_changes() {
    let dir = std::env::temp_dir().join(format!("thaw-compat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("cases.json");
    std::fs::write(
        &manifest,
        r#"{"cases":[{"name":"primitive","expect":"supported","source":"function main(): void {}"}]}"#,
    )
    .unwrap();
    assert!(run_compat(&[manifest.to_string_lossy().into_owned()]).is_ok());
    std::fs::write(
        &manifest,
        r#"{"cases":[{"name":"regression","expect":"unsupported","source":"function main(): void {}"}]}"#,
    )
    .unwrap();
    assert!(run_compat(&[manifest.to_string_lossy().into_owned()]).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn compatibility_commands_are_bounded_by_a_timeout() {
    let output = command_output_with_timeout(
        Command::new("sh").args(["-c", "printf ok"]),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(output.stdout, b"ok");
    assert!(command_output_with_timeout(
        Command::new("sh").args(["-c", "sleep 1"]),
        Duration::from_millis(10),
    )
    .unwrap_err()
    .contains("timed out"));
}

include!("tests/static_build.rs");

include!("tests/acceptance.rs");

include!("tests/module_graph.rs");

include!("tests/registry_jit_aggregates.rs");
include!("tests/registry_jit_primitives.rs");
include!("tests/registry_runtime.rs");
include!("tests/registry_fallback.rs");

include!("tests/ffi_metadata.rs");

include!("tests/native_addons.rs");

#[test]
fn prepared_runtime_variants_have_stable_names() {
    assert_eq!(staticlib_variant(false, &[]), "default");
    assert_eq!(staticlib_variant(true, &[]), "minimal");
    assert_eq!(
        staticlib_variant(true, &["quickjs", "quickjs-tls"]),
        "quickjs-quickjs-tls"
    );
}

#[test]
fn install_uses_the_project_directory() {
    let command = npm_install_command(Path::new("web"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["install", "--prefix", "web"]
    );
}

#[test]
fn install_requires_a_package_manifest() {
    let directory = std::env::temp_dir().join(format!("thaw-cli-install-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let error = run_install(&[directory.display().to_string()]).unwrap_err();
    assert!(error.contains("does not contain package.json"));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn add_installs_named_packages_into_the_project() {
    let packages = ["zod".into(), "nanoid@5".into()];
    let command = npm_add_command(&packages, Path::new("api"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["install", "--prefix", "api", "zod", "nanoid@5"]
    );
}

#[test]
fn external_native_sidecars_replace_stale_contents() {
    let root = std::env::temp_dir().join(format!("thaw-sidecar-{}", std::process::id()));
    let staging = root.join("staging");
    let destination = root.join("app.native");
    std::fs::create_dir_all(staging.join("pkg")).unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(staging.join("pkg/native.node"), "new").unwrap();
    std::fs::write(destination.join("obsolete.node"), "old").unwrap();
    promote_external_native_directory(&staging, &destination).unwrap();
    assert!(destination.join("pkg/native.node").is_file());
    assert!(!destination.join("obsolete.node").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn run_uses_the_named_script_and_project_directory() {
    let command = npm_run_command("build", Path::new("web"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["run", "build", "--prefix", "web"]
    );
}

#[test]
fn run_requires_a_script_name() {
    assert_eq!(
        run_script(&[]).unwrap_err(),
        "usage: thaw run <script> [--prefix <directory>]"
    );
}

#[test]
fn direct_file_execution_forwards_arguments_and_removes_the_binary() {
    let directory = std::env::temp_dir().join(format!("thaw-cli-run-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("main.ts");
    let output = directory.join("app");
    let arguments = directory.join("arguments.json");
    std::fs::write(
        &input,
        format!(
            "import {{ writeFileSync }} from 'node:fs'; import {{ argv }} from 'node:process'; function main(): void {{ writeFileSync({}, JSON.stringify(argv)); }}",
            serde_json::to_string(arguments.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    assert_eq!(
        run_file_at(input.to_str().unwrap(), &["forwarded".into()], &output).unwrap(),
        0
    );
    let recorded: Vec<String> = serde_json::from_slice(&std::fs::read(arguments).unwrap()).unwrap();
    assert_eq!(recorded.last().map(String::as_str), Some("forwarded"));
    assert!(!output.exists());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn dev_fingerprint_tracks_sources_but_ignores_dependencies() {
    let directory = std::env::temp_dir().join(format!("thaw-cli-dev-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("node_modules/package")).unwrap();
    let source = directory.join("main.ts");
    std::fs::write(&source, "const value = 1;\n").unwrap();
    let roots = [directory.clone()];
    let initial = source_fingerprint(&roots).unwrap();
    std::fs::write(&source, "const value = 200;\n").unwrap();
    assert_ne!(source_fingerprint(&roots).unwrap(), initial);
    let changed = source_fingerprint(&roots).unwrap();
    std::fs::write(directory.join("node_modules/package/index.js"), "changed").unwrap();
    assert_eq!(source_fingerprint(&roots).unwrap(), changed);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn dev_reuses_built_vite_assets_for_backend_only_changes() {
    assert_eq!(
        reuse_vite_assets(
            &["server.ts".into(), "--vite".into(), "web".into()],
            Path::new("web"),
        ),
        ["server.ts", "--assets", "web/dist"]
    );
}

#[test]
fn dev_reads_entry_and_vite_defaults_from_the_project() {
    let directory =
        std::env::temp_dir().join(format!("thaw-cli-dev-project-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("package.json"),
        r#"{"thaw":{"entry":"server.ts","vite":"web"}}"#,
    )
    .unwrap();
    std::fs::write(directory.join("server.ts"), "function main(): void {}\n").unwrap();
    assert_eq!(
        dev_project_paths(&[], &directory).unwrap(),
        (PathBuf::from("server.ts"), Some(PathBuf::from("web")))
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn dev_accepts_an_npm_project_directory() {
    let directory = std::env::temp_dir().join(format!("thaw-cli-dev-npm-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(
        directory.join("package.json"),
        r#"{"scripts":{"build":"vite build","dev":"tsx watch src/server.ts"}}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("src/server.ts"),
        "function main(): void {}\n",
    )
    .unwrap();
    assert_eq!(
        dev_project_paths(&[directory.display().to_string()], Path::new(".")).unwrap(),
        (directory.join("src/server.ts"), Some(directory.clone()))
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn project_build_reads_package_defaults() {
    assert_eq!(
        project_build_defaults(r#"{"thaw":{"entry":"server.ts","vite":"web","output":"app"}}"#)
            .unwrap(),
        (
            Some(PathBuf::from("server.ts")),
            Some(PathBuf::from("web")),
            Some(PathBuf::from("app"))
        )
    );
}

#[test]
fn project_build_uses_standard_npm_entry_and_vite_fields() {
    assert_eq!(
        project_build_defaults(r#"{"source":"src/server.ts","scripts":{"build":"vite build"}}"#)
            .unwrap(),
        (
            Some(PathBuf::from("src/server.ts")),
            Some(PathBuf::from(".")),
            None
        )
    );
    assert_eq!(
        project_build_defaults(r#"{"main":"index.ts"}"#).unwrap().0,
        Some(PathBuf::from("index.ts"))
    );
    assert_eq!(
        project_build_defaults(r#"{"module":"server.mjs","main":"index.cjs"}"#)
            .unwrap()
            .0,
        Some(PathBuf::from("server.mjs"))
    );
    assert_eq!(
        project_build_defaults(r#"{"scripts":{"start":"tsx watch src/api.ts"}}"#)
            .unwrap()
            .0,
        Some(PathBuf::from("src/api.ts"))
    );
}

#[test]
fn build_accepts_an_npm_project_directory() {
    let directory = std::env::temp_dir().join(format!("thaw-npm-project-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(
        directory.join("package.json"),
        r#"{"main":"dist/index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("src/index.ts"),
        "function main(): void { console.log('npm project'); }\n",
    )
    .unwrap();
    run_build(&[directory.display().to_string()]).unwrap();
    run_inspect(&[directory.join("app").display().to_string()]).unwrap();
    let result = Command::new(directory.join("app")).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "npm project\n");
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[test]
fn build_accepts_a_hoisted_npm_workspace_package() {
    let root = std::env::temp_dir().join(format!("thaw-npm-workspace-{}", std::process::id()));
    let app = root.join("packages/app");
    let shared = root.join("packages/shared");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(root.join("node_modules/@example")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    std::fs::write(app.join("package.json"), r#"{"main":"src/index.ts"}"#).unwrap();
    std::fs::write(
        app.join("src/index.ts"),
        "import { double } from '@example/shared'; function main(): void { console.log(double(21)); }\n",
    )
    .unwrap();
    std::fs::write(
        shared.join("package.json"),
        r#"{"name":"@example/shared","main":"index.js","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        shared.join("index.d.ts"),
        "export declare function double(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        shared.join("index.js"),
        "exports.double = value => value * 2;\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&shared, root.join("node_modules/@example/shared")).unwrap();

    run_build(&[app.display().to_string()]).unwrap();
    let result = Command::new(app.join("app")).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn frontend_only_vite_projects_get_a_static_server_entry() {
    let entry = write_static_asset_server_entry().unwrap();
    let source = std::fs::read_to_string(&entry).unwrap();
    assert!(source.contains("createServer"));
    assert!(source.contains("thawServeAsset(response, request.url)"));
    std::fs::remove_file(entry).unwrap();
}

#[test]
fn vite_assets_are_embedded_with_routes_and_content_types() {
    let directory = std::env::temp_dir().join(format!("thaw-cli-assets-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("assets")).unwrap();
    std::fs::write(directory.join("index.html"), "<main>hello</main>").unwrap();
    std::fs::write(directory.join("assets/app.js"), "console.log('hello')").unwrap();
    std::fs::write(
        directory.join("assets/image.png"),
        [0x89, 0x50, 0x4e, 0x47, 0xff],
    )
    .unwrap();

    let shim = generate_asset_shim(&directory).unwrap();
    assert!(shim.contains("path = thawAssetPath(path);"));
    assert!(shim.contains("if (path === \"/\") { return \"text/html; charset=utf-8\"; }"));
    assert!(shim.contains("if (path === \"/assets/app.js\") { return \"console.log('hello')\"; }"));
    assert!(shim.contains("if (path === \"/assets/image.png\") { return \"89504e47ff\"; }"));
    assert!(shim.contains("if (path === \"/assets/image.png\") { return \"hex\"; }"));
    assert!(shim.contains("function thawAssetRoute(path: string): string"));
    assert!(shim.contains("function thawServeAsset(response:"));

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn embedded_frontend_and_api_run_without_the_asset_directory() {
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let directory =
        std::env::temp_dir().join(format!("thaw-cli-fullstack-{}-{port}", std::process::id()));
    let assets = directory.join("dist");
    std::fs::create_dir_all(assets.join("assets")).unwrap();
    std::fs::write(
        assets.join("index.html"),
        "<main>Thaw SPA</main><script src=/assets/app.js></script>",
    )
    .unwrap();
    std::fs::write(assets.join("assets/app.js"), "console.log('vite')").unwrap();
    std::fs::write(
        assets.join("assets/app.css"),
        "@font-face{src:url('/assets/font.woff2')}",
    )
    .unwrap();
    std::fs::write(assets.join("assets/font.woff2"), [0, 1, 2, 0xff]).unwrap();
    let entry = directory.join("server.ts");
    std::fs::write(
        &entry,
        format!(
            r#"import {{ createServer }} from "node:http";
function main(): void {{
  const server = createServer((request: {{ method: string; url: string }}, response: {{ statusCode: number; setHeader: (name: string, value: string) => boolean; end: (body: string) => boolean; write: (body: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }}): boolean => {{
    if (request.url === "/api/message") {{
      response.setHeader("Content-Type", "application/json; charset=utf-8");
      return response.end(request.method === "POST" ? "{{\"saved\":true}}" : "{{\"message\":\"hello\"}}");
    }}
    if (request.method === "GET" && thawServeAsset(response, request.url)) {{ return true; }}
    response.statusCode = 404;
    return response.end("Not Found");
  }});
  server.listenMany({}, 6);
}}
"#,
            port
        ),
    )
    .unwrap();
    let executable = directory.join("app");
    build_with_assets(
        &entry,
        &executable,
        &[],
        &[],
        &[],
        &directory.join("registry"),
        &[],
        false,
        Some(&assets),
    )
    .unwrap();
    std::fs::remove_dir_all(&assets).unwrap();

    let child = Command::new(&executable)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fn request(port: u16, method: &str, target: &str) -> Vec<u8> {
        let mut stream = (0..200)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    None
                }
            })
            .expect("compiled full-stack server did not start listening");
        stream
            .write_all(
                format!(
                    "{method} {target} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        response
    }
    let responses = [
        request(port, "GET", "/"),
        request(port, "GET", "/assets/app.js?hash=1"),
        request(port, "GET", "/assets/app.css"),
        request(port, "GET", "/assets/font.woff2"),
        request(port, "GET", "/orders/42"),
        request(port, "POST", "/api/message"),
    ];
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(responses[0].ends_with(b"<main>Thaw SPA</main><script src=/assets/app.js></script>"));
    assert!(responses[1].ends_with(b"console.log('vite')"));
    assert!(responses[2].windows(8).any(|value| value == b"text/css"));
    assert!(responses[3].ends_with(&[0, 1, 2, 0xff]));
    assert!(responses[4].ends_with(b"<main>Thaw SPA</main><script src=/assets/app.js></script>"));
    assert!(responses[5].ends_with(b"{\"saved\":true}"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn adapts_typed_dynamic_callable_results_to_natural_calls() {
    let function = thaw_bridge::DtsFunction {
        name: "customAlphabet".into(),
        generic: None,
        params: vec![
            (
                "alphabet".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "defaultSize".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::F64,
                ))),
            ),
        ],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::CallableFunction(
            vec![thaw_hir::HirType::Optional(Box::new(
                thaw_hir::HirType::F64,
            ))],
            thaw_hir::HirOptionalMask::from_bools(&[true]),
            None,
            Box::new(thaw_hir::HirType::Str),
        )),
    };
    let no_observed_arities = std::collections::BTreeSet::new();
    let (target, shim) =
        typed_dynamic_declaration("nanoid", &function, false, &no_observed_arities, None).unwrap();
    assert!(target.starts_with("__thaw_typed_callable_"));
    assert!(shim.contains("defaultSize?: number | undefined"));
    assert!(shim.contains("const invoke: (arg0?: number | undefined) => string"));
    assert!(shim.contains("callDynamicValue(callable"));

    let (_, napi_shim) =
        typed_dynamic_declaration("native", &function, true, &no_observed_arities, None).unwrap();
    assert!(napi_shim.contains("callNativeAddonValue(callable"));
}

#[test]
fn widens_callable_union_parameters_at_the_quickjs_boundary() {
    let function = thaw_bridge::DtsFunction {
        name: "replace".into(),
        generic: None,
        params: vec![(
            "replacement".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
                thaw_hir::HirType::CallableFunction(
                    vec![thaw_hir::HirType::Str],
                    thaw_hir::HirOptionalMask::from_bools(&[false]),
                    Some(Box::new(thaw_hir::HirType::Json)),
                    Box::new(thaw_hir::HirType::Str),
                ),
                thaw_hir::HirType::Str,
            ])),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
    };
    let (_, declaration) = typed_dynamic_declaration(
        "text-kit",
        &function,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(declaration.contains("replacement: Json"));
}

#[test]
fn renders_namespace_scoped_options_for_quickjs_arity_dispatch() {
    let functions = thaw_bridge::parse_dts(
        r#"declare namespace ParcelWatcher {
            export type BackendType = "fs-events" | "watchman" | "inotify" | "windows";
            export interface Options { backend?: BackendType; }
            export type SubscribeCallback = (err: Error | null, events: Event[]) => unknown;
            export interface Event { type: "create" | "update" | "delete"; path: string; }
            export function subscribe(dir: string, fn: SubscribeCallback, opts?: Options): Promise<void>;
        }
        export = ParcelWatcher;"#,
    )
    .unwrap();
    let function = functions
        .iter()
        .find(|function| function.name == "subscribe")
        .unwrap();
    let (_, declaration) = typed_dynamic_declaration(
        "@parcel/watcher",
        function,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(
        declaration.contains("fn: Json, opts: { backend: string | undefined }"),
        "{declaration}"
    );
}

#[test]
fn quickjs_manifest_detection_tracks_dynamic_host_calls() {
    assert!(!source_uses_quickjs("function main() { return 42; }"));
    assert!(source_uses_quickjs(
        "function main() { return callDynamic('add', []); }"
    ));
    assert!(source_uses_quickjs(
        "declare function __thaw_typed_js_616464(value: number): number;"
    ));
}

#[test]
fn quickjs_fallback_reasons_include_operation_package_and_location() {
    let source = "import { parse } from 'dynamic-package';\nfunction main(): void { parse('x'); callDynamic('other', []); }\n";
    let mut exports = ExternalExports::new();
    exports.insert(
        "dynamic-package".into(),
        [("parse".into(), "__thaw_typed_js_7061727365".into())]
            .into_iter()
            .collect(),
    );
    let jit_fallback_reasons = [(
        ("dynamic-package".into(), "parse".into()),
        "function body uses unsupported control flow".into(),
    )]
    .into_iter()
    .collect();
    let reasons = quickjs_fallback_reasons(
        Path::new("main.ts"),
        source,
        "declare function __thaw_typed_js_64796e616d69632d7061636b6167653a3a7061727365(): string;",
        &["dynamic-package".into()],
        &exports,
        &jit_fallback_reasons,
    );
    assert!(reasons.iter().any(|reason| {
        reason["kind"] == "dynamic-operation"
            && reason["operation"] == "callDynamic"
            && reason["line"] == 2
    }));
    assert!(reasons.iter().any(|reason| {
        reason["kind"] == "registry-fallback"
            && reason["package"] == "dynamic-package"
            && reason["function"] == "parse"
            && reason["detail"] == "function body uses unsupported control flow"
            && reason["line"] == 2
    }));
}

#[test]
fn jit_rejection_reasons_distinguish_signature_from_body() {
    let mut function = thaw_bridge::DtsFunction {
        name: "parse".into(),
        generic: None,
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Json),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let reason = jit_rejection_reason("exports.parse = value => value", &function);
    assert!(reason.contains("parameter `value` has unsupported JIT type"));
    assert!(reason.contains("Json"));

    function.params[0].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    assert!(
        jit_rejection_reason("exports.parse = value => Date.now()", &function)
            .contains("function body uses an expression")
    );
}

#[test]
fn wasm_host_detection_only_enables_wasm_users() {
    assert!(!source_uses_wasm("module.exports = value => value + 1"));
    assert!(source_uses_wasm("new WebAssembly.Module(bytes)"));
    assert!(source_uses_wasm("require('wasi')"));
}

#[test]
fn tls_host_detection_only_enables_tls_bundles() {
    assert!(!source_uses_tls("module.exports = value => value + 1"));
    assert!(source_uses_tls("__thaw_tls_connect(host, port)"));
}

#[test]
fn brotli_host_detection_only_enables_brotli_users() {
    assert!(!source_uses_brotli("new CompressionStream('gzip')"));
    assert!(source_uses_brotli("brotliCompressSync(input)"));
    assert!(source_uses_brotli("new BrotliCompress()"));
}

#[test]
fn node_compat_manifest_covers_each_public_builtin_family() {
    let manifest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/node-compat.json"
    ))
    .unwrap();
    for builtin in [
        "assert",
        "async_hooks",
        "buffer",
        "child_process",
        "cluster",
        "console",
        "constants",
        "crypto",
        "dgram",
        "diagnostics_channel",
        "dns",
        "dns/promises",
        "domain",
        "events",
        "fs",
        "fs/promises",
        "http",
        "http2",
        "https",
        "inspector",
        "inspector/promises",
        "module",
        "net",
        "os",
        "path",
        "path/posix",
        "path/win32",
        "perf_hooks",
        "process",
        "punycode",
        "querystring",
        "readline",
        "readline/promises",
        "repl",
        "stream",
        "stream/consumers",
        "stream/promises",
        "stream/web",
        "string_decoder",
        "test",
        "test/reporters",
        "timers",
        "timers/promises",
        "tls",
        "trace_events",
        "tty",
        "url",
        "util",
        "util/types",
        "v8",
        "vm",
        "wasi",
        "worker_threads",
        "zlib",
    ] {
        assert!(
            manifest.contains(&format!("node:{builtin}")),
            "Node compatibility manifest does not cover node:{builtin}"
        );
    }
}

include!("tests/jit_exports.rs");
include!("tests/jit_analysis.rs");
include!("tests/external_classes.rs");
include!("tests/overload_inference.rs");
