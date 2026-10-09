/// Parses a `.d.ts` source string and extracts every top-level function
/// signature (`declare function foo(...): T;` and
/// `export declare function foo(...): T;` -- `.d.ts` files don't have
/// function bodies to begin with, so plain `export function foo(...): T;`
/// is equally ambient here). `interface` declarations are resolved first
/// (see `resolve_interfaces`) so a signature using one classifies as
/// `Native` just like an inline `{ ... }` type literal would. Anything else
/// at the top level (classes, `const`, re-exports, ...) is silently
/// skipped: this is a function-signature extractor, not a full `.d.ts`
/// model.
pub fn parse_dts(source: &str) -> Result<Vec<DtsFunction>, String> {
    parse_dts_named(source, thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn parse_dts_named(
    source: &str, filename: thaw_parser::common::FileName,
) -> Result<Vec<DtsFunction>, String> {
    let module = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone())?.0;
    let generated_internals = generated_public_alias_internals(&module);
    let type_only_namespaces = type_only_namespace_names_named(source, &filename);
    let export_assignment = export_assignment_namespace(&module);
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut functions = scoped_fn_decls(&module)
        .into_iter()
        .filter(|(full_name, _, _)| !namespace_member_is_type_only(full_name, &type_only_namespaces))
        .map(|(full_name, _, func)| {
            let (context, generic_context) = scoped_type_context(
                declaration_scope(&full_name), &interfaces, &generic_interfaces,
            );
            let public_name = export_assignment
                .and_then(|namespace| full_name.strip_prefix(format!("{namespace}.").as_str()))
                .map(str::to_string)
                .unwrap_or(full_name);
            lower_dts_function(&public_name, func, &context, &generic_context)
        })
        .collect::<Vec<_>>();
    for (original, public) in namespace_value_aliases(&module).into_iter()
        .chain(scoped_exported_import_equals_aliases(&module)) {
        let prefix = format!("{original}.");
        let aliases = functions.iter().filter_map(|function| {
            function.name.strip_prefix(&prefix).map(|suffix| {
                let mut alias = function.clone();
                alias.name = format!("{public}.{suffix}");
                alias
            })
        }).collect::<Vec<_>>();
        functions.extend(aliases);
    }
    let hidden_namespaces = nonpublic_namespace_sources_named(source, &filename);
    functions.retain(|function| !namespace_member_is_type_only(&function.name, &hidden_namespaces));
    if let Some(target) = export_assignment_interface_name(&module) {
        functions.extend(export_assignment_interface_methods(&module, &target)
            .into_iter()
            .map(|(name, owner, method)| {
                let (context, generic_context) = scoped_type_context(
                    declaration_scope(&owner), &interfaces, &generic_interfaces,
                );
                lower_dts_method_signature(&name, method, &context, &generic_context)
            }));
    }
    let call_signature_interfaces = all_interface_decls_by_name(&module);
    let merged_call_signature_interfaces = all_interface_decls_by_name_merged(&module);
    let local_type_aliases = all_type_alias_decls_by_name(&module);
    functions.extend(
        module
            .body
            .iter()
            .flat_map(|item| {
                extract_const_call_signature_decls(
                    item,
                    &merged_call_signature_interfaces,
                    &local_type_aliases,
                    "",
                    &HashMap::new(),
                )
            })
            .map(|(name, signature, _)| match signature {
                CallableConstSignature::Interface(call) => lower_dts_call_signature(
                    &name,
                    call,
                    &interfaces,
                    &generic_interfaces,
                    &local_type_aliases,
                    &call_signature_interfaces,
                ),
                CallableConstSignature::Direct(function) => lower_dts_fn_type(
                    &name,
                    function,
                    &interfaces,
                    &generic_interfaces,
                    &local_type_aliases,
                    &call_signature_interfaces,
                ),
                CallableConstSignature::Method(method) => lower_dts_method_signature(
                    &name,
                    method,
                    &interfaces,
                    &generic_interfaces,
                ),
            }),
    );
    let mut scoped_consts = Vec::new();
    let mut scoped_tables = HashMap::new();
    for (scope, item) in scoped_module_items(&module) {
        if scope.is_empty() { continue; }
        let is_variable = matches!(item,
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(_)))
                | ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(
                    swc_ecma_ast::ExportDecl { decl: Decl::Var(_), .. })));
        if !is_variable { continue; }
        let (raw_interfaces, scoped_aliases, owners) = scoped_tables.entry(scope.clone())
            .or_insert_with(|| scoped_callable_type_tables(&module, &scope));
        let extracted = extract_const_call_signature_decls(
            item, raw_interfaces, scoped_aliases, &scope, owners,
        );
        for (name, signature, owner) in extracted {
            let full_name = format!("{scope}.{name}");
            let (owner_interfaces, owner_aliases, _) = scoped_tables.entry(owner.clone())
                .or_insert_with(|| scoped_callable_type_tables(&module, &owner));
            let first_interfaces = owner_interfaces.iter().filter_map(|(name, declarations)|
                declarations.first().map(|declaration| (name.clone(), *declaration)))
                .collect::<HashMap<_, _>>();
            let (context, generic_context) = scoped_type_context(&owner, &interfaces, &generic_interfaces);
            let function = match signature {
                CallableConstSignature::Interface(call) => lower_dts_call_signature(
                    &full_name, call, &context, &generic_context, owner_aliases, &first_interfaces,
                ),
                CallableConstSignature::Direct(function) => lower_dts_fn_type(
                    &full_name, function, &context, &generic_context, owner_aliases, &first_interfaces,
                ),
                CallableConstSignature::Method(method) => lower_dts_method_signature(
                    &full_name, method, &context, &generic_context,
                ),
            };
            scoped_consts.push(function);
        }
    }
    for (original, public) in namespace_value_aliases(&module).into_iter()
        .chain(scoped_exported_import_equals_aliases(&module)) {
        let prefix = format!("{original}.");
        let aliases = scoped_consts.iter().filter_map(|function| {
            function.name.strip_prefix(&prefix).map(|suffix| {
                let mut alias = function.clone();
                alias.name = format!("{public}.{suffix}");
                alias
            })
        }).collect::<Vec<_>>();
        scoped_consts.extend(aliases);
    }
    scoped_consts.retain(|function| !namespace_member_is_type_only(&function.name, &type_only_namespaces)
        && !namespace_member_is_type_only(&function.name, &hidden_namespaces));
    functions.extend(scoped_consts);
    // An export specifier may expose a reserved property name that cannot be
    // the name of a function declaration (`export { _null as null }`). The
    // registry keeps the valid internal declaration and the public export;
    // carry its signatures under that public name for the qualified shim.
    // Ordinary aliases are already materialized as renamed declarations by
    // the registry and do not need a second copy here.
    let mut seen_aliases = HashSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only || export.src.is_some() {
            continue;
        }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else {
                continue;
            };
            let Some(swc_ecma_ast::ModuleExportName::Ident(exported)) = &named.exported else {
                continue;
            };
            let original = original.sym.as_ref();
            let exported = exported.sym.as_ref();
            if !is_reserved_js_identifier(exported)
                || !seen_aliases.insert((original, exported))
                || (!original.starts_with("__thaw_public_")
                    && functions.iter().any(|function| function.name == exported))
            {
                continue;
            }
            let aliases = functions
                .iter()
                .filter(|function| function.name == original)
                .cloned()
                .map(|mut function| {
                    function.name = exported.to_string();
                    function
                })
                .collect::<Vec<_>>();
            functions.extend(aliases);
        }
    }
    let mut scoped_generated_internals = HashSet::new();
    let mut scoped_aliases_seen = HashSet::new();
    for (scope, item) in scoped_module_items(&module) {
        if scope.is_empty() { continue; }
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.type_only || export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            if named.is_type_only { continue; }
            let (swc_ecma_ast::ModuleExportName::Ident(original),
                Some(swc_ecma_ast::ModuleExportName::Ident(public))) =
                (&named.orig, &named.exported) else { continue };
            let source_name = format!("{scope}.{}", original.sym);
            let public_name = format!("{scope}.{}", public.sym);
            if source_name == public_name
                || !scoped_aliases_seen.insert((source_name.clone(), public_name.clone())) {
                continue;
            }
            let aliases = functions.iter().filter(|function| function.name == source_name)
                .cloned().map(|mut function| {
                    function.name = public_name.clone();
                    function
                }).collect::<Vec<_>>();
            if !aliases.is_empty() && original.sym.as_str().starts_with("__thaw_public_") {
                scoped_generated_internals.insert(source_name);
            }
            functions.extend(aliases);
        }
    }
    // A selected function can be exposed directly through a qualified
    // `export import Public = Owner.function`; the namespace alias pass
    // above copies descendants only and has no descendant for the
    // function itself.
    for (original, public) in scoped_exported_import_equals_aliases(&module) {
        if functions.iter().any(|function| function.name == public) { continue; }
        let aliases = functions.iter().filter(|function| function.name == original)
            .cloned().map(|mut function| {
                function.name = public.clone();
                function
            }).collect::<Vec<_>>();
        functions.extend(aliases);
    }
    functions.retain(|function| !generated_internals.contains(&function.name)
        && !scoped_generated_internals.contains(&function.name));
    Ok(functions)
}

fn export_assignment_namespace(module: &Module) -> Option<&str> {
    module.body.iter().find_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) = item else {
            return None;
        };
        match export.expr.as_ref() {
            Expr::Ident(ident) => Some(ident.sym.as_str()),
            _ => None,
        }
    })
}

fn namespace_member_is_type_only(full_name: &str, type_only: &HashSet<String>) -> bool {
    full_name.match_indices('.').any(|(index, _)| type_only.contains(&full_name[..index]))
}

fn generated_type_only_namespace_marker(name: &str) -> bool {
    name.strip_prefix("__thaw_type_only_namespace_marker_")
        .is_some_and(|suffix| suffix.len() == 16
            && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn namespace_value_aliases(module: &Module) -> Vec<(String, String)> {
    use swc_ecma_ast::{ExportSpecifier, ModuleExportName};
    module.body.iter().filter_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { return None };
        (!export.type_only && export.src.is_none()).then_some(export)
    }).flat_map(|export| export.specifiers.iter().filter_map(|specifier| {
        let ExportSpecifier::Named(named) = specifier else { return None };
        if named.is_type_only { return None; }
        let (ModuleExportName::Ident(original), Some(ModuleExportName::Ident(public))) =
            (&named.orig, &named.exported) else { return None; };
        (original.sym != public.sym).then(|| (original.sym.to_string(), public.sym.to_string()))
    })).collect()
}

/// Original bindings that have a live namespace alias but are not themselves
/// public runtime properties. Their declarations still supply type context.
pub fn nonpublic_namespace_sources(source: &str) -> HashSet<String> {
    nonpublic_namespace_sources_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn nonpublic_namespace_sources_named(source: &str, filename: &thaw_parser::common::FileName) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else { return HashSet::new(); };
    let public = exported_value_names_named(source, filename);
    let assigned = export_assignment_namespace(&module);
    namespace_value_aliases(&module).into_iter().filter_map(|(original, _)| {
        (!public.contains(&original) && assigned != Some(original.as_str())).then_some(original)
    }).collect()
}

/// Namespaces with no runtime export of their original binding. A value
/// alias (`export { Source as Live }`) keeps Source live even if another
/// edge exports Source only as a type.
pub fn type_only_namespace_names(source: &str) -> HashSet<String> {
    type_only_namespace_names_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn type_only_namespace_names_named(source: &str, filename: &thaw_parser::common::FileName) -> HashSet<String> {
    use swc_ecma_ast::{ExportSpecifier, ModuleExportName, Stmt, TsKeywordTypeKind,
        TsModuleName, TsNamespaceBody, TsType};

    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashSet::new();
    };
    let mut candidates = HashSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else { continue };
            if !export.type_only && !named.is_type_only { continue; }
            if let ModuleExportName::Ident(original) = &named.orig {
                candidates.insert(original.sym.to_string());
            }
        }
    }
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                match &export.decl {
                    Decl::TsModule(namespace) => {
                        if let TsModuleName::Ident(ident) = &namespace.id {
                            candidates.remove(ident.sym.as_ref());
                        }
                    }
                    Decl::Class(class) => { candidates.remove(class.ident.sym.as_ref()); }
                    Decl::TsEnum(enumeration) => { candidates.remove(enumeration.id.sym.as_ref()); }
                    _ => {}
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if !export.type_only && export.src.is_none() => {
                for specifier in &export.specifiers {
                    match specifier {
                        ExportSpecifier::Named(named) if !named.is_type_only => {
                            if let ModuleExportName::Ident(original) = &named.orig {
                                candidates.remove(original.sym.as_ref());
                            }
                        }
                        ExportSpecifier::Namespace(namespace) => {
                            if let ModuleExportName::Ident(name) = &namespace.name {
                                candidates.remove(name.sym.as_ref());
                            }
                        }
                        _ => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
                if let Expr::Ident(ident) = export.expr.as_ref() {
                    candidates.remove(ident.sym.as_ref());
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(export)) => {
                if let Expr::Ident(ident) = export.expr.as_ref() {
                    candidates.remove(ident.sym.as_ref());
                }
            }
            _ => {}
        }
    }
    // A generated type-only namespace can itself live inside a public
    // namespace (for example `parts.Shapes`). Its local `export type`
    // marker must suppress runtime capture of class members below it.
    fn nested_type_only(
        body: &[ModuleItem], scope: &str, names: &mut HashSet<String>,
    ) {
        let mut local = HashSet::new();
        for item in body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
            if export.src.is_some() || !export.type_only { continue; }
            for specifier in &export.specifiers {
                let ExportSpecifier::Named(named) = specifier else { continue };
                if let ModuleExportName::Ident(original) = &named.orig {
                    local.insert(original.sym.to_string());
                }
            }
        }
        for item in body {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                    Decl::TsModule(namespace) => {
                        if let TsModuleName::Ident(ident) = &namespace.id {
                            local.remove(ident.sym.as_ref());
                        }
                    }
                    Decl::Class(class) => { local.remove(class.ident.sym.as_ref()); }
                    Decl::TsEnum(enumeration) => { local.remove(enumeration.id.sym.as_ref()); }
                    _ => {}
                },
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                    if !export.type_only && export.src.is_none() => {
                    for specifier in &export.specifiers {
                        let ExportSpecifier::Named(named) = specifier else { continue };
                        if let ModuleExportName::Ident(original) = &named.orig {
                            local.remove(original.sym.as_ref());
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
                    if let Expr::Ident(ident) = export.expr.as_ref() {
                        local.remove(ident.sym.as_ref());
                    }
                }
                _ => {}
            }
        }
        names.extend(local.into_iter().map(|name| format!("{scope}.{name}")));
        for item in body {
            let namespace = match item {
                ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(namespace))) => Some(namespace),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) =>
                    if let Decl::TsModule(namespace) = &export.decl { Some(namespace) } else { None },
                _ => None,
            };
            let Some(namespace) = namespace else { continue };
            let TsModuleName::Ident(name) = &namespace.id else { continue };
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { continue };
            let child_scope = format!("{scope}.{}", name.sym);
            if block.body.iter().any(|member| matches!(member,
                ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias)))
                    if generated_type_only_namespace_marker(alias.id.sym.as_ref())
                        && matches!(alias.type_ann.as_ref(), TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsNeverKeyword))) {
                names.insert(child_scope.clone());
            }
            nested_type_only(&block.body, &child_scope, names);
        }
    }
    for item in &module.body {
        let namespace = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(namespace))) => Some(namespace),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) =>
                if let Decl::TsModule(namespace) = &export.decl { Some(namespace) } else { None },
            _ => None,
        };
        let Some(namespace) = namespace else { continue };
        let TsModuleName::Ident(name) = &namespace.id else { continue };
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { continue };
        nested_type_only(&block.body, name.sym.as_ref(), &mut candidates);
    }
    candidates
}

fn generated_public_alias_internals(module: &Module) -> HashSet<String> {
    let mut names = HashSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.type_only || export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            if named.is_type_only { continue; }
            let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else { continue };
            let Some(swc_ecma_ast::ModuleExportName::Ident(exported)) = &named.exported else { continue };
            if original.sym.as_str().starts_with("__thaw_public_")
                && is_reserved_js_identifier(exported.sym.as_ref())
            {
                names.insert(original.sym.to_string());
            }
        }
    }
    names
}

/// Extracts typed, non-callable top-level value declarations. Callable
/// `const`s are already returned by [`parse_dts`] and are excluded here.
/// Whether a callable `const`'s annotation names (or intersects with) an object type that also has
/// members: `KyInstance`, `debug.Debug & { debug: Debug }`, `{ (x): R; get: ... }`.
fn type_is_callable_object(ty: &TsType, callable_objects: &HashSet<String>) -> bool {
    match ty {
        TsType::TsTypeRef(reference) => {
            let name = match &reference.type_name {
                TsEntityName::Ident(ident) => ident.sym.as_str(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.as_str(),
            };
            callable_objects.contains(name)
        }
        TsType::TsParenthesizedType(inner) => type_is_callable_object(&inner.type_ann, callable_objects),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(intersection)) => {
            intersection.types.iter().any(|member| type_is_callable_object(member, callable_objects))
        }
        TsType::TsTypeLit(literal) => literal.members.iter().any(|member| matches!(
            member,
            TsTypeElement::TsPropertySignature(_) | TsTypeElement::TsMethodSignature(_)
        )),
        _ => false,
    }
}

pub fn parse_dts_values(source: &str) -> Result<Vec<DtsValue>, String> {
    parse_dts_values_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn parse_dts_values_named(source: &str, filename: &thaw_parser::common::FileName) -> Result<Vec<DtsValue>, String> {
    let module = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module)?;
    let generated_internals = generated_public_alias_internals(&module);
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let interface_declarations = all_interface_decls_by_name(&module);
    let callable_objects = module
        .body
        .iter()
        .flat_map(extract_interface_decls)
        .filter(|interface| {
            interface.body.body.iter().any(|member| {
                matches!(
                    member,
                    TsTypeElement::TsPropertySignature(_) | TsTypeElement::TsMethodSignature(_)
                )
            })
        })
        .map(|interface| interface.id.sym.to_string())
        // `type KyInstance = { (url): R; get: ...; }` is the same callable object as the
        // interface spelling.
        .chain(module.body.iter().flat_map(extract_type_alias_decls).filter_map(|alias| {
            let TsType::TsTypeLit(literal) = alias.type_ann.as_ref() else { return None };
            literal.members.iter().any(|member| matches!(
                member,
                TsTypeElement::TsPropertySignature(_) | TsTypeElement::TsMethodSignature(_)
            )).then(|| alias.id.sym.to_string())
        }))
        .collect::<HashSet<_>>();
    let callable = parse_dts_named(source, filename.clone())?
        .into_iter()
        .map(|function| function.name)
        .collect::<HashSet<_>>();
    let class_names = parse_dts_classes_named(source, filename)?
        .into_iter()
        .map(|class| class.name)
        .collect::<HashSet<_>>();
    let mut values = Vec::new();
    // The same name can legitimately appear more than once in `source`:
    // thaw-registry's own `.d.ts` flattening concatenates every file a
    // package's type declarations span into this one string, and a
    // value re-exported (or separately ambient-declared) from more than
    // one of those files under its own name produces two declarations
    // for the same binding here -- real examples: `marked`'s own
    // `_defaults`, `js-yaml`'s own `binaryTag`. Undeduplicated, this
    // emitted the same `declare function .../let ...` shim pair twice,
    // which the shim generator's own duplicate-binding check (rightly)
    // rejects as a hard error -- the exact same class of bug already
    // fixed for `parse_dts_classes` (see its own doc comment: yaml's
    // `NodeBase`, socket.io's `StrictEventEmitter`), just never applied
    // here too. First occurrence wins, matching that precedent.
    let mut seen_names = HashSet::new();
    for (scope, item) in scoped_module_items(&module) {
        let (value_context, generic_value_context) = scoped_type_context(
            &scope, &interfaces, &generic_interfaces,
        );
        let declaration = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(declaration))) => {
                declaration.as_ref()
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Var(declaration) => declaration.as_ref(),
                _ => continue,
            },
            _ => continue,
        };
        for declarator in &declaration.decls {
            if let Pat::Object(object) = &declarator.name {
                let Some(annotation) = &object.type_ann else {
                    continue;
                };
                let TsType::TsTypeRef(reference) = annotation.type_ann.as_ref() else {
                    continue;
                };
                let interface_name = match &reference.type_name {
                    TsEntityName::Ident(ident) => ident.sym.as_str(),
                    TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.as_str(),
                };
                let Some(interface) = interface_declarations.get(interface_name) else {
                    continue;
                };
                for property in &object.props {
                    let (source_name, name) = match property {
                        swc_ecma_ast::ObjectPatProp::Assign(assign) => {
                            let name = assign.key.sym.to_string();
                            (name.clone(), name)
                        }
                        swc_ecma_ast::ObjectPatProp::KeyValue(property) => {
                            let swc_ecma_ast::PropName::Ident(source) = &property.key else {
                                continue;
                            };
                            let Pat::Ident(binding) = property.value.as_ref() else {
                                continue;
                            };
                            (source.sym.to_string(), binding.id.sym.to_string())
                        }
                        swc_ecma_ast::ObjectPatProp::Rest(_) => continue,
                    };
                    let name = if scope.is_empty() { name } else { format!("{scope}.{name}") };
                    if callable.contains(&name)
                        || class_names.contains(&name)
                        || !seen_names.insert(name.clone())
                    {
                        continue;
                    }
                    let Some(type_annotation) = interface.body.body.iter().find_map(|member| {
                        let TsTypeElement::TsPropertySignature(property) = member else {
                            return None;
                        };
                        let Expr::Ident(key) = property.key.as_ref() else {
                            return None;
                        };
                        (key.sym == source_name).then_some(property.type_ann.as_deref()).flatten()
                    }) else {
                        continue;
                    };
                    let ty = resolve_ts_type_with_substitution(
                        &type_annotation.type_ann,
                        &HashMap::new(),
                        &value_context,
                        &generic_value_context,
                        &mut Vec::new(),
                    );
                    values.push(DtsValue { name, ty });
                }
                continue;
            }
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            let name = if scope.is_empty() { binding.id.sym.to_string() }
                else { format!("{scope}.{}", binding.id.sym) };
            let Some(annotation) = &binding.type_ann else {
                continue;
            };
            if !seen_names.insert(name.clone()) {
                continue;
            }
            if callable.contains(&name) {
                let callable_object = type_is_callable_object(annotation.type_ann.as_ref(), &callable_objects);
                if callable_object {
                    values.push(DtsValue {
                        name,
                        ty: DtsType::Native(HirType::JsValue),
                    });
                }
                continue;
            }
            let mut ty = resolve_ts_type_with_substitution(
                &annotation.type_ann,
                &HashMap::new(),
                &value_context,
                &generic_value_context,
                &mut Vec::new(),
            );
            if matches!(&ty, DtsType::Unsupported(_)) {
                let referenced = match annotation.type_ann.as_ref() {
                    TsType::TsTypeRef(reference) => match &reference.type_name {
                        TsEntityName::Ident(ident) => Some(ident.sym.as_str()),
                        TsEntityName::TsQualifiedName(qualified) => Some(qualified.right.sym.as_str()),
                    },
                    _ => None,
                };
                if referenced.is_some_and(|name| class_names.contains(name)) {
                    ty = DtsType::Native(HirType::JsValue);
                }
            }
            values.push(DtsValue { name, ty });
        }
    }
    // A package whose whole `.d.ts` is `import * as m from "./index.js";
    // export default m;` plus a couple of option interfaces -- no usable
    // function/class types at all, because the real API is JSDoc in a
    // `.js` file thaw can't resolve (real example: `mustache`). Bind its
    // default export to the runtime module object as an opaque handle, so
    // `import M from "pkg"; M.method(...)` routes through the dynamic
    // method-call path instead of failing with "unknown function".
    if values.is_empty()
        && callable.is_empty()
        && class_names.is_empty()
        && self_referential_namespace_aliases_named(source, filename).contains("default")
    {
        values.push(DtsValue {
            name: "default".to_string(),
            ty: DtsType::Native(HirType::JsValue),
        });
    }
    let mut aliases = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else { continue };
            let Some(swc_ecma_ast::ModuleExportName::Ident(public)) = &named.exported else { continue };
            if original.sym == public.sym { continue; }
            if callable.contains(public.sym.as_ref()) || class_names.contains(public.sym.as_ref()) {
                continue;
            }
            aliases.push((original.sym.to_string(), public.sym.to_string()));
        }
    }
    for (original, public) in scoped_exported_import_equals_aliases(&module) {
        if callable.contains(&public) || class_names.contains(&public) {
            continue;
        }
        aliases.push((original, public));
    }
    loop {
        let mut changed = false;
        for (original, public) in &aliases {
            if values.iter().any(|value| &value.name == public) { continue; }
            if let Some(mut value) = values.iter().find(|value| &value.name == original).cloned() {
                value.name = public.clone();
                values.push(value);
                changed = true;
            }
        }
        if !changed { break; }
    }
    values.retain(|value| !generated_internals.contains(&value.name));
    Ok(values)
}

/// Every interface declared anywhere in `module`, by its own bare name --
/// unlike `resolve_interfaces`'s internal `raw` map, this doesn't split
/// generic from non-generic interfaces (a call-signature interface may
/// carry its own unrelated generic parameter, e.g. drizzle-orm's
/// `SQLiteTableFn<TSchema extends string | undefined = undefined>`, which
/// is irrelevant to extracting its call signatures below). Used only to
/// look an interface up by name for `extract_const_call_signature_decls`;
/// not a replacement for `resolve_interfaces`'s own classification map.
fn all_interface_decls_by_name(module: &Module) -> HashMap<String, &TsInterfaceDecl> {
    let mut map = HashMap::new();
    for iface in module.body.iter().flat_map(extract_interface_decls) {
        map.entry(iface.id.sym.to_string()).or_insert(iface);
    }
    map
}

fn all_interface_decls_by_name_merged(module: &Module) -> HashMap<String, Vec<&TsInterfaceDecl>> {
    let mut map: HashMap<String, Vec<&TsInterfaceDecl>> = HashMap::new();
    for iface in module.body.iter().flat_map(extract_interface_decls) {
        map.entry(iface.id.sym.to_string()).or_default().push(iface);
    }
    map
}

/// Every type alias declared anywhere in `module`, by its own bare name --
/// like `all_interface_decls_by_name` just above, this doesn't split
/// generic from non-generic aliases: a generic alias's own type parameter
/// is never substituted here (`resolve_local_callable_fn_types` below
/// doesn't attempt that), but the call signatures it chases through still
/// resolve correctly -- any member whose param/return type mentions the
/// alias's own type parameter simply fails to classify as a native type
/// and widens to `Unsupported`/`JsValue` downstream, the same graceful
/// degradation any other unresolvable type already gets. Real example:
/// tar's own `type TarCommand<AsyncClass, SyncClass extends { sync: true
/// }> = { (): AsyncClass; ... } & { (opt: TarOptionsWithAliasesAsyncFile):
/// Promise<void>; ... } & ...;` -- the `Promise<void>`-returning
/// with-file overloads (the shape real code overwhelmingly calls) fully
/// classify even though `AsyncClass`/`SyncClass` themselves never do.
/// Used only to look a type alias up by name when resolving a `declare
/// const`'s own type, not a replacement for `resolve_interfaces`'s own
/// classification map.
fn all_type_alias_decls_by_name(module: &Module) -> HashMap<String, &TsType> {
    let mut map = HashMap::new();
    for alias in module.body.iter().flat_map(extract_type_alias_decls) {
        map.entry(alias.id.sym.to_string())
            .or_insert(alias.type_ann.as_ref());
    }
    map
}

/// A callable const inside a namespace resolves local type names before
/// equally named declarations in an outer scope. Keep the old top-level
/// maps untouched; only nested declarations need this lexical view.
fn scoped_callable_type_tables<'a>(
    module: &'a Module,
    scope: &str,
) -> (HashMap<String, Vec<&'a TsInterfaceDecl>>, HashMap<String, &'a TsType>, HashMap<String, String>) {
    let (interfaces, aliases) = scoped_type_declarations(module);
    let mut scoped_interfaces = HashMap::<String, Vec<&TsInterfaceDecl>>::new();
    let mut scoped_aliases = HashMap::<String, &TsType>::new();
    let mut owners = HashMap::new();
    for (name, declaration) in interfaces {
        owners.insert(name.clone(), declaration_scope(&name).to_string());
        scoped_interfaces.entry(name).or_default().push(declaration);
    }
    for (name, declaration) in aliases {
        owners.insert(name.clone(), declaration_scope(&name).to_string());
        scoped_aliases.entry(name).or_insert(declaration.type_ann.as_ref());
    }
    let interface_names = scoped_interfaces.keys()
        .map(|name| name.rsplit('.').next().unwrap_or(name).to_string())
        .collect::<HashSet<_>>();
    for bare in interface_names {
        if let Some(name) = lexical_type_key(&bare, scope, |name| scoped_interfaces.contains_key(name)) {
            let declarations = scoped_interfaces[&name].clone();
            owners.insert(bare.clone(), declaration_scope(&name).to_string());
            scoped_interfaces.insert(bare, declarations);
        }
    }
    let alias_names = scoped_aliases.keys()
        .map(|name| name.rsplit('.').next().unwrap_or(name).to_string())
        .collect::<HashSet<_>>();
    for bare in alias_names {
        if let Some(name) = lexical_type_key(&bare, scope, |name| scoped_aliases.contains_key(name)) {
            let declaration = scoped_aliases[&name];
            owners.insert(bare.clone(), declaration_scope(&name).to_string());
            scoped_aliases.insert(bare, declaration);
        }
    }
    (scoped_interfaces, scoped_aliases, owners)
}

/// Resolves `ty` to zero or more direct function-type call signatures,
/// following a chain of local (bare or exported) non-generic type-alias
/// references and unwrapping an intersection into each of its own
/// operands. Real example: uuid's own `.d.ts` (via `@types/uuid`), `type
/// v4 = v4Buffer & v4String;`, where `v4Buffer`/`v4String` are themselves
/// further local aliases each resolving to a direct (possibly its own
/// separately-generic) function type -- `v1`/`v3`/`v5`/`v6`/`v7`/
/// `parse`/`stringify`/etc. all use the identical two-alias-intersection
/// shape. Also unwraps an inline call-signature-bearing object type
/// literal (`{ (): T; (opt): T; }`, the *interface-body* shape rather
/// than the arrow-function shape) as an intersection member -- real
/// example: tar's own `type TarCommand<...> = { (): AsyncClass; ... } &
/// { (opt): Promise<void>; ... } & ... & { syncFile: ...; validate?:
/// ...; };`, an 8-way intersection of exactly this shape (the final
/// member, with only property signatures and no call signature at all,
/// naturally contributes nothing). `visited` guards against a self-
/// referential alias cycle. Real `.d.ts` shapes seen so far never need
/// anything deeper than this (a plain reference, an intersection of
/// references/direct function types/inline call-signature literals) --
/// a union, mapped type, etc. isn't unwrapped, and just contributes no
/// call signatures (same as any other unresolvable type).
fn resolve_local_callable_fn_types<'a>(
    ty: &'a TsType,
    aliases: &HashMap<String, &'a TsType>,
    visited: &mut HashSet<String>,
    scope: &str,
    owners: &HashMap<String, String>,
) -> Vec<(CallableConstSignature<'a>, String)> {
    match ty {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            vec![(CallableConstSignature::Direct(function), scope.to_string())]
        }
        TsType::TsTypeLit(lit) => lit
            .members
            .iter()
            .filter_map(|member| match member {
                TsTypeElement::TsCallSignatureDecl(call) => {
                    Some((CallableConstSignature::Interface(call), scope.to_string()))
                }
                _ => None,
            })
            .collect(),
        TsType::TsTypeRef(ty_ref) => {
            let name = callable_type_lookup_key(&ty_ref.type_name, aliases, scope);
            if !visited.insert(name.clone()) {
                return Vec::new();
            }
            match aliases.get(name.as_str()) {
                Some(aliased) => resolve_local_callable_fn_types(
                    aliased, aliases, visited,
                    owners.get(&name).map(String::as_str).unwrap_or(scope), owners,
                ),
                None => Vec::new(),
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => intersection
            .types
            .iter()
            .flat_map(|member| resolve_local_callable_fn_types(member, aliases, visited, scope, owners))
            .collect(),
        _ => Vec::new(),
    }
}

fn callable_type_lookup_key<T>(name: &TsEntityName, table: &HashMap<String, T>, scope: &str) -> String {
    let qualified = type_reference_name(name);
    if let Some(name) = lexical_type_key(&qualified, scope, |name| table.contains_key(name)) {
        return name;
    }
    // Preserve the old top-level fallback. In a namespace, an unresolved
    // qualified reference must not bind to a same-named local declaration.
    if scope.is_empty() {
        qualified.rsplit('.').next().unwrap_or(&qualified).to_string()
    } else {
        qualified
    }
}

/// Which of `.d.ts`'s two callable-shape AST nodes a `declare const`
/// export's own type annotation actually is. `Interface` is the shape
/// `extract_const_call_signature_decls`'s doc comment describes
/// (`SQLiteTableFn`-style, possibly several overloads, one
/// `TsCallSignatureDecl` per); `Direct` is a *self-contained* inline
/// function type with no interface involved at all -- real example: zod
/// v3's `lib/types.d.ts`, `declare const objectType: <T extends
/// ZodRawShape>(shape: T, params?: RawCreateParams) => ZodObject<...>;`,
/// later locally rename-exported (`export { objectType as object, ... }`)
/// -- the shape essentially all of zod v3's own primitives (`object`,
/// `string`, `number`, ...) use, unlike zod v4 (unaffected) or drizzle-
/// orm's interface-based `sqliteTable`. `TsCallSignatureDecl` and
/// `TsFnType` carry identical `params`/`type_params`/`type_ann` fields,
/// just with `type_ann` optional on one and required on the other --
/// kept as two small sibling `lower_dts_*` functions
/// (`lower_dts_call_signature`/`lower_dts_fn_type`) rather than one
/// shared generic helper, matching this file's existing convention of
/// one function per distinct AST shape.
enum CallableConstSignature<'a> {
    Interface(&'a TsCallSignatureDecl),
    Direct(&'a TsFnType),
    Method(&'a TsMethodSignature),
}

/// Every top-level `declare const NAME: T;` (`let`/`var` too, exported or
/// bare -- a bare one only matters together with a later local rename-
/// export, see `all_reexported_function_declarations` in thaw-registry's
/// `install.rs`) whose type `T` is a callable shape: either a `TypeRef`
/// resolving (via `interfaces`, see `all_interface_decls_by_name`) to a
/// same-file interface with at least one call signature, or a direct
/// inline function type -- see `CallableConstSignature`'s own doc
/// comment for real examples of both. Parallel to `extract_fn_decls`/
/// `extract_interface_method_decls`: yields `NAME -> each callable
/// signature found (one per interface overload, or the one direct type),
/// so a caller can synthesize a `DtsFunction` per signature exactly like
/// an ordinary ambient function declaration.
fn extract_const_call_signature_decls<'a>(
    item: &'a ModuleItem,
    interfaces: &HashMap<String, Vec<&'a TsInterfaceDecl>>,
    local_type_aliases: &HashMap<String, &'a TsType>,
    scope: &str,
    owners: &HashMap<String, String>,
) -> Vec<(String, CallableConstSignature<'a>, String)> {
    let var_decl = match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(var_decl))) => var_decl.as_ref(),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Var(var_decl) => var_decl.as_ref(),
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    var_decl
        .decls
        .iter()
        .filter_map(|declarator| {
            if let Pat::Object(object) = &declarator.name {
                let annotation = object.type_ann.as_ref()?;
                let TsType::TsTypeRef(reference) = annotation.type_ann.as_ref() else {
                    return None;
                };
                let interface_name = callable_type_lookup_key(&reference.type_name, interfaces, scope);
                let declarations = interfaces.get(&interface_name)?;
                let interface_scope = owners.get(&interface_name).map(String::as_str).unwrap_or(scope);
                let bindings = object
                    .props
                    .iter()
                    .filter_map(|property| match property {
                        swc_ecma_ast::ObjectPatProp::Assign(assign) => {
                            let name = assign.key.sym.to_string();
                            Some((name.clone(), name))
                        }
                        swc_ecma_ast::ObjectPatProp::KeyValue(property) => {
                            let swc_ecma_ast::PropName::Ident(source) = &property.key else {
                                return None;
                            };
                            let Pat::Ident(binding) = property.value.as_ref() else {
                                return None;
                            };
                            Some((source.sym.to_string(), binding.id.sym.to_string()))
                        }
                        swc_ecma_ast::ObjectPatProp::Rest(_) => None,
                    })
                    .collect::<HashMap<_, _>>();
                let mut signatures = Vec::new();
                for member in declarations.iter().flat_map(|iface| iface.body.body.iter()) {
                    match member {
                        TsTypeElement::TsMethodSignature(method) => {
                            let Expr::Ident(key) = method.key.as_ref() else {
                                continue;
                            };
                            let Some(name) = bindings.get(key.sym.as_str()) else {
                                continue;
                            };
                            signatures.push((name.clone(), CallableConstSignature::Method(method), interface_scope.to_string()));
                        }
                        TsTypeElement::TsPropertySignature(property) => {
                            let Expr::Ident(key) = property.key.as_ref() else {
                                continue;
                            };
                            let Some(name) = bindings.get(key.sym.as_str()) else {
                                continue;
                            };
                            let Some(annotation) = &property.type_ann else {
                                continue;
                            };
                            signatures.extend(
                                resolve_local_callable_fn_types(
                                    annotation.type_ann.as_ref(),
                                    local_type_aliases,
                                    &mut HashSet::new(),
                                    interface_scope,
                                    owners,
                                )
                                .into_iter()
                                .map(|(signature, owner)| (name.clone(), signature, owner)),
                            );
                            if let TsType::TsTypeRef(reference) = annotation.type_ann.as_ref() {
                                let callable_name = callable_type_lookup_key(&reference.type_name, interfaces, interface_scope);
                                if let Some(callable) = interfaces.get(&callable_name) {
                                    let callable_scope = owners.get(&callable_name).map(String::as_str).unwrap_or(scope);
                                    signatures.extend(callable.iter().flat_map(|iface| iface.body.body.iter()).filter_map(|member| {
                                        match member {
                                            TsTypeElement::TsCallSignatureDecl(call) => Some((
                                                name.clone(),
                                                CallableConstSignature::Interface(call),
                                                callable_scope.to_string(),
                                            )),
                                            _ => None,
                                        }
                                    }));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                return (!signatures.is_empty()).then_some(signatures);
            }
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            let annotation = binding.type_ann.as_ref()?;
            let name = binding.id.sym.to_string();
            if let TsType::TsUnionOrIntersectionType(
                TsUnionOrIntersectionType::TsIntersectionType(intersection),
            ) = annotation.type_ann.as_ref() {
                let mut signatures = Vec::new();
                for member in &intersection.types {
                    if let TsType::TsTypeRef(reference) = member.as_ref() {
                        let iface_name = callable_type_lookup_key(&reference.type_name, interfaces, scope);
                        if let Some(iface) = interfaces.get(&iface_name) {
                            let iface_scope = owners.get(&iface_name).map(String::as_str).unwrap_or(scope);
                            signatures.extend(iface.iter().flat_map(|decl| decl.body.body.iter()).filter_map(|member| match member {
                                TsTypeElement::TsCallSignatureDecl(call) =>
                                    Some((name.clone(), CallableConstSignature::Interface(call), iface_scope.to_string())),
                                _ => None,
                            }));
                        }
                    }
                    signatures.extend(resolve_local_callable_fn_types(
                        member, local_type_aliases, &mut HashSet::new(), scope, owners,
                    ).into_iter().map(|(signature, owner)| (name.clone(), signature, owner)));
                }
                return (!signatures.is_empty()).then_some(signatures);
            }
            let type_ann = annotation.type_ann.as_ref();
            match type_ann {
                TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
                    Some(vec![(name, CallableConstSignature::Direct(function), scope.to_string())])
                }
                TsType::TsTypeRef(ty_ref) => {
                    let iface_name = callable_type_lookup_key(&ty_ref.type_name, interfaces, scope);
                    if let Some(iface) = interfaces.get(&iface_name) {
                        let iface_scope = owners.get(&iface_name).map(String::as_str).unwrap_or(scope);
                        let mut signatures = iface
                            .iter()
                            .flat_map(|decl| decl.body.body.iter())
                            .filter_map(|member| match member {
                                TsTypeElement::TsCallSignatureDecl(call) => {
                                    Some((name.clone(), CallableConstSignature::Interface(call), iface_scope.to_string()))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        // A callable interface's own function-typed
                        // *property* signatures are callable members of
                        // the value too (`debug`'s `createDebug.enable(ns)`
                        // / `.enabled(ns)` config API on top of its
                        // `(ns): Debugger` call signature). Emit each under
                        // its property name so it becomes a package-level
                        // Fallback function reachable via the value's
                        // member-access path -- only when the interface is
                        // actually callable, so an ordinary property-bag
                        // interface is unaffected.
                        if !signatures.is_empty() {
                            for member in iface.iter().flat_map(|decl| decl.body.body.iter()) {
                                if let TsTypeElement::TsPropertySignature(property) = member {
                                    let Some(annotation) = &property.type_ann else {
                                        continue;
                                    };
                                    let TsType::TsFnOrConstructorType(
                                        TsFnOrConstructorType::TsFnType(function),
                                    ) = annotation.type_ann.as_ref()
                                    else {
                                        continue;
                                    };
                                    let Expr::Ident(key) = property.key.as_ref() else {
                                        continue;
                                    };
                                    signatures.push((
                                        key.sym.to_string(),
                                        CallableConstSignature::Direct(function),
                                        iface_scope.to_string(),
                                    ));
                                }
                            }
                            return Some(signatures);
                        }
                    }
                    // Not a same-file call-signature interface -- try
                    // resolving it as a (possibly intersected, possibly
                    // chained) local type alias instead. Real example:
                    // uuid's own `.d.ts` (via `@types/uuid`), `export
                    // const v4: v4;` where `type v4 = v4Buffer &
                    // v4String;` is a *local, unexported* alias, not an
                    // interface at all.
                    let signatures = resolve_local_callable_fn_types(
                        annotation.type_ann.as_ref(),
                        local_type_aliases,
                        &mut HashSet::new(),
                        scope,
                        owners,
                    );
                    if signatures.is_empty() {
                        return None;
                    }
                    Some(
                        signatures
                            .into_iter()
                            .map(|(signature, owner)| (name.clone(), signature, owner))
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            }
        })
        .flatten()
        .collect()
}

/// Like `lower_dts_method_signature`, for an interface's call signature
/// (`(...): T`) instead of a named method (`name(...): T`) -- the shape
/// `export declare const NAME: SomeCallableInterface;` uses, one
/// `TsCallSignatureDecl` per overload. `TsCallSignatureDecl` carries the
/// same `params`/`type_ann`/`type_params` fields as `TsMethodSignature`,
/// just without a `key`/`computed`/`optional` (a call signature has no
/// method name of its own -- the const's own binding name is used
/// instead). Kept as its own function rather than sharing code with
/// `lower_dts_method_signature`, matching this file's existing convention
/// of one small `lower_dts_*` function per distinct AST shape.
fn lower_dts_call_signature(
    name: &str,
    call: &TsCallSignatureDecl,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    local_type_aliases: &HashMap<String, &TsType>,
    raw_interfaces: &HashMap<String, &TsInterfaceDecl>,
) -> DtsFunction {
    let name = name.to_string();
    let generic = call.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter.constraint.as_deref().map(describe_ts_type),
                )
            })
            .collect(),
        param_types: call
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: call
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: call.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: call
            .type_ann
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
        tuple_return_type: generic_tuple_return_type(
            call.type_ann.as_deref(),
            call.type_params.as_deref(),
            interfaces,
            generic_interfaces,
        ),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &call.type_params {
        for parameter in &type_params.params {
            let Some(constraint) = parameter
                .default
                .as_deref()
                .or(parameter.constraint.as_deref())
            else {
                continue;
            };
            if let DtsType::Native(constraint) = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            ) {
                substitution.insert(parameter.name.sym.to_string(), constraint);
            }
        }
    }
    let classify = |ty: &TsType| {
        resolve_ts_type_with_substitution(
            ty,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )
    };

    let rest_param = call.params.last().and_then(|param| {
        let TsFnParam::Rest(rest) = param else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => safe_param_name(binding.id.sym.as_ref()),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => classify(&array.elem_type),
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = call.params.len() - usize::from(rest_param.is_some());
    let required_params = call
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| !matches!(param, TsFnParam::Ident(binding) if binding.id.optional))
        .count();
    let mut param_field_constraints = Vec::new();
    let params = call
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let TsFnParam::Ident(binding) = param else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                param_field_constraints.push(None);
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = safe_param_name(binding.id.sym.as_ref());
            let ty = match &binding.type_ann {
                Some(ann) => {
                    param_field_constraints.push(field_constraints(
                        &ann.type_ann,
                        raw_interfaces,
                        local_type_aliases,
                        &mut HashSet::new(),
                    ));
                    classify(&ann.type_ann)
                }
                None => {
                    param_field_constraints.push(None);
                    DtsType::Unsupported("missing type annotation".to_string())
                }
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &call.type_ann {
        Some(ann) => classify(&ann.type_ann),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction {
        name,
        generic,
        params,
        param_field_constraints,
        required_params,
        rest_param,
        ret,
    }
}

/// Like `lower_dts_call_signature`, for a *direct* inline function type
/// (`(params) => Ret`) instead of an interface's call signature -- the
/// `CallableConstSignature::Direct` shape (see its own doc comment).
/// `TsFnType` carries the same `params`/`type_params` fields as
/// `TsCallSignatureDecl`, differing only in `type_ann`: always present
/// here (a function *type* is never written without one), rather than
/// optional. There can be only one of these per const (no interface-style
/// overloads), so unlike `extract_const_call_signature_decls`'s other
/// branch this never needs to be called more than once per name.
fn lower_dts_fn_type(
    name: &str,
    function: &TsFnType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    local_type_aliases: &HashMap<String, &TsType>,
    raw_interfaces: &HashMap<String, &TsInterfaceDecl>,
) -> DtsFunction {
    let name = name.to_string();
    let generic = function.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter.constraint.as_deref().map(describe_ts_type),
                )
            })
            .collect(),
        param_types: function
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: function
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: function.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: describe_ts_type(&function.type_ann.type_ann),
        tuple_return_type: generic_tuple_return_type(
            Some(function.type_ann.as_ref()),
            function.type_params.as_deref(),
            interfaces,
            generic_interfaces,
        ),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &function.type_params {
        for parameter in &type_params.params {
            let Some(constraint) = parameter
                .default
                .as_deref()
                .or(parameter.constraint.as_deref())
            else {
                continue;
            };
            if let DtsType::Native(constraint) = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            ) {
                substitution.insert(parameter.name.sym.to_string(), constraint);
            }
        }
    }
    let classify = |ty: &TsType| {
        resolve_ts_type_with_substitution(
            ty,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )
    };

    let rest_param = function.params.last().and_then(|param| {
        let TsFnParam::Rest(rest) = param else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => safe_param_name(binding.id.sym.as_ref()),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => classify(&array.elem_type),
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = function.params.len() - usize::from(rest_param.is_some());
    let required_params = function
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| !matches!(param, TsFnParam::Ident(binding) if binding.id.optional))
        .count();
    let mut param_field_constraints = Vec::new();
    let params = function
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let TsFnParam::Ident(binding) = param else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                param_field_constraints.push(None);
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = safe_param_name(binding.id.sym.as_ref());
            let ty = match &binding.type_ann {
                Some(ann) => {
                    param_field_constraints.push(field_constraints(
                        &ann.type_ann,
                        raw_interfaces,
                        local_type_aliases,
                        &mut HashSet::new(),
                    ));
                    classify(&ann.type_ann)
                }
                None => {
                    param_field_constraints.push(None);
                    DtsType::Unsupported("missing type annotation".to_string())
                }
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = classify(&function.type_ann.type_ann);

    DtsFunction {
        name,
        generic,
        params,
        param_field_constraints,
        required_params,
        rest_param,
        ret,
    }
}

/// For every top-level function declaration in `source` (see
/// `extract_fn_decls`) whose return type is a plain or namespace-
/// qualified type reference, `name -> <the reference's qualified
/// identity>` -- independent of whether that reference ever resolves
/// to a `Native` `DtsType` at all (`classify_ts_type` still calls a
/// qualified name like `dayjs.Dayjs` `Unsupported`, since qualified
/// names aren't resolved). Lets a caller elsewhere (thaw-cli's
/// shims.rs) link a Fallback factory function's return value to one of
/// this same file's own classes (`parse_dts_classes`) without widening
/// `DtsFunction`'s own already very widely constructed shape for it.
/// Real example: dayjs's `declare function dayjs(...): dayjs.Dayjs`,
/// linking the `dayjs` factory function to its `Dayjs` class.
/// Every name a package's own `.d.ts` re-exports as a self-referential
/// namespace alias for its *own* already-flattened export table -- real
/// example: zod v4's own `index.d.cts`, `import * as z from "./v4/
/// classic/external.cjs"; export * from "./v4/classic/external.cjs";
/// export { z, z as default };`. `z` (and `default`) here don't name a
/// function, class, or interface at all -- they're bound purely by the
/// `import *`, so a plain `import { z } from "zod"` (as common in real
/// zod code as `import * as z from "zod"`, since both reach the exact
/// same object) has nothing in `parse_dts`'s own function table to
/// resolve `z` against, and fails outright ("`zod` has no export named
/// `z`") even though `import * as z from "zod"` -- a genuine namespace
/// import, needing no special per-name knowledge at all -- already
/// works. Returns every such alias name (here, `["z", "default"]`) so a
/// caller (thaw-cli's `shims.rs`) can mark them for the module graph to
/// treat a *named* import of one exactly like a namespace import: the
/// whole package's own export table, not a single symbol.
///
/// Deliberately narrow: only a bare `export { X[, X as Y] };` (no
/// `from` clause -- `X` must already be bound in this same file) whose
/// original name `X` is bound by a top-level `import * as X from
/// "...";` counts. A `declare namespace X { ... }` block re-exported
/// the same way is a structurally different (and still unsupported)
/// shape -- its members are declared *inside* it, not a star-import of
/// an already-flattened sibling module -- and isn't recognized here.
pub fn self_referential_namespace_aliases(source: &str) -> HashSet<String> {
    self_referential_namespace_aliases_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn self_referential_namespace_aliases_named(source: &str, filename: &thaw_parser::common::FileName) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashSet::new();
    };
    let namespace_imports: HashSet<String> = module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
                return None;
            };
            Some(import.specifiers.iter().filter_map(|specifier| {
                match specifier {
                    swc_ecma_ast::ImportSpecifier::Namespace(namespace)
                        if import.src.value.starts_with(".") => {
                        Some(namespace.local.sym.to_string())
                    }
                    _ => None,
                }
            }))
        })
        .flatten()
        .collect();
    let named_aliases = module.body.iter().filter_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            return None;
        };
        if export.type_only || export.src.is_some() {
            return None;
        }
        Some(export.specifiers.iter().filter_map(|specifier| {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
                return None;
            };
            if named.is_type_only {
                return None;
            }
            let export_name = |name: &swc_ecma_ast::ModuleExportName| match name {
                swc_ecma_ast::ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                swc_ecma_ast::ModuleExportName::Str(_) => None,
            };
            let original = export_name(&named.orig)?;
            if !namespace_imports.contains(&original) {
                return None;
            }
            Some(
                named
                    .exported
                    .as_ref()
                    .and_then(export_name)
                    .unwrap_or(original),
            )
        }))
    }).flatten();
    // `export default X;` (a *separate* AST node from `export { X as
    // default }` above, and the shape a real npm package commonly uses
    // instead -- real example: zod v3's `lib/index.d.ts`, `import * as z
    // from "./external"; export { z }; export default z;`) is equally a
    // self-referential alias of a namespace-imported name, just under the
    // implicit name `"default"`.
    let default_alias = module.body.iter().find_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) = item else {
            return None;
        };
        let Expr::Ident(ident) = default_expr.expr.as_ref() else {
            return None;
        };
        namespace_imports
            .contains(ident.sym.as_str())
            .then(|| "default".to_string())
    });
    named_aliases.chain(default_alias).collect()
}

/// Names explicitly exported only as TypeScript types. Registry imports of
/// these names are erased at runtime but still need a local type binding.
pub fn exported_type_names(source: &str) -> HashSet<String> {
    exported_type_names_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

/// Interfaces and type aliases a declaration file exports directly
/// (`export interface X`, `export type X = ...`). They have no runtime value,
/// but an `import { X }` / `import type { X }` of one must still bind.
pub fn exported_declared_type_names_named(
    source: &str, filename: &thaw_parser::common::FileName,
) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone())
        .map(|(module, _)| module) else { return HashSet::new() };
    module.body.iter().filter_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            swc_ecma_ast::Decl::TsInterface(interface) => Some(interface.id.sym.to_string()),
            swc_ecma_ast::Decl::TsTypeAlias(alias) => Some(alias.id.sym.to_string()),
            _ => None,
        },
        _ => None,
    }).collect()
}

pub fn exported_type_names_named(source: &str, filename: &thaw_parser::common::FileName) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashSet::new();
    };
    let mut names: HashSet<String> = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => Some(export),
            _ => None,
        })
        .flat_map(|export| {
            export.specifiers.iter().filter_map(move |specifier| {
                let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
                    return None;
                };
                if !export.type_only && !named.is_type_only {
                    return None;
                }
                let name = named.exported.as_ref().unwrap_or(&named.orig);
                match name {
                    swc_ecma_ast::ModuleExportName::Ident(ident) => Some(ident.sym.to_string()),
                    swc_ecma_ast::ModuleExportName::Str(string) => {
                        string.value.as_str().map(str::to_string)
                    }
                }
            })
        })
        .collect();
    for (scope, item) in scoped_module_items(&module) {
        if scope.is_empty() { continue; }
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            if !export.type_only && !named.is_type_only { continue; }
            let swc_ecma_ast::ModuleExportName::Ident(name) =
                named.exported.as_ref().unwrap_or(&named.orig) else { continue };
            names.insert(format!("{scope}.{}", name.sym));
        }
    }
    names
}

/// Value exports in the flattened declaration, kept separate from
/// `exported_type_names`: the same class can be exported as a type through
/// one edge and as a value through another.
pub fn exported_value_names(source: &str) -> HashSet<String> {
    exported_value_names_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn exported_value_names_named(source: &str, filename: &thaw_parser::common::FileName) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashSet::new();
    };
    let mut value_bindings = HashSet::new();
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item {
            if !import.type_only {
                for specifier in &import.specifiers {
                    match specifier {
                        swc_ecma_ast::ImportSpecifier::Named(named) if !named.is_type_only => {
                            value_bindings.insert(named.local.sym.to_string());
                        }
                        swc_ecma_ast::ImportSpecifier::Default(default) => {
                            value_bindings.insert(default.local.sym.to_string());
                        }
                        swc_ecma_ast::ImportSpecifier::Namespace(namespace) => {
                            value_bindings.insert(namespace.local.sym.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        match declaration {
            Some(Decl::Class(class)) => { value_bindings.insert(class.ident.sym.to_string()); }
            Some(Decl::Fn(function)) => { value_bindings.insert(function.ident.sym.to_string()); }
            Some(Decl::TsEnum(enumeration)) => { value_bindings.insert(enumeration.id.sym.to_string()); }
            Some(Decl::TsModule(namespace)) => {
                if let swc_ecma_ast::TsModuleName::Ident(ident) = &namespace.id {
                    value_bindings.insert(ident.sym.to_string());
                }
            }
            Some(Decl::Var(variables)) => {
                for declarator in &variables.decls {
                    if let Pat::Ident(binding) = &declarator.name {
                        value_bindings.insert(binding.id.sym.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let mut names = HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let name = match &export.decl {
                    Decl::Class(class) => Some(class.ident.sym.to_string()),
                    Decl::Fn(function) => Some(function.ident.sym.to_string()),
                    Decl::TsEnum(enumeration) => Some(enumeration.id.sym.to_string()),
                    Decl::TsModule(namespace) => match &namespace.id {
                        swc_ecma_ast::TsModuleName::Ident(ident) => Some(ident.sym.to_string()),
                        _ => None,
                    },
                    Decl::Var(variables) => {
                        for declarator in &variables.decls {
                            if let Pat::Ident(binding) = &declarator.name {
                                names.insert(binding.id.sym.to_string());
                            }
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(name) = name { names.insert(name); }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only && export.src.is_none() => {
                for specifier in &export.specifiers {
                    let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
                    if named.is_type_only { continue; }
                    let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else { continue };
                    if !value_bindings.contains(original.sym.as_ref()) { continue; }
                    let name = named.exported.as_ref().unwrap_or(&named.orig);
                    if let swc_ecma_ast::ModuleExportName::Ident(ident) = name {
                        names.insert(ident.sym.to_string());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only => {
                for specifier in &export.specifiers {
                    let swc_ecma_ast::ExportSpecifier::Namespace(namespace) = specifier else { continue };
                    if let swc_ecma_ast::ModuleExportName::Ident(name) = &namespace.name {
                        names.insert(name.sym.to_string());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default))
                if matches!(&default.decl, DefaultDecl::Class(_) | DefaultDecl::Fn(_)) => {
                names.insert("default".to_string());
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(_)
                | ModuleDecl::TsExportAssignment(_)) => {
                names.insert("default".to_string());
            }
            _ => {}
        }
    }
    let scoped_bindings = scoped_module_items(&module).into_iter()
        .filter(|(scope, _)| !scope.is_empty())
        .flat_map(|(scope, item)| namespace_value_member_names(item).into_iter()
            .map(move |name| format!("{scope}.{name}")))
        .collect::<HashSet<_>>();
    for (scope, item) in scoped_module_items(&module) {
        if scope.is_empty() { continue; }
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.type_only || export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            if named.is_type_only { continue; }
            let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else { continue };
            if !scoped_bindings.contains(&format!("{scope}.{}", original.sym)) { continue; }
            let swc_ecma_ast::ModuleExportName::Ident(name) =
                named.exported.as_ref().unwrap_or(&named.orig) else { continue };
            names.insert(format!("{scope}.{}", name.sym));
        }
    }
    names
}

/// Every `declare namespace NAME { ... }` block whose members can be
/// reached as runtime values.
///
/// Two shapes are recognized; both retain their namespace-relative paths:
///
/// 1. `declare namespace NAME { export { A as B, C as D, ... }; }` --
///    an alias-marker shape accepted from existing declaration files.
///    Maps `B -> A` etc.
/// 2. A direct `declare namespace NAME { function foo(...): T;
///    var bar: ...; class Client { ... } }`, from hand-written files or
///    registry-flattened `export * as NAME` declarations. Each value
///    member is declared under its own name, so `foo -> foo`.
///
/// The result maps `NAME -> { relative member path -> runtime target }`.
/// Paths can be nested (`Outer -> Inner.make`) and the CLI preserves each
/// qualified function's own binding rather than collapsing equal bare names.
/// Names of generated, source-owned namespaces that hold only supporting
/// declaration shapes. The exact private `never` marker distinguishes them
/// from ordinary ambient namespaces, which may have runtime members.
pub fn internal_support_namespace_names_named(
    source: &str, filename: &thaw_parser::common::FileName,
) -> HashSet<String> {
    use thaw_parser::ast::{Stmt, TsModuleName};
    let Ok((module, _)) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()) else {
        return HashSet::new();
    };
    module.body.iter().filter_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(namespace))) = item else { return None };
        let TsModuleName::Ident(name) = &namespace.id else { return None };
        if !name.sym.as_ref().starts_with("__thaw_support_") { return None; }
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { return None };
        let marked = block.body.iter().any(|member| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = member else { return false };
            alias.id.sym == "__thaw_private_support_marker__"
                && matches!(alias.type_ann.as_ref(), TsType::TsKeywordType(keyword)
                    if keyword.kind == TsKeywordTypeKind::TsNeverKeyword)
        });
        marked.then(|| name.sym.to_string())
    }).collect()
}

/// Generated per-origin declaration containers are type lookup scopes, not
/// JavaScript namespace objects. Match the reserved direct marker as well
/// as the generated name; a user namespace with a similar prefix stays live.
pub fn internal_owned_namespace_names_named(
    source: &str, filename: &thaw_parser::common::FileName,
) -> HashSet<String> {
    use thaw_parser::ast::{Stmt, TsModuleName};
    let Ok((module, _)) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()) else {
        return HashSet::new();
    };
    module.body.iter().filter_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(namespace))) = item else { return None };
        let TsModuleName::Ident(name) = &namespace.id else { return None };
        let suffix = name.sym.as_ref().strip_prefix("__thaw_owned_")?;
        let expected_marker = format!("__thaw_owned_origin_marker_{suffix}");
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body else { return None };
        let marked = block.body.iter().any(|member| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = member else { return false };
            alias.id.sym.as_ref() == expected_marker
                && matches!(alias.type_ann.as_ref(), TsType::TsKeywordType(keyword)
                    if keyword.kind == TsKeywordTypeKind::TsNeverKeyword)
        });
        marked.then(|| name.sym.to_string())
    }).collect()
}

pub fn nested_namespace_members(source: &str) -> HashMap<String, HashMap<String, String>> {
    nested_namespace_members_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn nested_namespace_members_named(source: &str, filename: &thaw_parser::common::FileName) -> HashMap<String, HashMap<String, String>> {
    use thaw_parser::ast::{ExportSpecifier, ModuleExportName, Stmt, TsModuleName};

    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashMap::new();
    };
    let mut internal_support = internal_support_namespace_names_named(source, filename);
    internal_support.extend(internal_owned_namespace_names_named(source, filename));
    let type_only_namespaces = type_only_namespace_names_named(source, filename);
    let mut namespaces: HashMap<String, HashMap<String, String>> = HashMap::new();
    for item in &module.body {
        let module_decl = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) => module_decl,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::TsModule(module_decl) => module_decl,
                _ => continue,
            },
            _ => continue,
        };
        let TsModuleName::Ident(name) = &module_decl.id else {
            continue;
        };
        if internal_support.contains(name.sym.as_ref()) { continue; }
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
            continue;
        };
        let members = namespaces.entry(name.sym.to_string()).or_default();
        fn collect_members(
            body: &[ModuleItem], scope: &str, root: &str,
            type_only: &HashSet<String>, members: &mut HashMap<String, String>,
        ) {
            for member in body {
                match member {
                    ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) if import.is_export => {
                        let swc_ecma_ast::TsModuleRef::TsEntityName(target) = &import.module_ref else { continue };
                        let member_name = if scope.is_empty() { import.id.sym.to_string() }
                            else { format!("{scope}.{}", import.id.sym) };
                        members.insert(member_name, type_reference_name(target));
                    }
                    ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(nested)))
                    | ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(swc_ecma_ast::ExportDecl { decl: Decl::TsModule(nested), .. })) => {
                        let TsModuleName::Ident(name) = &nested.id else { continue; };
                        let Some(TsNamespaceBody::TsModuleBlock(block)) = &nested.body else { continue; };
                        let child = if scope.is_empty() { name.sym.to_string() }
                            else { format!("{scope}.{}", name.sym) };
                        if type_only.contains(&format!("{root}.{child}")) { continue; }
                        collect_members(&block.body, &child, root, type_only, members);
                    }
                    ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                        if export.type_only || export.src.is_some() {
                            continue;
                        }
                        for specifier in &export.specifiers {
                            let ExportSpecifier::Named(named) = specifier else { continue; };
                            if named.is_type_only { continue; }
                            let export_name = |name: &ModuleExportName| match name {
                                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                                ModuleExportName::Str(_) => None,
                            };
                            let Some(target) = export_name(&named.orig) else { continue; };
                            let member_name = named.exported.as_ref().and_then(export_name)
                                .unwrap_or_else(|| target.clone());
                            let member_name = if scope.is_empty() { member_name }
                                else { format!("{scope}.{member_name}") };
                            members.insert(member_name, target);
                        }
                    }
                    // Types/interfaces have no runtime binding.
                    other => {
                        for member_name in namespace_value_member_names(other) {
                            let path = if scope.is_empty() { member_name.clone() }
                                else { format!("{scope}.{member_name}") };
                            members.entry(path.clone()).or_insert(path);
                        }
                    }
                }
            }
        }
        collect_members(&block.body, "", name.sym.as_ref(), &type_only_namespaces, members);
    }
    for (original, public) in namespace_value_aliases(&module) {
        if let Some(members) = namespaces.get(&original).cloned() {
            namespaces.entry(public).or_default().extend(members);
        }
    }
    for original in nonpublic_namespace_sources_named(source, filename) {
        namespaces.remove(&original);
    }
    namespaces
}

/// The runtime value name(s) a single `declare namespace` body item
/// declares, for [`nested_namespace_members`]'s hand-written-namespace
/// case. `export { ... }` lists are handled separately by the caller.
fn namespace_value_member_names(item: &ModuleItem) -> Vec<String> {
    use thaw_parser::ast::{Decl, Stmt};
    fn from_decl(declaration: &Decl) -> Vec<String> {
        match declaration {
            Decl::Fn(function) => vec![function.ident.sym.to_string()],
            Decl::Class(class) => vec![class.ident.sym.to_string()],
            Decl::Var(variable) => variable
                .decls
                .iter()
                .filter_map(|declarator| match &declarator.name {
                    Pat::Ident(binding) => Some(binding.id.sym.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    match item {
        ModuleItem::Stmt(Stmt::Decl(declaration)) => from_decl(declaration),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => from_decl(&export.decl),
        _ => Vec::new(),
    }
}

/// Follows a bare (non-generic-substituting) type alias chain from
/// `name` down to the first name that isn't itself an alias -- e.g.
/// `type Transporter<T, D> = Mail<T, D>;` resolves `"Transporter"` to
/// `"Mail"`, ignoring the alias's own type parameters and the concrete
/// arguments the reference supplies (this only ever needs the bare
/// class *name* a factory function's return type ultimately names, not
/// a real generic substitution). `visited` guards a self-referential or
/// mutually-referential alias cycle. Real example: nodemailer's own
/// `createTransport(...): Transporter<...>`, where `Transporter` is
/// this exact generic alias and the class doing the real work
/// (`Mail`, with `sendMail`) lives only under its own name.
fn resolve_bare_type_alias_chain(
    name: &str,
    aliases: &HashMap<String, String>,
    visited: &mut HashSet<String>,
) -> String {
    if !visited.insert(name.to_string()) {
        return name.to_string();
    }
    match aliases.get(name) {
        Some(target) => resolve_bare_type_alias_chain(target, aliases, visited),
        None => name.to_string(),
    }
}

pub fn function_return_named_types(source: &str) -> HashMap<String, String> {
    function_return_named_types_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

pub fn function_return_named_types_named(source: &str, filename: &thaw_parser::common::FileName) -> HashMap<String, String> {
    let Ok(module) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).map(|(module, _)| module) else {
        return HashMap::new();
    };
    let type_only_namespaces = type_only_namespace_names_named(source, filename);
    let export_assignment = export_assignment_namespace(&module);
    let class_names = scoped_class_names(&module).into_iter()
        .map(|(qualified, _)| qualified).collect::<HashSet<_>>();
    // `type Alias<T, ...> = Target<...>;` -> `Alias -> Target`, bare
    // names only (see `resolve_bare_type_alias_chain`'s own doc comment).
    let type_aliases: HashMap<String, String> = module
        .body
        .iter()
        .flat_map(extract_type_alias_decls)
        .filter_map(|alias| {
            let TsType::TsTypeRef(ty_ref) = alias.type_ann.as_ref() else {
                return None;
            };
            let target = type_reference_name(&ty_ref.type_name);
            Some((alias.id.sym.to_string(), target))
        })
        .collect();
    let mut returns = scoped_fn_decls(&module)
        .into_iter()
        .filter(|(full_name, _, _)| !namespace_member_is_type_only(full_name, &type_only_namespaces))
        .filter_map(|(full_name, _, function)| {
            let ann = function.return_type.as_ref()?;
            let TsType::TsTypeRef(ty_ref) = ann.type_ann.as_ref() else {
                return None;
            };
            let type_name = type_reference_name(&ty_ref.type_name);
            let type_name =
                resolve_bare_type_alias_chain(&type_name, &type_aliases, &mut HashSet::new());
            let type_name = lexical_type_key(&type_name, declaration_scope(&full_name),
                |candidate| class_names.contains(candidate)).unwrap_or(type_name);
            let name = export_assignment
                .and_then(|namespace| full_name.strip_prefix(format!("{namespace}.").as_str()))
                .map(str::to_string)
                .unwrap_or(full_name);
            Some((name, type_name))
        })
        .collect::<HashMap<_, _>>();
    for (original, public) in namespace_value_aliases(&module) {
        let prefix = format!("{original}.");
        let aliases = returns.iter().filter_map(|(name, class)| {
            name.strip_prefix(&prefix).map(|suffix| {
                let class = class.strip_prefix(&prefix)
                    .map(|class_suffix| format!("{public}.{class_suffix}"))
                    .unwrap_or_else(|| class.clone());
                (format!("{public}.{suffix}"), class)
            })
        }).collect::<Vec<_>>();
        returns.extend(aliases);
    }
    let hidden = nonpublic_namespace_sources_named(source, filename);
    returns.retain(|name, _| !namespace_member_is_type_only(name, &hidden));
    returns
}
