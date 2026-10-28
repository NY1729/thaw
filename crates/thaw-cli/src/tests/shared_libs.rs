// `--shared-rt` (opt-in libthaw_rt.so.1) contract tests.
fn shared_rt_build(dir: &Path, name: &str, source: &str, shared_rt: bool) -> Result<PathBuf, String> {
    let input = dir.join(format!("{name}.ts"));
    let output = dir.join(name);
    std::fs::write(&input, source).unwrap();
    build_with_native_mode_shared(
        &input,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        false,
        None,
        true,
        false,
        false,
        shared_rt,
    )?;
    Ok(output)
}

#[test]
fn shared_rt_hello_runs_and_matches_inline() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-shared-rt-hello-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "function main(): void { console.log(\"hi\", 1 + 2); }\n";
    let inline = shared_rt_build(&dir, "inline", source, false).unwrap();
    let shared = shared_rt_build(&dir, "shared", source, true).unwrap();

    let inline_run = Command::new(&inline).output().unwrap();
    let shared_run = Command::new(&shared).output().unwrap();
    assert!(inline_run.status.success(), "{}", String::from_utf8_lossy(&inline_run.stderr));
    assert!(shared_run.status.success(), "{}", String::from_utf8_lossy(&shared_run.stderr));
    assert_eq!(inline_run.stdout, shared_run.stdout);

    let readelf = |path: &Path| {
        String::from_utf8_lossy(&Command::new("readelf").arg("-d").arg(path).output().unwrap().stdout).into_owned()
    };
    assert!(readelf(&shared).contains("libthaw_rt.so.1"), "{}", readelf(&shared));
    assert!(!readelf(&inline).contains("libthaw_rt"), "{}", readelf(&inline));
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&shared).unwrap()).unwrap();
    assert_eq!(manifest["shared_rt"], true);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shared_rt_rejects_static() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-shared-rt-static-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let error = shared_rt_build(&dir, "app", "function main(): void {}\n", true)
        .map(|_| ())
        .err();
    assert!(error.is_none(), "dynamic shared-rt build must succeed: {error:?}");
    let input = dir.join("app.ts");
    let rejected = build_with_native_mode_shared(
        &input,
        &dir.join("static-app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
        None,
        true,
        false,
        false,
        true,
    )
    .unwrap_err();
    assert!(rejected.contains("--shared-rt cannot be combined with --static"), "{rejected}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shared_rt_default_off_manifest_is_unchanged() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-shared-rt-default-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "function main(): void { console.log(\"same\"); }\n";
    let first = shared_rt_build(&dir, "first", source, false).unwrap();
    let second = shared_rt_build(&dir, "second", source, false).unwrap();
    let first_manifest = artifact_manifest_from_bytes(&std::fs::read(&first).unwrap()).unwrap();
    let second_manifest = artifact_manifest_from_bytes(&std::fs::read(&second).unwrap()).unwrap();
    assert_eq!(first_manifest, second_manifest);
    assert!(first_manifest.get("shared_rt").is_none(), "{first_manifest}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn shared_rt_c_abi_v1_links_and_runs_from_c() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-shared-rt-c-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let lib = build_rt_shared_library().unwrap();
    std::fs::write(
        dir.join("abi.c"),
        "#include <stdint.h>\n\
         typedef struct { uint32_t abi_version; uint32_t reserved[7]; } ThawConfig;\n\
         uint32_t thaw_rt_abi_version(void);\n\
         int32_t thaw_rt_init(const ThawConfig *config);\n\
         void thaw_rt_shutdown(void);\n\
         int main(void) {\n\
           if (thaw_rt_abi_version() != 1) return 1;\n\
           if (thaw_rt_init(0) != 0) return 2;\n\
           ThawConfig bad = {99, {0}};\n\
           if (thaw_rt_init(&bad) != 2) return 3;\n\
           thaw_rt_shutdown();\n\
           return 0;\n\
         }\n",
    )
    .unwrap();
    let exe = dir.join("abi");
    let status = Command::new("cc")
        .arg(dir.join("abi.c"))
        .arg(&lib)
        .arg(format!("-Wl,-rpath,{}", lib.parent().unwrap().display()))
        .arg("-o")
        .arg(&exe)
        .status()
        .unwrap();
    assert!(status.success());
    let run = Command::new(&exe).env_remove("LD_LIBRARY_PATH").status().unwrap();
    assert_eq!(run.code(), Some(0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rt_abi_hash_is_read_from_the_marker_only() {
    let mut bytes = b"junk THAW_ABI_HASH=0123456789abcdef; more".to_vec();
    assert_eq!(rt_abi_hash_of_bytes(&bytes).unwrap(), "0123456789abcdef");
    bytes = b"no marker here".to_vec();
    assert!(rt_abi_hash_of_bytes(&bytes).is_err());
    bytes = b"THAW_ABI_HASH=zz".to_vec();
    assert!(rt_abi_hash_of_bytes(&bytes).is_err());
}

#[test]
fn shared_rt_runtime_typeof_compares_by_content() {
    // The runtime's `typeof` string lives in libthaw_rt.so's rodata; comparing it by pointer
    // with a literal only worked when a static link merged both.
    let dir = std::env::temp_dir().join(format!("thaw-cli-shared-rt-typeof-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "class Q { [Symbol.toPrimitive](hint: string): any { return hint === \"number\" ? 7 : \"Q:\" + hint; } }\n\
                  function main(): void { const q = new Q(); console.log(q + 1, q == \"Q:default\"); }\n";
    let inline = shared_rt_build(&dir, "inline", source, false).unwrap();
    let shared = shared_rt_build(&dir, "shared", source, true).unwrap();
    let inline_out = Command::new(&inline).output().unwrap().stdout;
    let shared_out = Command::new(&shared).output().unwrap().stdout;
    assert_eq!(String::from_utf8_lossy(&shared_out), "Q:default1 true\n");
    assert_eq!(inline_out, shared_out);
    let _ = std::fs::remove_dir_all(dir);
}

// ---- `thaw lib` (library mode, store, manifest, header) ----

fn lib_fixture_registry(dir: &Path, package: &str, dts: &str, bundle: &str) -> PathBuf {
    let registry = dir.join("modules");
    let package_dir = registry.join(package);
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(package_dir.join("package.d.ts"), dts).unwrap();
    std::fs::write(package_dir.join("bundle.js"), bundle).unwrap();
    std::fs::write(package_dir.join("version.txt"), "1.2.3\n").unwrap();
    registry
}

const LIB_CLAMP_DTS: &str = "export declare function clamp(n: number, lo: number, hi: number): number;\nexport declare function greet(s: string): string;\n";
const LIB_CLAMP_BUNDLE: &str = "'use strict'; module.exports = { clamp: function(n, lo, hi) { if (n < lo) return lo; if (n > hi) return hi; return n; }, greet: function(s) { return 'hi ' + s; } };\n";

fn lib_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thaw-cli-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lib_command_stdout(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn lib_built_clamp(dir: &Path, store_name: &str) -> (PathBuf, String, PathBuf) {
    let registry = lib_fixture_registry(dir, "clamp-fixture", LIB_CLAMP_DTS, LIB_CLAMP_BUNDLE);
    let store = dir.join(store_name);
    let (key, built) = lib_build("clamp-fixture", &registry, &store, false).unwrap();
    assert!(built);
    let so = store.join("store").join(&key).join("libthaw_pkg_clamp_fixture.so");
    (store, key, so)
}

#[test]
fn lib_build_clamp_fixture_exports_scalar_symbol() {
    let dir = lib_test_dir("export");
    let (store, key, so) = lib_built_clamp(&dir, "store");
    let entry = store.join("store").join(&key);
    assert!(entry.join("clamp_fixture.h").is_file());
    let header = std::fs::read_to_string(entry.join("clamp_fixture.h")).unwrap();
    assert!(header.contains("double thaw_pkg_clamp_fixture_clamp(double, double, double);"), "{header}");

    let nm = lib_command_stdout(Command::new("nm").arg("-D").arg("--defined-only").arg(&so));
    let defined: Vec<&str> = nm.lines().filter_map(|line| line.split_whitespace().nth(2)).collect();
    assert!(defined.contains(&"thaw_pkg_clamp_fixture_clamp"), "{nm}");
    assert!(!defined.contains(&"greet"), "{nm}");
    assert!(defined.iter().all(|name| name.starts_with("thaw_pkg_") || name.starts_with("__thaw_pkg_")), "{nm}");

    let dynamic = lib_command_stdout(Command::new("readelf").arg("-d").arg(&so));
    assert!(dynamic.contains("libthaw_rt.so.1"), "{dynamic}");

    // `.thaw.pkg` is the manifest without its sha256.
    let section = lib_command_stdout(Command::new("readelf").args(["-p", ".thaw.pkg"]).arg(&so));
    assert!(section.contains("\"name\": \"clamp-fixture\""), "{section}");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry.join("thaw.json")).unwrap()).unwrap();
    assert_eq!(manifest["not_exported"][0]["name"], "greet");
    assert!(!manifest["not_exported"][0]["reason"].as_str().unwrap().is_empty());
    assert_eq!(manifest["exports"][0]["ts_signature"], "(n: number, lo: number, hi: number) => number");
    assert_eq!(manifest["sha256"], sha256_hex(&std::fs::read(&so).unwrap()));
    let bytes = std::fs::read(&so).unwrap();
    let marker = b"{\n  \"deps\"";
    let start = bytes.windows(marker.len()).position(|w| w == marker).unwrap();
    let end = start + bytes[start..].iter().position(|b| *b == 0).unwrap();
    let mut embedded: serde_json::Value = serde_json::from_slice(&bytes[start..end]).unwrap();
    let mut expected = manifest.clone();
    expected.as_object_mut().unwrap().remove("sha256");
    assert_eq!(embedded.take(), expected);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_called_from_c() {
    let dir = lib_test_dir("c");
    let (_store, _key, so) = lib_built_clamp(&dir, "store");
    std::fs::write(
        dir.join("main.c"),
        "#include <stdio.h>\n#include <math.h>\ndouble thaw_pkg_clamp_fixture_clamp(double, double, double);\n\
         int main(void) {\n  double c[5][3] = {{5,1,3},{0,1,3},{2,1,3},{NAN,1,3},{9,8.5,1.5}};\n\
         for (int i = 0; i < 5; i++) printf(\"%g\\n\", thaw_pkg_clamp_fixture_clamp(c[i][0], c[i][1], c[i][2]));\n  return 0;\n}\n",
    )
    .unwrap();
    let exe = dir.join("c_client");
    lib_command_stdout(
        Command::new("cc")
            .arg(dir.join("main.c"))
            .arg(&so)
            .arg(format!("-Wl,-rpath,{}", so.parent().unwrap().display()))
            .arg("-lm")
            .arg("-o")
            .arg(&exe),
    );
    let out = lib_command_stdout(Command::new(&exe).env_remove("LD_LIBRARY_PATH"));
    assert_eq!(out, "3\n1\n2\nnan\n1.5\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_called_from_thaw_program() {
    let dir = lib_test_dir("thaw");
    let (_store, _key, so) = lib_built_clamp(&dir, "store");
    let registry = dir.join("modules");
    let cases = "console.log(clamp(5, 1, 3), clamp(0, 1, 3), clamp(2, 1, 3), clamp(NaN, 1, 3), clamp(9, 8.5, 1.5));";
    let build = |name: &str, source: String, links: &[PathBuf]| {
        let input = dir.join(format!("{name}.ts"));
        let output = dir.join(name);
        std::fs::write(&input, source).unwrap();
        build_with_native_mode_shared(&input, &output, links, &[], &[], &registry, &[], false, None, true, false, false, false)
            .unwrap();
        lib_command_stdout(&mut Command::new(&output))
    };
    let inline = build(
        "inline",
        format!("import {{ clamp }} from \"clamp-fixture\";\nfunction main(): void {{ {cases} }}\n"),
        &[],
    );
    let linked = build(
        "linked",
        format!(
            "declare function thaw_pkg_clamp_fixture_clamp(n: number, lo: number, hi: number): number;\n\
             function clamp(n: number, lo: number, hi: number): number {{ return thaw_pkg_clamp_fixture_clamp(n, lo, hi); }}\n\
             function main(): void {{ {cases} }}\n"
        ),
        &[so.clone(), PathBuf::from(format!("-Wl,-rpath,{}", so.parent().unwrap().display()))],
    );
    assert_eq!(inline, linked);
    assert_eq!(linked, "3 1 2 NaN 1.5\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_build_twice_is_byte_identical() {
    let dir = lib_test_dir("twice");
    let (store_a, key_a, so_a) = lib_built_clamp(&dir, "store_a");
    let (_store_b, key_b, so_b) = lib_built_clamp(&dir, "store_b");
    assert_eq!(key_a, key_b);
    assert_eq!(std::fs::read(&so_a).unwrap(), std::fs::read(&so_b).unwrap());
    let registry = dir.join("modules");
    let (key_again, built) = lib_build("clamp-fixture", &registry, &store_a, false).unwrap();
    assert_eq!((key_again, built), (key_a, false));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_build_rejects_non_scalar_only_packages() {
    let dir = lib_test_dir("nonscalar");
    let registry = lib_fixture_registry(
        &dir,
        "greeter",
        "export declare function greet(s: string): string;\n",
        "module.exports = { greet: function(s) { return 'hi ' + s; } };\n",
    );
    let error = lib_build("greeter", &registry, &dir.join("store"), false).unwrap_err();
    assert!(error.contains("no exportable functions"), "{error}");
    assert!(!dir.join("store").join("store").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_default_build_unaffected() {
    let dir = lib_test_dir("default");
    let registry = lib_fixture_registry(&dir, "clamp-fixture", LIB_CLAMP_DTS, LIB_CLAMP_BUNDLE);
    let store = dir.join("store");
    let input = dir.join("app.ts");
    std::fs::write(&input, "import { clamp } from \"clamp-fixture\";\nfunction main(): void { console.log(clamp(5, 1, 3)); }\n").unwrap();
    let output = dir.join("app");
    build_with_native_mode_shared(&input, &output, &[], &[], &[], &registry, &[], false, None, true, false, false, false).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert!(manifest.get("shared_rt").is_none());
    assert!(!store.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lib_verify_detects_tamper() {
    let dir = lib_test_dir("tamper");
    let (store, key, so) = lib_built_clamp(&dir, "store");
    assert!(lib_verify(&store, Some(&key)).is_ok());
    let mut bytes = std::fs::read(&so).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&so, bytes).unwrap();
    assert!(lib_verify(&store, Some(&key)).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shared_rt_links_a_jit_registry_program() {
    // A registry function compiled by the specialization JIT calls `thaw_jit_call_f64`,
    // which libthaw_rt.so must provide.
    let dir = lib_test_dir("sharedjit");
    let registry = lib_fixture_registry(
        &dir,
        "clamp-fixture",
        "export declare function clamp(n: number, lo: number, hi: number): number;\n",
        LIB_CLAMP_BUNDLE,
    );
    let input = dir.join("app.ts");
    std::fs::write(
        &input,
        "import { clamp } from \"clamp-fixture\";\nfunction main(): void { console.log(clamp(5, 1, 3), clamp(0, 1, 3)); }\n",
    )
    .unwrap();
    let build = |name: &str, shared: bool| {
        let output = dir.join(name);
        build_with_native_mode_shared(&input, &output, &[], &[], &[], &registry, &[], false, None, true, false, false, shared)
            .unwrap();
        lib_command_stdout(&mut Command::new(&output))
    };
    assert_eq!(build("inline", false), "3 1\n");
    assert_eq!(build("shared", true), "3 1\n");
    let manifest = artifact_manifest_from_bytes(&std::fs::read(dir.join("shared")).unwrap()).unwrap();
    assert_eq!(manifest["shared_rt"], true, "fell back to the static runtime");
    let _ = std::fs::remove_dir_all(dir);
}
