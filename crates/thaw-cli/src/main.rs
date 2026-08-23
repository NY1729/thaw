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
        _ => {
            eprintln!(
                "usage: thaw build <input.ts> [-o <output>] [--link <path>]... [--bridge <path.d.ts>]... [--ffi-metadata <path.json>]... [--registry <dir>] [--use <package>]...\n       thaw registry add <package>[@<version>] [--registry <dir>]"
            );
            std::process::exit(1);
        }
    }
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
        &ffi_metadata,
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

/// `(package, name, alias)` -- see `rewrite_qualified_calls`. `package`
/// here is the *qualifier identifier* (`qualifier_identifier`), not
/// necessarily the real package name.
type QualifiedCallRewrite = (String, String, String);

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
) -> Result<(String, Vec<PathBuf>, Vec<QualifiedCallRewrite>), String> {
    let mut resolved = Vec::new();
    for name in use_packages {
        let package = thaw_registry::resolve(registry_dir, name)?;
        let functions = thaw_bridge::parse_dts(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts: {e}"))?;
        // Whether there's actually a `native.a` to link a FastPath
        // signature against -- without one, a fully-primitive real npm
        // function (e.g. date-fns's `daysToWeeks(days: number): number`)
        // would still classify FastPath on type shape alone and produce
        // an unresolvable `declare function`, even though a working JS
        // implementation is sitting right there in `bundle.js`. See
        // `thaw_bridge::effective_classifications`'s doc comment.
        let native_lib_available = package.native_lib.is_some();
        let classifications =
            thaw_bridge::effective_classifications(&functions, native_lib_available);
        resolved.push(ResolvedPackage {
            name: package.name.clone(),
            functions,
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
    let mut native_libs = Vec::new();
    let mut bundles: Vec<PendingBundle> = Vec::new();
    let mut native_addons: Vec<(String, String, Option<String>)> = Vec::new();
    let no_qualified: Vec<thaw_bridge::QualifiedFallback> = Vec::new();

    for pkg in &resolved {
        let native_lib_available = pkg.native_lib.is_some();
        let qualified = qualified_by_package.get(&pkg.name).unwrap_or(&no_qualified);
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
            native_addons.push((
                pkg.name.clone(),
                native_addon.to_string_lossy().into_owned(),
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
        .map(|(name, path, root_export)| thaw_bridge::NativeAddon {
            package_name: name,
            path,
            root_export: root_export.as_deref(),
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_native_addon_init(&native_addons));

    Ok((shim, native_libs, rewrites))
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
/// walks elsewhere in this project (thaw-registry's ESM rewrite only
/// ever needs to look at a module's immediate top-level items). Matched
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
        if !matches!(version, Some(1 | 2)) {
            return Err(format!(
                "`{}` must declare FFI metadata version 1 or 2",
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
                return_ownership: if version == Some(2) {
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
                error_ownership: if version == Some(2) {
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

fn build(
    input: &Path,
    output: &Path,
    extra_links: &[PathBuf],
    bridge_dts: &[PathBuf],
    ffi_metadata: &[PathBuf],
    registry_dir: &Path,
    use_packages: &[String],
) -> Result<(), String> {
    let user_source = std::fs::read_to_string(input)
        .map_err(|e| format!("failed to read `{}`: {e}", input.display()))?;
    let (registry_shim, registry_native_libs, qualified_call_rewrites) =
        generate_registry_shims(registry_dir, use_packages)?;
    // `qs.stringify(x)`-style calls, for a name that collided across two
    // `--use`d packages, only exist as source-level syntax sugar over the
    // package-qualified alias `generate_registry_shims` actually
    // generated -- see `rewrite_qualified_calls`'s doc comment. A no-op
    // (and no parse at all) when there were no collisions.
    let user_source = rewrite_qualified_calls(&user_source, &qualified_call_rewrites)?;
    let shim_source = registry_shim + &generate_bridge_shims(bridge_dts)?;
    let mut module = module_graph::bundle(input, &user_source)?;
    if !shim_source.is_empty() {
        let mut shim = thaw_parser::parse_typescript(&shim_source)?;
        shim.body.extend(module.body);
        module.body = shim.body;
    }
    let mut program = thaw_hir::lower_module(&module)?;
    for (symbol, metadata) in read_ffi_metadata(ffi_metadata)? {
        thaw_hir::set_ffi_error_abi(&mut program, &symbol, metadata.error_abi)?;
        thaw_hir::set_ffi_ownership(
            &mut program,
            &symbol,
            metadata.return_ownership,
            metadata.error_ownership,
        )?;
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
    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&quickjs_lib)
        .arg(&napi_lib)
        // QuickJS-NG's C code calls libm math functions directly; `rustc`
        // normally adds `-lm` automatically when it does the final link,
        // but this is a manual `cc` invocation instead.
        .arg("-lm")
        .arg("-ldl")
        .arg("-Wl,--export-dynamic")
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
    use std::net::TcpListener;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn builds_relative_typescript_module_graph_with_generics_and_aliases() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-user-modules-{}", std::process::id()));
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
            "export function value(): number { return 40; }\nexport default function offset(): number { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/index.ts"),
            "export { makePair, chooseFirst } from './pair';\nexport { value } from './values';\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lib/other.ts"),
            "export function value(): number { return 2; }\n",
        )
        .unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(
            &entry,
            r#"
                import { makePair, chooseFirst as first, value } from "./lib";
                import offset, { value as sameValue } from "./lib/values";
                import { value as otherValue } from "./lib/other";
                function main(): void {
                    const pair = makePair(value(), "ok");
                    console.log(pair.first + otherValue());
                    console.log(first("selected", sameValue() + offset()));
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
        assert_eq!(String::from_utf8_lossy(&result.stdout), "42\nselected\n");
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
                export async function transform(event: Json): Promise<Json> {
                    await sleep(1);
                    return event;
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
        std::fs::write(&version, r#"{"version":3,"functions":{}}"#).unwrap();
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
            }
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
            "export declare function add(argsArray: any): any;\n",
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
            "function main(): void { console.log(Number(add(JSON.parse(\"[20,22]\")))); }\n",
        )
        .unwrap();
        build(
            &source,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &["native-add".into()],
        )
        .unwrap();
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
}
