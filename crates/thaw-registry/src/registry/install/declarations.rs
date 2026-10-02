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
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    base_offset: usize,
    views: &mut OwnedSourceViews,
) -> Result<String, String> {
    let namespace = export_as_namespace_name(entry_path, source)?;
    let canonical_entry = entry_path
        .canonicalize()
        .unwrap_or_else(|_| entry_path.to_path_buf());
    visited.insert(canonical_entry.clone());
    inline_triple_slash_references_inner(
        entry_path, source, &canonical_entry, namespace.as_deref(), visited,
        emitted, base_offset, views,
    )
}

fn append_referenced_owned_declaration(
    output: &mut String,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    base_offset: usize,
    origin: &Path,
    snippet: String,
) {
    output.push('\n');
    let start = base_offset + output.len();
    output.push_str(&snippet);
    let owned = OwnedDeclaration::new(origin, snippet);
    record_owned_namespace_children(emitted, &owned, None, Some(start));
    let public_names = emitted_public_names(
        &owned.snippet, owned.local_name.as_deref(), None,
    );
    emitted.push(EmittedOwnedDeclaration {
        declaration: owned, scope: None, public_names,
        start, end: base_offset + output.len(),
        metadata_only: false,
    });
}

fn inline_triple_slash_references_inner(
    current_path: &Path,
    source: &str,
    canonical_entry: &Path,
    namespace: Option<&str>,
    visited: &mut std::collections::BTreeSet<PathBuf>,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    base_offset: usize,
    views: &mut OwnedSourceViews,
) -> Result<String, String> {
    use thaw_parser::ast::{Decl, ModuleItem, Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};

    let mut output = String::new();
    for reference in triple_slash_reference_paths(source) {
        let Some(target_path) = current_path
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
            thaw_parser::parse_declarations_with_source_map_named(
                &referenced_source, thaw_parser::common::FileName::Real(target_path.clone()),
            )?;
        for item in &referenced_module.body {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
                // Referenced files also carry ordinary declarations. Their
                // source files disappear after flattening, so preserve the
                // original declarations alongside module augmentations.
                if matches!(item, ModuleItem::Stmt(Stmt::Decl(_))
                    | ModuleItem::ModuleDecl(thaw_parser::ast::ModuleDecl::ExportDecl(_))) {
                    let snippet = source_map.span_to_snippet(item.span()).map_err(|error| {
                        format!("failed to read referenced declaration: {error:?}")
                    })?;
                    append_referenced_owned_declaration(
                        &mut output, emitted, base_offset, &target_path, snippet,
                    );
                }
                continue;
            };
            let TsModuleName::Str(target) = &module_decl.id else {
                let snippet = source_map.span_to_snippet(item.span()).map_err(|error| {
                    format!("failed to read referenced namespace: {error:?}")
                })?;
                append_referenced_owned_declaration(
                    &mut output, emitted, base_offset, &target_path, snippet,
                );
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
                if let Some(namespace) = namespace {
                    let projection_start = output.len();
                    output.push_str(&format!("\ndeclare namespace {namespace} {{\n"));
                    for snippet in &snippets {
                        let start = base_offset + output.len();
                        output.push_str(snippet);
                        let mut owned = OwnedDeclaration::new(&target_path, snippet.clone());
                        fn prepend_scope(declaration: &mut OwnedDeclaration, scope: &str) {
                            declaration.source_scope.insert(0, scope.to_string());
                            for child in &mut declaration.children { prepend_scope(child, scope); }
                        }
                        prepend_scope(&mut owned, namespace);
                        record_owned_namespace_children(
                            emitted, &owned, Some(namespace), Some(start),
                        );
                        let public_names = emitted_public_names(
                            &owned.snippet, owned.local_name.as_deref(), None,
                        );
                        emitted.push(EmittedOwnedDeclaration {
                            declaration: owned, scope: Some(namespace.to_string()), public_names,
                            start, end: base_offset + output.len(),
                            metadata_only: false,
                        });
                        output.push('\n');
                    }
                    output.push_str("}\n");
                    views.append(&target_path, &output[projection_start..])?;
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
        let nested_start = base_offset + output.len();
        let nested = inline_triple_slash_references_inner(
            &target_path,
            &referenced_source,
            canonical_entry,
            namespace,
            visited,
            emitted,
            nested_start,
            views,
        )?;
        output.push_str(&nested);
    }
    Ok(output)
}

/// The `/// <reference path="..." />` directives in a `.d.ts` source --
/// these are ordinary comments as far as the parser is concerned (and,
/// per the TS convention this follows, only recognized at the very top of
/// the file), so this scans the raw text rather than the AST.
fn triple_slash_reference_paths(source: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut in_block_comment = false;
    for line in source.lines() {
        let mut rest = line.trim_start();
        loop {
            if in_block_comment {
                let Some(end) = rest.find("*/") else { break };
                rest = rest[end + 2..].trim_start();
                in_block_comment = false;
            }
            if rest.is_empty() { break; }
            if rest.starts_with("///") {
                if let Some(start) = rest.find("path=\"").map(|index| index + "path=\"".len())
                    && let Some(end) = rest[start..].find('"') {
                    paths.push(rest[start..start + end].to_string());
                }
                break;
            }
            if rest.starts_with("//") { break; }
            if let Some(after_open) = rest.strip_prefix("/*") {
                in_block_comment = true;
                rest = after_open;
                continue;
            }
            return paths;
        }
    }
    paths
}

/// `export as namespace <name>;` -- the name a `.d.ts` file's own exported
/// value is additionally reachable under as a global namespace, and (for
/// `inline_triple_slash_references`'s purposes) the namespace a referenced
/// file's self-targeting module augmentation actually means to extend.
fn export_as_namespace_name(entry_path: &Path, source: &str) -> Result<Option<String>, String> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem};

    let module = thaw_parser::parse_declarations_with_source_map_named(
        source, thaw_parser::common::FileName::Custom(format!("{} (declaration transform input)", entry_path.display()).into()),
    )?.0;
    Ok(module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsNamespaceExport(export)) => {
            Some(export.id.sym.to_string())
        }
        _ => None,
    }))
}

/// A package whose entire real API lives inside a TypeScript
/// `declare namespace X { ... }` block, exported wholesale via
/// `export = X;` -- real example: `winston`'s complete `.d.ts` is
/// exactly this shape (`declare namespace winston { class Logger
/// {...} let createLogger: ...; ... } export = winston;`). None of
/// this crate's own `.d.ts` extractors
/// (`thaw_bridge::parse_dts`/`parse_dts_values`/`parse_dts_classes`)
/// ever look inside a `TsModuleDecl`'s own body -- they only scan a
/// module's direct top-level items -- so a package shaped like this
/// silently produced zero declarations at all: no error, just every
/// one of its real functions/classes reported as "undeclared"
/// (or, for a bare value reference, "cannot infer the type") the
/// moment anything tried to use one.
///
/// Scoped to a namespace actually `export =`'d as the package's own
/// primary export (not just any incidentally-declared auxiliary
/// namespace with a different real export elsewhere) -- extracts each
/// of its own top-level items' raw source text via a span snippet
/// (this file's own established idiom for "make this look like it was
/// already at the top level", see `all_reexported_type_declarations`)
/// and appends them so every other extractor sees them as if they'd
/// been ordinary top-level declarations all along, rather than
/// modifying any of those extractors themselves.
fn hoisted_export_equals_namespace_members(
    entry_path: &Path,
    entry_source: &str,
    visited_namespace_wraps: &mut std::collections::BTreeSet<PathBuf>,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    base_offset: usize,
    views: &mut OwnedSourceViews,
) -> Result<String, String> {
    use thaw_parser::ast::{
        Decl, Expr, ModuleDecl, ModuleItem, Stmt, TsModuleName, TsNamespaceBody,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        entry_source, thaw_parser::common::FileName::Custom(format!("{} (declaration transform input)", entry_path.display()).into()),
    )?;
    let Some(exported_name) = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => match export.expr.as_ref()
        {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    }) else {
        return Ok(String::new());
    };
    let Some(namespace_body) = module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
            return None;
        };
        let TsModuleName::Ident(id) = &module_decl.id else {
            return None;
        };
        if id.sym.as_ref() != exported_name {
            return None;
        }
        match &module_decl.body {
            Some(TsNamespaceBody::TsModuleBlock(block)) => Some(&block.body),
            _ => None,
        }
    }) else {
        return Ok(String::new());
    };
    // A namespace member can itself be `export import NAME = BASE[.MEMBER];`
    // -- winston's real `.d.ts` is exactly this shape (`import * as logform
    // from 'logform'; declare namespace winston { export import format =
    // logform.format; export import transports = Transports; ... }`).
    // `named_import_targets` resolves `BASE` (here `logform`/`Transports`,
    // themselves ordinary top-level namespace imports of the entry file)
    // back to the file each was imported from -- the same lookup the
    // top-level `TsImportEquals` handling below already uses for a
    // qualified name at the *entry file's own* top level, reused here for
    // one nested inside the hoisted namespace instead, since a raw
    // source-text copy of `export import format = logform.format;` (this
    // function's fallback for every other member shape) means nothing to
    // any of this crate's `.d.ts` extractors: none of them resolve a
    // qualified reference into another *package's* file.
    let named_import_targets = named_import_targets(entry_path, &module);
    fn prepend_scope(declaration: &mut OwnedDeclaration, scope: &str) {
        declaration.source_scope.insert(0, scope.to_string());
        for child in &mut declaration.children { prepend_scope(child, scope); }
    }
    let mut output = String::new();
    for item in namespace_body {
        if let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item {
            if import.is_export {
                if let Some(snippet) = resolve_namespace_hoisted_import_equals(
                    import,
                    &named_import_targets,
                    visited_namespace_wraps,
                    emitted,
                    base_offset + output.len(),
                    entry_path,
                    &exported_name,
                    views,
                )? {
                    output.push_str(&snippet);
                    continue;
                }
            }
        }
        output.push('\n');
        let start = base_offset + output.len();
        let snippet = source_map.span_to_snippet(item.span()).map_err(|error| {
            format!("failed to read a namespace member: {error:?}")
        })?;
        output.push_str(&snippet);
        let mut owned = OwnedDeclaration::new(entry_path, snippet);
        prepend_scope(&mut owned, &exported_name);
        record_owned_namespace_children(emitted, &owned, None, Some(start));
        let public_names = emitted_public_names(
            &owned.snippet, owned.local_name.as_deref(), None,
        );
        emitted.push(EmittedOwnedDeclaration {
            declaration: owned, scope: None, public_names,
            start, end: base_offset + output.len(),
            metadata_only: false,
        });
    }
    Ok(output)
}

/// Resolves one namespace member shaped `export import NAME = BASE;` or
/// `export import NAME = BASE.MEMBER;` into real, already-understood
/// declaration text, or `None` to fall back to a raw source-text copy
/// (the caller's default for every other member shape, and for this shape
/// too when `BASE` can't be resolved at all -- e.g. it refers to something
/// declared in the same file rather than an import, a case not observed in
/// any real package yet).
fn resolve_namespace_hoisted_import_equals(
    import: &thaw_parser::ast::TsImportEqualsDecl,
    named_import_targets: &std::collections::HashMap<String, (PathBuf, String)>,
    visited_namespace_wraps: &mut std::collections::BTreeSet<PathBuf>,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    base_offset: usize,
    entry_path: &Path,
    source_namespace: &str,
    views: &mut OwnedSourceViews,
) -> Result<Option<String>, String> {
    use thaw_parser::ast::{TsEntityName, TsModuleRef};

    let exported = import.id.sym.to_string();
    let (base, member) = match &import.module_ref {
        TsModuleRef::TsEntityName(TsEntityName::TsQualifiedName(qualified)) => {
            let TsEntityName::Ident(base) = &qualified.left else {
                return Ok(None);
            };
            (base.sym.to_string(), Some(qualified.right.sym.to_string()))
        }
        TsModuleRef::TsEntityName(TsEntityName::Ident(base)) => (base.sym.to_string(), None),
        _ => return Ok(None),
    };
    let Some((target_path, target_member)) = named_import_targets.get(&base) else {
        return Ok(None);
    };
    let member = member.or_else(|| (!target_member.is_empty()).then(|| target_member.clone()));
    match member {
        Some(member) => {
            let mut visited = std::collections::BTreeSet::new();
            let mut declarations = exported_owned_value_declarations_with_views(
                target_path, &member, &mut visited, views,
            )?;
            if declarations.is_empty() {
                declarations = exported_owned_type_declarations(target_path, &member, views)?;
            }
            if declarations.is_empty() {
                return Ok(None);
            }
            let mut snippet = String::new();
            let selected = declarations.first().and_then(|declaration| declaration.local_name.as_ref()
                .map(|name| (declaration.origin.clone(), declaration.source_scope.clone(), name.clone())));
            for mut declaration in declarations {
                if exported != member {
                    declaration.snippet = rename_declared_function(declaration.snippet, &exported);
                }
                snippet.push('\n');
                let start = base_offset + snippet.len();
                snippet.push_str(&declaration.snippet);
                let public_name = selected.as_ref().is_some_and(|key| declaration.local_name.as_ref()
                    .is_some_and(|name| &(declaration.origin.clone(), declaration.source_scope.clone(), name.clone()) == key))
                    .then_some(exported.as_str());
                record_owned_namespace_children(emitted, &declaration, None, Some(start));
                emitted.push(EmittedOwnedDeclaration {
                    public_names: emitted_public_names(
                        &declaration.snippet, declaration.local_name.as_deref(), public_name,
                    ),
                    declaration, scope: None,
                    start, end: base_offset + snippet.len(),
                    metadata_only: false,
                });
            }
            Ok(Some(snippet))
        }
        None => {
            // A bare `export import NAME = BASE;` where `BASE` is itself a
            // whole namespace import (`import * as BASE from "pkg"`) --
            // real example: winston's `export import transports =
            // Transports;` (`import * as Transports from
            // './lib/winston/transports/index'`), later used as
            // `transports.Console`. Wraps the target file's own fully
            // flattened declarations in `declare namespace NAME { ... }` so
            // a later dotted access resolves the same way any other
            // `declare namespace` block already does.
            if !visited_namespace_wraps.insert(target_path.clone()) {
                return Ok(None);
            }
            let target_source = fs::read_to_string(target_path).map_err(|error| {
                format!(
                    "failed to read a namespace-aliased module `{}`: {error}",
                    target_path.display()
                )
            })?;
            let mut target_metadata = FlattenedOwnedMetadata::default();
            let _flattened = dts_source_with_reexported_functions_inner_with_metadata(
                target_path,
                &target_source,
                visited_namespace_wraps,
                Some(&mut target_metadata),
                true,
            )?;
            for (origin, source) in &target_metadata.views.by_origin {
                views.by_origin.entry(origin.clone()).or_insert_with(|| source.clone());
            }
            let mut snippet = String::new();
            let members = exported_const_object_properties(target_path, views)?;
            let selected = if members.is_empty() {
                selected_namespace_bindings(target_path, &target_metadata.emitted, views)?
            } else {
                Vec::new()
            };
            if members.is_empty() {
                let selected_records = selected.iter().flat_map(|binding|
                    binding.fragments.iter().filter(move |declaration|
                        !(!binding.value_export
                            && is_type_only_namespace_binding(declaration, &binding.public)))
                        .cloned()).map(|declaration|
                    EmittedOwnedDeclaration {
                        declaration, scope: None, public_names: Vec::new(),
                        start: 0, end: 0, metadata_only: false,
                    }).collect();
                append_owned_origin_wrappers(
                    &mut snippet, emitted, selected_records,
                    views, base_offset,
                );
            }
            // A target's `export = X;` may expose properties of an object
            // interface rather than direct declarations. Read those
            // properties from X's original source view: flattened names
            // can collide with entry declarations and erase generic
            // arguments or typeof value roles.
            let mut public_members = Vec::new();
            for (property, declaration) in &members {
                public_members.push(declaration.snippet.clone());
                views.composite_aliases.insert((
                    entry_path.canonicalize().unwrap_or_else(|_| entry_path.to_path_buf()),
                    vec![source_namespace.to_string()], vec![exported.clone(), property.clone()],
                ), CompositeAlias {
                    fragments: vec![declaration.clone()],
                    value_export: true,
                });
            }
            let canonical_target = target_path.canonicalize().unwrap_or_else(|_| target_path.to_path_buf());
            let mut nested_members = Vec::new();
            for ((origin, _, path), alias) in &target_metadata.views.composite_aliases {
                if origin == &canonical_target && path.len() >= 2 {
                    let [declaration] = alias.fragments.as_slice() else {
                        return Err(format!("ambiguous nested composite property in `{}`",
                            target_path.display()));
                    };
                    let public = path.last().expect("composite path has a member");
                    if declaration.local_name.as_deref() != Some(public) {
                        return Err(format!("renamed nested composite property `{public}` in `{}`",
                            target_path.display()));
                    }
                    let mut member = declaration.snippet.clone();
                    let mut local_start = 0;
                    for scope in path[..path.len() - 1].iter().rev() {
                        let prefix = format!("export namespace {scope} {{\n");
                        local_start += prefix.len();
                        member = format!("{prefix}{member}\n}}");
                    }
                    if !nested_members.iter().any(|(existing, _, _, _, _)| existing == &member) {
                        let emitted_scope = std::iter::once(exported.as_str())
                            .chain(path[..path.len() - 1].iter().map(String::as_str))
                            .collect::<Vec<_>>().join(".");
                        nested_members.push((member, local_start, declaration.clone(),
                            emitted_scope, public.clone()));
                    }
                    let mut outer_path = vec![exported.clone()];
                    outer_path.extend(path.iter().cloned());
                    views.composite_aliases.insert((
                        entry_path.canonicalize().unwrap_or_else(|_| entry_path.to_path_buf()),
                        vec![source_namespace.to_string()], outer_path,
                    ), alias.clone());
                }
            }
            for (key, alias) in target_metadata.views.composite_aliases {
                views.composite_aliases.entry(key).or_insert(alias);
            }
            if !members.is_empty() {
                snippet.push_str(&format!("\ndeclare namespace {exported} {{\n"));
                for ((property, declaration), public_member) in members.iter().zip(public_members.iter()) {
                    snippet.push_str("    ");
                    let start = base_offset + snippet.len();
                    snippet.push_str(public_member);
                    emitted.push(EmittedOwnedDeclaration {
                        declaration: declaration.clone(), scope: Some(exported.clone()),
                        public_names: vec![property.clone()],
                        start, end: base_offset + snippet.len(), metadata_only: false,
                    });
                    snippet.push('\n');
                }
                snippet.push_str("}\n");
            }
            if !nested_members.is_empty() {
                snippet.push_str(&format!("\ndeclare namespace {exported} {{\n"));
                for (member, local_start, declaration, emitted_scope, public) in nested_members {
                    let start = base_offset + snippet.len() + local_start;
                    let end = start + declaration.snippet.len();
                    snippet.push_str(&member);
                    emitted.push(EmittedOwnedDeclaration {
                        declaration, scope: Some(emitted_scope),
                        public_names: vec![public], start, end, metadata_only: false,
                    });
                    snippet.push('\n');
                }
                snippet.push_str("}\n");
            }
            if !selected.is_empty() {
                snippet.push_str(&format!("\ndeclare namespace {exported} {{\n"));
                for binding in selected {
                    let declaration = binding.fragments.first().ok_or_else(||
                        format!("missing selected namespace fragment `{}`", binding.public))?;
                    let local = declaration.local_name.as_ref().ok_or_else(||
                        format!("missing selected namespace name `{}`", binding.public))?;
                    let type_namespace = !binding.value_export
                        && is_type_only_namespace_binding(declaration, &binding.public);
                    let alias = if type_namespace {
                        type_only_namespace_public_member(&declaration.snippet,
                            &binding.public, &declaration.origin)?
                    } else {
                        let owner = views.owned_names.get(&declaration.origin).ok_or_else(||
                            format!("missing selected namespace owner `{}`", declaration.origin.display()))?;
                        let mut target = owner.clone();
                        for scope in &declaration.source_scope {
                            target.push('.');
                            target.push_str(scope);
                        }
                        target.push('.');
                        target.push_str(local);
                        if binding.value_export {
                            format!("export import {} = {target};", binding.public)
                        } else {
                            let (parameters, forwarding) =
                                owned_type_parameter_forwarding(declaration)?;
                            format!("export type {}{parameters} = {target}{forwarding};",
                                binding.public)
                        }
                    };
                    let start = base_offset + snippet.len();
                    snippet.push_str(&alias);
                    let mut owned = declaration.clone().with_snippet(alias);
                    if !type_namespace { owned.children.clear(); }
                    if type_namespace {
                        record_owned_namespace_children(emitted, &owned,
                            Some(&exported), Some(start));
                    }
                    emitted.push(EmittedOwnedDeclaration {
                        declaration: owned, scope: Some(exported.clone()),
                        public_names: vec![binding.public],
                        start, end: base_offset + snippet.len(),
                        metadata_only: false,
                    });
                    snippet.push('\n');
                }
                snippet.push_str("}\n");
            }
            Ok(Some(snippet))
        }
    }
}

fn parse_labeled_declarations(source: &str, label: String) -> Result<thaw_parser::ast::Module, String> {
    thaw_parser::parse_declarations_with_source_map_named(
        source, thaw_parser::common::FileName::Custom(label.into()),
    ).map(|(module, _)| module)
}

/// Public object properties selected from the original `export = X`
/// declaration. Each synthetic const retains the source scope where its
/// annotation resolves; an instantiated interface projects a property
/// through indexed access so generic type arguments remain bound.
fn exported_const_object_properties(
    origin: &Path, views: &OwnedSourceViews,
) -> Result<Vec<(String, OwnedDeclaration)>, String> {
    use thaw_parser::ast::{
        Decl, Expr, Ident, Lit, MemberProp, ModuleDecl, ModuleItem, Pat, Stmt, TsEntityName, TsType, TsTypeElement,
        TsUnionOrIntersectionType,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let source = views.read(origin)?;
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(origin.to_path_buf()),
    )?;
    let Some(exported) = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            match export.expr.as_ref() {
                Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            }
        }
        _ => None,
    }) else { return Ok(Vec::new()) };
    let variable = module.body.iter().find_map(|item| {
        let declaration = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(variable))) => Some(variable.as_ref()),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) =>
                if let Decl::Var(variable) = &export.decl { Some(variable.as_ref()) } else { None },
            _ => None,
        }?;
        declaration.decls.iter().find_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            (binding.id.sym.as_ref() == exported).then_some(binding)
        })
    });
    let Some(binding) = variable else { return Ok(Vec::new()) };
    let Some(annotation) = &binding.type_ann else { return Ok(Vec::new()) };
    let TsType::TsTypeRef(reference) = annotation.type_ann.as_ref() else {
        return Ok(Vec::new());
    };
    fn segments(name: &TsEntityName) -> Vec<String> {
        match name {
            TsEntityName::Ident(ident) => vec![ident.sym.to_string()],
            TsEntityName::TsQualifiedName(qualified) => {
                let mut parts = segments(&qualified.left);
                parts.push(qualified.right.sym.to_string());
                parts
            }
        }
    }
    fn heritage_segments(expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::Ident(ident) => Some(vec![ident.sym.to_string()]),
            Expr::Member(member) => {
                let mut parts = heritage_segments(&member.obj)?;
                let MemberProp::Ident(name) = &member.prop else { return None };
                parts.push(name.sym.to_string());
                Some(parts)
            }
            _ => None,
        }
    }
    fn alias_members_and_references<'a>(
        annotation: &'a TsType,
        members: &mut Vec<&'a TsTypeElement>,
        references: &mut Vec<Vec<String>>,
    ) {
        match annotation {
            TsType::TsTypeLit(literal) => members.extend(&literal.members),
            TsType::TsTypeRef(reference) => references.push(segments(&reference.type_name)),
            TsType::TsParenthesizedType(parenthesized) =>
                alias_members_and_references(&parenthesized.type_ann, members, references),
            TsType::TsUnionOrIntersectionType(
                TsUnionOrIntersectionType::TsIntersectionType(intersection)) => {
                for member in &intersection.types {
                    alias_members_and_references(member, members, references);
                }
            }
            _ => {}
        }
    }
    let fragments = resolve_owned_qualified_source_type_reference_with_views(
        origin, &[], &segments(&reference.type_name), views,
    )?;
    let object_annotation = source_map.span_to_snippet(annotation.type_ann.span())
        .map_err(|error| format!("failed to read export-assignment type: {error:?}"))?;
    let instantiated = reference.type_params.as_ref().is_some_and(|params|
        !params.params.is_empty());
    let mut candidates = Vec::<(String, String, PathBuf, Vec<String>, bool)>::new();
    let mut pending = std::collections::VecDeque::from_iter(fragments.into_iter()
        .map(|fragment| (fragment, false)));
    let mut visited = std::collections::BTreeSet::new();
    while let Some((fragment, through_alias)) = pending.pop_front() {
        if !visited.insert((fragment.origin.clone(), fragment.source_scope.clone(),
            fragment.local_name.clone(), fragment.snippet.clone())) { continue; }
        let (parsed, snippet_map) = thaw_parser::parse_declarations_with_source_map_named(
            &fragment.snippet,
            thaw_parser::common::FileName::Custom(
                format!("{} (owned object type)", fragment.origin.display()).into(),
            ),
        )?;
        for item in &parsed.body {
            let declaration = match item {
                ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
                _ => None,
            };
            let mut members = Vec::new();
            let mut references = Vec::new();
            let (generic, alias) = match declaration {
                Some(Decl::TsInterface(interface)) => {
                    members.extend(&interface.body.body);
                    references.extend(interface.extends.iter().filter_map(|parent|
                        heritage_segments(&parent.expr)));
                    (interface.type_params.is_some(), false)
                }
                Some(Decl::TsTypeAlias(alias)) => {
                    alias_members_and_references(&alias.type_ann, &mut members, &mut references);
                    (alias.type_params.is_some(), true)
                }
                _ => continue,
            };
            for reference in references {
                for reached in resolve_owned_qualified_source_type_reference_with_views(
                    &fragment.origin, &fragment.source_scope, &reference, views,
                )? {
                    pending.push_back((reached, true));
                }
            }
            for member in members {
                let TsTypeElement::TsPropertySignature(property) = member else { continue };
                if property.computed { continue; }
                let property_name = match property.key.as_ref() {
                    Expr::Ident(key) => key.sym.to_string(),
                    Expr::Lit(Lit::Str(key)) => match key.value.as_str() {
                        Some(name) => name.to_string(), None => continue,
                    },
                    _ => continue,
                };
                if Ident::verify_symbol(&property_name).is_err() { continue; }
                let Some(property_annotation) = &property.type_ann else { continue };
                let declared_type = snippet_map.span_to_snippet(property_annotation.type_ann.span())
                    .map_err(|error| format!("failed to read object property type: {error:?}"))?;
                let indexed = instantiated || generic || alias || through_alias;
                candidates.push((property_name, declared_type, fragment.origin.clone(),
                    fragment.source_scope.clone(), indexed));
            }
        }
    }
    // If any branch is inherited, aliased, or instantiated, project every
    // property through the same root. Otherwise a valid non-generic override
    // can compare raw `number` against `Members["id"]` and look conflicting.
    let project_root = candidates.iter().any(|(_, _, _, _, indexed)| *indexed);
    let mut properties = std::collections::BTreeMap::<String, (String, String, OwnedDeclaration)>::new();
    for (property_name, declared_type, source_origin, source_scope, indexed) in candidates {
        let indexed = project_root || indexed;
        let effective_type = if indexed {
            format!("{object_annotation}[\"{property_name}\"]")
        } else {
            declared_type.clone()
        };
        let mut synthetic = OwnedDeclaration::new(
            if indexed { origin } else { &source_origin },
            format!("export const {property_name}: {effective_type};"),
        );
        synthetic.source_scope = if indexed { Vec::new() } else { source_scope };
        if let Some((previous, previous_effective, _)) = properties.get(&property_name) {
            if (!indexed && previous != &declared_type)
                || previous_effective != &effective_type {
                return Err(format!("conflicting object property `{property_name}` in `{}`",
                    origin.display()));
            }
        } else {
            properties.insert(property_name, (declared_type, effective_type, synthetic));
        }
    }
    Ok(properties.into_iter().map(|(name, (_, _, declaration))| (name, declaration)).collect())
}

/// A `.d.cts` (CommonJS) entry that delegates its whole API to a sibling
/// ESM declaration file via `declare const X: typeof import("./x.mjs").
/// default; export = X;` -- real example: markdown-it's
/// `dist/markdown-it.d.cts`, which only re-exports the namespace types and
/// the default from `./markdown-it.mjs` (`dist/markdown-it.d.mts`). The
/// delegated file holds the real `declare class`/`declare const`, so
/// without following it `new MarkdownIt()` had no constructor to resolve
/// ("only `new Promise<T>(...)` is supported").
///
/// Returns the sibling file's own path and source, so subsequent relative
/// resolution happens against *that* file. Only the exact
/// `typeof import("<relative>").default` delegation shape is followed;
/// anything else is left untouched.
fn dts_delegation_target(entry_path: &Path, entry_source: &str) -> Option<(PathBuf, String)> {
    use thaw_parser::ast::{
        Decl, Expr, ModuleDecl, ModuleItem, Pat, Stmt, TsEntityName, TsType, TsTypeQueryExpr,
    };

    let module = thaw_parser::parse_declarations_with_source_map_named(
        entry_source, thaw_parser::common::FileName::Real(entry_path.to_path_buf()),
    ).ok()?.0;
    let exported = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => match export.expr.as_ref()
        {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    })?;
    let specifier = module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) = item else {
            return None;
        };
        var_decl.decls.iter().find_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            if binding.id.sym.as_str() != exported {
                return None;
            }
            let annotation = binding.type_ann.as_ref()?;
            let TsType::TsTypeQuery(query) = annotation.type_ann.as_ref() else {
                return None;
            };
            let TsTypeQueryExpr::Import(import) = &query.expr_name else {
                return None;
            };
            let is_default = matches!(
                import.qualifier.as_ref(),
                Some(TsEntityName::Ident(ident)) if ident.sym == "default"
            );
            (is_default)
                .then(|| import.arg.value.as_str().map(str::to_string))
                .flatten()
        })
    })?;
    // Resolve `./impl.mjs` to `./impl.d.mts` (not `./impl.d.ts`), which
    // `declaration_reexport_path` alone would miss.
    let sibling = with_explicit_d_ts_suffix(&entry_path.parent()?.join(&specifier))
        .filter(|candidate| candidate.is_file())?;
    let source = fs::read_to_string(&sibling).ok()?;
    Some((sibling, source))
}

fn dts_source_with_reexported_functions(
    entry_path: &Path,
    entry_source: &str,
) -> Result<String, String> {
    let (entry_path, entry_source) = match dts_delegation_target(entry_path, entry_source) {
        Some((path, source)) => (path, source),
        None => (entry_path.to_path_buf(), entry_source.to_string()),
    };
    let mut visited_namespace_wraps = std::collections::BTreeSet::new();
    dts_source_with_reexported_functions_inner(
        &entry_path,
        &entry_source,
        &mut visited_namespace_wraps,
    )
}

/// Unwraps a top-level ambient container block, hoisting its body to the
/// top level. Two shapes:
///
/// - A `declare module "<own name>" { ... }` block covering the whole
///   declaration file. Real-world example: highlight.js's
///   `types/index.d.ts` is nothing but `declare module
///   'highlight.js/private' { ... }` followed by `declare module
///   'highlight.js' { ...; const hljs: HLJSApi; export default hljs; }`.
///   thaw-bridge only reads a `.d.ts`'s *top-level* exports, so the whole
///   API (including its default export) stayed invisible and every
///   `hljs.someMethod(...)` failed with "call to unknown function". Only a
///   block whose specifier resolves back to *this same file*
///   (`declaration_reexport_path`, which already resolves a bare package
///   name to its own entry point) is unwrapped; a sibling ambient module
///   for a subpath, a wildcard, or an unrelated package is kept exactly as
///   written.
/// - A `declare global { ... }` augmentation, unwrapped unconditionally
///   (it names no module). Real-world example: `@types/crypto-js`, whose
///   entire `namespace CryptoJS { ... }` body is inside one, so `export =
///   CryptoJS;` had no top-level namespace to expose members from and
///   every `CryptoJS.SHA256(...)` was unknown.
///
/// The block's own `import { ... } from "<own name>/private"` is left in
/// place and unresolved, which is harmless: those types are never part of
/// the bridgeable surface.
fn unwrap_self_ambient_module(entry_path: &Path, entry_source: &str) -> Result<String, String> {
    use thaw_parser::ast::{Decl, ModuleItem, Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};

    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        entry_source, thaw_parser::common::FileName::Custom(format!("{} (declaration transform input)", entry_path.display()).into()),
    )?;
    let canonical_entry = entry_path
        .canonicalize()
        .unwrap_or_else(|_| entry_path.to_path_buf());
    let mut output = entry_source.to_string();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
            continue;
        };
        // A top-level `declare global { ... }` augmentation -- hoist its
        // contents unconditionally. Real example: `@types/crypto-js` wraps
        // its *entire* `namespace CryptoJS { ... }` body in one, so with
        // nothing unwrapping it, `export = CryptoJS;` had no namespace to
        // resolve members against and every `CryptoJS.SHA256(...)` failed
        // with "call to unknown function". The rest of this function's
        // self-targeting `declare module "<own name>"` rule doesn't apply
        // (a global augmentation names no module).
        let unwrap = if module_decl.global {
            true
        } else {
            let TsModuleName::Str(target) = &module_decl.id else {
                continue;
            };
            let Some(specifier) = target.value.as_str() else {
                continue;
            };
            declaration_reexport_path(entry_path, specifier)
                .map(|resolved| resolved.canonicalize().unwrap_or(resolved))
                .is_some_and(|resolved| resolved == canonical_entry)
        };
        if !unwrap {
            continue;
        }
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
            continue;
        };
        let wrapper = source_map
            .span_to_snippet(module_decl.span())
            .map_err(|error| format!("failed to read ambient module declaration: {error:?}"))?;
        let mut unwrapped = String::new();
        for body_item in &block.body {
            unwrapped.push_str(
                &source_map
                    .span_to_snippet(body_item.span())
                    .map_err(|error| format!("failed to read ambient module body: {error:?}"))?,
            );
            unwrapped.push('\n');
        }
        output = output.replacen(&wrapper, &unwrapped, 1);
    }
    Ok(output)
}

fn dts_source_with_reexported_functions_inner(
    entry_path: &Path,
    entry_source: &str,
    visited_namespace_wraps: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<String, String> {
    dts_source_with_reexported_functions_inner_with_metadata(
        entry_path, entry_source, visited_namespace_wraps, None, false,
    )
}

fn dts_source_with_reexported_functions_inner_with_metadata(
    entry_path: &Path,
    entry_source: &str,
    visited_namespace_wraps: &mut std::collections::BTreeSet<PathBuf>,
    metadata: Option<&mut FlattenedOwnedMetadata>,
    defer_closure: bool,
) -> Result<String, String> {
    use thaw_parser::ast::{
        Decl, ExportSpecifier, Expr, MemberProp, ModuleDecl, ModuleExportName, ModuleItem, Stmt,
    };

    let unwrapped = unwrap_self_ambient_module(entry_path, entry_source)?;
    let entry_source: &str = &unwrapped;
    let module = parse_labeled_declarations(entry_source, format!("{} (unwrapped declaration entry)", entry_path.display()))?;
    let mut source_views = OwnedSourceViews::default();
    source_views.set(entry_path, entry_source.to_string());
    let mut output = entry_source.to_string();
    let mut emitted_owned = Vec::<EmittedOwnedDeclaration>::new();
    let mut materialized_class_imports = std::collections::BTreeSet::new();
    let hoisted_start = output.len();
    let hoisted = hoisted_export_equals_namespace_members(
        entry_path,
        entry_source,
        visited_namespace_wraps,
        &mut emitted_owned,
        hoisted_start,
        &mut source_views,
    )?;
    output.push_str(&hoisted);
    let mut visited_references = std::collections::BTreeSet::new();
    let reference_start = output.len();
    let referenced = inline_triple_slash_references(
        entry_path,
        entry_source,
        &mut visited_references,
        &mut emitted_owned,
        reference_start,
        &mut source_views,
    )?;
    output.push_str(&referenced);
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
    if let Some((origin, text)) = inline_import_equals_value_type(
        &module,
        &import_equals_targets,
    )? {
        let start = output.len();
        output.push_str(&text);
        record_owned_entry_declarations(&mut emitted_owned, &origin, &text, start)?;
    }
    for (origin, text) in inline_import_equals_referenced_types(
        &module,
        &import_equals_targets,
        export_assignment_value_type_name(&module).as_deref(),
    )? {
        let start = output.len();
        output.push_str(&text);
        record_owned_entry_declarations(&mut emitted_owned, &origin, &text, start)?;
    }
    let named_import_targets = named_import_targets(entry_path, &module);
    let named_type_import_targets = named_type_import_targets(entry_path, &module);
    let mut local_export_visited = std::collections::BTreeSet::new();
    let local_exports = all_reexported_function_declarations_owned(
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
            if export.src.is_none()
                && !named_import_targets.contains_key(&original)
                && named_type_import_targets.contains_key(&original) {
                continue;
            }
            if export.src.is_none() {
                if let Some((target, imported)) = named_import_targets.get(&original) {
                    let mut visited = std::collections::BTreeSet::new();
                    if reexport_is_type_only(target, imported, &mut visited)? == Some(true) {
                        continue;
                    }
                }
            }
            if let Some(target) = &target_path {
                let mut visited = std::collections::BTreeSet::new();
                if reexport_is_type_only(target, &original, &mut visited)? == Some(true) {
                    continue;
                }
            }
            let exported = named
                .exported
                .as_ref()
                .and_then(export_name)
                .unwrap_or_else(|| original.clone());
            if seen.contains(&exported) {
                continue;
            }
            // A same-file binding already has its declaration in the entry
            // source. Imported bindings still need following even when the
            // export specifier has no `as` clause.
            if export.src.is_none() && exported == original
                && !named_import_targets.contains_key(&original)
                && !import_equals_targets.contains_key(&original) {
                continue;
            }
            let declarations = match &target_path {
                Some(target_path) => {
                    let mut visited = std::collections::BTreeSet::new();
                    let functions =
                        reexported_function_declarations_owned(target_path, &original, &mut visited)?;
                    if !functions.is_empty() {
                        functions
                    } else {
                        reexported_class_or_interface_declarations_owned(target_path, &original)?
                    }
                }
                None => match import_equals_targets.get(&original) {
                    Some(target_path) => {
                        let functions = export_assignment_function_declarations_owned(target_path)?;
                        if !functions.is_empty() {
                            functions
                        } else {
                            export_assignment_class_or_interface_declarations_owned(target_path)?
                        }
                    }
                    None => match named_import_targets.get(&original) {
                        Some((target_path, target_name)) => {
                            let mut visited = std::collections::BTreeSet::new();
                            let functions = reexported_function_declarations_owned(
                                target_path,
                                target_name,
                                &mut visited,
                            )?;
                            if !functions.is_empty() {
                                functions
                            } else {
                                reexported_class_or_interface_declarations_owned(
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
            let selected = declarations.first().and_then(|declaration| declaration.local_name.as_ref()
                .map(|name| (declaration.origin.clone(), name.clone())));
            for declaration in reexported_owned_declarations_as(declarations, &exported, entry_path, false) {
                let is_type = is_type_declaration_snippet(&declaration.snippet);
                let public = selected.as_ref().is_some_and(|(origin, name)|
                    origin == &declaration.origin && Some(name) == declaration.local_name.as_ref());
                append_owned_declaration(
                    &mut output, &mut emitted_owned, declaration, None,
                    public.then_some(exported.as_str()), is_type,
                );
            }
        }
    }
    // Named type re-exports have no runtime binding, but the target's
    // declaration must still be present for class instances and aliases.
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.type_only {
            if let Some(target) = export.src.as_ref()
                .and_then(|source| source.value.as_str())
                .and_then(|source| declaration_reexport_path(entry_path, source)) {
                for specifier in &export.specifiers {
                    let ExportSpecifier::Namespace(namespace) = specifier else { continue };
                    let ModuleExportName::Ident(alias) = &namespace.name else { continue };
                    let mut visited = std::collections::BTreeSet::new();
                    let children = all_reexported_type_declarations_owned(&target, &mut visited)?;
                    let snippet = type_only_namespace_declaration(
                        alias.sym.as_ref(), children.iter().map(|child| child.snippet.clone()).collect(),
                    );
                    let mut declaration = OwnedDeclaration::new(entry_path, snippet);
                    declaration.children = children;
                    append_owned_declaration(
                        &mut output, &mut emitted_owned, declaration, None,
                        Some(alias.sym.as_ref()), true,
                    );
                }
            }
        }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else { continue };
            let ModuleExportName::Ident(original) = &named.orig else { continue };
            let imported_only_as_type = export.src.is_none()
                && !named_import_targets.contains_key(original.sym.as_ref())
                && named_type_import_targets.contains_key(original.sym.as_ref());
            let source_target = export.src.as_ref()
                .and_then(|source| source.value.as_str())
                .and_then(|source| declaration_reexport_path(entry_path, source))
                .map(|target| (target, original.sym.to_string()))
                .or_else(|| named_import_targets.get(original.sym.as_ref()).cloned());
            let source_only_as_type = if let Some((target, imported)) = source_target {
                let mut visited = std::collections::BTreeSet::new();
                reexport_is_type_only(&target, &imported, &mut visited)? == Some(true)
            } else { false };
            if !export.type_only && !named.is_type_only
                && !imported_only_as_type && !source_only_as_type { continue; }
            let public = named.exported.as_ref().unwrap_or(&named.orig);
            let ModuleExportName::Ident(public) = public else { continue };
            let target = export.src.as_ref()
                .and_then(|source| source.value.as_str())
                .and_then(|source| declaration_reexport_path(entry_path, source))
                .or_else(|| named_import_targets.get(original.sym.as_ref()).map(|(path, _)| path.clone()))
                .or_else(|| named_type_import_targets.get(original.sym.as_ref()).map(|(path, _)| path.clone()));
            if let Some(target) = target {
                let target_name = named_import_targets.get(original.sym.as_ref())
                    .filter(|_| export.src.is_none())
                    .or_else(|| named_type_import_targets.get(original.sym.as_ref()).filter(|_| export.src.is_none()))
                    .map_or(original.sym.as_ref(), |(_, name)| name.as_str());
                let declarations = reexported_class_or_interface_declarations_owned(&target, target_name)?;
                let selected = declarations.first().and_then(|declaration| declaration.local_name.as_ref()
                    .map(|name| (declaration.origin.clone(), name.clone())));
                for declaration in reexported_owned_declarations_as(
                    declarations, public.sym.as_ref(), entry_path, true,
                ) {
                    let is_public = selected.as_ref().is_some_and(|(origin, name)|
                        origin == &declaration.origin && Some(name) == declaration.local_name.as_ref());
                    append_owned_declaration(
                        &mut output, &mut emitted_owned, declaration, None,
                        is_public.then_some(public.sym.as_ref()), true,
                    );
                }
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
        let declarations = reexported_function_declarations_owned(target_path, &member, &mut visited)?;
        if !declarations.is_empty() {
            seen.insert(exported.clone());
        }
        for mut declaration in declarations {
            if exported != member {
                let text = export_function_as(declaration.snippet.clone(), &exported, entry_path);
                declaration = declaration.with_snippet(text);
            }
            append_owned_declaration(
                &mut output, &mut emitted_owned, declaration, None, Some(&exported), false,
            );
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
        for mut declaration in all_reexported_type_declarations_owned(&target_path, &mut visited_types)? {
            if export.type_only {
                declaration.snippet = type_only_declaration(declaration.snippet, None);
            }
            let public_name = declaration.local_name.clone();
            append_owned_declaration(
                &mut output, &mut emitted_owned, declaration, None,
                public_name.as_deref(), true,
            );
        }
        if export.type_only {
            continue;
        }
        let mut visited = std::collections::BTreeSet::new();
        let declarations = all_reexported_function_declarations_owned(&target_path, &mut visited)?;
        let names = declarations
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !seen.contains(name))
            .collect::<std::collections::BTreeSet<_>>();
        for (name, declaration) in declarations {
            if names.contains(&name) {
                append_owned_declaration(
                    &mut output, &mut emitted_owned, declaration, None, Some(&name), false,
                );
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
    // Keep each member directly inside its namespace. This preserves
    // distinct `number` members in the package and `coerce`, and lets a
    // function signature refer to sibling types such as `Options` or
    // `Client` without losing the declaration's lexical scope. The bridge
    // reads direct namespace functions and classes under qualified names.
    let mut namespace_visited = std::collections::BTreeSet::new();
    for (alias, target_path) in
        collect_namespace_reexports(entry_path, &module, &mut namespace_visited, false)?
    {
        let mut visited_functions = std::collections::BTreeSet::new();
        let functions = all_reexported_function_declarations_owned(&target_path, &mut visited_functions)?;
        let mut visited_types = std::collections::BTreeSet::new();
        let types = all_reexported_type_declarations_owned(&target_path, &mut visited_types)?;
        if functions.is_empty() && types.is_empty() {
            continue;
        }
        output.push_str(&format!("\ndeclare namespace {alias} {{\n"));
        let mut seen_members = std::collections::BTreeSet::new();
        for declaration in types.into_iter().chain(functions.into_iter().map(|(_, declaration)| declaration)) {
            for member in namespace_member_declarations_owned(declaration)? {
                if seen_members.insert(member.snippet.clone()) {
                    let start = output.len();
                    output.push_str(&member.snippet);
                    output.push('\n');
                    emitted_owned.push(EmittedOwnedDeclaration {
                        public_names: namespace_member_public_names(&member),
                        declaration: member,
                        scope: Some(alias.clone()),
                        start,
                        end: output.len(),
                        metadata_only: false,
                    });
                }
            }
        }
        output.push_str("}\n");
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
        for declaration in all_reexported_type_declarations_owned(&target_path, &mut visited_types)? {
            let public_name = declaration.local_name.clone();
            append_owned_declaration(
                &mut output, &mut emitted_owned, declaration, None,
                public_name.as_deref(), true,
            );
        }
        let mut visited = std::collections::BTreeSet::new();
        for (name, declaration) in all_reexported_function_declarations_owned(&target_path, &mut visited)? {
            append_owned_declaration(
                &mut output, &mut emitted_owned, declaration, None, Some(&name), false,
            );
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
    let mut materialized_builtin_names =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
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
                let declarations =
                    reexported_class_or_interface_declarations_owned(target_path, target_name)?;
                if declarations.is_empty() { continue }
                if !materialized_class_imports.insert(base.sym.to_string()) { continue }
                for declaration in imported_owned_class_as_local_binding(
                    declarations, base.sym.as_ref(), true, target_path,
                ) {
                    append_owned_declaration(
                        &mut output, &mut emitted_owned, declaration, None, None, false,
                    );
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
                let builtin_name = specifier.strip_prefix("node:").unwrap_or(specifier);
                let declarations = builtin_class_and_ancestor_declarations(
                    &builtin.dts_source,
                    specifier,
                    base_name.sym.as_ref(),
                    materialized_builtin_names.entry(builtin_name.to_string()).or_default(),
                )?;
                for snippet in declarations {
                    output.push('\n');
                    output.push_str(&snippet);
                }
            }
            _ => continue,
        }
    }
    let retained_entry = without_materialized_class_imports_named(
        entry_source, &materialized_class_imports,
        thaw_parser::common::FileName::Custom(format!("{} (unwrapped declaration entry)", entry_path.display()).into()),
    )?;
    output.replace_range(..entry_source.len(), &retained_entry);
    for record in &mut emitted_owned {
        if record.start == 0 && record.end == 0 { continue; }
        record.start = record.start.checked_sub(entry_source.len())
            .and_then(|offset| offset.checked_add(retained_entry.len()))
            .ok_or("invalid emitted declaration offset")?;
        record.end = record.end.checked_sub(entry_source.len())
            .and_then(|offset| offset.checked_add(retained_entry.len()))
            .ok_or("invalid emitted declaration offset")?;
    }
    record_owned_entry_declarations(&mut emitted_owned, entry_path, &retained_entry, 0)?;
    if !defer_closure {
        append_private_type_closure(&mut output, &emitted_owned, &source_views)?;
    }
    if let Some(metadata) = metadata {
        metadata.emitted = emitted_owned;
        metadata.views = source_views;
    }
    Ok(output)
}

/// The namespace-qualified-extends counterpart to
/// `reexported_class_or_interface_declarations_inner`: extracts a class
/// declaration by name, and (transitively) its own ancestors, from an
/// in-memory Node-builtin `.d.ts` string (`resolve_builtin`'s own
/// output) rather than a file on disk -- a builtin's synthetic
/// declaration is self-contained (no further external imports to
/// follow). Follow its referenced sibling types as well as class
/// ancestors, since `Transform` uses `TransformOptions` in constructors.
fn builtin_class_and_ancestor_declarations(
    dts_source: &str,
    specifier: &str,
    name: &str,
    visited: &mut std::collections::BTreeSet<String>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(name.to_string()) {
        return Ok(Vec::new());
    }
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        dts_source, thaw_parser::common::FileName::Custom(
            format!("node:{} (generated builtin declarations)", specifier.strip_prefix("node:").unwrap_or(specifier)).into(),
        ),
    )?;
    let mut declarations = Vec::new();
    let mut superclass = None;
    let mut selected = None;
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
            continue;
        };
        let matching = match &export.decl {
            Decl::Class(class) if class.ident.sym == name => {
                superclass = class.class.super_class.as_deref().and_then(|expr| match expr {
                    Expr::Ident(ident) => Some(ident.sym.to_string()),
                    _ => None,
                });
                Some(class.span())
            }
            Decl::TsInterface(interface) if interface.id.sym == name => Some(interface.span()),
            Decl::TsTypeAlias(alias) if alias.id.sym == name => Some(alias.span()),
            Decl::TsEnum(enumeration) if enumeration.id.sym == name => Some(enumeration.span()),
            _ => None,
        };
        if let Some(span) = matching {
            selected = Some(source_map.span_to_snippet(span).map_err(|error| {
                format!("failed to read builtin declaration `{name}`: {error:?}")
            })?);
            break;
        }
    }
    let Some(snippet) = selected else { return Ok(Vec::new()) };
    declarations.push(snippet.clone());
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else { continue };
        let candidate = match &export.decl {
            Decl::Class(class) => Some(class.ident.sym.as_ref()),
            Decl::TsInterface(interface) => Some(interface.id.sym.as_ref()),
            Decl::TsTypeAlias(alias) => Some(alias.id.sym.as_ref()),
            Decl::TsEnum(enumeration) => Some(enumeration.id.sym.as_ref()),
            _ => None,
        };
        let Some(candidate) = candidate else { continue };
        if candidate == name || Some(candidate) == superclass.as_deref() { continue; }
        if type_reference_sites(&snippet, candidate, Path::new(specifier))
            .is_some_and(|sites| !sites.is_empty()) {
            declarations.extend(builtin_class_and_ancestor_declarations(
                dts_source, specifier, candidate, visited,
            )?);
        }
    }
    if let Some(superclass) = superclass {
        declarations.extend(builtin_class_and_ancestor_declarations(
            dts_source,
            specifier,
            &superclass,
            visited,
        )?);
    }
    Ok(declarations)
}

// The terminal source identity survives alias and wildcard traversal.
#[derive(Clone)]
struct OwnedDeclaration {
    origin: PathBuf,
    source_scope: Vec<String>,
    local_name: Option<String>,
    snippet: String,
    children: Vec<OwnedDeclaration>,
}

impl OwnedDeclaration {
    fn new(origin: &Path, snippet: String) -> Self {
        let children = source_namespace_children(origin, &snippet);
        Self {
            origin: origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf()),
            source_scope: Vec::new(),
            local_name: declaration_identity(&snippet),
            snippet,
            children,
        }
    }

    fn with_snippet(mut self, snippet: String) -> Self {
        self.snippet = snippet;
        self
    }
}

// A source namespace is one owned declaration, but its type references
// resolve relative to its lexical member scope. This is metadata only: the
// source snippet still passes through the existing formatter unchanged.
fn source_namespace_children(origin: &Path, snippet: &str) -> Vec<OwnedDeclaration> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
        Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};
    fn prepend_scope(declaration: &mut OwnedDeclaration, name: &str) {
        declaration.source_scope.insert(0, name.to_string());
        for child in &mut declaration.children {
            prepend_scope(child, name);
        }
    }
    let Ok((module, source_map)) = thaw_parser::parse_declarations_with_source_map_named(
        snippet,
        thaw_parser::common::FileName::Custom(
            format!("{} (owned namespace)", origin.display()).into(),
        ),
    ) else { return Vec::new() };
    let namespace = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::TsModule(module) => Some(module), _ => None,
        },
        ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module))) => Some(module),
        _ => None,
    });
    let Some(namespace) = namespace else { return Vec::new() };
    let TsModuleName::Ident(name) = &namespace.id else { return Vec::new() };
    let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { return Vec::new() };
    block.body.iter().flat_map(|item| {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item {
            // A namespace's `export { Model as Renamed }` is selected even
            // though the marker has no declaration node of its own. Keep
            // one source key per specifier; synthetic metadata snippets do
            // not replace the unchanged original output text.
            return export.specifiers.iter().filter_map(|specifier| {
                let ExportSpecifier::Named(named) = specifier else { return None };
                let ModuleExportName::Ident(original) = &named.orig else { return None };
                let ModuleExportName::Ident(public) = named.exported.as_ref().unwrap_or(&named.orig) else { return None };
                let keyword = if export.type_only || named.is_type_only { "export type" } else { "export" };
                let text = format!("{keyword} {{ {} as {} }};", original.sym, public.sym);
                let mut child = OwnedDeclaration::new(origin, text);
                child.local_name = Some(original.sym.to_string());
                prepend_scope(&mut child, name.sym.as_ref());
                Some(child)
            }).collect::<Vec<_>>();
        }
        let is_declaration = matches!(item,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(_))
                | ModuleItem::Stmt(Stmt::Decl(_)));
        if !is_declaration { return Vec::new(); }
        let Ok(text) = source_map.span_to_snippet(item.span()) else { return Vec::new() };
        let mut child = OwnedDeclaration::new(origin, text);
        prepend_scope(&mut child, name.sym.as_ref());
        vec![child]
    }).collect()
}

// A selected declaration's source identity and exact generated interval.
// The closure pass uses these records to keep private names from distinct
// source files separate even after their declarations have been flattened.
#[derive(Clone)]
struct EmittedOwnedDeclaration {
    declaration: OwnedDeclaration,
    scope: Option<String>,
    public_names: Vec<String>,
    start: usize,
    end: usize,
    // A binding already closed by a recursively flattened namespace target.
    // It participates in public name resolution, but has no interval in this output.
    metadata_only: bool,
}

fn emitted_public_names(snippet: &str, local_name: Option<&str>, selected: Option<&str>) -> Vec<String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    let Some(local_name) = local_name else { return Vec::new() };
    let mut names = selected.map(str::to_owned).into_iter().collect::<Vec<_>>();
    if snippet.trim_start().starts_with("export ")
        && declaration_identity(snippet).as_deref() == Some(local_name) {
        names.push(local_name.to_string());
    }
    if let Ok(module) = parse_labeled_declarations(snippet, "owned public aliases".to_string()) {
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
            for specifier in &export.specifiers {
                let ExportSpecifier::Named(named) = specifier else { continue };
                let ModuleExportName::Ident(original) = &named.orig else { continue };
                if original.sym.as_ref() != local_name { continue; }
                let ModuleExportName::Ident(public) = named.exported.as_ref().unwrap_or(&named.orig) else { continue };
                names.push(public.sym.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn namespace_member_public_names(declaration: &OwnedDeclaration) -> Vec<String> {
    if !declaration.snippet.trim_start().starts_with("export ") {
        return Vec::new();
    }
    // A bare declaration followed by `export { Original as Alias }` is
    // locally named Original but exposes only Alias. The marker has no
    // declaration identity; adding Original to its public names would make
    // a private binding look selected and could suppress its closure.
    let structural_export = declaration_identity(&declaration.snippet);
    emitted_public_names(
        &declaration.snippet,
        declaration.local_name.as_deref(),
        structural_export.as_deref(),
    )
}

type SourceTypeKey = (PathBuf, Vec<String>, String);
type SourceTypeBindings = std::collections::BTreeMap<SourceTypeKey, Vec<OwnedDeclaration>>;
type SourceValueBindings = std::collections::BTreeMap<SourceTypeKey, Vec<OwnedDeclaration>>;

#[derive(Default)]
struct FlattenedOwnedMetadata {
    emitted: Vec<EmittedOwnedDeclaration>,
    views: OwnedSourceViews,
}

struct SelectedNamespaceBinding {
    public: String,
    value_export: bool,
    fragments: Vec<OwnedDeclaration>,
}

fn is_type_only_namespace_binding(declaration: &OwnedDeclaration, public: &str) -> bool {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
        Stmt, TsKeywordTypeKind, TsModuleName, TsNamespaceBody, TsType};
    let Ok(module) = parse_labeled_declarations(&declaration.snippet,
        "selected type-only namespace".to_string()) else { return false };
    let has_namespace = module.body.iter().any(|item| {
        let namespace = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(namespace))) => Some(namespace),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) =>
                if let Decl::TsModule(namespace) = &export.decl { Some(namespace) } else { None },
            _ => None,
        };
        matches!(namespace.map(|namespace| &namespace.id),
            Some(TsModuleName::Ident(name)) if name.sym.as_ref() == public)
    });
    let explicit_marker = module.body.iter().any(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { return false };
        if !export.type_only { return false; }
        export.specifiers.iter().any(|specifier| {
            let ExportSpecifier::Named(named) = specifier else { return false };
            matches!(named.exported.as_ref().unwrap_or(&named.orig),
                ModuleExportName::Ident(name) if name.sym.as_ref() == public)
        })
    });
    let generated_marker = module.body.iter().any(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else { return false };
        let Decl::TsModule(namespace) = &export.decl else { return false };
        let TsModuleName::Ident(name) = &namespace.id else { return false };
        if name.sym.as_ref() != public { return false; }
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { return false };
        block.body.iter().any(|member| matches!(member,
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias)))
                if generated_type_only_namespace_marker(alias.id.sym.as_ref())
                    && matches!(alias.type_ann.as_ref(), TsType::TsKeywordType(keyword)
                        if keyword.kind == TsKeywordTypeKind::TsNeverKeyword)))
    });
    has_namespace && (explicit_marker || generated_marker)
}

fn generated_type_only_namespace_marker(name: &str) -> bool {
    name.strip_prefix("__thaw_type_only_namespace_marker_")
        .is_some_and(|suffix| suffix.len() == 16
            && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn type_only_namespace_public_member(
    snippet: &str, public: &str, origin: &Path,
) -> Result<String, String> {
    let prefix = format!("declare namespace {public} {{");
    let suffix = format!("\n}}\nexport type {{ {public} }};");
    let body = snippet.trim().strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(&suffix))
        .ok_or_else(|| format!("invalid selected type-only namespace `{public}`"))?;
    // TS1194 forbids `export type { ... }` inside a namespace. The marker
    // records the erased runtime role while the exported declaration keeps
    // its child types accessible through the public qualified path.
    let digest = format!("{:x}", Sha256::digest(format!("{}\0{public}",
        origin.display()).as_bytes()));
    let marker = format!("__thaw_type_only_namespace_marker_{}", &digest[..16]);
    if body.contains(&marker) {
        return Err(format!("type-only namespace marker collision in `{public}`"));
    }
    Ok(format!("export namespace {public} {{\n  type {marker} = never;{body}\n}}"))
}

fn selected_namespace_bindings(
    target_path: &Path,
    records: &[EmittedOwnedDeclaration],
    views: &OwnedSourceViews,
) -> Result<Vec<SelectedNamespaceBinding>, String> {
    let mut selected = records.iter().filter(|record| record.scope.is_none())
        .flat_map(|record| record.public_names.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let source = views.read(target_path)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(target_path.to_path_buf()),
    )?.0;
    let mut namespace_containers = std::collections::BTreeMap::new();
    let mut namespace_targets = std::collections::BTreeMap::new();
    for (alias, namespace_target) in collect_namespace_reexports_with_views(target_path, &module,
        &mut std::collections::BTreeSet::new(), false, views)? {
        let namespace_target = namespace_target.canonicalize().unwrap_or(namespace_target);
        if let Some(previous) = namespace_targets.insert(alias.clone(), namespace_target.clone()) {
            if previous == namespace_target { continue; }
            return Err(format!("ambiguous namespace export `{alias}`"));
        }
        let public_owners = records.iter().filter(|record|
            record.scope.as_deref() == Some(alias.as_str())
                && !record.public_names.is_empty())
            .filter_map(|record| record.declaration.local_name.as_ref().map(|local|
                (record.declaration.origin.clone(),
                    record.declaration.source_scope.clone(), local.clone())))
            .collect::<std::collections::BTreeSet<_>>();
        let mut children = Vec::new();
        for record in records.iter().filter(|record| record.scope.as_deref() == Some(alias.as_str())) {
            let selected = record.declaration.local_name.as_ref().is_some_and(|local|
                public_owners.contains(&(record.declaration.origin.clone(),
                    record.declaration.source_scope.clone(), local.clone())));
            if !selected { continue; }
            if !children.iter().any(|existing: &OwnedDeclaration|
                existing.origin == record.declaration.origin
                    && existing.source_scope == record.declaration.source_scope
                    && existing.snippet == record.declaration.snippet) {
                children.push(record.declaration.clone());
            }
        }
        if children.is_empty() { continue; }
        let body = children.iter().map(|child| child.snippet.as_str())
            .collect::<Vec<_>>().join("\n");
        let mut container = OwnedDeclaration::new(target_path,
            format!("declare namespace {alias} {{\n{body}\n}}"));
        container.children = children;
        namespace_containers.insert(alias.clone(), container);
        selected.insert(alias);
    }
    let mut result = Vec::new();
    for public in selected {
        // Flattened wildcard records enumerate spellings, but they cannot
        // decide export precedence: an explicit export shadows a star even
        // when the star appears first. Reuse the selected-source followers.
        let mut fragments = exported_owned_type_declarations(target_path, &public, views)?;
        let values = exported_owned_value_declarations_with_views(target_path, &public,
            &mut std::collections::BTreeSet::new(), views)?;
        let value_export = !values.is_empty();
        for fragment in values {
            if !fragments.iter().any(|existing| existing.origin == fragment.origin
                && existing.source_scope == fragment.source_scope
                && existing.local_name == fragment.local_name
                && existing.snippet == fragment.snippet) {
                fragments.push(fragment);
            }
        }
        if fragments.is_empty() {
            use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
            let named_explicit = module.body.iter().any(|item| {
                let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { return false };
                export.specifiers.iter().any(|specifier| {
                    let ExportSpecifier::Named(named) = specifier else { return false };
                    matches!(named.exported.as_ref().unwrap_or(&named.orig),
                        ModuleExportName::Ident(name) if name.sym.as_ref() == public)
                })
            });
            if named_explicit { continue; }
            if let Some(container) = namespace_containers.remove(&public) {
                result.push(SelectedNamespaceBinding {
                    public, value_export: true, fragments: vec![container],
                });
                continue;
            }
            // `export * as NS` and `export type * as NS` materialize a
            // namespace container, not an ordinary selected type/value.
            // Only that exact generated container may fall back to the
            // flattened records; an ambiguous or unresolved wildcard
            // binding must not regain a value here.
            let containers = records.iter().filter(|record| record.scope.is_none()
                && record.public_names.contains(&public)
                && !record.declaration.children.is_empty())
                .collect::<Vec<_>>();
            let owners = containers.iter().filter_map(|record| record.declaration.local_name.as_ref()
                .map(|local| (record.declaration.origin.clone(),
                    record.declaration.source_scope.clone(), local.clone())))
                .collect::<std::collections::BTreeSet<_>>();
            if owners.len() > 1 {
                return Err(format!("ambiguous namespace export `{public}`"));
            }
            let value_export = containers.iter().any(|record| record.metadata_only
                || public_name_has_value_export(&record.declaration.snippet, &public));
            fragments = containers.into_iter().map(|record| record.declaration.clone()).collect();
            if fragments.is_empty() { continue; }
            result.push(SelectedNamespaceBinding { public, value_export, fragments });
            continue;
        }
        let owners = fragments.iter().filter_map(|fragment| fragment.local_name.as_ref()
            .map(|local| (fragment.origin.clone(), fragment.source_scope.clone(), local.clone())))
            .collect::<std::collections::BTreeSet<_>>();
        if owners.len() > 1 {
            return Err(format!("conflicting type/value namespace export `{public}`"));
        }
        result.push(SelectedNamespaceBinding { public, value_export, fragments });
    }
    Ok(result)
}

fn owned_origin_namespace_name(
    origin: &Path, output: &str, views: &mut OwnedSourceViews,
) -> String {
    let origin = origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf());
    if let Some(existing) = views.owned_names.get(&origin) { return existing.clone(); }
    let digest = format!("{:x}", Sha256::digest(origin.to_string_lossy().as_bytes()));
    let base = format!("__thaw_owned_{}", &digest[..16]);
    let mut name = base.clone();
    let mut suffix = 0usize;
    while output.contains(&name)
        || views.by_origin.values().any(|source| source.contains(&name))
        || views.owned_names.values().any(|existing| existing == &name) {
        suffix += 1;
        name = format!("{base}_{suffix}");
    }
    views.owned_names.insert(origin, name.clone());
    name
}

fn owned_origin_marker(name: &str) -> String {
    format!("__thaw_owned_origin_marker_{}", name.trim_start_matches("__thaw_owned_"))
}

fn exported_owned_member(declaration: &OwnedDeclaration) -> (String, usize, usize) {
    let mut member = private_support_declaration_member(&declaration.snippet);
    let mut start = 0;
    let mut end = member.len();
    for scope in declaration.source_scope.iter().rev() {
        let prefix = format!("export namespace {scope} {{\n");
        start += prefix.len();
        end += prefix.len();
        member = format!("{prefix}{member}\n}}");
    }
    (member, start, end)
}

// Keep a generic selected type's public alias generic as well. A plain
// `type Public = Owner.Model` would erase Model<T>'s type parameters and
// make its original constraints/defaults disappear from closure traversal.
fn owned_type_parameter_forwarding(
    declaration: &OwnedDeclaration,
) -> Result<(String, String), String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &declaration.snippet,
        thaw_parser::common::FileName::Custom(
            format!("{} (selected generic alias)", declaration.origin.display()).into(),
        ),
    )?;
    let parameters = module.body.iter().find_map(|item| {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        }?;
        match declaration {
            Decl::Class(class) => class.class.type_params.as_deref(),
            Decl::TsInterface(interface) => interface.type_params.as_deref(),
            Decl::TsTypeAlias(alias) => alias.type_params.as_deref(),
            _ => None,
        }
    });
    let Some(parameters) = parameters else { return Ok((String::new(), String::new())) };
    let signature = source_map.span_to_snippet(parameters.span()).map_err(|error|
        format!("failed to read selected generic parameters: {error:?}"))?;
    let forwarding = format!("<{}>", parameters.params.iter()
        .map(|parameter| parameter.name.sym.as_ref()).collect::<Vec<_>>().join(", "));
    Ok((signature, forwarding))
}

fn append_owned_origin_wrappers(
    output: &mut String,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    records: Vec<EmittedOwnedDeclaration>,
    views: &mut OwnedSourceViews,
    base_offset: usize,
) {
    let mut by_origin = std::collections::BTreeMap::<PathBuf, Vec<EmittedOwnedDeclaration>>::new();
    for record in records {
        if record.metadata_only { continue; }
        by_origin.entry(record.declaration.origin.clone()).or_default().push(record);
    }
    for (origin, records) in by_origin {
        let namespace = owned_origin_namespace_name(&origin, output, views);
        output.push_str(&format!(
            "\ndeclare namespace {namespace} {{\n  type {} = never;\n",
            owned_origin_marker(&namespace),
        ));
        let mut included = std::collections::BTreeSet::new();
        for mut record in records {
            let identity = (record.declaration.source_scope.clone(), record.declaration.snippet.clone());
            if !included.insert(identity) { continue; }
            let (member, local_start, local_end) = exported_owned_member(&record.declaration);
            let start = base_offset + output.len();
            output.push_str(&member);
            output.push('\n');
            record.start = start + local_start;
            record.end = start + local_end;
            record.declaration.snippet = member[local_start..local_end].to_string();
            let mut scope = namespace.clone();
            for part in &record.declaration.source_scope {
                scope.push('.');
                scope.push_str(part);
            }
            record.scope = Some(scope);
            record.public_names.clear();
            let child_start = emitted.len();
            record_owned_namespace_children(emitted, &record.declaration,
                record.scope.as_deref(), Some(record.start));
            for child in &mut emitted[child_start..] {
                child.public_names.clear();
            }
            emitted.push(record);
        }
        output.push_str("}\n");
    }
}

#[derive(Clone)]
struct CompositeAlias {
    fragments: Vec<OwnedDeclaration>,
    value_export: bool,
}

#[derive(Default)]
struct OwnedSourceViews {
    by_origin: std::collections::BTreeMap<PathBuf, String>,
    composite_aliases: std::collections::BTreeMap<(PathBuf, Vec<String>, Vec<String>), CompositeAlias>,
    owned_names: std::collections::BTreeMap<PathBuf, String>,
}

impl OwnedSourceViews {
    fn set(&mut self, path: &Path, source: String) {
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.by_origin.insert(origin, source);
    }

    fn append(&mut self, path: &Path, source: &str) -> Result<(), String> {
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !self.by_origin.contains_key(&origin) {
            let source = self.read(&origin)?;
            self.by_origin.insert(origin.clone(), source);
        }
        let view = self.by_origin.get_mut(&origin).expect("source view was inserted");
        if !view.ends_with('\n') { view.push('\n'); }
        view.push_str(source);
        Ok(())
    }

    fn read(&self, path: &Path) -> Result<String, String> {
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.by_origin.get(&origin).cloned().map(Ok).unwrap_or_else(|| {
            let disk = fs::read_to_string(&origin).map_err(|error|
                format!("failed to read source view `{}`: {error}", origin.display()))?;
            unwrap_self_ambient_module(&origin, &disk)
        })
    }

    fn has(&self, path: &Path) -> bool {
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.by_origin.contains_key(&origin)
    }
}

#[derive(Default)]
struct SourceSupportBindings {
    types: SourceTypeBindings,
    values: SourceValueBindings,
}

fn source_type_bindings(path: &Path) -> Result<SourceTypeBindings, String> {
    source_type_bindings_with_views(path, &OwnedSourceViews::default())
}

fn source_type_bindings_with_views(
    path: &Path, views: &OwnedSourceViews,
) -> Result<SourceTypeBindings, String> {
    use thaw_parser::ast::{Decl, DefaultDecl, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};
    fn insert_owned(
        table: &mut SourceTypeBindings,
        declaration: OwnedDeclaration,
    ) {
        if let Some(local) = &declaration.local_name {
            if is_type_declaration_snippet(&declaration.snippet) {
                table.entry((declaration.origin.clone(), declaration.source_scope.clone(), local.clone()))
                    .or_default().push(declaration.clone());
            }
        }
        for child in &declaration.children { insert_owned(table, child.clone()); }
    }
    let source = views.read(path)?;
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
    let mut table = std::collections::BTreeMap::new();
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) = item {
            let named = match &default.decl {
                DefaultDecl::Class(class) => class.ident.as_ref().map(|id| (id.sym.to_string(), class.span(), true)),
                DefaultDecl::TsInterfaceDecl(interface) =>
                    Some((interface.id.sym.to_string(), interface.span(), false)),
                _ => None,
            };
            if let Some((name, span, declare)) = named {
                let snippet = source_map.span_to_snippet(span).map_err(|error|
                    format!("failed to read default type in `{}`: {error:?}", path.display()))?;
                let mut owned = OwnedDeclaration::new(path,
                    if declare { format!("declare {snippet}") } else { snippet });
                owned.local_name = Some(name);
                insert_owned(&mut table, owned);
            }
            continue;
        }
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        if !matches!(declaration, Some(Decl::Class(_) | Decl::TsInterface(_)
            | Decl::TsTypeAlias(_) | Decl::TsEnum(_) | Decl::TsModule(_))) { continue; }
        let snippet = source_map.span_to_snippet(item.span()).map_err(|error|
            format!("failed to read source type binding in `{}`: {error:?}", path.display()))?;
        insert_owned(&mut table, OwnedDeclaration::new(path, snippet));
    }
    Ok(table)
}

fn source_value_bindings(path: &Path) -> Result<SourceValueBindings, String> {
    source_value_bindings_with_views(path, &OwnedSourceViews::default())
}

fn source_value_bindings_with_views(
    path: &Path, views: &OwnedSourceViews,
) -> Result<SourceValueBindings, String> {
    use thaw_parser::ast::{Decl, DefaultDecl, ModuleDecl, ModuleItem, Pat, Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};
    fn collect(
        path: &Path,
        source_map: &thaw_parser::common::SourceMap,
        items: &[ModuleItem],
        scope: &[String],
        values: &mut SourceValueBindings,
    ) -> Result<(), String> {
        for item in items {
            if let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) = item {
                let named = match &default.decl {
                    DefaultDecl::Class(class) => class.ident.as_ref().map(|name| (name, class.span())),
                    DefaultDecl::Fn(function) => function.ident.as_ref().map(|name| (name, function.span())),
                    _ => None,
                };
                if let Some((name, span)) = named {
                    let snippet = source_map.span_to_snippet(span).map_err(|error|
                        format!("failed to read default value in `{}`: {error:?}", path.display()))?;
                    let mut owned = OwnedDeclaration::new(path, format!("declare {snippet}"));
                    owned.source_scope = scope.to_vec();
                    owned.local_name = Some(name.sym.to_string());
                    values.entry((owned.origin.clone(), scope.to_vec(), name.sym.to_string()))
                        .or_default().push(owned);
                }
                continue;
            }
            let (declaration, exported) = match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => (&export.decl, true),
                ModuleItem::Stmt(Stmt::Decl(declaration)) => (declaration, false),
                _ => continue,
            };
            if let Decl::TsModule(namespace) = declaration {
                let TsModuleName::Ident(name) = &namespace.id else { continue };
                let snippet = source_map.span_to_snippet(item.span()).map_err(|error|
                    format!("failed to read value namespace in `{}`: {error:?}", path.display()))?;
                let mut owned = OwnedDeclaration::new(path, snippet);
                owned.source_scope = scope.to_vec();
                values.entry((owned.origin.clone(), scope.to_vec(), name.sym.to_string()))
                    .or_default().push(owned);
                if let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body {
                    let mut child_scope = scope.to_vec();
                    child_scope.push(name.sym.to_string());
                    collect(path, source_map, &block.body, &child_scope, values)?;
                }
                continue;
            }
            if let Decl::Var(variable) = declaration {
                for declarator in &variable.decls {
                    let Pat::Ident(binding) = &declarator.name else { continue };
                    let snippet = selected_var_declaration_snippet(
                        source_map, variable, declarator, exported,
                    )?;
                    let mut owned = OwnedDeclaration::new(path, snippet);
                    owned.source_scope = scope.to_vec();
                    values.entry((owned.origin.clone(), scope.to_vec(), binding.id.sym.to_string()))
                        .or_default().push(owned);
                }
                continue;
            }
            if !matches!(declaration, Decl::Fn(_) | Decl::Class(_) | Decl::TsEnum(_)) {
                continue;
            }
            let snippet = source_map.span_to_snippet(item.span()).map_err(|error|
                format!("failed to read source value binding in `{}`: {error:?}", path.display()))?;
            let mut owned = OwnedDeclaration::new(path, snippet);
            owned.source_scope = scope.to_vec();
            if let Some(name) = owned.local_name.clone() {
                values.entry((owned.origin.clone(), scope.to_vec(), name))
                    .or_default().push(owned);
            }
        }
        Ok(())
    }
    let source = views.read(path)?;
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
    let mut values = SourceValueBindings::new();
    collect(path, &source_map, &module.body, &[], &mut values)?;
    Ok(values)
}

fn resolve_lexical_source_type<'a>(
    table: &'a SourceTypeBindings,
    origin: &Path,
    scope: &[String],
    name: &str,
) -> Option<&'a [OwnedDeclaration]> {
    let origin = origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf());
    (0..=scope.len()).rev().find_map(|depth|
        table.get(&(origin.clone(), scope[..depth].to_vec(), name.to_string())).map(Vec::as_slice))
}

fn resolve_lexical_source_value<'a>(
    table: &'a SourceValueBindings,
    origin: &Path,
    scope: &[String],
    name: &str,
) -> Option<&'a [OwnedDeclaration]> {
    let origin = origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf());
    (0..=scope.len()).rev().find_map(|depth|
        table.get(&(origin.clone(), scope[..depth].to_vec(), name.to_string())).map(Vec::as_slice))
}

fn exported_owned_value_declarations(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<OwnedDeclaration>, String> {
    exported_owned_value_declarations_with_views(
        path, name, visited, &OwnedSourceViews::default(),
    )
}

fn exported_owned_value_declarations_with_views(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    use thaw_parser::ast::{Decl, DefaultDecl, Expr, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat};
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert((canonical.clone(), name.to_string())) { return Ok(Vec::new()); }
    let source = views.read(&canonical)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(canonical.clone()),
    )?.0;
    let table = source_value_bindings_with_views(&canonical, views)?;
    let imports = named_import_targets(&canonical, &module);
    let import_equals = import_equals_targets(&canonical, &module);
    let mut explicit = Vec::new();
    let mut wildcard_targets = Vec::new();
    let mut explicit_selected = false;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) if name == "default" => {
                explicit_selected = true;
                let local = match &default.decl {
                    DefaultDecl::Fn(function) => function.ident.as_ref().map(|id| id.sym.to_string()),
                    DefaultDecl::Class(class) => class.ident.as_ref().map(|id| id.sym.to_string()),
                    _ => None,
                };
                if let Some(local) = local {
                    explicit.extend(table.get(&(canonical.clone(), Vec::new(), local))
                        .cloned().unwrap_or_default());
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default)) if name == "default" => {
                explicit_selected = true;
                if let Expr::Ident(id) = default.expr.as_ref() {
                    explicit.extend(table.get(&(canonical.clone(), Vec::new(), id.sym.to_string()))
                        .cloned().unwrap_or_default());
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let direct = match &export.decl {
                    Decl::Fn(function) => function.ident.sym == name,
                    Decl::Class(class) => class.ident.sym == name,
                    Decl::TsEnum(enumeration) => enumeration.id.sym == name,
                    Decl::TsModule(namespace) => matches!(&namespace.id,
                        thaw_parser::ast::TsModuleName::Ident(id) if id.sym == name),
                    Decl::Var(variable) => variable.decls.iter().any(|declarator|
                        matches!(&declarator.name, Pat::Ident(binding) if binding.id.sym == name)),
                    _ => false,
                };
                if direct {
                    explicit_selected = true;
                    explicit.extend(table.get(&(canonical.clone(), Vec::new(), name.to_string()))
                        .cloned().unwrap_or_default());
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only => {
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else { continue };
                    if named.is_type_only { continue; }
                    let ModuleExportName::Ident(public) = named.exported.as_ref().unwrap_or(&named.orig) else { continue };
                    if public.sym != name { continue; }
                    explicit_selected = true;
                    let ModuleExportName::Ident(original) = &named.orig else { continue };
                    let target = if let Some(source) = &export.src {
                        // A source-qualified export can never select an
                        // unrelated local/imported binding as fallback.
                        source.value.as_str()
                            .and_then(|specifier| declaration_reexport_path(&canonical, specifier))
                            .map(|target| (target, original.sym.to_string()))
                    } else {
                        imports.get(original.sym.as_ref()).cloned()
                    };
                    if let Some((target, imported)) = target {
                        explicit.extend(exported_owned_value_declarations_with_views(
                            &target, &imported, visited, views,
                        )?);
                    } else if export.src.is_none() {
                        if let Some(target) = import_equals.get(original.sym.as_ref()) {
                            explicit.extend(selected_export_assignment_declarations_with_visited(
                                target, TypeReferenceKind::ValueQuery, views, visited,
                            )?);
                        } else {
                            explicit.extend(table.get(&(canonical.clone(), Vec::new(), original.sym.to_string()))
                                .cloned().unwrap_or_default());
                        }
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only && name != "default" => {
                if let Some(target) = export.src.value.as_str()
                    .and_then(|specifier| declaration_reexport_path(&canonical, specifier)) {
                    wildcard_targets.push(target);
                }
            }
            _ => {}
        }
    }
    if name == "default" && !explicit_selected && !views.has(&canonical)
        && fs::read_to_string(&canonical).is_ok_and(|disk| disk == source) {
        let mut functions = reexported_function_declarations_owned(
            &canonical, name, &mut std::collections::BTreeSet::new(),
        )?;
        if functions.is_empty() {
            functions = reexported_class_or_interface_declarations_owned(&canonical, name)?;
        }
        let terminal = functions.first().and_then(|first| first.local_name.as_ref()
            .map(|local| (first.origin.clone(), first.source_scope.clone(), local.clone())));
        explicit.extend(functions.into_iter().filter(|fragment| terminal.as_ref().is_some_and(|key|
            fragment.local_name.as_ref().is_some_and(|local|
                &(fragment.origin.clone(), fragment.source_scope.clone(), local.clone()) == key))));
    }
    let mut wildcard = Vec::new();
    if !explicit_selected && explicit.is_empty() {
        for target in wildcard_targets {
            wildcard.extend(exported_owned_value_declarations_with_views(
                &target, name, visited, views,
            )?);
        }
    }
    visited.remove(&(canonical, name.to_string()));
    Ok(unambiguous_terminal_fragments(
        if explicit_selected || !explicit.is_empty() { explicit } else { wildcard },
    ))
}

fn resolve_owned_source_value_reference(
    origin: &Path,
    scope: &[String],
    local_spelling: &str,
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_owned_source_value_reference_with_views(
        origin, scope, local_spelling, &OwnedSourceViews::default(),
    )
}

fn resolve_owned_source_value_reference_with_views(
    origin: &Path,
    scope: &[String],
    local_spelling: &str,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    let table = source_value_bindings_with_views(origin, views)?;
    if let Some(declaration) = resolve_lexical_source_value(&table, origin, scope, local_spelling) {
        return Ok(declaration.to_vec());
    }
    let source = views.read(origin)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(origin.to_path_buf()),
    )?.0;
    if let Some(target) = import_equals_targets(origin, &module).get(local_spelling) {
        return selected_export_assignment_declarations_with_views(
            target, TypeReferenceKind::ValueQuery, views,
        );
    }
    let imports = named_import_targets(origin, &module);
    let Some((target, imported)) = imports.get(local_spelling) else { return Ok(Vec::new()) };
    let followed = exported_owned_value_declarations_with_views(
        target, imported, &mut std::collections::BTreeSet::new(), views,
    )?;
    Ok(first_terminal_fragments(followed))
}

fn first_terminal_fragments(followed: Vec<OwnedDeclaration>) -> Vec<OwnedDeclaration> {
    let first = followed.first().and_then(|declaration| declaration.local_name.as_ref()
        .map(|name| (declaration.origin.clone(), declaration.source_scope.clone(), name.clone())));
    followed.into_iter().filter(|declaration| first.as_ref().is_some_and(|key|
        declaration.local_name.as_ref().is_some_and(|name|
            &(declaration.origin.clone(), declaration.source_scope.clone(), name.clone()) == key)))
        .collect()
}

fn unambiguous_terminal_fragments(followed: Vec<OwnedDeclaration>) -> Vec<OwnedDeclaration> {
    let keys = followed.iter().filter_map(|declaration| declaration.local_name.as_ref()
        .map(|name| (declaration.origin.clone(), declaration.source_scope.clone(), name.clone())))
        .collect::<std::collections::BTreeSet<_>>();
    if keys.len() != 1 { return Vec::new(); }
    let mut unique = Vec::new();
    for fragment in followed {
        if !unique.iter().any(|existing: &OwnedDeclaration|
            existing.origin == fragment.origin
                && existing.source_scope == fragment.source_scope
                && existing.local_name == fragment.local_name
                && existing.snippet == fragment.snippet) {
            unique.push(fragment);
        }
    }
    unique
}

// `import Local = require('./target')` binds the target's `export = Actual`,
// not a declaration literally named Local. Keep the target source owner and
// the type/value role while following a possible chain of import-equals.
fn selected_export_assignment_declarations_with_views(
    path: &Path, kind: TypeReferenceKind, views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    selected_export_assignment_declarations_with_visited(
        path, kind, views, &mut std::collections::BTreeSet::new(),
    )
}

fn selected_export_assignment_declarations_with_visited(
    path: &Path, kind: TypeReferenceKind, views: &OwnedSourceViews,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<OwnedDeclaration>, String> {
    fn follow(
        path: &Path, kind: TypeReferenceKind, views: &OwnedSourceViews,
        visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
    ) -> Result<Vec<OwnedDeclaration>, String> {
        use thaw_parser::ast::{Expr, ModuleDecl, ModuleItem};
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        // Share the export follower's visit set across an export-assignment
        // boundary. A reserved key cannot collide with a parsed export name.
        let marker = (origin.clone(), "\0export_assignment".to_string());
        if !visited.insert(marker.clone()) {
            return Ok(Vec::new());
        }
        let result = (|| -> Result<Vec<OwnedDeclaration>, String> {
        let source = views.read(&origin)?;
        let module = thaw_parser::parse_declarations_with_source_map_named(
            &source, thaw_parser::common::FileName::Real(origin.clone()),
        )?.0;
        let Some(actual) = module.body.iter().find_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) = item else {
                return None;
            };
            let Expr::Ident(ident) = export.expr.as_ref() else { return None };
            Some(ident.sym.to_string())
        }) else { return Ok(Vec::new()); };
        let key = (origin.clone(), Vec::new(), actual.clone());
        let local = match kind {
            TypeReferenceKind::Type => source_type_bindings_with_views(&origin, views)?
                .get(&key).cloned(),
            TypeReferenceKind::ValueQuery => source_value_bindings_with_views(&origin, views)?
                .get(&key).cloned(),
        };
        if let Some(local) = local { return Ok(local); }
        if let Some(target) = import_equals_targets(&origin, &module).get(&actual) {
            return follow(target, kind, views, visited);
        }
        let imported = match kind {
            TypeReferenceKind::Type => named_type_import_targets(&origin, &module)
                .get(&actual).cloned()
                .or_else(|| named_import_targets(&origin, &module).get(&actual).cloned()),
            TypeReferenceKind::ValueQuery => named_import_targets(&origin, &module)
                .get(&actual).cloned(),
        };
        let Some((target, exported)) = imported else { return Ok(Vec::new()); };
        match kind {
            TypeReferenceKind::Type => exported_owned_type_declarations_with_visited(
                &target, &exported, views, visited,
            ),
            TypeReferenceKind::ValueQuery => exported_owned_value_declarations_with_views(
                &target, &exported, visited, views,
            ),
        }
        })();
        visited.remove(&marker);
        result
    }
    follow(path, kind, views, visited)
}

// A qualified `import Alias = require('./target')` reference starts at the
// target's `export = Actual` namespace, not at a fictitious export named
// Alias. Keep the same owner/view and follow import-equals chains without
// reinterpreting the member as an unrelated top-level wildcard export.
fn selected_export_assignment_member_with_views(
    path: &Path, members: &[String], kind: TypeReferenceKind,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    use thaw_parser::ast::{Expr, ImportSpecifier, ModuleDecl, ModuleItem};
    let mut current = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut visited = std::collections::BTreeSet::new();
    while visited.insert(current.clone()) {
        let source = views.read(&current)?;
        let module = thaw_parser::parse_declarations_with_source_map_named(
            &source, thaw_parser::common::FileName::Real(current.clone()),
        )?.0;
        let Some(actual) = module.body.iter().find_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) = item else {
                return None;
            };
            let Expr::Ident(ident) = export.expr.as_ref() else { return None };
            Some(ident.sym.to_string())
        }) else { return Ok(Vec::new()); };
        let table = match kind {
            TypeReferenceKind::Type => source_type_bindings_with_views(&current, views)?,
            TypeReferenceKind::ValueQuery => source_value_bindings_with_views(&current, views)?,
        };
        if table.contains_key(&(current.clone(), Vec::new(), actual.clone())) {
            let mut scope = vec![actual];
            scope.extend_from_slice(&members[..members.len().saturating_sub(1)]);
            let Some(last) = members.last() else { return Ok(Vec::new()); };
            if let Some(direct) = table.get(&(current.clone(), scope, last.clone())) {
                return Ok(direct.clone());
            }
            if kind == TypeReferenceKind::ValueQuery && members.len() == 1 {
                // `export = Actual` may name an object-valued const. Its
                // property signatures already have source-owned synthetic
                // value declarations with generic indexed types.
                return Ok(exported_const_object_properties(&current, views)?
                    .into_iter().filter_map(|(name, declaration)|
                        (name == last.as_str()).then_some(declaration)).collect());
            }
            return Ok(Vec::new());
        }
        if let Some(next) = import_equals_targets(&current, &module).get(&actual) {
            current = next.canonicalize().unwrap_or_else(|_| next.clone());
            continue;
        }
        let imported = match kind {
            TypeReferenceKind::Type => named_type_import_targets(&current, &module)
                .get(&actual).cloned()
                .or_else(|| named_import_targets(&current, &module).get(&actual).cloned()),
            TypeReferenceKind::ValueQuery => named_import_targets(&current, &module)
                .get(&actual).cloned(),
        };
        if let Some((target, imported)) = imported {
            if imported.is_empty() {
                return resolve_imported_namespace_member_with_views(&target, members, kind, views);
            }
            let mut path = vec![imported];
            path.extend_from_slice(members);
            return resolve_imported_namespace_member_with_views(&target, &path, kind, views);
        }
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else { continue };
            if kind == TypeReferenceKind::ValueQuery && import.type_only { continue; }
            let Some(specifier) = import.src.value.as_str() else { continue };
            let Some(target) = declaration_reexport_path(&current, specifier) else { continue };
            if import.specifiers.iter().any(|specifier| matches!(specifier,
                ImportSpecifier::Namespace(namespace)
                    if namespace.local.sym.as_ref() == actual.as_str())) {
                return resolve_imported_namespace_member_with_views(&target, members, kind, views);
            }
        }
        return Ok(Vec::new());
    }
    Ok(Vec::new())
}

fn exported_owned_type_declarations(
    path: &Path, name: &str, views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    exported_owned_type_declarations_with_visited(
        path, name, views, &mut std::collections::BTreeSet::new(),
    )
}

fn exported_owned_type_declarations_with_visited(
    path: &Path, name: &str, views: &OwnedSourceViews,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<OwnedDeclaration>, String> {
    fn follow(
        path: &Path, name: &str, views: &OwnedSourceViews,
        visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
    ) -> Result<Vec<OwnedDeclaration>, String> {
        use thaw_parser::ast::{Decl, DefaultDecl, Expr, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
        let origin = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !visited.insert((origin.clone(), name.to_string())) { return Ok(Vec::new()) }
        let source = views.read(&origin)?;
        let module = thaw_parser::parse_declarations_with_source_map_named(
            &source, thaw_parser::common::FileName::Real(origin.clone()),
        )?.0;
        let table = source_type_bindings_with_views(&origin, views)?;
        let value_imports = named_import_targets(&origin, &module);
        let type_imports = named_type_import_targets(&origin, &module);
        let import_equals = import_equals_targets(&origin, &module);
        let mut explicit = Vec::new();
        let mut wildcard_targets = Vec::new();
        let mut explicit_selected = false;
        for item in &module.body {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                    let direct = match &export.decl {
                        Decl::Class(class) => class.ident.sym == name,
                        Decl::TsInterface(interface) => interface.id.sym == name,
                        Decl::TsTypeAlias(alias) => alias.id.sym == name,
                        Decl::TsEnum(enumeration) => enumeration.id.sym == name,
                        Decl::TsModule(namespace) => matches!(&namespace.id,
                            thaw_parser::ast::TsModuleName::Ident(id) if id.sym == name),
                        _ => false,
                    };
                    if direct {
                        explicit_selected = true;
                        explicit.extend(table.get(&(origin.clone(), Vec::new(), name.to_string()))
                            .cloned().unwrap_or_default());
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) if name == "default" => {
                    explicit_selected = true;
                    let local = match &default.decl {
                        DefaultDecl::Class(class) => class.ident.as_ref().map(|id| id.sym.to_string()),
                        DefaultDecl::TsInterfaceDecl(interface) => Some(interface.id.sym.to_string()),
                        _ => None,
                    };
                    if let Some(local) = local {
                        explicit.extend(table.get(&(origin.clone(), Vec::new(), local))
                            .cloned().unwrap_or_default());
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default)) if name == "default" => {
                    explicit_selected = true;
                    if let Expr::Ident(id) = default.expr.as_ref() {
                        explicit.extend(table.get(&(origin.clone(), Vec::new(), id.sym.to_string()))
                            .cloned().unwrap_or_default());
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                    for specifier in &export.specifiers {
                        let ExportSpecifier::Named(named) = specifier else { continue };
                        let ModuleExportName::Ident(public) =
                            named.exported.as_ref().unwrap_or(&named.orig) else { continue };
                        if public.sym != name { continue; }
                        explicit_selected = true;
                        let ModuleExportName::Ident(original) = &named.orig else { continue };
                        let imported = type_imports.get(original.sym.as_ref())
                            .or_else(|| value_imports.get(original.sym.as_ref()));
                        let target = if let Some(source) = &export.src {
                            source.value.as_str()
                                .and_then(|specifier| declaration_reexport_path(&origin, specifier))
                                .map(|target| (target, original.sym.to_string()))
                        } else {
                            imported.cloned()
                        };
                        if let Some((target, target_name)) = target {
                            explicit.extend(follow(&target, &target_name, views, visited)?);
                        } else if export.src.is_none() {
                            if let Some(target) = import_equals.get(original.sym.as_ref()) {
                                explicit.extend(selected_export_assignment_declarations_with_visited(
                                    target, TypeReferenceKind::Type, views, visited,
                                )?);
                            } else {
                                explicit.extend(table.get(&(origin.clone(), Vec::new(), original.sym.to_string()))
                                    .cloned().unwrap_or_default());
                            }
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if name != "default" => {
                    if let Some(target) = export.src.value.as_str()
                        .and_then(|specifier| declaration_reexport_path(&origin, specifier)) {
                        wildcard_targets.push(target);
                    }
                }
                _ => {}
            }
        }
        let mut wildcard = Vec::new();
        if !explicit_selected {
            for target in wildcard_targets {
                wildcard.extend(follow(&target, name, views, visited)?);
            }
        }
        visited.remove(&(origin, name.to_string()));
        Ok(unambiguous_terminal_fragments(
            if explicit_selected { explicit } else { wildcard },
        ))
    }
    follow(path, name, views, visited)
}

fn namespace_reexport_target(
    path: &Path,
    alias: &str,
    include_type_only: bool,
    views: &OwnedSourceViews,
) -> Result<Option<PathBuf>, String> {
    let source = views.read(path)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?.0;
    Ok(collect_namespace_reexports_with_views(
        path, &module, &mut std::collections::BTreeSet::new(), include_type_only, views,
    )?.into_iter().find(|(name, _)| name == alias).map(|(_, target)| target))
}

fn named_namespace_reexport_target(
    path: &Path,
    public: &str,
    kind: TypeReferenceKind,
    views: &OwnedSourceViews,
) -> Result<(bool, Option<(PathBuf, Vec<String>)>), String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    let source = views.read(path)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?.0;
    let value_imports = named_import_targets(path, &module);
    let type_imports = named_type_import_targets(path, &module);
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else { continue };
            let ModuleExportName::Ident(exported) =
                named.exported.as_ref().unwrap_or(&named.orig) else { continue };
            if exported.sym != public { continue; }
            if kind == TypeReferenceKind::ValueQuery && (export.type_only || named.is_type_only) {
                return Ok((true, None));
            }
            let ModuleExportName::Ident(original) = &named.orig else {
                return Ok((true, None));
            };
            let target = if let Some(source) = &export.src {
                source.value.as_str()
                    .and_then(|specifier| declaration_reexport_path(path, specifier))
                    .map(|target| (target, vec![original.sym.to_string()]))
            } else {
                let imported = if kind == TypeReferenceKind::Type {
                    type_imports.get(original.sym.as_ref())
                        .or_else(|| value_imports.get(original.sym.as_ref()))
                } else {
                    value_imports.get(original.sym.as_ref())
                };
                imported.map(|(target, name)| (
                    target.clone(),
                    if name.is_empty() { Vec::new() } else { vec![name.clone()] },
                ))
            };
            return Ok((true, target));
        }
    }
    Ok((false, None))
}

fn directly_exported_namespace(
    path: &Path,
    segments: &[String],
    kind: TypeReferenceKind,
    views: &OwnedSourceViews,
) -> Result<Option<(Vec<String>, Option<String>)>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, TsModuleName, TsNamespaceBody};
    fn exported_local_name(
        items: &[ModuleItem], public: &str, kind: TypeReferenceKind,
    ) -> Option<String> {
        for item in items {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                    let local = match &export.decl {
                        Decl::TsModule(namespace) => match &namespace.id {
                            TsModuleName::Ident(id) => Some(id.sym.as_ref()),
                            _ => None,
                        },
                        Decl::Class(class) => Some(class.ident.sym.as_ref()),
                        Decl::TsEnum(enumeration) => Some(enumeration.id.sym.as_ref()),
                        Decl::Fn(function) if kind == TypeReferenceKind::ValueQuery =>
                            Some(function.ident.sym.as_ref()),
                        Decl::TsInterface(interface) if kind == TypeReferenceKind::Type =>
                            Some(interface.id.sym.as_ref()),
                        Decl::TsTypeAlias(alias) if kind == TypeReferenceKind::Type =>
                            Some(alias.id.sym.as_ref()),
                        Decl::Var(variable) if kind == TypeReferenceKind::ValueQuery => {
                            if variable.decls.iter().any(|declarator| matches!(&declarator.name,
                                thaw_parser::ast::Pat::Ident(binding) if binding.id.sym == public)) {
                                Some(public)
                            } else { None }
                        }
                        _ => None,
                    };
                    if local == Some(public) { return Some(public.to_string()); }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if export.src.is_none()
                    && (kind == TypeReferenceKind::Type || !export.type_only) => {
                    for specifier in &export.specifiers {
                        let ExportSpecifier::Named(named) = specifier else { continue };
                        if kind == TypeReferenceKind::ValueQuery && named.is_type_only { continue; }
                        let ModuleExportName::Ident(exported) =
                            named.exported.as_ref().unwrap_or(&named.orig) else { continue };
                        let ModuleExportName::Ident(original) = &named.orig else { continue };
                        if exported.sym == public { return Some(original.sym.to_string()); }
                    }
                }
                _ => {}
            }
        }
        None
    }
    fn local_namespace<'a>(items: &'a [ModuleItem], local: &str)
        -> Option<&'a thaw_parser::ast::TsModuleDecl> {
        items.iter().find_map(|item| {
            let declaration = match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
                ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(declaration)) => Some(declaration),
                _ => None,
            };
            match declaration {
                Some(Decl::TsModule(namespace)) if matches!(&namespace.id,
                    TsModuleName::Ident(id) if id.sym == local) => Some(namespace),
                _ => None,
            }
        })
    }
    let source = views.read(path)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?.0;
    let mut items = module.body.as_slice();
    let mut source_scope = Vec::new();
    for (depth, public) in segments.iter().take(segments.len().saturating_sub(1)).enumerate() {
        let Some(local) = exported_local_name(items, public, kind) else {
            return Ok((depth != 0).then_some((source_scope, None)));
        };
        let Some(namespace) = local_namespace(items, &local) else {
            return Ok((depth != 0).then_some((source_scope, None)));
        };
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else {
            return Ok(Some((source_scope, None)));
        };
        source_scope.push(local);
        items = &block.body;
    }
    let Some(member) = segments.last() else { return Ok(None) };
    Ok(Some((source_scope, exported_local_name(items, member, kind))))
}

fn resolve_imported_namespace_member(
    target: &Path,
    segments: &[String],
    kind: TypeReferenceKind,
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_imported_namespace_member_with_views(
        target, segments, kind, &OwnedSourceViews::default(),
    )
}

fn resolve_imported_namespace_member_with_views(
    target: &Path,
    segments: &[String],
    kind: TypeReferenceKind,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_imported_namespace_member_inner(
        target, segments, kind, &mut std::collections::BTreeSet::new(), views,
    )
}

fn resolve_imported_namespace_member_inner(
    target: &Path,
    segments: &[String],
    kind: TypeReferenceKind,
    visited: &mut std::collections::BTreeSet<(PathBuf, Vec<String>, TypeReferenceKind)>,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    let Some(member) = segments.last() else { return Ok(Vec::new()) };
    let target = target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    if !visited.insert((target.clone(), segments.to_vec(), kind)) {
        return Ok(Vec::new());
    }
    if segments.len() == 1 {
        return match kind {
            TypeReferenceKind::Type => exported_owned_type_declarations(&target, member, views),
            TypeReferenceKind::ValueQuery => Ok(first_terminal_fragments(
                exported_owned_value_declarations_with_views(
                    &target, member, &mut std::collections::BTreeSet::new(), views,
                )?)),
        };
    }
    if let Some((source_scope, local_member)) = directly_exported_namespace(&target, segments, kind, views)? {
        let Some(local_member) = local_member else { return Ok(Vec::new()) };
        let key = (target.clone(), source_scope, local_member);
        let direct = match kind {
            TypeReferenceKind::Type => source_type_bindings_with_views(&target, views)?.get(&key).cloned(),
            TypeReferenceKind::ValueQuery => source_value_bindings_with_views(&target, views)?.get(&key).cloned(),
        };
        if let Some(direct) = direct { return Ok(direct); }
        // The selected local namespace shadows any coincidental export
        // alias with the same root, even when its member is missing.
        return Ok(Vec::new());
    }
    let (selected, followed) = named_namespace_reexport_target(&target, &segments[0], kind, views)?;
    if selected {
        let Some((next, mut prefix)) = followed else { return Ok(Vec::new()) };
        prefix.extend_from_slice(&segments[1..]);
        return resolve_imported_namespace_member_inner(&next, &prefix, kind, visited, views);
    }
    let Some(next) = namespace_reexport_target(
        &target, &segments[0], kind == TypeReferenceKind::Type, views,
    )? else { return Ok(Vec::new()) };
    resolve_imported_namespace_member_inner(&next, &segments[1..], kind, visited, views)
}

fn resolve_owned_qualified_source_value_reference(
    origin: &Path,
    scope: &[String],
    segments: &[String],
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_owned_qualified_source_value_reference_with_views(
        origin, scope, segments, &OwnedSourceViews::default(),
    )
}

fn resolve_owned_qualified_source_value_reference_with_views(
    origin: &Path,
    scope: &[String],
    segments: &[String],
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    if segments.len() == 1 {
        return resolve_owned_source_value_reference_with_views(
            origin, scope, &segments[0], views,
        );
    }
    let table = source_value_bindings_with_views(origin, views)?;
    let origin = origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf());
    let member = segments.last().expect("qualified value query has a member");
    for depth in (0..=scope.len()).rev() {
        let root = (origin.clone(), scope[..depth].to_vec(), segments[0].clone());
        if !table.contains_key(&root) { continue; }
        let mut namespace = scope[..depth].to_vec();
        namespace.extend(segments[..segments.len() - 1].iter().cloned());
        return Ok(table.get(&(origin.clone(), namespace, member.clone()))
            .cloned().unwrap_or_default());
    }
    for depth in (0..=scope.len()).rev() {
        let key = (origin.clone(), scope[..depth].to_vec(), segments.to_vec());
        if let Some(alias) = views.composite_aliases.get(&key) {
            return Ok(if alias.value_export { alias.fragments.clone() } else { Vec::new() });
        }
    }
    let source = views.read(&origin)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(origin.clone()),
    )?.0;
    let named_imports = named_import_targets(&origin, &module);
    if let Some((target, imported)) = named_imports
        .get(&segments[0]).filter(|(_, imported)| !imported.is_empty()) {
        let mut public_path = vec![imported.clone()];
        public_path.extend_from_slice(&segments[1..]);
        return resolve_imported_namespace_member_with_views(
            target, &public_path, TypeReferenceKind::ValueQuery, views,
        );
    }
    let import_equals = import_equals_targets(&origin, &module);
    if let Some(target) = import_equals.get(&segments[0]) {
        return selected_export_assignment_member_with_views(
            target, &segments[1..], TypeReferenceKind::ValueQuery, views,
        );
    }
    let mut namespaces = std::collections::HashMap::new();
    for item in &module.body {
        let thaw_parser::ast::ModuleItem::ModuleDecl(thaw_parser::ast::ModuleDecl::Import(import)) = item else { continue };
        if import.type_only { continue; }
        let Some(specifier) = import.src.value.as_str() else { continue };
        let Some(target) = declaration_reexport_path(&origin, specifier) else { continue };
        for binding in &import.specifiers {
            if let thaw_parser::ast::ImportSpecifier::Namespace(namespace) = binding {
                namespaces.insert(namespace.local.sym.to_string(), target.clone());
            }
        }
    }
    let Some(target) = namespaces.get(&segments[0]) else { return Ok(Vec::new()) };
    resolve_imported_namespace_member_with_views(
        target, &segments[1..], TypeReferenceKind::ValueQuery, views,
    )
}

fn resolve_owned_source_type_reference(
    origin: &Path,
    scope: &[String],
    local_spelling: &str,
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_owned_source_type_reference_with_views(
        origin, scope, local_spelling, &OwnedSourceViews::default(),
    )
}

fn resolve_owned_source_type_reference_with_views(
    origin: &Path,
    scope: &[String],
    local_spelling: &str,
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    let table = source_type_bindings_with_views(origin, views)?;
    if let Some(declaration) = resolve_lexical_source_type(&table, origin, scope, local_spelling) {
        return Ok(declaration.to_vec());
    }
    // Only relative imports enter this map. The existing follower resolves
    // named barrels and default aliases to a terminal owned declaration;
    // keep `local_spelling` separate from that source identity for later
    // span replacement (`import type { Key as Id }`).
    let source = views.read(origin)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(origin.to_path_buf()),
    )?.0;
    let type_imports = named_type_import_targets(origin, &module);
    let value_imports = named_import_targets(origin, &module);
    if let Some(target) = import_equals_targets(origin, &module).get(local_spelling) {
        return selected_export_assignment_declarations_with_views(
            target, TypeReferenceKind::Type, views,
        );
    }
    let Some((target, imported)) = type_imports.get(local_spelling)
        .or_else(|| value_imports.get(local_spelling)) else { return Ok(Vec::new()) };
    // The selected follower deliberately returns empty for ambiguous stars
    // and unresolved explicit exports. A legacy first-star fallback would
    // turn either result into a different source owner.
    exported_owned_type_declarations(target, imported, views)
}

fn resolve_owned_qualified_source_type_reference(
    origin: &Path,
    scope: &[String],
    segments: &[String],
) -> Result<Vec<OwnedDeclaration>, String> {
    resolve_owned_qualified_source_type_reference_with_views(
        origin, scope, segments, &OwnedSourceViews::default(),
    )
}

fn resolve_owned_qualified_source_type_reference_with_views(
    origin: &Path,
    scope: &[String],
    segments: &[String],
    views: &OwnedSourceViews,
) -> Result<Vec<OwnedDeclaration>, String> {
    if segments.len() == 1 {
        return resolve_owned_source_type_reference_with_views(
            origin, scope, &segments[0], views,
        );
    }
    let table = source_type_bindings_with_views(origin, views)?;
    let origin = origin.canonicalize().unwrap_or_else(|_| origin.to_path_buf());
    let member = segments.last().expect("qualified reference has a member");
    for depth in (0..=scope.len()).rev() {
        let root = (origin.clone(), scope[..depth].to_vec(), segments[0].clone());
        if !table.contains_key(&root) { continue; }
        let mut namespace = scope[..depth].to_vec();
        namespace.extend(segments[..segments.len() - 1].iter().cloned());
        return Ok(table.get(&(origin.clone(), namespace, member.clone()))
            .cloned().unwrap_or_default());
    }
    for depth in (0..=scope.len()).rev() {
        let key = (origin.clone(), scope[..depth].to_vec(), segments.to_vec());
        if let Some(alias) = views.composite_aliases.get(&key) {
            return Ok(alias.fragments.clone());
        }
    }
    let source = views.read(&origin)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(origin.clone()),
    )?.0;
    let named_type_imports = named_type_import_targets(&origin, &module);
    let named_imports = named_import_targets(&origin, &module);
    if let Some((target, imported)) = named_type_imports
        .get(&segments[0])
        .or_else(|| named_imports.get(&segments[0])
            .filter(|(_, imported)| !imported.is_empty())) {
        let mut public_path = vec![imported.clone()];
        public_path.extend_from_slice(&segments[1..]);
        return resolve_imported_namespace_member_with_views(
            target, &public_path, TypeReferenceKind::Type, views,
        );
    }
    let import_equals = import_equals_targets(&origin, &module);
    if let Some(target) = import_equals.get(&segments[0]) {
        return selected_export_assignment_member_with_views(
            target, &segments[1..], TypeReferenceKind::Type, views,
        );
    }
    let mut namespaces = std::collections::HashMap::new();
    for item in &module.body {
        let thaw_parser::ast::ModuleItem::ModuleDecl(thaw_parser::ast::ModuleDecl::Import(import)) = item else { continue };
        let Some(specifier) = import.src.value.as_str() else { continue };
        let Some(target) = declaration_reexport_path(&origin, specifier) else { continue };
        for binding in &import.specifiers {
            if let thaw_parser::ast::ImportSpecifier::Namespace(namespace) = binding {
                namespaces.insert(namespace.local.sym.to_string(), target.clone());
            }
        }
    }
    let Some(target) = namespaces.get(&segments[0]) else { return Ok(Vec::new()) };
    resolve_imported_namespace_member_with_views(
        target, &segments[1..], TypeReferenceKind::Type, views,
    )
}

struct OwnedTypeReference {
    range: (usize, usize),
    full_range: (usize, usize),
    kind: TypeReferenceKind,
    local_spelling: String,
    terminal: SourceTypeKey,
    fragments: Vec<OwnedDeclaration>,
}

fn owned_type_reference_edges(declaration: &OwnedDeclaration) -> Result<Vec<OwnedTypeReference>, String> {
    owned_type_reference_edges_with_views(declaration, &OwnedSourceViews::default())
}

fn owned_type_reference_edges_with_views(
    declaration: &OwnedDeclaration, views: &OwnedSourceViews,
) -> Result<Vec<OwnedTypeReference>, String> {
    let table = source_type_bindings_with_views(&declaration.origin, views)?;
    let value_table = source_value_bindings_with_views(&declaration.origin, views)?;
    let source = views.read(&declaration.origin)?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(declaration.origin.clone()),
    )?.0;
    let mut names = table.keys().filter(|(origin, scope, _)|
        origin == &declaration.origin && declaration.source_scope.starts_with(scope))
        .map(|(_, _, name)| name.clone()).collect::<std::collections::BTreeSet<_>>();
    names.extend(value_table.keys().filter(|(origin, scope, _)|
        origin == &declaration.origin && declaration.source_scope.starts_with(scope))
        .map(|(_, _, name)| name.clone()));
    names.extend(named_type_import_targets(&declaration.origin, &module).into_keys());
    names.extend(named_import_targets(&declaration.origin, &module).into_keys());
    names.extend(import_equals_targets(&declaration.origin, &module).into_keys());
    names.extend(views.composite_aliases.keys().filter(|(origin, scope, _)|
        origin == &declaration.origin && declaration.source_scope.starts_with(scope))
        .filter_map(|(_, _, path)| path.first().cloned()));
    for item in &module.body {
        if let thaw_parser::ast::ModuleItem::ModuleDecl(thaw_parser::ast::ModuleDecl::Import(import)) = item {
            names.extend(import.specifiers.iter().filter_map(|specifier| {
                let thaw_parser::ast::ImportSpecifier::Namespace(namespace) = specifier else { return None };
                Some(namespace.local.sym.to_string())
            }));
        }
    }
    let mut edges = Vec::new();
    for name in names {
        let sites = type_reference_sites(&declaration.snippet, &name, &declaration.origin)
            .ok_or_else(|| format!("failed to parse type references in `{}`", declaration.origin.display()))?;
        for site in sites {
            let fragments = match site.kind {
                TypeReferenceKind::Type => resolve_owned_qualified_source_type_reference_with_views(
                    &declaration.origin, &declaration.source_scope, &site.segments, views,
                )?,
                TypeReferenceKind::ValueQuery => resolve_owned_qualified_source_value_reference_with_views(
                    &declaration.origin, &declaration.source_scope, &site.segments, views,
                )?,
            };
            let Some(first) = fragments.first() else { continue };
            let Some(local_name) = &first.local_name else { continue };
            let terminal = (first.origin.clone(), first.source_scope.clone(), local_name.clone());
            edges.push(OwnedTypeReference {
                range: site.root, full_range: site.full, kind: site.kind,
                local_spelling: name.clone(),
                terminal, fragments,
            });
        }
    }
    edges.sort_by_key(|edge| edge.range);
    edges.dedup_by(|right, left| right.full_range == left.full_range
        && right.terminal == left.terminal && right.kind == left.kind);
    Ok(edges)
}

fn referenced_private_type_closure(
    emitted: &[EmittedOwnedDeclaration],
    public: &std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>>,
) -> Result<SourceSupportBindings, String> {
    referenced_private_type_closure_with_views(
        emitted, public, &OwnedSourceViews::default(),
    )
}

fn referenced_private_type_closure_with_views(
    emitted: &[EmittedOwnedDeclaration],
    public: &std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>>,
    views: &OwnedSourceViews,
) -> Result<SourceSupportBindings, String> {
    let mut support = SourceSupportBindings::default();
    let mut visited = std::collections::BTreeSet::<(SourceTypeKey, String)>::new();
    let mut pending = std::collections::VecDeque::<OwnedDeclaration>::new();
    pending.extend(emitted.iter().filter(|record| !record.public_names.is_empty()
        || record.scope.as_deref().is_some_and(|scope|
            views.owned_names.values().any(|owner|
                scope == owner || scope.starts_with(&format!("{owner}.")))))
        .map(|record| record.declaration.clone()));
    while let Some(declaration) = pending.pop_front() {
        let Some(local_name) = &declaration.local_name else { continue };
        let owner = (declaration.origin.clone(), declaration.source_scope.clone(), local_name.clone());
        if !visited.insert((owner, declaration.snippet.clone())) { continue; }
        for edge in owned_type_reference_edges_with_views(&declaration, views)? {
            let has_public = public.get(&edge.terminal).is_some_and(|occurrences|
                occurrences.iter().any(|occurrence|
                    edge.kind == TypeReferenceKind::Type || occurrence.value_export));
            if has_public { continue; }
            let selected = match edge.kind {
                TypeReferenceKind::Type => support.types.entry(edge.terminal).or_default(),
                TypeReferenceKind::ValueQuery => support.values.entry(edge.terminal).or_default(),
            };
            for fragment in edge.fragments {
                if selected.iter().any(|existing| existing.snippet == fragment.snippet) { continue; }
                selected.push(fragment.clone());
                pending.push_back(fragment);
            }
        }
    }
    Ok(support)
}

fn private_support_namespace_names(
    output: &str,
    support: &SourceSupportBindings,
) -> std::collections::BTreeMap<PathBuf, String> {
    let mut names = std::collections::BTreeMap::new();
    for (origin, _, _) in support.types.keys().chain(support.values.keys()) {
        if names.contains_key(origin) { continue; }
        let digest = format!("{:x}", Sha256::digest(origin.to_string_lossy().as_bytes()));
        let base = format!("__thaw_support_{}", &digest[..16]);
        let mut selected = base.clone();
        let mut suffix = 0usize;
        while output.contains(&selected) || names.values().any(|name| name == &selected) {
            suffix += 1;
            selected = format!("{base}_{suffix}");
        }
        names.insert(origin.clone(), selected);
    }
    names
}

fn rewritten_owned_type_snippet(
    declaration: &OwnedDeclaration,
    emitted_scope: Option<&str>,
    public: &std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>>,
    support_names: &std::collections::BTreeMap<PathBuf, String>,
    views: &OwnedSourceViews,
) -> Result<String, String> {
    let mut edits = Vec::<(usize, usize, String)>::new();
    for edge in owned_type_reference_edges_with_views(declaration, views)? {
        let public_replacement = public.get(&edge.terminal).and_then(|occurrences| {
            occurrences.iter().filter(|occurrence|
                edge.kind == TypeReferenceKind::Type || occurrence.value_export)
                .find(|occurrence| occurrence.scope.as_deref() == emitted_scope)
                .or_else(|| occurrences.iter().find(|occurrence|
                    edge.kind == TypeReferenceKind::Type || occurrence.value_export))
                .map(|occurrence| occurrence.type_reference_from(emitted_scope))
        });
        let replacement = public_replacement.or_else(|| {
            support_names.get(&edge.terminal.0).map(|namespace| {
                let mut parts = vec![namespace.clone()];
                parts.extend(edge.terminal.1.iter().cloned());
                parts.push(edge.terminal.2.clone());
                parts.join(".")
            })
        });
        let Some(replacement) = replacement else { continue };
        let (start, end) = edge.full_range;
        if declaration.snippet.get(start..end) == Some(replacement.as_str()) { continue; }
        edits.push((start, end, replacement));
    }
    edits.sort_by_key(|(start, _, _)| *start);
    if edits.windows(2).any(|window| window[0].1 > window[1].0) {
        return Err(format!("overlapping type references in `{}`", declaration.origin.display()));
    }
    let mut rewritten = declaration.snippet.clone();
    for (start, end, replacement) in edits.into_iter().rev() {
        rewritten.replace_range(start..end, &replacement);
    }
    Ok(rewritten)
}

fn private_support_declaration_member(snippet: &str) -> String {
    let trimmed = snippet.trim();
    if let Some(body) = trimmed.strip_prefix("export declare ") {
        format!("export {body}")
    } else if let Some(body) = trimmed.strip_prefix("declare ") {
        format!("export {body}")
    } else if trimmed.starts_with("export ") {
        trimmed.to_string()
    } else {
        format!("export {trimmed}")
    }
}

fn render_private_support_declarations(
    support: &SourceSupportBindings,
    public: &std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>>,
    names: &std::collections::BTreeMap<PathBuf, String>,
    views: &OwnedSourceViews,
) -> Result<String, String> {
    let mut by_origin = std::collections::BTreeMap::<PathBuf, Vec<String>>::new();
    for ((origin, source_scope, _), fragments) in support.types.iter().chain(support.values.iter()) {
        for fragment in fragments {
            let rewritten = rewritten_owned_type_snippet(fragment, None, public, names, views)?;
            let mut member = private_support_declaration_member(&rewritten);
            for scope in source_scope.iter().rev() {
                member = format!("export namespace {scope} {{\n{member}\n}}");
            }
            let members = by_origin.entry(origin.clone()).or_default();
            if !members.contains(&member) { members.push(member); }
        }
    }
    let mut output = String::new();
    for (origin, members) in by_origin {
        let namespace = names.get(&origin).ok_or_else(||
            format!("missing support namespace for `{}`", origin.display()))?;
        output.push_str(&format!(
            "\ndeclare namespace {namespace} {{\n  type __thaw_private_support_marker__ = never;\n{}\n}}",
            members.join("\n"),
        ));
    }
    Ok(output)
}

fn append_private_type_closure(
    output: &mut String,
    emitted: &[EmittedOwnedDeclaration],
    views: &OwnedSourceViews,
) -> Result<(), String> {
    let mut public = public_type_occurrences(emitted);
    add_owned_origin_occurrences(&mut public, emitted, views);
    let support = referenced_private_type_closure_with_views(emitted, &public, views)?;
    let support_names = private_support_namespace_names(output, &support);
    let mut edits = Vec::<(usize, usize, String)>::new();
    let mut rewritten_intervals = std::collections::BTreeSet::new();
    for record in emitted {
        if record.metadata_only { continue; }
        if !record.declaration.children.is_empty() { continue; }
        if record.end == record.start {
            // A child that cannot be located in its formatted wrapper must
            // fail closed if it has source-owned references to rewrite.
            // A repeated append of the same source binding may legitimately
            // emit nothing after `append_flattened_type` deduplicates it.
            if emitted.iter().any(|other| other.end > other.start
                && other.declaration.origin == record.declaration.origin
                && other.declaration.source_scope == record.declaration.source_scope
                && other.declaration.local_name == record.declaration.local_name
                && other.declaration.snippet == record.declaration.snippet) {
                continue;
            }
            if !owned_type_reference_edges_with_views(&record.declaration, views)?.is_empty() {
                return Err(format!("unlocated owned declaration in `{}`", record.declaration.origin.display()));
            }
            continue;
        }
        // A multi-declarator variable contributes one public identity per
        // binding, but its complete statement is rewritten only once.
        if !rewritten_intervals.insert((record.start, record.end)) { continue; }
        let start = record.start;
        let end = record.end;
        let snippet = output.get(start..end).ok_or_else(||
            format!("invalid owned declaration interval in `{}`", record.declaration.origin.display()))?;
        let declaration = record.declaration.clone().with_snippet(snippet.to_string());
        let rewritten = rewritten_owned_type_snippet(
            &declaration, record.scope.as_deref(), &public, &support_names, views,
        )?;
        if rewritten != snippet { edits.push((start, end, rewritten)); }
    }
    edits.sort_by_key(|(start, _, _)| *start);
    if edits.windows(2).any(|window| window[0].1 > window[1].0) {
        return Err("overlapping owned declaration intervals".into());
    }
    for (start, end, rewritten) in edits.into_iter().rev() {
        output.replace_range(start..end, &rewritten);
    }
    output.push_str(&render_private_support_declarations(&support, &public, &support_names, views)?);
    Ok(())
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
struct PublicTypeOccurrence {
    scope: Option<String>,
    // The local structural binding remains addressable inside flattened
    // declaration text even when the export is renamed.
    structural_name: String,
    public_name: String,
    value_export: bool,
}

impl PublicTypeOccurrence {
    fn type_reference_from(&self, emitted_scope: Option<&str>) -> String {
        match (&self.scope, emitted_scope) {
            (Some(scope), Some(current)) if scope == current => self.structural_name.clone(),
            (Some(scope), _) => format!("{scope}.{}", self.public_name),
            (None, _) => self.structural_name.clone(),
        }
    }
}

fn public_name_has_value_export(snippet: &str, public_name: &str) -> bool {
    use thaw_parser::ast::{Decl, DefaultDecl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat};
    let Ok(module) = parse_labeled_declarations(snippet, "owned public value role".to_string()) else {
        return false;
    };
    module.body.iter().any(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            match &export.decl {
                Decl::Var(variable) => variable.decls.iter().any(|declarator|
                    matches!(&declarator.name, Pat::Ident(binding) if binding.id.sym == public_name)),
                Decl::TsInterface(_) | Decl::TsTypeAlias(_) => false,
                _ => is_runtime_declaration_snippet(snippet)
                    && declaration_identity(snippet).as_deref() == Some(public_name),
            }
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only =>
            export.specifiers.iter().any(|specifier| {
                let ExportSpecifier::Named(named) = specifier else { return false };
                if named.is_type_only { return false; }
                matches!(named.exported.as_ref().unwrap_or(&named.orig),
                    ModuleExportName::Ident(name) if name.sym == public_name)
            }),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) if public_name == "default" =>
            matches!(&default.decl, DefaultDecl::Class(_) | DefaultDecl::Fn(_)),
        ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) =>
            import.is_export && import.id.sym.as_ref() == public_name,
        _ => false,
    })
}

fn public_import_equals_name(snippet: &str) -> Option<String> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem};
    let module = parse_labeled_declarations(snippet, "owned public import alias".to_string()).ok()?;
    module.body.iter().find_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else {
            return None;
        };
        import.is_export.then(|| import.id.sym.to_string())
    })
}

fn is_var_declaration_snippet(snippet: &str) -> bool {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt};
    parse_labeled_declarations(snippet, "owned variable declaration".to_string())
        .is_ok_and(|module| module.body.iter().any(|item| matches!(item,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) if matches!(&export.decl, Decl::Var(_))
        ) || matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Var(_))))))
}

fn public_type_occurrences(
    emitted: &[EmittedOwnedDeclaration],
) -> std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>> {
    let mut by_source = std::collections::BTreeMap::<SourceTypeKey, Vec<PublicTypeOccurrence>>::new();
    for record in emitted {
        if record.public_names.is_empty() { continue; }
        // A selected class may be represented only by a following
        // `export { Hidden as Public }` marker. The marker has no type
        // declaration node, but still names the same source-owned class.
        let Some(local_name) = &record.declaration.local_name else { continue };
        let entries = by_source.entry((record.declaration.origin.clone(),
            record.declaration.source_scope.clone(), local_name.clone()))
            .or_default();
        for public_name in &record.public_names {
            let occurrence = PublicTypeOccurrence {
                scope: record.scope.clone(),
                structural_name: if let Some(alias) =
                    public_import_equals_name(&record.declaration.snippet) {
                    alias
                } else if is_var_declaration_snippet(&record.declaration.snippet) {
                    local_name.clone()
                } else {
                    declaration_identity(&record.declaration.snippet)
                        .unwrap_or_else(|| local_name.clone())
                },
                public_name: public_name.clone(),
                value_export: !is_type_only_namespace_binding(&record.declaration, public_name)
                    && (record.metadata_only
                        || public_name_has_value_export(&record.declaration.snippet, public_name)),
            };
            if !entries.contains(&occurrence) { entries.push(occurrence); }
        }
    }
    for entries in by_source.values_mut() { entries.sort(); }
    by_source
}

fn add_owned_origin_occurrences(
    available: &mut std::collections::BTreeMap<SourceTypeKey, Vec<PublicTypeOccurrence>>,
    emitted: &[EmittedOwnedDeclaration],
    views: &OwnedSourceViews,
) {
    // Public aliases were inserted first. These additional exact physical
    // bindings are declaration-only lookup targets, so a selected class's
    // self/sibling references reuse its owner container instead of adding
    // the same class again under private support.
    for record in emitted {
        if record.metadata_only || record.end <= record.start { continue; }
        let Some(scope) = &record.scope else { continue };
        if !views.owned_names.values().any(|owner|
            scope == owner || scope.starts_with(&format!("{owner}."))) { continue; }
        let Some(local) = &record.declaration.local_name else { continue };
        let Some(structural_name) = declaration_identity(&record.declaration.snippet) else { continue };
        let occurrence = PublicTypeOccurrence {
            scope: Some(scope.clone()),
            structural_name: structural_name.clone(),
            public_name: structural_name,
            value_export: is_runtime_declaration_snippet(&record.declaration.snippet),
        };
        let entries = available.entry((record.declaration.origin.clone(),
            record.declaration.source_scope.clone(), local.clone())).or_default();
        if !entries.contains(&occurrence) { entries.push(occurrence); }
    }
}

fn record_owned_entry_declarations(
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    entry_path: &Path,
    retained_entry: &str,
    base_offset: usize,
) -> Result<(), String> {
    use thaw_parser::ast::{Decl, DefaultDecl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        retained_entry,
        thaw_parser::common::FileName::Custom(
            format!("{} (retained declaration entry)", entry_path.display()).into(),
        ),
    )?;
    let mut aliases = std::collections::BTreeMap::<String, Vec<String>>::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else { continue };
            let ModuleExportName::Ident(local) = &named.orig else { continue };
            let ModuleExportName::Ident(public) = named.exported.as_ref().unwrap_or(&named.orig) else { continue };
            aliases.entry(local.sym.to_string()).or_default().push(public.sym.to_string());
        }
    }
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) = item {
            let local = match &default.decl {
                DefaultDecl::Class(class) => class.ident.as_ref().map(|id| id.sym.to_string()),
                DefaultDecl::Fn(function) => function.ident.as_ref().map(|id| id.sym.to_string()),
                _ => None,
            };
            if let Some(local) = local {
                let snippet = source_map.span_to_snippet(item.span()).map_err(|error|
                    format!("failed to locate retained default declaration: {error:?}"))?;
                let start = source_map.lookup_byte_offset(item.span().lo).pos.0 as usize;
                let end = source_map.lookup_byte_offset(item.span().hi).pos.0 as usize;
                if retained_entry.get(start..end) != Some(snippet.as_str()) {
                    return Err("retained default declaration span mismatch".into());
                }
                let mut owned = OwnedDeclaration::new(entry_path, snippet);
                owned.local_name = Some(local);
                emitted.push(EmittedOwnedDeclaration {
                    declaration: owned, scope: None, public_names: vec!["default".into()],
                    start: base_offset + start, end: base_offset + end,
                    metadata_only: false,
                });
            }
            continue;
        }
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        if !matches!(declaration, Some(Decl::Class(_) | Decl::Fn(_) | Decl::TsInterface(_)
            | Decl::TsTypeAlias(_) | Decl::TsEnum(_) | Decl::TsModule(_) | Decl::Var(_))) { continue; }
        let snippet = source_map.span_to_snippet(item.span()).map_err(|error|
            format!("failed to locate retained entry declaration: {error:?}"))?;
        let start = source_map.lookup_byte_offset(item.span().lo).pos.0 as usize;
        let end = source_map.lookup_byte_offset(item.span().hi).pos.0 as usize;
        if retained_entry.get(start..end) != Some(snippet.as_str()) {
            return Err("retained entry declaration span mismatch".into());
        }
        let owned = OwnedDeclaration::new(entry_path, snippet);
        if let Some(Decl::Var(variable)) = declaration {
            for declarator in &variable.decls {
                let Pat::Ident(binding) = &declarator.name else { continue };
                let local = binding.id.sym.to_string();
                let mut selected = owned.clone();
                selected.local_name = Some(local.clone());
                let mut public_names = aliases.get(&local).cloned().unwrap_or_default();
                if matches!(item, ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(_))) {
                    public_names.push(local);
                }
                public_names.sort();
                public_names.dedup();
                emitted.push(EmittedOwnedDeclaration {
                    declaration: selected, scope: None, public_names,
                    start: base_offset + start, end: base_offset + end,
                    metadata_only: false,
                });
            }
            continue;
        }
        let mut public_names = owned.local_name.as_ref().and_then(|local|
            aliases.get(local)).cloned().unwrap_or_default();
        if matches!(item, ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(_))) {
            public_names.extend(emitted_public_names(&owned.snippet,
                owned.local_name.as_deref(), owned.local_name.as_deref()));
        }
        public_names.sort();
        public_names.dedup();
        record_owned_namespace_children(emitted, &owned, None, Some(base_offset + start));
        emitted.push(EmittedOwnedDeclaration {
            declaration: owned, scope: None, public_names,
            start: base_offset + start, end: base_offset + end,
            metadata_only: false,
        });
    }
    Ok(())
}

fn record_owned_namespace_children(
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    declaration: &OwnedDeclaration,
    scope: Option<&str>,
    written_start: Option<usize>,
) {
    if declaration.children.is_empty() { return; }
    let Some(name) = declaration_identity(&declaration.snippet) else { return; };
    let child_scope = scope.map_or_else(|| name.clone(), |scope| format!("{scope}.{name}"));
    let mut cursor = 0;
    for child in &declaration.children {
        let trimmed = child.snippet.trim_start();
        let normalized = trimmed.strip_prefix("export declare ")
            .map_or_else(|| trimmed.to_string(), |rest| format!("export {rest}"));
        let relative = declaration.snippet[cursor..].find(&normalized).map(|offset| cursor + offset);
        if let Some(relative) = relative { cursor = relative + normalized.len(); }
        let start = relative.and_then(|offset| written_start.map(|start| start + offset)).unwrap_or(0);
        let end = if start == 0 { 0 } else { start + normalized.len() };
        emitted.push(EmittedOwnedDeclaration {
            declaration: child.clone(),
            scope: Some(child_scope.clone()),
            public_names: namespace_member_public_names(child),
            start,
            end,
            metadata_only: false,
        });
        record_owned_namespace_children(emitted, child, Some(&child_scope),
            (start != 0).then_some(start));
    }
}

fn append_owned_declaration(
    output: &mut String,
    emitted: &mut Vec<EmittedOwnedDeclaration>,
    declaration: OwnedDeclaration,
    scope: Option<&str>,
    public_name: Option<&str>,
    flattened_type: bool,
) {
    let start = output.len();
    if flattened_type {
        append_flattened_type(output, declaration.snippet.clone());
    } else {
        output.push('\n');
        output.push_str(&declaration.snippet);
    }
    let written_start = output.get(start..).and_then(|written|
        written.strip_prefix('\n')).filter(|written| written.starts_with(&declaration.snippet))
        .map(|_| start + 1);
    record_owned_namespace_children(emitted, &declaration, scope, written_start);
    let public_names = emitted_public_names(
        &declaration.snippet, declaration.local_name.as_deref(), public_name,
    );
    emitted.push(EmittedOwnedDeclaration {
        declaration,
        scope: scope.map(str::to_owned),
        public_names,
        start,
        end: output.len(),
        metadata_only: false,
    });
}

fn reexported_owned_declarations_as(
    declarations: Vec<OwnedDeclaration>, exported: &str, origin: &Path, type_only: bool,
) -> Vec<OwnedDeclaration> {
    let rewritten = reexported_declarations_as(
        declarations.iter().map(|declaration| declaration.snippet.clone()).collect(),
        exported, origin, type_only,
    );
    declarations.into_iter().zip(rewritten).map(|(declaration, snippet)|
        declaration.with_snippet(snippet)).collect()
}

fn all_reexported_type_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<String>, String> {
    Ok(all_reexported_type_declarations_owned(path, visited)?
        .into_iter().map(|declaration| declaration.snippet).collect())
}

fn all_reexported_type_declarations_owned(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<OwnedDeclaration>, String> {
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
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
    let mut declarations = Vec::new();
    let value_imports = named_import_targets(path, &module);
    let type_imports = named_type_import_targets(path, &module);
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
                declarations.push(OwnedDeclaration::new(path, source_map.span_to_snippet(export.span()).map_err(|error| {
                    format!("failed to read type declaration in `{}`: {error:?}", path.display())
                })?));
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        for snippet in all_reexported_type_declarations_owned(&target, visited)? {
                            declarations.push(if export.type_only {
                                snippet.clone().with_snippet(type_only_declaration(snippet.snippet, None))
                            } else {
                                snippet
                            });
                        }
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else { continue };
                    let ModuleExportName::Ident(original) = &named.orig else { continue };
                    let exported = named.exported.as_ref().unwrap_or(&named.orig);
                    let ModuleExportName::Ident(exported) = exported else { continue };
                    let imported = value_imports.get(original.sym.as_ref())
                        .or_else(|| type_imports.get(original.sym.as_ref()));
                    let imported_only_as_type = export.src.is_none()
                        && !value_imports.contains_key(original.sym.as_ref())
                        && type_imports.contains_key(original.sym.as_ref());
                    let target = export.src.as_ref()
                        .and_then(|source| source.value.as_str())
                        .and_then(|source| declaration_reexport_path(path, source))
                        .or_else(|| imported.map(|(path, _)| path.clone()))
                        .or_else(|| export.src.is_none().then(|| path.to_path_buf()));
                    let Some(target) = target else { continue };
                    let target_name = if export.src.is_some() {
                        original.sym.as_ref()
                    } else {
                        imported.map_or(original.sym.as_ref(), |(_, name)| name.as_str())
                    };
                    let mut status_visited = std::collections::BTreeSet::new();
                    let inherited_type_only = imported_only_as_type
                        || reexport_is_type_only(&target, target_name, &mut status_visited)? == Some(true);
                    let resolved = reexported_class_or_interface_declarations_owned(&target, target_name)?
                        .into_iter().filter(|declaration| is_type_declaration_snippet(&declaration.snippet)).collect();
                    declarations.extend(reexported_owned_declarations_as(
                        resolved, exported.sym.as_ref(), path,
                        export.type_only || named.is_type_only || inherited_type_only,
                    ));
                }
                for specifier in &export.specifiers {
                    if !export.type_only { continue; }
                    let ExportSpecifier::Namespace(namespace) = specifier else { continue };
                    let ModuleExportName::Ident(alias) = &namespace.name else { continue };
                    let Some(source) = export.src.as_ref().and_then(|source| source.value.as_str()) else { continue };
                    let Some(target) = declaration_reexport_path(path, source) else { continue };
                    let children = all_reexported_type_declarations_owned(&target, visited)?;
                    let snippet = type_only_namespace_declaration(alias.sym.as_ref(),
                        children.iter().map(|child| child.snippet.clone()).collect());
                    let mut owned = OwnedDeclaration::new(path, snippet);
                    owned.children = children;
                    declarations.push(owned);
                }
            }
            _ => {}
        }
    }
    visited.remove(path);
    Ok(declarations)
}

// Keep the declaration's structural type while marking its public name as
// type-only. The bridge reads this explicit export separately from value
// exports, because even an unexported `declare class` has an instance shape.
fn type_only_declaration(snippet: String, exported: Option<&str>) -> String {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt, TsModuleName};
    let Ok(module) = parse_labeled_declarations(&snippet, "generated type-only declaration snippet".to_string()) else { return snippet };
    let declaration = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
        ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
        _ => None,
    });
    let name = declaration.and_then(|declaration| match declaration {
        Decl::Class(class) => Some(class.ident.sym.as_ref()),
        Decl::TsInterface(interface) => Some(interface.id.sym.as_ref()),
        Decl::TsTypeAlias(alias) => Some(alias.id.sym.as_ref()),
        Decl::TsEnum(enumeration) => Some(enumeration.id.sym.as_ref()),
        Decl::TsModule(module) => match &module.id {
            TsModuleName::Ident(ident) => Some(ident.sym.as_ref()),
            _ => None,
        },
        _ => None,
    });
    let Some(name) = name else { return snippet };
    let public = exported.unwrap_or(name);
    let bare = snippet.trim_start().strip_prefix("export ")
        .map_or(snippet.as_str(), |declaration| declaration);
    // A value alias generated for this class may cross a later `export
    // type *` edge. Carry its public name as a type and remove the value
    // export; the class body and its self references stay under `name`.
    let value_alias_prefix = format!("export {{ {name} as ");
    let own_type_marker = format!("export type {{ {name} }};");
    let mut aliases = Vec::new();
    let body = bare.lines().filter(|line| {
        if let Some(alias) = line.strip_prefix(&value_alias_prefix)
            .and_then(|suffix| suffix.strip_suffix(" };")) {
            aliases.push(alias.to_string());
            false
        } else if *line == own_type_marker.as_str() {
            false
        } else {
            true
        }
    }).collect::<Vec<_>>().join("\n");
    let mut result = format!("{body}\nexport type {{ {name} }};");
    if name != public {
        aliases.push(public.to_string());
    }
    for alias in aliases {
        let marker = format!("export type {{ {name} as {alias} }};");
        if !result.contains(&marker) {
            result.push('\n');
            result.push_str(&marker);
        }
    }
    result
}

fn append_flattened_type(output: &mut String, snippet: String) {
    // The same source declaration can be reached through `export type *`
    // and an explicitly named alias. Keep one structural declaration but
    // all public type markers, so the flattened `.d.ts` stays parseable.
    if snippet.trim_start().starts_with("declare namespace ") {
        if !output.contains(&snippet) {
            output.push('\n');
            output.push_str(&snippet);
        }
        return;
    }
    if let Some(marker) = snippet.find("\nexport type {") {
        let declaration = &snippet[..marker];
        if output.contains(declaration) {
            for alias in snippet[marker..].lines().filter(|line| line.starts_with("export type {") || line.starts_with("export {")) {
                if !output.contains(alias) {
                    output.push('\n');
                    output.push_str(alias);
                }
            }
            return;
        }
    } else if output.contains(&snippet) {
        return;
    }
    output.push('\n');
    output.push_str(&snippet);
}

fn type_only_namespace_declaration(alias: &str, snippets: Vec<String>) -> String {
    let body = snippets.into_iter().map(|snippet| {
        let snippet = snippet.trim_start();
        snippet.strip_prefix("export declare ")
            .map_or_else(|| snippet.to_string(), |rest| format!("export {rest}"))
    }).collect::<Vec<_>>().join("\n");
    format!("declare namespace {alias} {{\n{body}\n}}\nexport type {{ {alias} }};")
}

fn namespace_member_declarations_owned(
    declaration: OwnedDeclaration,
) -> Result<Vec<OwnedDeclaration>, String> {
    Ok(namespace_member_declarations(&declaration.snippet)?
        .into_iter().map(|snippet| declaration.clone().with_snippet(snippet)).collect())
}

fn namespace_member_declarations(snippet: &str) -> Result<Vec<String>, String> {
    use thaw_parser::common::{SourceMapper, Spanned};
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        snippet,
        thaw_parser::common::FileName::Custom("generated namespace member".into()),
    )?;
    let has_alias_marker = module.body.iter().any(|item|
        matches!(item, thaw_parser::ast::ModuleItem::ModuleDecl(
            thaw_parser::ast::ModuleDecl::ExportNamed(_))));
    module.body.iter().map(|item| {
        let text = source_map.span_to_snippet(item.span())
            .map_err(|error| format!("failed to read namespace member declaration: {error:?}"))?;
        let text = text.trim_start();
        Ok(if let Some(rest) = text.strip_prefix("export declare ") {
            format!("export {rest}")
        } else if let Some(rest) = text.strip_prefix("declare ") {
            // A named re-export already controls which member is public.
            if has_alias_marker { rest.to_string() } else { format!("export {rest}") }
        } else {
            text.to_string()
        })
    }).collect()
}

fn is_type_declaration_snippet(snippet: &str) -> bool {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt};
    let Ok(module) = parse_labeled_declarations(snippet, "extracted type declaration snippet".to_string()) else { return false };
    module.body.iter().any(|item| {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        matches!(declaration,
            Some(Decl::Class(_) | Decl::TsInterface(_) | Decl::TsTypeAlias(_)
                | Decl::TsEnum(_) | Decl::TsModule(_)))
    })
}

fn is_runtime_declaration_snippet(snippet: &str) -> bool {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt};
    let Ok(module) = parse_labeled_declarations(snippet, "extracted runtime declaration snippet".to_string()) else { return false };
    module.body.iter().any(|item| {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        matches!(declaration, Some(Decl::Class(_) | Decl::Fn(_) | Decl::Var(_)
            | Decl::TsEnum(_) | Decl::TsModule(_)))
    })
}

fn declaration_identity(snippet: &str) -> Option<String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Pat, Stmt, TsModuleName};
    let module = parse_labeled_declarations(snippet, "extracted declaration identity snippet".to_string()).ok()?;
    let declaration = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
        ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
        _ => None,
    })?;
    match declaration {
        Decl::Class(class) => Some(class.ident.sym.to_string()),
        Decl::Fn(function) => Some(function.ident.sym.to_string()),
        Decl::Var(variable) => variable.decls.first().and_then(|declarator| match &declarator.name {
            Pat::Ident(binding) => Some(binding.id.sym.to_string()),
            _ => None,
        }),
        Decl::TsInterface(interface) => Some(interface.id.sym.to_string()),
        Decl::TsTypeAlias(alias) => Some(alias.id.sym.to_string()),
        Decl::TsEnum(enumeration) => Some(enumeration.id.sym.to_string()),
        Decl::TsModule(module) => match &module.id {
            TsModuleName::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    }
}

// The resolver returns the selected declaration first, followed by its
// merged namespace/overloads and any imported superclass or type support.
// Only declarations with the selected binding's identity receive the public
// alias; an appended Base must remain Base when exporting Derived.
fn reexported_declarations_as(
    declarations: Vec<String>,
    exported: &str,
    origin: &Path,
    type_only: bool,
) -> Vec<String> {
    let selected = declarations.first().and_then(|snippet| declaration_identity(snippet));
    declarations.into_iter().map(|snippet| {
        let is_selected = selected.is_some()
            && selected.as_deref() == declaration_identity(&snippet).as_deref();
        if type_only {
            return type_only_declaration(snippet, is_selected.then_some(exported));
        }
        if !is_selected {
            return if is_type_declaration_snippet(&snippet) {
                type_only_declaration(snippet, None)
            } else {
                snippet
            };
        }
        if is_runtime_declaration_snippet(&snippet) {
            export_value_declaration_as(snippet, exported, origin)
        } else {
            type_only_declaration(snippet, Some(exported))
        }
    }).collect()
}

fn export_value_declaration_as(snippet: String, exported: &str, origin: &Path) -> String {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt, TsModuleName};
    let Ok(module) = parse_labeled_declarations(&snippet, format!("{} (exported declaration snippet)", origin.display())) else { return snippet };
    let declaration = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
        ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
        _ => None,
    });
    match declaration {
        Some(Decl::Class(class)) => {
            let name = class.ident.sym.as_ref();
            if name == exported { return snippet; }
            let bare = snippet.trim_start().strip_prefix("export ").unwrap_or(&snippet);
            // Keep the class's declared identity for `extends` and self
            // returns. The bridge clones its shape for the public alias.
            format!("{bare}\nexport type {{ {name} }};\nexport {{ {name} as {exported} }};")
        }
        Some(Decl::TsEnum(enumeration)) => {
            let name = enumeration.id.sym.as_ref();
            if name == exported { return snippet; }
            let bare = snippet.trim_start().strip_prefix("export ").unwrap_or(&snippet);
            format!("{bare}\nexport {{ {name} as {exported} }};")
        }
        Some(Decl::TsModule(module)) => {
            let TsModuleName::Ident(ident) = &module.id else { return snippet };
            let name = ident.sym.as_ref();
            if name == exported { return snippet; }
            let bare = snippet.trim_start().strip_prefix("export ").unwrap_or(&snippet);
            format!("{bare}\nexport {{ {name} as {exported} }};")
        }
        _ => export_function_as(snippet, exported, origin),
    }
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
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(entry_path.to_path_buf()),
    )?;

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
    include_type_only: bool,
) -> Result<Vec<(String, PathBuf)>, String> {
    collect_namespace_reexports_with_views(
        entry_path, module, visited, include_type_only, &OwnedSourceViews::default(),
    )
}

fn collect_namespace_reexports_with_views(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
    visited: &mut std::collections::BTreeSet<PathBuf>,
    include_type_only: bool,
    views: &OwnedSourceViews,
) -> Result<Vec<(String, PathBuf)>, String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    if !visited.insert(entry_path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if include_type_only || !export.type_only => {
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
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export))
                if include_type_only || !export.type_only => {
                let Some(source) = export.src.value.as_str() else {
                    continue;
                };
                let Some(target_path) = declaration_reexport_path(entry_path, source) else {
                    continue;
                };
                let target_source = views.read(&target_path)?;
                let target_module = thaw_parser::parse_declarations_with_source_map_named(
                    &target_source, thaw_parser::common::FileName::Real(target_path.clone()),
                )?.0;
                found.extend(collect_namespace_reexports_with_views(
                    &target_path,
                    &target_module,
                    visited,
                    include_type_only,
                    views,
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

// Preserve one variable declarator and the statement modifiers for both
// callable values and ordinary re-exported values.
fn selected_var_declaration_snippet(
    source_map: &thaw_parser::common::SourceMap,
    var_decl: &thaw_parser::ast::VarDecl,
    declarator: &thaw_parser::ast::VarDeclarator,
    exported: bool,
) -> Result<String, String> {
    use thaw_parser::ast::VarDeclKind;
    use thaw_parser::common::{SourceMapper, Spanned};

    let declarator_snippet = source_map.span_to_snippet(declarator.span())
        .map_err(|error| format!("failed to read variable declarator: {error:?}"))?;
    let kind = match var_decl.kind {
        VarDeclKind::Const => "const",
        VarDeclKind::Let => "let",
        VarDeclKind::Var => "var",
    };
    Ok(format!(
        "{}{}{kind} {declarator_snippet};",
        if exported { "export " } else { "" },
        if var_decl.declare { "declare " } else { "" },
    ))
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
/// Take only the requested declarator from a `const a: A, b: B` statement.
/// Keeping the whole statement confuses both the alias renamer (it sees `a`)
/// and the bridge's signature extractor (which sees an unrelated binding).
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
    var_decl: &thaw_parser::ast::VarDecl,
    declarator: &thaw_parser::ast::VarDeclarator,
    exported: bool,
    binding: &thaw_parser::ast::BindingIdent,
    path: &Path,
) -> Result<Option<String>, String> {
    use thaw_parser::ast::{
        Decl, ModuleDecl, ModuleItem, TsEntityName, TsFnOrConstructorType, TsType, TsTypeElement,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    /// Every top-level `type NAME = T;`/`interface NAME { ... }` in the
    /// file at `path` (bare or exported), as re-parseable snippets --
    /// used to inline a sibling `.d.ts` file's declarations after
    /// chasing a cross-file `import("./sibling.js").Name<Args>` type
    /// reference. `.d.ts` type declarations are erasable and side-
    /// effect-free, so including every one (not just the referenced
    /// `Name`) is harmless even when most turn out unrelated -- same
    /// reasoning this function's own local-alias branch below already
    /// uses. Also recurses into every relative `import ... from
    /// "./other.js"` this file itself has (type-only or not -- a plain
    /// value import can equally carry a type used only in a type
    /// position, and re-deriving that distinction isn't worth it when
    /// over-including is free), the same "chase one hop, inline
    /// everything found, let unrelated declarations sit unused" policy
    /// one level deeper -- real example: `tar`'s own `make-command.
    /// d.ts` imports `TarOptions`/`TarOptionsWithAliasesAsyncFile`/etc.
    /// from `./options.js`, which never gets fetched otherwise (no
    /// `package.json` exports-map entry reaches it either). `visited`
    /// guards against re-visiting the same file twice (a cycle, or two
    /// different imports resolving to the same path).
    fn local_type_declaration_snippets(
        path: &Path,
        visited: &mut std::collections::BTreeSet<PathBuf>,
    ) -> Result<Vec<String>, String> {
        use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Stmt};
        use thaw_parser::common::{SourceMapper, Spanned};

        // Canonicalize the visited key for the same reason
        // `declaration_reexport_path` does: a non-normalized path (an
        // extra `./` segment) must not defeat the cycle guard.
        if !visited.insert(path.canonicalize().unwrap_or_else(|_| path.to_path_buf())) {
            return Ok(Vec::new());
        }
        let source = fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read `{}`'s type declarations: {error}",
                path.display()
            )
        })?;
        let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
            &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
        )?;
        let mut declarations = module
            .body
            .iter()
            .filter_map(|item| {
                let decl = match item {
                    ModuleItem::Stmt(Stmt::Decl(decl)) => decl,
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
                    _ => return None,
                };
                let span = match decl {
                    Decl::TsInterface(iface) => iface.span(),
                    Decl::TsTypeAlias(alias) => alias.span(),
                    _ => return None,
                };
                Some(source_map.span_to_snippet(span).map_err(|error| {
                    format!("failed to read a local type declaration: {error:?}")
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
                continue;
            };
            let Some(specifier) = import.src.value.as_str() else {
                continue;
            };
            let Some(target) = declaration_reexport_path(path, specifier) else {
                continue;
            };
            declarations.extend(local_type_declaration_snippets(&target, visited)?);
        }
        Ok(declarations)
    }

    let Some(annotation) = binding.type_ann.as_ref() else {
        return Ok(None);
    };
    let const_snippet = selected_var_declaration_snippet(source_map, var_decl, declarator, exported)?;
    match annotation.type_ann.as_ref() {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(_)) => {
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
            let mut combined = const_snippet;
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
        // A cross-file inline import type (`import("./make-command.js").
        // TarCommand<Pack, PackSync>`) -- real example: `tar`'s own
        // `create.d.ts`/`extract.d.ts`/`list.d.ts`/`update.d.ts`, each a
        // bare `export declare const NAME: import("./make-command.js").
        // TarCommand<...>;` with no local type at all. `make-command.d.ts`
        // isn't a `package.json` exports-map entry, so nothing else ever
        // fetches it. Resolved the same way an ordinary `export * from
        // "./x.js"` re-export already is (`declaration_reexport_path`),
        // then every local type alias/interface in that sibling file is
        // inlined (see `local_type_declaration_snippets`'s own doc
        // comment for why "every one", not just the referenced name).
        //
        // The const's own declaration is re-synthesized (name + bare
        // qualifier + its original type-argument text) rather than
        // string-replacing the `import("...").` prefix out of the
        // original snippet -- simpler than locating exactly where the
        // qualifier starts within the snippet's own span arithmetic, and
        // just as correct: nothing downstream cares about the const
        // declaration's exact original formatting.
        TsType::TsImportType(import_type) => {
            let Some(qualifier) = import_type.qualifier.as_ref() else {
                return Ok(None);
            };
            let Some(specifier) = import_type.arg.value.as_str() else {
                return Ok(None);
            };
            let Some(sibling_path) = declaration_reexport_path(path, specifier) else {
                return Ok(None);
            };
            let qualifier_name = match qualifier {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            let type_args = import_type
                .type_args
                .as_ref()
                .map(|args| source_map.span_to_snippet(args.span()))
                .transpose()
                .map_err(|error| format!("failed to read a type argument list: {error:?}"))?
                .unwrap_or_default();
            let mut combined = format!(
                "export declare const {}: {qualifier_name}{type_args};\n",
                binding.id.sym
            );
            for snippet in
                local_type_declaration_snippets(&sibling_path, &mut std::collections::BTreeSet::new())?
            {
                combined.push_str(&snippet);
                combined.push('\n');
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
    path: &Path,
) -> Result<Vec<(String, String)>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Pat};

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
            if let Some(snippet) = callable_const_declaration_snippet(
                module,
                source_map,
                var_decl,
                declarator,
                true,
                binding,
                path,
            )? {
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
fn declared_function_name_range(snippet: &str) -> Option<(usize, usize)> {
    use thaw_parser::ast::{Decl, DefaultDecl, ModuleDecl, ModuleItem, Pat, Stmt, TsModuleName};
    use thaw_parser::common::Spanned;

    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        snippet, thaw_parser::common::FileName::Custom("extracted function declaration snippet".into()),
    ).ok()?;
    // The first declaration is the callable binding, possibly followed by
    // an interface supplying its signature. Use its AST identifier span so
    // whitespace/comments and keyword text in comments cannot move it.
    let ident_span = module.body.iter().find_map(|item| {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) => {
                return match &default.decl {
                    DefaultDecl::Fn(function) => function.ident.as_ref().map(|ident| ident.span()),
                    DefaultDecl::Class(class) => class.ident.as_ref().map(|ident| ident.span()),
                    _ => None,
                };
            }
            _ => None,
        }?;
        match declaration {
            Decl::Fn(function) => Some(function.ident.span()),
            Decl::Class(class) => Some(class.ident.span()),
            Decl::TsInterface(interface) => Some(interface.id.span()),
            Decl::TsModule(namespace) => match &namespace.id {
                TsModuleName::Ident(ident) => Some(ident.span()),
                TsModuleName::Str(_) => None,
            },
            Decl::Var(variable) => variable.decls.first().and_then(|declarator| {
                if let Pat::Ident(binding) = &declarator.name {
                    Some(binding.id.span())
                } else {
                    None
                }
            }),
            _ => None,
        }
    })?;
    let start = source_map.lookup_byte_offset(ident_span.lo).pos.0 as usize;
    let end = source_map.lookup_byte_offset(ident_span.hi).pos.0 as usize;
    (start < end && snippet.get(start..end).is_some()).then_some((start, end))
}

fn rename_declared_function(snippet: String, exported: &str) -> String {
    let Some((name_start, name_end)) = declared_function_name_range(&snippet) else {
        return snippet;
    };
    if &snippet[name_start..name_end] == exported {
        return snippet;
    }
    let mut renamed = String::with_capacity(snippet.len());
    renamed.push_str(&snippet[..name_start]);
    renamed.push_str(exported);
    renamed.push_str(&snippet[name_end..]);
    renamed
}

// A reserved public property name is legal in an export specifier but not
// as a function's own declaration name. Keep the internal binding valid so
// the bridge can carry the public alias into the generated qualified shim.
fn export_function_as(mut snippet: String, exported: &str, origin: &Path) -> String {
    const GENERATED_ALIAS: &str = "\n/* __thaw_public_function_alias__ */\nexport { ";
    if let Some((declaration, alias)) = snippet.rsplit_once(GENERATED_ALIAS) {
        if alias.ends_with(" };") {
            snippet = declaration.to_string();
        }
    }
    if !is_ecmascript_keyword(exported) {
        return rename_declared_function(snippet, exported);
    }
    if declared_function_name_range(&snippet).is_none() {
        return snippet;
    }
    // Two source files can both declare `f` and independently export a
    // reserved property. Use the origin plus public key for a valid binding:
    // all overloads at one origin share it, unrelated files never do.
    let origin_hash = format!("{:x}", Sha256::digest(origin.to_string_lossy().as_bytes()));
    let internal = format!(
        "__thaw_public_{}_{}",
        exported.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        &origin_hash[..16],
    );
    snippet = rename_declared_function(snippet, &internal);
    snippet.push_str(&format!("{GENERATED_ALIAS}{internal} as {exported} }};"));
    snippet
}

// The function/class AST span starts at its declaration keyword, after the
// `export default` wrapper. Build an ordinary ambient declaration from that
// exact body; comments/line breaks around the wrapper cannot affect it.
fn reexported_default_declaration(
    body: &str,
    keyword: &str,
    origin: &Path,
    declared_name: Option<&str>,
    is_async: bool,
    is_abstract: bool,
) -> Result<String, String> {
    let digest = format!("{:x}", Sha256::digest(origin.to_string_lossy().as_bytes()));
    let internal = format!("__thaw_default_{keyword}_{}", &digest[..16]);
    let expected = declared_name.unwrap_or(&internal);
    for (offset, _) in body.match_indices(keyword) {
        let prefix = &body[..offset];
        let prefix = if is_async {
            let Some(prefix) = prefix.strip_prefix("async") else { continue };
            prefix
        } else {
            prefix
        };
        let tail = &body[offset + keyword.len()..];
        let name = declared_name.map_or_else(|| format!(" {internal}"), |_| String::new());
        let abstract_modifier = if is_abstract { "abstract " } else { "" };
        let candidate = format!("export declare {abstract_modifier}{prefix}{keyword}{name}{tail}");
        if declaration_identity(&candidate).as_deref() == Some(expected) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "failed to normalize default {keyword} declaration in `{}`",
        origin.display()
    ))
}

fn all_reexported_function_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, String)>, String> {
    Ok(all_reexported_function_declarations_owned(path, visited)?
        .into_iter().map(|(name, declaration)| (name, declaration.snippet)).collect())
}

fn all_reexported_function_declarations_owned(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, OwnedDeclaration)>, String> {
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
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
    let mut declarations = Vec::new();
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) = item {
            if let Decl::Fn(function) = &declaration.decl {
                declarations.push((
                    function.ident.sym.to_string(),
                    OwnedDeclaration::new(path, source_map
                        .span_to_snippet(declaration.span())
                        .map_err(|error| {
                            format!(
                                "failed to read declaration for `{}`: {error:?}",
                                function.ident.sym
                            )
                        })?),
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
    let mut local_declarations: std::collections::HashMap<String, Vec<OwnedDeclaration>> =
        std::collections::HashMap::new();
    for item in &module.body {
        let function = match item {
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) => {
                Some((function, function.span()))
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) => {
                match &declaration.decl {
                    Decl::Fn(function) => Some((function, declaration.span())),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((function, span)) = function else { continue };
        let snippet = source_map.span_to_snippet(span).map_err(|error| {
            format!("failed to read declaration for `{}`: {error:?}", function.ident.sym)
        })?;
        local_declarations
            .entry(function.ident.sym.to_string())
            .or_default()
            .push(OwnedDeclaration::new(path, snippet));
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
            if let Some(snippet) = callable_const_declaration_snippet(
                &module,
                &source_map,
                var_decl,
                declarator,
                false,
                binding,
                path,
            )? {
                local_declarations
                    .entry(binding.id.sym.to_string())
                    .or_default()
                    .push(OwnedDeclaration::new(path, snippet));
            }
        }
    }
    let named_import_targets = named_import_targets(path, &module);
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
            let snippets = if let Some(snippets) = local_declarations.get(&original) {
                snippets.clone()
            } else if let Some((target, target_name)) = named_import_targets.get(&original) {
                let mut nested_visited = visited.clone();
                all_reexported_function_declarations_owned(target, &mut nested_visited)?
                    .into_iter()
                    .filter_map(|(name, snippet)| (name == *target_name).then_some(snippet))
                    .collect()
            } else {
                continue;
            };
            for snippet in snippets {
                if exported == original && snippet.snippet.trim_start().starts_with("export ") {
                    continue;
                }
                let text = export_function_as(snippet.snippet.clone(), &exported, path);
                declarations.push((exported.clone(), snippet.with_snippet(text)));
            }
        }
    }
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        declarations.extend(all_reexported_function_declarations_owned(
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
                    for mut snippet in reexported_function_declarations_owned(
                        &target,
                        &original,
                        &mut named_visited,
                    )? {
                        if exported != original {
                            let text = export_function_as(snippet.snippet.clone(), &exported, path);
                            snippet = snippet.with_snippet(text);
                        }
                        declarations.push((exported.clone(), snippet));
                    }
                }
            }
            _ => {}
        }
    }
    declarations.extend(callable_const_declarations(&module, &source_map, path)?
        .into_iter().map(|(name, snippet)| (name, OwnedDeclaration::new(path, snippet))));
    Ok(declarations)
}

fn reexported_function_declarations(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<String>, String> {
    Ok(reexported_function_declarations_owned(path, name, visited)?
        .into_iter().map(|declaration| declaration.snippet).collect())
}

fn reexported_function_declarations_owned(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<OwnedDeclaration>, String> {
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
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
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
                OwnedDeclaration::new(path, source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| {
                        format!("failed to read declaration for `{name}`: {error:?}")
                    })?),
            );
        }
    }
    if declarations.is_empty() {
        for (const_name, snippet) in callable_const_declarations(&module, &source_map, path)? {
            if const_name == name {
                declarations.push(OwnedDeclaration::new(path, snippet));
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
                let function = match item {
                    ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) => {
                        Some((function, function.span()))
                    }
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) => {
                        match &declaration.decl {
                            Decl::Fn(function) => Some((function, declaration.span())),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((function, span)) = function {
                    if function.ident.sym.as_ref() == local_name.as_str() {
                        let snippet = source_map.span_to_snippet(span).map_err(|error| {
                            format!("failed to read declaration for `{local_name}`: {error:?}")
                        })?;
                        declarations.push(OwnedDeclaration::new(path, snippet.clone())
                            .with_snippet(export_function_as(snippet, name, path)));
                    }
                }
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
                            var_decl,
                            declarator,
                            false,
                            binding,
                            path,
                        )? {
                            declarations.push(OwnedDeclaration::new(path, snippet.clone())
                            .with_snippet(export_function_as(snippet, name, path)));
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
                            declarations.push(OwnedDeclaration::new(path, source_map.span_to_snippet(function.span()).map_err(
                                |error| format!("failed to read declaration for `{resolved}`: {error:?}"),
                            )?));
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
                            declarations.push(OwnedDeclaration::new(path, source_map.span_to_snippet(var_decl.span()).map_err(
                                |error| format!("failed to read declaration for `{resolved}`: {error:?}"),
                            )?));
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) => {
                    if let thaw_parser::ast::DefaultDecl::Fn(fn_expr) = &default_decl.decl {
                        let body = source_map.span_to_snippet(fn_expr.function.span()).map_err(
                            |error| format!("failed to read default function declaration: {error:?}"),
                        )?;
                        declarations.push(OwnedDeclaration::new(path, reexported_default_declaration(
                            &body,
                            "function",
                            path,
                            fn_expr.ident.as_ref().map(|ident| ident.sym.as_ref()),
                            fn_expr.function.is_async,
                            false,
                        )?));
                    }
                }
                _ => {}
            }
        }
        if !declarations.is_empty() {
            return Ok(declarations);
        }
    }
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let Some(source) = export.src.as_ref().and_then(|source| source.value.as_str())
        else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(path, source) else {
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
            let original = export_name(&named.orig);
            let exported = named.exported.as_ref().and_then(export_name).or_else(|| original.clone());
            if exported.as_deref() == Some(name) {
                let snippets = reexported_function_declarations_owned(
                    &target_path,
                    original.as_deref().unwrap_or(name),
                    visited,
                )?;
                if !snippets.is_empty() {
                    return Ok(snippets.into_iter().map(|snippet| {
                        let text = export_function_as(snippet.snippet.clone(), name, path);
                        snippet.with_snippet(text)
                    }).collect());
                }
            }
        }
    }
    if name != "default" {
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) = item else { continue; };
            if export.type_only { continue; }
            let Some(target) = export.src.value.as_str()
                .and_then(|source| declaration_reexport_path(path, source)) else { continue; };
            let snippets = reexported_function_declarations_owned(&target, name, visited)?;
            if !snippets.is_empty() { return Ok(snippets); }
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
fn export_assignment_function_declarations_owned(path: &Path) -> Result<Vec<OwnedDeclaration>, String> {
    Ok(export_assignment_function_declarations(path)?
        .into_iter().map(|snippet| OwnedDeclaration::new(path, snippet)).collect())
}

fn export_assignment_function_declarations(path: &Path) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
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

/// The `export_assignment_function_declarations`'s sibling for a plain
/// (non-exported) `declare class X {...}`/`declare interface X {...}`
/// matching whatever identifier `path`'s own `export = X;` names --
/// real-world example: `@types/semver`'s `classes/semver.d.ts`
/// (`import semver = require("../index"); declare class SemVer {...}
/// export = SemVer;`), reached the identical way (`import SemVer =
/// require("./classes/semver")` in the barrel `index.d.ts`, then a
/// plain local `export { SemVer };`) `export_assignment_function_
/// declarations` already handles for a *function*-shaped target. Before
/// this fix, `SemVer`/`Range`/`Comparator` -- semver's three real
/// classes -- were silently dropped entirely (`export_assignment_
/// function_declarations` returns empty for a class-shaped target,
/// with nothing to try next), so `import { SemVer } from "semver"`
/// failed outright: `` `semver` has no export named `SemVer` ``.
fn export_assignment_class_or_interface_declarations_owned(path: &Path) -> Result<Vec<OwnedDeclaration>, String> {
    Ok(export_assignment_class_or_interface_declarations(path)?
        .into_iter().map(|snippet| OwnedDeclaration::new(path, snippet)).collect())
}

fn export_assignment_class_or_interface_declarations(path: &Path) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
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
            let ModuleItem::Stmt(Stmt::Decl(declaration)) = item else {
                return None;
            };
            let (name, span) = match declaration {
                Decl::Class(class) => (class.ident.sym.as_ref(), class.span()),
                Decl::TsInterface(interface) => (interface.id.sym.as_ref(), interface.span()),
                _ => return None,
            };
            (name == target).then(|| {
                source_map
                    .span_to_snippet(span)
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
                // `import * as NAME from "path"` -- a namespace import,
                // real example: winston's own `.d.ts`, `import * as
                // logform from 'logform'; declare namespace winston {
                // export import format = logform.format; ... }`. Every
                // *caller* of `named_import_targets` reaches this entry
                // only through a qualified access (`logform.format`,
                // `TsImportEquals`'s own `TsQualifiedName` handling
                // below), never a bare reference to `NAME` itself, so
                // the second field (which real member within `path` a
                // bare reference to `NAME` alone would mean) is never
                // actually read for this shape -- unlike the `Named`/
                // `Default` cases above, which record it because a bare
                // reference *is* how those get consumed.
                ImportSpecifier::Namespace(namespace) => {
                    targets.insert(
                        namespace.local.sym.to_string(),
                        (target_path.clone(), String::new()),
                    );
                }
                ImportSpecifier::Named(_) => {}
            }
        }
    }
    targets
}

/// Type-only imports feed local `export type { Local }` declarations but
/// must never enter the value import resolver above.
fn named_type_import_targets(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> std::collections::HashMap<String, (PathBuf, String)> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    let mut targets = std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else { continue };
        let Some(source) = import.src.value.as_str() else { continue };
        let Some(path) = declaration_reexport_path(entry_path, source) else { continue };
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Named(named) if import.type_only || named.is_type_only => {
                    let original = match &named.imported {
                        Some(ModuleExportName::Ident(original)) => original.sym.to_string(),
                        Some(ModuleExportName::Str(_)) => continue,
                        None => named.local.sym.to_string(),
                    };
                    targets.insert(named.local.sym.to_string(), (path.clone(), original));
                }
                ImportSpecifier::Default(default) if import.type_only => {
                    targets.insert(default.local.sym.to_string(), (path.clone(), "default".to_string()));
                }
                _ => {}
            }
        }
    }
    targets
}

// `export { Local }` can re-export an `import type` without repeating the
// `type` keyword. Track the declaration's provenance through named and
// wildcard barrels; a value edge wins when both kinds expose one name.
fn reexport_is_type_only(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Option<bool>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat, Stmt};
    if !visited.insert((path.to_path_buf(), name.to_string())) { return Ok(None); }
    let source = fs::read_to_string(path).map_err(|error|
        format!("failed to read re-exported declarations `{}`: {error}", path.display()))?;
    let module = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?.0;
    let value_imports = named_import_targets(path, &module);
    let type_imports = named_type_import_targets(path, &module);
    let decl_kind = |declaration: &Decl, local: &str| match declaration {
        Decl::Class(class) if class.ident.sym == local => Some(false),
        Decl::Fn(function) if function.ident.sym == local => Some(false),
        Decl::TsInterface(interface) if interface.id.sym == local => Some(true),
        Decl::TsTypeAlias(alias) if alias.id.sym == local => Some(true),
        Decl::TsEnum(enumeration) if enumeration.id.sym == local => Some(false),
        Decl::TsModule(namespace) if matches!(&namespace.id,
            thaw_parser::ast::TsModuleName::Ident(ident) if ident.sym == local) => Some(false),
        Decl::Var(variables) if variables.decls.iter().any(|declarator|
            matches!(&declarator.name, Pat::Ident(binding) if binding.id.sym == local)) => Some(false),
        _ => None,
    };
    let local_kind = |local: &str| module.body.iter().filter_map(|item| match item {
        ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
        _ => None,
    }).filter_map(|declaration| decl_kind(declaration, local))
        .fold(None, |status, candidate| merge_type_only_status(status, Some(candidate)));
    let mut status = None;
    for item in &module.body {
        let candidate = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                decl_kind(&export.decl, name)
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                let mut named_status = None;
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else { continue };
                    let ModuleExportName::Ident(original) = &named.orig else { continue };
                    let public = named.exported.as_ref().unwrap_or(&named.orig);
                    let ModuleExportName::Ident(public) = public else { continue };
                    if public.sym != name { continue; }
                    let kind = if export.type_only || named.is_type_only { Some(true) }
                    else if let Some(target) = export.src.as_ref()
                        .and_then(|source| source.value.as_str())
                        .and_then(|source| declaration_reexport_path(path, source)) {
                        reexport_is_type_only(&target, original.sym.as_ref(), visited)?
                    } else if value_imports.contains_key(original.sym.as_ref()) {
                        let (target, imported) = &value_imports[original.sym.as_ref()];
                        reexport_is_type_only(target, imported, visited)?.or(Some(false))
                    } else if type_imports.contains_key(original.sym.as_ref()) {
                        Some(true)
                    } else {
                        local_kind(original.sym.as_ref())
                    };
                    named_status = merge_type_only_status(named_status, kind);
                }
                named_status
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if name != "default" => {
                let target = export.src.value.as_str()
                    .and_then(|source| declaration_reexport_path(path, source));
                if let Some(target) = target {
                    reexport_is_type_only(&target, name, visited)?
                        .map(|inherited| export.type_only || inherited)
                } else { None }
            }
            _ => None,
        };
        status = merge_type_only_status(status, candidate);
    }
    visited.remove(&(path.to_path_buf(), name.to_string()));
    Ok(status)
}

fn merge_type_only_status(current: Option<bool>, next: Option<bool>) -> Option<bool> {
    match (current, next) {
        (Some(left), Some(right)) => Some(left && right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

/// Follow a class or interface binding through named and wildcard declaration
/// re-exports, retaining imported superclass declarations as type support.
fn reexported_class_or_interface_declarations(
    path: &Path,
    name: &str,
) -> Result<Vec<String>, String> {
    Ok(reexported_class_or_interface_declarations_owned(path, name)?
        .into_iter().map(|declaration| declaration.snippet).collect())
}

fn reexported_class_or_interface_declarations_owned(
    path: &Path,
    name: &str,
) -> Result<Vec<OwnedDeclaration>, String> {
    let mut visited = std::collections::BTreeSet::new();
    reexported_class_or_interface_declarations_inner(path, name, &mut visited)
}

fn imported_owned_class_as_local_binding(
    declarations: Vec<OwnedDeclaration>, local: &str, type_only: bool, origin: &Path,
) -> Vec<OwnedDeclaration> {
    let rewritten = imported_class_as_local_binding(
        declarations.iter().map(|declaration| declaration.snippet.clone()).collect(),
        local, type_only, origin,
    );
    declarations.into_iter().zip(rewritten).map(|(declaration, snippet)|
        declaration.with_snippet(snippet)).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum TypeReferenceKind {
    Type,
    ValueQuery,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct TypeReferenceSite {
    root: (usize, usize),
    full: (usize, usize),
    segments: Vec<String>,
    kind: TypeReferenceKind,
}

/// Source-relative spans of unshadowed type-position references to one binding.
/// Shared with imported-class renaming and source-owned support closure.
fn type_reference_sites(snippet: &str, original: &str, origin: &Path) -> Option<Vec<TypeReferenceSite>> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        Class, Expr, Function, Ident, MemberProp, ObjectPatProp, Pat, TsCallSignatureDecl, TsConditionalType, TsConstructorType,
        TsConstructSignatureDecl, TsEntityName, TsFnType, TsInferType, TsInterfaceDecl,
        TsExprWithTypeArgs, TsFnParam, TsMappedType, TsMethodSignature,
        TsTypeAliasDecl, TsTypeParamDecl, TsTypeQuery, TsTypeQueryExpr,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    struct SelfReferences<'a> {
        original: &'a str,
        spans: Vec<(thaw_parser::common::Span, thaw_parser::common::Span, Vec<String>, TypeReferenceKind)>,
        type_shadows: Vec<thaw_parser::common::Span>,
        value_shadows: Vec<thaw_parser::common::Span>,
        value_query_names: Vec<thaw_parser::common::Span>,
    }
    impl SelfReferences<'_> {
        fn pat_binds_name(pat: &Pat, name: &str) -> bool {
            match pat {
                Pat::Ident(binding) => binding.id.sym == name,
                Pat::Array(array) => array.elems.iter().flatten()
                    .any(|element| Self::pat_binds_name(element, name)),
                Pat::Object(object) => object.props.iter().any(|property| match property {
                    ObjectPatProp::KeyValue(property) => Self::pat_binds_name(&property.value, name),
                    ObjectPatProp::Assign(property) => property.key.id.sym == name,
                    ObjectPatProp::Rest(property) => Self::pat_binds_name(&property.arg, name),
                }),
                Pat::Rest(rest) => Self::pat_binds_name(&rest.arg, name),
                Pat::Assign(assign) => Self::pat_binds_name(&assign.left, name),
                Pat::Invalid(_) | Pat::Expr(_) => false,
            }
        }
        fn ts_param_binds_name(param: &TsFnParam, name: &str) -> bool {
            match param {
                TsFnParam::Ident(binding) => binding.id.sym == name,
                TsFnParam::Array(array) => Self::pat_binds_name(&Pat::Array(array.clone()), name),
                TsFnParam::Rest(rest) => Self::pat_binds_name(&Pat::Rest(rest.clone()), name),
                TsFnParam::Object(object) => Self::pat_binds_name(&Pat::Object(object.clone()), name),
            }
        }
        fn shadow_value_params(&mut self, span: thaw_parser::common::Span, params: &[TsFnParam]) {
            if params.iter().any(|param| Self::ts_param_binds_name(param, self.original)) {
                self.value_shadows.push(span);
            }
        }
        fn reference(&mut self, ident: &Ident, full: thaw_parser::common::Span, segments: Vec<String>) {
            let kind = if self.value_query_names.contains(&full) {
                TypeReferenceKind::ValueQuery
            } else {
                TypeReferenceKind::Type
            };
            let shadows = match kind {
                TypeReferenceKind::Type => &self.type_shadows,
                TypeReferenceKind::ValueQuery => &self.value_shadows,
            };
            if ident.sym == self.original && !shadows.iter().any(|scope| {
                scope.lo <= ident.span.lo && ident.span.hi <= scope.hi
            }) {
                self.spans.push((ident.span(), full, segments, kind));
            }
        }
        fn reference_expr(&mut self, expr: &Expr) {
            let mut root = expr;
            let mut names = Vec::new();
            while let Expr::Member(member) = root {
                let MemberProp::Ident(property) = &member.prop else { return };
                names.push(property.sym.to_string());
                root = member.obj.as_ref();
            }
            if let Expr::Ident(ident) = root {
                names.push(ident.sym.to_string());
                names.reverse();
                self.reference(ident, expr.span(), names);
            }
        }
        fn shadow(&mut self, span: thaw_parser::common::Span, params: Option<&TsTypeParamDecl>) {
            if params.is_some_and(|params| params.params.iter().any(|param| param.name.sym == self.original)) {
                self.type_shadows.push(span);
            }
        }
    }
    impl Visit for SelfReferences<'_> {
        fn visit_ts_type_query(&mut self, query: &TsTypeQuery) {
            if let TsTypeQueryExpr::TsEntityName(name) = &query.expr_name {
                self.value_query_names.push(name.span());
            }
            query.visit_children_with(self);
        }
        fn visit_class(&mut self, class: &Class) {
            self.shadow(class.span, class.type_params.as_deref());
            if let Some(base) = class.super_class.as_deref() { self.reference_expr(base); }
            class.visit_children_with(self);
        }
        fn visit_ts_expr_with_type_args(&mut self, base: &TsExprWithTypeArgs) {
            self.reference_expr(base.expr.as_ref());
            base.visit_children_with(self);
        }
        fn visit_function(&mut self, function: &Function) {
            self.shadow(function.span, function.type_params.as_deref());
            if function.params.iter().any(|param|
                SelfReferences::pat_binds_name(&param.pat, self.original)) {
                self.value_shadows.push(function.span);
            }
            function.visit_children_with(self);
        }
        fn visit_ts_interface_decl(&mut self, interface: &TsInterfaceDecl) {
            self.shadow(interface.span, interface.type_params.as_deref());
            interface.visit_children_with(self);
        }
        fn visit_ts_type_alias_decl(&mut self, alias: &TsTypeAliasDecl) {
            self.shadow(alias.span, alias.type_params.as_deref());
            alias.visit_children_with(self);
        }
        fn visit_ts_method_signature(&mut self, method: &TsMethodSignature) {
            self.shadow(method.span, method.type_params.as_deref());
            self.shadow_value_params(method.span, &method.params);
            method.visit_children_with(self);
        }
        fn visit_ts_call_signature_decl(&mut self, call: &TsCallSignatureDecl) {
            self.shadow(call.span, call.type_params.as_deref());
            self.shadow_value_params(call.span, &call.params);
            call.visit_children_with(self);
        }
        fn visit_ts_construct_signature_decl(&mut self, construct: &TsConstructSignatureDecl) {
            self.shadow(construct.span, construct.type_params.as_deref());
            self.shadow_value_params(construct.span, &construct.params);
            construct.visit_children_with(self);
        }
        fn visit_ts_fn_type(&mut self, function: &TsFnType) {
            self.shadow(function.span, function.type_params.as_deref());
            self.shadow_value_params(function.span, &function.params);
            function.visit_children_with(self);
        }
        fn visit_ts_constructor_type(&mut self, constructor: &TsConstructorType) {
            self.shadow(constructor.span, constructor.type_params.as_deref());
            self.shadow_value_params(constructor.span, &constructor.params);
            constructor.visit_children_with(self);
        }
        fn visit_ts_mapped_type(&mut self, mapped: &TsMappedType) {
            if mapped.type_param.name.sym == self.original {
                // The constraint is evaluated before the mapped key is
                // bound; only the key remapping and value body are shadowed.
                if let Some(name_type) = &mapped.name_type { self.type_shadows.push(name_type.span()); }
                if let Some(type_ann) = &mapped.type_ann { self.type_shadows.push(type_ann.span()); }
            }
            mapped.visit_children_with(self);
        }
        fn visit_ts_conditional_type(&mut self, conditional: &TsConditionalType) {
            struct InferNames<'a>(&'a str, bool);
            impl Visit for InferNames<'_> {
                fn visit_ts_infer_type(&mut self, infer: &TsInferType) {
                    if infer.type_param.name.sym == self.0 { self.1 = true; }
                }
                fn visit_ts_conditional_type(&mut self, _: &TsConditionalType) {
                    // A nested conditional's infer binding stays nested.
                }
            }
            let mut inferred = InferNames(self.original, false);
            conditional.extends_type.visit_with(&mut inferred);
            if inferred.1 { self.type_shadows.push(conditional.true_type.span()); }
            conditional.visit_children_with(self);
        }
        fn visit_ts_entity_name(&mut self, name: &TsEntityName) {
            let mut names = Vec::new();
            let mut left = name;
            while let TsEntityName::TsQualifiedName(qualified) = left {
                names.push(qualified.right.sym.to_string());
                left = qualified.left.as_ref();
            }
            let TsEntityName::Ident(root) = left else { return };
            names.push(root.sym.to_string());
            names.reverse();
            let full = match name {
                TsEntityName::Ident(ident) => ident.span(),
                TsEntityName::TsQualifiedName(qualified) => qualified.span(),
            };
            self.reference(root, full, names);
        }
    }

    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        snippet, thaw_parser::common::FileName::Custom(
            format!("{} (type reference snippet)", origin.display()).into(),
        ),
    ).ok()?;
    let mut references = SelfReferences { original, spans: Vec::new(),
        type_shadows: Vec::new(), value_shadows: Vec::new(), value_query_names: Vec::new() };
    module.visit_with(&mut references);
    let mut ranges = references.spans.into_iter().filter_map(|(root, full, segments, kind)| {
        let start = source_map.lookup_byte_offset(root.lo).pos.0 as usize;
        let end = source_map.lookup_byte_offset(root.hi).pos.0 as usize;
        let full_start = source_map.lookup_byte_offset(full.lo).pos.0 as usize;
        let full_end = source_map.lookup_byte_offset(full.hi).pos.0 as usize;
        (snippet.get(start..end) == Some(original) && full_start == start
            && snippet.get(full_start..full_end).is_some())
            .then_some(TypeReferenceSite { root: (start, end), full: (full_start, full_end), segments, kind })
    }).collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.dedup();
    Some(ranges)
}

fn type_reference_ranges(snippet: &str, original: &str, origin: &Path) -> Option<Vec<(usize, usize)>> {
    Some(type_reference_sites(snippet, original, origin)?.into_iter().map(|site| site.root).collect())
}

/// An imported class is flattened without its import statement. Give its
/// declaration and merged namespace the local binding used by `extends`,
/// while leaving supporting ancestor declarations under their own names.
/// AST spans limit self-type changes to references, never method/property
/// names or coincidental text in comments.
fn imported_class_as_local_binding(declarations: Vec<String>, local: &str, type_only: bool, origin: &Path) -> Vec<String> {
    let selected = declarations.first().and_then(|snippet| declaration_identity(snippet));
    let Some(selected) = selected else {
        return declarations.into_iter().map(|snippet| {
            if type_only { type_only_declaration(snippet, None) } else { snippet }
        }).collect()
    };
    declarations.into_iter().map(|snippet| {
        let is_selected = declaration_identity(&snippet).as_deref() == Some(selected.as_str());
        if !is_selected || selected == local {
            return if type_only { type_only_declaration(snippet, None) } else { snippet };
        }
        let Some(mut edits) = type_reference_ranges(&snippet, &selected, origin) else {
            return if type_only { type_only_declaration(snippet, None) } else { snippet }
        };
        let Some((start, end)) = declared_function_name_range(&snippet) else {
            return if type_only { type_only_declaration(snippet, None) } else { snippet }
        };
        edits.push((start, end));
        edits.sort_unstable();
        edits.dedup();
        let mut renamed = snippet;
        for (start, end) in edits.into_iter().rev() {
            renamed.replace_range(start..end, local);
        }
        if type_only {
            // A transitive import may already carry the source binding's
            // type marker. Its old name is gone after this local rename.
            renamed = renamed.replace(&format!("\nexport type {{ {selected} }};"), "");
            type_only_declaration(renamed, None)
        } else {
            renamed
        }
    }).collect()
}

/// A materialized local class replaces its declaration import binding.
/// Retain other specifiers from the same import, including their original
/// `type` modifiers and any import attributes after the source literal.
fn without_materialized_class_imports(source: &str, locals: &std::collections::BTreeSet<String>) -> Result<String, String> {
    without_materialized_class_imports_named(
        source, locals, thaw_parser::common::FileName::Custom("input.ts".into()),
    )
}

fn without_materialized_class_imports_named(
    source: &str, locals: &std::collections::BTreeSet<String>,
    source_name: thaw_parser::common::FileName,
) -> Result<String, String> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if locals.is_empty() { return Ok(source.to_string()) }
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        source, source_name,
    )?;
    let offset = |position| source_map.lookup_byte_offset(position).pos.0 as usize;
    let mut edits = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else { continue };
        let kept = import.specifiers.iter().filter(|specifier| {
            let local = match specifier {
                ImportSpecifier::Named(named) => named.local.sym.as_ref(),
                ImportSpecifier::Default(default) => default.local.sym.as_ref(),
                ImportSpecifier::Namespace(namespace) => namespace.local.sym.as_ref(),
            };
            !locals.contains(local)
        }).collect::<Vec<_>>();
        if kept.len() == import.specifiers.len() { continue }
        let start = offset(import.span.lo);
        let end = offset(import.span.hi);
        let replacement = if kept.is_empty() {
            String::new()
        } else {
            let mut default = None;
            let mut namespace = None;
            let mut named = Vec::new();
            for specifier in kept {
                let snippet = source_map.span_to_snippet(specifier.span())
                    .map_err(|error| format!("failed to retain declaration import: {error:?}"))?;
                match specifier {
                    ImportSpecifier::Default(_) => default = Some(snippet),
                    ImportSpecifier::Namespace(_) => namespace = Some(snippet),
                    ImportSpecifier::Named(_) => named.push(snippet),
                }
            }
            let mut clauses = Vec::new();
            if let Some(default) = default { clauses.push(default) }
            if let Some(namespace) = namespace { clauses.push(namespace) }
            if !named.is_empty() { clauses.push(format!("{{ {} }}", named.join(", "))) }
            let source_end = offset(import.src.span.hi);
            let literal = source_map.span_to_snippet(import.src.span())
                .map_err(|error| format!("failed to retain declaration import source: {error:?}"))?;
            let keyword = if import.type_only { "import type" } else { "import" };
            format!("{keyword} {} from {}{}", clauses.join(", "), literal, &source[source_end..end])
        };
        edits.push((start, end, replacement));
    }
    let mut result = source.to_string();
    for (start, end, replacement) in edits.into_iter().rev() {
        result.replace_range(start..end, &replacement);
    }
    Ok(result)
}

fn reexported_class_or_interface_declarations_inner(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<OwnedDeclaration>, String> {
    use thaw_parser::ast::{
        Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Stmt, TsModuleName,
    };
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
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        &source, thaw_parser::common::FileName::Real(path.to_path_buf()),
    )?;
    let mut local_name = name.to_string();
    if name == "default" {
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) = item else {
                continue;
            };
            if let thaw_parser::ast::Expr::Ident(identifier) = default_expr.expr.as_ref() {
                local_name = identifier.sym.to_string();
            }
        }
    }
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
    if name == "default" && local_name == "default" {
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) = item else {
                continue;
            };
            let thaw_parser::ast::DefaultDecl::Class(class_expr) = &default_decl.decl else {
                continue;
            };
            superclass = class_expr.class.super_class.as_deref().and_then(|expr| match expr {
                thaw_parser::ast::Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            });
            let body = source_map.span_to_snippet(class_expr.class.span()).map_err(|error| {
                format!("failed to read default class declaration in `{}`: {error:?}", path.display())
            })?;
            declarations.push(OwnedDeclaration::new(path, reexported_default_declaration(
                &body,
                "class",
                path,
                class_expr.ident.as_ref().map(|ident| ident.sym.as_ref()),
                false,
                class_expr.class.is_abstract,
            )?));
        }
        if let Some(superclass) = superclass {
            if let Some((target_path, target_name)) =
                named_import_targets(path, &module).get(&superclass)
            {
                declarations.extend(imported_owned_class_as_local_binding(
                    reexported_class_or_interface_declarations_inner(
                        target_path, target_name, visited,
                    )?, &superclass, true, target_path,
                ));
            } else {
                declarations.extend(reexported_class_or_interface_declarations_inner(
                    path, &superclass, visited,
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
            Decl::TsTypeAlias(alias) => alias.id.sym == local_name,
            Decl::TsEnum(enumeration) => enumeration.id.sym == local_name,
            // A class/interface can be merged with a same-named
            // `declare namespace X { ... }` block (a common real-world
            // pattern for attaching static types alongside a class, e.g.
            // `minipass`'s own `export declare namespace Minipass {
            // export type Events<...> = ...; ... }` merged with `export
            // declare class Minipass<...> { ... }`). A dependent
            // package's own `.d.ts` referencing `Minipass.Events<...>`
            // (`@isaacs/fs-minipass`'s real `ReadStreamEvents`) needs
            // this namespace half inlined too, not just the class --
            // without it, `Minipass.Events`/`.Options`/`.ContiguousData`
            // etc. are all unresolvable, and the generic `.on(event,
            // handler)` overload that indexes into `Events[Event]` widens
            // its handler parameter list to nothing, rejecting any real
            // callback with an argument at all.
            Decl::TsModule(module) => match &module.id {
                TsModuleName::Ident(id) => id.sym.as_ref() == local_name,
                TsModuleName::Str(_) => false,
            },
            // A non-callable `declare const X: T;` value (real example:
            // graphql's `export declare const GraphQLString:
            // GraphQLScalarType;`, reached through two barrel re-export
            // hops). Not a class or interface, but this function is the
            // only follower that inlines a named declaration from another
            // file for the value/const machinery (`parse_dts_values`) to
            // then pick up; without it `GraphQLString`/`GraphQLInt`/... were
            // missing from the flattened `package.d.ts` entirely.
            Decl::Var(var_decl) => var_decl.decls.iter().any(|declarator| {
                matches!(
                    &declarator.name,
                    thaw_parser::ast::Pat::Ident(binding) if binding.id.sym == local_name
                )
            }),
            _ => false,
        };
        if matches {
            let mut snippet = if let Decl::Var(var_decl) = declaration {
                let declarator = var_decl.decls.iter().find(|declarator| {
                    matches!(&declarator.name, thaw_parser::ast::Pat::Ident(binding)
                        if binding.id.sym == local_name)
                }).expect("matched variable declarator");
                selected_var_declaration_snippet(
                    &source_map, var_decl, declarator,
                    matches!(item, ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(_))),
                )?
            } else {
                source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| format!("failed to read declaration for `{name}`: {error:?}"))?
            };
            if name != "default" && local_name != name {
                snippet = imported_class_as_local_binding(vec![snippet], name, false, path)
                    .into_iter().next().unwrap();
            }
            if !snippet.trim_start().starts_with("export ") {
                snippet = format!("export {snippet}");
            }
            let mut owned = OwnedDeclaration::new(path, snippet);
            owned.local_name = Some(local_name.clone());
            declarations.push(owned);
        }
    }
    if let Some(superclass) = superclass {
        if let Some((target_path, target_name)) = named_import_targets(path, &module).get(&superclass)
        {
            declarations.extend(imported_owned_class_as_local_binding(
                reexported_class_or_interface_declarations_inner(
                    target_path, target_name, visited,
                )?, &superclass, true, target_path,
            ));
        } else {
            // `Base` may be declared beside the selected class rather than
            // imported. Follow that local binding through the same visited
            // guard so its methods and further ancestors remain available.
            declarations.extend(reexported_class_or_interface_declarations_inner(
                path, &superclass, visited,
            )?);
        }
    }
    if declarations.is_empty() {
        let imports = named_import_targets(path, &module);
        let type_imports = named_type_import_targets(path, &module);
        if let Some((target, original)) = imports.get(&local_name)
            .or_else(|| type_imports.get(&local_name)) {
            return reexported_class_or_interface_declarations_inner(
                target, original, visited);
        }
        // The name isn't declared in this file at all -- follow a *barrel*
        // re-export (`export { X } from "./y.js"`) into the target file,
        // exactly the way `reexported_function_declarations` already does
        // for functions/callable consts. Real-world example: graphql's
        // `type/index.d.ts`, which has *no* declarations of its own and is
        // entirely `export { ... } from "./definition.js"`-style lines, so
        // `GraphQLObjectType`/`GraphQLSchema` (and every other class it
        // re-exports) were silently missing from the flattened
        // `package.d.ts` (``graphql` has no export named `GraphQLObjectType`).
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
                continue;
            };
            let Some(source) = export.src.as_ref().and_then(|source| source.value.as_str())
            else {
                continue;
            };
            let Some(target_path) = declaration_reexport_path(path, source) else {
                continue;
            };
            for specifier in &export.specifiers {
                let ExportSpecifier::Named(named) = specifier else {
                    continue;
                };
                let export_name = |candidate: &ModuleExportName| match candidate {
                    ModuleExportName::Ident(ident) => Some(ident.sym.to_string()),
                    ModuleExportName::Str(_) => None,
                };
                let original = export_name(&named.orig);
                let exported = named
                    .exported
                    .as_ref()
                    .and_then(export_name)
                    .or_else(|| original.clone());
                if exported.as_deref() == Some(local_name.as_str()) {
                    let snippets = reexported_class_or_interface_declarations_inner(
                        &target_path,
                        original.as_deref().unwrap_or(name),
                        visited,
                    )?;
                    if !snippets.is_empty() {
                        return Ok(snippets);
                    }
                }
            }
        }
        // A named export can point at a barrel whose only declaration edge
        // is `export *`. Keep the selected declaration and its supporting
        // superclass snippets under their original names on this hop.
        if name != "default" {
            for item in &module.body {
                let ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) = item else {
                    continue;
                };
                let Some(target) = export.src.value.as_str()
                    .and_then(|source| declaration_reexport_path(path, source)) else {
                    continue;
                };
                let snippets = reexported_class_or_interface_declarations_inner(
                    &target, &local_name, visited,
                )?;
                if !snippets.is_empty() {
                    return Ok(snippets);
                }
            }
        }
    }
    Ok(declarations)
}

/// Builds `path`'s own `.d.ts` sibling -- unlike a blanket `Path::with_
/// extension("d.ts")`, which always *replaces* whatever Rust considers
/// the current "extension" (everything after the final `.` in the file
/// name). That's exactly right when `path`'s own last segment already
/// names a real source-file extension (`import * as ns from
/// "./impl.js"` -> `impl.d.ts`, an ordinary, common re-export shape),
/// but wrong when a real npm/`.d.ts` file name instead embeds a dot as
/// part of its own name, not an extension (`expose.decorator.d.ts`,
/// `expose-options.interface.d.ts`, `transformation-type.enum.d.ts`,
/// ... all real files in `@types/semver`/`@types/class-transformer`'s
/// own layout): `Path::new("expose.decorator").with_extension("d.ts")`
/// then silently produces `expose.d.ts` (wrong, usually nonexistent)
/// instead of `expose.decorator.d.ts`, and the whole reexport chain
/// resolves to nothing. Distinguished by checking whether the current
/// extension is one of the source-file kinds a `.d.ts` sibling would
/// ever actually replace; anything else is treated as part of the name
/// and appended after, not replaced.
fn with_d_ts_suffix(path: &Path) -> Option<PathBuf> {
    const SOURCE_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        if SOURCE_EXTENSIONS.contains(&extension) {
            return Some(path.with_extension("d.ts"));
        }
    }
    let file_name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!("{file_name}.d.ts")))
}

/// `with_d_ts_suffix` for the ESM/CJS-explicit extensions: `.mjs`/`.mts`
/// declarations are `.d.mts`, `.cjs`/`.cts` are `.d.cts`. Deliberately
/// separate from `with_d_ts_suffix` (which maps every source extension to
/// `.d.ts`) so a re-export hop can try the precise extension first while
/// retaining the ordinary `.d.ts` fallback. Package-entry selection still
/// uses its manifest; it must not load both ESM and CJS entry declarations.
fn with_explicit_d_ts_suffix(path: &Path) -> Option<PathBuf> {
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    let declaration_extension = match extension {
        "mjs" | "mts" => "d.mts",
        "cjs" | "cts" => "d.cts",
        "js" | "jsx" | "ts" | "tsx" => "d.ts",
        _ => return None,
    };
    Some(path.with_extension(declaration_extension))
}

fn declaration_file_candidate(path: &Path) -> Option<PathBuf> {
    let direct_declaration = path.file_name().and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts"))
        .then(|| path.to_path_buf());
    [direct_declaration, with_explicit_d_ts_suffix(path), with_d_ts_suffix(path), Some(path.join("index.d.ts"))]
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_file())
}

fn declaration_reexport_path(entry_path: &Path, source: &str) -> Option<PathBuf> {
    if source == "." || source.starts_with("./") || source.starts_with("../") {
        let path = entry_path.parent()?.join(source);
        let direct_declaration = path.file_name().and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts"))
            .then(|| path.clone());
        let file = [direct_declaration, with_explicit_d_ts_suffix(&path), with_d_ts_suffix(&path)]
            .into_iter()
            .flatten()
            .find(|candidate| candidate.is_file());
        let manifest_entry = || read_manifest(&path).ok()
            .and_then(|manifest| find_own_dts(&manifest, &path))
            .map(|(_, absolute)| absolute)
            .filter(|candidate| candidate.is_file());
        return file.or_else(manifest_entry)
            .or_else(|| { let index = path.join("index.d.ts"); index.is_file().then_some(index) })
            // Canonicalize so a re-export cycle is seen as the *same*
            // `PathBuf` on every hop. Without this, each `./x.js` hop
            // appends another `./` segment (`a/./b.d.ts` ->
            // `a/././b.d.ts` -> ...), so a path-keyed `visited` guard
            // never matches and `local_type_declaration_snippets` recurses
            // forever (real trigger: drizzle-orm's `supabase/index.d.ts`
            // re-export cycle).
            .map(|candidate| candidate.canonicalize().unwrap_or(candidate));
    }
    // Bare declaration re-exports use the same package-name split and
    // nearest dependency lookup as runtime imports. A package's root types
    // entry still uses `find_own_dts`; a subpath must honor its exports
    // map before any physical-file fallback.
    let node_modules = entry_path
        .ancestors()
        .filter(|path| path.file_name().is_some_and(|name| name == "node_modules"))
        .last()?;
    let (package_name, subpath) = split_bare_spec(source);
    if let Some(subpath) = subpath {
        validate_export_subpath(subpath).ok()?;
    }
    let requiring_dir = entry_path.parent()?;
    let project_root = node_modules.parent().unwrap_or(node_modules);
    let mut package_dir = None;
    for ancestor in requiring_dir.ancestors() {
        let candidate = ancestor.join("node_modules").join(package_name);
        let requested = subpath.map_or_else(|| candidate.clone(), |subpath| candidate.join(subpath));
        if read_manifest(&candidate).is_ok()
            || declaration_file_candidate(&candidate).is_some()
            || declaration_file_candidate(&requested).is_some()
        {
            package_dir = Some(candidate);
            break;
        }
        if ancestor == project_root {
            break;
        }
    }
    let package_dir = package_dir.unwrap_or_else(||
        resolve_dependency_dir(node_modules, requiring_dir, package_name));
    if let Ok(manifest) = read_manifest(&package_dir) {
        if let Some(subpath) = subpath {
            if manifest.get("exports").is_some() {
                // Once the nearest package declares exports, an absent or
                // blocked subpath cannot fall through to a physical file or
                // a different version in an ancestor node_modules.
                let target = package_subpath_runtime_target(&manifest, subpath, &["types"])?;
                let relative = target.strip_prefix("./")?;
                validate_export_subpath(relative).ok()?;
                if relative.split('/').any(|segment| segment == "node_modules") {
                    return None;
                }
                let absolute = package_dir.join(relative);
                return absolute.is_file().then(|| absolute.canonicalize().unwrap_or(absolute));
            }
        } else if let Some((_, absolute)) = find_own_dts(&manifest, &package_dir) {
            if absolute.is_file() {
                return Some(absolute.canonicalize().unwrap_or(absolute));
            }
        }
    }
    let path = subpath.map_or_else(|| package_dir.clone(), |subpath| package_dir.join(subpath));
    declaration_file_candidate(&path)
        .map(|candidate| candidate.canonicalize().unwrap_or(candidate))
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
) -> Result<Option<(PathBuf, String)>, String> {
    let Some(type_name) = export_assignment_value_type_name(module) else {
        return Ok(None);
    };
    let Some(target_path) = import_equals_targets.get(&type_name) else {
        return Ok(None);
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
    Ok(Some((target_path.clone(), format!("\n{source}\n"))))
}

/// Inlines the file backing *any* `import Name = require("./path")` (see
/// `import_equals_targets`) whose bound `Name` is referenced as a bare
/// type name anywhere else in this same entry file -- not just the one
/// `declare const x: Name; export = x;` shape `inline_import_equals_
/// value_type` above already covers. Real-world example: nodemailer's
/// own `import Mail = require("./lib/mailer"); ... export type
/// Transporter<T = any, D extends TransportOptions = TransportOptions> =
/// Mail<T, D>;` -- `Mail`'s entire class (with `sendMail`, the actual
/// reason anyone imports this package) lives only in that sibling file,
/// reachable *only* through a generic type alias, never via a `declare
/// const`/`export {}` re-export any existing mechanism here follows.
/// Without this, every call to a method on a value whose declared type
/// traces back through `Transporter` failed to build ("call to unknown
/// function `transporter.sendMail`") regardless of whether the method
/// itself was ever the problem.
///
/// `already_inlined` skips a name `inline_import_equals_value_type`
/// already spliced in (its own single-name shape is a special case of
/// this one, just with a `export default class` rewrite this general
/// version doesn't need) so a package matching *both* shapes doesn't get
/// the same file's declarations duplicated.
///
/// Deliberately shallow: only scans for a bare `TsTypeRef` naming an
/// import-equals-bound identifier, once, in the *entry* file. A referenced
/// file's own further cross-file references (nodemailer's `Mail` file
/// itself imports `DKIM`/`MimeNode`/`XOAuth2` from further sibling files)
/// are not chased -- thaw-bridge's classification already degrades an
/// unresolved type to `Unsupported`/`JsValue`/`Json` gracefully rather
/// than failing the whole file, so a peripheral type staying unresolved
/// is an acceptable, honest gap, not a crash.
fn inline_import_equals_referenced_types(
    module: &thaw_parser::ast::Module,
    import_equals_targets: &std::collections::HashMap<String, PathBuf>,
    already_inlined: Option<&str>,
) -> Result<Vec<(PathBuf, String)>, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{TsEntityName, TsTypeRef};

    struct TypeRefNames {
        names: std::collections::BTreeSet<String>,
    }
    impl Visit for TypeRefNames {
        fn visit_ts_type_ref(&mut self, type_ref: &TsTypeRef) {
            if let TsEntityName::Ident(ident) = &type_ref.type_name {
                self.names.insert(ident.sym.to_string());
            }
            type_ref.visit_children_with(self);
        }
    }
    let mut finder = TypeRefNames {
        names: std::collections::BTreeSet::new(),
    };
    module.visit_with(&mut finder);

    let mut targets = import_equals_targets
        .iter()
        .filter(|(name, _)| {
            Some(name.as_str()) != already_inlined && finder.names.contains(*name)
        })
        .collect::<Vec<_>>();
    targets.sort_by_key(|(name, _)| *name);

    let mut output = Vec::new();
    for (name, target_path) in targets {
        let source = fs::read_to_string(target_path).map_err(|error| {
            format!(
                "failed to read `{}`'s imported type `{name}`: {error}",
                target_path.display()
            )
        })?;
        output.push((target_path.clone(), format!("\n{source}\n")));
    }
    Ok(output)
}
