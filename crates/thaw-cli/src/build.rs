#[cfg(test)]
fn build(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    ffi_metadata: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
) -> Result<(), String> {
    build_with_link_mode(
        input,
        output,
        extra_links,
        bridge_dts,
        ffi_metadata,
        registry_dir,
        use_packages,
        false,
    )
}

fn registry_import_meta_resolutions(
    registry_dir: &Path,
    packages: &[String],
) -> std::collections::HashMap<String, String> {
    let mut resolutions = std::collections::HashMap::new();
    for specifier in packages {
        if specifier.starts_with("node:") {
            resolutions.insert(specifier.clone(), specifier.clone());
            continue;
        }
        let parts = specifier.split('/').collect::<Vec<_>>();
        let package_parts = usize::from(specifier.starts_with('@')) + 1;
        if parts.len() < package_parts {
            continue;
        }
        let package = parts[..package_parts].join("/");
        let mut directory = registry_dir.join(package);
        if parts.len() > package_parts {
            directory = directory
                .join("subpaths")
                .join(parts[package_parts..].join("/"));
        }
        let artifact = ["native.node", "bundle.js", "native.a"]
            .into_iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file());
        if let Some(artifact) = artifact {
            let artifact = artifact.canonicalize().unwrap_or(artifact);
            resolutions.insert(specifier.clone(), module_graph::file_url(&artifact));
        }
    }
    resolutions
}

fn generate_asset_shim(directory: &Path) -> Result<String, String> {
    if !directory.is_dir() {
        return Err(format!(
            "--assets expects a directory, got `{}`",
            directory.display()
        ));
    }

    fn collect(root: &Path, directory: &Path, files: &mut Vec<(String, String)>) -> Result<(), String> {
        let entries = std::fs::read_dir(directory)
            .map_err(|error| format!("failed to read asset directory `{}`: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read asset entry in `{}`: {error}", directory.display())
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("failed to inspect asset `{}`: {error}", path.display())
            })?;
            if file_type.is_dir() {
                collect(root, &path, files)?;
            } else if file_type.is_file() {
                let relative = path.strip_prefix(root).unwrap();
                let url = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
                let bytes = std::fs::read(&path)
                    .map_err(|error| format!("failed to read asset `{}`: {error}", path.display()))?;
                let content = String::from_utf8(bytes).map_err(|_| {
                    format!(
                        "asset `{}` is not UTF-8; binary asset embedding is not implemented yet",
                        path.display()
                    )
                })?;
                files.push((url, content));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    collect(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!("asset directory `{}` is empty", directory.display()));
    }

    fn mime(path: &str) -> &'static str {
        match Path::new(path).extension().and_then(|extension| extension.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("js" | "mjs") => "text/javascript; charset=utf-8",
            Some("json" | "map") => "application/json; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("txt") => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        }
    }

    let mut routes = Vec::new();
    for (path, content) in files {
        let content_type = mime(&path);
        if path == "/index.html" {
            routes.push(("/".to_string(), content.clone(), content_type));
        }
        routes.push((path, content, content_type));
    }

    let mut source = String::from(
        "function thawHasAsset(path: string): boolean {\n",
    );
    for (path, _, _) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return true; }}\n",
            serde_json::to_string(path).unwrap()
        ));
    }
    source.push_str("return false;\n}\nfunction thawAsset(path: string): string {\n");
    for (path, content, _) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return {}; }}\n",
            serde_json::to_string(path).unwrap(),
            serde_json::to_string(content).unwrap()
        ));
    }
    source.push_str("return \"\";\n}\nfunction thawAssetContentType(path: string): string {\n");
    for (path, _, content_type) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return {}; }}\n",
            serde_json::to_string(path).unwrap(),
            serde_json::to_string(content_type).unwrap()
        ));
    }
    source.push_str("return \"application/octet-stream\";\n}\n");
    Ok(source)
}

// Keep the build inputs explicit: the slices come from separate CLI/registry
// sources and are independently varied by integration tests.
#[allow(clippy::too_many_arguments)]
fn build_with_link_mode(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    ffi_metadata: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
    static_link: bool,
) -> Result<(), String> {
    build_with_assets(
        input,
        output,
        extra_links,
        bridge_dts,
        ffi_metadata,
        registry_dir,
        use_packages,
        static_link,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_with_assets(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    ffi_metadata: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
    static_link: bool,
    assets: Option<&Path>,
) -> Result<(), String> {
    if static_link {
        ensure_static_system_libraries()?;
        for path in extra_links {
            if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("so" | "dylib")
            ) {
                return Err(format!(
                    "--static cannot link shared library `{}`; provide a static archive (`.a`)",
                    path.display()
                ));
            }
        }
    }
    let user_source = std::fs::read_to_string(input)
        .map_err(|e| format!("failed to read `{}`: {e}", input.display()))?;
    let external_specifiers = module_graph::external_specifiers(input, &user_source)?;
    let mut resolved_packages = use_packages.to_vec();
    for (specifier, location) in &external_specifiers {
        let package = if specifier.starts_with("node:") {
            thaw_registry::resolve_builtin(specifier)
                .map_err(|error| format!("{location}: {error}"))?;
            specifier.clone()
        } else {
            let package = specifier.clone();
            thaw_registry::resolve(registry_dir, &package)
                .map_err(|error| format!("{location}: {error}"))?;
            package
        };
        if !resolved_packages.contains(&package) {
            resolved_packages.push(package);
        }
    }
    let (
        registry_shim,
        registry_native_libs,
        qualified_call_rewrites,
        class_constructor_rewrites,
        class_method_rewrites,
        static_class_method_rewrites,
        class_getter_rewrites,
        class_setter_rewrites,
        static_class_getter_rewrites,
        static_class_setter_rewrites,
        factory_class_rewrites,
        fallback_function_overload_rewrites,
        external_exports,
        external_namespace_aliases,
        external_nested_namespaces,
    ) = generate_registry_shims(registry_dir, &resolved_packages, &user_source)?;
    let external_resolutions = registry_import_meta_resolutions(registry_dir, &resolved_packages);
    // `qs.stringify(x)`-style calls, for a name that collided across two
    // `--use`d packages, only exist as source-level syntax sugar over the
    // package-qualified alias `generate_registry_shims` actually
    // generated -- see `rewrite_qualified_calls`'s doc comment. A no-op
    // (and no parse at all) when there were no collisions.
    let mut shim_source = registry_shim + &generate_bridge_shims(bridge_dts)?;
    if let Some(directory) = assets {
        shim_source.push_str(&generate_asset_shim(directory)?);
    }
    if static_link && shim_source.contains("loadNativeAddonEmbedded(") {
        return Err(
            "--static cannot include an N-API addon: `.node` modules require the dynamic loader; use the package's JavaScript fallback or a static `native.a` backend"
                .into(),
        );
    }
    let manifest = serde_json::json!({
        "packages": resolved_packages,
        // Registry fallback wrappers contain `callDynamic(...)` even when a
        // JIT declaration shadows them, so only count the generated shim's
        // module initializer here. User-authored dynamic calls and explicit
        // QuickJS typed symbols are checked separately below.
        "quickjs": shim_source.contains("loadScript(")
            || source_uses_quickjs(&user_source)
            || shim_source.contains("__thaw_typed_js_"),
        "napi": shim_source.contains("loadNativeAddonEmbedded("),
    });
    let marker = format!("{ARTIFACT_MARKER}{manifest}");
    let marker_literal = serde_json::to_string(&marker)
        .map_err(|error| format!("failed to encode artifact metadata: {error}"))?;
    shim_source.push_str(&format!(
        "function __thaw_artifact_metadata(): string {{ return {marker_literal}; }}\n"
    ));
    let transform = |source: &str| {
        let imported_overload_aliases = fallback_function_overload_rewrites
            .iter()
            .map(|rewrite| rewrite.0.clone())
            .collect();
        let source = rewrite_qualified_calls(
            source,
            &qualified_call_rewrites,
            &imported_overload_aliases,
        )?;
        let source = rewrite_external_class_methods_with_static(
            &source,
            &class_constructor_rewrites,
            &class_method_rewrites,
            &static_class_method_rewrites,
            &class_getter_rewrites,
            &class_setter_rewrites,
            &static_class_getter_rewrites,
            &static_class_setter_rewrites,
            &factory_class_rewrites,
            &fallback_function_overload_rewrites,
        )?;
        rewrite_external_class_constructors(&source, &class_constructor_rewrites)
    };
    let mut module = module_graph::bundle_with_source_transform(
        input,
        &user_source,
        &external_exports,
        &external_namespace_aliases,
        &external_nested_namespaces,
        &external_resolutions,
        &transform,
    )?;
    if !shim_source.is_empty() {
        let mut shim = thaw_parser::parse_typescript(&shim_source)?;
        shim.body.extend(module.body);
        module.body = shim.body;
    }
    let mut program = thaw_hir::lower_module(&module)?;
    for (symbol, metadata) in read_ffi_metadata(ffi_metadata)? {
        let param_count = program
            .extern_functions
            .iter()
            .find(|signature| signature.symbol == symbol)
            .map(|signature| signature.params.len())
            .ok_or_else(|| {
                format!("FFI metadata references unknown ambient function `{symbol}`")
            })?;
        thaw_hir::set_ffi_error_abi(&mut program, &symbol, metadata.error_abi)?;
        thaw_hir::set_ffi_ownership(
            &mut program,
            &symbol,
            metadata.return_ownership,
            metadata.error_ownership,
        )?;
        thaw_hir::set_ffi_string_abi(
            &mut program,
            &symbol,
            metadata
                .param_string_abis
                .unwrap_or_else(|| vec![thaw_hir::FfiStringAbi::NullTerminated; param_count]),
            metadata.return_string_abi,
            metadata.calling_convention,
            metadata.aggregate_return_abi,
        )?;
        if let Some(layout) = metadata.aggregate_return_layout {
            thaw_hir::set_ffi_aggregate_layout(&mut program, &symbol, layout)?;
        }
        thaw_hir::set_ffi_variadic_abi(&mut program, &symbol, metadata.variadic_abi)?;
    }

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, input.to_string_lossy().as_ref());
    compiler.compile_program(&program)?;
    let uses_quickjs = compiler.uses_quickjs();
    let uses_napi = compiler.uses_napi();

    let obj_path = output.with_extension("o");
    compiler.write_object_file(&obj_path)?;

    // Always link the runtime support crates: thaw-arena backs array/object allocation
    // (Phase 1/2), thaw-runtime backs the Lambda event loop for
    // `handler`-based programs (Phase 2), thaw-std backs `fetch`/`JSON.*`,
    // thaw-quickjs backs `loadScript`/`callDynamic` (the QuickJS-NG
    // fallback path, docs/design/bridge.md section 7). It is built and
    // linked only when LLVM found a call into that host.
    let arena_lib = build_staticlib("thaw-arena")?;
    let runtime_lib = build_staticlib("thaw-runtime")?;
    let std_lib = build_staticlib("thaw-std")?;
    let jit_lib = build_staticlib("thaw-jit")?;
    let quickjs_lib = uses_quickjs
        .then(|| build_staticlib("thaw-quickjs"))
        .transpose()?;
    let napi_lib = uses_napi.then(|| {
        if uses_quickjs {
            build_staticlib("thaw-napi")
        } else {
            build_staticlib_without_default_features("thaw-napi")
        }
    }).transpose()?;

    // `--link <path>` lets a program using `declare function` (see
    // docs/design/bridge.md section 6) actually resolve at link time,
    // until thaw-registry can fetch/build that library automatically.
    let mut linker = Command::new("cc");
    if static_link {
        linker
            .arg("-static")
            .arg("-no-pie")
            .arg("-Wl,--no-dynamic-linker");
    }
    linker
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&jit_lib);
    if let Some(quickjs_lib) = quickjs_lib {
        linker.arg(quickjs_lib);
    }
    if let Some(napi_lib) = napi_lib {
        linker.arg(napi_lib);
    }
    linker
        // QuickJS-NG's C code calls libm math functions directly; `rustc`
        // normally adds `-lm` automatically when it does the final link,
        // but this is a manual `cc` invocation instead.
        .arg("-lm")
        .arg("-ldl")
        .args(&registry_native_libs)
        .args(extra_links);
    if !static_link {
        linker.arg("-Wl,--export-dynamic");
    }
    let link_output = linker
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("failed to invoke system `cc` linker: {e}"))?;

    let _ = std::fs::remove_file(&obj_path);

    if !link_output.status.success() {
        let stderr = String::from_utf8_lossy(&link_output.stderr);
        let hint = if static_link {
            "\nstatic linking requires the target's static libc/libm/libdl archives (on Fedora, install glibc-static; alternatively use a musl toolchain)"
        } else {
            ""
        };
        return Err(format!("linking failed:\n{stderr}{hint}"));
    }
    if static_link && elf_has_program_interpreter(output)? {
        return Err(format!(
            "static link produced `{}` with a dynamic ELF interpreter",
            output.display()
        ));
    }

    println!("built `{}`", output.display());
    Ok(())
}

/// Returns whether generated or user source explicitly requires the dynamic
/// JavaScript host. QuickJS typed symbols are intentionally included here:
/// unlike ordinary extracted functions, they are direct calls into that host
/// ABI and therefore determine whether the QuickJS archive must be retained.
fn source_uses_quickjs(source: &str) -> bool {
    [
        "loadScript(",
        "callDynamic(",
        "getDynamicValue(",
        "callDynamicValue(",
        "callDynamicValueHandle(",
        "callDynamicValueWithValue(",
        "releaseDynamicValue(",
        "getDynamicProperty(",
        "setDynamicProperty(",
        "callDynamicMethod(",
        "readDynamicValue(",
        "callDynamicValueMixed(",
        "constructDynamicValue(",
        "__thaw_typed_js_",
    ]
    .iter()
    .any(|marker| source.contains(marker))
}

fn ensure_static_system_libraries() -> Result<(), String> {
    let mut missing = Vec::new();
    for library in ["libc.a", "libm.a", "libdl.a"] {
        let output = Command::new("cc")
            .arg(format!("-print-file-name={library}"))
            .output()
            .map_err(|error| format!("failed to inspect the system C toolchain: {error}"))?;
        let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() || resolved == library || !Path::new(&resolved).is_file() {
            missing.push(library);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "--static requires missing system archives: {} (on Fedora, install glibc-static; alternatively use a musl toolchain)",
            missing.join(", ")
        ))
    }
}

fn elf_has_program_interpreter(path: &Path) -> Result<bool, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
    if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" {
        return Err(format!("`{}` is not an ELF executable", path.display()));
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("static output verification currently requires little-endian ELF64".into());
    }
    let read_u16 = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let read_u64 =
        |offset: usize| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    let program_offset = read_u64(32) as usize;
    let entry_size = read_u16(54) as usize;
    let entry_count = read_u16(56) as usize;
    for index in 0..entry_count {
        let offset = program_offset + index * entry_size;
        if offset + 4 > bytes.len() {
            return Err(format!(
                "`{}` has a truncated ELF program table",
                path.display()
            ));
        }
        let kind = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if kind == 3 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Builds `pkg` as a staticlib (thaw-arena, thaw-runtime, ...) and returns
/// the path to the resulting `.a` file, parsed out of `cargo build`'s JSON
/// artifact output. That's robust to `CARGO_TARGET_DIR` overrides, unlike
/// guessing a relative path.
fn build_staticlib(pkg: &str) -> Result<PathBuf, String> {
    build_staticlib_with_options(pkg, false)
}

/// Builds a package without its default features. This is used for the N-API
/// host when the generated program has no QuickJS call sites, so the host's
/// optional QuickJS bridge is not compiled into the final archive.
fn build_staticlib_without_default_features(pkg: &str) -> Result<PathBuf, String> {
    build_staticlib_with_options(pkg, true)
}

fn build_staticlib_with_options(pkg: &str, no_default_features: bool) -> Result<PathBuf, String> {
    let mut command = Command::new("cargo");
    command
        .args(["build", "--release", "-p", pkg, "--message-format=json"])
        .arg("--manifest-path")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"));
    if no_default_features {
        command.arg("--no-default-features");
        // Keep feature variants in separate target directories. CLI tests build
        // several artifacts concurrently; sharing target/release would let a
        // QuickJS-enabled and a QuickJS-free static archive replace each other.
        command
            .arg("--target-dir")
            .arg(no_quickjs_target_dir());
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to invoke `cargo build -p {pkg}`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "building {pkg} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let target_name = pkg.replace('-', "_");
    for line in stdout.lines() {
        if !line.contains(&format!("\"name\":\"{target_name}\"")) {
            continue;
        }
        if let Some(idx) = line.find("\"filenames\":[\"") {
            let rest = &line[idx + "\"filenames\":[\"".len()..];
            if let Some(end) = rest.find(".a\"") {
                return Ok(PathBuf::from(&rest[..end + 2]));
            }
        }
    }
    Err(format!(
        "could not find a staticlib for `{pkg}` in `cargo build` output"
    ))
}

fn no_quickjs_target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("thaw-no-quickjs")
}
