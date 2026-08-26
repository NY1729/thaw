type UnionDiscriminants = HashMap<Symbol, Vec<Option<HirLit>>>;
type NestedArrayDiscriminants = HashMap<usize, UnionDiscriminants>;
type ObjectArrayPropertyDiscriminants = HashMap<Vec<Symbol>, UnionDiscriminants>;

#[derive(Clone, PartialEq)]
struct FunctionPropertyDiscriminants {
    value: Option<UnionDiscriminants>,
    array: Option<UnionDiscriminants>,
    nested_array: NestedArrayDiscriminants,
    object: ObjectArrayPropertyDiscriminants,
    functions: ObjectFunctionPropertyDiscriminants,
}

type ObjectFunctionPropertyDiscriminants = HashMap<Vec<Symbol>, FunctionPropertyDiscriminants>;

fn method_signature_function_type(
    method: &swc_ecma_ast::TsMethodSignature,
) -> Result<TsType, String> {
    let type_ann = method
        .type_ann
        .clone()
        .ok_or("interface method needs an explicit return type")?;
    Ok(TsType::TsFnOrConstructorType(
        TsFnOrConstructorType::TsFnType(swc_ecma_ast::TsFnType {
            span: method.span,
            params: method.params.clone(),
            type_params: method.type_params.clone(),
            type_ann,
        }),
    ))
}

fn replace_metadata<T>(metadata: &mut HashMap<Symbol, T>, name: &str, value: Option<T>) {
    if let Some(value) = value {
        metadata.insert(name.to_string(), value);
    } else {
        metadata.remove(name);
    }
}

#[derive(Default)]
/// Non-generic interfaces (fully resolved up front into `HirType::Object`,
/// the first map) plus generic interfaces (kept raw and resolved on demand).
struct GenericInterfaces<'a> {
    interfaces: HashMap<Symbol, &'a TsInterfaceDecl>,
    plain_interfaces: HashMap<Symbol, &'a TsInterfaceDecl>,
    aliases: HashMap<Symbol, &'a swc_ecma_ast::TsTypeAliasDecl>,
    plain_aliases: HashMap<Symbol, &'a swc_ecma_ast::TsTypeAliasDecl>,
    function_aliases: HashMap<Symbol, &'a swc_ecma_ast::TsTypeAliasDecl>,
    function_alias_chains: HashMap<Symbol, Symbol>,
    function_interfaces: HashMap<Symbol, &'a TsInterfaceDecl>,
    function_interface_chains: HashMap<Symbol, Symbol>,
    function_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_array_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    function_object_array_property_discriminants: HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    function_object_function_property_discriminants:
        HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
}

impl GenericInterfaces<'_> {
    fn new() -> Self {
        Self::default()
    }
}

struct GenericMetadataTypeMaterializer<'a, 'ast> {
    generic: &'a GenericInterfaces<'ast>,
    substitutions: HashMap<Symbol, Box<TsType>>,
    visiting: HashSet<Symbol>,
}

impl VisitMut for GenericMetadataTypeMaterializer<'_, '_> {
    fn visit_mut_ts_type(&mut self, ty: &mut TsType) {
        let TsType::TsTypeRef(reference) = ty else {
            ty.visit_mut_children_with(self);
            return;
        };
        let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else {
            ty.visit_mut_children_with(self);
            return;
        };
        if reference.type_params.is_none() {
            let substitution_name = name.sym.to_string();
            if let Some(replacement) = self.substitutions.remove(&substitution_name) {
                let original = replacement.clone();
                *ty = *replacement;
                ty.visit_mut_with(self);
                self.substitutions.insert(substitution_name, original);
                return;
            }
        }
        let Some(interface) = self.generic.interfaces.get(name.sym.as_ref()) else {
            ty.visit_mut_children_with(self);
            return;
        };
        if !self.visiting.insert(name.sym.to_string()) {
            return;
        }
        let parameters = &interface
            .type_params
            .as_ref()
            .expect("generic metadata interface parameters")
            .params;
        let arguments = reference
            .type_params
            .as_ref()
            .map(|arguments| arguments.params.as_slice())
            .unwrap_or_default();
        if arguments.len() > parameters.len()
            || parameters
                .iter()
                .skip(arguments.len())
                .any(|parameter| parameter.default.is_none())
        {
            self.visiting.remove(name.sym.as_ref());
            return;
        }
        let outer = self.substitutions.clone();
        let mut nested = outer.clone();
        for (index, parameter) in parameters.iter().enumerate() {
            let mut argument = arguments
                .get(index)
                .cloned()
                .or_else(|| parameter.default.clone())
                .expect("validated generic metadata argument");
            self.substitutions = nested.clone();
            argument.visit_mut_with(self);
            nested.insert(parameter.name.sym.to_string(), argument);
        }
        self.substitutions = nested;
        let mut members = Vec::new();
        for base in &interface.extends {
            let Expr::Ident(base_name) = base.expr.as_ref() else {
                continue;
            };
            let mut base = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_name.clone()),
                type_params: base.type_args.clone(),
            });
            base.visit_mut_with(self);
            if let TsType::TsTypeLit(base) = base {
                members.extend(base.members);
            }
        }
        let mut own = interface.body.body.clone();
        own.visit_mut_with(self);
        members.extend(own);
        self.substitutions = outer;
        self.visiting.remove(name.sym.as_ref());
        *ty = TsType::TsTypeLit(swc_ecma_ast::TsTypeLit {
            span: interface.span,
            members,
        });
    }
}

fn materialize_generic_metadata_type(ty: &TsType, generic: &GenericInterfaces<'_>) -> TsType {
    let mut ty = ty.clone();
    ty.visit_mut_with(&mut GenericMetadataTypeMaterializer {
        generic,
        substitutions: HashMap::new(),
        visiting: HashSet::new(),
    });
    ty
}

fn strip_parenthesized_ts_type(mut ty: &TsType) -> &TsType {
    loop {
        ty = match ty {
            TsType::TsParenthesizedType(parenthesized) => &parenthesized.type_ann,
            TsType::TsTypeOperator(operator)
                if operator.op == swc_ecma_ast::TsTypeOperatorOp::ReadOnly =>
            {
                &operator.type_ann
            }
            _ => return ty,
        };
    }
}

fn resolve_plain_alias_type<'a>(
    mut ty: &'a TsType,
    generic: &'a GenericInterfaces<'a>,
) -> Option<&'a TsType> {
    let mut visited = HashSet::new();
    loop {
        ty = strip_parenthesized_ts_type(ty);
        let TsType::TsTypeRef(reference) = ty else {
            return Some(ty);
        };
        if reference.type_params.is_some() {
            return None;
        }
        let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else {
            return None;
        };
        if !visited.insert(name.sym.to_string()) {
            return None;
        }
        ty = &generic.plain_aliases.get(name.sym.as_ref())?.type_ann;
    }
}

fn discriminant_literal(ty: &TsType, generic: &GenericInterfaces<'_>) -> Option<HirLit> {
    let TsType::TsLitType(literal) = resolve_plain_alias_type(ty, generic)? else {
        return None;
    };
    match &literal.lit {
        TsLit::Str(value) => Some(HirLit::Str(value.value.to_string_lossy().into_owned())),
        TsLit::Number(value) => Some(HirLit::F64(value.value)),
        TsLit::Bool(value) => Some(HirLit::Bool(value.value)),
        _ => None,
    }
}

fn object_union_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> HashMap<Symbol, Vec<Option<HirLit>>> {
    let raw = strip_parenthesized_ts_type(ty);
    let mut resolved = resolve_plain_alias_type(raw, generic);
    if let TsType::TsTypeRef(reference) = raw {
        if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name) if name.sym == *"Promise")
        {
            if let Some(arguments) = &reference.type_params {
                if let [inner] = arguments.params.as_slice() {
                    resolved = resolve_plain_alias_type(inner, generic);
                }
            }
        }
    }
    let Some(TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union))) =
        resolved
    else {
        return HashMap::new();
    };
    let mut by_property: HashMap<Symbol, Vec<Option<HirLit>>> = HashMap::new();
    for (index, member) in union.types.iter().enumerate() {
        let Some(TsType::TsTypeLit(object)) = resolve_plain_alias_type(member, generic) else {
            return HashMap::new();
        };
        for property in &object.members {
            let TsTypeElement::TsPropertySignature(property) = property else {
                continue;
            };
            let name = match property.key.as_ref() {
                Expr::Ident(name) => name.sym.to_string(),
                Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                _ => continue,
            };
            let literal = property
                .type_ann
                .as_ref()
                .and_then(|annotation| discriminant_literal(&annotation.type_ann, generic));
            by_property
                .entry(name)
                .or_insert_with(|| vec![None; union.types.len()])[index] = literal;
        }
    }
    by_property.retain(|_, values| {
        values.iter().all(Option::is_some)
            && values
                .iter()
                .enumerate()
                .all(|(index, value)| values[..index].iter().all(|previous| previous != value))
    });
    by_property
}

fn array_element_union_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> HashMap<Symbol, Vec<Option<HirLit>>> {
    let raw = strip_parenthesized_ts_type(ty);
    let unwrapped = if let TsType::TsTypeRef(reference) = raw {
        if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name) if name.sym == *"Promise")
        {
            reference
                .type_params
                .as_ref()
                .and_then(|arguments| arguments.params.first())
                .map(AsRef::as_ref)
                .unwrap_or(raw)
        } else {
            raw
        }
    } else {
        raw
    };
    let Some(resolved) = resolve_plain_alias_type(unwrapped, generic) else {
        return HashMap::new();
    };
    let element = match resolved {
        TsType::TsArrayType(array) => Some(array.elem_type.as_ref()),
        TsType::TsTypeRef(reference)
            if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name)
                if name.sym == *"Array" || name.sym == *"ReadonlyArray") =>
        {
            reference
                .type_params
                .as_ref()
                .and_then(|arguments| arguments.params.first())
                .map(AsRef::as_ref)
        }
        _ => None,
    };
    element
        .map(|element| object_union_discriminants(element, generic))
        .unwrap_or_default()
}

fn nested_array_union_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> NestedArrayDiscriminants {
    let mut current = strip_parenthesized_ts_type(ty);
    if let TsType::TsTypeRef(reference) = current {
        if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name) if name.sym == *"Promise")
        {
            if let Some(inner) = reference
                .type_params
                .as_ref()
                .and_then(|arguments| arguments.params.first())
            {
                current = inner;
            }
        }
    }
    let mut result = NestedArrayDiscriminants::new();
    for depth in 1.. {
        let Some(resolved) = resolve_plain_alias_type(current, generic) else {
            break;
        };
        let element = match resolved {
            TsType::TsArrayType(array) => Some(array.elem_type.as_ref()),
            TsType::TsTypeRef(reference)
                if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name)
                    if name.sym == *"Array" || name.sym == *"ReadonlyArray") =>
            {
                reference
                    .type_params
                    .as_ref()
                    .and_then(|arguments| arguments.params.first())
                    .map(AsRef::as_ref)
            }
            _ => None,
        };
        let Some(element) = element else {
            break;
        };
        let discriminants = object_union_discriminants(element, generic);
        if !discriminants.is_empty() {
            result.insert(depth, discriminants);
        }
        current = element;
    }
    result
}

fn object_array_property_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> ObjectArrayPropertyDiscriminants {
    fn collect(
        ty: &TsType,
        generic: &GenericInterfaces<'_>,
        prefix: &mut Vec<Symbol>,
        visiting: &mut HashSet<Symbol>,
        result: &mut ObjectArrayPropertyDiscriminants,
    ) {
        let raw = strip_parenthesized_ts_type(ty);
        let unwrapped = if let TsType::TsTypeRef(reference) = raw {
            if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name) if name.sym == *"Promise")
            {
                reference
                    .type_params
                    .as_ref()
                    .and_then(|arguments| arguments.params.first())
                    .map(AsRef::as_ref)
                    .unwrap_or(raw)
            } else {
                raw
            }
        } else {
            raw
        };
        let resolved = resolve_plain_alias_type(unwrapped, generic).unwrap_or(unwrapped);
        let (members, visited_name) = match resolved {
            TsType::TsTypeLit(object) => (object.members.as_slice(), None),
            TsType::TsTypeRef(reference) => {
                let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else {
                    return;
                };
                let name = name.sym.to_string();
                if !visiting.insert(name.clone()) {
                    return;
                }
                let Some(interface) = generic
                    .interfaces
                    .get(&name)
                    .or_else(|| generic.plain_interfaces.get(&name))
                else {
                    visiting.remove(&name);
                    return;
                };
                for base in &interface.extends {
                    let Expr::Ident(base_name) = base.expr.as_ref() else {
                        continue;
                    };
                    let base = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                        span: base.span,
                        type_name: swc_ecma_ast::TsEntityName::Ident(base_name.clone()),
                        type_params: base.type_args.clone(),
                    });
                    collect(&base, generic, prefix, visiting, result);
                }
                (interface.body.body.as_slice(), Some(name))
            }
            _ => return,
        };
        for member in members {
            let TsTypeElement::TsPropertySignature(property) = member else {
                continue;
            };
            let name = match property.key.as_ref() {
                Expr::Ident(name) => name.sym.to_string(),
                Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                _ => continue,
            };
            let Some(annotation) = &property.type_ann else {
                continue;
            };
            prefix.push(name);
            let discriminants = array_element_union_discriminants(&annotation.type_ann, generic);
            if discriminants.is_empty() {
                collect(&annotation.type_ann, generic, prefix, visiting, result);
            } else {
                result.insert(prefix.clone(), discriminants);
            }
            prefix.pop();
        }
        if let Some(name) = visited_name {
            visiting.remove(&name);
        }
    }

    let ty = materialize_generic_metadata_type(ty, generic);
    let mut result = HashMap::new();
    collect(
        &ty,
        generic,
        &mut Vec::new(),
        &mut HashSet::new(),
        &mut result,
    );
    result
}

fn function_return_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> HashMap<Symbol, Vec<Option<HirLit>>> {
    let Some(resolved) = resolve_plain_alias_type(ty, generic) else {
        return HashMap::new();
    };
    let result = match resolved {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    };
    result
        .map(|result| object_union_discriminants(result, generic))
        .unwrap_or_default()
}

fn function_return_array_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> HashMap<Symbol, Vec<Option<HirLit>>> {
    let Some(resolved) = resolve_plain_alias_type(ty, generic) else {
        return HashMap::new();
    };
    let result = match resolved {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    };
    result
        .map(|result| array_element_union_discriminants(result, generic))
        .unwrap_or_default()
}

fn function_return_nested_array_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> NestedArrayDiscriminants {
    let Some(resolved) = resolve_plain_alias_type(ty, generic) else {
        return HashMap::new();
    };
    let result = match resolved {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    };
    result
        .map(|result| nested_array_union_discriminants(result, generic))
        .unwrap_or_default()
}

fn function_return_object_array_property_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> ObjectArrayPropertyDiscriminants {
    let Some(resolved) = resolve_plain_alias_type(ty, generic) else {
        return HashMap::new();
    };
    let result = match resolved {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    };
    result
        .map(|result| object_array_property_discriminants(result, generic))
        .unwrap_or_default()
}

fn function_return_type_annotation<'a>(
    ty: &'a TsType,
    generic: &'a GenericInterfaces<'a>,
) -> Option<&'a TsType> {
    match resolve_plain_alias_type(ty, generic)? {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

fn function_return_object_function_property_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> ObjectFunctionPropertyDiscriminants {
    let Some(resolved) = resolve_plain_alias_type(ty, generic) else {
        return HashMap::new();
    };
    let result = match resolved {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            Some(function.type_ann.type_ann.as_ref())
        }
        TsType::TsTypeLit(literal) => match literal.members.as_slice() {
            [TsTypeElement::TsCallSignatureDecl(call)] => call
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.as_ref()),
            _ => None,
        },
        _ => None,
    };
    result
        .map(|result| object_function_property_discriminants(result, generic))
        .unwrap_or_default()
}

fn object_function_property_discriminants(
    ty: &TsType,
    generic: &GenericInterfaces<'_>,
) -> ObjectFunctionPropertyDiscriminants {
    fn collect(
        ty: &TsType,
        generic: &GenericInterfaces<'_>,
        prefix: &mut Vec<Symbol>,
        visiting: &mut HashSet<Symbol>,
        result: &mut ObjectFunctionPropertyDiscriminants,
    ) {
        let raw = strip_parenthesized_ts_type(ty);
        let unwrapped = if let TsType::TsTypeRef(reference) = raw {
            if matches!(&reference.type_name, swc_ecma_ast::TsEntityName::Ident(name) if name.sym == *"Promise")
            {
                reference
                    .type_params
                    .as_ref()
                    .and_then(|arguments| arguments.params.first())
                    .map(AsRef::as_ref)
                    .unwrap_or(raw)
            } else {
                raw
            }
        } else {
            raw
        };
        let resolved = resolve_plain_alias_type(unwrapped, generic).unwrap_or(unwrapped);
        let (members, visited_name) = match resolved {
            TsType::TsTypeLit(object) => (object.members.as_slice(), None),
            TsType::TsTypeRef(reference) => {
                let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else {
                    return;
                };
                let name = name.sym.to_string();
                if !visiting.insert(name.clone()) {
                    return;
                }
                let Some(interface) = generic
                    .interfaces
                    .get(&name)
                    .or_else(|| generic.plain_interfaces.get(&name))
                else {
                    visiting.remove(&name);
                    return;
                };
                for base in &interface.extends {
                    let Expr::Ident(base_name) = base.expr.as_ref() else {
                        continue;
                    };
                    let base = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                        span: base.span,
                        type_name: swc_ecma_ast::TsEntityName::Ident(base_name.clone()),
                        type_params: base.type_args.clone(),
                    });
                    collect(&base, generic, prefix, visiting, result);
                }
                (interface.body.body.as_slice(), Some(name))
            }
            _ => return,
        };
        for member in members {
            let (key, annotation) = match member {
                TsTypeElement::TsPropertySignature(property) => {
                    let Some(annotation) = &property.type_ann else {
                        continue;
                    };
                    (
                        property.key.as_ref(),
                        std::borrow::Cow::Borrowed(annotation.type_ann.as_ref()),
                    )
                }
                TsTypeElement::TsMethodSignature(method) => (
                    method.key.as_ref(),
                    std::borrow::Cow::Owned(match method_signature_function_type(method) {
                        Ok(ty) => ty,
                        Err(_) => continue,
                    }),
                ),
                _ => continue,
            };
            let name = match key {
                Expr::Ident(name) => name.sym.to_string(),
                Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                _ => continue,
            };
            prefix.push(name);
            let value = function_return_discriminants(&annotation, generic);
            let array = function_return_array_discriminants(&annotation, generic);
            let nested_array = function_return_nested_array_discriminants(&annotation, generic);
            let object = function_return_object_array_property_discriminants(&annotation, generic);
            let mut functions = ObjectFunctionPropertyDiscriminants::new();
            if let Some(return_type) = function_return_type_annotation(&annotation, generic) {
                collect(
                    return_type,
                    generic,
                    &mut Vec::new(),
                    visiting,
                    &mut functions,
                );
            }
            if !value.is_empty()
                || !array.is_empty()
                || !nested_array.is_empty()
                || !object.is_empty()
                || !functions.is_empty()
            {
                result.insert(
                    prefix.clone(),
                    FunctionPropertyDiscriminants {
                        value: (!value.is_empty()).then_some(value),
                        array: (!array.is_empty()).then_some(array),
                        nested_array,
                        object,
                        functions,
                    },
                );
            } else {
                collect(&annotation, generic, prefix, visiting, result);
            }
            prefix.pop();
        }
        if let Some(name) = visited_name {
            visiting.remove(&name);
        }
    }

    let ty = materialize_generic_metadata_type(ty, generic);
    let mut result = HashMap::new();
    collect(
        &ty,
        generic,
        &mut Vec::new(),
        &mut HashSet::new(),
        &mut result,
    );
    result
}

fn function_expression_as_arrow(
    expression: &swc_ecma_ast::FnExpr,
) -> Result<swc_ecma_ast::ArrowExpr, String> {
    let function = expression.function.as_ref();
    if function.this_param.is_some()
        || !function.decorators.is_empty()
        || function
            .params
            .iter()
            .any(|parameter| !parameter.decorators.is_empty())
    {
        return Err(
            "function expressions with this parameters or decorators are not supported".into(),
        );
    }
    let body = function
        .body
        .as_ref()
        .ok_or("function expression needs a body")?;
    Ok(swc_ecma_ast::ArrowExpr {
        span: function.span,
        ctxt: function.ctxt,
        params: function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect(),
        body: Box::new(ArrowFunctionBody::FunctionBody(body.clone())),
        is_async: function.is_async,
        is_generator: function.is_generator,
        type_params: function.type_params.clone(),
        return_type: function.return_type.clone(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InferredGenericReturn {
    Parameter(usize),
    Keyword(TsKeywordTypeKind),
}

fn expression_is_definitely_string(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(Lit::Str(_)) | Expr::Tpl(_) => true,
        Expr::Paren(parenthesized) => expression_is_definitely_string(&parenthesized.expr),
        Expr::TsAs(assertion) => expression_is_definitely_string(&assertion.expr),
        Expr::TsTypeAssertion(assertion) => expression_is_definitely_string(&assertion.expr),
        Expr::Call(call) => matches!(
            &call.callee,
            Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == *"String")
        ),
        _ => false,
    }
}

fn inferred_generic_return(expr: &Expr, params: &[Pat]) -> Option<InferredGenericReturn> {
    let returned = match expr {
        Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
        Expr::TsAs(assertion) => assertion.expr.as_ref(),
        Expr::TsTypeAssertion(assertion) => assertion.expr.as_ref(),
        expression => expression,
    };
    if let Expr::Cond(conditional) = returned {
        let consequent = inferred_generic_return(&conditional.cons, params)?;
        let alternate = inferred_generic_return(&conditional.alt, params)?;
        return (consequent == alternate).then_some(consequent);
    }
    if let Expr::Bin(binary) = returned {
        if matches!(
            binary.op,
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::NullishCoalescing
        ) {
            let left = inferred_generic_return(&binary.left, params)?;
            let right = inferred_generic_return(&binary.right, params)?;
            return (left == right).then_some(left);
        }
    }
    let keyword = match returned {
        Expr::Lit(Lit::Num(_)) => Some(TsKeywordTypeKind::TsNumberKeyword),
        Expr::Lit(Lit::Str(_)) => Some(TsKeywordTypeKind::TsStringKeyword),
        Expr::Lit(Lit::Bool(_)) => Some(TsKeywordTypeKind::TsBooleanKeyword),
        Expr::Lit(Lit::Null(_)) => Some(TsKeywordTypeKind::TsNullKeyword),
        Expr::Tpl(_) => Some(TsKeywordTypeKind::TsStringKeyword),
        Expr::Unary(unary) => match unary.op {
            UnaryOp::Bang => Some(TsKeywordTypeKind::TsBooleanKeyword),
            UnaryOp::TypeOf => Some(TsKeywordTypeKind::TsStringKeyword),
            UnaryOp::Void => Some(TsKeywordTypeKind::TsUndefinedKeyword),
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Tilde => {
                Some(TsKeywordTypeKind::TsNumberKeyword)
            }
            _ => None,
        },
        Expr::Bin(binary)
            if matches!(
                binary.op,
                BinaryOp::EqEq
                    | BinaryOp::NotEq
                    | BinaryOp::EqEqEq
                    | BinaryOp::NotEqEq
                    | BinaryOp::Lt
                    | BinaryOp::LtEq
                    | BinaryOp::Gt
                    | BinaryOp::GtEq
                    | BinaryOp::In
                    | BinaryOp::InstanceOf
            ) =>
        {
            Some(TsKeywordTypeKind::TsBooleanKeyword)
        }
        Expr::Bin(binary)
            if matches!(
                binary.op,
                BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Exp
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitAnd
                    | BinaryOp::LShift
                    | BinaryOp::RShift
                    | BinaryOp::ZeroFillRShift
            ) =>
        {
            Some(TsKeywordTypeKind::TsNumberKeyword)
        }
        Expr::Bin(binary)
            if binary.op == BinaryOp::Add
                && (expression_is_definitely_string(&binary.left)
                    || expression_is_definitely_string(&binary.right)) =>
        {
            Some(TsKeywordTypeKind::TsStringKeyword)
        }
        Expr::Call(call) => match &call.callee {
            Callee::Expr(callee) => match callee.as_ref() {
                Expr::Ident(identifier) if identifier.sym == *"String" => {
                    Some(TsKeywordTypeKind::TsStringKeyword)
                }
                Expr::Ident(identifier) if identifier.sym == *"Number" => {
                    Some(TsKeywordTypeKind::TsNumberKeyword)
                }
                Expr::Ident(identifier) if identifier.sym == *"Boolean" => {
                    Some(TsKeywordTypeKind::TsBooleanKeyword)
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };
    if let Some(keyword) = keyword {
        return Some(InferredGenericReturn::Keyword(keyword));
    }
    let Expr::Ident(returned) = returned else {
        return None;
    };
    let parameter = params.iter().enumerate().find_map(|(index, parameter)| {
        let Pat::Ident(binding) = parameter else {
            return None;
        };
        if binding.id.sym != returned.sym {
            return None;
        }
        let keyword = binding.type_ann.as_ref().and_then(|annotation| {
            let TsType::TsKeywordType(keyword) = strip_parenthesized_ts_type(&annotation.type_ann)
            else {
                return None;
            };
            Some(keyword.kind)
        });
        Some(
            keyword
                .map(InferredGenericReturn::Keyword)
                .unwrap_or(InferredGenericReturn::Parameter(index)),
        )
    });
    parameter.or_else(|| {
        (returned.sym == *"undefined").then_some(InferredGenericReturn::Keyword(
            TsKeywordTypeKind::TsUndefinedKeyword,
        ))
    })
}

fn collect_generic_return_parameters(
    statement: &Stmt,
    params: &[Pat],
    returned: &mut Vec<InferredGenericReturn>,
) -> bool {
    match statement {
        Stmt::Return(return_statement) => return_statement
            .arg
            .as_deref()
            .and_then(|expr| inferred_generic_return(expr, params))
            .map(|index| returned.push(index))
            .is_some(),
        Stmt::If(if_statement) => {
            collect_generic_return_parameters(&if_statement.cons, params, returned)
                && if_statement.alt.as_deref().is_none_or(|alternate| {
                    collect_generic_return_parameters(alternate, params, returned)
                })
        }
        Stmt::Block(block) => block
            .stmts
            .iter()
            .all(|statement| collect_generic_return_parameters(statement, params, returned)),
        Stmt::Empty(_) => true,
        _ => false,
    }
}

fn inferred_generic_arrow_return_type(arrow: &swc_ecma_ast::ArrowExpr) -> Option<Box<TsType>> {
    let inferred = match arrow.body.as_ref() {
        ArrowFunctionBody::Expr(expression) => inferred_generic_return(expression, &arrow.params)?,
        ArrowFunctionBody::FunctionBody(body) => {
            let mut returned = Vec::new();
            if !body.stmts.iter().all(|statement| {
                collect_generic_return_parameters(statement, &arrow.params, &mut returned)
            }) {
                return None;
            }
            let first = *returned.first()?;
            returned
                .iter()
                .all(|index| *index == first)
                .then_some(first)?
        }
    };
    match inferred {
        InferredGenericReturn::Parameter(index) => {
            let Pat::Ident(binding) = &arrow.params[index] else {
                return None;
            };
            binding
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.clone())
        }
        InferredGenericReturn::Keyword(kind) => Some(Box::new(TsType::TsKeywordType(
            swc_ecma_ast::TsKeywordType {
                span: swc_common::DUMMY_SP,
                kind,
            },
        ))),
    }
}
