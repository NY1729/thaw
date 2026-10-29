// `thaw lib build|list|verify`: compile a registry package's scalar exports into a shared library
// (`libthaw_pkg_<ident>.so`) kept in a content-addressed store. See designs/npm-shared-libs.

use sha2::{Digest, Sha256};
use thaw_llvm::hir_codegen::LibraryExport;

const LIB_USAGE: &str = "usage: thaw lib build <package> [--registry <dir>] [--store <dir>] [--no-install]\n       thaw lib list [--store <dir>]\n       thaw lib verify [<key>] [--store <dir>]";

/// What `build` needs to compile the generated entry as a library.
struct LibBuildMode {
    exports: Vec<LibraryExport>,
    /// `.thaw.pkg` payload (the manifest without `sha256`).
    metadata: Vec<u8>,
    soname: String,
    /// Directory holding `libthaw_rt.so.1` at link time.
    rt_dir: PathBuf,
    /// `DT_RUNPATH`, relative to the library (`$ORIGIN/../../rt/<abi hash>`).
    rpath: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// `just-clamp` -> `just_clamp`.
fn package_ident(package: &str) -> String {
    package.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// Recursively rebuilds objects with sorted keys, so output is stable whatever `serde_json` features
/// the build enabled.
fn sorted_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(entries.into_iter().map(|(k, v)| (k, sorted_json(v))).collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sorted_json).collect())
        }
        other => other,
    }
}

/// The store key: every input that can change the produced library.
fn lib_store_key(
    bundle_sha: &str,
    dts_sha: &str,
    version: &str,
    fingerprint: &str,
    rt_abi_hash: &str,
    triple: &str,
) -> String {
    // The trailing empty line is the (M2: always empty) sorted dependency-key list.
    let text = format!(
        "thaw-lib-v1\n{bundle_sha}\n{dts_sha}\n{version}\n{fingerprint}\n{rt_abi_hash}\nvariant=core;engine=none\n{triple}\nO2\n\n"
    );
    sha256_hex(text.as_bytes())[..32].to_string()
}

fn lib_header(ident: &str, exports: &[(String, usize)]) -> String {
    let guard = format!("THAW_PKG_{}_H", ident.to_ascii_uppercase());
    let mut out = format!(
        "#ifndef {guard}\n#define {guard}\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n"
    );
    for (symbol, params) in exports {
        let list = if *params == 0 { "void".to_string() } else { vec!["double"; *params].join(", ") };
        out.push_str(&format!("double {symbol}({list});\n"));
    }
    out.push_str("#ifdef __cplusplus\n}\n#endif\n#endif\n");
    out
}

fn lib_default_store() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var_os("THAW_LIB_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME").ok_or("neither THAW_LIB_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".thaw").join("lib"))
}

fn run_lib(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(LIB_USAGE.into());
    };
    if matches!(command, "--help" | "-h" | "help") {
        println!("{LIB_USAGE}");
        return Ok(());
    }
    let mut positional = Vec::new();
    let mut registry = PathBuf::from("thaw_modules");
    let mut store: Option<PathBuf> = None;
    let mut install = true;
    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--registry" => registry = iter.next().ok_or("--registry needs a directory")?.into(),
            "--store" => store = Some(iter.next().ok_or("--store needs a directory")?.into()),
            "--no-install" => install = false,
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`\n{LIB_USAGE}")),
            other => positional.push(other.to_string()),
        }
    }
    let store = match store {
        Some(store) => store,
        None => lib_default_store()?,
    };
    match (command, positional.as_slice()) {
        ("build", [package]) => {
            let (key, built) = lib_build(package, &registry, &store, install)?;
            println!("{} {key}", if built { "built" } else { "up-to-date" });
            Ok(())
        }
        ("list", []) => lib_list(&store),
        ("verify", []) => lib_verify(&store, None),
        ("verify", [key]) => lib_verify(&store, Some(key)),
        _ => Err(LIB_USAGE.into()),
    }
}

struct LibPackage {
    dts: String,
    version: String,
    bundle_sha: String,
}

fn read_lib_package(registry: &Path, package: &str) -> Result<LibPackage, String> {
    let dir = registry.join(package);
    let read = |name: &str| {
        std::fs::read(dir.join(name)).map_err(|e| format!("failed to read `{}`: {e}", dir.join(name).display()))
    };
    let dts = String::from_utf8_lossy(&read("package.d.ts")?).into_owned();
    let version = std::fs::read_to_string(dir.join("version.txt"))
        .map(|text| text.trim().to_string())
        .map_err(|e| format!("failed to read `{}`: {e}", dir.join("version.txt").display()))?;
    let bundle = if dir.join("bundle.js").is_file() {
        read("bundle.js")?
    } else {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&read("bundle.js.gz")?[..]), &mut bytes)
            .map_err(|e| format!("failed to decompress bundle.js.gz: {e}"))?;
        bytes
    };
    Ok(LibPackage { dts, version, bundle_sha: sha256_hex(&bundle) })
}

/// Exportable scalar functions, and `(name, reason)` for the rest.
type LibClassification = (Vec<thaw_bridge::DtsFunction>, Vec<(String, String)>);

/// Splits the `.d.ts` functions into exportable scalars and `(name, reason)` rejections.
fn classify_lib_exports(
    dts: &str,
) -> Result<LibClassification, String> {
    let functions = thaw_bridge::parse_dts(dts)?;
    let mut exported: Vec<thaw_bridge::DtsFunction> = Vec::new();
    let mut rejected: Vec<(String, String)> = Vec::new();
    let number = |ty: &thaw_bridge::DtsType| {
        matches!(ty, thaw_bridge::DtsType::Native(thaw_hir::HirType::F64))
    };
    for function in functions {
        let reason = if functions_named(&exported, &rejected, &function.name) {
            Some("overloaded".to_string())
        } else if function.generic.is_some() {
            Some("generic".to_string())
        } else if function.rest_param.is_some() {
            Some("rest parameter".to_string())
        } else if function.required_params != function.params.len() {
            Some("optional parameter".to_string())
        } else if let Some((name, _)) = function.params.iter().find(|(_, ty)| !number(ty)) {
            Some(format!("parameter `{name}` is not `number`"))
        } else if !number(&function.ret) {
            Some("return type is not `number`".to_string())
        } else {
            None
        };
        match reason {
            None => exported.push(function),
            Some(reason) => {
                // An overload also withdraws the first declaration of the same name.
                if reason == "overloaded" {
                    exported.retain(|f| f.name != function.name);
                }
                rejected.push((function.name.clone(), reason));
            }
        }
    }
    Ok((exported, rejected))
}

fn functions_named(
    exported: &[thaw_bridge::DtsFunction],
    rejected: &[(String, String)],
    name: &str,
) -> bool {
    exported.iter().any(|f| f.name == name) || rejected.iter().any(|(n, _)| n == name)
}

/// Returns the store key and whether this call built it (`false`: it was already there).
fn lib_build(
    package: &str,
    registry: &Path,
    store: &Path,
    install: bool,
) -> Result<(String, bool), String> {
    let ident = package_ident(package);
    let info = read_lib_package(registry, package)?;
    let (exported, mut rejected) = classify_lib_exports(&info.dts)?;
    rejected.sort();
    rejected.dedup();
    if exported.is_empty() {
        return Err(format!("no exportable functions in `{package}`"));
    }

    // The runtime: built once (cached next to the compiler), copied into the store.
    let rt_cached = build_rt_shared_library()?;
    let rt_hash = rt_cached
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or("runtime library has no ABI hash directory")?
        .to_string();
    let rt_dir = store.join("rt").join(&rt_hash);
    let rt_lib = rt_dir.join("libthaw_rt.so.1");
    if !rt_lib.is_file() {
        lib_private_dir(&rt_dir)?;
        let tmp = rt_dir.join(format!("libthaw_rt.so.1.tmp-{}-{}", std::process::id(), unique_suffix()));
        std::fs::copy(&rt_cached, &tmp).map_err(|e| format!("failed to copy the runtime: {e}"))?;
        lib_set_mode(&tmp, 0o755)?;
        std::fs::rename(&tmp, &rt_lib).map_err(|e| format!("failed to install the runtime: {e}"))?;
    }

    let triple = inkwell::targets::TargetMachine::get_default_triple().to_string();
    let key = lib_store_key(&info.bundle_sha, &sha256_hex(info.dts.as_bytes()), &info.version, runtime_fingerprint(), &rt_hash, &triple);
    let final_dir = store.join("store").join(&key);
    let soname = format!("libthaw_pkg_{ident}.so");
    if final_dir.join("thaw.json").is_file() && lib_verify_one(&final_dir).is_ok() {
        return Ok((key, false));
    }

    let mut exports = Vec::new();
    let mut manifest_exports = Vec::new();
    let mut header_exports = Vec::new();
    let mut source = String::new();
    for (index, function) in exported.iter().enumerate() {
        let c_symbol = format!("thaw_pkg_{ident}_{}", package_ident(&function.name));
        let params: Vec<String> = (0..function.params.len()).map(|i| format!("a{i}")).collect();
        let typed: Vec<String> = params.iter().map(|p| format!("{p}: number")).collect();
        source.push_str(&if function.name == "default" {
            format!("import __e{index} from \"{package}\";\n")
        } else {
            format!("import {{ {} as __e{index} }} from \"{package}\";\n", function.name)
        });
        source.push_str(&format!(
            "export function {c_symbol}({}): number {{ return __e{index}({}); }}\n",
            typed.join(", "),
            params.join(", ")
        ));
        let signature = function
            .params
            .iter()
            .map(|(name, _)| format!("{name}: number"))
            .collect::<Vec<_>>()
            .join(", ");
        manifest_exports.push(serde_json::json!({
            "name": function.name,
            "c_symbol": c_symbol,
            "ts_signature": format!("({signature}) => number"),
            "abi_class": "scalar",
        }));
        header_exports.push((c_symbol.clone(), function.params.len()));
        exports.push(LibraryExport { hir_name: c_symbol.clone(), c_symbol });
    }
    let manifest = sorted_json(serde_json::json!({
        "schema": 1,
        "name": package,
        "version": info.version,
        "key": key,
        "tier": "L1",
        "engine": "none",
        "rt": { "variant": "core", "abi_hash": rt_hash },
        "target": triple,
        "soname": soname,
        "deps": [],
        "exports": manifest_exports,
        "not_exported": rejected.iter().map(|(name, reason)| serde_json::json!({"name": name, "reason": reason})).collect::<Vec<_>>(),
    }));
    let mode = LibBuildMode {
        exports,
        metadata: serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?.into_bytes(),
        soname: soname.clone(),
        rt_dir,
        rpath: format!("$ORIGIN/../../rt/{rt_hash}"),
    };

    lib_private_dir(store)?;
    let scratch = BuildScratch::new(&store.join("build"))?;
    let entry = scratch.path.join("lib_entry.ts");
    std::fs::write(&entry, &source).map_err(|e| format!("failed to write the entry: {e}"))?;
    // Compile against a copy of the package that declares only the exported scalars, so the
    // registry shims of the other (non-exported) functions cannot pull a JS engine into the library.
    let scratch_registry = scratch.path.join("registry");
    let scratch_package = scratch_registry.join(package);
    std::fs::create_dir_all(&scratch_package).map_err(|e| e.to_string())?;
    let mut declarations = String::new();
    for function in &exported {
        let params = function
            .params
            .iter()
            .map(|(name, _)| format!("{name}: number"))
            .collect::<Vec<_>>()
            .join(", ");
        declarations.push_str(&if function.name == "default" {
            format!("export default function __default({params}): number;\n")
        } else {
            format!("export declare function {}({params}): number;\n", function.name)
        });
    }
    std::fs::write(scratch_package.join("package.d.ts"), declarations).map_err(|e| e.to_string())?;
    for file in ["bundle.js", "bundle.js.gz", "version.txt"] {
        let source = registry.join(package).join(file);
        if source.is_file() {
            std::fs::copy(&source, scratch_package.join(file)).map_err(|e| e.to_string())?;
        }
    }
    let built = scratch.path.join(&soname);
    build_with_thaw_js_routing_shared(
        &entry, &built, &[], &[], &[], &scratch_registry, &[], false, None, true, install, false, false,
        Some(&mode), false,
    )?;

    let sha = sha256_hex(&std::fs::read(&built).map_err(|e| e.to_string())?);
    let mut full = manifest;
    full["sha256"] = serde_json::json!(sha);
    let full = sorted_json(full);

    lib_private_dir(&store.join("store"))?;
    let tmp_dir = store.join("store").join(format!("{key}.tmp-{}-{}", std::process::id(), unique_suffix()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    lib_private_dir(&tmp_dir)?;
    let write = |name: &str, bytes: &[u8], mode: u32| -> Result<(), String> {
        let path = tmp_dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("failed to write `{}`: {e}", path.display()))?;
        lib_set_mode(&path, mode)
    };
    write(&soname, &std::fs::read(&built).map_err(|e| e.to_string())?, 0o755)?;
    write("thaw.json", (serde_json::to_string_pretty(&full).map_err(|e| e.to_string())? + "\n").as_bytes(), 0o600)?;
    write(&format!("{ident}.h"), lib_header(&ident, &header_exports).as_bytes(), 0o600)?;
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir).map_err(|e| format!("failed to replace `{}`: {e}", final_dir.display()))?;
    }
    std::fs::rename(&tmp_dir, &final_dir).map_err(|e| format!("failed to install `{}`: {e}", final_dir.display()))?;
    Ok((key, true))
}

#[cfg(unix)]
fn lib_set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("failed to set permissions on `{}`: {e}", path.display()))
}

#[cfg(not(unix))]
fn lib_set_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn lib_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("failed to create `{}`: {e}", path.display()))?;
    lib_set_mode(path, 0o700)
}

/// Links `obj` into the shared library `output` against the store's `libthaw_rt.so.1`.
fn link_library(obj: &Path, output: &Path, mode: &LibBuildMode) -> Result<(), String> {
    let map = output.with_extension("map");
    let globals: Vec<&str> = mode.exports.iter().map(|e| e.c_symbol.as_str()).collect();
    std::fs::write(
        &map,
        format!("{{\n  global: {}; __thaw_pkg_metadata;\n  local: *;\n}};\n", globals.join("; ")),
    )
    .map_err(|e| format!("failed to write the version script: {e}"))?;
    let result = Command::new("cc")
        .env("SOURCE_DATE_EPOCH", "0")
        .args(["-shared", "-fPIC"])
        .arg(obj)
        .arg(format!("-L{}", mode.rt_dir.display()))
        .arg("-l:libthaw_rt.so.1")
        .arg(format!("-Wl,-rpath,{}", mode.rpath))
        .args(["-lm", "-ldl"])
        .arg(format!("-Wl,-soname,{}", mode.soname))
        .arg(format!("-Wl,--version-script={}", map.display()))
        .args([
            "-Wl,-Bsymbolic",
            "-Wl,-z,defs",
            "-Wl,--build-id=none",
            "-Wl,--gc-sections",
            "-Wl,--strip-all",
        ])
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("failed to invoke `cc`: {e}"))?;
    let _ = std::fs::remove_file(&map);
    if !result.status.success() {
        return Err(format!("linking the library failed:\n{}", String::from_utf8_lossy(&result.stderr)));
    }
    Ok(())
}

fn lib_entries(store: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(store.join("store"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("thaw.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn lib_list(store: &Path) -> Result<(), String> {
    for dir in lib_entries(store) {
        let text = std::fs::read_to_string(dir.join("thaw.json")).map_err(|e| e.to_string())?;
        let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        println!(
            "{} {}@{} {}",
            manifest["key"].as_str().unwrap_or("?"),
            manifest["name"].as_str().unwrap_or("?"),
            manifest["version"].as_str().unwrap_or("?"),
            manifest["soname"].as_str().unwrap_or("?"),
        );
    }
    Ok(())
}

/// Checks one store entry: private permissions and the recorded sha256 of the library.
fn lib_verify_one(dir: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(dir.join("thaw.json"))
        .map_err(|e| format!("failed to read thaw.json: {e}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("bad thaw.json: {e}"))?;
    let soname = manifest["soname"].as_str().ok_or("thaw.json has no soname")?;
    let library = dir.join(soname);
    for path in [dir, library.as_path(), &dir.join("thaw.json")] {
        if !cache_file_is_private(path) {
            return Err(format!("`{}` is writable by others or owned by another user", path.display()));
        }
    }
    let bytes = std::fs::read(&library).map_err(|e| format!("failed to read `{}`: {e}", library.display()))?;
    let expected = manifest["sha256"].as_str().ok_or("thaw.json has no sha256")?;
    if sha256_hex(&bytes) != expected {
        return Err(format!("`{}` does not match its recorded sha256", library.display()));
    }
    Ok(())
}

fn lib_verify(store: &Path, key: Option<&str>) -> Result<(), String> {
    let dirs = match key {
        Some(key) => vec![store.join("store").join(key)],
        None => lib_entries(store),
    };
    let mut failed = 0;
    for dir in &dirs {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match lib_verify_one(dir) {
            Ok(()) => println!("ok {name}"),
            Err(error) => {
                eprintln!("FAILED {name}: {error}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} library(ies) failed verification"));
    }
    Ok(())
}
