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

fn external_package_name(specifier: &str) -> Option<String> {
    let parts = specifier.split('/').collect::<Vec<_>>();
    let package_parts = usize::from(specifier.starts_with('@')) + 1;
    Some(parts.get(..package_parts)?.join("/"))
}

fn installed_node_modules(input: &Path, package: &str) -> Option<PathBuf> {
    input.parent()?.ancestors().find_map(|directory| {
        let node_modules = directory.join("node_modules");
        node_modules
            .join(package)
            .join("package.json")
            .is_file()
            .then_some(node_modules)
    })
}

fn nearest_package_directory(input: &Path) -> Option<&Path> {
    input
        .parent()?
        .ancestors()
        .find(|directory| directory.join("package.json").is_file())
}

fn generate_asset_shim(directory: &Path) -> Result<String, String> {
    if !directory.is_dir() {
        return Err(format!(
            "--assets expects a directory, got `{}`",
            directory.display()
        ));
    }

    fn collect(
        root: &Path,
        directory: &Path,
        files: &mut Vec<(String, String, &'static str)>,
    ) -> Result<(), String> {
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
                let (content, encoding) = match String::from_utf8(bytes.clone()) {
                    Ok(content) => (content, "utf8"),
                    Err(_) => (
                        bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
                        "hex",
                    ),
                };
                files.push((url, content, encoding));
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
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("ico") => "image/x-icon",
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("txt") => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        }
    }

    let mut routes = Vec::new();
    for (path, content, encoding) in files {
        let content_type = mime(&path);
        if path == "/index.html" {
            routes.push(("/".to_string(), content.clone(), content_type, encoding));
        }
        routes.push((path, content, content_type, encoding));
    }

    let mut source = String::from(
        "function thawAssetPath(path: string): string {\nreturn path.split(\"?\")[0].split(\"#\")[0];\n}\nfunction thawHasAsset(path: string): boolean {\npath = thawAssetPath(path);\n",
    );
    for (path, _, _, _) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return true; }}\n",
            serde_json::to_string(path).unwrap()
        ));
    }
    source.push_str("return false;\n}\nfunction thawAsset(path: string): string {\npath = thawAssetPath(path);\n");
    for (path, content, _, _) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return {}; }}\n",
            serde_json::to_string(path).unwrap(),
            serde_json::to_string(content).unwrap()
        ));
    }
    source.push_str("return \"\";\n}\nfunction thawAssetContentType(path: string): string {\npath = thawAssetPath(path);\n");
    for (path, _, content_type, _) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return {}; }}\n",
            serde_json::to_string(path).unwrap(),
            serde_json::to_string(content_type).unwrap()
        ));
    }
    source.push_str("return \"application/octet-stream\";\n}\nfunction thawAssetEncoding(path: string): string {\npath = thawAssetPath(path);\n");
    for (path, _, _, encoding) in &routes {
        source.push_str(&format!(
            "if (path === {}) {{ return {}; }}\n",
            serde_json::to_string(path).unwrap(),
            serde_json::to_string(encoding).unwrap()
        ));
    }
    source.push_str("return \"utf8\";\n}\n");
    source.push_str(
        "function thawAssetRoute(path: string): string {\npath = thawAssetPath(path);\nif (thawHasAsset(path)) { return path; }\nif (path.indexOf(\".\") < 0 && thawHasAsset(\"/\")) { return \"/\"; }\nreturn path;\n}\nfunction thawServeAsset(response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (body: string) => boolean; write: (body: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }, path: string): boolean {\npath = thawAssetRoute(path);\nif (!thawHasAsset(path)) { return false; }\nresponse.setHeader(\"Content-Type\", thawAssetContentType(path));\nreturn response.endEncoded(thawAsset(path), thawAssetEncoding(path));\n}\n",
    );
    Ok(source)
}

// Keep the build inputs explicit: the slices come from separate CLI/registry
// sources and are independently varied by integration tests.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
#[cfg(test)]
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
    build_with_native_mode(
        input,
        output,
        extra_links,
        bridge_dts,
        ffi_metadata,
        registry_dir,
        use_packages,
        static_link,
        assets,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_with_native_mode(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    ffi_metadata: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
    static_link: bool,
    assets: Option<&Path>,
    embed_native_addons: bool,
) -> Result<(), String> {
    if static_link && !embed_native_addons {
        return Err("--static and --external-native cannot be used together".into());
    }
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
            if thaw_registry::resolve(registry_dir, &package).is_err() {
                if let Some(name) = external_package_name(&package) {
                    if let Some(node_modules) = installed_node_modules(input, &name) {
                        if thaw_registry::resolve(registry_dir, &name).is_err() {
                            thaw_registry::add_installed_root(
                                registry_dir,
                                &node_modules,
                                &name,
                            )
                            .map_err(|error| {
                                format!(
                                    "{location}: failed to register installed `{name}`: {error}"
                                )
                            })?;
                        }
                        if package != name
                            && thaw_registry::resolve(registry_dir, &package).is_err()
                        {
                            thaw_registry::add_installed_subpath(
                                registry_dir,
                                &node_modules,
                                &package,
                            )
                            .map_err(|error| {
                                format!(
                                    "{location}: failed to register installed `{package}`: {error}"
                                )
                            })?;
                        }
                    }
                }
            }
            thaw_registry::resolve(registry_dir, &package).map_err(|error| {
                let help = nearest_package_directory(input)
                    .map(|directory| format!("\nhelp: run `thaw install {}`", directory.display()))
                    .unwrap_or_default();
                format!("{location}: {error}{help}")
            })?;
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
    ) = generate_registry_shims(
        registry_dir,
        &resolved_packages,
        &user_source,
        embed_native_addons,
    )?;
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
    if static_link
        && (shim_source.contains("loadNativeAddonEmbedded(")
            || shim_source.contains("loadNativeAddon("))
    {
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
        "napi": shim_source.contains("loadNativeAddonEmbedded(")
            || shim_source.contains("loadNativeAddon("),
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
    let uses_wasm = source_uses_wasm(&shim_source) || source_uses_wasm(&user_source);
    let uses_tls = source_uses_tls(&shim_source) || source_uses_tls(&user_source);
    let uses_brotli = source_uses_brotli(&shim_source) || source_uses_brotli(&user_source);
    // A QuickJS-enabled `thaw-napi` staticlib already contains its Rust
    // dependency objects. Linking a second standalone QuickJS archive would
    // define every host symbol twice.
    let quickjs_lib = (uses_quickjs && !uses_napi)
        .then(|| {
            let mut features = Vec::new();
            if uses_brotli {
                features.push("brotli");
            }
            if uses_tls {
                features.push("tls");
            }
            if uses_wasm {
                features.push("wasm");
            }
            if features.len() == 3 {
                build_staticlib("thaw-quickjs")
            } else {
                build_staticlib_with_features("thaw-quickjs", &features)
            }
        })
        .transpose()?;
    let napi_lib = uses_napi
        .then(|| {
            if !uses_quickjs {
                return build_staticlib_without_default_features("thaw-napi");
            }
            let mut features = vec!["quickjs"];
            if uses_brotli {
                features.push("quickjs-brotli");
            }
            if uses_tls {
                features.push("quickjs-tls");
            }
            if uses_wasm {
                features.push("quickjs-wasm");
            }
            build_staticlib_with_features("thaw-napi", &features)
        })
        .transpose()?;

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
        // Every support crate is a static archive. Keep only the runtime
        // sections reached by this particular program, then remove the
        // remaining symbol/debug tables from the final executable. This is
        // especially important for the optional QuickJS/N-API hosts, whose
        // unrelated feature implementations otherwise survive the link.
        .arg("-Wl,--gc-sections")
        .args(&registry_native_libs)
        .args(extra_links);
    if !static_link {
        linker.arg("-Wl,--export-dynamic");
    }
    let link_output = linker
        .arg("-Wl,--strip-all")
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

fn source_uses_wasm(source: &str) -> bool {
    ["WebAssembly", "node:wasi", "require('wasi')", "require(\"wasi\")"]
        .iter()
        .any(|marker| source.contains(marker))
}

fn source_uses_tls(source: &str) -> bool {
    source.contains("__thaw_tls_")
}

fn source_uses_brotli(source: &str) -> bool {
    source.contains("brotli") || source.contains("Brotli")
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
    build_staticlib_with_options(pkg, false, &[])
}

/// Builds a package without its default features. This is used for the N-API
/// host when the generated program has no QuickJS call sites, so the host's
/// optional QuickJS bridge is not compiled into the final archive.
fn build_staticlib_without_default_features(pkg: &str) -> Result<PathBuf, String> {
    build_staticlib_with_options(pkg, true, &[])
}

fn build_staticlib_with_features(pkg: &str, features: &[&str]) -> Result<PathBuf, String> {
    build_staticlib_with_options(pkg, true, features)
}

fn build_staticlib_with_options(
    pkg: &str,
    no_default_features: bool,
    features: &[&str],
) -> Result<PathBuf, String> {
    let mut command = Command::new("cargo");
    command
        .args(["build", "--release", "-p", pkg, "--message-format=json"])
        .arg("--manifest-path")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"));
    if no_default_features {
        command.arg("--no-default-features");
        if !features.is_empty() {
            command.args(["--features", &features.join(",")]);
        }
        // Keep feature variants in separate target directories. CLI tests build
        // several artifacts concurrently; sharing target/release would let a
        // QuickJS-enabled and a QuickJS-free static archive replace each other.
        command
            .arg("--target-dir")
            .arg(feature_target_dir(pkg, features));
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

fn feature_target_dir(pkg: &str, features: &[&str]) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join(format!(
            "thaw-{pkg}-{}",
            if features.is_empty() {
                "minimal".to_string()
            } else {
                features.join("-")
            }
        ))
}
