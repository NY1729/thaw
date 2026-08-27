use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use thaw_llvm::HirCompiler;

mod module_graph;

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
        Some("inspect") => {
            if let Err(err) = run_inspect(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: thaw build <input.ts> [-o <output>] [--static] [--link <path>]... [--bridge <path.d.ts>]... [--ffi-metadata <path.json>]... [--registry <dir>] [--use <package>]...\n       thaw inspect <executable>\n       thaw registry add <package>[@<version>] [--registry <dir>]"
            );
            std::process::exit(1);
        }
    }
}

const ARTIFACT_MARKER: &str = "THAW_ARTIFACT_V1:";

fn run_inspect(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: thaw inspect <executable>".into());
    }
    let path = Path::new(&args[0]);
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    if bytes.len() < 20 || &bytes[..4] != b"\x7fELF" {
        return Err(format!("`{}` is not an ELF executable", path.display()));
    }
    let architecture = match u16::from_le_bytes([bytes[18], bytes[19]]) {
        0x3e => "x86_64",
        0xb7 => "aarch64",
        _ => "unknown",
    };
    let linkage = if elf_has_program_interpreter(path)? {
        "dynamic-system"
    } else {
        "fully-static"
    };
    let manifest = artifact_manifest_from_bytes(&bytes)?;
    println!("file: {}", path.display());
    println!("format: ELF64");
    println!("architecture: {architecture}");
    println!("linkage: {linkage}");
    println!(
        "packages: {}",
        manifest["packages"]
            .as_array()
            .map(|packages| packages
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );
    println!(
        "quickjs: {}",
        manifest["quickjs"].as_bool().unwrap_or(false)
    );
    println!("napi: {}", manifest["napi"].as_bool().unwrap_or(false));
    Ok(())
}

fn artifact_manifest_from_bytes(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let marker = ARTIFACT_MARKER.as_bytes();
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or("executable does not contain Thaw artifact metadata")?
        + marker.len();
    let end = bytes[offset..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|length| offset + length)
        .ok_or("Thaw artifact metadata is not null terminated")?;
    serde_json::from_slice(&bytes[offset..end])
        .map_err(|error| format!("invalid Thaw artifact metadata: {error}"))
}

fn run_registry(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => run_registry_add(&args[1..]),
        _ => Err("usage: thaw registry add <package>[@<version>] [--registry <dir>]".to_string()),
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

    let package =
        package.ok_or("missing package name (usage: thaw registry add <package>[@<version>])")?;
    // `add` accepts an optional `@<version>` suffix (same syntax as `npm
    // install`) but the registry's own directory is always named after
    // the bare package name, independent of which version was requested
    // -- `thaw_registry::package_name` does the same split thaw-registry
    // itself uses internally, purely for this function's own display
    // purposes.
    let name = thaw_registry::package_name(&package);

    println!("fetching `{package}`...");
    let added = thaw_registry::add(&registry_dir, &package)?;
    let bundle_note = if added.bundled_file_count > 1 {
        format!(" (bundled {} files)", added.bundled_file_count)
    } else {
        String::new()
    };
    let deps_note = if added.dependency_versions.len() > 1 {
        let mut deps: Vec<String> = added
            .dependency_versions
            .iter()
            .filter(|(dep_name, _)| dep_name.as_str() != name)
            .map(|(dep_name, version)| format!("{dep_name}@{version}"))
            .collect();
        deps.sort();
        format!("\n  deps:  {}", deps.join(", "))
    } else {
        String::new()
    };
    let native_note = if let Some(native) = &added.native_addon {
        format!(
            "\n  native: {} ({}/{} {}, sha256 {})",
            native.source, native.platform, native.arch, native.libc, native.sha256
        )
    } else if let Some(diagnostic) = &added.native_diagnostic {
        format!("\n  native: skipped ({diagnostic}); using JavaScript fallback")
    } else {
        String::new()
    };
    println!(
        "added `{name}@{}` to `{}`\n  types: {}\n  main:  {}{bundle_note}{deps_note}{native_note}",
        added.resolved_version,
        registry_dir.join(name).display(),
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
    let mut ffi_metadata: Vec<PathBuf> = Vec::new();
    let mut registry_dir = PathBuf::from("thaw_modules");
    let mut use_packages: Vec<String> = Vec::new();
    let mut static_link = false;

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
            "--ffi-metadata" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or("--ffi-metadata requires a path argument")?;
                ffi_metadata.push(PathBuf::from(value));
            }
            "--registry" => {
                i += 1;
                let value = args.get(i).ok_or("--registry requires a path argument")?;
                registry_dir = PathBuf::from(value);
            }
            "--use" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or("--use requires a package name argument")?;
                use_packages.push(value.clone());
            }
            "--static" => static_link = true,
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

    build_with_link_mode(
        &input,
        &output,
        &extra_links,
        &bridge_dts,
        &ffi_metadata,
        &registry_dir,
        &use_packages,
        static_link,
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
        // `true`: the manual `--bridge` path trusts the classification
        // as-is, since the user is already responsible for supplying a
        // matching `--link`ed library themselves for any FastPath
        // function it declares (unlike `--use`, see
        // `generate_registry_shims`, where thaw-registry knows whether a
        // `native.a` actually exists).
        shim.push_str(&thaw_bridge::generate_shim(&functions, true, &[]));
    }
    Ok(shim)
}

include!("registry_integration/shims.rs");
include!("registry_integration/class_methods.rs");
include!("registry_integration/calls.rs");

include!("ffi_metadata.rs");

include!("build.rs");

#[cfg(test)]
mod tests;
