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

/// One `--use`d package, resolved and classified, but with no shim text
/// generated yet -- collision resolution (see `generate_registry_shims`)
/// needs to see every package's declared names *before* deciding how any
/// individual one should be rendered, so this is deliberately kept as an
/// intermediate step rather than folded into one pass.
struct ResolvedPackage {
    name: String,
    functions: Vec<thaw_bridge::DtsFunction>,
    classes: Vec<thaw_bridge::DtsClass>,
    classifications: Vec<(String, thaw_bridge::Classification)>,
    native_lib: Option<PathBuf>,
    native_addon: Option<PathBuf>,
    bundle_js: Option<String>,
}

/// A valid JS/Thaw identifier fragment from an arbitrary package name --
/// `@hapi/hoek` -> `_hapi_hoek`. Used to build a package-qualified alias
/// identifier (`generate_registry_shims`'s collision resolution); doesn't
/// need to be reversible or collision-free against unrelated packages
/// with a similar sanitized form, since it's always combined with the
/// original function name too.
fn sanitize_identifier(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn render_dynamic_type(ty: &thaw_hir::HirType) -> Option<String> {
    match ty {
        thaw_hir::HirType::F64 => Some("number".into()),
        thaw_hir::HirType::Str => Some("string".into()),
        thaw_hir::HirType::Bool => Some("boolean".into()),
        thaw_hir::HirType::Void => Some("void".into()),
        thaw_hir::HirType::Json => Some("Json".into()),
        thaw_hir::HirType::JsValue => Some("JsValue".into()),
        thaw_hir::HirType::Array(element) => {
            render_dynamic_type(element).map(|element| format!("{element}[]"))
        }
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .map(|(name, ty)| render_dynamic_type(ty).map(|ty| format!("{name}: {ty}")))
            .collect::<Option<Vec<_>>>()
            .map(|fields| format!("{{ {} }}", fields.join("; "))),
        thaw_hir::HirType::Function(params, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| render_dynamic_type(ty).map(|ty| format!("arg{index}: {ty}")))
                .collect::<Option<Vec<_>>>()?;
            let ret = render_dynamic_type(ret)?;
            Some(format!("({}) => {ret}", params.join(", ")))
        }
        _ => None,
    }
}

fn typed_dynamic_declaration(
    package: &str,
    function: &thaw_bridge::DtsFunction,
    napi: bool,
) -> Option<(String, String)> {
    let params = function
        .params
        .iter()
        .map(|(name, ty)| match ty {
            thaw_bridge::DtsType::Native(ty) => {
                render_dynamic_type(ty).map(|ty| format!("{name}: {ty}"))
            }
            thaw_bridge::DtsType::Unsupported(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let thaw_bridge::DtsType::Native(ret) = &function.ret else {
        return None;
    };
    let ret = render_dynamic_type(ret)?;
    let runtime_key = if napi {
        function.name.clone()
    } else {
        format!("{package}::{}", function.name)
    };
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!(
        "__thaw_typed_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    Some((
        symbol.clone(),
        format!("declare function {symbol}({}): {ret};\n", params.join(", ")),
    ))
}

/// `(package, name, alias)` -- see `rewrite_qualified_calls`. `package`
/// here is the *qualifier identifier* (`qualifier_identifier`), not
/// necessarily the real package name.
type QualifiedCallRewrite = (String, String, String);
type ClassConstructorRewrite = (String, String);
/// `(class, method, helper, argument_count, has_callback, parameter_types)`.
type ClassMethodRewrite = (String, String, String, usize, bool, Vec<thaw_hir::HirType>);
/// `(class, property, helper)` for an instance getter.
type ClassGetterRewrite = (String, String, String);
/// `(class, property, helper, value_type)` for an instance setter.
type ClassSetterRewrite = (String, String, String, thaw_hir::HirType);
/// `(qualifier, class, property, helper)` for a static getter.
type StaticClassGetterRewrite = (String, String, String, String);
/// `(qualifier, class, property, helper, value_type)` for a static setter.
type StaticClassSetterRewrite = (String, String, String, String, thaw_hir::HirType);
/// `(qualifier, class, method, helper, argument_count, has_callback, parameter_types)`.
type StaticClassMethodRewrite = (
    String,
    String,
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
);

fn supported_class_method_param(ty: &thaw_bridge::DtsType, index: usize, len: usize) -> bool {
    matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
                | thaw_hir::HirType::Object(_)
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if **element == thaw_hir::HirType::F64
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Function(params, ret))
            if index + 1 == len
                && params.len() <= 2
                && params.iter().all(|param| *param == thaw_hir::HirType::Json)
                && matches!(**ret, thaw_hir::HirType::Json | thaw_hir::HirType::Void)
    )
}

fn supported_class_method_return(ty: &thaw_bridge::DtsType) -> bool {
    matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
                | thaw_hir::HirType::Void
                | thaw_hir::HirType::Object(_)
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if **element == thaw_hir::HirType::F64
    )
}

fn generate_napi_class_method_overloads(
    class: &thaw_bridge::DtsClass,
    is_static: bool,
    observed_arities: &std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    shim: &mut String,
) -> Vec<(String, String, usize, bool, Vec<thaw_hir::HirType>)> {
    let mut generated = Vec::new();
    let mut method_names = std::collections::HashSet::new();
    for method in &class.methods {
        if method.is_static != is_static
            || method.kind != thaw_bridge::DtsMethodKind::Method
            || !method_names.insert(method.name.clone())
        {
            continue;
        }
        let overloads = class
            .methods
            .iter()
            .filter(|candidate| {
                candidate.name == method.name
                    && candidate.is_static == is_static
                    && candidate.kind == thaw_bridge::DtsMethodKind::Method
            })
            .filter(|candidate| {
                candidate.params.iter().enumerate().all(|(index, (_, ty))| {
                    supported_class_method_param(ty, index, candidate.params.len())
                }) && candidate
                    .rest_param
                    .as_ref()
                    .is_none_or(|(_, ty)| supported_class_method_param(ty, 0, 1))
                    && supported_class_method_return(&candidate.ret)
            })
            .collect::<Vec<_>>();
        for (overload_index, overload) in overloads.into_iter().enumerate() {
            let thaw_bridge::DtsType::Native(return_type) = &overload.ret else {
                continue;
            };
            let Some(return_type) = (if *return_type == thaw_hir::HirType::Void {
                Some("Json".to_string())
            } else {
                render_dynamic_type(return_type)
            }) else {
                continue;
            };
            let argument_counts = if overload.rest_param.is_some() {
                observed_arities
                    .get(&method.name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|count| *count >= overload.required_params)
                    .collect::<Vec<_>>()
            } else {
                (overload.required_params..=overload.params.len()).collect()
            };
            for argument_count in argument_counts {
                let fixed_count = argument_count.min(overload.params.len());
                let mut included_params = overload.params[..fixed_count]
                    .iter()
                    .filter_map(|(name, ty)| match ty {
                        thaw_bridge::DtsType::Native(ty) => Some((name.clone(), ty.clone())),
                        thaw_bridge::DtsType::Unsupported(_) => None,
                    })
                    .collect::<Vec<_>>();
                if argument_count > overload.params.len() {
                    let Some((name, thaw_bridge::DtsType::Native(rest_type))) =
                        &overload.rest_param
                    else {
                        continue;
                    };
                    included_params.extend(
                        (overload.params.len()..argument_count)
                            .map(|index| (format!("{name}{index}"), rest_type.clone())),
                    );
                }
                let params = (if is_static {
                    Vec::new()
                } else {
                    vec!["receiver: JsValue".to_string()]
                })
                .into_iter()
                .chain(included_params.iter().map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        render_dynamic_type(ty).expect("filtered above")
                    )
                }))
                .collect::<Vec<_>>()
                .join(", ");
                let has_callback = matches!(
                    included_params.last(),
                    Some((_, thaw_hir::HirType::Function(_, _)))
                );
                let runtime_key = format!(
                    "{}{}${}$overload{overload_index}$arity{argument_count}",
                    match (is_static, &overload.ret) {
                        (true, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$staticmethodvoid$"
                        }
                        (true, _) => "$staticmethod$",
                        (false, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$methodvoid$"
                        }
                        (false, _) => "$method$",
                    },
                    class.name,
                    method.name
                );
                let encoded = runtime_key
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let symbol = format!("__thaw_typed_napi_{encoded}");
                shim.push_str(&format!(
                    "declare function {symbol}({params}): {return_type};\n"
                ));
                generated.push((
                    method.name.clone(),
                    symbol,
                    argument_count,
                    has_callback,
                    included_params.into_iter().map(|(_, ty)| ty).collect(),
                ));
            }
        }
    }
    generated
}
type ExternalExports = std::collections::HashMap<String, std::collections::HashMap<String, String>>;
type RegistryShims = (
    String,
    Vec<PathBuf>,
    Vec<QualifiedCallRewrite>,
    Vec<ClassConstructorRewrite>,
    Vec<ClassMethodRewrite>,
    Vec<StaticClassMethodRewrite>,
    Vec<ClassGetterRewrite>,
    Vec<ClassSetterRewrite>,
    Vec<StaticClassGetterRewrite>,
    Vec<StaticClassSetterRewrite>,
    ExternalExports,
);

/// The identifier a user writes as the object in `pkg.name(...)`
/// qualified-call syntax for a `--use`d package. A scoped package's real
/// name (`@hapi/hoek`) isn't a valid identifier at all (`@`, `/`), so
/// this uses its last path segment (`hoek`) instead -- an unscoped name
/// has no `/` to split on and passes through unchanged. Two different
/// scoped packages sharing a last segment (`@foo/utils`/`@bar/utils`)
/// would collide *here* instead, ambiguously; not handled specially --
/// no real package pair triggering this has been found yet, matching
/// how every other gap in this project got filled (see
/// docs/design/registry.md).
fn qualifier_identifier(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

fn is_native_builtin(package: &str) -> bool {
    matches!(package, "node:fs" | "node:http")
}

fn observed_member_call_arities(
    source: &str,
) -> Result<std::collections::HashMap<String, std::collections::BTreeSet<usize>>, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, MemberProp};

    #[derive(Default)]
    struct Finder {
        arities: std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    }

    impl Visit for Finder {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    if let MemberProp::Ident(method) = &member.prop {
                        self.arities
                            .entry(method.sym.to_string())
                            .or_default()
                            .insert(call.args.len());
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder::default();
    module.visit_with(&mut finder);
    Ok(finder.arities)
}

/// Resolves each `--use`d package against the local registry
/// (thaw-registry; `registry_dir` defaults to `thaw_modules/`),
/// generating its callable surface exactly like `generate_bridge_shims`
/// does for a standalone `.d.ts` -- but additionally auto-linking the
/// package's `native.a` if it ships one (replacing a manual `--link`),
/// and collecting its `bundle.js` (if any) into a single generated
/// `__thaw_module_init` (thaw-bridge's `generate_module_init`) so it's
/// auto-loaded before user code runs (replacing a manual `loadScript`
/// call). Returns the generated shim text, the native lib paths to
/// link, and any cross-package name-collision rewrites the caller must
/// also apply to the user's own source (`rewrite_qualified_calls`).
fn generate_registry_shims(
    registry_dir: &Path,
    use_packages: &[String],
    user_source: &str,
) -> Result<RegistryShims, String> {
    let observed_arities = observed_member_call_arities(user_source)?;
    let mut resolved = Vec::new();
    for name in use_packages {
        let package = if name.starts_with("node:") {
            thaw_registry::resolve_builtin(name)?
        } else {
            thaw_registry::resolve(registry_dir, name)?
        };
        let functions = thaw_bridge::parse_dts(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts: {e}"))?;
        let classes = thaw_bridge::parse_dts_classes(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts classes: {e}"))?;
        // Whether there's actually a `native.a` to link a FastPath
        // signature against -- without one, a fully-primitive real npm
        // function (e.g. date-fns's `daysToWeeks(days: number): number`)
        // would still classify FastPath on type shape alone and produce
        // an unresolvable `declare function`, even though a working JS
        // implementation is sitting right there in `bundle.js`. See
        // `thaw_bridge::effective_classifications`'s doc comment.
        let native_lib_available = package.native_lib.is_some() || is_native_builtin(&package.name);
        let classifications =
            thaw_bridge::effective_classifications(&functions, native_lib_available);
        resolved.push(ResolvedPackage {
            name: package.name.clone(),
            functions,
            classes,
            classifications,
            native_lib: package.native_lib,
            native_addon: package.native_addon,
            bundle_js: package.bundle_js,
        });
    }

    // Every generated top-level name -- FastPath ambient declaration or
    // Fallback wrapper alike -- would otherwise land in the *same* flat
    // global scope (QuickJS-NG globals for Fallback, the LLVM module's
    // own symbol table for FastPath). Two different packages exporting
    // the same name (e.g. `qs` and `@hapi/hoek` both export `stringify`)
    // used to silently collide: whichever package's shim/binding ran
    // last won, with no error -- exactly the kind of order-dependent
    // surprise this project has treated as a bug to catch loudly every
    // other time it showed up (see thaw-bridge's `classify_all`, for the
    // same problem one level down, *within* one `.d.ts`'s own
    // overloads). A collision where every involved package classifies
    // the name as Fallback is auto-resolved below by dropping the bare
    // name for it (`QualifiedFallback::suppress_bare`), forcing
    // qualified syntax (`qs.stringify(x)`, rewritten to a package-
    // qualified alias -- see `rewrite_qualified_calls`); a
    // FastPath-involved collision is a real native-symbol clash this
    // can't paper over, so it stays a hard error.
    let mut declared_by: std::collections::HashMap<String, Vec<(String, bool)>> =
        std::collections::HashMap::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            let is_fast_path = matches!(classification, thaw_bridge::Classification::FastPath(_));
            declared_by
                .entry(name.clone())
                .or_default()
                .push((pkg.name.clone(), is_fast_path));
        }
    }
    for (name, packages) in &declared_by {
        if packages.len() < 2 {
            continue;
        }
        if let Some((fast_path_pkg, _)) = packages.iter().find(|(_, is_fast_path)| *is_fast_path) {
            let other_pkg = packages
                .iter()
                .map(|(p, _)| p.as_str())
                .find(|p| *p != fast_path_pkg)
                .unwrap_or(fast_path_pkg);
            return Err(format!(
                "`{name}` is declared by both `{fast_path_pkg}` and `{other_pkg}` -- automatic \
                 resolution only covers Fallback functions, not a Fast path native symbol clash"
            ));
        }
    }
    let colliding: std::collections::HashSet<&String> = declared_by
        .iter()
        .filter(|(_, pkgs)| pkgs.len() > 1)
        .map(|(name, _)| name)
        .collect();

    // Every Fallback name of every `--use`d package also gets a package-
    // qualified alias -- not just names that actually collide -- so
    // `pkg.name(...)` syntax works consistently for any `--use`d
    // package's function, whether or not `name` happens to collide with
    // some other package (see `thaw_bridge::QualifiedFallback`'s doc
    // comment: `suppress_bare` is the only thing collision status
    // changes). `rewrites` is `(package, name, alias)`, for rewriting
    // `pkg.name(...)` call syntax in the user's own source (see
    // `rewrite_qualified_calls`).
    let mut qualified_by_package: std::collections::HashMap<
        String,
        Vec<thaw_bridge::QualifiedFallback>,
    > = std::collections::HashMap::new();
    let mut rewrites: Vec<QualifiedCallRewrite> = Vec::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            if !matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                continue;
            }
            let alias = format!("{}_{name}", sanitize_identifier(&pkg.name));
            let qualified_key = format!("{}::{name}", pkg.name);
            qualified_by_package
                .entry(pkg.name.clone())
                .or_default()
                .push(thaw_bridge::QualifiedFallback {
                    name: name.clone(),
                    alias: alias.clone(),
                    qualified_key,
                    suppress_bare: colliding.contains(name),
                });
            rewrites.push((
                qualifier_identifier(&pkg.name).to_string(),
                name.clone(),
                alias,
            ));
        }
    }

    /// `(package_name, js_source, fallback_function_names, qualified_aliases)`
    /// -- kept as owned data so the borrowed `ModuleBundle`s built from it
    /// below can outlive the loop that collects it.
    type PendingBundle = (String, String, Vec<String>, Vec<(String, String)>);

    let mut shim = String::new();
    let mut typed_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut class_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut class_rewrites = Vec::new();
    let mut class_method_rewrites = Vec::new();
    let mut static_class_method_rewrites = Vec::new();
    let mut class_getter_rewrites = Vec::new();
    let mut class_setter_rewrites = Vec::new();
    let mut static_class_getter_rewrites = Vec::new();
    let mut static_class_setter_rewrites = Vec::new();
    let mut native_libs = Vec::new();
    let mut bundles: Vec<PendingBundle> = Vec::new();
    let mut native_addons: Vec<(String, Vec<u8>, Option<String>)> = Vec::new();
    let no_qualified: Vec<thaw_bridge::QualifiedFallback> = Vec::new();

    for pkg in &resolved {
        let native_lib_available = pkg.native_lib.is_some() || is_native_builtin(&pkg.name);
        let qualified = qualified_by_package.get(&pkg.name).unwrap_or(&no_qualified);
        if pkg.native_addon.is_some() {
            for class in &pkg.classes {
                let Some(params) = class
                    .constructors
                    .iter()
                    .map(|constructor| {
                        constructor
                            .params
                            .iter()
                            .take_while(|(_, ty)| {
                                matches!(ty, thaw_bridge::DtsType::Native(native) if render_dynamic_type(native).is_some())
                            })
                            .collect::<Vec<_>>()
                    })
                    .filter(|params| !params.is_empty())
                    .min_by_key(|params| params.len())
                else {
                    continue;
                };
                let rendered = params
                    .iter()
                    .map(|(name, ty)| match ty {
                        thaw_bridge::DtsType::Native(ty) => {
                            format!("{name}: {}", render_dynamic_type(ty).unwrap())
                        }
                        thaw_bridge::DtsType::Unsupported(_) => unreachable!(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let runtime_key = format!("$new${}", class.name);
                let encoded = runtime_key
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let symbol = format!("__thaw_typed_napi_{encoded}");
                shim.push_str(&format!(
                    "declare function {symbol}({rendered}): JsValue;\n"
                ));
                class_targets.insert((pkg.name.clone(), class.name.clone()), symbol);
                class_rewrites.push((
                    qualifier_identifier(&pkg.name).to_string(),
                    class.name.clone(),
                ));

                for (method, symbol, argument_count, has_callback, parameter_types) in
                    generate_napi_class_method_overloads(class, false, &observed_arities, &mut shim)
                {
                    class_method_rewrites.push((
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$getter${}{}", class.name, format_args!("${}", getter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue): {return_type};\n"
                    ));
                    class_getter_rewrites.push((class.name.clone(), getter.name.clone(), symbol));
                }
                for setter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$setter${}{}", class.name, format_args!("${}", setter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue, value: {rendered_type}): {rendered_type};\n"
                    ));
                    class_setter_rewrites.push((
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticgetter${}{}",
                        class.name,
                        format_args!("${}", getter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!("declare function {symbol}(): {return_type};\n"));
                    static_class_getter_rewrites.push((
                        qualifier_identifier(&pkg.name).to_string(),
                        class.name.clone(),
                        getter.name.clone(),
                        symbol,
                    ));
                }
                for setter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticsetter${}{}",
                        class.name,
                        format_args!("${}", setter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(value: {rendered_type}): {rendered_type};\n"
                    ));
                    static_class_setter_rewrites.push((
                        qualifier_identifier(&pkg.name).to_string(),
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for (method, symbol, argument_count, has_callback, parameter_types) in
                    generate_napi_class_method_overloads(class, true, &observed_arities, &mut shim)
                {
                    static_class_method_rewrites.push((
                        qualifier_identifier(&pkg.name).to_string(),
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
            }
        }
        for function in &pkg.functions {
            let is_fallback = pkg.classifications.iter().any(|(name, classification)| {
                name == &function.name
                    && matches!(classification, thaw_bridge::Classification::Fallback { .. })
            });
            if is_fallback {
                if let Some((symbol, declaration)) =
                    typed_dynamic_declaration(&pkg.name, function, pkg.native_addon.is_some())
                {
                    shim.push_str(&declaration);
                    typed_targets.insert((pkg.name.clone(), function.name.clone()), symbol);
                }
            }
        }
        if pkg.native_addon.is_some() {
            shim.push_str(&thaw_bridge::generate_native_addon_shim(
                &pkg.functions,
                qualified,
            ));
        } else {
            shim.push_str(&thaw_bridge::generate_shim(
                &pkg.functions,
                native_lib_available,
                qualified,
            ));
        }

        if let Some(native_lib) = &pkg.native_lib {
            native_libs.push(native_lib.clone());
        }
        if let Some(native_addon) = &pkg.native_addon {
            let bytes = std::fs::read(native_addon).map_err(|error| {
                format!(
                    "failed to embed native addon `{}`: {error}",
                    native_addon.display()
                )
            })?;
            native_addons.push((
                pkg.name.clone(),
                bytes,
                (pkg.functions.len() == 1).then(|| pkg.functions[0].name.clone()),
            ));
        } else if let Some(bundle_js) = &pkg.bundle_js {
            // Only Fallback functions need binding inside the loaded
            // script (see `ModuleBundle::fallback_names`'s doc comment);
            // FastPath functions are real FFI calls and never touch
            // QuickJS-NG at all.
            let fallback_names = pkg
                .classifications
                .iter()
                .filter_map(|(name, classification)| match classification {
                    thaw_bridge::Classification::Fallback { .. } => Some(name.clone()),
                    thaw_bridge::Classification::FastPath(_) => None,
                })
                .collect();
            let qualified_aliases = qualified
                .iter()
                .map(|q| (q.name.clone(), q.qualified_key.clone()))
                .collect();
            bundles.push((
                pkg.name.clone(),
                bundle_js.clone(),
                fallback_names,
                qualified_aliases,
            ));
        }
    }

    let module_bundles: Vec<thaw_bridge::ModuleBundle> = bundles
        .iter()
        .map(
            |(name, js, fallback_names, qualified_aliases)| thaw_bridge::ModuleBundle {
                package_name: name.as_str(),
                js_source: js.as_str(),
                fallback_names,
                qualified_aliases,
            },
        )
        .collect();
    shim.push_str(&thaw_bridge::generate_module_init(&module_bundles));
    let native_addons: Vec<thaw_bridge::NativeAddon<'_>> = native_addons
        .iter()
        .map(|(name, bytes, root_export)| thaw_bridge::NativeAddon {
            package_name: name,
            bytes,
            root_export: root_export.as_deref(),
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_native_addon_init(&native_addons));

    let mut external_exports = ExternalExports::new();
    for pkg in &resolved {
        let mut package_exports = std::collections::HashMap::new();
        for (name, classification) in &pkg.classifications {
            let target = if matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                typed_targets
                    .get(&(pkg.name.clone(), name.clone()))
                    .cloned()
                    .unwrap_or_else(|| format!("{}_{name}", sanitize_identifier(&pkg.name)))
            } else {
                name.clone()
            };
            package_exports.insert(name.clone(), target.clone());
        }
        for class in &pkg.classes {
            if let Some(target) = class_targets.get(&(pkg.name.clone(), class.name.clone())) {
                package_exports.insert(class.name.clone(), target.clone());
            }
        }
        if package_exports.len() == 1 {
            let target = package_exports.values().next().unwrap().clone();
            package_exports.insert("default".to_string(), target);
        }
        external_exports.insert(pkg.name.clone(), package_exports);
    }

    Ok((
        shim,
        native_libs,
        rewrites,
        class_rewrites,
        class_method_rewrites,
        static_class_method_rewrites,
        class_getter_rewrites,
        class_setter_rewrites,
        static_class_getter_rewrites,
        static_class_setter_rewrites,
        external_exports,
    ))
}

#[cfg(test)]
fn rewrite_external_class_methods(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
) -> Result<String, String> {
    rewrite_external_class_methods_with_static(source, classes, methods, &[], &[], &[], &[], &[])
}

#[allow(clippy::too_many_arguments)]
fn rewrite_external_class_methods_with_static(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
    static_methods: &[StaticClassMethodRewrite],
    getters: &[ClassGetterRewrite],
    setters: &[ClassSetterRewrite],
    static_getters: &[StaticClassGetterRewrite],
    static_setters: &[StaticClassSetterRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowFunctionBody, AssignExpr, AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, Expr,
        FnDecl, ForStmt, FunctionBody, IfStmt, Lit, MemberProp, Pat, Prop, PropName, PropOrSpread,
        ReturnStmt, SimpleAssignTarget, TryStmt, TsKeywordTypeKind, TsType, UnaryOp, VarDeclarator,
        WhileStmt,
    };
    use thaw_parser::common::Spanned;

    if methods.is_empty()
        && static_methods.is_empty()
        && getters.is_empty()
        && setters.is_empty()
        && static_getters.is_empty()
        && static_setters.is_empty()
    {
        return Ok(source.to_string());
    }

    fn constructed_class<'a>(
        expression: &'a Expr,
        classes: &'a [ClassConstructorRewrite],
    ) -> Option<&'a str> {
        let Expr::New(new_expression) = expression else {
            return None;
        };
        match new_expression.callee.as_ref() {
            Expr::Ident(class) => classes
                .iter()
                .find(|(_, name)| name == class.sym.as_str())
                .map(|(_, name)| name.as_str()),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(package), MemberProp::Ident(class)) => classes
                    .iter()
                    .find(|(qualifier, name)| {
                        qualifier == package.sym.as_str() && name == class.sym.as_str()
                    })
                    .map(|(_, name)| name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn source_instance_class(
        expression: &Expr,
        classes: &[ClassConstructorRewrite],
        variables: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => variables.get(identifier.sym.as_str()).cloned(),
            Expr::Member(member) => member_assignment_path(member)
                .map(|(root, path)| instance_path_key(&root, &path))
                .and_then(|key| variables.get(&key).cloned()),
            Expr::Paren(parenthesized) => {
                source_instance_class(&parenthesized.expr, classes, variables)
            }
            Expr::TsAs(assertion) => source_instance_class(&assertion.expr, classes, variables),
            Expr::TsTypeAssertion(assertion) => {
                source_instance_class(&assertion.expr, classes, variables)
            }
            _ => constructed_class(expression, classes).map(str::to_owned),
        }
    }

    fn instance_path_key(root: &str, path: &[String]) -> String {
        let mut key = root.to_string();
        for property in path {
            key.push('\u{1f}');
            key.push_str(property);
        }
        key
    }

    fn invalidate_instance_path(
        variables: &mut std::collections::HashMap<String, String>,
        key: &str,
    ) {
        let descendant = format!("{key}\u{1f}");
        variables.retain(|candidate, _| candidate != key && !candidate.starts_with(&descendant));
    }

    fn instance_receiver(expression: &Expr) -> Option<(String, String)> {
        match expression {
            Expr::Ident(identifier) => {
                let name = identifier.sym.to_string();
                Some((name.clone(), name))
            }
            Expr::Member(member) => {
                let (key, rendered) = instance_receiver(&member.obj)?;
                match &member.prop {
                    MemberProp::Ident(property) => Some((
                        format!("{key}\u{1f}{}", property.sym),
                        format!("{rendered}.{}", property.sym),
                    )),
                    MemberProp::Computed(computed) => {
                        let Expr::Lit(Lit::Str(property)) = computed.expr.as_ref() else {
                            return None;
                        };
                        let property = property.value.to_string_lossy().into_owned();
                        Some((
                            format!("{key}\u{1f}{property}"),
                            format!("{rendered}[{}]", serde_json::to_string(&property).ok()?),
                        ))
                    }
                    MemberProp::PrivateName(_) => None,
                }
            }
            Expr::Paren(parenthesized) => instance_receiver(&parenthesized.expr),
            Expr::TsAs(assertion) => instance_receiver(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => instance_receiver(&assertion.expr),
            _ => None,
        }
    }

    fn static_class_receiver(expression: &Expr) -> Option<(Option<String>, String)> {
        match expression {
            Expr::Ident(class) => Some((None, class.sym.to_string())),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                    Some((Some(qualifier.sym.to_string()), class.sym.to_string()))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn collect_object_instance_classes(
        expression: &Expr,
        prefix: &str,
        classes: &[ClassConstructorRewrite],
        variables: &std::collections::HashMap<String, String>,
        additions: &mut Vec<(String, String)>,
    ) {
        let Expr::Object(object) = expression else {
            return;
        };
        for property in &object.props {
            let PropOrSpread::Prop(property) = property else {
                continue;
            };
            let (name, value): (String, &Expr) = match property.as_ref() {
                Prop::KeyValue(property) => {
                    let Some(name) = (match &property.key {
                        PropName::Ident(identifier) => Some(identifier.sym.to_string()),
                        PropName::Str(value) => Some(value.value.to_string_lossy().into_owned()),
                        PropName::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(value)) => {
                                Some(value.value.to_string_lossy().into_owned())
                            }
                            _ => None,
                        },
                        _ => None,
                    }) else {
                        continue;
                    };
                    (name, property.value.as_ref())
                }
                Prop::Shorthand(identifier) => {
                    let key = format!("{prefix}\u{1f}{}", identifier.sym);
                    if let Some(class) = variables.get(identifier.sym.as_str()) {
                        additions.push((key, class.clone()));
                    }
                    continue;
                }
                _ => continue,
            };
            let key = format!("{prefix}\u{1f}{name}");
            if let Some(class) = source_instance_class(value, classes, variables) {
                additions.push((key.clone(), class));
            }
            collect_object_instance_classes(value, &key, classes, variables, additions);
        }
    }

    fn source_expr_type(
        expression: &Expr,
        variables: &std::collections::HashMap<String, thaw_hir::HirType>,
        functions: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        match expression {
            Expr::Lit(Lit::Num(_)) => Some(thaw_hir::HirType::F64),
            Expr::Lit(Lit::Str(_)) => Some(thaw_hir::HirType::Str),
            Expr::Lit(Lit::Bool(_)) => Some(thaw_hir::HirType::Bool),
            Expr::Array(array)
                if array.elems.iter().all(|element| {
                    element.as_ref().is_some_and(|element| {
                        source_expr_type(element.expr.as_ref(), variables, functions)
                            == Some(thaw_hir::HirType::F64)
                    })
                }) =>
            {
                Some(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)))
            }
            Expr::Object(object) => {
                let mut fields = Vec::with_capacity(object.props.len());
                for property in &object.props {
                    if let PropOrSpread::Spread(spread) = property {
                        let Some(thaw_hir::HirType::Object(spread_fields)) =
                            source_expr_type(&spread.expr, variables, functions)
                        else {
                            return Some(thaw_hir::HirType::Object(Vec::new()));
                        };
                        for (name, value_type) in spread_fields {
                            fields.retain(|(existing, _)| existing != &name);
                            fields.push((name, value_type));
                        }
                        continue;
                    }
                    let PropOrSpread::Prop(property) = property else {
                        unreachable!();
                    };
                    let (name, value_type) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let name = match &property.key {
                                PropName::Ident(identifier) => identifier.sym.to_string(),
                                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => match computed.expr.as_ref() {
                                    Expr::Lit(Lit::Str(value)) => {
                                        value.value.to_string_lossy().into_owned()
                                    }
                                    _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                                },
                                _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                            };
                            let Some(value_type) =
                                source_expr_type(&property.value, variables, functions)
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (name, value_type)
                        }
                        Prop::Shorthand(identifier) => {
                            let Some(value_type) = variables.get(identifier.sym.as_str()).cloned()
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (identifier.sym.to_string(), value_type)
                        }
                        _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                    };
                    fields.retain(|(existing, _)| existing != &name);
                    fields.push((name, value_type));
                }
                Some(thaw_hir::HirType::Object(fields))
            }
            Expr::Ident(identifier) => variables.get(identifier.sym.as_str()).cloned(),
            Expr::Paren(parenthesized) => {
                source_expr_type(&parenthesized.expr, variables, functions)
            }
            Expr::TsAs(assertion) => source_expr_type(&assertion.expr, variables, functions),
            Expr::TsTypeAssertion(assertion) => {
                source_expr_type(&assertion.expr, variables, functions)
            }
            Expr::Tpl(_) => Some(thaw_hir::HirType::Str),
            Expr::Unary(unary) => match unary.op {
                UnaryOp::Plus | UnaryOp::Minus
                    if source_expr_type(&unary.arg, variables, functions)
                        == Some(thaw_hir::HirType::F64) =>
                {
                    Some(thaw_hir::HirType::F64)
                }
                UnaryOp::Bang => Some(thaw_hir::HirType::Bool),
                _ => None,
            },
            Expr::Bin(binary) => {
                let left = source_expr_type(&binary.left, variables, functions);
                let right = source_expr_type(&binary.right, variables, functions);
                match binary.op {
                    BinaryOp::Add
                        if left == Some(thaw_hir::HirType::Str)
                            && right == Some(thaw_hir::HirType::Str) =>
                    {
                        Some(thaw_hir::HirType::Str)
                    }
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Exp
                        if left == Some(thaw_hir::HirType::F64)
                            && right == Some(thaw_hir::HirType::F64) =>
                    {
                        Some(thaw_hir::HirType::F64)
                    }
                    BinaryOp::EqEq
                    | BinaryOp::NotEq
                    | BinaryOp::EqEqEq
                    | BinaryOp::NotEqEq
                    | BinaryOp::Lt
                    | BinaryOp::LtEq
                    | BinaryOp::Gt
                    | BinaryOp::GtEq => Some(thaw_hir::HirType::Bool),
                    _ => None,
                }
            }
            Expr::Cond(conditional) => {
                let consequent = source_expr_type(&conditional.cons, variables, functions);
                let alternate = source_expr_type(&conditional.alt, variables, functions);
                (consequent == alternate).then_some(consequent).flatten()
            }
            Expr::Call(call) => match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(identifier) if identifier.sym == *"Number" => {
                        Some(thaw_hir::HirType::F64)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"String" => {
                        Some(thaw_hir::HirType::Str)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"Boolean" => {
                        Some(thaw_hir::HirType::Bool)
                    }
                    Expr::Ident(identifier) => functions.get(identifier.sym.as_str()).cloned(),
                    _ => None,
                },
                _ => None,
            },
            Expr::Member(member) => {
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                if property.sym == *"length" {
                    return Some(thaw_hir::HirType::F64);
                }
                let thaw_hir::HirType::Object(fields) =
                    source_expr_type(&member.obj, variables, functions)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find(|(name, _)| name == property.sym.as_str())
                    .map(|(_, ty)| ty)
            }
            _ => None,
        }
    }

    fn source_ts_type(ty: &TsType) -> Option<thaw_hir::HirType> {
        match ty {
            TsType::TsKeywordType(keyword) => match keyword.kind {
                TsKeywordTypeKind::TsNumberKeyword => Some(thaw_hir::HirType::F64),
                TsKeywordTypeKind::TsStringKeyword => Some(thaw_hir::HirType::Str),
                TsKeywordTypeKind::TsBooleanKeyword => Some(thaw_hir::HirType::Bool),
                TsKeywordTypeKind::TsVoidKeyword => Some(thaw_hir::HirType::Void),
                _ => None,
            },
            TsType::TsArrayType(array) => match source_ts_type(&array.elem_type) {
                Some(thaw_hir::HirType::F64) => {
                    Some(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)))
                }
                _ => None,
            },
            TsType::TsParenthesizedType(parenthesized) => source_ts_type(&parenthesized.type_ann),
            _ => None,
        }
    }

    #[derive(Default)]
    struct FunctionTypeFinder {
        types: std::collections::HashMap<String, thaw_hir::HirType>,
    }

    impl Visit for FunctionTypeFinder {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if let Some(return_type) = declaration
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| source_ts_type(&annotation.type_ann))
            {
                self.types
                    .insert(declaration.ident.sym.to_string(), return_type);
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                let annotation = match initializer.as_ref() {
                    Expr::Arrow(arrow) => arrow.return_type.as_ref(),
                    Expr::Fn(function) => function.function.return_type.as_ref(),
                    _ => None,
                };
                if let Some(return_type) =
                    annotation.and_then(|annotation| source_ts_type(&annotation.type_ann))
                {
                    self.types.insert(binding.id.sym.to_string(), return_type);
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn inferred_block_return_type(
        block: &FunctionBody,
        functions: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        struct Returns<'a> {
            functions: &'a std::collections::HashMap<String, thaw_hir::HirType>,
            types: Vec<Option<thaw_hir::HirType>>,
        }
        impl Visit for Returns<'_> {
            fn visit_return_stmt(&mut self, statement: &ReturnStmt) {
                self.types
                    .push(statement.arg.as_ref().and_then(|expression| {
                        source_expr_type(
                            expression,
                            &std::collections::HashMap::new(),
                            self.functions,
                        )
                    }));
            }

            // Returns inside nested functions belong to those functions, not
            // to the block currently being inferred.
            fn visit_fn_decl(&mut self, _declaration: &FnDecl) {}

            fn visit_arrow_expr(&mut self, _expression: &thaw_parser::ast::ArrowExpr) {}

            fn visit_fn_expr(&mut self, _expression: &thaw_parser::ast::FnExpr) {}
        }

        let mut returns = Returns {
            functions,
            types: Vec::new(),
        };
        block.visit_with(&mut returns);
        let first = returns.types.first()?.clone()?;
        returns
            .types
            .iter()
            .all(|candidate| candidate.as_ref() == Some(&first))
            .then_some(first)
    }

    struct InferredFunctionTypeFinder<'a> {
        known: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        additions: std::collections::HashMap<String, thaw_hir::HirType>,
    }

    impl Visit for InferredFunctionTypeFinder<'_> {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if !self.known.contains_key(declaration.ident.sym.as_str()) {
                if let Some(return_type) = declaration
                    .function
                    .body
                    .as_ref()
                    .and_then(|body| inferred_block_return_type(body, self.known))
                {
                    self.additions
                        .insert(declaration.ident.sym.to_string(), return_type);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                if !self.known.contains_key(binding.id.sym.as_str()) {
                    let return_type = match initializer.as_ref() {
                        Expr::Arrow(arrow) => match arrow.body.as_ref() {
                            ArrowFunctionBody::FunctionBody(block) => {
                                inferred_block_return_type(block, self.known)
                            }
                            ArrowFunctionBody::Expr(expression) => source_expr_type(
                                expression,
                                &std::collections::HashMap::new(),
                                self.known,
                            ),
                        },
                        Expr::Fn(function) => function
                            .function
                            .body
                            .as_ref()
                            .and_then(|body| inferred_block_return_type(body, self.known)),
                        _ => None,
                    };
                    if let Some(return_type) = return_type {
                        self.additions
                            .insert(binding.id.sym.to_string(), return_type);
                    }
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn overload_type_score(declared: &thaw_hir::HirType, actual: &thaw_hir::HirType) -> Option<u8> {
        match (declared, actual) {
            (thaw_hir::HirType::Json, _) => Some(0),
            (thaw_hir::HirType::Object(declared), thaw_hir::HirType::Object(actual)) => {
                if declared.is_empty() || actual.is_empty() {
                    return Some(1);
                }
                let mut score = 2u8;
                for (name, declared_type) in declared {
                    let (_, actual_type) =
                        actual.iter().find(|(candidate, _)| candidate == name)?;
                    score = score.saturating_add(overload_type_score(declared_type, actual_type)?);
                }
                Some(score)
            }
            (left, right) if left == right => Some(2),
            _ => None,
        }
    }

    fn member_property_name(property: &MemberProp) -> Option<String> {
        match property {
            MemberProp::Ident(identifier) => Some(identifier.sym.to_string()),
            MemberProp::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                _ => None,
            },
            MemberProp::PrivateName(_) => None,
        }
    }

    fn member_assignment_path(
        member: &thaw_parser::ast::MemberExpr,
    ) -> Option<(String, Vec<String>)> {
        let mut path = vec![member_property_name(&member.prop)?];
        let mut object = member.obj.as_ref();
        loop {
            match object {
                Expr::Ident(identifier) => {
                    path.reverse();
                    return Some((identifier.sym.to_string(), path));
                }
                Expr::Member(parent) => {
                    path.push(member_property_name(&parent.prop)?);
                    object = parent.obj.as_ref();
                }
                _ => return None,
            }
        }
    }

    fn update_object_property_type(
        object: &mut thaw_hir::HirType,
        path: &[String],
        value: thaw_hir::HirType,
    ) -> bool {
        let thaw_hir::HirType::Object(fields) = object else {
            return false;
        };
        let Some((property, remaining)) = path.split_first() else {
            return false;
        };
        if remaining.is_empty() {
            if let Some((_, ty)) = fields.iter_mut().find(|(name, _)| name == property) {
                *ty = value;
            } else {
                fields.push((property.clone(), value));
            }
            return true;
        }
        fields
            .iter_mut()
            .find(|(name, _)| name == property)
            .is_some_and(|(_, nested)| update_object_property_type(nested, remaining, value))
    }

    struct Finder<'a> {
        classes: &'a [ClassConstructorRewrite],
        methods: &'a [ClassMethodRewrite],
        static_methods: &'a [StaticClassMethodRewrite],
        getters: &'a [ClassGetterRewrite],
        setters: &'a [ClassSetterRewrite],
        static_getters: &'a [StaticClassGetterRewrite],
        static_setters: &'a [StaticClassSetterRewrite],
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        function_types: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
        edits: Vec<(u32, u32, String)>,
    }

    #[derive(Clone)]
    struct FlowState {
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
    }

    impl Finder<'_> {
        fn flow_state(&self) -> FlowState {
            FlowState {
                variables: self.variables.clone(),
                value_types: self.value_types.clone(),
                callbacks: self.callbacks.clone(),
            }
        }

        fn restore_flow_state(&mut self, state: FlowState) {
            self.variables = state.variables;
            self.value_types = state.value_types;
            self.callbacks = state.callbacks;
        }

        fn join_current_flow_with(&mut self, other: &FlowState) {
            self.variables
                .retain(|name, class| other.variables.get(name) == Some(class));
            self.value_types
                .retain(|name, ty| other.value_types.get(name) == Some(ty));
            self.callbacks.retain(|name| other.callbacks.contains(name));
        }
    }

    impl Visit for Finder<'_> {
        fn visit_member_expr(&mut self, member: &thaw_parser::ast::MemberExpr) {
            if let (Some((qualifier, class)), MemberProp::Ident(property)) =
                (static_class_receiver(&member.obj), &member.prop)
            {
                if let Some((_, _, _, helper)) = self.static_getters.iter().find(
                    |(candidate_qualifier, candidate_class, candidate_property, _)| {
                        candidate_class == &class
                            && candidate_property == property.sym.as_str()
                            && qualifier
                                .as_ref()
                                .is_none_or(|qualifier| candidate_qualifier == qualifier)
                    },
                ) {
                    let span = member.span();
                    self.edits
                        .push((span.lo.0, span.hi.0, format!("{helper}()")));
                    return;
                }
            }
            if let (Some((receiver_key, receiver_source)), MemberProp::Ident(property)) =
                (instance_receiver(&member.obj), &member.prop)
            {
                if let Some(class) = self.variables.get(&receiver_key) {
                    if let Some((_, _, helper)) =
                        self.getters
                            .iter()
                            .find(|(candidate_class, candidate_property, _)| {
                                candidate_class == class
                                    && candidate_property == property.sym.as_str()
                            })
                    {
                        let span = member.span();
                        self.edits.push((
                            span.lo.0,
                            span.hi.0,
                            format!("{helper}({receiver_source})"),
                        ));
                        member.obj.visit_with(self);
                        return;
                    }
                }
            }
            member.visit_children_with(self);
        }

        fn visit_if_stmt(&mut self, statement: &IfStmt) {
            statement.test.visit_with(self);
            let base = self.flow_state();

            statement.cons.visit_with(self);
            let consequent = self.flow_state();

            self.restore_flow_state(base);
            if let Some(alternate) = &statement.alt {
                alternate.visit_with(self);
            }

            let alternate = self.flow_state();
            self.restore_flow_state(consequent);
            self.join_current_flow_with(&alternate);
        }

        fn visit_while_stmt(&mut self, statement: &WhileStmt) {
            statement.test.visit_with(self);
            let zero_iterations = self.flow_state();
            statement.body.visit_with(self);
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_for_stmt(&mut self, statement: &ForStmt) {
            if let Some(initializer) = &statement.init {
                initializer.visit_with(self);
            }
            if let Some(test) = &statement.test {
                test.visit_with(self);
            }
            let zero_iterations = self.flow_state();
            statement.body.visit_with(self);
            if let Some(update) = &statement.update {
                update.visit_with(self);
            }
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_try_stmt(&mut self, statement: &TryStmt) {
            let incoming = self.flow_state();
            statement.block.visit_with(self);
            let try_exit = self.flow_state();

            if let Some(handler) = &statement.handler {
                // A throw may occur after any prefix of the try block. Facts
                // that differ between entry and normal exit are therefore not
                // safe assumptions at catch entry.
                let mut catch_entry = try_exit.clone();
                catch_entry
                    .variables
                    .retain(|name, class| incoming.variables.get(name) == Some(class));
                catch_entry
                    .value_types
                    .retain(|name, ty| incoming.value_types.get(name) == Some(ty));
                catch_entry
                    .callbacks
                    .retain(|name| incoming.callbacks.contains(name));
                self.restore_flow_state(catch_entry);
                handler.body.visit_with(self);
                let catch_exit = self.flow_state();

                self.restore_flow_state(try_exit);
                self.join_current_flow_with(&catch_exit);
            }

            // `finally` executes on every path that leaves the construct, so
            // assignments made there can establish new facts after the join.
            if let Some(finalizer) = &statement.finalizer {
                finalizer.visit_with(self);
            }
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                if let Some(class) =
                    source_instance_class(initializer, self.classes, &self.variables)
                {
                    self.variables.insert(binding.id.sym.to_string(), class);
                }
                let mut property_classes = Vec::new();
                collect_object_instance_classes(
                    initializer,
                    binding.id.sym.as_str(),
                    self.classes,
                    &self.variables,
                    &mut property_classes,
                );
                self.variables.extend(property_classes);
                if matches!(initializer.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                    self.callbacks.insert(binding.id.sym.to_string());
                }
                if let Some(ty) =
                    source_expr_type(initializer, &self.value_types, self.function_types)
                {
                    self.value_types.insert(binding.id.sym.to_string(), ty);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    let static_target = match member.obj.as_ref() {
                        Expr::Ident(class) => Some((None, class.sym.as_str())),
                        Expr::Member(class_member) => {
                            match (class_member.obj.as_ref(), &class_member.prop) {
                                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                                    Some((Some(qualifier.sym.as_str()), class.sym.as_str()))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let (Some((qualifier, class)), MemberProp::Ident(method)) =
                        (static_target, &member.prop)
                    {
                        let has_callback = call.args.last().is_some_and(|argument| {
                            matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                        });
                        let selected = self
                            .static_methods
                            .iter()
                            .filter(|candidate| {
                                candidate.1 == class
                                    && candidate.2 == method.sym.as_str()
                                    && candidate.4 == call.args.len()
                                    && candidate.5 == has_callback
                                    && qualifier.is_none_or(|qualifier| candidate.0 == qualifier)
                            })
                            .filter_map(|candidate| {
                                let mut score = 0u16;
                                for (argument, declared) in call.args.iter().zip(&candidate.6) {
                                    if let Some(actual) = source_expr_type(
                                        argument.expr.as_ref(),
                                        &self.value_types,
                                        self.function_types,
                                    ) {
                                        score += u16::from(overload_type_score(declared, &actual)?);
                                    }
                                }
                                Some((score, candidate))
                            })
                            .reduce(|best, candidate| {
                                if candidate.0 > best.0 {
                                    candidate
                                } else {
                                    best
                                }
                            })
                            .map(|(_, candidate)| candidate);
                        if let Some(candidate) = selected {
                            let span = member.span();
                            self.edits.push((span.lo.0, span.hi.0, candidate.3.clone()));
                            call.visit_children_with(self);
                            return;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), MemberProp::Ident(method)) =
                        (instance_receiver(&member.obj), &member.prop)
                    {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            let has_callback = call.args.last().is_some_and(|argument| {
                                matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                    || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                            });
                            let selected = self
                                .methods
                                .iter()
                                .filter(
                                    |(
                                        candidate_class,
                                        candidate_method,
                                        _,
                                        argument_count,
                                        candidate_callback,
                                        _,
                                    )| {
                                        candidate_class == class
                                            && candidate_method == method.sym.as_str()
                                            && *argument_count == call.args.len()
                                            && *candidate_callback == has_callback
                                    },
                                )
                                .filter_map(|candidate| {
                                    let mut score = 0u16;
                                    for (argument, declared) in
                                        call.args.iter().zip(candidate.5.iter())
                                    {
                                        if let Some(actual) = source_expr_type(
                                            argument.expr.as_ref(),
                                            &self.value_types,
                                            self.function_types,
                                        ) {
                                            score +=
                                                u16::from(overload_type_score(declared, &actual)?);
                                        }
                                    }
                                    Some((score, candidate))
                                })
                                .reduce(|best, candidate| {
                                    if candidate.0 > best.0 {
                                        candidate
                                    } else {
                                        best
                                    }
                                })
                                .map(|(_, candidate)| candidate);
                            if let Some((_, _, helper, _, _, _)) = selected {
                                let span = member.span();
                                self.edits.push((span.lo.0, span.hi.0, helper.clone()));
                                let insertion = if call.args.is_empty() {
                                    receiver_source
                                } else {
                                    format!("{receiver_source}, ")
                                };
                                self.edits.push((span.hi.0 + 1, span.hi.0 + 1, insertion));
                            }
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
            let mut setter_rewritten = false;
            if assignment.op == AssignOp::Assign {
                if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left {
                    if let (Some((qualifier, class)), Some(property)) = (
                        static_class_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some((_, _, _, helper, _)) = self.static_setters.iter().find(
                            |(
                                candidate_qualifier,
                                candidate_class,
                                candidate_property,
                                _,
                                declared,
                            )| {
                                candidate_class == &class
                                    && candidate_property == &property
                                    && qualifier
                                        .as_ref()
                                        .is_none_or(|qualifier| candidate_qualifier == qualifier)
                                    && source_expr_type(
                                        &assignment.right,
                                        &self.value_types,
                                        self.function_types,
                                    )
                                    .is_none_or(|actual| {
                                        overload_type_score(declared, &actual).is_some()
                                    })
                            },
                        ) {
                            let member_span = member.span();
                            let right_span = assignment.right.span();
                            let assignment_span = assignment.span();
                            self.edits
                                .push((member_span.lo.0, member_span.hi.0, helper.clone()));
                            self.edits
                                .push((member_span.hi.0, right_span.lo.0, "(".into()));
                            self.edits.push((
                                assignment_span.hi.0,
                                assignment_span.hi.0,
                                ")".into(),
                            ));
                            setter_rewritten = true;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), Some(property)) = (
                        instance_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            if let Some((_, _, helper, _)) =
                                self.setters.iter().find(
                                    |(candidate_class, candidate_property, _, declared)| {
                                        candidate_class == class
                                            && candidate_property == &property
                                            && source_expr_type(
                                                &assignment.right,
                                                &self.value_types,
                                                self.function_types,
                                            )
                                            .is_none_or(|actual| {
                                                overload_type_score(declared, &actual).is_some()
                                            })
                                    },
                                )
                            {
                                let member_span = member.span();
                                let right_span = assignment.right.span();
                                let assignment_span = assignment.span();
                                self.edits.push((
                                    member_span.lo.0,
                                    member_span.hi.0,
                                    helper.clone(),
                                ));
                                self.edits.push((
                                    member_span.hi.0,
                                    right_span.lo.0,
                                    format!("({receiver_source}, "),
                                ));
                                self.edits.push((
                                    assignment_span.hi.0,
                                    assignment_span.hi.0,
                                    ")".into(),
                                ));
                                setter_rewritten = true;
                            }
                        }
                    }
                }
            }
            if setter_rewritten {
                assignment.right.visit_with(self);
            } else {
                assignment.visit_children_with(self);
            }
            let inferred = (assignment.op == AssignOp::Assign)
                .then(|| {
                    source_expr_type(&assignment.right, &self.value_types, self.function_types)
                })
                .flatten();
            let assigned_class = (assignment.op == AssignOp::Assign)
                .then(|| source_instance_class(&assignment.right, self.classes, &self.variables))
                .flatten();
            let AssignTarget::Simple(target) = &assignment.left else {
                return;
            };
            match target {
                SimpleAssignTarget::Ident(binding) => {
                    if let Some(ty) = inferred {
                        self.value_types.insert(binding.id.sym.to_string(), ty);
                    } else {
                        self.value_types.remove(binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                        if let Some(class) = assigned_class {
                            self.variables.insert(binding.id.sym.to_string(), class);
                        }
                    } else {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign
                        && matches!(assignment.right.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                    {
                        self.callbacks.insert(binding.id.sym.to_string());
                    } else {
                        self.callbacks.remove(binding.id.sym.as_str());
                    }
                }
                SimpleAssignTarget::Member(member) => {
                    let (root, path) = match member_assignment_path(member) {
                        Some(path) => path,
                        None => return,
                    };
                    let instance_key = instance_path_key(&root, &path);
                    invalidate_instance_path(&mut self.variables, &instance_key);
                    if let Some(class) = assigned_class {
                        self.variables.insert(instance_key, class);
                    }
                    let Some(value) = inferred else {
                        self.value_types.remove(&root);
                        return;
                    };
                    if let Some(object) = self.value_types.get_mut(&root) {
                        if !update_object_property_type(object, &path, value) {
                            self.value_types.remove(&root);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut function_types = FunctionTypeFinder::default();
    module.visit_with(&mut function_types);
    loop {
        let mut inferred = InferredFunctionTypeFinder {
            known: &function_types.types,
            additions: std::collections::HashMap::new(),
        };
        module.visit_with(&mut inferred);
        inferred
            .additions
            .retain(|name, _| !function_types.types.contains_key(name));
        if inferred.additions.is_empty() {
            break;
        }
        function_types.types.extend(inferred.additions);
    }
    let mut finder = Finder {
        classes,
        methods,
        static_methods,
        getters,
        setters,
        static_getters,
        static_setters,
        variables: std::collections::HashMap::new(),
        value_types: std::collections::HashMap::new(),
        function_types: &function_types.types,
        callbacks: std::collections::HashSet::new(),
        edits: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut edits = finder
        .edits
        .into_iter()
        .map(|(lo, hi, replacement)| {
            let lo = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(lo))
                .pos
                .0 as usize;
            let hi = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(hi))
                .pos
                .0 as usize;
            (lo, hi, replacement)
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(lo, hi, _)| (*lo, *hi));
    let mut output = source.to_string();
    for (lo, hi, replacement) in edits.into_iter().rev() {
        output.replace_range(lo..hi, &replacement);
    }
    Ok(output)
}

fn rewrite_external_class_constructors(
    source: &str,
    classes: &[ClassConstructorRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{Expr, MemberProp, NewExpr};
    use thaw_parser::common::Spanned;

    if classes.is_empty() {
        return Ok(source.to_string());
    }
    struct Finder<'a> {
        classes: &'a [(String, String)],
        removals: Vec<(u32, u32)>,
    }
    impl Visit for Finder<'_> {
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let matches = match expression.callee.as_ref() {
                Expr::Ident(class) => self
                    .classes
                    .iter()
                    .any(|(_, name)| name == class.sym.as_str()),
                Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                    (Expr::Ident(package), MemberProp::Ident(class)) => {
                        self.classes.iter().any(|(qualifier, name)| {
                            qualifier == package.sym.as_str() && name == class.sym.as_str()
                        })
                    }
                    _ => false,
                },
                _ => false,
            };
            if matches {
                self.removals
                    .push((expression.span().lo.0, expression.callee.span().lo.0));
            }
            expression.visit_children_with(self);
        }
    }
    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder {
        classes,
        removals: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut output = source.to_string();
    for (start, end) in finder.removals.into_iter().rev() {
        output.replace_range((start - 1) as usize..(end - 1) as usize, "");
    }
    Ok(output)
}

/// Rewrites `pkg.name(...)` call expressions in a user's own `.ts` source
/// to the package-qualified alias identifier `generate_registry_shims`'s
/// collision resolution actually emits for a colliding name (e.g.
/// `qs.stringify(x)` -> `qs_stringify(x)`), so a user can keep writing
/// the familiar namespace-qualified form even though Thaw has no real
/// object/member-call support backing it -- this is resolved *entirely*
/// as source-level syntax sugar, at this preprocessing step, not by the
/// compiler. `rewrites` is empty when there were no collisions at all,
/// in which case this returns `source` untouched without even parsing it.
///
/// Uses `swc_ecma_visit`'s `Visit` to walk the whole AST (a call can be
/// nested arbitrarily deep in an expression), unlike the top-level-only
/// walks elsewhere in this project (thaw-registry also uses a full AST walk
/// for JavaScript dependency discovery). Matched
/// spans are collected first and applied as one pass of text
/// substitution over the original source afterward, copying everything
/// else verbatim -- this project carries no general JS/TS code
/// generator, so re-printing from the AST isn't an option.
fn rewrite_qualified_calls(
    source: &str,
    rewrites: &[QualifiedCallRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, MemberProp};
    use thaw_parser::common::Spanned;

    if rewrites.is_empty() {
        return Ok(source.to_string());
    }

    struct Finder<'a> {
        rewrites: &'a [(String, String, String)],
        matches: Vec<(u32, u32, String)>,
    }
    impl Visit for Finder<'_> {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = &**callee {
                    if let (Expr::Ident(obj), MemberProp::Ident(prop)) =
                        (&*member.obj, &member.prop)
                    {
                        if let Some((_, _, alias)) = self.rewrites.iter().find(|(pkg, name, _)| {
                            pkg.as_str() == &*obj.sym && name.as_str() == &*prop.sym
                        }) {
                            let span = member.span();
                            self.matches.push((span.lo.0, span.hi.0, alias.clone()));
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut finder = Finder {
        rewrites,
        matches: Vec::new(),
    };
    module.visit_with(&mut finder);

    if finder.matches.is_empty() {
        return Ok(source.to_string());
    }
    finder.matches.sort_by_key(|(lo, ..)| *lo);

    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, alias) in &finder.matches {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*hi))
            .pos
            .0 as usize;
        out.push_str(&source[cursor..lo]);
        out.push_str(alias);
        cursor = hi;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
struct FfiMetadata {
    error_abi: thaw_hir::FfiErrorAbi,
    return_ownership: thaw_hir::FfiOwnership,
    error_ownership: thaw_hir::FfiOwnership,
    param_string_abis: Option<Vec<thaw_hir::FfiStringAbi>>,
    return_string_abi: thaw_hir::FfiStringAbi,
    calling_convention: thaw_hir::FfiCallingConvention,
    aggregate_return_abi: thaw_hir::FfiAggregateAbi,
    aggregate_return_layout: Option<thaw_hir::FfiAggregateLayout>,
}

fn parse_string_abi(
    value: &str,
    symbol: &str,
    path: &Path,
) -> Result<thaw_hir::FfiStringAbi, String> {
    match value {
        "null-terminated" => Ok(thaw_hir::FfiStringAbi::NullTerminated),
        "pointer-length" => Ok(thaw_hir::FfiStringAbi::PointerLength),
        other => Err(format!(
            "unknown string ABI `{other}` for `{symbol}` in `{}`",
            path.display()
        )),
    }
}

fn parse_ffi_ownership(
    entry: &serde_json::Value,
    ownership_key: &str,
    destroy_key: &str,
    fallback_destroy_key: Option<&str>,
    symbol: &str,
    path: &Path,
) -> Result<thaw_hir::FfiOwnership, String> {
    let spelling = entry
        .get(ownership_key)
        .and_then(|value| value.as_str())
        .unwrap_or("borrowed");
    let destroy = entry
        .get(destroy_key)
        .or_else(|| fallback_destroy_key.and_then(|key| entry.get(key)))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    match spelling {
        "borrowed" => Ok(thaw_hir::FfiOwnership::Borrowed),
        "owned" => destroy
            .map(|destroy| thaw_hir::FfiOwnership::Owned { destroy })
            .ok_or_else(|| {
                format!(
                    "FFI metadata for `{symbol}` in `{}` needs `{destroy_key}` for `{ownership_key}: owned`",
                    path.display()
                )
            }),
        "arena-copy" => Ok(thaw_hir::FfiOwnership::ArenaCopy { destroy }),
        other => Err(format!(
            "unknown {ownership_key} `{other}` for `{symbol}` in `{}`",
            path.display()
        )),
    }
}

fn parse_ffi_aggregate_layout(
    value: &serde_json::Value,
    symbol: &str,
    path: &Path,
    root: bool,
) -> Result<thaw_hir::FfiAggregateLayout, String> {
    let object = value.as_object().ok_or_else(|| {
        format!(
            "FFI metadata for `{symbol}` in `{}` requires aggregate layouts to be objects",
            path.display()
        )
    })?;
    let field_offsets = object
        .get("fieldOffsets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires `fieldOffsets` to be an array",
                path.display()
            )
        })?
        .iter()
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                format!(
                    "FFI metadata for `{symbol}` in `{}` requires non-negative integer field offsets",
                    path.display()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let field_layouts = object
        .get("fieldLayouts")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` requires `fieldLayouts` to be an array",
                        path.display()
                    )
                })?
                .iter()
                .map(|value| {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        parse_ffi_aggregate_layout(value, symbol, path, false)
                            .map(Box::new)
                            .map(Some)
                    }
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![None; field_offsets.len()]);
    let size = object
        .get("size")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires an integer aggregate layout `size`",
                path.display()
            )
        })?;
    let alignment = object
        .get("alignment")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires a 32-bit integer aggregate layout `alignment`",
                path.display()
            )
        })?;
    let indirect = match object.get("indirect") {
        Some(value) => value.as_bool().ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires aggregate layout `indirect` to be a boolean",
                path.display()
            )
        })?,
        None if root => {
            return Err(format!(
                "FFI metadata for `{symbol}` in `{}` requires `aggregateReturnLayout.indirect`",
                path.display()
            ))
        }
        None => false,
    };
    Ok(thaw_hir::FfiAggregateLayout {
        field_offsets,
        field_layouts,
        size,
        alignment,
        indirect,
    })
}

fn read_ffi_metadata(
    paths: &[PathBuf],
) -> Result<std::collections::HashMap<String, FfiMetadata>, String> {
    let mut configured = std::collections::HashMap::new();
    for path in paths {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
        let document: serde_json::Value = serde_json::from_str(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", path.display()))?;
        let version = document.get("version").and_then(|value| value.as_u64());
        if !matches!(version, Some(1..=4)) {
            return Err(format!(
                "`{}` must declare FFI metadata version 1, 2, 3 or 4",
                path.display()
            ));
        }
        let functions = document
            .get("functions")
            .and_then(|value| value.as_object())
            .ok_or_else(|| format!("`{}` needs a `functions` object", path.display()))?;
        for (symbol, entry) in functions {
            let spelling = entry
                .get("errorAbi")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` needs `errorAbi`",
                        path.display()
                    )
                })?;
            let abi = match spelling {
                "direct" => thaw_hir::FfiErrorAbi::Direct,
                "thaw-result" => thaw_hir::FfiErrorAbi::ThawResult,
                other => {
                    return Err(format!(
                        "unknown errorAbi `{other}` for `{symbol}` in `{}`",
                        path.display()
                    ))
                }
            };
            let metadata = FfiMetadata {
                error_abi: abi,
                return_ownership: if matches!(version, Some(2..=4)) {
                    parse_ffi_ownership(
                        entry,
                        "returnOwnership",
                        "returnDestroy",
                        Some("destroy"),
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiOwnership::Borrowed
                },
                error_ownership: if matches!(version, Some(2..=4)) {
                    parse_ffi_ownership(
                        entry,
                        "errorOwnership",
                        "errorDestroy",
                        None,
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiOwnership::Borrowed
                },
                param_string_abis: if matches!(version, Some(3 | 4)) {
                    entry
                        .get("parameterStringAbis")
                        .map(|value| {
                            value
                                .as_array()
                                .ok_or_else(|| {
                                    format!(
                                        "FFI metadata for `{symbol}` in `{}` requires `parameterStringAbis` to be an array",
                                        path.display()
                                    )
                                })?
                                .iter()
                                .map(|value| {
                                    let value = value.as_str().ok_or_else(|| {
                                        format!(
                                            "FFI metadata for `{symbol}` in `{}` requires string ABI names",
                                            path.display()
                                        )
                                    })?;
                                    parse_string_abi(value, symbol, path)
                                })
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .transpose()?
                } else {
                    None
                },
                return_string_abi: if matches!(version, Some(3 | 4)) {
                    parse_string_abi(
                        entry
                            .get("returnStringAbi")
                            .and_then(|value| value.as_str())
                            .unwrap_or("null-terminated"),
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiStringAbi::NullTerminated
                },
                calling_convention: if matches!(version, Some(3 | 4)) {
                    match entry
                        .get("callingConvention")
                        .and_then(|value| value.as_str())
                        .unwrap_or("c")
                    {
                        "c" => thaw_hir::FfiCallingConvention::C,
                        "fast" => thaw_hir::FfiCallingConvention::Fast,
                        "cold" => thaw_hir::FfiCallingConvention::Cold,
                        other => {
                            return Err(format!(
                                "unknown calling convention `{other}` for `{symbol}` in `{}`",
                                path.display()
                            ))
                        }
                    }
                } else {
                    thaw_hir::FfiCallingConvention::C
                },
                aggregate_return_abi: if matches!(version, Some(3 | 4)) {
                    match entry
                        .get("aggregateReturnAbi")
                        .and_then(|value| value.as_str())
                        .unwrap_or("internal")
                    {
                        "internal" => thaw_hir::FfiAggregateAbi::Internal,
                        "portable" => thaw_hir::FfiAggregateAbi::Portable,
                        "packed" => thaw_hir::FfiAggregateAbi::Packed,
                        other => {
                            return Err(format!(
                                "unknown aggregate return ABI `{other}` for `{symbol}` in `{}`",
                                path.display()
                            ))
                        }
                    }
                } else {
                    thaw_hir::FfiAggregateAbi::Internal
                },
                aggregate_return_layout: if version == Some(4) {
                    entry
                        .get("aggregateReturnLayout")
                        .map(|value| parse_ffi_aggregate_layout(value, symbol, path, true))
                        .transpose()?
                } else {
                    None
                },
            };
            if let Some(previous) = configured.insert(symbol.clone(), metadata.clone()) {
                if previous != metadata {
                    return Err(format!("conflicting FFI metadata for `{symbol}`"));
                }
            }
        }
    }
    Ok(configured)
}

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
        mut qualified_call_rewrites,
        class_constructor_rewrites,
        class_method_rewrites,
        static_class_method_rewrites,
        class_getter_rewrites,
        class_setter_rewrites,
        static_class_getter_rewrites,
        static_class_setter_rewrites,
        external_exports,
    ) = generate_registry_shims(registry_dir, &resolved_packages, &user_source)?;
    let external_resolutions = registry_import_meta_resolutions(registry_dir, &resolved_packages);
    let use_qualifiers: std::collections::HashSet<&str> = use_packages
        .iter()
        .map(|package| qualifier_identifier(package))
        .collect();
    qualified_call_rewrites.retain(|(qualifier, _, _)| use_qualifiers.contains(qualifier.as_str()));
    // `qs.stringify(x)`-style calls, for a name that collided across two
    // `--use`d packages, only exist as source-level syntax sugar over the
    // package-qualified alias `generate_registry_shims` actually
    // generated -- see `rewrite_qualified_calls`'s doc comment. A no-op
    // (and no parse at all) when there were no collisions.
    let user_source = rewrite_qualified_calls(&user_source, &qualified_call_rewrites)?;
    let user_source = rewrite_external_class_methods_with_static(
        &user_source,
        &class_constructor_rewrites,
        &class_method_rewrites,
        &static_class_method_rewrites,
        &class_getter_rewrites,
        &class_setter_rewrites,
        &static_class_getter_rewrites,
        &static_class_setter_rewrites,
    )?;
    let user_source =
        rewrite_external_class_constructors(&user_source, &class_constructor_rewrites)?;
    let mut shim_source = registry_shim + &generate_bridge_shims(bridge_dts)?;
    if static_link && shim_source.contains("loadNativeAddonEmbedded(") {
        return Err(
            "--static cannot include an N-API addon: `.node` modules require the dynamic loader; use the package's JavaScript fallback or a static `native.a` backend"
                .into(),
        );
    }
    let manifest = serde_json::json!({
        "packages": resolved_packages,
        "quickjs": shim_source.contains("loadScript("),
        "napi": shim_source.contains("loadNativeAddonEmbedded("),
    });
    let marker = format!("{ARTIFACT_MARKER}{manifest}");
    let marker_literal = serde_json::to_string(&marker)
        .map_err(|error| format!("failed to encode artifact metadata: {error}"))?;
    shim_source.push_str(&format!(
        "function __thaw_artifact_metadata(): string {{ return {marker_literal}; }}\n"
    ));
    let mut module = module_graph::bundle(
        input,
        &user_source,
        &external_exports,
        &external_resolutions,
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
    }

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, input.to_string_lossy().as_ref());
    compiler.compile_program(&program)?;

    let obj_path = output.with_extension("o");
    compiler.write_object_file(&obj_path)?;

    // Always link the runtime support crates: thaw-arena backs array/object allocation
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
    let napi_lib = build_staticlib("thaw-napi")?;

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
        .arg(&quickjs_lib)
        .arg(&napi_lib)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detects_dynamic_interpreters_in_elf_outputs() {
        let dir = std::env::temp_dir().join(format!("thaw-cli-elf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app");
        let mut elf = vec![0_u8; 120];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
        elf[54..56].copy_from_slice(&56_u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1_u16.to_le_bytes());
        elf[64..68].copy_from_slice(&3_u32.to_le_bytes());
        std::fs::write(&path, &elf).unwrap();
        assert!(elf_has_program_interpreter(&path).unwrap());

        elf[64..68].copy_from_slice(&1_u32.to_le_bytes());
        std::fs::write(&path, &elf).unwrap();
        assert!(!elf_has_program_interpreter(&path).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn static_toolchain_check_is_actionable_or_complete() {
        if let Err(error) = ensure_static_system_libraries() {
            assert!(error.contains("--static"), "{error}");
            assert!(error.contains("glibc-static"), "{error}");
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn builds_and_runs_a_fully_static_elf() {
        if ensure_static_system_libraries().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("thaw-cli-static-elf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
        build_with_link_mode(
            &source,
            &output,
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            true,
        )
        .unwrap();
        assert!(!elf_has_program_interpreter(&output).unwrap());
        let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(manifest["packages"], serde_json::json!([]));
        assert_eq!(manifest["quickjs"], false);
        assert_eq!(manifest["napi"], false);
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn static_build_rejects_dynamic_napi_addons() {
        if ensure_static_system_libraries().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("thaw-cli-static-napi-{}", std::process::id()));
        let package = dir.join("registry/native-add");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "export declare function add(a: number, b: number): number;\n",
        )
        .unwrap();
        std::fs::write(
            package.join("native.node"),
            b"not loaded during compilation",
        )
        .unwrap();
        let source = dir.join("main.ts");
        std::fs::write(
            &source,
            "import { add } from \"native-add\"; function main(): void {}\n",
        )
        .unwrap();
        let error = build_with_link_mode(
            &source,
            &dir.join("app"),
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            true,
        )
        .unwrap_err();
        assert!(error.contains("N-API addon"), "{error}");
        assert!(error.contains("native.a"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn fully_static_binary_runs_in_an_isolated_container_when_enabled() {
        if std::env::var("THAW_RUN_CONTAINER_INTEGRATION").as_deref() != Ok("1")
            || ensure_static_system_libraries().is_err()
        {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-static-container-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
        build_with_link_mode(
            &source,
            &output,
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            true,
        )
        .unwrap();
        let result = Command::new("podman")
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--security-opt",
                "label=disable",
                "-v",
            ])
            .arg(format!("{}:/app:ro", output.display()))
            .args(["registry.fedoraproject.org/fedora:41", "/app"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builds_relative_typescript_module_graph_with_generics_and_aliases() {
        let dir =
            std::env::temp_dir().join(format!("thaw cli user modules {}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(
            dir.join("lib/pair.ts"),
            r#"
                export interface Pair<T, U> { first: T; second: U; }
                export function makePair<T, U>(first: T, second: U): Pair<T, U> {
                    return { first: first, second: second };
                }
                export function chooseFirst<T, U>(first: T, second: U): T {
                    return first;
                }
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/values.ts"),
            "export function value(): number { return 40; }\nexport default function(): number { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/index.ts"),
            "export { makePair, chooseFirst } from './pair';\nexport { value, default as offset } from './values';\nexport * as values from './values';\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/other.ts"),
            "export function value(): number { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/expression.ts"),
            "function offset(): number { return 2; }\nexport function moduleUrl(): string { return import.meta.url; }\nexport function moduleFilename(): string { return import.meta.filename; }\nexport function moduleDirname(): string { return import.meta.dirname; }\nexport function moduleMain(): boolean { return import.meta.main; }\nexport function resolvedUrl(): string { return import.meta.resolve((`../${'data '}` as string) + 'file.json?raw#part'); }\nexport default offset;\n",
        )
        .unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import { makePair, chooseFirst as first, value, values, offset as reexportedOffset } from "./lib";
                import offset, { value as sameValue } from "./lib/values";
                import { value as otherValue } from "./lib/other";
                import expressionOffset, { moduleUrl, moduleFilename, moduleDirname, moduleMain, resolvedUrl } from "./lib/expression";
                function main(): void {
                    const pair = makePair(value(), "ok");
                    console.log(pair.first + otherValue());
                    console.log(first("selected", sameValue() + offset()));
                    console.log(values.value() + values.default());
                    console.log(value() + reexportedOffset());
                    console.log(value() + expressionOffset());
                    console.log(moduleUrl());
                    console.log(moduleFilename());
                    console.log(moduleDirname());
                    console.log(moduleMain());
                    console.log(import.meta.main);
                    console.log(resolvedUrl());
                }
            "#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!(
                "42\nselected\n42\n42\n42\nfile://{}\n{}\n{}\nfalse\ntrue\nfile://{}/data%20file.json?raw#part\n",
                dir.join("lib/expression.ts")
                    .display()
                    .to_string()
                    .replace(' ', "%20"),
                dir.join("lib/expression.ts").display(),
                dir.join("lib").display(),
                dir.display().to_string().replace(' ', "%20")
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_and_reports_star_export_ambiguity() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-star-export-ambiguity-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("left.ts"),
            "export function left(): number { return 1; } export function shared(): number { return 10; }",
        )
        .unwrap();
        std::fs::write(
            dir.join("right.ts"),
            "export function right(): number { return 2; } export function shared(): number { return 20; }",
        )
        .unwrap();
        std::fs::write(
            dir.join("index.ts"),
            "export * from './left'; export * from './right'; export { shared } from './left';",
        )
        .unwrap();
        std::fs::write(dir.join("barrel.ts"), "export * from './index';").unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            "import { left, right, shared } from './barrel'; function main(): void { console.log(left() + right() + shared()); }",
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(result.status.success());
        assert_eq!(String::from_utf8_lossy(&result.stdout), "13\n");

        std::fs::write(
            dir.join("index.ts"),
            "export * from './left'; export * from './right';",
        )
        .unwrap();
        let source = std::fs::read_to_string(&entry).unwrap();
        let error = module_graph::bundle(
            &entry,
            &source,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(
            error.contains("ambiguous star export named `shared`"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_relative_typescript_import_cycles_with_the_full_chain() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-user-module-cycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.ts"),
            "import { b } from './b'; function main(): void { b(); }",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.ts"),
            "import { c } from './c'; export function b(): void { c(); }",
        )
        .unwrap();
        std::fs::write(
            dir.join("c.ts"),
            "import { b } from './b'; export function c(): void { b(); }",
        )
        .unwrap();
        let error = module_graph::bundle(
            &dir.join("main.ts"),
            &std::fs::read_to_string(dir.join("main.ts")).unwrap(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(error.contains("cyclic user-module import"));
        assert!(error.contains("b.ts"));
        assert!(error.contains("c.ts"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builds_and_runs_a_multifile_async_json_lambda_handler() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-user-module-lambda-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("transform.ts"),
            r#"
                interface PendingEvent { value: Promise<Json>; }
                async function delayed(event: Json): Promise<Json> {
                    await sleep(1);
                    return event;
                }
                export async function transform(event: Json): Promise<Json> {
                    const chained: Promise<Json> = new Promise<Json>((resolve, reject) => {
                        resolve(event);
                    }).then(value => delayed(value));
                    const pending: PendingEvent = { value: chained };
                    return await pending.value;
                }
            "#,
        )
        .unwrap();
        let entry = dir.join("handler.ts");
        std::fs::write(
            &entry,
            r#"
                import { transform } from "./transform";
                async function handler(event: Json): Promise<Json> {
                    return await transform(event);
                }
            "#,
        )
        .unwrap();
        let output = dir.join("bootstrap");
        build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = conn.read(&mut request).unwrap();
            let event = "{\"message\":\"module lambda\"}";
            conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: module-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    event.len(), event
                )
                .as_bytes(),
            )
            .unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request).into_owned())
                .unwrap();
            conn.write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });
        let mut child = Command::new(&output)
            .env("AWS_LAMBDA_RUNTIME_API", addr)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let request = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("multifile Lambda handler did not post a response");
        server.join().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(request.starts_with("POST /2018-06-01/runtime/invocation/module-request/response"));
        assert!(request.ends_with("{\"message\":\"module lambda\"}"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bare_imports_automatically_resolve_registry_packages() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-bare-imports-{}", std::process::id()));
        let registry = dir.join("modules");
        std::fs::create_dir_all(registry.join("math-kit")).unwrap();
        std::fs::write(
            registry.join("math-kit/package.d.ts"),
            "export interface Point { x: number; y: number; }\nexport declare function add(a: number, b: number): number;\nexport declare function sub(a: number, b: number): number;\nexport declare function greet(name: string): string;\nexport declare function negate(value: boolean): boolean;\nexport declare function echo(value: Json): Json;\nexport declare function sum(values: number[]): number;\nexport declare function reverse(values: number[]): number[];\nexport declare function shift(point: Point): Point;\nexport declare function fail(): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join("math-kit/bundle.js"),
            "module.exports = { add: function(a,b){ return a+b; }, sub: function(a,b){ return a-b; }, greet: function(name){ return 'hello ' + name; }, negate: function(value){ return !value; }, echo: function(value){ return value; }, sum: function(values){ return values.reduce(function(a,b){ return a+b; }, 0); }, reverse: function(values){ return values.reverse(); }, shift: function(point){ return { x: point.x + 1, y: point.y + 2 }; }, fail: function(){ throw new Error('typed dynamic failed'); } };\n",
        )
        .unwrap();
        std::fs::create_dir_all(registry.join("math-kit/subpaths/advanced")).unwrap();
        std::fs::write(
            registry.join("math-kit/subpaths/advanced/package.d.ts"),
            "export declare function square(value: number): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join("math-kit/subpaths/advanced/bundle.js"),
            "module.exports = { square: function(value){ return value * value; } };\n",
        )
        .unwrap();
        std::fs::create_dir_all(registry.join("twice")).unwrap();
        std::fs::write(
            registry.join("twice/package.d.ts"),
            "export default function twice(value: number): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join("twice/bundle.js"),
            "module.exports = function(value){ return value * 2; };\n",
        )
        .unwrap();

        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import { add, greet, negate, echo, sum, reverse, shift, fail } from "math-kit";
                import * as math from "math-kit";
                import { square } from "math-kit/advanced";
                import twice from "twice";
                function main(): void {
                    const sum: number = add(10, 11);
                    const difference: number = math.sub(13, 2);
                    console.log(twice(21));
                    console.log(sum + difference);
                    console.log(square(7));
                    console.log(greet("thaw"));
                    console.log(negate(false));
                    console.log(String(echo(JSON.parse("{\"ok\":true}")).ok));
                    console.log(sum([10, 20, 12]));
                    const reversed = reverse([1, 2, 3]);
                    console.log(reversed[0]);
                    const point = shift({ x: 3, y: 4 });
                    console.log(point.x * 10 + point.y);
                    console.log(import.meta.resolve("math-kit"));
                    console.log(import.meta.resolve("math-kit/advanced?raw"));
                    try {
                        const ignored = fail();
                    } catch (error) {
                        console.log(error);
                    }
                }
            "#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!(
                "42\n32\n49\nhello thaw\ntrue\ntrue\n42\n3\n46\nfile://{}\nfile://{}?raw\n`math-kit::fail` threw: typed dynamic failed\n",
                registry.join("math-kit/bundle.js").display(),
                registry
                    .join("math-kit/subpaths/advanced/bundle.js")
                    .display()
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn import_meta_resolution_prefers_the_selected_registry_backend() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-import-meta-backend-{}",
            std::process::id()
        ));
        let package = dir.join("pkg");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("bundle.js"), "module.exports = {};").unwrap();
        std::fs::write(package.join("native.node"), []).unwrap();
        let resolutions =
            registry_import_meta_resolutions(&dir, &["pkg".to_string(), "node:fs".to_string()]);
        assert_eq!(
            resolutions.get("pkg"),
            Some(&module_graph::file_url(&package.join("native.node")))
        );
        assert_eq!(
            resolutions.get("node:fs").map(String::as_str),
            Some("node:fs")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_bundle_uses_timers_microtasks_and_text_encoding() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-platform-globals-{}", std::process::id()));
        let registry = dir.join("modules");
        let package = registry.join("platform-work");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "export interface PendingValue { (): void; }\nexport declare function exercise(seed: number): PendingValue;\n",
        )
        .unwrap();
        std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { exercise: function() {
                return new Promise(resolve => {
                    const events = [];
                    const cancelled = setTimeout(() => events.push('cancelled'), 0);
                    clearTimeout(cancelled);
                    queueMicrotask(() => events.push('microtask'));
                    let ticks = 0;
                    const interval = setInterval(() => {
                        ticks++;
                        if (ticks === 2) {
                            clearInterval(interval);
                            const bytes = new TextEncoder().encode('雪');
                            const text = new TextDecoder().decode(bytes);
                            setTimeout(() => resolve(events.join(',') + ':' + ticks + ':' + text), 0);
                        }
                    }, 1);
                });
            } };"#,
        )
        .unwrap();
        let source = dir.join("main.ts");
        std::fs::write(
            &source,
            r#"import { exercise } from "platform-work";
                function main(): void {
                    const pending: JsValue = exercise(0);
                    console.log(String(readDynamicValue(pending)));
                    releaseDynamicValue(pending);
                }"#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "microtask:2:雪\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imports_supported_node_builtin_modules() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-node-imports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import * as path from "node:path";
                import { inspect } from "node:util";
                import { cwd } from "node:process";
                import { byteLength } from "node:buffer";
                function main(): void {
                    console.log(String(path.join(JSON.parse("[\"a\",\"b\"]"))));
                    console.log(String(inspect(JSON.parse("[42]"))));
                    console.log(String(cwd(JSON.parse("[]"))));
                    console.log(Number(byteLength(JSON.parse("[\"thaw\"]"))));
                }
            "#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
        let result = Command::new(output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "a/b\n42\n/\n4\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn node_fs_reads_and_writes_real_files_in_a_static_binary() {
        if ensure_static_system_libraries().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("thaw-cli-node-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data_dir = dir.join("data");
        let data_file = data_dir.join("message.txt");
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            format!(
                r#"
                    import {{ existsSync, readFileSync, writeFileSync, mkdirSync }} from "node:fs";
                    function main(): void {{
                        console.log(mkdirSync("{}"));
                        console.log(writeFileSync("{}", "hello from thaw"));
                        console.log(existsSync("{}"));
                        console.log(readFileSync("{}", "utf8"));
                    }}
                "#,
                data_dir.display(),
                data_file.display(),
                data_file.display(),
                data_file.display()
            ),
        )
        .unwrap();
        let output = dir.join("app");
        build_with_link_mode(
            &entry,
            &output,
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            true,
        )
        .unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "true\ntrue\ntrue\nhello from thaw\n"
        );
        assert_eq!(
            std::fs::read_to_string(data_file).unwrap(),
            "hello from thaw"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn node_http_serves_a_real_request_from_a_static_binary() {
        if ensure_static_system_libraries().is_err() {
            return;
        }
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let occupied_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied_port = occupied_listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-node-http-{}-{}",
            std::process::id(),
            port
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            format!(
                r#"
                    import {{ createServer }} from "node:http";
                    function main(): void {{
                        const prefix: string = "hello";
                        let requests: number = 0;
                        const server = createServer(
                            (
                                request: {{ method: string; url: string }},
                                response: {{
                                    statusCode: number;
                                    setHeader: (name: string, value: string) => boolean;
                                    write: (chunk: string) => boolean;
                                    end: (chunk: string) => boolean;
                                }}
                            ): boolean => {{
                                requests = requests + 1;
                                response.statusCode = 201;
                                response.setHeader("X-Thaw", request.method);
                                response.write(prefix);
                                return response.end(request.url);
                            }}
                        );
                        const target: string = server.listenMany({}, 2);
                        console.log(target);
                        console.log(requests);
                        console.log(server.close());
                        console.log(server.close());
                        server.on("listening", (): void => {{
                            console.log("event:listening");
                        }});
                        server.on("close", (): void => {{
                            console.log("event:close");
                        }});
                        server.on("error", (error: {{ message: string; code: string; syscall: string; address: string; port: number }}): void => {{
                            console.log(error.code);
                        }});
                        server.listen({}, (): void => {{
                            console.log("listening");
                        }});
                        console.log(server.close((): void => {{
                            console.log("closed");
                        }}));
                        server.listen(70000);
                        server.listen({});
                    }}
                "#,
                port, port, occupied_port
            ),
        )
        .unwrap();
        let executable = dir.join("app");
        build_with_link_mode(
            &entry,
            &executable,
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
            true,
        )
        .unwrap();
        assert!(!elf_has_program_interpreter(&executable).unwrap());

        let child = Command::new(&executable)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let request = |target: &str| {
            let mut stream = (0..200)
                .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => Some(stream),
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(5));
                        None
                    }
                })
                .expect("compiled HTTP server did not start listening");
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let response = request("/health");
        let second_response = request("/ready");
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(response.contains("X-Thaw: GET\r\n"));
        assert!(response.ends_with("hello/health"));
        assert!(second_response.ends_with("hello/ready"));
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "/ready\n2\ntrue\nfalse\ntrue\nevent:listening\nlistening\nERR_SOCKET_BAD_PORT\nEADDRINUSE\nevent:close\nclosed\n"
        );

        std::fs::write(
            &entry,
            r#"import { createServer } from "node:http";
            function main(): void {
                const server = createServer((
                    request: { method: string; url: string },
                    response: { statusCode: number; setHeader: (name: string, value: string) => boolean; write: (chunk: string) => boolean; end: (chunk: string) => boolean }
                ): boolean => true);
                server.listen(70000);
            }"#,
        )
        .unwrap();
        let unhandled = dir.join("unhandled-error");
        build_with_link_mode(
            &entry,
            &unhandled,
            &[],
            &[],
            &[],
            &dir.join("registry-unhandled"),
            &[],
            true,
        )
        .unwrap();
        let failed = Command::new(&unhandled).output().unwrap();
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("Unhandled 'error' event"));
        drop(occupied_listener);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bare_import_resolution_errors_include_source_location() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-missing-bare-import-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            "\n\nimport { missing } from \"not-installed\";\nfunction main(): void {}\n",
        )
        .unwrap();
        let error = build(
            &entry,
            &dir.join("app"),
            &[],
            &[],
            &[],
            &dir.join("registry"),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("main.ts:3:"), "{error}");
        assert!(error.contains("not-installed"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imports_a_scoped_package_subpath_with_default_and_namespace_forms() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-scoped-subpath-import-{}",
            std::process::id()
        ));
        let registry = dir.join("registry");
        let package = registry.join("@scope/tools/subpaths/feature");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "export default function double(value: number): number;\n",
        )
        .unwrap();
        std::fs::write(
            package.join("bundle.js"),
            "module.exports = function(value) { return value * 2; };\n",
        )
        .unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import double from "@scope/tools/feature";
                import * as feature from "@scope/tools/feature";
                function main(): void {
                    console.log(double(20));
                    console.log(feature.double(21));
                }
            "#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registers_builds_and_runs_an_installed_npm_wildcard_subpath() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-installed-wildcard-{}",
            std::process::id()
        ));
        let node_modules = dir.join("node_modules");
        let package = node_modules.join("feature-kit");
        std::fs::create_dir_all(package.join("dist/features")).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            package.join("index.d.ts"),
            "export declare function root(): number;\n",
        )
        .unwrap();
        std::fs::write(
            package.join("index.js"),
            "module.exports = { root: function() { return 1; } };\n",
        )
        .unwrap();
        std::fs::write(
            package.join("dist/features/triple.d.ts"),
            "export default function triple(value: number): number;\n",
        )
        .unwrap();
        std::fs::write(
            package.join("dist/features/triple.js"),
            "module.exports = function(value) { return value * 3; };\n",
        )
        .unwrap();
        let registry = dir.join("registry");
        thaw_registry::add_installed(&registry, &node_modules, "feature-kit").unwrap();

        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import triple from "feature-kit/features/triple";
                function main(): void { console.log(triple(14)); }
            "#,
        )
        .unwrap();
        let output = dir.join("app");
        build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_package_subpath_reports_the_import_location() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-missing-subpath-{}", std::process::id()));
        let registry = dir.join("registry");
        std::fs::create_dir_all(registry.join("math-kit")).unwrap();
        std::fs::write(
            registry.join("math-kit/package.d.ts"),
            "export declare function add(a: number, b: number): number;\n",
        )
        .unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            "\n\nimport { square } from \"math-kit/missing\";\nfunction main(): void {}\n",
        )
        .unwrap();
        let error = build(&entry, &dir.join("app"), &[], &[], &[], &registry, &[]).unwrap_err();
        assert!(error.contains("main.ts:3:"), "{error}");
        assert!(error.contains("math-kit/missing"), "{error}");
        assert!(error.contains("package.d.ts"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn runs_a_multifile_lambda_with_two_bare_import_packages() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-bare-import-lambda-{}",
            std::process::id()
        ));
        let registry = dir.join("modules");
        for (package, value) in [("left-mark", 20), ("right-mark", 22)] {
            std::fs::create_dir_all(registry.join(package)).unwrap();
            std::fs::write(
                registry.join(package).join("package.d.ts"),
                "export declare function mark(): number;\n",
            )
            .unwrap();
            std::fs::write(
                registry.join(package).join("bundle.js"),
                format!("module.exports = {{ mark: function() {{ return {value}; }} }};\n"),
            )
            .unwrap();
        }
        std::fs::write(
            dir.join("work.ts"),
            r#"
                import { mark as leftMark } from "left-mark";
                import { mark as rightMark } from "right-mark";
                export async function work(): Promise<void> {
                    await sleep(1);
                    console.log(leftMark() + rightMark());
                }
            "#,
        )
        .unwrap();
        let entry = dir.join("handler.ts");
        std::fs::write(
            &entry,
            r#"
                import { work } from "./work";
                async function handler(event: Json): Promise<Json> {
                    await work();
                    return event;
                }
            "#,
        )
        .unwrap();
        let output = dir.join("bootstrap");
        build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = conn.read(&mut request).unwrap();
            let event = "{\"packages\":2}";
            conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: packages-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    event.len(), event
                )
                .as_bytes(),
            )
            .unwrap();
            drop(conn);
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request).into_owned())
                .unwrap();
            conn.write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });
        let mut child = Command::new(&output)
            .env("AWS_LAMBDA_RUNTIME_API", addr)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let request = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("package Lambda handler did not post a response");
        server.join().unwrap();
        let _ = child.kill();
        let result = child.wait_with_output().unwrap();
        assert!(
            request.starts_with("POST /2018-06-01/runtime/invocation/packages-request/response")
        );
        assert!(request.ends_with("{\"packages\":2}"));
        assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn qualifier_identifier_passes_through_unscoped_names() {
        assert_eq!(qualifier_identifier("qs"), "qs");
        assert_eq!(qualifier_identifier("left-pad"), "left-pad");
    }

    #[test]
    fn qualifier_identifier_uses_the_last_segment_of_a_scoped_name() {
        assert_eq!(qualifier_identifier("@hapi/hoek"), "hoek");
        assert_eq!(qualifier_identifier("@babel/core"), "core");
    }

    #[test]
    fn sanitize_identifier_replaces_non_alphanumerics() {
        assert_eq!(sanitize_identifier("qs"), "qs");
        assert_eq!(sanitize_identifier("@hapi/hoek"), "_hapi_hoek");
        assert_eq!(sanitize_identifier("left-pad"), "left_pad");
    }

    #[test]
    fn reads_versioned_ffi_error_abi_metadata() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-ffi-metadata-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ffi.json");
        std::fs::write(
            &path,
            r#"{"version":1,"functions":{"externalRead":{"errorAbi":"thaw-result"}}}"#,
        )
        .unwrap();
        let metadata = read_ffi_metadata(&[path]).unwrap();
        assert_eq!(
            metadata["externalRead"],
            FfiMetadata {
                error_abi: thaw_hir::FfiErrorAbi::ThawResult,
                return_ownership: thaw_hir::FfiOwnership::Borrowed,
                error_ownership: thaw_hir::FfiOwnership::Borrowed,
                param_string_abis: None,
                return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
                calling_convention: thaw_hir::FfiCallingConvention::C,
                aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unknown_ffi_metadata_versions_and_abis() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-invalid-ffi-metadata-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let version = dir.join("version.json");
        std::fs::write(&version, r#"{"version":5,"functions":{}}"#).unwrap();
        assert!(read_ffi_metadata(&[version])
            .unwrap_err()
            .contains("version 1"));
        let abi = dir.join("abi.json");
        std::fs::write(
            &abi,
            r#"{"version":1,"functions":{"f":{"errorAbi":"errno"}}}"#,
        )
        .unwrap();
        assert!(read_ffi_metadata(&[abi])
            .unwrap_err()
            .contains("unknown errorAbi"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_version_two_ffi_ownership_metadata() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-ffi-ownership-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ffi.json");
        std::fs::write(
            &path,
            r#"{"version":2,"functions":{"read":{"errorAbi":"thaw-result","returnOwnership":"owned","destroy":"free_read","errorOwnership":"arena-copy","errorDestroy":"free_error"}}}"#,
        )
        .unwrap();
        let metadata = read_ffi_metadata(&[path]).unwrap();
        assert_eq!(
            metadata["read"],
            FfiMetadata {
                error_abi: thaw_hir::FfiErrorAbi::ThawResult,
                return_ownership: thaw_hir::FfiOwnership::Owned {
                    destroy: "free_read".into()
                },
                error_ownership: thaw_hir::FfiOwnership::ArenaCopy {
                    destroy: Some("free_error".into())
                },
                param_string_abis: None,
                return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
                calling_convention: thaw_hir::FfiCallingConvention::C,
                aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_version_three_string_abi_metadata() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-ffi-string-abi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ffi.json");
        std::fs::write(
            &path,
            r#"{"version":3,"functions":{"slice":{"errorAbi":"direct","parameterStringAbis":["pointer-length"],"returnStringAbi":"pointer-length","callingConvention":"fast","aggregateReturnAbi":"portable"},"record":{"errorAbi":"direct","aggregateReturnAbi":"packed"}}}"#,
        )
        .unwrap();
        let metadata = read_ffi_metadata(&[path]).unwrap();
        assert_eq!(
            metadata["slice"],
            FfiMetadata {
                error_abi: thaw_hir::FfiErrorAbi::Direct,
                return_ownership: thaw_hir::FfiOwnership::Borrowed,
                error_ownership: thaw_hir::FfiOwnership::Borrowed,
                param_string_abis: Some(vec![thaw_hir::FfiStringAbi::PointerLength]),
                return_string_abi: thaw_hir::FfiStringAbi::PointerLength,
                calling_convention: thaw_hir::FfiCallingConvention::Fast,
                aggregate_return_abi: thaw_hir::FfiAggregateAbi::Portable,
                aggregate_return_layout: None,
            }
        );
        assert_eq!(
            metadata["record"].aggregate_return_abi,
            thaw_hir::FfiAggregateAbi::Packed
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_version_four_explicit_aggregate_layout() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-ffi-aggregate-layout-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ffi.json");
        std::fs::write(
            &path,
            r#"{"version":4,"functions":{"record":{"errorAbi":"direct","aggregateReturnAbi":"portable","aggregateReturnLayout":{"fieldOffsets":[0,16],"fieldLayouts":[null,{"fieldOffsets":[0],"size":8,"alignment":8}],"size":32,"alignment":32,"indirect":true}}}}"#,
        )
        .unwrap();
        let metadata = read_ffi_metadata(&[path]).unwrap();
        assert_eq!(
            metadata["record"].aggregate_return_layout,
            Some(thaw_hir::FfiAggregateLayout {
                field_offsets: vec![0, 16],
                field_layouts: vec![
                    None,
                    Some(Box::new(thaw_hir::FfiAggregateLayout {
                        field_offsets: vec![0],
                        field_layouts: vec![None],
                        size: 8,
                        alignment: 8,
                        indirect: false,
                    })),
                ],
                size: 32,
                alignment: 32,
                indirect: true,
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_native_addon_builds_and_runs_end_to_end() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-native-addon-{}", std::process::id()));
        let registry = dir.join("modules");
        let package = registry.join("native-add");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "export declare function add(a: number, b: number): number;\n",
        )
        .unwrap();
        let addon_c = dir.join("addon.c");
        std::fs::write(&addon_c, r#"
            #include <stddef.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                (void)exports;
                napi_value fn; napi_create_function(env, "add", 3, add, 0, &fn);
                return fn;
            }
        "#).unwrap();
        assert!(Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(&addon_c)
            .arg("-o")
            .arg(package.join("native.node"))
            .status()
            .unwrap()
            .success());
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            "import { add } from \"native-add\"; function main(): void { console.log(add(20, 22)); }\n",
        )
        .unwrap();
        build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_native_addon_class_method_builds_and_runs_end_to_end() {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-native-addon-class-{}",
            std::process::id()
        ));
        let registry = dir.join("modules");
        let package = registry.join("native-box");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "export declare class NativeBox { constructor(value: number); static twice(value: number): number; static get version(): number; static set version(value: number); get value(): number; set value(value: number); get(): number; add(delta?: number): number; sum(...values: number[]): number; getLater(callback: (error: Json, result: Json) => void): number; }\n",
        )
        .unwrap();
        let addon_c = dir.join("addon.c");
        std::fs::write(
            &addon_c,
            r#"
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef struct { const char* utf8name; napi_value name; napi_callback method; napi_callback getter; napi_callback setter; napi_value value; unsigned attributes; void* data; } napi_property_descriptor;
            typedef struct { double value; } native_box;
            static double box_version = 1;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_null(napi_env, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            extern napi_status napi_wrap(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            extern napi_status napi_unwrap(napi_env, napi_value, void**);
            extern napi_status napi_define_class(napi_env, const char*, size_t, napi_callback, void*, size_t, const napi_property_descriptor*, napi_value*);
            static void finalize_box(napi_env env, void* data, void* hint) { (void)env; (void)hint; free(data); }
            static napi_value box_new(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_get_value_double(env, arg, &value);
                native_box* box = malloc(sizeof(*box)); box->value = value;
                napi_wrap(env, self, box, finalize_box, 0, 0); return self;
            }
            static napi_value box_get(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, result; native_box* box;
                napi_get_cb_info(env, info, &argc, 0, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_create_double(env, box->value, &result); return result;
            }
            static napi_value box_twice(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value);
                napi_create_double(env, value * 2, &result); return result;
            }
            static napi_value box_set(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; native_box* box; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_value_double(env, arg, &value); box->value = value; return arg;
            }
            static napi_value box_get_version(napi_env env, napi_callback_info info) {
                napi_value result; (void)info;
                napi_create_double(env, box_version, &result); return result;
            }
            static napi_value box_set_version(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value); box_version = value; return arg;
            }
            static napi_value box_add(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self, result; native_box* box; double delta = 0;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                if (argc == 1) napi_get_value_double(env, arg, &delta);
                napi_create_double(env, box->value + delta, &result); return result;
            }
            static napi_value box_sum(napi_env env, napi_callback_info info) {
                size_t argc = 8; napi_value args[8], self, result; double sum = 0, value;
                napi_get_cb_info(env, info, &argc, args, &self, 0);
                for (size_t index = 0; index < argc; index++) {
                    napi_get_value_double(env, args[index], &value); sum += value;
                }
                napi_create_double(env, sum, &result); return result;
            }
            static napi_value box_get_later(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, self, callback_args[2], ignored, queued; native_box* box;
                napi_get_cb_info(env, info, &argc, &callback, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_null(env, &callback_args[0]);
                napi_create_double(env, box->value, &callback_args[1]);
                napi_call_function(env, self, callback, 2, callback_args, &ignored);
                napi_create_double(env, 1, &queued); return queued;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value constructor;
                napi_property_descriptor properties[7] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "value", 0, 0, box_get, box_set, 0, 0, 0 },
                    { "add", 0, box_add, 0, 0, 0, 0, 0 },
                    { "sum", 0, box_sum, 0, 0, 0, 0, 0 },
                    { "getLater", 0, box_get_later, 0, 0, 0, 0, 0 },
                    { "twice", 0, box_twice, 0, 0, 0, 1024, 0 },
                    { "version", 0, 0, box_get_version, box_set_version, 0, 1024, 0 }
                };
                napi_define_class(env, "NativeBox", 9, box_new, 0, 7, properties, &constructor);
                napi_set_named_property(env, exports, "NativeBox", constructor);
                return exports;
            }
        "#,
        )
        .unwrap();
        assert!(Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(&addon_c)
            .arg("-o")
            .arg(package.join("native.node"))
            .status()
            .unwrap()
            .success());
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            "import { NativeBox } from \"native-box\"; function main(): void { console.log(NativeBox.twice(21)); console.log(NativeBox.version); console.log(NativeBox.version = 3); console.log(NativeBox.version); const box: JsValue = new NativeBox(42); console.log(box.value); console.log(box.value = 10); console.log(box.value); console.log(box.get()); console.log(box.add()); console.log(box.add(8)); console.log(box.sum()); console.log(box.sum(1, 2, 3)); const callback = (error: Json, result: Json): void => { console.log(Number(result)); }; console.log(box.getLater(callback)); }\n",
        )
        .unwrap();
        build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "42\n1\n3\n3\n42\n10\n10\n10\n10\n18\n0\n6\n10\n1\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_runs_utf8_validate_prebuild_when_supplied() {
        let Ok(prebuild) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
            return;
        };
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-utf8-validate-{}", std::process::id()));
        let registry = dir.join("modules");
        let package = registry.join("utf-8-validate");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "declare function isValidUTF8(buffer: any): boolean;\n",
        )
        .unwrap();
        std::fs::copy(prebuild, package.join("native.node")).unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["utf-8-validate".into()],
        )
        .unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_runs_bcrypt_async_callbacks_when_supplied() {
        let Ok(prebuild) = std::env::var("THAW_BCRYPT_NODE") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("thaw-cli-bcrypt-{}", std::process::id()));
        let registry = dir.join("modules");
        let package = registry.join("bcrypt");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.d.ts"),
            "declare function gen_salt(argsArray: Json): Json;\n\
             declare function encrypt(argsArray: Json): Json;\n\
             declare function compare(argsArray: Json): Json;\n",
        )
        .unwrap();
        std::fs::copy(prebuild, package.join("native.node")).unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            r#"function main(): void {
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["bcrypt".into()],
        )
        .unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.lines().any(|line| line.starts_with("$2b$04$")),
            "{stdout}"
        );
        assert!(stdout.lines().any(|line| line == "false"), "{stdout}");
        assert!(
            stdout.lines().any(|line| line.contains("Invalid salt")),
            "{stdout}"
        );
        assert_eq!(stdout.lines().count(), 4, "{stdout}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_fetches_and_runs_bcrypt_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("thaw-cli-auto-bcrypt-{}", std::process::id()));
        let registry = dir.join("modules");
        let added = thaw_registry::add(&registry, "bcrypt@6.0.0").unwrap();
        let native = added.native_addon.expect("a matching prebuild is bundled");
        assert_eq!(native.platform, "linux");
        assert_eq!(native.arch, "x64");
        assert_eq!(native.libc, "glibc");
        assert!(registry.join("bcrypt/native.node").is_file());
        assert!(registry.join("bcrypt/native-addon.json").is_file());

        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            r#"function main(): void {
                const salt: Json = callNativeAddon(
                    "gen_salt_sync",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]")
                );
                console.log(String(salt));
                const hash: Json = callNativeAddon(
                    "encrypt_sync",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                );
                console.log(String(hash));
                console.log(Boolean(callNativeAddon(
                    "compare_sync",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                )));
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["bcrypt".into()],
        )
        .unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        let lines = stdout.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 7, "{stdout}");
        assert!(lines[0].starts_with("$2b$04$"), "{stdout}");
        assert!(lines[1].starts_with("$2b$04$"), "{stdout}");
        assert_eq!(lines[2], "false", "{stdout}");
        assert!(
            lines[3..].iter().any(|line| line.starts_with("$2b$04$")),
            "{stdout}"
        );
        assert!(lines[3..].contains(&"false"), "{stdout}");
        assert!(
            lines[3..].iter().any(|line| line.contains("Invalid salt")),
            "{stdout}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_fetches_and_loads_sqlite3_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-auto-sqlite3-{}", std::process::id()));
        let registry = dir.join("modules");
        let added = thaw_registry::add(&registry, "sqlite3@5.1.7").unwrap();
        let native = added
            .native_addon
            .expect("the official GitHub prebuild should be downloaded");
        assert_eq!(native.platform, "linux");
        assert_eq!(native.arch, "x64");
        assert_eq!(native.libc, "glibc");
        assert!(native.source.contains("TryGhost/node-sqlite3/releases"));

        // Keep this fixture focused on construction and one error-first
        // callback method rather than sqlite3's full overloaded surface.
        std::fs::write(
            registry.join("sqlite3/package.d.ts"),
            "export declare class Database { constructor(filename: string); close(callback: (error: Error | null) => void): void; }\n",
        )
        .unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            "import { Database } from \"sqlite3\";\n\
             function main(): void {\n\
                 const database: JsValue = new Database(\":memory:\");\n\
                 const onClose = (error: Json): void => { console.log(\"sqlite3-closed\"); };\n\
                 database.close(onClose);\n\
                 console.log(\"sqlite3-constructed\");\n\
             }\n",
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["sqlite3".into()],
        )
        .unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "sqlite3-constructed\nsqlite3-closed\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_fetches_and_runs_utf8_validate_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-auto-utf8-validate-{}",
            std::process::id()
        ));
        let registry = dir.join("modules");
        let added = thaw_registry::add(&registry, "utf-8-validate@6.0.6").unwrap();
        let native = added.native_addon.expect("a matching prebuild is bundled");
        assert_eq!(native.platform, "linux");
        assert_eq!(native.arch, "x64");
        assert_eq!(native.libc, "glibc");
        assert!(registry.join("utf-8-validate/native.node").is_file());
        assert!(registry.join("utf-8-validate/native-addon.json").is_file());

        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["utf-8-validate".into()],
        )
        .unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_runs_real_p_limit_promise_workload_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-auto-p-limit-{}", std::process::id()));
        let registry = dir.join("modules");
        let added = thaw_registry::add(&registry, "p-limit@2.3.0").unwrap();
        assert_eq!(
            added.dependency_versions.get("p-try").map(String::as_str),
            Some("2.2.0")
        );
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &source,
            r#"import pLimit from "p-limit";
                function main(): void {
                    const limit: JsValue = pLimit(2);
                    const task: JsValue = getDynamicValue("Number");
                    const result: Json = callDynamicValueWithValue(limit, task);
                    console.log(Number(result));
                }"#,
        )
        .unwrap();
        build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_builds_and_runs_a_real_esm_package_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-auto-has-flag-{}", std::process::id()));
        let registry = dir.join("modules");
        thaw_registry::add(&registry, "has-flag@5.0.1").unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &source,
            r#"import hasFlag from "has-flag";
                function main(): void {
                    const present: Json = hasFlag(JSON.parse("[\"--thaw-parser-backed-bundler\"]"));
                    console.log(Boolean(present));
                }"#,
        )
        .unwrap();
        build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "false\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_add_fetches_and_runs_parcel_watcher_when_enabled() {
        if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-auto-parcel-watcher-{}",
            std::process::id()
        ));
        let registry = dir.join("modules");
        let watched = dir.join("watched");
        let snapshot = dir.join("snapshot.bin");
        std::fs::create_dir_all(&watched).unwrap();
        std::fs::write(watched.join("before.txt"), "before").unwrap();
        let added = thaw_registry::add(&registry, "@parcel/watcher@2.5.1").unwrap();
        let native = added
            .native_addon
            .expect("the platform optional dependency contains a prebuild");
        assert_eq!(native.platform, "linux");
        assert_eq!(native.arch, "x64");
        assert_eq!(native.libc, "glibc");
        assert!(native
            .source
            .contains("watcher-linux-x64-glibc/watcher.node"));

        let args = serde_json::to_string(&serde_json::json!([
            watched.to_string_lossy(),
            snapshot.to_string_lossy(),
            {}
        ]))
        .unwrap();
        let args_literal = serde_json::to_string(&args).unwrap();
        let source = dir.join("main.ts");
        let output = dir.join("app");
        std::fs::write(
            &source,
            format!(
                "function main(): void {{ const result: Json = writeSnapshot(JSON.parse({args_literal})); console.log(\"snapshot-created\"); }}\n"
            ),
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["@parcel/watcher".into()],
        )
        .unwrap();
        let event_file = watched.join("event.txt");
        let subscribe_args =
            serde_json::to_string(&serde_json::json!([watched.to_string_lossy()])).unwrap();
        let subscribe_literal = serde_json::to_string(&subscribe_args).unwrap();
        let event_source = dir.join("events.ts");
        let event_output = dir.join("events-app");
        std::fs::write(
            &event_source,
            format!(
                r#"import * as fs from "node:fs";
                function main(): void {{
                    let received: number = 0;
                    const callback = (error: Json, result: Json): Json => {{
                        received = 1;
                        return result;
                    }};
                    const subscribed: Json = callNativeAddonWithCallback("subscribe", JSON.parse({subscribe_literal}), callback);
                    fs.writeFileSync("{}", "event");
                    while (received < 1) {{
                        const count: number = pollNativeAddonEvents();
                    }}
                    const unsubscribed: Json = callNativeAddonWithCallback("unsubscribe", JSON.parse({subscribe_literal}), callback);
                    console.log("watch-event");
                }}
                "#,
                event_file.to_string_lossy()
            ),
        )
        .unwrap();
        build(
            &event_source,
            &event_output,
            &[],
            &[],
            &[],
            &registry,
            &["@parcel/watcher".into()],
        )
        .unwrap();
        std::fs::remove_dir_all(&registry).unwrap();
        let result = Command::new(&output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "snapshot-created\n"
        );
        assert!(snapshot.is_file());
        let result = Command::new(&event_output).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), "watch-event\n");
        assert!(event_file.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rewrite_qualified_calls_is_a_no_op_with_no_rewrites() {
        let source = "function main(): void { console.log(qs.stringify(x)); }";
        assert_eq!(rewrite_qualified_calls(source, &[]).unwrap(), source);
    }

    #[test]
    fn rewrite_qualified_calls_replaces_matching_qualified_calls_only() {
        let source = "function main(): void {\n\
             console.log(qs.stringify(x));\n\
             console.log(hoek.stringify(y));\n\
             console.log(qs.parse(z));\n\
             console.log(unrelated.stringify(w));\n\
         }";
        let rewrites = vec![
            (
                "qs".to_string(),
                "stringify".to_string(),
                "qs_stringify".to_string(),
            ),
            (
                "hoek".to_string(),
                "stringify".to_string(),
                "hoek_stringify".to_string(),
            ),
        ];
        let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();

        assert!(rewritten.contains("console.log(qs_stringify(x));"));
        assert!(rewritten.contains("console.log(hoek_stringify(y));"));
        // Not in `rewrites` (no collision for `parse`, or the object
        // isn't a known qualifier at all) -- left completely alone.
        assert!(rewritten.contains("console.log(qs.parse(z));"));
        assert!(rewritten.contains("console.log(unrelated.stringify(w));"));
    }

    #[test]
    fn rewrite_qualified_calls_handles_a_call_nested_in_an_expression() {
        let source = "function main(): void { const r = String(qs.stringify(x)); }";
        let rewrites = vec![(
            "qs".to_string(),
            "stringify".to_string(),
            "qs_stringify".to_string(),
        )];
        let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();
        assert!(rewritten.contains("const r = String(qs_stringify(x));"));
    }

    #[test]
    fn rewrites_external_class_constructors_without_touching_other_new_expressions() {
        let source = "const a = new Database(\":memory:\"); const b = new sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);";
        let rewritten =
            rewrite_external_class_constructors(source, &[("sqlite3".into(), "Database".into())])
                .unwrap();
        assert_eq!(
            rewritten,
            "const a = Database(\":memory:\"); const b = sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);"
        );
    }

    #[test]
    fn rewrites_methods_on_values_created_from_external_classes() {
        let source = "const db = new Database(\":memory:\"); db.configure(\"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("sqlite3".into(), "Database".into())],
            &[(
                "Database".into(),
                "configure".into(),
                "__thaw_configure".into(),
                2,
                false,
                vec![thaw_hir::HirType::Str, thaw_hir::HirType::F64],
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const db = new Database(\":memory:\"); __thaw_configure(db, \"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);"
        );
    }

    #[test]
    fn rewrites_named_and_namespace_static_class_methods() {
        let source = "NativeBox.create(1); addon.NativeBox.create(\"text\"); LocalBox.create(2);";
        let rewritten = rewrite_external_class_methods_with_static(
            source,
            &[],
            &[],
            &[
                (
                    "addon".into(),
                    "NativeBox".into(),
                    "create".into(),
                    "__thaw_create_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "addon".into(),
                    "NativeBox".into(),
                    "create".into(),
                    "__thaw_create_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "__thaw_create_number(1); __thaw_create_string(\"text\"); LocalBox.create(2);"
        );
    }

    #[test]
    fn rewrites_typed_napi_instance_getters() {
        let source = "const box = new NativeBox(42); console.log(box.value);";
        let rewritten = rewrite_external_class_methods_with_static(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[],
            &[],
            &[(
                "NativeBox".into(),
                "value".into(),
                "__thaw_get_value".into(),
            )],
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); console.log(__thaw_get_value(box));"
        );
    }

    #[test]
    fn rewrites_typed_napi_instance_setters_and_preserves_expression_values() {
        let source = "const box = new NativeBox(42); const assigned: number = box.value = 7;";
        let rewritten = rewrite_external_class_methods_with_static(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[],
            &[],
            &[],
            &[(
                "NativeBox".into(),
                "value".into(),
                "__thaw_set_value".into(),
                thaw_hir::HirType::F64,
            )],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const assigned: number = __thaw_set_value(box, 7);"
        );
    }

    #[test]
    fn rewrites_named_and_namespace_static_accessors() {
        let source = "console.log(NativeBox.version); addon.NativeBox.version = 7; console.log(addon.NativeBox.version);";
        let rewritten = rewrite_external_class_methods_with_static(
            source,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[(
                "addon".into(),
                "NativeBox".into(),
                "version".into(),
                "__thaw_get_version".into(),
            )],
            &[(
                "addon".into(),
                "NativeBox".into(),
                "version".into(),
                "__thaw_set_version".into(),
                thaw_hir::HirType::F64,
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "console.log(__thaw_get_version()); __thaw_set_version(7); console.log(__thaw_get_version());"
        );
    }

    #[test]
    fn generates_typed_napi_static_method_shims_without_instance_receivers() {
        let class = thaw_bridge::DtsClass {
            name: "NativeBox".into(),
            extends: None,
            constructors: vec![],
            methods: vec![thaw_bridge::DtsMethod {
                name: "create".into(),
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                rest_param: None,
                ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                is_static: true,
                kind: thaw_bridge::DtsMethodKind::Method,
                overloaded: false,
            }],
            properties: vec![],
        };
        let mut shim = String::new();
        let generated = generate_napi_class_method_overloads(
            &class,
            true,
            &std::collections::HashMap::new(),
            &mut shim,
        );
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].0, "create");
        assert!(shim.contains("(value: number): number;"));
        assert!(!shim.contains("receiver"));
    }

    #[test]
    fn rewrites_zero_argument_external_class_methods() {
        let source = "const box = new NativeBox(42); const value = box.get();";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[(
                "NativeBox".into(),
                "get".into(),
                "__thaw_get".into(),
                0,
                false,
                vec![],
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const value = __thaw_get(box);"
        );
    }

    #[test]
    fn tracks_external_class_instance_aliases_and_invalidates_reassignments() {
        let source = "const box = new NativeBox(42); const alias = box; alias.get(); let assigned = alias; assigned.get(); assigned = box; assigned.get(); assigned = unknown; assigned.get();";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[(
                "NativeBox".into(),
                "get".into(),
                "__thaw_get".into(),
                0,
                false,
                vec![],
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const alias = box; __thaw_get(alias); let assigned = alias; __thaw_get(assigned); assigned = box; __thaw_get(assigned); assigned = unknown; assigned.get();"
        );
    }

    #[test]
    fn tracks_external_class_instances_through_object_properties() {
        let source = "const box = new NativeBox(42); const holder = { box }; holder.box.get(); holder[\"box\"].get(); const nested = { inner: { value: new NativeBox(7) } }; nested.inner.value.get(); holder.box = box; holder.box.get(); holder.box = unknown; holder.box.get();";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[(
                "NativeBox".into(),
                "get".into(),
                "__thaw_get".into(),
                0,
                false,
                vec![],
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const holder = { box }; __thaw_get(holder.box); __thaw_get(holder[\"box\"]); const nested = { inner: { value: new NativeBox(7) } }; __thaw_get(nested.inner.value); holder.box = box; __thaw_get(holder.box); holder.box = unknown; holder.box.get();"
        );
    }

    #[test]
    fn joins_object_property_instance_facts_across_branches() {
        let source = "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } holder.box.get(); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[(
                "NativeBox".into(),
                "get".into(),
                "__thaw_get".into(),
                0,
                false,
                vec![],
            )],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } __thaw_get(holder.box); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();"
        );
    }

    #[test]
    fn selects_external_method_overloads_by_arity_and_callback_shape() {
        let source = "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; db.run(\"select 1\"); db.run(\"select 1\", done);";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("sqlite3".into(), "Database".into())],
            &[
                (
                    "Database".into(),
                    "run".into(),
                    "__run_sync".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
                (
                    "Database".into(),
                    "run".into(),
                    "__run_callback".into(),
                    2,
                    true,
                    vec![
                        thaw_hir::HirType::Str,
                        thaw_hir::HirType::Function(
                            vec![thaw_hir::HirType::Json],
                            Box::new(thaw_hir::HirType::Void),
                        ),
                    ],
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; __run_sync(db, \"select 1\"); __run_callback(db, \"select 1\", done);"
        );
    }

    #[test]
    fn selects_same_arity_external_method_overloads_by_argument_type() {
        let source = "const box = new NativeBox(1); const n = 42; const s = \"hello\"; box.set(n); box.set(s); box.set(7); box.set(\"world\");";
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            rewritten,
            "const box = new NativeBox(1); const n = 42; const s = \"hello\"; __set_number(box, n); __set_string(box, s); __set_number(box, 7); __set_string(box, \"world\");"
        );
    }

    #[test]
    fn infers_external_overload_types_from_composed_expressions() {
        let source = r#"const box = new NativeBox(1); const n = 20 + 22; const s = "hel" + "lo"; const b = n > 0; const config = { n, nested: { text: s }, enabled: b }; box.set(n); box.set(s); box.set(b); box.set(Number("7")); box.set(`value-${s}`); box.set(true ? "yes" : "no"); box.set(config.n); box.set(config.nested.text); box.set(config.enabled); box.set(({ value: 7 }).value);"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_boolean".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Bool],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__set_number(box, n)"));
        assert!(rewritten.contains("__set_string(box, s)"));
        assert!(rewritten.contains("__set_boolean(box, b)"));
        assert!(rewritten.contains("__set_number(box, Number(\"7\"))"));
        assert!(rewritten.contains("__set_string(box, `value-${s}`)"));
        assert!(rewritten.contains("__set_string(box, true ? \"yes\" : \"no\")"));
        assert!(rewritten.contains("__set_number(box, config.n)"));
        assert!(rewritten.contains("__set_string(box, config.nested.text)"));
        assert!(rewritten.contains("__set_boolean(box, config.enabled)"));
        assert!(rewritten.contains("__set_number(box, ({ value: 7 }).value)"));
    }

    #[test]
    fn infers_external_overload_types_from_user_function_returns_and_forward_references() {
        let source = r#"const box = new NativeBox(1); const makeText = (): string => "text"; const makeFlag = function(): boolean { return true; }; const inferredFlag = () => true; box.set(makeNumber()); box.set(makeText()); box.set(makeFlag()); box.set(inferredNumber()); box.set(inferredText()); box.set(inferredFlag()); function makeNumber(): number { return 42; } function inferredNumber() { return 40 + 2; } function inferredText() { return forwardText(); } function forwardText() { return "text"; }"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_boolean".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Bool],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__set_number(box, makeNumber())"));
        assert!(rewritten.contains("__set_string(box, makeText())"));
        assert!(rewritten.contains("__set_boolean(box, makeFlag())"));
        assert!(rewritten.contains("__set_number(box, inferredNumber())"));
        assert!(rewritten.contains("__set_string(box, inferredText())"));
        assert!(rewritten.contains("__set_boolean(box, inferredFlag())"));
    }

    #[test]
    fn selects_object_overloads_from_structural_property_types() {
        let source = r#"function makeNumeric(): { value: number } { return { value: 11 }; } const box = new NativeBox(1); const numeric = { value: 42 }; const textual = { value: "text" }; const choose = true; box.configure(numeric); box.configure(textual); box.configure({ value: 7 }); box.configure({ ["value"]: "computed" }); box.configure({ ...numeric }); box.configure({ ...numeric, value: "override" }); box.configure({ ...{ value: 9 } }); box.configure({ ...makeNumeric() }); box.configure({ ...(choose ? makeNumeric() : numeric) });"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "configure".into(),
                    "__configure_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Object(vec![(
                        "value".into(),
                        thaw_hir::HirType::F64,
                    )])],
                ),
                (
                    "NativeBox".into(),
                    "configure".into(),
                    "__configure_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Object(vec![(
                        "value".into(),
                        thaw_hir::HirType::Str,
                    )])],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__configure_number(box, numeric)"));
        assert!(rewritten.contains("__configure_string(box, textual)"));
        assert!(rewritten.contains("__configure_number(box, { value: 7 })"));
        assert!(rewritten.contains("__configure_string(box, { [\"value\"]: \"computed\" })"));
        assert!(rewritten.contains("__configure_number(box, { ...numeric })"));
        assert!(rewritten.contains("__configure_string(box, { ...numeric, value: \"override\" })"));
        assert!(rewritten.contains("__configure_number(box, { ...{ value: 9 } })"));
        assert!(rewritten.contains("__configure_number(box, { ...makeNumeric() })"));
        assert!(rewritten
            .contains("__configure_number(box, { ...(choose ? makeNumeric() : numeric) })"));
    }

    #[test]
    fn tracks_assignment_flow_for_variables_and_nested_object_properties() {
        let source = r#"const box = new NativeBox(1); let value = 42; box.set(value); value = "text"; box.set(value); const config = { nested: { value: 1 }, direct: true }; config.nested.value = "nested"; config["direct"] = 7; box.set(config.nested.value); box.set(config.direct);"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__set_number(box, value); value = \"text\""));
        assert!(rewritten.contains("__set_string(box, value); const config"));
        assert!(rewritten.contains("__set_string(box, config.nested.value)"));
        assert!(rewritten.contains("__set_number(box, config.direct)"));
    }

    #[test]
    fn joins_if_branch_types_and_discards_conflicting_facts() {
        let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; if (flag) { stable = 2; } else { stable = 3; } box.set(stable); let conflict = 1; if (flag) { conflict = "text"; box.set(conflict); } else { conflict = 2; box.set(conflict); } box.set(conflict); let oneSided = 1; if (flag) { oneSided = "changed"; } box.set(oneSided);"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_unknown".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Bool],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__set_number(box, stable)"));
        assert!(rewritten.contains("conflict = \"text\"; __set_string(box, conflict)"));
        assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
        assert!(rewritten.contains("} __set_unknown(box, conflict)"));
        assert!(rewritten.contains("} __set_unknown(box, oneSided)"));
    }

    #[test]
    fn joins_while_and_for_types_against_the_zero_iteration_path() {
        let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; while (flag) { stable = 2; box.set(stable); break; } box.set(stable); let changed = 1; while (flag) { changed = "text"; box.set(changed); break; } box.set(changed); let loopValue = 1; for (let index = 0; index < 1; index = index + 1) { box.set(index); loopValue = "loop"; box.set(loopValue); } box.set(loopValue);"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_unknown".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Bool],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("stable = 2; __set_number(box, stable)"));
        assert!(rewritten.contains("} __set_number(box, stable)"));
        assert!(rewritten.contains("changed = \"text\"; __set_string(box, changed)"));
        assert!(rewritten.contains("} __set_unknown(box, changed)"));
        assert!(rewritten.contains("__set_number(box, index)"));
        assert!(rewritten.contains("loopValue = \"loop\"; __set_string(box, loopValue)"));
        assert!(rewritten.contains("} __set_unknown(box, loopValue)"));
    }

    #[test]
    fn joins_try_catch_paths_and_applies_finally_to_every_exit() {
        let source = r#"const box = new NativeBox(1); let stable = 1; try { stable = 2; } catch (error) { stable = 3; } box.set(stable); let conflict = 1; try { conflict = "try"; box.set(conflict); } catch (error) { conflict = 2; box.set(conflict); } box.set(conflict); let catchInput = 1; try { catchInput = "changed"; throw "fail"; } catch (error) { box.set(catchInput); } let finalized = 1; try { finalized = "try"; } catch (error) { finalized = true; } finally { finalized = 7; box.set(finalized); } box.set(finalized);"#;
        let rewritten = rewrite_external_class_methods(
            source,
            &[("addon".into(), "NativeBox".into())],
            &[
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_unknown".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Bool],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_number".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::F64],
                ),
                (
                    "NativeBox".into(),
                    "set".into(),
                    "__set_string".into(),
                    1,
                    false,
                    vec![thaw_hir::HirType::Str],
                ),
            ],
        )
        .unwrap();
        assert!(rewritten.contains("__set_number(box, stable)"));
        assert!(rewritten.contains("conflict = \"try\"; __set_string(box, conflict)"));
        assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
        assert!(rewritten.contains("} __set_unknown(box, conflict)"));
        assert!(rewritten.contains("catch (error) { __set_unknown(box, catchInput)"));
        assert!(rewritten.contains("finalized = 7; __set_number(box, finalized)"));
        assert!(rewritten.ends_with("__set_number(box, finalized);"));
    }
}
