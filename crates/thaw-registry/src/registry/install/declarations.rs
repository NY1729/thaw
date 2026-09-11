/// Follows `/// <reference path="..." />` directives (the classic
/// DefinitelyTyped-style split for a package whose real API is spread
/// across many files, e.g. lodash's `index.d.ts` referencing a dozen files
/// under `common/`), inlining each referenced file's content -- since
/// thaw-registry discards every individual `.d.ts` source file except the
/// one flattened `package.d.ts` it writes out, a reference to a file about
/// to disappear needs to be followed now or never.
///
/// A referenced file commonly augments the *entry* file's own exported
/// namespace via `declare module "<path back to the entry file>" { ... }`
/// (TS module augmentation) rather than declaring its own top-level types
/// -- lodash's `common/array.d.ts` opens with `declare module "../index" {
/// interface LoDashStatic { chunk(...): ...; } }`. When that module
/// specifier resolves back to `entry_path` itself, and `entry_path` exports
/// a namespace via `export as namespace <name>;`, the augmentation is
/// rewritten to `declare namespace <name> { ... }` so the (repeated, once
/// per referenced file) `interface LoDashStatic { ... }` blocks land in the
/// same namespace the entry file's own declaration lives in, matching how
/// a real TS compiler resolves the augmentation. An augmentation targeting
/// some other module is inlined as its own `declare module "..." { ... }`,
/// unresolved but harmless.
fn inline_triple_slash_references(
    entry_path: &Path,
    source: &str,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<String, String> {
    use thaw_parser::ast::{Decl, ModuleItem, Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};

    let namespace = export_as_namespace_name(source)?;
    let mut output = String::new();
    for reference in triple_slash_reference_paths(source) {
        let Some(target_path) = entry_path
            .parent()
            .map(|dir| dir.join(&reference))
            .filter(|path| path.is_file())
        else {
            continue;
        };
        let canonical = target_path
            .canonicalize()
            .unwrap_or_else(|_| target_path.clone());
        if !visited.insert(canonical) {
            continue;
        }
        let referenced_source = fs::read_to_string(&target_path).map_err(|error| {
            format!(
                "failed to read referenced declarations `{}`: {error}",
                target_path.display()
            )
        })?;
        let (referenced_module, source_map) =
            thaw_parser::parse_typescript_with_source_map(&referenced_source)?;
        let canonical_entry = entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf());
        for item in &referenced_module.body {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
                continue;
            };
            let TsModuleName::Str(target) = &module_decl.id else {
                continue;
            };
            let Some(target_specifier) = target.value.as_str() else {
                continue;
            };
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                continue;
            };
            let targets_entry = declaration_reexport_path(&target_path, target_specifier)
                .map(|resolved| resolved.canonicalize().unwrap_or(resolved))
                .is_some_and(|resolved| resolved == canonical_entry);
            let snippets = block
                .body
                .iter()
                .map(|item| {
                    source_map.span_to_snippet(item.span()).map_err(|error| {
                        format!("failed to read module augmentation body: {error:?}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if targets_entry {
                // A self-targeting augmentation with no exported namespace
                // to merge it into has nothing sensible to attach to;
                // drop it rather than leaving a dangling, unresolvable
                // reference to a namespace that was never declared.
                if let Some(namespace) = &namespace {
                    output.push_str(&format!("\ndeclare namespace {namespace} {{\n"));
                    for snippet in &snippets {
                        output.push_str(snippet);
                        output.push('\n');
                    }
                    output.push_str("}\n");
                }
            } else {
                output.push_str(&format!("\ndeclare module \"{target_specifier}\" {{\n"));
                for snippet in &snippets {
                    output.push_str(snippet);
                    output.push('\n');
                }
                output.push_str("}\n");
            }
        }
        // A referenced file can itself carry further references (not
        // exercised by any package tested so far, but the DefinitelyTyped
        // convention allows it).
        output.push_str(&inline_triple_slash_references(
            &target_path,
            &referenced_source,
            visited,
        )?);
    }
    Ok(output)
}

/// The `/// <reference path="..." />` directives in a `.d.ts` source --
/// these are ordinary comments as far as the parser is concerned (and,
/// per the TS convention this follows, only recognized at the very top of
/// the file), so this scans the raw text rather than the AST.
fn triple_slash_reference_paths(source: &str) -> Vec<String> {
    source
        .lines()
        .take_while(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("///") || trimmed.is_empty()
        })
        .filter_map(|line| {
            let start = line.find("path=\"")? + "path=\"".len();
            let end = start + line[start..].find('"')?;
            Some(line[start..end].to_string())
        })
        .collect()
}

/// `export as namespace <name>;` -- the name a `.d.ts` file's own exported
/// value is additionally reachable under as a global namespace, and (for
/// `inline_triple_slash_references`'s purposes) the namespace a referenced
/// file's self-targeting module augmentation actually means to extend.
fn export_as_namespace_name(source: &str) -> Result<Option<String>, String> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem};

    let module = thaw_parser::parse_typescript(source)?;
    Ok(module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsNamespaceExport(export)) => {
            Some(export.id.sym.to_string())
        }
        _ => None,
    }))
}

fn dts_source_with_reexported_functions(
    entry_path: &Path,
    entry_source: &str,
) -> Result<String, String> {
    use thaw_parser::ast::{
        Decl, ExportSpecifier, Expr, MemberProp, ModuleDecl, ModuleExportName, ModuleItem, Stmt,
    };

    let module = thaw_parser::parse_typescript(entry_source)?;
    let mut output = entry_source.to_string();
    let mut visited_references = std::collections::BTreeSet::new();
    output.push_str(&inline_triple_slash_references(
        entry_path,
        entry_source,
        &mut visited_references,
    )?);
    let mut seen = std::collections::BTreeSet::new();
    // `import Name = require("./path")` + a *local* `export { Name as
    // exported };` (no `from` clause -- `Name` is already a value bound
    // earlier in this same file, not a re-export of another module's own
    // export) -- real-world example: semver's `index.d.ts`, which
    // imports one function per file this way and re-exports them all
    // together. Each such `Name` resolves through its own file's `export
    // = X;` to the identifier actually declared there (see
    // `export_assignment_function_declarations`), so unlike the
    // `from`-clause case below, the target file can differ per specifier
    // even within one `export { ... }` statement.
    let import_equals_targets = import_equals_targets(entry_path, &module);
    output.push_str(&inline_import_equals_value_type(
        &module,
        &import_equals_targets,
    )?);
    let named_import_targets = named_import_targets(entry_path, &module);
    let mut local_export_visited = std::collections::BTreeSet::new();
    let local_exports = all_reexported_function_declarations(
        entry_path,
        &mut local_export_visited,
    )?;
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let target_path = match &export.src {
            Some(source) => source
                .value
                .as_str()
                .and_then(|source| declaration_reexport_path(entry_path, source)),
            None => None,
        };
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let export_name = |name: &ModuleExportName| match name {
                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                ModuleExportName::Str(_) => None,
            };
            let Some(original) = export_name(&named.orig) else {
                continue;
            };
            let exported = named
                .exported
                .as_ref()
                .and_then(export_name)
                .unwrap_or_else(|| original.clone());
            if seen.contains(&exported) {
                continue;
            }
            let declarations = match &target_path {
                Some(target_path) => {
                    let mut visited = std::collections::BTreeSet::new();
                    let functions =
                        reexported_function_declarations(target_path, &original, &mut visited)?;
                    if !functions.is_empty() {
                        functions
                    } else {
                        reexported_class_or_interface_declarations(target_path, &original)?
                    }
                }
                None => match import_equals_targets.get(&original) {
                    Some(target_path) => export_assignment_function_declarations(target_path)?,
                    None => match named_import_targets.get(&original) {
                        Some((target_path, target_name)) => {
                            let mut visited = std::collections::BTreeSet::new();
                            let functions = reexported_function_declarations(
                                target_path,
                                target_name,
                                &mut visited,
                            )?;
                            if !functions.is_empty() {
                                functions
                            } else {
                                reexported_class_or_interface_declarations(
                                    target_path,
                                    target_name,
                                )?
                            }
                        }
                        None => local_exports
                            .iter()
                            .filter(|(name, _)| name == &exported)
                            .map(|(_, snippet)| snippet.clone())
                            .collect(),
                    },
                },
            };
            if !declarations.is_empty() {
                seen.insert(exported.clone());
            }
            for mut snippet in declarations {
                if exported != original {
                    snippet = rename_declared_function(snippet, &exported);
                }
                output.push('\n');
                output.push_str(&snippet);
            }
        }
    }
    // `export import NAME = BASE.MEMBER;` -- a TS import-equals
    // declaration whose module reference is a qualified *entity name* (a
    // property access into an already-imported value), not a
    // `require(...)` call (`import_equals_targets`'s own shape, handled
    // above). Real example: uuid@8's real `.d.ts` (via `@types/uuid`),
    // `import uuid from "./index.js"; export import v1 = uuid.v1; export
    // import v4 = uuid.v4; ...`. `uuid.MEMBER` resolves to whatever
    // `./index.js`'s own `.d.ts` exports under the plain name `MEMBER` --
    // a default-imported value's shape mirrors its target's own named
    // exports -- so this reuses `reexported_function_declarations`
    // exactly the way a named re-export (`export { X } from "..."`)
    // already does above, just keyed off a qualified-name AST shape
    // instead of an `ExportSpecifier`.
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else {
            continue;
        };
        if !import.is_export {
            continue;
        }
        let thaw_parser::ast::TsModuleRef::TsEntityName(thaw_parser::ast::TsEntityName::TsQualifiedName(qualified)) =
            &import.module_ref
        else {
            continue;
        };
        let thaw_parser::ast::TsEntityName::Ident(base) = &qualified.left else {
            continue;
        };
        let Some((target_path, _)) = named_import_targets.get(base.sym.as_str()) else {
            continue;
        };
        let member = qualified.right.sym.to_string();
        let exported = import.id.sym.to_string();
        if seen.contains(&exported) {
            continue;
        }
        let mut visited = std::collections::BTreeSet::new();
        let declarations = reexported_function_declarations(target_path, &member, &mut visited)?;
        if !declarations.is_empty() {
            seen.insert(exported.clone());
        }
        for mut snippet in declarations {
            if exported != member {
                snippet = rename_declared_function(snippet, &exported);
            }
            output.push('\n');
            output.push_str(&snippet);
        }
    }
    let mut visited_types = std::collections::BTreeSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) = item else {
            continue;
        };
        let Some(source) = export.src.value.as_str() else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(entry_path, source) else {
            continue;
        };
        for snippet in all_reexported_type_declarations(&target_path, &mut visited_types)? {
            output.push('\n');
            output.push_str(&snippet);
        }
        if export.type_only {
            continue;
        }
        let mut visited = std::collections::BTreeSet::new();
        let declarations = all_reexported_function_declarations(&target_path, &mut visited)?;
        let names = declarations
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !seen.contains(name))
            .collect::<std::collections::BTreeSet<_>>();
        for (name, snippet) in declarations {
            if names.contains(&name) {
                output.push('\n');
                output.push_str(&snippet);
            }
        }
        seen.extend(names);
    }
    // `export * as NAME from "SOURCE";` -- a *namespace* re-export,
    // structurally different from both the named (`export { X } from
    // "..."` above) and wildcard (`export * from "..."` just above) forms:
    // `NAME` isn't a single symbol, it's every one of `SOURCE`'s own
    // exports, reachable one level down (`z.coerce.number(...)`). Real-
    // world example: zod's own re-export barrel declares four of these --
    // `export * as core from "../core/index.cjs";`, `export * as locales
    // from "../locales/index.cjs";`, `export * as iso from "./iso.cjs";`,
    // `export * as coerce from "./coerce.cjs";` -- each exposing its own
    // handful of functions this way, one file below the entry point's own
    // (which reaches them only transitively, through its own plain
    // `export * from "./v4/classic/external.cjs";`) -- so this recurses
    // through plain wildcard re-exports the same way
    // `all_reexported_function_declarations` does, via
    // `collect_namespace_reexports`.
    //
    // Flattened the same way a class method or namespace-nested function
    // elsewhere in this codebase is: each function is renamed to a
    // synthesized, collision-free top-level name (`coerce` and the
    // package's own top-level functions can freely share a bare name --
    // zod's top-level `number()` and `coerce.number()` are unrelated
    // functions, and simply flattening both to bare `number` would silently
    // drop one), then a `declare namespace NAME { export { synthesized as
    // original, ... }; }` block records the mapping back to each function's
    // real member name -- see `thaw_bridge::nested_namespace_members`,
    // which parses this exact shape back out of the finished flattened
    // `.d.ts`.
    let mut namespace_visited = std::collections::BTreeSet::new();
    for (alias, target_path) in
        collect_namespace_reexports(entry_path, &module, &mut namespace_visited)?
    {
        let mut visited = std::collections::BTreeSet::new();
        let declarations = all_reexported_function_declarations(&target_path, &mut visited)?;
        if declarations.is_empty() {
            continue;
        }
        let mut members = Vec::new();
        for (name, snippet) in declarations {
            let synthetic = format!("__thaw_ns_{alias}_{name}");
            output.push('\n');
            output.push_str(&rename_declared_function(snippet, &synthetic));
            members.push(format!("{synthetic} as {name}"));
        }
        output.push_str(&format!(
            "\ndeclare namespace {alias} {{\n    export {{ {} }};\n}}\n",
            members.join(", ")
        ));
    }
    let mut self_referential_visited = std::collections::BTreeSet::new();
    for snippet in
        self_referential_namespace_alias_snippets(entry_path, &mut self_referential_visited)?
    {
        output.push('\n');
        output.push_str(&snippet);
    }
    if let Some(target_path) = export_assignment_namespace_import_target(entry_path, &module) {
        let mut visited_types = std::collections::BTreeSet::new();
        for snippet in all_reexported_type_declarations(&target_path, &mut visited_types)? {
            output.push('\n');
            output.push_str(&snippet);
        }
        let mut visited = std::collections::BTreeSet::new();
        for (_, snippet) in all_reexported_function_declarations(&target_path, &mut visited)? {
            output.push('\n');
            output.push_str(&snippet);
        }
    }
    // A locally declared class whose `extends` clause names a plain
    // default-imported base (`import AjvCore from "./core"; export
    // declare class Ajv extends AjvCore { ... }`) -- the base class's
    // own declaration lives entirely in another file thaw-registry
    // otherwise discards. Real example: ajv's own entry `.d.ts`, whose
    // `AjvCore` base (`dist/core.d.ts`'s default-exported `Ajv` class)
    // declares essentially the entire real API (`compile`, `validate`,
    // `addSchema`, ...) -- without inlining it, only the 3 methods the
    // entry file adds directly were ever reachable. Reuses
    // `reexported_class_or_interface_declarations`'s existing
    // (same-file) class-inheritance resolution in thaw-bridge
    // downstream: once the base class's own text is present in the
    // flattened output under the alias name the entry file's `extends`
    // clause expects, that resolution already works unmodified.
    // A namespace import of a Node builtin (`import * as stream from
    // "stream";`), used only for its *specifier* below -- collected
    // once up front rather than re-scanning `module.body` inside the
    // loop for every class.
    let namespace_import_specifiers: std::collections::HashMap<String, String> = module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
                return None;
            };
            import.specifiers.iter().find_map(|specifier| {
                let thaw_parser::ast::ImportSpecifier::Namespace(namespace) = specifier else {
                    return None;
                };
                Some((
                    namespace.local.sym.to_string(),
                    import.src.value.to_string_lossy().into_owned(),
                ))
            })
        })
        .collect();
    for item in &module.body {
        let class = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Class(class) => Some(class),
                _ => None,
            },
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => Some(class),
            _ => None,
        };
        let Some(class) = class else { continue };
        match class.class.super_class.as_deref() {
            Some(Expr::Ident(base)) => {
                let Some((target_path, target_name)) =
                    named_import_targets.get(base.sym.as_str())
                else {
                    continue;
                };
                let mut declarations =
                    reexported_class_or_interface_declarations(target_path, target_name)?;
                if let Some(first) = declarations.first_mut() {
                    *first = rename_declared_function(std::mem::take(first), base.sym.as_ref());
                }
                for snippet in declarations {
                    output.push('\n');
                    output.push_str(&snippet);
                }
            }
            // The same shape, but the base is namespace-qualified
            // (`stream.Transform`) rather than a bare imported
            // identifier -- real example: csv-parse's own `Parser
            // extends stream.Transform`. Unlike the case above, the
            // base class's declaration doesn't live in a file thaw
            // fetched at all; it's one of thaw's own synthetic
            // ambient declarations for a Node builtin module
            // (`resolve_builtin`, already used for a top-level `--use
            // node:stream`, reused here read-only for its `.d.ts`
            // text). Extracted from that in-memory string (not a file
            // on disk, hence a dedicated helper rather than
            // `reexported_class_or_interface_declarations`), following
            // the base's own `extends` chain transitively within that
            // same string (e.g. `Transform extends Duplex extends
            // Readable`) so every inherited method is reachable, not
            // just the immediate base's own.
            Some(Expr::Member(member)) => {
                let Expr::Ident(namespace) = member.obj.as_ref() else {
                    continue;
                };
                let MemberProp::Ident(base_name) = &member.prop else {
                    continue;
                };
                let Some(specifier) = namespace_import_specifiers.get(namespace.sym.as_str())
                else {
                    continue;
                };
                let Ok(builtin) = resolve_builtin(specifier) else {
                    continue;
                };
                let declarations = builtin_class_and_ancestor_declarations(
                    &builtin.dts_source,
                    base_name.sym.as_ref(),
                    &mut std::collections::BTreeSet::new(),
                )?;
                for snippet in declarations {
                    output.push('\n');
                    output.push_str(&snippet);
                }
            }
            _ => continue,
        }
    }
    Ok(output)
}

/// The namespace-qualified-extends counterpart to
/// `reexported_class_or_interface_declarations_inner`: extracts a class
/// declaration by name, and (transitively) its own ancestors, from an
/// in-memory Node-builtin `.d.ts` string (`resolve_builtin`'s own
/// output) rather than a file on disk -- a builtin's synthetic
/// declaration is self-contained (no further external imports to
/// follow), so this only ever needs to look within `dts_source` itself.
fn builtin_class_and_ancestor_declarations(
    dts_source: &str,
    name: &str,
    visited: &mut std::collections::BTreeSet<String>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(name.to_string()) {
        return Ok(Vec::new());
    }
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(dts_source)?;
    let mut declarations = Vec::new();
    let mut superclass = None;
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
            continue;
        };
        let Decl::Class(class) = &export.decl else {
            continue;
        };
        if class.ident.sym.as_ref() != name {
            continue;
        }
        superclass = class.class.super_class.as_deref().and_then(|expr| match expr {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        });
        let snippet = source_map.span_to_snippet(class.span()).map_err(|error| {
            format!("failed to read builtin class declaration `{name}`: {error:?}")
        })?;
        declarations.push(snippet);
        break;
    }
    if let Some(superclass) = superclass {
        declarations.extend(builtin_class_and_ancestor_declarations(
            dts_source,
            &superclass,
            visited,
        )?);
    }
    Ok(declarations)
}

fn all_reexported_type_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let mut declarations = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export))
                if matches!(
                    &export.decl,
                    Decl::Class(_)
                        | Decl::TsInterface(_)
                        | Decl::TsTypeAlias(_)
                        | Decl::TsEnum(_)
                        | Decl::TsModule(_)
                ) =>
            {
                declarations.push(source_map.span_to_snippet(export.span()).map_err(|error| {
                    format!("failed to read type declaration in `{}`: {error:?}", path.display())
                })?);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        declarations.extend(all_reexported_type_declarations(&target, visited)?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(declarations)
}

/// A same-file `import * as X from "SOURCE"; export { X[, X as Y], ... };`
/// (and/or `export default X;`) -- the shape `thaw_bridge::self_
/// referential_namespace_aliases` recognizes in an already-flattened
/// `.d.ts`, but only at the top level of *one* file. Real npm packages
/// sometimes declare this in a file reached only *transitively* through
/// the entry point's own `export * from "...";`, not the entry point
/// itself -- real example: zod v3's `lib/index.d.ts` (`import * as z
/// from "./external"; export * from "./external"; export { z }; export
/// default z;`), one file below the package's own entry point
/// (`index.d.ts`, just `export * from "./lib";`) -- unlike zod v4, whose
/// entry point declares this alias directly, so it was already visible
/// to `self_referential_namespace_aliases` without this. Recurses through
/// wildcard re-exports the same way `collect_namespace_reexports` does,
/// returning the raw import/export snippet text verbatim for each
/// namespace-imported name that's re-exported this way -- its import
/// source doesn't need to resolve to anything real in the flattened
/// output (`self_referential_namespace_aliases` only checks the AST
/// shape, never follows the source path), so a caller can splice it into
/// the flattened output to make the alias visible to that same
/// downstream detector.
fn self_referential_namespace_alias_snippets(
    entry_path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{
        Expr, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
    };
    use thaw_parser::common::{SourceMapper, Span, Spanned};

    if !visited.insert(entry_path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(entry_path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            entry_path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;

    let mut namespace_imports: std::collections::HashMap<String, (Span, Vec<Span>)> =
        std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        for specifier in &import.specifiers {
            if let ImportSpecifier::Namespace(namespace) = specifier {
                namespace_imports
                    .entry(namespace.local.sym.to_string())
                    .or_insert_with(|| (import.span(), Vec::new()));
            }
        }
    }
    let mut found = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if !export.type_only && export.src.is_none() =>
            {
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else {
                        continue;
                    };
                    if named.is_type_only {
                        continue;
                    }
                    let ModuleExportName::Ident(ident) = &named.orig else {
                        continue;
                    };
                    if let Some((_, exports)) = namespace_imports.get_mut(ident.sym.as_str()) {
                        exports.push(export.span());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                if let Expr::Ident(ident) = default_expr.expr.as_ref() {
                    if let Some((_, exports)) = namespace_imports.get_mut(ident.sym.as_str()) {
                        exports.push(default_expr.span());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                if let Some(source_path) = export.src.value.as_str() {
                    if let Some(target_path) = declaration_reexport_path(entry_path, source_path) {
                        found.extend(self_referential_namespace_alias_snippets(
                            &target_path,
                            visited,
                        )?);
                    }
                }
            }
            _ => {}
        }
    }
    for (name, (import_span, export_spans)) in &namespace_imports {
        if export_spans.is_empty() {
            continue;
        }
        let mut snippet = source_map
            .span_to_snippet(*import_span)
            .map_err(|error| format!("failed to read namespace import `{name}`: {error:?}"))?;
        for export_span in export_spans {
            snippet.push('\n');
            snippet.push_str(&source_map.span_to_snippet(*export_span).map_err(|error| {
                format!("failed to read self-referential export for `{name}`: {error:?}")
            })?);
        }
        found.push(snippet);
    }
    Ok(found)
}

/// Every `export * as NAME from "SOURCE";` reachable from `entry_path`'s
/// own `module` -- including one declared in a file only reached
/// transitively through a plain `export * from "...";` (real zod: the
/// namespace exports live one file below the package's own entry point).
/// Doesn't recurse through a *named* re-export (`export { x } from
/// "...";"`) -- that forwards one specific symbol, not a whole module's
/// worth of further bindings, and no package seen so far needs it to.
fn collect_namespace_reexports(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, PathBuf)>, String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    if !visited.insert(entry_path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only => {
                let Some(source) = export.src.as_ref().and_then(|s| s.value.as_str()) else {
                    continue;
                };
                for specifier in &export.specifiers {
                    let ExportSpecifier::Namespace(namespace) = specifier else {
                        continue;
                    };
                    let ModuleExportName::Ident(alias) = &namespace.name else {
                        continue;
                    };
                    if let Some(target_path) = declaration_reexport_path(entry_path, source) {
                        found.push((alias.sym.to_string(), target_path));
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                let Some(source) = export.src.value.as_str() else {
                    continue;
                };
                let Some(target_path) = declaration_reexport_path(entry_path, source) else {
                    continue;
                };
                let target_source = fs::read_to_string(&target_path).map_err(|error| {
                    format!(
                        "failed to read re-exported declarations `{}`: {error}",
                        target_path.display()
                    )
                })?;
                let target_module = thaw_parser::parse_typescript(&target_source)?;
                found.extend(collect_namespace_reexports(
                    &target_path,
                    &target_module,
                    visited,
                )?);
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Whether `name` is a full ECMAScript reserved word -- one that can
/// never be used as an ordinary binding/declaration name (a function
/// declared `declare function null(...)` is a syntax error, full stop,
/// not just an unusual identifier choice). Deliberately excludes a
/// "future reserved word" like `enum` and a merely-predefined global
/// like `undefined`, neither of which is actually reserved in this
/// position -- both parse fine as a function's own name, and real npm
/// packages rename an internal helper to exactly these words (zod's
/// `z.enum(...)`, `z.undefined()`) since they're the desired public API
/// name (see the `enum`/`catch`/`instanceof`/etc. rename-export handling
/// above, whose alias this guards).
fn is_ecmascript_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

/// Given one `const`/`let`/`var` declarator, if its type annotation names
/// a callable shape -- a same-file interface with a call signature
/// (`SQLiteTableFn`-style), or a *direct* inline function type with no
/// interface involved at all (zod v3's `declare const objectType: <T
/// extends ZodRawShape>(shape: T, params?) => ZodObject<...>;`, one per
/// primitive) -- the declaration text needed to make its call signature
/// visible in a flattened `.d.ts`: just `decl_span`'s own snippet for a
/// direct function type (self-contained), or that snippet plus the
/// referenced interface's own snippet for a `TsTypeRef` (the call
/// signature lives there, not in the const itself). `None` if the type
/// doesn't name a callable shape at all (an ordinary data constant, or a
/// type this doesn't resolve).
///
/// `decl_span` is the caller's choice of "the const's own declaration
/// text" -- an `ExportDecl`'s span (to keep an `export` prefix) for an
/// exported const (`callable_const_declarations`, below), or a bare
/// `VarDecl`'s own span for one reached only through a later local
/// rename-export (`all_reexported_function_declarations`'s own
/// `local_declarations` loop, mirroring how it already handles a bare
/// `Decl::Fn`).
///
/// The interface lookup is restricted to a *same-file* interface, not one
/// reached through a further import: every real package seen so far
/// declares the factory-value const and its call-signature interface
/// side by side in one file, and resolving a cross-file interface would
/// need machinery parallel to `reexported_class_or_interface_declarations`
/// -- not worth building speculatively.
fn callable_const_declaration_snippet(
    module: &thaw_parser::ast::Module,
    source_map: &thaw_parser::common::SourceMap,
    decl_span: thaw_parser::common::Span,
    binding: &thaw_parser::ast::BindingIdent,
) -> Result<Option<String>, String> {
    use thaw_parser::ast::{
        Decl, ModuleDecl, ModuleItem, TsEntityName, TsFnOrConstructorType, TsType, TsTypeElement,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let Some(annotation) = binding.type_ann.as_ref() else {
        return Ok(None);
    };
    match annotation.type_ann.as_ref() {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(_)) => {
            let const_snippet = source_map.span_to_snippet(decl_span).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    binding.id.sym
                )
            })?;
            Ok(Some(const_snippet))
        }
        TsType::TsTypeRef(ty_ref) => {
            let iface_name = match &ty_ref.type_name {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            let mut matched_iface_export = None;
            for other in &module.body {
                let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(iface_export)) = other else {
                    continue;
                };
                let Decl::TsInterface(iface) = &iface_export.decl else {
                    continue;
                };
                if iface.id.sym.as_ref() == iface_name.as_str() {
                    matched_iface_export = Some((iface_export, iface));
                    break;
                }
            }
            if let Some((iface_export, iface)) = matched_iface_export {
                if iface
                    .body
                    .body
                    .iter()
                    .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_)))
                {
                    let const_snippet = source_map.span_to_snippet(decl_span).map_err(|error| {
                        format!(
                            "failed to read declaration for `{}`: {error:?}",
                            binding.id.sym
                        )
                    })?;
                    let iface_snippet =
                        source_map.span_to_snippet(iface_export.span()).map_err(|error| {
                            format!("failed to read declaration for `{iface_name}`: {error:?}")
                        })?;
                    return Ok(Some(format!("{const_snippet}\n{iface_snippet}")));
                }
            }
            // A local (bare or exported) type alias, or an interface with
            // no call signature of its own, whose shape might still
            // resolve to something callable through a chain of further
            // local aliases/interfaces -- rather than resolving that
            // chain ourselves (thaw-bridge's own `classify_ts_type`/
            // `resolve_interfaces` already does, downstream, once the
            // text is present), carry the const's own snippet together
            // with *every* type alias and interface declared in this
            // same file (bare or exported). `.d.ts` type declarations are
            // erasable and side-effect-free, so including ones that turn
            // out unrelated to this particular const is harmless. Real
            // example: uuid's own `.d.ts` (via `@types/uuid`), where
            // `v1`'s declared type (`type v1 = v1Buffer & v1String;`) is
            // a *local, unexported* type alias chaining through several
            // more (`v1Buffer`, `v1String`, `V1Options`, ...). Conditioned
            // on the referenced name actually resolving to *something*
            // local at all -- a truly unknown/external type name still
            // returns `None`, unchanged from before.
            let resolves_locally = module.body.iter().any(|item| {
                let decl = match item {
                    ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(decl)) => decl,
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
                    _ => return false,
                };
                match decl {
                    Decl::TsInterface(iface) => iface.id.sym.as_ref() == iface_name.as_str(),
                    Decl::TsTypeAlias(alias) => alias.id.sym.as_ref() == iface_name.as_str(),
                    _ => false,
                }
            });
            if !resolves_locally {
                return Ok(None);
            }
            let mut combined = source_map.span_to_snippet(decl_span).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    binding.id.sym
                )
            })?;
            for item in &module.body {
                let decl = match item {
                    ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(decl)) => decl,
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
                    _ => continue,
                };
                let span = match decl {
                    Decl::TsInterface(iface) => iface.span(),
                    Decl::TsTypeAlias(alias) => alias.span(),
                    _ => continue,
                };
                combined.push('\n');
                combined.push_str(&source_map.span_to_snippet(span).map_err(|error| {
                    format!("failed to read a local type declaration: {error:?}")
                })?);
            }
            Ok(Some(combined))
        }
        _ => Ok(None),
    }
}

/// Every top-level `export declare const NAME: T;` in `module` whose type
/// `T` names a callable shape -- see `callable_const_declaration_snippet`
/// for the two shapes recognized and real examples of each. The
/// `Decl::Var` counterpart to a plain exported `Decl::Fn`.
fn callable_const_declarations(
    module: &thaw_parser::ast::Module,
    source_map: &thaw_parser::common::SourceMap,
) -> Result<Vec<(String, String)>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Pat};
    use thaw_parser::common::Spanned;

    let mut declarations = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
            continue;
        };
        let Decl::Var(var_decl) = &export.decl else {
            continue;
        };
        for declarator in &var_decl.decls {
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            if let Some(snippet) =
                callable_const_declaration_snippet(module, source_map, export.span(), binding)?
            {
                declarations.push((binding.id.sym.to_string(), snippet));
            }
        }
    }
    Ok(declarations)
}

/// Renames a single function declaration snippet's own declared name to
/// `exported`, wherever it actually is -- rather than assuming it matches
/// whatever name it was originally looked up under. That assumption holds
/// for a plain `export { original as exported }`, but not when `original`
/// was `"default"`: the snippet `reexported_function_declarations` returns
/// for a `default` lookup is resolved through `export default <ident>;` (or
/// an inline `export default function <ident>(...) {}`) and keeps that
/// real `<ident>`, which need not equal the literal string `"default"`.
fn rename_declared_function(snippet: String, exported: &str) -> String {
    // `"const "` covers a callable-const snippet (`callable_const_
    // declaration_snippet`, e.g. zod v3's `declare const objectType:
    // (...) => ...;`) -- for the interface-backed shape, whose snippet
    // concatenates the const's own declaration with its referenced
    // interface's, `"const "` always appears before that interface's own
    // `"interface "`, so this still renames the *const's* binding name
    // (the one actually being re-exported), not the interface's.
    let Some((keyword_index, keyword)) = ["function ", "class ", "interface ", "const "]
        .into_iter()
        .filter_map(|keyword| snippet.find(keyword).map(|index| (index, keyword)))
        .min_by_key(|(index, _)| *index)
    else {
        return snippet;
    };
    let name_start = keyword_index + keyword.len();
    let name_end = snippet[name_start..]
        .find(|character: char| !(character.is_alphanumeric() || character == '_' || character == '$'))
        .map(|offset| name_start + offset)
        .unwrap_or(snippet.len());
    if name_start == name_end || &snippet[name_start..name_end] == exported {
        return snippet;
    }
    let mut renamed = String::with_capacity(snippet.len());
    renamed.push_str(&snippet[..name_start]);
    renamed.push_str(exported);
    renamed.push_str(&snippet[name_end..]);
    renamed
}

fn all_reexported_function_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, String)>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let mut declarations = Vec::new();
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) = item {
            if let Decl::Fn(function) = &declaration.decl {
                declarations.push((
                    function.ident.sym.to_string(),
                    source_map
                        .span_to_snippet(declaration.span())
                        .map_err(|error| {
                            format!(
                                "failed to read declaration for `{}`: {error:?}",
                                function.ident.sym
                            )
                        })?,
                ));
            }
        }
    }
    // A *local*, non-exported `declare function` statement -- collected
    // up front (by name, supporting more than one overload) so a
    // same-file, no-`from`-clause rename-export below (`export { _enum
    // as enum };`) can resolve to one even though it was never itself
    // `export`-prefixed. Real example: zod's own `schemas.d.cts`
    // declares `declare function _enum(...)` (two overloads) bare, then
    // separately does `export { _enum as enum };` -- `enum` being a
    // reserved word, it can't be the function's own declared name.
    // Without this, `_enum` (and thus `z.enum(...)`) was silently
    // missing from the flattened `package.d.ts` entirely, not even
    // present under its own internal name.
    let mut local_declarations: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) = item {
            let snippet = source_map.span_to_snippet(function.span()).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    function.ident.sym
                )
            })?;
            local_declarations
                .entry(function.ident.sym.to_string())
                .or_default()
                .push(snippet);
        }
    }
    // A *local*, non-exported callable `const` -- the `Decl::Var`
    // counterpart to the bare `Decl::Fn` loop just above, for the exact
    // same reason (a same-file rename-export below needs to resolve it).
    // Real example: zod v3's `lib/types.d.ts`, which declares `declare
    // const objectType: <T extends ZodRawShape>(shape: T, params?) =>
    // ZodObject<...>;` (and ~30 siblings) bare, then separately does
    // `export { ..., objectType as object, ... };` -- without this,
    // `object`/`string`/`number`/etc. (essentially all of zod v3's own
    // API) were silently missing from the flattened `package.d.ts`
    // entirely.
    for item in &module.body {
        let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(var_decl))) = item else {
            continue;
        };
        for declarator in &var_decl.decls {
            let thaw_parser::ast::Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            if let Some(snippet) =
                callable_const_declaration_snippet(&module, &source_map, var_decl.span(), binding)?
            {
                local_declarations
                    .entry(binding.id.sym.to_string())
                    .or_default()
                    .push(snippet);
            }
        }
    }
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only || export.src.is_some() {
            continue;
        }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let export_name = |name: &ModuleExportName| match name {
                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                ModuleExportName::Str(_) => None,
            };
            let Some(original) = export_name(&named.orig) else {
                continue;
            };
            let exported = named
                .exported
                .as_ref()
                .and_then(export_name)
                .unwrap_or_else(|| original.clone());
            let Some(snippets) = local_declarations.get(&original) else {
                continue;
            };
            for snippet in snippets {
                let snippet = if exported == original {
                    snippet.clone()
                } else {
                    rename_declared_function(snippet.clone(), &exported)
                };
                // The exported alias can be any identifier-like text at
                // all in TS export-specifier syntax, including a real
                // ECMAScript reserved word (`null`, `void`, `function`,
                // `catch`, `instanceof`) that can never actually be a
                // function's own declared name -- real example: zod's
                // own `export { _null as null };`. Renaming to one of
                // those would produce text no parser accepts (`declare
                // function null(...)`), and letting that reach the
                // flattened file poisons parsing for every other
                // declaration in it, not just this one -- confirmed via
                // a standalone repro of each word below. `enum` (a
                // future-reserved word, not a full keyword) and
                // `undefined` (not reserved at all, just a predefined
                // global) are deliberately not in this list -- both are
                // real, valid function names to this parser, and zod
                // uses both (`z.enum(...)`, `z.undefined()`).
                if is_ecmascript_keyword(&exported) {
                    continue;
                }
                declarations.push((exported.clone(), snippet));
            }
        }
    }
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        declarations.extend(all_reexported_function_declarations(
                            &target, visited,
                        )?);
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if !export.type_only && export.src.is_some() =>
            {
                let source = export.src.as_ref().unwrap();
                let Some(source) = source.value.as_str() else {
                    continue;
                };
                let Some(target) = declaration_reexport_path(path, source) else {
                    continue;
                };
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else {
                        continue;
                    };
                    if named.is_type_only {
                        continue;
                    }
                    let export_name = |name: &ModuleExportName| match name {
                        ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                        ModuleExportName::Str(_) => None,
                    };
                    let Some(original) = export_name(&named.orig) else {
                        continue;
                    };
                    let exported = named
                        .exported
                        .as_ref()
                        .and_then(export_name)
                        .unwrap_or_else(|| original.clone());
                    let mut named_visited = std::collections::BTreeSet::new();
                    for mut snippet in reexported_function_declarations(
                        &target,
                        &original,
                        &mut named_visited,
                    )? {
                        if exported != original {
                            snippet = rename_declared_function(snippet, &exported);
                        }
                        declarations.push((exported.clone(), snippet));
                    }
                }
            }
            _ => {}
        }
    }
    declarations.extend(callable_const_declarations(&module, &source_map)?);
    Ok(declarations)
}

fn reexported_function_declarations(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert((path.to_path_buf(), name.to_string())) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let mut declarations = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) = item else {
            continue;
        };
        let Decl::Fn(function) = &declaration.decl else {
            continue;
        };
        if function.ident.sym == name {
            declarations.push(
                source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| {
                        format!("failed to read declaration for `{name}`: {error:?}")
                    })?,
            );
        }
    }
    if declarations.is_empty() {
        for (const_name, snippet) in callable_const_declarations(&module, &source_map)? {
            if const_name == name {
                declarations.push(snippet);
            }
        }
    }
    if declarations.is_empty() {
        let local_name = module.body.iter().find_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
                return None;
            };
            if export.type_only || export.src.is_some() {
                return None;
            }
            export.specifiers.iter().find_map(|specifier| {
                let ExportSpecifier::Named(named) = specifier else {
                    return None;
                };
                let exported = named.exported.as_ref().unwrap_or(&named.orig);
                let ModuleExportName::Ident(exported) = exported else {
                    return None;
                };
                if exported.sym.as_ref() != name {
                    return None;
                }
                let ModuleExportName::Ident(original) = &named.orig else {
                    return None;
                };
                Some(original.sym.to_string())
            })
        });
        if let Some(local_name) = local_name {
            for item in &module.body {
                let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(var_decl))) = item
                else {
                    continue;
                };
                for declarator in &var_decl.decls {
                    let thaw_parser::ast::Pat::Ident(binding) = &declarator.name else {
                        continue;
                    };
                    if binding.id.sym == local_name {
                        if let Some(snippet) = callable_const_declaration_snippet(
                            &module,
                            &source_map,
                            var_decl.span(),
                            binding,
                        )? {
                            declarations.push(snippet);
                        }
                    }
                }
            }
        }
    }
    if !declarations.is_empty() {
        return Ok(declarations);
    }
    // `export { default as v4 } from './v4.js'` (a barrel re-exporting
    // another file's *default* export under a name, the common shape for
    // packages that split one function per file, e.g. `uuid`) resolves to
    // `name == "default"` here, but a `.d.ts` file never declares a
    // function literally named `default` -- it either exports an inline
    // `export default function v4(...) {}` or (more commonly, so multiple
    // overloads can share one export) declares `function v4(...)` one or
    // more times as a plain top-level statement and separately writes
    // `export default v4;`. Resolve that indirection before giving up.
    if name == "default" {
        for item in &module.body {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                    let thaw_parser::ast::Expr::Ident(ident) = default_expr.expr.as_ref() else {
                        continue;
                    };
                    let resolved = ident.sym.as_ref();
                    for item in &module.body {
                        let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) =
                            item
                        else {
                            continue;
                        };
                        if function.ident.sym.as_ref() == resolved {
                            declarations.push(source_map.span_to_snippet(function.span()).map_err(
                                |error| format!("failed to read declaration for `{resolved}`: {error:?}"),
                            )?);
                        }
                    }
                    // A bare data *constant*, not a function -- real
                    // example: uuid's own `dist/nil.js`'s `.d.ts`:
                    // `declare const _default: "00000000-0000-0000-
                    // 0000-000000000000"; export default _default;`
                    // (one file per constant, re-exported under a real
                    // name -- `NIL`/`MAX` -- via a barrel file's `export
                    // { default as NIL } from './nil.js'`). The `declare
                    // const` statement itself is never directly
                    // exported (only indirectly, via this separate
                    // `export default <ident>;`), so its own snippet
                    // carries no `export` keyword -- fine, since
                    // `thaw_bridge::parse_dts_values` (which reads the
                    // flattened result this eventually becomes part of)
                    // already recognizes a bare ambient `declare const`
                    // exactly like every other ambient `.d.ts`
                    // declaration. `rename_declared_function` (this
                    // snippet's only caller, when `exported != original`)
                    // already handles the `"const "` keyword generically.
                    for item in &module.body {
                        let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(var_decl))) =
                            item
                        else {
                            continue;
                        };
                        for declarator in &var_decl.decls {
                            let thaw_parser::ast::Pat::Ident(binding) = &declarator.name else {
                                continue;
                            };
                            if binding.id.sym.as_ref() != resolved {
                                continue;
                            }
                            let Some(annotation) = &binding.type_ann else {
                                continue;
                            };
                            if !matches!(
                                annotation.type_ann.as_ref(),
                                thaw_parser::ast::TsType::TsLitType(_)
                            ) {
                                continue;
                            }
                            declarations.push(source_map.span_to_snippet(var_decl.span()).map_err(
                                |error| format!("failed to read declaration for `{resolved}`: {error:?}"),
                            )?);
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) => {
                    if let thaw_parser::ast::DefaultDecl::Fn(fn_expr) = &default_decl.decl {
                        if fn_expr.ident.is_some() {
                            declarations.push(source_map.span_to_snippet(default_decl.span()).map_err(
                                |error| format!("failed to read default function declaration: {error:?}"),
                            )?);
                        }
                    }
                }
                _ => {}
            }
        }
        if !declarations.is_empty() {
            return Ok(declarations);
        }
    }
    for item in module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let Some(source) = export.src.and_then(|source| source.value.as_str().map(str::to_owned))
        else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(path, &source) else {
            continue;
        };
        for specifier in export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let export_name = |name: &ModuleExportName| match name {
                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                ModuleExportName::Str(_) => None,
            };
            let original = export_name(&named.orig);
            let exported = named.exported.as_ref().and_then(export_name).or_else(|| original.clone());
            if exported.as_deref() == Some(name) {
                return reexported_function_declarations(
                    &target_path,
                    original.as_deref().unwrap_or(name),
                    visited,
                );
            }
        }
    }
    Ok(Vec::new())
}

/// Every plain (non-exported) `declare function X(...)` overload matching
/// whatever identifier `path`'s own `export = X;` names -- the shape
/// `import Name = require("./path")` (a TS import-equals declaration)
/// binds `Name` to, one file per function, real-world example: semver's
/// `functions/valid.d.ts` (`declare function valid(...): ...;\nexport =
/// valid;`). A plain `export = X;` module has no re-export chain of its
/// own to follow beyond this (unlike `reexported_function_declarations`,
/// which also handles `export { X } from another`), so this only ever
/// looks inside `path` itself.
fn export_assignment_function_declarations(path: &Path) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let Some(target) = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => match export.expr.as_ref() {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    }) else {
        return Ok(Vec::new());
    };
    module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
                return None;
            };
            (function.ident.sym.as_ref() == target).then(|| {
                source_map
                    .span_to_snippet(function.span())
                    .map_err(|error| format!("failed to read declaration for `{target}`: {error:?}"))
            })
        })
        .collect()
}

/// The relative path each `import Name = require("./path")` (a TS
/// import-equals declaration) in `module` resolves to, keyed by `Name` --
/// used to follow a *local* `export { Name as exported };` (no `from`
/// clause: `Name` is already a value imported earlier in this same file,
/// not a re-export of another module's own export), real-world example:
/// semver's `index.d.ts`, which imports one function per file this way
/// and re-exports all of them together.
fn import_equals_targets(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> std::collections::HashMap<String, PathBuf> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem, TsModuleRef};

    module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else {
                return None;
            };
            let TsModuleRef::TsExternalModuleRef(reference) = &import.module_ref else {
                return None;
            };
            let source = reference.expr.value.as_str()?;
            let target_path = declaration_reexport_path(entry_path, source)?;
            Some((import.id.sym.to_string(), target_path))
        })
        .collect()
}

/// The relative path a top-level `import * as NAME from "./y"` (a
/// namespace import, ES-module style -- distinct from `import_equals_
/// targets`'s `import Name = require("./y")`) resolves to, but only
/// when `module`'s own top-level `export = NAME;` names that exact
/// import's local binding -- i.e. the whole file's declared shape *is*
/// another file's namespace, wholesale, with nothing declared locally
/// at all. Real example: bcryptjs's `umd/index.d.ts`: `import * as
/// bcrypt from "./types.js"; export = bcrypt; export as namespace
/// bcrypt;` -- every one of bcryptjs's actual functions (`hashSync`,
/// `compareSync`, ...) lives in the sibling `types.d.ts`, never
/// otherwise reachable, since thaw-registry discards every individual
/// `.d.ts` source file except the one flattened `package.d.ts` it
/// writes out. Without this, an entry `.d.ts` shaped this way flattens
/// to just the three lines above -- no functions, no types, nothing --
/// and every call against the package fails to build ("call to unknown
/// function"). Returns `None` for the unrelated, much more common case
/// where `export = X;` names something declared directly in the entry
/// file itself (joi's `declare const Joi: Joi.Root; export = Joi;`),
/// which already works today since the whole file's own text is kept.
fn export_assignment_namespace_import_target(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> Option<PathBuf> {
    use thaw_parser::ast::{Expr, ImportSpecifier, ModuleDecl, ModuleItem};

    let exported_name = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => match export.expr.as_ref() {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    })?;
    module.body.iter().find_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            return None;
        };
        if import.type_only {
            return None;
        }
        let source = import.src.value.as_str()?;
        import.specifiers.iter().find_map(|specifier| {
            let ImportSpecifier::Namespace(namespace) = specifier else {
                return None;
            };
            (namespace.local.sym.as_ref() == exported_name)
                .then(|| declaration_reexport_path(entry_path, source))
                .flatten()
        })
    })
}

/// The `(target_path, name_in_target)` each *ordinary* ES import
/// (`import { X } from "./y"`, `import { X as Local } from "./y"`, or a
/// default import `import Local from "./y"`) in `module` resolves to,
/// keyed by the local binding name -- the ES-module counterpart of
/// `import_equals_targets` above, used the same way: to follow a local
/// `export { Local };` (no `from` clause) back to whatever `Local` was
/// actually bound to. Real-world example: hono's `index.d.ts`, which
/// does `import { Hono } from './hono'; export { Hono };` -- a class,
/// not a function, split into its own file and re-exported under the
/// same local name. Only `.`-relative specifiers resolve (matching
/// `declaration_reexport_path`); a bare-specifier import (a real
/// dependency, not an internal file split) is left alone.
fn named_import_targets(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> std::collections::HashMap<String, (PathBuf, String)> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleItem};

    let mut targets = std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if import.type_only {
            continue;
        }
        let Some(source) = import.src.value.as_str() else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(entry_path, source) else {
            continue;
        };
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Named(named) if !named.is_type_only => {
                    let imported_name = named
                        .imported
                        .as_ref()
                        .and_then(|name| match name {
                            thaw_parser::ast::ModuleExportName::Ident(id) => {
                                Some(id.sym.to_string())
                            }
                            thaw_parser::ast::ModuleExportName::Str(_) => None,
                        })
                        .unwrap_or_else(|| named.local.sym.to_string());
                    targets.insert(
                        named.local.sym.to_string(),
                        (target_path.clone(), imported_name),
                    );
                }
                ImportSpecifier::Default(default) => {
                    targets.insert(
                        default.local.sym.to_string(),
                        (target_path.clone(), "default".to_string()),
                    );
                }
                _ => {}
            }
        }
    }
    targets
}

/// Same shape as `reexported_function_declarations`, but for a class or
/// interface declared directly in `path` under `name` (the counterpart
/// to that function's `Decl::Fn` handling for `Decl::Class`/
/// `Decl::TsInterface`) -- doesn't follow further `export ... from`
/// re-export chains itself, since `named_import_targets` only ever
/// points at the file a name was *imported* from, which for every
/// package seen so far declares the class/interface directly rather
/// than re-exporting it yet again.
fn reexported_class_or_interface_declarations(
    path: &Path,
    name: &str,
) -> Result<Vec<String>, String> {
    let mut visited = std::collections::BTreeSet::new();
    reexported_class_or_interface_declarations_inner(path, name, &mut visited)
}

fn reexported_class_or_interface_declarations_inner(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert((path.to_path_buf(), name.to_string())) {
        return Ok(Vec::new());
    }

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let mut local_name = name.to_string();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.src.is_some() {
            continue;
        }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            let exported = named.exported.as_ref().unwrap_or(&named.orig);
            let (ModuleExportName::Ident(exported), ModuleExportName::Ident(original)) =
                (exported, &named.orig)
            else {
                continue;
            };
            if exported.sym == name {
                local_name = original.sym.to_string();
            }
        }
    }

    let mut declarations = Vec::new();
    let mut superclass = None;
    // `name == "default"` (the same sentinel `named_import_targets`
    // stores for a plain default import, `import AjvCore from
    // "./core";`) means "whatever class `path` exports as its default,
    // regardless of its own declared identifier" -- there's at most one
    // per module, so no name-matching is needed at all, unlike every
    // other case this function handles. Real example: ajv's own `dist/
    // core.d.ts`: `export default class Ajv { compile(...): ...;
    // validate(...): ...; ... }`, extended by the *entry* file's
    // `export declare class Ajv extends AjvCore { ... }` -- every one
    // of `AjvCore`'s own methods used to be completely unreachable,
    // since nothing followed that default import to find them. Returned
    // under the class's own real identifier (`Ajv`, not `"default"`,
    // which isn't a valid identifier to splice into a declaration) --
    // the caller (`dts_source_with_reexported_functions`) renames it to
    // the local alias its own `extends` clause actually names.
    if name == "default" {
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) = item else {
                continue;
            };
            let thaw_parser::ast::DefaultDecl::Class(class_expr) = &default_decl.decl else {
                continue;
            };
            if class_expr.ident.is_none() {
                continue;
            }
            superclass = class_expr.class.super_class.as_deref().and_then(|expr| match expr {
                thaw_parser::ast::Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            });
            let snippet = source_map.span_to_snippet(default_decl.span()).map_err(|error| {
                format!("failed to read default class declaration in `{}`: {error:?}", path.display())
            })?;
            declarations.push(snippet.replacen("export default ", "export declare ", 1));
        }
        if let Some(superclass) = superclass {
            if let Some((target_path, target_name)) =
                named_import_targets(path, &module).get(&superclass)
            {
                declarations.extend(reexported_class_or_interface_declarations_inner(
                    target_path,
                    target_name,
                    visited,
                )?);
            }
        }
        return Ok(declarations);
    }
    for item in &module.body {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) => Some(&declaration.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        let Some(declaration) = declaration else {
            continue;
        };
        let matches = match declaration {
            Decl::Class(class) => {
                if class.ident.sym == local_name {
                    superclass = class.class.super_class.as_deref().and_then(|expr| match expr {
                        thaw_parser::ast::Expr::Ident(ident) => Some(ident.sym.to_string()),
                        _ => None,
                    });
                    true
                } else {
                    false
                }
            }
            Decl::TsInterface(interface) => interface.id.sym == local_name,
            _ => false,
        };
        if matches {
            let mut snippet =
                source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| format!("failed to read declaration for `{name}`: {error:?}"))?;
            if local_name != name {
                snippet = rename_declared_function(snippet, name);
            }
            if !snippet.trim_start().starts_with("export ") {
                snippet = format!("export {snippet}");
            }
            declarations.push(snippet);
        }
    }
    if let Some(superclass) = superclass {
        if let Some((target_path, target_name)) = named_import_targets(path, &module).get(&superclass)
        {
            declarations.extend(reexported_class_or_interface_declarations_inner(
                target_path,
                target_name,
                visited,
            )?);
        }
    }
    Ok(declarations)
}

fn declaration_reexport_path(entry_path: &Path, source: &str) -> Option<PathBuf> {
    let path = if source == "." || source.starts_with("./") || source.starts_with("../") {
        entry_path.parent()?.join(source)
    } else {
        let node_modules = entry_path
            .ancestors()
            .find(|path| path.file_name().is_some_and(|name| name == "node_modules"))?;
        node_modules.join(source)
    };
    [path.with_extension("d.ts"), path.join("index.d.ts")]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// The name of the *type* that `export = X;`'s `X` is declared with in
/// `module` -- e.g. mime's `export = mime;` alongside `declare const
/// mime: Mime;` resolves to `"Mime"`. Mirrors
/// `export_assignment_function_declarations`'s own `export = X` lookup,
/// but reads the type annotation of a `declare const` binding rather
/// than a function body: `X` here names a *value* whose class lives
/// entirely in a different file (see `import_equals_targets`), not a
/// function declared locally.
fn export_assignment_value_type_name(module: &thaw_parser::ast::Module) -> Option<String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Pat, Stmt, TsEntityName, TsType};

    let target = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            match export.expr.as_ref() {
                Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            }
        }
        _ => None,
    })?;
    module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) = item else {
            return None;
        };
        var_decl.decls.iter().find_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            if binding.id.sym.as_str() != target {
                return None;
            }
            let annotation = binding.type_ann.as_ref()?;
            let TsType::TsTypeRef(ty_ref) = annotation.type_ann.as_ref() else {
                return None;
            };
            match &ty_ref.type_name {
                TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
                TsEntityName::TsQualifiedName(_) => None,
            }
        })
    })
}

/// Inlines the file backing a `declare const x: Name;` value's *type*
/// (`Name`) when `Name` isn't declared anywhere in this same file but
/// resolves through this file's own `import Name = require("./path")`
/// (see `import_equals_targets`) -- real-world example: mime's
/// `import Mime = require("./Mime"); ... declare const mime: Mime;
/// export = mime;`, `Mime` itself living in `./Mime.d.ts`, a *type*
/// reference across files rather than the *value* re-export
/// `import_equals_targets`'s other callers already follow. Conceptually
/// the same "read the referenced file and splice its declarations in"
/// pattern as `inline_triple_slash_references`, just reached through an
/// import-equals value binding's type annotation instead of a `///
/// <reference path="..." />` comment.
///
/// `Mime.d.ts` itself is a plain ES module (`export default class Mime
/// {...}`, with its own `import { TypeMap } from "./index"` back to the
/// entry file) rather than an ambient declaration -- only the `export
/// default class Name` prefix is rewritten to `declare class Name`
/// (the DefinitelyTyped convention for a single-class module); anything
/// else in the file, notably that `import`, is inlined unchanged since
/// thaw-bridge's own `.d.ts` processing already ignores any top-level
/// item it doesn't specifically look for.
fn inline_import_equals_value_type(
    module: &thaw_parser::ast::Module,
    import_equals_targets: &std::collections::HashMap<String, PathBuf>,
) -> Result<String, String> {
    let Some(type_name) = export_assignment_value_type_name(module) else {
        return Ok(String::new());
    };
    let Some(target_path) = import_equals_targets.get(&type_name) else {
        return Ok(String::new());
    };
    let source = fs::read_to_string(target_path).map_err(|error| {
        format!(
            "failed to read `{}`'s imported type `{type_name}`: {error}",
            target_path.display()
        )
    })?;
    let source = source.replacen(
        &format!("export default class {type_name}"),
        &format!("declare class {type_name}"),
        1,
    );
    Ok(format!("\n{source}\n"))
}
