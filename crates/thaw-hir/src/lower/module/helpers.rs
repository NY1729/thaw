fn declaration_names_for_normalization(declaration: &Decl) -> Vec<String> {
    struct Collector(Vec<String>);
    impl Visit for Collector {
        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            self.0.push(binding.id.sym.to_string());
        }
    }
    let mut collector = Collector(Vec::new());
    declaration.visit_with(&mut collector);
    collector.0
}

fn class_property_name(name: &PropName) -> Result<Symbol, String> {
    match name {
        PropName::Ident(name) => Ok(name.sym.to_string()),
        PropName::Str(name) => Ok(name.value.to_string_lossy().into_owned()),
        PropName::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Ok(name.value.to_string_lossy().into_owned()),
            _ => Err("native class computed members require a string-literal name".into()),
        },
        _ => Err("native class members require an identifier or string-literal name".into()),
    }
}

/// Infers a class field's type from a self-contained literal initializer.
/// This runs before the class body is resolved, so identifiers, calls,
/// spreads, empty arrays, and mixed-element arrays still require an explicit
/// annotation rather than introducing a lowering-order dependency or guessing
/// a union layout.
fn infer_class_field_literal_type(expr: &Expr) -> Option<HirType> {
    match expr {
        Expr::Lit(Lit::Num(_)) => Some(HirType::F64),
        Expr::Lit(Lit::Str(_)) => Some(HirType::Str),
        Expr::Lit(Lit::Bool(_)) => Some(HirType::Bool),
        Expr::Paren(parenthesized) => infer_class_field_literal_type(&parenthesized.expr),
        Expr::Array(array) => {
            let elements = array
                .elems
                .iter()
                .map(|element| {
                    let element = element.as_ref()?;
                    element.spread.is_none().then_some(())?;
                    infer_class_field_literal_type(&element.expr)
                })
                .collect::<Option<Vec<_>>>()?;
            let first = elements.first()?.clone();
            elements
                .iter()
                .all(|element| element == &first)
                .then(|| HirType::Array(Box::new(first)))
        }
        Expr::Object(object) => {
            let mut fields = Vec::with_capacity(object.props.len());
            for property in &object.props {
                let PropOrSpread::Prop(property) = property else {
                    return None;
                };
                let Prop::KeyValue(property) = property.as_ref() else {
                    return None;
                };
                let name = class_property_name(&property.key).ok()?;
                if fields.iter().any(|(existing, _)| existing == &name) {
                    return None;
                }
                fields.push((
                    name,
                    infer_class_field_literal_type(&property.value)?,
                ));
            }
            Some(HirType::Object(fields))
        }
        _ => None,
    }
}

fn member_property_name(name: &MemberProp) -> Option<Symbol> {
    match name {
        MemberProp::Ident(name) => Some(name.sym.to_string()),
        MemberProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn ordinary_optional_chain_expression(chain: &swc_ecma_ast::OptChainExpr) -> Expr {
    match chain.base.as_ref() {
        OptChainBase::Member(member) => Expr::Member(member.clone()),
        OptChainBase::Call(call) => Expr::Call(CallExpr::from(call.clone())),
    }
}

fn ordinary_optional_expression(expression: &Expr) -> Expr {
    match expression {
        Expr::OptChain(chain) => ordinary_optional_chain_expression(chain),
        expression => expression.clone(),
    }
}

fn class_constructor_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_constructor")
}

fn class_initializer_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_initialize")
}

fn default_arity_symbol(symbol: &str, arity: usize) -> Symbol {
    format!("{symbol}__thawdefault_arity_{arity}")
}

fn omitted_parameter_symbol(symbol: &str, mask: usize) -> Symbol {
    format!("{symbol}__thawomitted_mask_{mask:x}")
}

fn pattern_is_omittable(pattern: &Pat) -> bool {
    matches!(pattern, Pat::Assign(_))
        || matches!(pattern, Pat::Ident(binding) if binding.id.optional)
}

fn optional_parameter_type(ty: HirType) -> HirType {
    match ty {
        HirType::Optional(_) | HirType::Nullish(_) => ty,
        HirType::Nullable(payload) => HirType::Nullish(payload),
        other => HirType::Optional(Box::new(other)),
    }
}

fn optional_parameter_mask(optional: &[bool]) -> HirOptionalMask {
    HirOptionalMask::from_bools(optional)
}

fn is_optional_parameter(mask: &HirOptionalMask, index: usize) -> bool {
    mask.contains(index)
}

fn omitted_parameter_value(ty: &HirType) -> Result<HirExpr, String> {
    match ty {
        HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
        HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
        other => Err(format!(
            "omittable callable parameter needs an undefined-capable ABI type, got {other:?}"
        )),
    }
}

fn trailing_omittable_start(patterns: &[Pat]) -> Option<usize> {
    let start = patterns
        .iter()
        .rposition(|pattern| !pattern_is_omittable(pattern))
        .map_or(0, |index| index + 1);
    (start < patterns.len()).then_some(start)
}

fn omitted_parameter_masks(patterns: &[Pat], receiver_count: usize) -> Result<Vec<usize>, String> {
    let omittable = patterns
        .iter()
        .enumerate()
        .filter_map(|(index, pattern)| {
            pattern_is_omittable(pattern).then_some(index + receiver_count)
        })
        .collect::<Vec<_>>();
    if omittable.len() > 16 {
        return Err("native class callables support at most 16 omittable parameters".into());
    }
    let mut masks = Vec::with_capacity((1usize << omittable.len()).saturating_sub(1));
    for subset in 1usize..(1usize << omittable.len()) {
        let mut mask = 0usize;
        for (bit, parameter) in omittable.iter().enumerate() {
            if subset & (1usize << bit) != 0 {
                mask |= 1usize << parameter;
            }
        }
        masks.push(mask);
    }
    Ok(masks)
}

fn native_rest_element(patterns: &[Pat], params: &[HirType]) -> Result<Option<HirType>, String> {
    if patterns
        .iter()
        .take(patterns.len().saturating_sub(1))
        .any(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Err("native class rest parameter must be last".into());
    }
    if !patterns
        .last()
        .is_some_and(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Ok(None);
    }
    match params.last() {
        Some(HirType::Array(element)) => Ok(Some(element.as_ref().clone())),
        Some(other) => Err(format!(
            "native class rest parameter needs an array annotation, got {other:?}"
        )),
        None => Err("native class rest parameter is missing its signature type".into()),
    }
}

fn native_rest_array(values: Vec<HirExpr>, element: &HirType) -> HirExpr {
    if values.is_empty() {
        HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element.clone())
    } else {
        HirExpr::ArrayLit(values)
    }
}

fn insert_omitted_parameter_signatures(
    signatures: &mut HashMap<Symbol, FnSignature>,
    symbol: &str,
    patterns: &[Pat],
    receiver_count: usize,
) -> Result<(), String> {
    let full = signatures[symbol].clone();
    for mask in omitted_parameter_masks(patterns, receiver_count)? {
        let mut wrapper = full.clone();
        wrapper.params = wrapper
            .params
            .into_iter()
            .enumerate()
            .filter_map(|(index, parameter)| (mask & (1usize << index) == 0).then_some(parameter))
            .collect();
        signatures.insert(omitted_parameter_symbol(symbol, mask), wrapper);
    }
    Ok(())
}

fn class_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_method_{method}")
}

fn unbound_class_method_symbol(method: &str) -> Symbol {
    format!("{method}__thaw_unbound")
}

fn class_static_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_static_{method}")
}

fn class_static_field_symbol(class: &str, field: &str) -> Symbol {
    format!("__thaw_class_{class}_static_field_{field}")
}

fn is_class_static_field_symbol(symbol: &str) -> bool {
    symbol.starts_with("__thaw_class_") && symbol.contains("_static_field_")
}

fn class_getter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_getter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_setter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_setter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_member_symbol(class: &str, member: &swc_ecma_ast::ClassMethod) -> Result<Symbol, String> {
    let name = class_property_name(&member.key)?;
    Ok(match member.kind {
        MethodKind::Getter => class_getter_symbol(class, &name, member.is_static),
        MethodKind::Setter => class_setter_symbol(class, &name, member.is_static),
        MethodKind::Method if member.is_static => class_static_method_symbol(class, &name),
        MethodKind::Method => class_method_symbol(class, &name),
    })
}

fn super_property_name(property: &SuperProp) -> Result<Symbol, String> {
    match property {
        SuperProp::Ident(name) => Ok(name.sym.to_string()),
        SuperProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Ok(name.value.to_string_lossy().into_owned()),
            _ => Err("computed super properties require a string literal name".into()),
        },
    }
}

fn class_name_from_type(ty: &HirType) -> Option<&str> {
    let HirType::Object(fields) = ty else {
        return None;
    };
    fields.first().and_then(|(name, ty)| {
        (*ty == HirType::Bool)
            .then(|| name.strip_prefix("__thaw_class_identity_"))
            .flatten()
            .and_then(|identities| identities.split('$').next())
    })
}

fn class_type_has_identity(ty: &HirType, expected: &str) -> bool {
    let HirType::Object(fields) = ty else {
        return false;
    };
    fields.first().is_some_and(|(name, ty)| {
        *ty == HirType::Bool
            && name
                .strip_prefix("__thaw_class_identity_")
                .is_some_and(|identities| identities.split('$').any(|name| name == expected))
    })
}

fn class_constructor_param_pattern(parameter: &ParamOrTsParamProp) -> Pat {
    match parameter {
        ParamOrTsParamProp::Param(parameter) => parameter.pat.clone(),
        ParamOrTsParamProp::TsParamProp(property) => match &property.param {
            TsParamPropParam::Ident(binding) => Pat::Ident(binding.clone()),
            TsParamPropParam::Assign(assignment) => Pat::Assign(assignment.clone()),
        },
    }
}

fn parameter_property_binding(
    property: &swc_ecma_ast::TsParamProp,
) -> Result<&swc_ecma_ast::BindingIdent, String> {
    match &property.param {
        TsParamPropParam::Ident(binding) => Ok(binding),
        TsParamPropParam::Assign(assignment) => match assignment.left.as_ref() {
            Pat::Ident(binding) => Ok(binding),
            _ => Err("constructor parameter properties require identifier bindings".into()),
        },
    }
}
