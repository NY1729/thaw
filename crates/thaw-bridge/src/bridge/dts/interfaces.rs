/// Visit declarations under their lexical namespace while keeping the
/// original module order. Functions, callable constants, and aliases share
/// this traversal so a member cannot silently lose its owner namespace.
fn scoped_module_items<'a>(module: &'a Module) -> Vec<(String, &'a ModuleItem)> {
    fn walk<'a>(item: &'a ModuleItem, scope: &str, found: &mut Vec<(String, &'a ModuleItem)>) {
        found.push((scope.to_string(), item));
        let decl = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => decl,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
            _ => return,
        };
        if let Decl::TsModule(namespace) = decl {
            let swc_ecma_ast::TsModuleName::Ident(name) = &namespace.id else { return; };
            let scope = if scope.is_empty() { name.sym.to_string() }
                else { format!("{scope}.{}", name.sym) };
            if let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body {
                for item in &block.body { walk(item, &scope, found); }
            }
        }
    }
    let mut found = Vec::new();
    for item in &module.body { walk(item, "", &mut found); }
    found
}

/// Retains each declaration's full namespace path while returning its bare
/// identifier separately. `parse_dts` keeps ordinary namespace identities
/// distinct and preserves the bare CommonJS method ABI for `export = NS`.
fn scoped_fn_decls<'a>(module: &'a Module) -> Vec<(String, String, &'a Function)> {
    let mut found = Vec::new();
    for (scope, item) in scoped_module_items(module) {
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) = item {
            if let DefaultDecl::Fn(function) = &export.decl {
                if let Some(name) = &function.ident {
                    let name = name.sym.to_string();
                    found.push((name.clone(), name, &*function.function));
                }
            }
            continue;
        }
        let decl = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => decl,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
            _ => continue,
        };
        if let Decl::Fn(function) = decl {
            let bare = function.ident.sym.to_string();
            let full = if scope.is_empty() { bare.clone() } else { format!("{scope}.{bare}") };
            found.push((full, bare, &*function.function));
        }
    }
    found
}

/// The interface name that `export = <ident>;` together with `declare
/// const <ident>: <TypeRef>;` (`let`/`var` too) resolves to, if the file
/// uses that shape -- the specific pattern that identifies "this
/// interface IS the package's own namespace object" for
/// `extract_interface_method_decls`, as opposed to just some interface
/// used elsewhere as an ordinary parameter/return type (e.g. a callback
/// interface that has nothing to do with the package's own exports).
/// `<TypeRef>` may be namespace-qualified (`_.LoDashStatic`); keep its
/// complete name so a same-named interface in another namespace is not used.
fn export_assignment_interface_name(module: &Module) -> Option<String> {
    let exported = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            match export.expr.as_ref() {
                Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            }
        }
        _ => None,
    })?;
    module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(var_decl))) = item else {
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
            let TsType::TsTypeRef(ty_ref) = annotation.type_ann.as_ref() else {
                return None;
            };
            Some(type_reference_name(&ty_ref.type_name))
        })
    })
}

/// Collect public methods from the `export =` interface and its bases,
/// including all merged declarations under each namespace-qualified name.
fn export_assignment_interface_methods<'a>(
    module: &'a Module,
    target: &str,
) -> Vec<(String, String, &'a TsMethodSignature)> {
    fn expr_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.sym.to_string()),
            Expr::Member(member) => {
                let MemberProp::Ident(name) = &member.prop else { return None; };
                Some(format!("{}.{}", expr_name(&member.obj)?, name.sym))
            }
            _ => None,
        }
    }
    fn collect<'a>(
        name: &str,
        declarations: &HashMap<String, Vec<&'a TsInterfaceDecl>>,
        visited: &mut HashSet<String>,
        methods: &mut Vec<(String, String, &'a TsMethodSignature)>,
    ) {
        if !visited.insert(name.to_string()) { return; }
        let Some(interfaces) = declarations.get(name) else { return; };
        for interface in interfaces {
            for base in &interface.extends {
                let Some(base_name) = expr_name(&base.expr) else { continue; };
                let base_name = lexical_type_key(
                    &base_name, declaration_scope(name), |candidate| declarations.contains_key(candidate),
                ).unwrap_or(base_name);
                collect(&base_name, declarations, visited, methods);
            }
        }
        for interface in interfaces {
            for member in &interface.body.body {
                if let TsTypeElement::TsMethodSignature(method) = member {
                    if !method.computed {
                        if let Some(method_name) = type_property_name(&method.key) {
                            methods.push((method_name, name.to_string(), method));
                        }
                    }
                }
            }
        }
    }
    let (interfaces, _) = scoped_type_declarations(module);
    let mut declarations = HashMap::<String, Vec<&TsInterfaceDecl>>::new();
    for (name, interface) in interfaces {
        declarations.entry(name).or_default().push(interface);
    }
    let mut methods = Vec::new();
    collect(target, &declarations, &mut HashSet::new(), &mut methods);
    methods
}

/// Like `extract_fn_decls`/`extract_class_decls`, but for an `interface`
/// declaration -- including one nested inside a `declare namespace X {
/// ... }` block, real-world example: `ms`'s own namespace carrying its
/// `Unit`/`UnitAnyCase`/`StringValue` helper types alongside its
/// `ms(...)` overloads. Without this, a namespace-nested interface (or,
/// via `extract_type_alias_decls`, type alias) was invisible to
/// `resolve_interfaces` entirely -- any reference to it anywhere, even a
/// same-namespace-qualified one (`ms.StringValue`), stayed permanently
/// `Unsupported`.
fn extract_interface_decls(item: &ModuleItem) -> Vec<&TsInterfaceDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_interface_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_interface_decls_from_decl(&export.decl)
        }
        _ => Vec::new(),
    }
}

fn extract_interface_decls_from_decl(decl: &Decl) -> Vec<&TsInterfaceDecl> {
    match decl {
        Decl::TsInterface(iface) => vec![iface],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_interface_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn extract_type_alias_decls(item: &ModuleItem) -> Vec<&swc_ecma_ast::TsTypeAliasDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_type_alias_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_type_alias_decls_from_decl(&export.decl)
        }
        _ => Vec::new(),
    }
}

fn extract_type_alias_decls_from_decl(decl: &Decl) -> Vec<&swc_ecma_ast::TsTypeAliasDecl> {
    match decl {
        Decl::TsTypeAlias(alias) => vec![alias],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_type_alias_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn type_reference_name(name: &TsEntityName) -> String {
    match name {
        TsEntityName::Ident(name) => name.sym.to_string(),
        TsEntityName::TsQualifiedName(name) =>
            format!("{}.{}", type_reference_name(&name.left), name.right.sym),
    }
}

fn declaration_scope(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(scope, _)| scope)
}

fn lexical_type_key(
    reference: &str,
    scope: &str,
    contains: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut current = scope;
    while !current.is_empty() {
        let candidate = format!("{current}.{reference}");
        if contains(&candidate) { return Some(candidate); }
        current = current.rsplit_once('.').map_or("", |(parent, _)| parent);
    }
    contains(reference).then(|| reference.to_string())
}

fn scoped_type_context<'a>(
    scope: &str,
    interfaces: &HashMap<String, DtsType>,
    generic: &GenericInterfaces<'a>,
) -> (HashMap<String, DtsType>, GenericInterfaces<'a>) {
    let mut resolved = interfaces.clone();
    let mut scoped_generic = generic.clone();
    let mut references = HashSet::new();
    for name in interfaces.keys().chain(generic.interfaces.keys())
        .chain(generic.aliases.keys()).chain(generic.classes.iter())
    {
        let mut suffix = name.as_str();
        while let Some((_, rest)) = suffix.split_once('.') {
            references.insert(rest.to_string());
            suffix = rest;
        }
    }
    for reference in references {
        let Some(key) = lexical_type_key(&reference, scope, |name| {
            interfaces.contains_key(name) || generic.interfaces.contains_key(name)
                || generic.aliases.contains_key(name) || generic.classes.contains(name)
        }) else { continue; };
        if key == reference { continue; }
        resolved.remove(&reference);
        scoped_generic.interfaces.remove(&reference);
        scoped_generic.aliases.remove(&reference);
        scoped_generic.classes.remove(&reference);
        scoped_generic.canonical_names.remove(&reference);
        if let Some(ty) = interfaces.get(&key) {
            resolved.insert(reference, ty.clone());
        } else if let Some(decl) = generic.interfaces.get(&key) {
            scoped_generic.canonical_names.insert(reference.clone(), key);
            scoped_generic.interfaces.insert(reference, *decl);
        } else if let Some(decl) = generic.aliases.get(&key) {
            scoped_generic.canonical_names.insert(reference.clone(), key);
            scoped_generic.aliases.insert(reference, *decl);
        } else if generic.classes.contains(&key) {
            scoped_generic.classes.insert(reference);
        }
    }
    (resolved, scoped_generic)
}

/// Preserve the enclosing declaration namespace in resolver keys. Bare names
/// are added separately only when they cannot identify a different scope.
fn scoped_type_declarations<'a>(
    module: &'a Module,
) -> (
    Vec<(String, &'a TsInterfaceDecl)>,
    Vec<(String, &'a swc_ecma_ast::TsTypeAliasDecl)>,
) {
    fn walk_item<'a>(
        item: &'a ModuleItem,
        scope: &str,
        interfaces: &mut Vec<(String, &'a TsInterfaceDecl)>,
        aliases: &mut Vec<(String, &'a swc_ecma_ast::TsTypeAliasDecl)>,
    ) {
        let decl = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => decl,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
            _ => return,
        };
        match decl {
            Decl::TsInterface(iface) => {
                let name = iface.id.sym.as_str();
                interfaces.push((if scope.is_empty() { name.to_string() } else { format!("{scope}.{name}") }, iface));
            }
            Decl::TsTypeAlias(alias) => {
                let name = alias.id.sym.as_str();
                aliases.push((if scope.is_empty() { name.to_string() } else { format!("{scope}.{name}") }, alias));
            }
            Decl::TsModule(namespace) => {
                let swc_ecma_ast::TsModuleName::Ident(name) = &namespace.id else { return; };
                let name = if scope.is_empty() {
                    name.sym.to_string()
                } else {
                    format!("{scope}.{}", name.sym)
                };
                if let Some(TsNamespaceBody::TsModuleBlock(block)) = &namespace.body {
                    for item in &block.body {
                        walk_item(item, &name, interfaces, aliases);
                    }
                }
            }
            _ => {}
        }
    }
    let mut interfaces = Vec::new();
    let mut aliases = Vec::new();
    for item in &module.body {
        walk_item(item, "", &mut interfaces, &mut aliases);
    }
    (interfaces, aliases)
}

#[derive(Default, Clone)]
struct GenericInterfaces<'a> {
    interfaces: HashMap<String, &'a TsInterfaceDecl>,
    aliases: HashMap<String, &'a swc_ecma_ast::TsTypeAliasDecl>,
    classes: HashSet<String>,
    canonical_names: HashMap<String, String>,
}

fn generic_canonical_name<'a>(name: &'a str, generic: &'a GenericInterfaces<'_>) -> &'a str {
    generic.canonical_names.get(name).map_or(name, String::as_str)
}

/// Resolves every top-level *non-generic* `interface` into a `DtsType`
/// (first map), mirroring `thaw_hir::lower::resolve_interfaces` but
/// degrading to `DtsType::Unsupported` (with a reason) instead of erroring
/// on a self-referential/otherwise-unrepresentable interface -- one broken
/// interface should make signatures that use it fall back, not abort
/// classifying the rest of the `.d.ts` file. Generic interfaces are kept
/// raw (second map), resolved on demand via substitution -- see
/// `resolve_generic_interface`, and `thaw_hir::lower::GenericInterfaces`'s
/// doc comment for the scope limits this mirrors (no nested-inside-
/// another-interface use, no `extends` on the generic interface itself).
fn resolve_interfaces(module: &Module) -> (HashMap<String, DtsType>, GenericInterfaces<'_>) {
    let mut raw: HashMap<String, Vec<&TsInterfaceDecl>> = HashMap::new();
    let mut names = Vec::new();
    let mut generic = GenericInterfaces::default();
    let scoped_classes = scoped_class_names(module);
    let mut class_locations = HashMap::<String, HashSet<String>>::new();
    for (qualified, bare) in &scoped_classes {
        class_locations.entry(bare.clone()).or_default().insert(qualified.clone());
    }
    for (qualified, bare) in scoped_classes {
        generic.classes.insert(qualified.clone());
        if class_locations[&bare].len() == 1 {
            generic.classes.insert(bare);
        }
    }
    let (interface_decls, alias_decls) = scoped_type_declarations(module);
    for (name, iface) in &interface_decls {
        if iface.type_params.is_some() {
            generic.interfaces.insert(name.to_string(), iface);
        } else {
            if !raw.contains_key(name) {
                names.push(name.to_string());
            }
            raw.entry(name.to_string()).or_default().push(iface);
        }
    }
    for (name, alias) in &alias_decls {
        if alias.type_params.is_some() {
            generic.aliases.insert(name.to_string(), alias);
        }
    }

    let mut resolved = HashMap::new();
    for name in &names {
        resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
    }
    let aliases = alias_decls.iter()
        .filter(|(_, alias)| alias.type_params.is_none())
        .collect::<Vec<_>>();
    for _ in 0..raw.len() + aliases.len() {
        for name in &names {
            // A newly resolved alias in the same lexical namespace may
            // replace a previously selected outer name, even when that
            // earlier interface result looked native.
            resolved.remove(name);
        }
        for name in &names {
            resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
        }
        for (name, alias) in &aliases {
            let (context, generic_context) = scoped_type_context(
                declaration_scope(name), &resolved, &generic,
            );
            let ty = classify_ts_type(&alias.type_ann, &context, &generic_context);
            resolved.insert(name.to_string(), ty);
        }
    }
    // Keep the old convenient bare spelling for a unique namespace member,
    // but never let one namespace's Options replace another's Options.
    let mut bare_counts = HashMap::<String, HashSet<String>>::new();
    for name in resolved.keys().chain(generic.interfaces.keys()).chain(generic.aliases.keys()) {
        bare_counts.entry(name.rsplit('.').next().unwrap_or(name).to_string())
            .or_default().insert(name.to_string());
    }
    for (bare, keys) in bare_counts {
        if keys.len() != 1 || resolved.contains_key(&bare)
            || generic.interfaces.contains_key(&bare) || generic.aliases.contains_key(&bare) {
            continue;
        }
        let name = keys.into_iter().next().unwrap();
        if let Some(ty) = resolved.get(&name).cloned() {
            resolved.insert(bare, ty);
        } else if let Some(decl) = generic.interfaces.get(&name).copied() {
            generic.canonical_names.insert(bare.clone(), name);
            generic.interfaces.insert(bare, decl);
        } else if let Some(decl) = generic.aliases.get(&name).copied() {
            generic.canonical_names.insert(bare.clone(), name);
            generic.aliases.insert(bare, decl);
        }
    }
    (resolved, generic)
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, Vec<&TsInterfaceDecl>>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let Some(ty) = resolved.get(name) {
        return ty.clone();
    }
    if in_progress.iter().any(|n| n == name) {
        let ty = DtsType::Unsupported(format!(
            "interface `{name}` is (indirectly) self-referential"
        ));
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }
    let Some(declarations) = raw.get(name) else {
        return DtsType::Unsupported(format!("unknown or generic interface `{name}`"));
    };

    if declarations.iter().any(|iface| iface.body.body.iter()
        .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_))))
    {
        let ty = DtsType::Native(HirType::JsValue);
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }

    in_progress.push(name.to_string());

    // `extends`: same rule as thaw-hir's `resolve_interface` -- base
    // fields first (in `extends`-list, then declaration, order), then this
    // interface's own fields; any name collision degrades the whole
    // interface to `Unsupported` rather than guessing an override rule.
    let mut fields: Vec<(String, HirType)> = Vec::new();
    let mut dictionary = None;
    let mut failure = None;
    // Set when an `extends` base itself resolved to one opaque `JsValue`
    // handle (a bare-call-signature interface, or another interface that
    // already collapsed this same way -- see the final `result` match's
    // own "every field opaque" rule below). There's no concrete `Object`
    // shape to merge inherited fields into in that case, so the whole
    // derived interface becomes opaque too, regardless of what fields it
    // adds of its own -- real example: axios's `AxiosStatic extends
    // AxiosInstance`, where `AxiosInstance` itself has call signatures.
    let mut base_is_opaque = false;
    'extends: for iface in declarations {
    for base in &iface.extends {
        if base.type_args.is_some() {
            failure =
                Some("extends a base with type arguments, which is not classified yet".to_string());
            break;
        }
        let base_name = match base.expr.as_ref() {
            Expr::Ident(base_ident) => lexical_type_key(
                base_ident.sym.as_str(), declaration_scope(name), |candidate| raw.contains_key(candidate),
            ).unwrap_or_else(|| base_ident.sym.to_string()),
            _ => {
                failure = Some("has an unsupported `extends` target".to_string());
                break;
            }
        };
        match resolve_interface(&base_name, raw, generic, resolved, in_progress) {
            DtsType::Native(HirType::Object(base_fields)) => {
                for (field_name, field_ty) in base_fields {
                    if fields.iter().any(|(n, _)| *n == field_name) {
                        failure = Some(format!(
                            "inherits field `{field_name}` from `{base_name}`, which collides with an earlier field"
                        ));
                        break 'extends;
                    }
                    fields.push((field_name, field_ty));
                }
            }
            DtsType::Native(HirType::Dictionary(element)) => {
                if dictionary
                    .as_ref()
                    .is_some_and(|existing| existing != element.as_ref())
                    || fields.iter().any(|(_, field)| field != element.as_ref())
                {
                    failure = Some(format!(
                        "inherits an incompatible dictionary value from `{base_name}`"
                    ));
                    break;
                }
                dictionary = Some(*element);
            }
            DtsType::Native(HirType::JsValue) => {
                base_is_opaque = true;
            }
            DtsType::Native(_) => {
                unreachable!("resolve_interface always returns an Object, a Dictionary, JsValue, or Unsupported")
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("extends unresolvable base `{base_name}`: {reason}"));
                break;
            }
        }
    }

    }
    if failure.is_none() {
        'members: for iface in declarations {
        for member in &iface.body.body {
            if let TsTypeElement::TsIndexSignature(signature) = member {
                let value = match index_signature_value(signature) {
                    Ok(value) => resolve_type_with_interfaces(
                        value,
                        raw,
                        generic,
                        resolved,
                        in_progress,
                    ),
                    Err(reason) => DtsType::Unsupported(reason),
                };
                match value {
                    DtsType::Native(value)
                        if dictionary.as_ref().is_none_or(|existing| existing == &value)
                            && fields.iter().all(|(_, field)| field == &value) =>
                    {
                        dictionary = Some(value);
                    }
                    DtsType::Native(_) => {
                        failure = Some("declares an incompatible dictionary value type".into());
                        break;
                    }
                    DtsType::Unsupported(reason) => {
                        failure = Some(reason);
                        break;
                    }
                }
                continue;
            }
            let TsTypeElement::TsPropertySignature(prop) = member else {
                failure = Some("has a non-property member (method/index signature)".to_string());
                break;
            };
            let Some(field_name) = type_property_name(&prop.key) else {
                failure = Some("has an unsupported property key".to_string());
                break;
            };
            if fields.iter().any(|(n, _)| *n == field_name) {
                failure = Some(format!(
                    "declares field `{field_name}`, which collides with an inherited field"
                ));
                break;
            }
            let field_ty = match &prop.type_ann {
                Some(ann) => {
                    resolve_type_with_interfaces(
                        &ann.type_ann,
                        raw,
                        generic,
                        resolved,
                        in_progress,
                    )
                }
                None => {
                    DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                }
            };
            match field_ty {
                // Any type hir_codegen's `basic_type` can represent is fine
                // as a field now (fields are word-sized regardless of
                // their own type -- see hir_codegen.rs's `basic_type` for
                // the `HirType::Object` case), including a nested object.
                DtsType::Native(ty) => {
                    let ty = if prop.optional {
                        optional_hir_type(ty)
                    } else {
                        ty
                    };
                    if dictionary.as_ref().is_some_and(|element| element != &ty) {
                        failure = Some(format!(
                            "field `{field_name}` does not match its index value type"
                        ));
                        break;
                    }
                    fields.push((field_name, ty));
                }
                DtsType::Unsupported(reason) => {
                    failure = Some(format!("field `{field_name}`: {reason}"));
                    break;
                }
            }
        }
        if failure.is_some() {
            break 'members;
        }
        }
    }

    in_progress.pop();

    let result = match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        // An opaque `extends` base (see `base_is_opaque` above) leaves
        // nothing concrete to combine with, so the whole interface is
        // opaque too, whatever its own fields look like.
        None if base_is_opaque => DtsType::Native(HirType::JsValue),
        // *Any* field an opaque `JsValue` (a method, or a property typed
        // as a bare-call-signature interface -- kleur's `interface Kleur
        // { red: Color; ... }` where `Color` is `{ (x): string }`, every
        // field opaque). A native `Object` decoded from a live JS value
        // can only ever carry its *data* fields across (a JSON decode
        // drops any function, whether that's the whole value or just one
        // field of it) -- fine on its own for a field nothing ever reads
        // back, but real schema-builder objects (joi's `AnySchema`,
        // mixing plain data like `_flags: Record<string, any>` with
        // methods like `.min()`) are live objects whose *methods*
        // themselves depend on that exact identity (joi's own internal
        // "Must be invoked on a Joi instance" check) -- reconstructing
        // even just the data fields from a JSON decode and discarding
        // the rest silently produces a different, no-longer-callable
        // object. Once decided opaque, the whole interface is modelled
        // as one live handle instead, so `value.red(...)` / `schema.min
        // (...)` alike route through the dynamic host against the real,
        // original object.
        None if dictionary.is_none()
            && !fields.is_empty()
            && fields.iter().any(|(_, ty)| *ty == HirType::JsValue) =>
        {
            DtsType::Native(HirType::JsValue)
        }
        None => DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
            HirType::Dictionary(Box::new(element))
        })),
    };
    resolved.insert(name.to_string(), result.clone());
    result
}

/// Like `classify_ts_type`, but additionally resolves a `TsTypeRef` naming
/// a not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`).
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<String, Vec<&TsInterfaceDecl>>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        let ref_name = type_reference_name(&ty_ref.type_name);
        let scope = in_progress.last().map_or("", |name| declaration_scope(name));
        if let Some(name) = lexical_type_key(&ref_name, scope, |candidate| {
            raw.contains_key(candidate) || resolved.contains_key(candidate)
                || generic.interfaces.contains_key(candidate)
                || generic.aliases.contains_key(candidate) || generic.classes.contains(candidate)
        }) {
            if raw.contains_key(&name) {
                return resolve_interface(&name, raw, generic, resolved, in_progress);
            }
        }
    }
    let scope = in_progress.last().map_or("", |name| declaration_scope(name));
    let (context, generic_context) = scoped_type_context(scope, resolved, generic);
    classify_ts_type(ty, &context, &generic_context)
}

/// Never fails: an unsupported parameter pattern (e.g. destructuring)
/// degrades that one parameter to `DtsType::Unsupported`, same as any
/// other unclassifiable type, rather than aborting this function -- and,
/// since `parse_dts` used to propagate that abort as a `Result::Err`
/// covering *every* function in the file, rather than aborting every
/// other function in the same `.d.ts` file along with it. Validated
/// against a real npm package's `.d.ts` corpus (date-fns): one function
/// with a destructured parameter (`function milliseconds({ years, ... }:
/// Duration)`) used to silently delete every other function in that file
/// from `parse_dts`'s result.
fn lower_dts_function(
    name: &str,
    func: &Function,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = func.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter
                        .constraint
                        .as_deref()
                        .map(describe_ts_type),
                )
            })
            .collect(),
        param_types: func
            .params
            .iter()
            .map(|parameter| match &parameter.pat {
                Pat::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: func
            .params
            .iter()
            .map(|parameter| match &parameter.pat {
                Pat::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: func.params.last().and_then(|parameter| {
            let Pat::Rest(rest) = &parameter.pat else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: func
            .return_type
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
        tuple_return_type: generic_tuple_return_type(
            func.return_type.as_deref(),
            func.type_params.as_deref(),
            interfaces,
            generic_interfaces,
        ),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &func.type_params {
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

    let rest_param = func.params.last().and_then(|param| {
        let Pat::Rest(rest) = &param.pat else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => {
                    classify(&array.elem_type)
                }
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = func.params.len() - usize::from(rest_param.is_some());
    let required_params = func
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| !matches!(&param.pat, Pat::Ident(binding) if binding.optional))
        .count();
    let params = func
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let Pat::Ident(binding) = &param.pat else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify(&ann.type_ann),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &func.return_type {
        Some(ann) => classify(&ann.type_ann),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction {
        name,
        generic,
        params,
        param_field_constraints: Vec::new(),
        required_params,
        rest_param,
        ret,
    }
}

/// Like `lower_dts_function`, for an interface's method signature instead
/// of a plain ambient function declaration -- the same shape of
/// information (name, optional generics, fixed/rest parameters, return
/// type), just spread across `TsMethodSignature`'s own AST fields
/// (`TsFnParam` parameters, `type_ann` for the return) rather than
/// `Function`'s.
fn lower_dts_method_signature(
    name: &str,
    method: &TsMethodSignature,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = method.type_params.as_ref().map(|parameters| DtsGenericFunction {
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
        param_types: method
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
        contextual_param_types: method
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
        contextual_rest_param_type: method.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: method
            .type_ann
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
        tuple_return_type: generic_tuple_return_type(
            method.type_ann.as_deref(),
            method.type_params.as_deref(),
            interfaces,
            generic_interfaces,
        ),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &method.type_params {
        for parameter in &type_params.params {
            let constrained = parameter
                .default
                .as_deref()
                .or(parameter.constraint.as_deref())
                .and_then(|constraint| match resolve_ts_type_with_substitution(
                    constraint, &substitution, interfaces, generic_interfaces, &mut Vec::new(),
                ) {
                    DtsType::Native(ty) => Some(ty),
                    DtsType::Unsupported(_) => None,
                });
            substitution.insert(parameter.name.sym.to_string(), constrained.unwrap_or(HirType::Json));
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

    let rest_param = method.params.last().and_then(|param| {
        let TsFnParam::Rest(rest) = param else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
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
    let fixed_param_count = method.params.len() - usize::from(rest_param.is_some());
    let required_params = method
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| !matches!(param, TsFnParam::Ident(binding) if binding.optional))
        .count();
    let params = method
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let TsFnParam::Ident(binding) = param else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify(&ann.type_ann),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &method.type_ann {
        Some(ann) => classify(&ann.type_ann),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction {
        name,
        generic,
        params,
        param_field_constraints: Vec::new(),
        required_params,
        rest_param,
        ret,
    }
}
