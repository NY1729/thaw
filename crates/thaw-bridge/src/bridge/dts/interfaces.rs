/// A single top-level `declare function`/`export declare function`
/// extracts one `(name, function)` pair; a `declare namespace Foo {
/// function bar(...): ...; }` recurses into its body and extracts every
/// function found inside (at any nesting depth -- a namespace can itself
/// contain a nested namespace). Found necessary by a real npm package
/// (`qs`), whose entire type surface -- including every function --
/// lives inside `declare namespace QueryString { ... }` rather than at
/// the top level; without this, `parse_dts` found zero functions in it.
/// The extracted `DtsFunction.name` is the bare function name (`parse`,
/// not `QueryString.parse`) -- that's also what `wrap_as_commonjs_module`'s
/// object-export hoisting binds it to at runtime (`qs`'s own
/// `module.exports = { parse, stringify, ... }`), so the two already
/// agree without any extra namespace-qualification logic.
///
/// Also handles a *named* `export default function foo(...): T;` (an
/// `ExportDefaultDecl`, a different AST shape than `ExportDecl` --
/// found necessary by real ESM packages, whose `.d.ts` commonly uses
/// this form, e.g. `escape-string-regexp`'s `export default function
/// escapeStringRegexp(string: string): string;`). An *anonymous*
/// `export default function(...): T;` has no name to extract a callable
/// `DtsFunction` under and is silently skipped -- `.d.ts` authors
/// essentially always name it in practice specifically so it's
/// referenceable, so this isn't expected to matter.
fn extract_fn_decls(item: &ModuleItem) -> Vec<(&str, &Function)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_fn_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_fn_decls_from_decl(&export.decl)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Fn(fn_expr) => match &fn_expr.ident {
                Some(ident) => vec![(ident.sym.as_str(), &fn_expr.function)],
                None => Vec::new(),
            },
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_fn_decls_from_decl(decl: &Decl) -> Vec<(&str, &Function)> {
    match decl {
        Decl::Fn(fn_decl) => vec![(fn_decl.ident.sym.as_str(), &fn_decl.function)],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_fn_decls).collect()
        }
        _ => Vec::new(),
    }
}

/// The interface name that `export = <ident>;` together with `declare
/// const <ident>: <TypeRef>;` (`let`/`var` too) resolves to, if the file
/// uses that shape -- the specific pattern that identifies "this
/// interface IS the package's own namespace object" for
/// `extract_interface_method_decls`, as opposed to just some interface
/// used elsewhere as an ordinary parameter/return type (e.g. a callback
/// interface that has nothing to do with the package's own exports).
/// `<TypeRef>` may be namespace-qualified (`_.LoDashStatic`); only the
/// rightmost segment is used, matching how every interface here is
/// tracked by its own bare name regardless of which namespace it's
/// nested in.
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
            match &ty_ref.type_name {
                TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
                TsEntityName::TsQualifiedName(qualified) => {
                    Some(qualified.right.sym.to_string())
                }
            }
        })
    })
}

/// Every method signature from every declaration (top-level or inside a
/// `declare namespace` block) of the one interface named `target` --
/// parallel to `extract_fn_decls`, for an `export = obj` binding whose
/// declared type is an interface with method members rather than a
/// plain callable/namespace-function object. Real-world example:
/// lodash's `declare const _: _.LoDashStatic;` with `interface
/// LoDashStatic { chunk(...): ...; /* ~300 more */ }`, commonly re-opened
/// (TS declaration merging) across several files each augmenting the
/// same interface with a handful of methods -- rather than actually
/// merging same-named interfaces into one combined type, this just
/// extracts every declaration's own methods independently, the same
/// "grab every function-shaped thing, wherever it is" policy
/// `extract_fn_decls` already applies to plain functions; a name used by
/// more than one still only needs *a* signature that classifies, and
/// `Classification::classify_all` already falls back to `Fallback` for a
/// name with more than one anyway. Restricted to `target` (see
/// `export_assignment_interface_name`) rather than every interface in
/// the file: an interface used only as some other value's parameter or
/// return type (e.g. a callback interface with its own unrelated
/// methods) must not contribute spurious package-level functions.
fn extract_interface_method_decls<'a>(
    item: &'a ModuleItem,
    target: &str,
) -> Vec<(String, &'a TsMethodSignature)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => {
            extract_interface_method_decls_from_decl(decl, target)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_interface_method_decls_from_decl(&export.decl, target)
        }
        _ => Vec::new(),
    }
}

fn extract_interface_method_decls_from_decl<'a>(
    decl: &'a Decl,
    target: &str,
) -> Vec<(String, &'a TsMethodSignature)> {
    match decl {
        Decl::TsInterface(iface) if iface.id.sym.as_str() == target => iface
            .body
            .body
            .iter()
            .filter_map(|member| match member {
                TsTypeElement::TsMethodSignature(method) if !method.computed => {
                    type_property_name(&method.key).map(|name| (name, method))
                }
                _ => None,
            })
            .collect(),
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block
                .body
                .iter()
                .flat_map(|item| extract_interface_method_decls(item, target))
                .collect()
        }
        _ => Vec::new(),
    }
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

#[derive(Default)]
struct GenericInterfaces<'a> {
    interfaces: HashMap<String, &'a TsInterfaceDecl>,
    aliases: HashMap<String, &'a swc_ecma_ast::TsTypeAliasDecl>,
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
    let mut raw: HashMap<String, &TsInterfaceDecl> = HashMap::new();
    let mut names = Vec::new();
    let mut generic = GenericInterfaces::default();
    for iface in module.body.iter().flat_map(extract_interface_decls) {
        let name = iface.id.sym.to_string();
        if iface.type_params.is_some() {
            generic.interfaces.insert(name, iface);
        } else {
            if !raw.contains_key(&name) {
                names.push(name.clone());
            }
            raw.insert(name, iface);
        }
    }
    for alias in module
        .body
        .iter()
        .flat_map(extract_type_alias_decls)
        .filter(|alias| alias.type_params.is_some())
    {
        generic.aliases.insert(alias.id.sym.to_string(), alias);
    }

    let mut resolved = HashMap::new();
    for name in &names {
        resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
    }
    let aliases = module
        .body
        .iter()
        .flat_map(extract_type_alias_decls)
        .filter(|alias| alias.type_params.is_none())
        .collect::<Vec<_>>();
    for _ in 0..raw.len() + aliases.len() {
        for name in &names {
            if matches!(resolved.get(name), Some(DtsType::Unsupported(_))) {
                resolved.remove(name);
            }
        }
        for name in &names {
            resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
        }
        for alias in &aliases {
            let ty = classify_ts_type(&alias.type_ann, &resolved, &generic);
            resolved.insert(alias.id.sym.to_string(), ty);
        }
    }
    (resolved, generic)
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, &TsInterfaceDecl>,
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
    let Some(iface) = raw.get(name) else {
        return DtsType::Unsupported(format!("unknown or generic interface `{name}`"));
    };

    if iface
        .body
        .body
        .iter()
        .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_)))
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
    'extends: for base in &iface.extends {
        if base.type_args.is_some() {
            failure =
                Some("extends a base with type arguments, which is not classified yet".to_string());
            break;
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            failure = Some(
                "has an unsupported `extends` target (only a plain interface name)".to_string(),
            );
            break;
        };
        let base_name = base_ident.sym.to_string();
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
            DtsType::Native(_) => {
                unreachable!("resolve_interface always returns an Object or Unsupported")
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("extends unresolvable base `{base_name}`: {reason}"));
                break;
            }
        }
    }

    if failure.is_none() {
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
    }

    in_progress.pop();

    let result = match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
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
    raw: &HashMap<String, &TsInterfaceDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if raw.contains_key(ref_name) {
                return resolve_interface(ref_name, raw, generic, resolved, in_progress);
            }
        }
    }
    classify_ts_type(ty, resolved, generic)
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
        .take_while(|param| matches!(&param.pat, Pat::Ident(binding) if !binding.optional))
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
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &method.type_params {
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
        .take_while(|param| matches!(param, TsFnParam::Ident(binding) if !binding.optional))
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
        required_params,
        rest_param,
        ret,
    }
}
