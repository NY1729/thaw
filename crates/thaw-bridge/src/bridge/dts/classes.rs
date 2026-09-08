pub fn parse_dts_classes(source: &str) -> Result<Vec<DtsClass>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut classes = module
        .body
        .iter()
        .flat_map(extract_class_decls)
        .map(|(name, class)| lower_dts_class(name, class, &interfaces, &generic_interfaces))
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
    Ok(classes)
}

/// TypeScript packages often publish a private class through a public
/// constructor-valued constant (`export const Public: typeof Internal`).
/// Treat that value as the same class shape under its runtime export name.
fn class_constructor_aliases(module: &Module) -> Vec<(String, String)> {
    module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(declaration))) => {
                Some(declaration.as_ref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Var(declaration) => Some(declaration.as_ref()),
                _ => None,
            },
            _ => None,
        })
        .flat_map(|declaration| &declaration.decls)
        .filter_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            let TsType::TsTypeQuery(query) = binding.type_ann.as_ref()?.type_ann.as_ref() else {
                return None;
            };
            let swc_ecma_ast::TsTypeQueryExpr::TsEntityName(target) = &query.expr_name else {
                return None;
            };
            let target = match target {
                TsEntityName::Ident(target) => target.sym.to_string(),
                TsEntityName::TsQualifiedName(target) => target.right.sym.to_string(),
            };
            Some((binding.id.sym.to_string(), target))
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
            DefaultDecl::Class(class) => class
                .ident
                .as_ref()
                .map(|ident| vec![(ident.sym.as_str(), class.class.as_ref())])
                .unwrap_or_default(),
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
            (binding.id.sym.to_string(), ty)
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
            for parameter in &function.params {
                let TsFnParam::Ident(parameter) = parameter else {
                    continue;
                };
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
            let ret = contextual_dynamic_type(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic,
                true,
                in_progress,
            );
            if optional.iter().any(|optional| *optional) {
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
) -> DtsClass {
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
            let name = match entity {
                TsEntityName::Ident(name) => name.sym.to_string(),
                TsEntityName::TsQualifiedName(name) => name.right.sym.to_string(),
            };
            Some((parameter.name.sym.to_string(), name))
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
    let extends = class
        .super_class
        .as_deref()
        .and_then(|super_class| match super_class {
            Expr::Ident(name) => Some(name.sym.to_string()),
            _ => None,
        });
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
                params: lower_class_params(&constructor.params, interfaces, generic_interfaces),
                required_params: constructor
                    .params
                    .iter()
                    .take_while(|param| class_param_is_required(param))
                    .count(),
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
                        substitution.insert(parameter.name.sym.to_string(), HirType::JsValue);
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
                    if hir_type_contains_callback(&contextual) {
                        *classified = DtsType::Native(contextual);
                    }
                }
                let rest_param = function.rest_param;
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
                    let function = lower_dts_fn_type(
                        &property_name,
                        function,
                        interfaces,
                        generic_interfaces,
                    );
                    methods.push(DtsMethod {
                        name: function.name,
                        params: function.params,
                        required_params: function.required_params,
                        rest_param: function.rest_param,
                        ret: function.ret,
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
