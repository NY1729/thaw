use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use thaw_llvm::HirCompiler;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("build") => {
            if let Err(err) = run_build(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("registry") => {
            if let Err(err) = run_registry(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: thaw build <input.ts> [-o <output>] [--link <path>]... [--bridge <path.d.ts>]... [--registry <dir>] [--use <package>]...\n       thaw registry add <package> [--registry <dir>]"
            );
            std::process::exit(1);
        }
    }
}

fn run_registry(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => run_registry_add(&args[1..]),
        _ => Err("usage: thaw registry add <package> [--registry <dir>]".to_string()),
    }
}

/// The "automatic" half of the registry story (docs/design/registry.md
/// section 6): fetches a real npm package and drops it into the local
/// registry directory in the layout `thaw build --use` expects, so a
/// package name is all a user needs -- no hand-copying `package.d.ts`/
/// `bundle.js` themselves.
fn run_registry_add(args: &[String]) -> Result<(), String> {
    let mut package: Option<String> = None;
    let mut registry_dir = PathBuf::from("thaw_modules");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--registry" => {
                i += 1;
                let value = args.get(i).ok_or("--registry requires a path argument")?;
                registry_dir = PathBuf::from(value);
            }
            other => {
                if package.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                package = Some(other.to_string());
            }
        }
        i += 1;
    }

    let package = package.ok_or("missing package name (usage: thaw registry add <package>)")?;

    println!("fetching `{package}`...");
    let added = thaw_registry::add(&registry_dir, &package)?;
    println!(
        "added `{package}` to `{}`\n  types: {}\n  main:  {}",
        registry_dir.join(&package).display(),
        added.dts_relative_path,
        added.js_relative_path
    );
    Ok(())
}

fn run_build(args: &[String]) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut extra_links: Vec<PathBuf> = Vec::new();
    let mut bridge_dts: Vec<PathBuf> = Vec::new();
    let mut registry_dir = PathBuf::from("thaw_modules");
    let mut use_packages: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                let value = args.get(i).ok_or("-o requires a path argument")?;
                output = Some(PathBuf::from(value));
            }
            "--link" => {
                i += 1;
                let value = args.get(i).ok_or("--link requires a path argument")?;
                extra_links.push(PathBuf::from(value));
            }
            "--bridge" => {
                i += 1;
                let value = args.get(i).ok_or("--bridge requires a path argument")?;
                bridge_dts.push(PathBuf::from(value));
            }
            "--registry" => {
                i += 1;
                let value = args.get(i).ok_or("--registry requires a path argument")?;
                registry_dir = PathBuf::from(value);
            }
            "--use" => {
                i += 1;
                let value = args.get(i).ok_or("--use requires a package name argument")?;
                use_packages.push(value.clone());
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    let input = input.ok_or("missing input file (usage: thaw build <input.ts> [-o <output>])")?;
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        PathBuf::from(stem)
    });

    build(
        &input,
        &output,
        &extra_links,
        &bridge_dts,
        &registry_dir,
        &use_packages,
    )
}

/// Reads each `.d.ts` in `bridge_dts`, classifies its functions (thaw-bridge,
/// docs/design/bridge.md sections 3-4), and generates the corresponding
/// callable surface (section 6: an ambient `declare function` per fast-path
/// function; section 7: a `callDynamic` wrapper per fallback function) --
/// concatenated ahead of the user's own source. Thaw has no real module/
/// import system yet, so "prepend the generated text" is the whole
/// integration; there's nothing to separately compile or link at this step
/// (fast-path symbols still need `--link`, fallback functions still need
/// `loadScript` called with the package's actual JS source, per
/// `generate_shim`'s doc comment).
fn generate_bridge_shims(bridge_dts: &[PathBuf]) -> Result<String, String> {
    let mut shim = String::new();
    for path in bridge_dts {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;
        let functions = thaw_bridge::parse_dts(&source)
            .map_err(|e| format!("failed to parse `{}`: {e}", path.display()))?;
        shim.push_str(&thaw_bridge::generate_shim(&functions));
    }
    Ok(shim)
}

/// Resolves each `--use`d package against the local registry (thaw-registry;
/// `registry_dir` defaults to `thaw_modules/`), generating its callable
/// surface exactly like `generate_bridge_shims` does for a standalone
/// `.d.ts` -- but additionally auto-linking the package's `native.a` if it
/// ships one (replacing a manual `--link`), and collecting its `bundle.js`
/// (if any) into a single generated `__thaw_module_init` (thaw-bridge's
/// `generate_module_init`) so it's auto-loaded before user code runs
/// (replacing a manual `loadScript` call). Returns the generated shim text
/// and the native lib paths to link.
fn generate_registry_shims(
    registry_dir: &Path,
    use_packages: &[String],
) -> Result<(String, Vec<PathBuf>), String> {
    let mut shim = String::new();
    let mut native_libs = Vec::new();
    // (package_name, js_source, fallback_function_names) -- kept as owned
    // data so the borrowed `ModuleBundle`s built from it below can outlive
    // this loop.
    let mut bundles: Vec<(String, String, Vec<String>)> = Vec::new();

    for name in use_packages {
        let package = thaw_registry::resolve(registry_dir, name)?;
        let functions = thaw_bridge::parse_dts(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts: {e}"))?;
        shim.push_str(&thaw_bridge::generate_shim(&functions));

        if let Some(native_lib) = package.native_lib {
            native_libs.push(native_lib);
        }
        if let Some(bundle_js) = package.bundle_js {
            // Only Fallback functions need binding inside the loaded
            // script (see `ModuleBundle::fallback_names`'s doc comment);
            // FastPath functions are real FFI calls and never touch
            // QuickJS-NG at all. `classify_all` (not per-function
            // `classify`) so an overloaded name that's a mix of FastPath/
            // Fallback signatures is counted once, consistently with
            // what `generate_shim` actually emitted for it.
            let fallback_names = thaw_bridge::classify_all(&functions)
                .into_iter()
                .filter_map(|(name, classification)| match classification {
                    thaw_bridge::Classification::Fallback { .. } => Some(name),
                    thaw_bridge::Classification::FastPath(_) => None,
                })
                .collect();
            bundles.push((package.name.clone(), bundle_js, fallback_names));
        }
    }

    let module_bundles: Vec<thaw_bridge::ModuleBundle> = bundles
        .iter()
        .map(|(name, js, fallback_names)| thaw_bridge::ModuleBundle {
            package_name: name.as_str(),
            js_source: js.as_str(),
            fallback_names,
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_module_init(&module_bundles));

    Ok((shim, native_libs))
}

fn build(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
) -> Result<(), String> {
    let user_source = std::fs::read_to_string(input)
        .map_err(|e| format!("failed to read `{}`: {e}", input.display()))?;
    let (registry_shim, registry_native_libs) =
        generate_registry_shims(registry_dir, use_packages)?;
    let source = registry_shim + &generate_bridge_shims(bridge_dts)? + &user_source;

    let module = thaw_parser::parse_typescript(&source)?;
    let program = thaw_hir::lower_module(&module)?;

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, input.to_string_lossy().as_ref());
    compiler.compile_program(&program)?;

    let obj_path = output.with_extension("o");
    compiler.write_object_file(&obj_path)?;

    // Always link all four: thaw-arena backs array/object allocation
    // (Phase 1/2), thaw-runtime backs the Lambda event loop for
    // `handler`-based programs (Phase 2), thaw-std backs `fetch`/`JSON.*`,
    // thaw-quickjs backs `loadScript`/`callDynamic` (the QuickJS-NG
    // fallback path, docs/design/bridge.md section 7). An unreferenced
    // static archive member is simply never pulled into the final binary,
    // so linking all of them unconditionally is harmless and keeps this
    // simple.
    let arena_lib = build_staticlib("thaw-arena")?;
    let runtime_lib = build_staticlib("thaw-runtime")?;
    let std_lib = build_staticlib("thaw-std")?;
    let quickjs_lib = build_staticlib("thaw-quickjs")?;

    // `--link <path>` lets a program using `declare function` (see
    // docs/design/bridge.md section 6) actually resolve at link time,
    // until thaw-registry can fetch/build that library automatically.
    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&quickjs_lib)
        // QuickJS-NG's C code calls libm math functions directly; `rustc`
        // normally adds `-lm` automatically when it does the final link,
        // but this is a manual `cc` invocation instead.
        .arg("-lm")
        .args(&registry_native_libs)
        .args(extra_links)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|e| format!("failed to invoke system `cc` linker: {e}"))?;

    let _ = std::fs::remove_file(&obj_path);

    if !link_status.success() {
        return Err("linking failed".to_string());
    }

    println!("built `{}`", output.display());
    Ok(())
}

/// Builds `pkg` as a staticlib (thaw-arena, thaw-runtime, ...) and returns
/// the path to the resulting `.a` file, parsed out of `cargo build`'s JSON
/// artifact output. That's robust to `CARGO_TARGET_DIR` overrides, unlike
/// guessing a relative path.
fn build_staticlib(pkg: &str) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["build", "--release", "-p", pkg, "--message-format=json"])
        .output()
        .map_err(|e| format!("failed to invoke `cargo build -p {pkg}`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "building {pkg} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(idx) = line.find("\"filenames\":[\"") {
            let rest = &line[idx + "\"filenames\":[\"".len()..];
            if let Some(end) = rest.find(".a\"") {
                return Ok(PathBuf::from(&rest[..end + 2]));
            }
        }
    }
    Err(format!("could not find a staticlib for `{pkg}` in `cargo build` output"))
}
