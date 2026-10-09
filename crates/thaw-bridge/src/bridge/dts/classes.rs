pub fn parse_dts_classes(source: &str) -> Result<Vec<DtsClass>, String> {
    parse_dts_classes_named(source, &thaw_parser::common::FileName::Custom("input.ts".into()))
}

// An emitted public namespace may alias a fully-qualified declaration in a
// generated per-origin type container. Keep the source path intact; taking
// only its final identifier would bind a colliding class from another file.
fn scoped_exported_import_equals_aliases(module: &Module) -> Vec<(String, String)> {
    use swc_ecma_ast::{TsModuleRef};
    scoped_module_items(module).into_iter().filter_map(|(scope, item)| {
        let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else { return None };
        if !import.is_export { return None; }
        let TsModuleRef::TsEntityName(target) = &import.module_ref else { return None };
        let source = type_reference_name(target);
        let alias = if scope.is_empty() { import.id.sym.to_string() }
            else { format!("{scope}.{}", import.id.sym) };
        Some((source, alias))
    }).collect()
}

pub fn parse_dts_classes_named(
    source: &str, filename: &thaw_parser::common::FileName,
) -> Result<Vec<DtsClass>, String> {
    let module = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone())?.0;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let scoped_class_decls = scoped_class_decls(&module);
    let scoped_classes = scoped_class_decls.iter().map(|(qualified, bare, _)|
        (qualified.clone(), bare.clone())).collect::<Vec<_>>();
    let mut locations = HashMap::<String, HashSet<String>>::new();
    for (qualified, bare) in &scoped_classes {
        locations.entry(bare.clone()).or_default().insert(qualified.clone());
    }
    let mut instance_class_names = HashMap::<String, String>::new();
    for (qualified, _) in &scoped_classes {
        instance_class_names.insert(qualified.clone(), qualified.clone());
    }
    for (bare, scopes) in &locations {
        if scopes.contains(bare) {
            instance_class_names.insert(bare.clone(), bare.clone());
        } else if scopes.len() == 1 {
            instance_class_names.insert(bare.clone(), scopes.iter().next().unwrap().clone());
        }
    }
    for (alias, target) in class_constructor_aliases(&module) {
        if instance_class_names.contains_key(&target) {
            instance_class_names.insert(alias.clone(), alias);
        }
    }
    let mut declared_type_names = scoped_classes.iter().map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    declared_type_names.extend(class_constructor_aliases(&module).into_iter().map(|(alias, _)| alias));
    let (type_interfaces, type_aliases) = scoped_type_declarations(&module);
    for name in type_interfaces.into_iter().map(|(name, _)| name)
        .chain(type_aliases.into_iter().map(|(name, _)| name))
    {
        declared_type_names.insert(name.clone());
        if !name.contains('.') && !scoped_classes.iter().any(|(class, _)| class == &name) {
            instance_class_names.remove(&name);
        }
    }
    let mut classes = scoped_class_decls.iter().map(|(qualified, _name, class)| {
        let (context, generic_context) = scoped_type_context(
            declaration_scope(qualified), &interfaces, &generic_interfaces,
        );
        lower_dts_class(qualified, class, &context, &generic_context, &instance_class_names,
            declaration_scope(qualified), &declared_type_names)
    })
        .collect::<Vec<_>>();
    let declared = classes
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect::<HashMap<_, _>>();
    for class in &mut classes {
        if class.constructors.is_empty() {
            class.constructors = inherited_class_constructors(class, &declared, &mut Vec::new());
        }
        let (methods, properties) = inherited_class_members(class, &declared, &mut Vec::new());
        class.methods = methods;
        class.properties = properties;
    }
    for class in &mut classes {
        let constructor_overloaded = class.constructors.len() > 1;
        for constructor in &mut class.constructors {
            constructor.overloaded = constructor_overloaded;
        }
        let mut counts = HashMap::<(String, bool), usize>::new();
        for method in &class.methods {
            *counts
                .entry((method.name.clone(), method.is_static))
                .or_default() += 1;
        }
        for method in &mut class.methods {
            method.overloaded = counts[&(method.name.clone(), method.is_static)] > 1;
        }
    }
    let aliases = class_constructor_aliases(&module);
    for (alias, target) in aliases {
        if classes.iter().any(|class| class.name == alias) {
            continue;
        }
        if let Some(mut class) = classes.iter().find(|class| class.name == target).cloned() {
            class.name = alias;
            classes.push(class);
        }
    }
    for class in constructor_interface_classes(&module, &interfaces, &generic_interfaces) {
        if !classes.iter().any(|existing| existing.name == class.name) {
            classes.push(class);
        }
    }
    for class in self_constructible_interface_classes(
        source,
        filename,
        &module,
        &interfaces,
        &generic_interfaces,
    ) {
        if !classes.iter().any(|existing| existing.name == class.name) {
            classes.push(class);
        }
    }
    // Flattened barrels keep a declaration's valid local identifier and
    // expose its public property through an export specifier. Carry the
    // class shape under that public name for both value and type aliases.
    let mut generated_internals = HashSet::new();
    let mut aliases = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            let swc_ecma_ast::ModuleExportName::Ident(original) = &named.orig else { continue };
            let Some(swc_ecma_ast::ModuleExportName::Ident(public)) = &named.exported else { continue };
            if original.sym == public.sym { continue; }
            aliases.push((original.sym.to_string(), public.sym.to_string()));
            if original.sym.as_str().starts_with("__thaw_public_") {
                generated_internals.insert(original.sym.to_string());
            }
        }
    }
    for (scope, item) in scoped_module_items(&module) {
        if scope.is_empty() { continue; }
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else { continue };
        if export.src.is_some() { continue; }
        for specifier in &export.specifiers {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else { continue };
            let (swc_ecma_ast::ModuleExportName::Ident(original),
                Some(swc_ecma_ast::ModuleExportName::Ident(public))) =
                (&named.orig, &named.exported) else { continue };
            if original.sym == public.sym { continue; }
            let source_name = format!("{scope}.{}", original.sym);
            let public_name = format!("{scope}.{}", public.sym);
            aliases.push((source_name.clone(), public_name));
            if original.sym.as_str().starts_with("__thaw_public_") {
                generated_internals.insert(source_name);
            }
        }
    }
    aliases.extend(scoped_exported_import_equals_aliases(&module));
    loop {
        let mut changed = false;
        for (original, public) in &aliases {
            if classes.iter().any(|class| &class.name == public) { continue; }
            if let Some(mut class) = classes.iter().find(|class| &class.name == original).cloned() {
                let original_name = class.name.clone();
                class.name = public.clone();
                for method in &mut class.methods {
                    if method.return_instance_class.as_deref() == Some(original_name.as_str()) {
                        method.return_instance_class = Some(class.name.clone());
                    }
                    for callback in &mut method.callback_instance_classes {
                        for identity in callback {
                            if identity.as_deref() == Some(original_name.as_str()) {
                                *identity = Some(class.name.clone());
                            }
                        }
                    }
                }
                classes.push(class);
                changed = true;
            }
        }
        if !changed { break; }
    }
    for (original, public) in namespace_value_aliases(&module).into_iter()
        .chain(scoped_exported_import_equals_aliases(&module)) {
        let prefix = format!("{original}.");
        let aliases = classes.iter().filter_map(|class| {
            let suffix = class.name.strip_prefix(&prefix)?;
            let mut alias = class.clone();
            alias.name = format!("{public}.{suffix}");
            let retarget = |identity: &mut String| {
                if let Some(suffix) = identity.strip_prefix(&prefix) {
                    *identity = format!("{public}.{suffix}");
                }
            };
            if let Some(parent) = &mut alias.extends { retarget(parent); }
            for method in &mut alias.methods {
                if let Some(returned) = &mut method.return_instance_class { retarget(returned); }
                for callback in &mut method.callback_instance_classes {
                    for identity in callback.iter_mut().flatten() { retarget(identity); }
                }
            }
            Some(alias)
        }).collect::<Vec<_>>();
        classes.extend(aliases);
    }
    classes.retain(|class| !generated_internals.contains(&class.name));
    // Flattened declaration files can repeat the same qualified binding.
    // Keep the first declaration for each binding without collapsing
    // distinct namespace classes that happen to share a bare name.
    let mut seen_names = HashSet::new();
    classes.retain(|class| seen_names.insert(class.name.clone()));
    Ok(classes)
}

/// A bare `declare const X: SomeInterface` whose `SomeInterface` itself
/// carries a `new (...): T` construct signature (real trigger:
/// better-sqlite3's own `declare const Database: BetterSqlite3.
/// DatabaseConstructor`, its entire default export) is a callable
/// *interface value*, not a `class` declaration -- `extract_class_decls`
/// never sees it, so it never became `constructible`, and `new Database
/// (...)` fell through every real constructor path to a plain-call
/// fallback that drops `new`/`new.target` semantics. Synthesizes a
/// minimal `DtsClass` directly from the one construct signature so it
/// gets the same `constructible` treatment (and the same real
/// `$new$`-prefixed native constructor wrapper,
/// `shim_support.rs`'s `generate_napi_class_constructors`) an ordinary
/// `declare class` already does. Only the constructor is synthesized --
/// the instance type's own methods/properties (e.g. `Database.exec()`)
/// are resolved separately, through whatever already handles an ordinary
/// interface-typed Fallback value's own property/method access.
fn constructor_interface_classes(
    module: &Module,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Vec<DtsClass> {
    use swc_ecma_ast::{Expr, TsLit};
    let mut interface_decls: HashMap<String, Vec<&TsInterfaceDecl>> = HashMap::new();
    for (name, iface) in scoped_type_declarations(module).0 {
        interface_decls.entry(name).or_default().push(iface);
    }
    fn arguments(
        reference: &swc_ecma_ast::TsTypeRef,
        interface: &TsInterfaceDecl,
        outer: &HashMap<String, HirType>,
        interfaces: &HashMap<String, DtsType>,
        generic_interfaces: &GenericInterfaces,
    ) -> Option<HashMap<String, HirType>> {
        let parameters = interface.type_params.as_ref().map(|params|
            params.params.as_slice()).unwrap_or(&[]);
        let arguments = reference.type_params.as_ref().map(|params|
            params.params.as_slice()).unwrap_or(&[]);
        if arguments.len() > parameters.len() { return None; }
        let mut substitution = outer.clone();
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = arguments.get(index).map(|value| value.as_ref())
                .or_else(|| parameter.default.as_deref())?;
            let DtsType::Native(value) = resolve_ts_type_with_substitution(
                argument, &substitution, interfaces, generic_interfaces, &mut Vec::new(),
            ) else { return None };
            substitution.insert(parameter.name.sym.to_string(), value);
        }
        Some(substitution)
    }

    let var_decls = scoped_module_items(module).into_iter().filter_map(|(scope, item)| match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(declaration))) => {
            Some((scope, declaration.as_ref()))
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Var(declaration) => Some((scope, declaration.as_ref())),
            _ => None,
        },
        _ => None,
    });

    var_decls
        .flat_map(|(scope, declaration)| declaration.decls.iter().map(move |declarator| (scope.clone(), declarator)))
        .filter_map(|(scope, declarator)| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            let annotation = binding.type_ann.as_ref()?;
            let (reference, lookup_scope, outer) = match annotation.type_ann.as_ref() {
                TsType::TsTypeRef(reference) =>
                    (reference, scope.clone(), HashMap::new()),
                TsType::TsIndexedAccessType(indexed) => {
                    let TsType::TsTypeRef(object) = indexed.obj_type.as_ref() else { return None };
                    let TsType::TsLitType(index) = indexed.index_type.as_ref() else { return None };
                    let TsLit::Str(index) = &index.lit else { return None };
                    let object_name = lexical_type_key(
                        &type_reference_name(&object.type_name), &scope,
                        |name| interface_decls.contains_key(name),
                    )?;
                    let object_decls = interface_decls.get(&object_name)?;
                    let outer = arguments(
                        object, object_decls[0], &HashMap::new(), interfaces, generic_interfaces,
                    )?;
                    let properties = object_decls.iter().flat_map(|interface|
                        interface.body.body.iter()).filter_map(|member| {
                        let TsTypeElement::TsPropertySignature(property) = member else {
                            return None;
                        };
                        let Expr::Ident(key) = property.key.as_ref() else { return None };
                        if key.sym.as_ref() != index.value.to_string_lossy().as_ref() {
                            return None;
                        }
                        let TsType::TsTypeRef(reference) =
                            property.type_ann.as_ref()?.type_ann.as_ref() else { return None };
                        Some(reference)
                    }).collect::<Vec<_>>();
                    let property = *properties.first()?;
                    let signature = |reference: &swc_ecma_ast::TsTypeRef| (
                        type_reference_name(&reference.type_name),
                        reference.type_params.as_ref().map(|arguments| arguments.params.iter()
                            .map(|argument| describe_ts_type(argument)).collect::<Vec<_>>()),
                    );
                    if properties.iter().skip(1).any(|other|
                        signature(other) != signature(property)) { return None; }
                    (property, declaration_scope(&object_name).to_string(), outer)
                }
                _ => return None,
            };
            let interface_name = lexical_type_key(
                &type_reference_name(&reference.type_name), &lookup_scope,
                |name| interface_decls.contains_key(name),
            )?;
            let declarations = interface_decls.get(&interface_name)?;
            let substitution = arguments(
                reference, declarations[0], &outer, interfaces, generic_interfaces,
            )?;
            let name = if scope.is_empty() { binding.id.sym.to_string() }
                else { format!("{scope}.{}", binding.id.sym) };
            let (context, generic_context) = scoped_type_context(
                declaration_scope(&interface_name), interfaces, generic_interfaces,
            );
            let constructors = declarations.iter().flat_map(|interface|
                interface.body.body.iter()).filter_map(|member| {
                let TsTypeElement::TsConstructSignatureDecl(signature) = member else {
                    return None;
                };
                let synthetic_fn_type = TsFnType {
                    span: signature.span,
                    params: signature.params.clone(),
                    type_params: signature.type_params.clone(),
                    type_ann: signature.type_ann.clone()?,
                };
                let mut signature_substitution = substitution.clone();
                if let Some(parameters) = &signature.type_params {
                    for parameter in &parameters.params {
                        signature_substitution.remove(parameter.name.sym.as_ref());
                    }
                }
                let mut function = lower_dts_fn_type(
                    &name, &synthetic_fn_type, &context, &generic_context,
                    &HashMap::new(), &HashMap::new(),
                );
                if !signature_substitution.is_empty() {
                    for ((_, classified), parameter) in function.params.iter_mut()
                        .zip(&synthetic_fn_type.params) {
                        let swc_ecma_ast::TsFnParam::Ident(binding) = parameter else { continue };
                        let Some(annotation) = &binding.type_ann else { continue };
                        *classified = resolve_ts_type_with_substitution(
                            &annotation.type_ann, &signature_substitution, &context, &generic_context,
                            &mut Vec::new(),
                        );
                    }
                    if let (Some((_, classified)), Some(swc_ecma_ast::TsFnParam::Rest(rest))) =
                        (function.rest_param.as_mut(), synthetic_fn_type.params.last()) {
                        if let Some(annotation) = &rest.type_ann {
                            if let TsType::TsArrayType(array) = annotation.type_ann.as_ref() {
                                *classified = resolve_ts_type_with_substitution(
                                    &array.elem_type, &signature_substitution, &context, &generic_context,
                                    &mut Vec::new(),
                                );
                            }
                        }
                    }
                }
                Some(DtsConstructor {
                    params: function.params,
                    required_params: function.required_params,
                    rest_param: function.rest_param,
                    overloaded: false,
                })
            }).collect::<Vec<_>>();
            if constructors.is_empty() {
                return None;
            }
            let overloaded = constructors.len() > 1;
            let constructors = constructors.into_iter().map(|mut constructor| {
                constructor.overloaded = overloaded;
                constructor
            }).collect();
            Some(DtsClass {
                name,
                extends: None,
                constructible: true,
                constructors,
                methods: Vec::new(),
                properties: Vec::new(),
            })
        })
        .collect()
}

/// An `interface X { new (...): Y }` that is only ever reached through a
/// *namespace member alias* (real trigger: winston's own `declare
/// namespace transports { export { ConsoleTransportInstance as Console,
/// ... } }`, where `ConsoleTransportInstance` carries the construct
/// signature) has no `declare class` and no top-level `declare const`
/// binding it -- `extract_class_decls` only sees `class`, and
/// `constructor_interface_classes` only sees a top-level
/// `const X: SomeInterface` -- so `new transports.Console(...)` never
/// reached a real constructor. Synthesizes the same constructible shape
/// from the interface's own construct signature. The real runtime value
/// is bound as a JS global under the interface's own type name by
/// `nested_namespace_aliases` (`wrap_as_commonjs_module` reads
/// `module.exports.<namespace>.<member>`), which is exactly the name
/// `thaw_js_get_global` looks up for a Fallback `$new$` constructor.
fn self_constructible_interface_classes(
    source: &str,
    filename: &thaw_parser::common::FileName,
    module: &Module,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Vec<DtsClass> {
    let namespace_members = nested_namespace_members_named(source, filename);
    if namespace_members.is_empty() {
        return Vec::new();
    }
    let mut scoped = HashMap::<String, Vec<&TsInterfaceDecl>>::new();
    for (name, interface) in scoped_type_declarations(module).0 {
        scoped.entry(name).or_default().push(interface);
    }
    let mut targets = std::collections::BTreeSet::<String>::new();
    for (namespace, members) in namespace_members {
        for target in members.into_values() {
            if let Some(key) = lexical_type_key(&target, &namespace, |name| scoped.contains_key(name)) {
                targets.insert(key);
            }
        }
    }
    targets.into_iter().filter_map(|name| {
            let interfaces_for_name = scoped.get(&name)?;
            let (context, generic_context) = scoped_type_context(
                declaration_scope(&name), interfaces, generic_interfaces,
            );
            let constructors = interfaces_for_name.iter()
                .flat_map(|interface| interface.body.body.iter())
                .filter_map(|member| {
                let TsTypeElement::TsConstructSignatureDecl(signature) = member else {
                    return None;
                };
                let synthetic_fn_type = TsFnType {
                    span: signature.span,
                    params: signature.params.clone(),
                    type_params: signature.type_params.clone(),
                    type_ann: signature.type_ann.clone()?,
                };
                let function = lower_dts_fn_type(
                    &name, &synthetic_fn_type, &context, &generic_context,
                    &HashMap::new(), &HashMap::new(),
                );
                Some(DtsConstructor {
                    params: function.params,
                    required_params: function.required_params,
                    rest_param: function.rest_param,
                    overloaded: false,
                })
            }).collect::<Vec<_>>();
            if constructors.is_empty() {
                return None;
            }
            let overloaded = constructors.len() > 1;
            let constructors = constructors.into_iter().map(|mut constructor| {
                constructor.overloaded = overloaded;
                constructor
            }).collect();
            Some(DtsClass {
                name,
                extends: None,
                constructible: true,
                constructors,
                methods: Vec::new(),
                properties: Vec::new(),
            })
        })
        .collect()
}

/// TypeScript packages often publish a private class through a public
/// constructor-valued constant (`export const Public: typeof Internal`).
/// Treat that value as the same class shape under its runtime export name.
fn class_constructor_aliases(module: &Module) -> Vec<(String, String)> {
    use swc_ecma_ast::{Expr, TsLit, TsTypeElement};
    let mut interfaces: HashMap<String, Vec<&TsInterfaceDecl>> = HashMap::new();
    for (name, interface) in scoped_type_declarations(module).0 {
        interfaces.entry(name).or_default().push(interface);
    }
    fn query_target(annotation: &TsType) -> Option<String> {
        let TsType::TsTypeQuery(query) = annotation else { return None };
        let swc_ecma_ast::TsTypeQueryExpr::TsEntityName(target) = &query.expr_name else {
            return None;
        };
        Some(type_reference_name(target))
    }
    scoped_module_items(module)
        .into_iter()
        .filter_map(|(scope, item)| match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(declaration))) => {
                Some((scope, declaration.as_ref()))
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Var(declaration) => Some((scope, declaration.as_ref())),
                _ => None,
            },
            _ => None,
        })
        .flat_map(|(scope, declaration)| declaration.decls.iter().map(move |declarator| (scope.clone(), declarator)))
        .filter_map(|(scope, declarator)| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            let annotation = binding.type_ann.as_ref()?.type_ann.as_ref();
            let target = if let Some(target) = query_target(annotation) {
                target
            } else {
                // A generic export-assignment object may expose a
                // constructor as Members<T>["ctor"]. Recover only a
                // property whose declared annotation is explicitly typeof.
                let TsType::TsIndexedAccessType(indexed) = annotation else { return None };
                let TsType::TsTypeRef(object) = indexed.obj_type.as_ref() else { return None };
                let TsType::TsLitType(index) = indexed.index_type.as_ref() else { return None };
                let TsLit::Str(index) = &index.lit else { return None };
                let interface_name = lexical_type_key(
                    &type_reference_name(&object.type_name), &scope,
                    |name| interfaces.contains_key(name),
                )?;
                let mut targets = interfaces.get(&interface_name)?.iter()
                    .flat_map(|interface| interface.body.body.iter())
                    .filter_map(|member| {
                        let TsTypeElement::TsPropertySignature(property) = member else { return None };
                        let Expr::Ident(key) = property.key.as_ref() else { return None };
                        if key.sym.as_ref() != index.value.to_string_lossy().as_ref() {
                            return None;
                        }
                        query_target(property.type_ann.as_ref()?.type_ann.as_ref())
                    });
                let target = targets.next()?;
                if targets.any(|other| other != target) { return None; }
                target
            };
            let alias = if scope.is_empty() { binding.id.sym.to_string() }
                else { format!("{scope}.{}", binding.id.sym) };
            Some((alias, target))
        })
        .collect()
}

fn inherited_class_constructors(
    class: &DtsClass,
    declared: &HashMap<String, DtsClass>,
    in_progress: &mut Vec<String>,
) -> Vec<DtsConstructor> {
    if !class.constructors.is_empty() || in_progress.contains(&class.name) {
        return class.constructors.clone();
    }
    in_progress.push(class.name.clone());
    let constructors = class
        .extends
        .as_ref()
        .and_then(|base| declared.get(base))
        .map(|base| inherited_class_constructors(base, declared, in_progress))
        .unwrap_or_default();
    in_progress.pop();
    constructors
}

fn inherited_class_members(
    class: &DtsClass,
    declared: &HashMap<String, DtsClass>,
    in_progress: &mut Vec<String>,
) -> (Vec<DtsMethod>, Vec<DtsProperty>) {
    if in_progress.contains(&class.name) {
        return (class.methods.clone(), class.properties.clone());
    }
    in_progress.push(class.name.clone());
    let (mut methods, mut properties) = class
        .extends
        .as_ref()
        .and_then(|base| declared.get(base))
        .map(|base| inherited_class_members(base, declared, in_progress))
        .unwrap_or_default();
    in_progress.pop();

    let shadows = |name: &str, is_static: bool| {
        class
            .methods
            .iter()
            .any(|member| member.name == name && member.is_static == is_static)
            || class
                .properties
                .iter()
                .any(|member| member.name == name && member.is_static == is_static)
    };
    methods.retain(|member| !shadows(&member.name, member.is_static));
    properties.retain(|member| !shadows(&member.name, member.is_static));
    methods.extend(class.methods.clone());
    properties.extend(class.properties.clone());
    (methods, properties)
}

/// Like `extract_fn_decls`/`extract_fn_decls_from_decl`, but for a class
/// declaration -- including one nested inside a `declare namespace X {
/// ... }` block, real-world example: dayjs's own `Dayjs` class, declared
/// inside `declare namespace dayjs { class Dayjs {...} } ` rather than
/// at the top level (its factory function `dayjs(...)`, by contrast, is
/// a genuinely top-level `declare function`). Without this, a
/// namespace-nested class was silently invisible to `parse_dts_classes`
/// entirely -- structurally parseable as a *type* (an ordinary
/// `TsTypeRef` resolves it via `resolve_interfaces` same as a top-level
/// one), but never bridgeable as a *class* since nothing here ever
/// extracted its own methods/constructors.
fn extract_class_decls(item: &ModuleItem) -> Vec<(&str, &Class)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_class_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_class_decls_from_decl(&export.decl)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Class(class) => vec![(
                class.ident.as_ref().map(|ident| ident.sym.as_str()).unwrap_or("default"),
                class.class.as_ref(),
            )],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_class_decls_from_decl(decl: &Decl) -> Vec<(&str, &Class)> {
    match decl {
        Decl::Class(class) => vec![(class.ident.sym.as_str(), &class.class)],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_class_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn scoped_class_decls<'a>(module: &'a Module) -> Vec<(String, String, &'a Class)> {
    fn walk<'a>(item: &'a ModuleItem, scope: &str, names: &mut Vec<(String, String, &'a Class)>) {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) = item {
            if let DefaultDecl::Class(class) = &export.decl {
                let bare = class.ident.as_ref().map_or("default", |ident| ident.sym.as_str());
                names.push((bare.to_string(), bare.to_string(), class.class.as_ref()));
            }
            return;
        }
        let decl = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => decl,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
            _ => return,
        };
        match decl {
            Decl::Class(class) => {
                let bare = class.ident.sym.to_string();
                let qualified = if scope.is_empty() { bare.clone() } else { format!("{scope}.{bare}") };
                names.push((qualified, bare, &class.class));
            }
            Decl::TsModule(namespace) => {
                let swc_ecma_ast::TsModuleName::Ident(name) = &namespace.id else { return; };
                let scope = if scope.is_empty() {
                    name.sym.to_string()
                } else {
                    format!("{scope}.{}", name.sym)
                };
                if let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body {
                    for item in &block.body { walk(item, &scope, names); }
                }
            }
            _ => {}
        }
    }
    let mut names = Vec::new();
    for item in &module.body { walk(item, "", &mut names); }
    names
}

fn scoped_class_names(module: &Module) -> Vec<(String, String)> {
    scoped_class_decls(module).into_iter().map(|(qualified, bare, _)| (qualified, bare)).collect()
}

fn property_name(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(name) => Some(name.sym.to_string()),
        PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
        PropName::Num(name) => Some(name.value.to_string()),
        _ => None,
    }
}

fn type_property_name(key: &Expr) -> Option<String> {
    match key {
        Expr::Ident(name) => Some(name.sym.to_string()),
        Expr::Lit(swc_ecma_ast::Lit::Str(name)) => {
            Some(name.value.to_string_lossy().into_owned())
        }
        Expr::Lit(swc_ecma_ast::Lit::Num(name)) => Some(name.value.to_string()),
        _ => None,
    }
}

fn named_return_type(
    annotation: Option<&swc_ecma_ast::TsTypeAnn>,
    self_name: &str,
    instance_class_names: &HashMap<String, String>,
    scope: &str,
    declared_type_names: &HashSet<String>,
    type_params: &HashSet<String>,
) -> Option<String> {
    match annotation?.type_ann.as_ref() {
        // A fluent `this` return -- `name(str): this` on commander's
        // `Command`, and every builder API like it. It's the same class,
        // so route it through the existing named-instance-return path
        // (`return_instance_class` -> a `JsValue` handle, chainable) the
        // same as an explicit `foo(): Command` would.
        TsType::TsThisType(_) => Some(self_name.to_string()),
        TsType::TsTypeRef(reference) => {
            let reference = type_reference_name(&reference.type_name);
            if type_params.contains(&reference) {
                None
            } else {
                let key = lexical_type_key(&reference, scope, |name| declared_type_names.contains(name))?;
                instance_class_names.get(&key).cloned()
            }
        }
        _ => None,
    }
}

fn index_signature_value(signature: &swc_ecma_ast::TsIndexSignature) -> Result<&TsType, String> {
    let [TsFnParam::Ident(key)] = signature.params.as_slice() else {
        return Err("index signature requires one identifier key".into());
    };
    let key_type = key
        .type_ann
        .as_ref()
        .ok_or("index signature key needs a type annotation")?;
    if !matches!(
        key_type.type_ann.as_ref(),
        TsType::TsKeywordType(keyword)
            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
    ) {
        return Err("native dictionary index signatures require a string key".into());
    }
    signature
        .type_ann
        .as_ref()
        .map(|annotation| annotation.type_ann.as_ref())
        .ok_or_else(|| "index signature needs a value type annotation".into())
}

fn lower_class_params(
    params: &[ParamOrTsParamProp],
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Vec<(String, DtsType)> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let binding = match param {
                ParamOrTsParamProp::Param(param) => match &param.pat {
                    Pat::Ident(binding) => Some(binding),
                    Pat::Assign(assign) => match assign.left.as_ref() {
                        Pat::Ident(binding) => Some(binding),
                        _ => None,
                    },
                    _ => None,
                },
                ParamOrTsParamProp::TsParamProp(property) => match &property.param {
                    TsParamPropParam::Ident(binding) => Some(binding),
                    TsParamPropParam::Assign(assign) => match assign.left.as_ref() {
                        Pat::Ident(binding) => Some(binding),
                        _ => None,
                    },
                },
            };
            let Some(binding) = binding else {
                return (
                    format!("arg{index}"),
                    DtsType::Unsupported("unsupported constructor parameter pattern".into()),
                );
            };
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
            (safe_param_name(binding.id.sym.as_ref()), ty)
        })
        .collect()
}

fn class_param_is_required(param: &ParamOrTsParamProp) -> bool {
    match param {
        ParamOrTsParamProp::Param(param) => match &param.pat {
            Pat::Ident(binding) => !binding.optional,
            Pat::Assign(_) | Pat::Rest(_) => false,
            _ => true,
        },
        ParamOrTsParamProp::TsParamProp(property) => match &property.param {
            TsParamPropParam::Ident(binding) => !binding.optional,
            TsParamPropParam::Assign(_) => false,
        },
    }
}

fn is_public_member(accessibility: Option<Accessibility>) -> bool {
    accessibility.is_none_or(|accessibility| accessibility == Accessibility::Public)
}

fn hir_type_contains_callback(ty: &HirType) -> bool {
    match ty {
        HirType::Function(..) | HirType::CallableFunction(..) => true,
        HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner)
        | HirType::Array(inner) => hir_type_contains_callback(inner),
        HirType::Tuple(elements) | HirType::Union(elements) => {
            elements.iter().any(hir_type_contains_callback)
        }
        HirType::Object(fields) => fields
            .iter()
            .any(|(_, field)| hir_type_contains_callback(field)),
        _ => false,
    }
}

fn contextualize_native_callbacks(ty: HirType) -> HirType {
    match ty {
        HirType::CallableFunction(params, optional, rest, ret) if rest.is_none() => {
            let required = optional
                .first_at_or_after(0)
                .unwrap_or(params.len())
                .min(params.len());
            let params = params
                .into_iter()
                .map(|param| match param {
                    HirType::Optional(payload) => *payload,
                    other => other,
                })
                .collect::<Vec<_>>();
            HirType::Union(
                (required..=params.len())
                    .map(|arity| {
                        HirType::Function(
                            params[..arity].to_vec(),
                            Box::new(contextualize_native_callbacks(ret.as_ref().clone())),
                        )
                    })
                    .collect(),
            )
        }
        HirType::Object(fields) => HirType::Object(
            fields
                .into_iter()
                .map(|(name, ty)| (name, contextualize_native_callbacks(ty)))
                .collect(),
        ),
        HirType::Array(inner) => {
            HirType::Array(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Optional(inner) => {
            HirType::Optional(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Nullable(inner) => {
            HirType::Nullable(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Nullish(inner) => {
            HirType::Nullish(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Tuple(elements) => HirType::Tuple(
            elements
                .into_iter()
                .map(contextualize_native_callbacks)
                .collect(),
        ),
        HirType::Union(elements) => HirType::Union(
            elements
                .into_iter()
                .flat_map(|element| match contextualize_native_callbacks(element) {
                    HirType::Union(nested) => nested,
                    other => vec![other],
                })
                .collect(),
        ),
        other => other,
    }
}

/// Best-effort type used only to contextually type callbacks nested in a
/// fallback class method's dynamic object argument. Unknown callback values
/// stay as live `JsValue`s; unrelated object fields degrade to JSON.
fn contextual_dynamic_type(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic: &GenericInterfaces,
    dynamic_leaf: bool,
    in_progress: &mut Vec<String>,
) -> HirType {
    if !matches!(
        ty,
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function))
            if !function.params.iter().any(|parameter| matches!(parameter, TsFnParam::Rest(_)))
    ) {
        if let DtsType::Native(native) = resolve_ts_type_with_substitution(
            ty,
            substitution,
            interfaces,
            generic,
            &mut Vec::new(),
        ) {
            let explicitly_json = matches!(
                ty,
                TsType::TsTypeRef(reference)
                    if matches!(&reference.type_name, TsEntityName::Ident(name) if name.sym == *"JsValue" || name.sym == *"Json")
            );
            if dynamic_leaf && native == HirType::Json && !explicitly_json {
                return HirType::JsValue;
            }
            return contextualize_native_callbacks(native);
        }
    }
    if dynamic_leaf {
        return HirType::JsValue;
    }
    match ty {
        TsType::TsParenthesizedType(value) => contextual_dynamic_type(
            &value.type_ann,
            substitution,
            interfaces,
            generic,
            false,
            in_progress,
        ),
        TsType::TsArrayType(array) => HirType::Array(Box::new(contextual_dynamic_type(
            &array.elem_type,
            substitution,
            interfaces,
            generic,
            false,
            in_progress,
        ))),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let DtsType::Native(result) = classify_native_union(union, |element| {
                DtsType::Native(contextual_dynamic_type(
                    element,
                    substitution,
                    interfaces,
                    generic,
                    false,
                    in_progress,
                ))
            }) else {
                unreachable!("contextual union classifier always returns native members")
            };
            result
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            let mut params = Vec::new();
            let mut optional = Vec::new();
            let mut rest = None;
            for parameter in &function.params {
                match parameter {
                    TsFnParam::Ident(parameter) => {
                        if parameter.id.sym == "this" {
                            continue;
                        }
                        params.push(parameter.type_ann.as_ref().map_or(HirType::JsValue, |annotation| {
                            contextual_dynamic_type(
                                &annotation.type_ann,
                                substitution,
                                interfaces,
                                generic,
                                true,
                                in_progress,
                            )
                        }));
                        optional.push(parameter.id.optional);
                    }
                    // A rest parameter (`...args: Events[Event]`, real
                    // example: `minipass`'s own generic `on<Event extends
                    // keyof Events>(ev: Event, handler: (...args: Events[
                    // Event]) => any)`) used to fall through the `else`
                    // arm below and get silently dropped -- `params`
                    // stayed empty, producing a zero-argument `HirType::
                    // Function` for the whole callback and rejecting any
                    // real inline literal with an actual parameter
                    // ("contextual callback accepts at most 0
                    // parameter(s), got 1"). `Events[Event]` can't
                    // classify natively here (`Event` is still this
                    // method's own unresolved type parameter, not yet a
                    // call-site literal), so this mirrors the same "stay
                    // dynamic" fallback `classify_ts_type`'s own
                    // non-generic rest-callback handling already uses
                    // (`types/classification.rs`).
                    TsFnParam::Rest(rest_param) => {
                        rest = Some(Box::new(rest_param.type_ann.as_ref().map_or(
                            HirType::JsValue,
                            |annotation| {
                                contextual_dynamic_type(
                                    rest_element_type(&annotation.type_ann),
                                    substitution,
                                    interfaces,
                                    generic,
                                    true,
                                    in_progress,
                                )
                            },
                        )));
                    }
                    _ => continue,
                }
            }
            let ret = contextual_dynamic_type(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic,
                true,
                in_progress,
            );
            if let Some(rest) = rest {
                HirType::CallableFunction(
                    params,
                    HirOptionalMask::from_bools(&optional),
                    Some(rest),
                    Box::new(ret),
                )
            } else if optional.iter().any(|optional| *optional) {
                let required = optional
                    .iter()
                    .position(|optional| *optional)
                    .unwrap_or(params.len());
                HirType::Union(
                    (required..=params.len())
                        .map(|arity| {
                            HirType::Function(params[..arity].to_vec(), Box::new(ret.clone()))
                        })
                        .collect(),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            }
        }
        TsType::TsTypeRef(reference) => {
            let name = match &reference.type_name {
                TsEntityName::Ident(name) => name.sym.to_string(),
                TsEntityName::TsQualifiedName(name) => name.right.sym.to_string(),
            };
            if let Some(value) = substitution.get(&name) {
                return value.clone();
            }
            let declaration = if let Some(declaration) = generic.interfaces.get(&name) {
                declaration
            } else if let Some(alias) = generic.aliases.get(&name) {
                if in_progress.contains(&name) {
                    return HirType::Json;
                }
                in_progress.push(name.clone());
                let result = contextual_dynamic_type(
                    &alias.type_ann,
                    substitution,
                    interfaces,
                    generic,
                    false,
                    in_progress,
                );
                in_progress.pop();
                return result;
            } else {
                return HirType::Json;
            };
            if in_progress.contains(&name) {
                return HirType::Json;
            }
            let mut local = substitution.clone();
            if let Some(parameters) = &declaration.type_params {
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|arguments| arguments.params.as_slice())
                    .unwrap_or_default();
                for (index, parameter) in parameters.params.iter().enumerate() {
                    let value = arguments
                        .get(index)
                        .map(|argument| {
                            contextual_dynamic_type(
                                argument,
                                substitution,
                                interfaces,
                                generic,
                                true,
                                in_progress,
                            )
                        })
                        .unwrap_or(HirType::JsValue);
                    local.insert(parameter.name.sym.to_string(), value);
                }
            }
            in_progress.push(name);
            let fields = declaration
                .body
                .body
                .iter()
                .filter_map(|member| {
                    let TsTypeElement::TsPropertySignature(property) = member else {
                        return None;
                    };
                    let name = type_property_name(&property.key)?;
                    let mut value = property.type_ann.as_ref().map_or(HirType::Json, |annotation| {
                        contextual_dynamic_type(
                            &annotation.type_ann,
                            &local,
                            interfaces,
                            generic,
                            false,
                            in_progress,
                        )
                    });
                    if property.optional {
                        value = optional_hir_type(value);
                    }
                    Some((name, value))
                })
                .collect();
            in_progress.pop();
            HirType::Object(fields)
        }
        _ => HirType::Json,
    }
}

fn lower_dts_class(
    name: &str,
    class: &Class,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    instance_class_names: &HashMap<String, String>,
    scope: &str,
    declared_type_names: &HashSet<String>,
) -> DtsClass {
    let class_type_params = class.type_params.iter().flat_map(|params| &params.params)
        .map(|param| param.name.sym.to_string()).collect::<HashSet<_>>();
    let class_type_defaults = class
        .type_params
        .iter()
        .flat_map(|params| &params.params)
        .filter_map(|parameter| {
            let TsType::TsTypeQuery(query) = parameter.default.as_deref()? else {
                return None;
            };
            let swc_ecma_ast::TsTypeQueryExpr::TsEntityName(entity) = &query.expr_name else {
                return None;
            };
            let name = type_reference_name(entity);
            let key = lexical_type_key(&name, scope, |key| instance_class_names.contains_key(key))?;
            let identity = instance_class_names.get(&key)?;
            Some((parameter.name.sym.to_string(), identity.clone()))
        })
        .collect::<HashMap<_, _>>();

    let callback_instance_classes = |function: &Function| {
        function
            .params
            .iter()
            .map(|parameter| {
                let Pat::Ident(parameter) = &parameter.pat else {
                    return Vec::new();
                };
                let Some(annotation) = &parameter.type_ann else {
                    return Vec::new();
                };
                let TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(callback)) =
                    annotation.type_ann.as_ref()
                else {
                    return Vec::new();
                };
                callback
                    .params
                    .iter()
                    .filter_map(|parameter| match parameter {
                        TsFnParam::Ident(parameter) if parameter.id.sym == *"this" => None,
                        TsFnParam::Ident(parameter) => Some(parameter.type_ann.as_ref()),
                        TsFnParam::Array(parameter) => Some(parameter.type_ann.as_ref()),
                        TsFnParam::Rest(parameter) => Some(parameter.type_ann.as_ref()),
                        TsFnParam::Object(parameter) => Some(parameter.type_ann.as_ref()),
                    })
                    .map(|annotation| {
                        let TsType::TsTypeRef(instance) = annotation?.type_ann.as_ref() else {
                            return None;
                        };
                        let TsEntityName::Ident(wrapper) = &instance.type_name else {
                            return None;
                        };
                        if wrapper.sym != *"InstanceType" {
                            return None;
                        }
                        let argument = instance.type_params.as_ref()?.params.first()?;
                        let TsType::TsTypeRef(reference) = argument.as_ref() else {
                            return None;
                        };
                        let TsEntityName::Ident(parameter) = &reference.type_name else {
                            return None;
                        };
                        class_type_defaults.get(parameter.sym.as_str()).cloned()
                    })
                    .collect()
            })
            .collect::<Vec<_>>()
    };
    let literal_params = |function: &Function| {
        function
            .params
            .iter()
            .map(|parameter| {
                let Pat::Ident(parameter) = &parameter.pat else {
                    return None;
                };
                let TsType::TsLitType(literal) = parameter.type_ann.as_ref()?.type_ann.as_ref()
                else {
                    return None;
                };
                match &literal.lit {
                    TsLit::Str(value) => Some(value.value.to_string_lossy().into_owned()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
    };
    // A namespace-qualified base (`class Parser extends stream.
    // Transform { ... }`, real example: csv-parse's own `Parser`,
    // `import * as stream from "stream"`) matched nothing here at all
    // before -- `declared` (below) is keyed by plain class name, built
    // purely from this same flattened source, so `Parser`'s inherited
    // members (`.read()`, `.write()`, `.pipe()`, ... -- everything
    // `stream.Transform` itself declares) were silently dropped, not
    // just "not found": a call to any of them failed to compile
    // ("call to unknown function `parser.read`") since thaw's type
    // system never even *attempted* to resolve `stream.Transform`,
    // regardless of whether `Transform`'s own declaration was
    // available anywhere. Fixed by extracting just the property name
    // (`Transform`) and matching it the same way a bare-identifier
    // extends already does -- `declared.get("Transform")` finds a
    // match once thaw-registry's own declaration-flattening (see
    // `dts_source_with_reexported_functions`'s handling of this same
    // shape) inlines the referenced builtin's class declaration into
    // the flattened output under that plain name.
    fn super_class_name(expression: &Expr) -> Option<String> {
        match expression {
            Expr::Ident(name) => Some(name.sym.to_string()),
            Expr::Member(member) => {
                let MemberProp::Ident(name) = &member.prop else { return None; };
                Some(format!("{}.{}", super_class_name(&member.obj)?, name.sym))
            }
            _ => None,
        }
    }
    let extends = class
        .super_class
        .as_deref()
        .and_then(super_class_name)
        .map(|base| lexical_type_key(&base, scope, |key| {
            instance_class_names.values().any(|name| name == key)
        }).or_else(|| {
            base.rsplit('.').next().and_then(|bare| instance_class_names.get(bare).cloned())
        }).unwrap_or(base));
    let mut constructors = Vec::new();
    let mut methods = Vec::new();
    let mut properties = Vec::new();
    let has_constructor = class
        .body
        .iter()
        .any(|member| matches!(member, ClassMember::Constructor(_)));
    let constructible = !class.is_abstract
        && (!has_constructor
            || class.body.iter().any(|member| {
            matches!(
                member,
                ClassMember::Constructor(constructor)
                    if is_public_member(constructor.accessibility)
            )
        }));
    for member in &class.body {
        match member {
            ClassMember::Constructor(constructor)
                if is_public_member(constructor.accessibility) => constructors.push(DtsConstructor {
                params: lower_class_params(
                    if matches!(constructor.params.last(), Some(ParamOrTsParamProp::Param(param)) if matches!(&param.pat, Pat::Rest(_))) {
                        &constructor.params[..constructor.params.len() - 1]
                    } else {
                        &constructor.params
                    },
                    interfaces,
                    generic_interfaces,
                ),
                required_params: constructor
                    .params
                    .iter()
                    .take_while(|param| class_param_is_required(param))
                    .count(),
                rest_param: constructor.params.last().and_then(|param| {
                    let ParamOrTsParamProp::Param(param) = param else { return None; };
                    let Pat::Rest(rest) = &param.pat else { return None; };
                    let Pat::Ident(binding) = rest.arg.as_ref() else { return None; };
                    let ty = rest.type_ann.as_ref().map(|annotation| {
                        classify_ts_type(rest_element_type(&annotation.type_ann), interfaces, generic_interfaces)
                    }).unwrap_or_else(|| DtsType::Unsupported("missing rest type annotation".into()));
                    Some((safe_param_name(binding.id.sym.as_ref()), ty))
                }),
                overloaded: false,
            }),
            ClassMember::Method(method) if is_public_member(method.accessibility) => {
                let Some(method_name) = property_name(&method.key) else {
                    continue;
                };
                let function = lower_dts_function(
                    &method_name,
                    &method.function,
                    interfaces,
                    generic_interfaces,
                );
                let mut params = function.params;
                let mut substitution = HashMap::new();
                if let Some(parameters) = &method.function.type_params {
                    for parameter in &parameters.params {
                        // `Json`, not `JsValue`: this method type parameter
                        // (real example: `on<Event extends keyof Events>
                        // (ev: Event, ...)`'s own `Event`) most often ends
                        // up as a *plain* parameter's own type (`ev:
                        // Event`), not just nested inside a callback's
                        // rest-parameter element type. `Json` is this
                        // codebase's established "matches any call-site
                        // value, no extra coercion needed" wildcard
                        // (`overload_type_score`'s `(HirType::Json, _) =>
                        // Some(0)`, and the many existing Json<->native
                        // coercions in `thaw-hir`'s own `coerce_to_
                        // declared`) -- `JsValue` has neither, so a plain
                        // literal argument (`rs.on("data", ...)`) failed
                        // both overload matching and, once matched,
                        // argument coercion ("value has type Json,
                        // expected JsValue").
                        substitution.insert(parameter.name.sym.to_string(), HirType::Json);
                    }
                }
                for (parameter, (_, classified)) in method.function.params.iter().zip(&mut params) {
                    let Pat::Ident(parameter) = &parameter.pat else {
                        continue;
                    };
                    let Some(annotation) = &parameter.type_ann else {
                        continue;
                    };
                    let contextual = contextual_dynamic_type(
                        &annotation.type_ann,
                        &substitution,
                        interfaces,
                        generic_interfaces,
                        false,
                        &mut Vec::new(),
                    );
                    // Also re-applies the substituted result when the
                    // parameter's own declared type is *directly* one of
                    // this method's own type parameters (`ev: Event`),
                    // not just when the contextual result happens to look
                    // like a callback. `Event` isn't a resolvable type on
                    // its own (it's this method's own type parameter), so
                    // the first, non-substituted pass leaves it
                    // `Unsupported`; the substituted pass above correctly
                    // resolves it to `HirType::Json`, a plain scalar shape
                    // `hir_type_contains_callback` alone would never
                    // catch. Deliberately narrow (checks the parameter's
                    // *own* raw annotation, not merely "was `Unsupported`
                    // for any reason") -- an earlier, broader attempt
                    // here (re-applying whenever `classified` was already
                    // `Unsupported`, regardless of why) regressed real
                    // Fallback overload-collapsing elsewhere (a later
                    // pass expects an *unresolved* parameter to still
                    // read `Unsupported`, to fold it into a wider `Union`
                    // across sibling overloads -- pre-empting that here
                    // left it a plain `Native` type instead, missing
                    // members other real overloads still needed).
                    let is_bare_generic_reference = matches!(
                        annotation.type_ann.as_ref(),
                        TsType::TsTypeRef(reference)
                            if reference.type_params.is_none()
                                && matches!(
                                    &reference.type_name,
                                    TsEntityName::Ident(id)
                                        if substitution.contains_key(id.sym.as_str())
                                )
                    );
                    if hir_type_contains_callback(&contextual) || is_bare_generic_reference {
                        *classified = DtsType::Native(contextual);
                    }
                }
                let rest_param = function.rest_param;
                let mut return_type_params = class_type_params.clone();
                if let Some(params) = &method.function.type_params {
                    return_type_params.extend(params.params.iter().map(|param| param.name.sym.to_string()));
                }
                let return_instance_class = named_return_type(
                    method.function.return_type.as_deref(), name, instance_class_names,
                    scope, declared_type_names, &return_type_params,
                );
                methods.push(DtsMethod {
                    name: function.name,
                    params,
                    required_params: method
                        .function
                        .params
                        .iter()
                        .take_while(|param| match &param.pat {
                            Pat::Ident(binding) => !binding.optional,
                            Pat::Assign(_) | Pat::Rest(_) => false,
                            _ => true,
                        })
                        .count(),
                    rest_param,
                    ret: function.ret,
                    return_instance_class,
                    callback_instance_classes: callback_instance_classes(&method.function),
                    literal_params: literal_params(&method.function),
                    is_static: method.is_static,
                    kind: match method.kind {
                        MethodKind::Method => DtsMethodKind::Method,
                        MethodKind::Getter => DtsMethodKind::Getter,
                        MethodKind::Setter => DtsMethodKind::Setter,
                    },
                    overloaded: false,
                });
            }
            ClassMember::ClassProp(property) if is_public_member(property.accessibility) => {
                let Some(property_name) = property_name(&property.key) else {
                    continue;
                };
                if let Some(TsType::TsFnOrConstructorType(
                    TsFnOrConstructorType::TsFnType(function),
                )) = property.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref())
                {
                    let mut return_type_params = class_type_params.clone();
                    if let Some(params) = &function.type_params {
                        return_type_params.extend(params.params.iter().map(|param| param.name.sym.to_string()));
                    }
                    let return_annotation = &function.type_ann;
                    let function = lower_dts_fn_type(
                        &property_name,
                        function,
                        interfaces,
                        generic_interfaces,
                        &HashMap::new(),
                        &HashMap::new(),
                    );
                    let return_instance_class = named_return_type(
                        Some(return_annotation), name, instance_class_names,
                        scope, declared_type_names, &return_type_params,
                    );
                    methods.push(DtsMethod {
                        name: function.name,
                        params: function.params,
                        required_params: function.required_params,
                        rest_param: function.rest_param,
                        ret: function.ret,
                        return_instance_class,
                        callback_instance_classes: Vec::new(),
                        literal_params: Vec::new(),
                        is_static: property.is_static,
                        kind: DtsMethodKind::Method,
                        overloaded: false,
                    });
                    continue;
                }
                let ty = property
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    })
                    .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
                let ty = match ty {
                    DtsType::Native(ty) if property.is_optional => {
                        DtsType::Native(optional_hir_type(ty))
                    }
                    other => other,
                };
                properties.push(DtsProperty {
                    name: property_name,
                    ty,
                    is_static: property.is_static,
                    readonly: property.readonly,
                });
            }
            _ => {}
        }
    }
    DtsClass {
        name: name.to_string(),
        extends,
        constructible,
        constructors,
        methods,
        properties,
    }
}
