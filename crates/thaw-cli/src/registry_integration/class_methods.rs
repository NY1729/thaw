#[cfg(test)]
fn rewrite_external_class_methods(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
) -> Result<String, String> {
    rewrite_external_class_methods_with_static(
        source, classes, methods, &[], &[], &[], &[], &[], &[], &[],
    )
}

#[cfg(test)]
fn rewrite_fallback_function_overloads(
    source: &str,
    functions: &[FallbackFunctionOverloadRewrite],
) -> Result<String, String> {
    rewrite_external_class_methods_with_static(
        source, &[], &[], &[], &[], &[], &[], &[], &[], functions,
    )
}

#[allow(clippy::too_many_arguments)]
fn rewrite_external_class_methods_with_static(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
    static_methods: &[StaticClassMethodRewrite],
    getters: &[ClassGetterRewrite],
    setters: &[ClassSetterRewrite],
    static_getters: &[StaticClassGetterRewrite],
    static_setters: &[StaticClassSetterRewrite],
    factories: &[FactoryClassRewrite],
    functions: &[FallbackFunctionOverloadRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowFunctionBody, AssignExpr, AssignOp, AssignTarget, BinaryOp, BreakStmt, CallExpr,
        Callee, DoWhileStmt, Expr, FnDecl, ForInStmt, ForOfStmt, ForStmt, FunctionBody, IfStmt,
        Lit, MemberProp, NewExpr, Pat, Prop, PropName, PropOrSpread, ReturnStmt, SimpleAssignTarget,
        Function, Stmt, SwitchStmt, TryStmt, TsEntityName, TsInterfaceDecl, TsKeywordTypeKind,
        TsLit, TsType, TsTypeAliasDecl, TsTypeElement, TsTypeOperatorOp,
        TsUnionOrIntersectionType, UnaryOp, VarDeclarator, WhileStmt,
    };
    use thaw_parser::common::Spanned;

    if classes.is_empty()
        && methods.is_empty()
        && static_methods.is_empty()
        && getters.is_empty()
        && setters.is_empty()
        && static_getters.is_empty()
        && static_setters.is_empty()
        && factories.is_empty()
        && functions.is_empty()
    {
        return Ok(source.to_string());
    }

    fn constructed_class<'a>(
        expression: &'a Expr,
        classes: &'a [ClassConstructorRewrite],
    ) -> Option<&'a str> {
        let Expr::New(new_expression) = expression else {
            return None;
        };
        match new_expression.callee.as_ref() {
            Expr::Ident(class) => classes
                .iter()
                .find(|(_, name, _)| name == class.sym.as_str())
                .map(|(_, name, _)| name.as_str()),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(package), MemberProp::Ident(class)) => classes
                    .iter()
                    .find(|(qualifier, name, _)| {
                        qualifier == package.sym.as_str() && name == class.sym.as_str()
                    })
                    .map(|(_, name, _)| name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    /// A bare call to a registered Fallback factory function (real
    /// example: dayjs's `dayjs(...)`, whose declared return type
    /// `dayjs.Dayjs` names its own `Dayjs` class) -- tracked as
    /// producing a class instance the same way `constructed_class`
    /// tracks `new ClassName(...)`, even though nothing about the call
    /// syntax itself says `new`. Matched by bare callee name only, same
    /// simplification `constructed_class` already makes for a bare `new
    /// ClassName(...)` (no import/qualifier resolution).
    fn factory_call_class<'a>(
        expression: &'a Expr,
        factories: &'a [FactoryClassRewrite],
    ) -> Option<&'a str> {
        let Expr::Call(call) = expression else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let function = match callee.as_ref() {
            Expr::Ident(function) => function.sym.as_str(),
            Expr::Member(member) => match &member.prop {
                MemberProp::Ident(function) => function.sym.as_str(),
                _ => return None,
            },
            _ => return None,
        };
        factories
            .iter()
            .find(|(name, _)| name == function)
            .map(|(_, class)| class.as_str())
    }

    fn source_instance_class(
        expression: &Expr,
        classes: &[ClassConstructorRewrite],
        factories: &[FactoryClassRewrite],
        variables: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => variables.get(identifier.sym.as_str()).cloned(),
            Expr::Member(member) => member_assignment_path(member)
                .map(|(root, path)| instance_path_key(&root, &path))
                .and_then(|key| variables.get(&key).cloned()),
            Expr::Paren(parenthesized) => {
                source_instance_class(&parenthesized.expr, classes, factories, variables)
            }
            Expr::TsAs(assertion) => {
                source_instance_class(&assertion.expr, classes, factories, variables)
            }
            Expr::TsTypeAssertion(assertion) => {
                source_instance_class(&assertion.expr, classes, factories, variables)
            }
            _ => constructed_class(expression, classes)
                .or_else(|| factory_call_class(expression, factories))
                .map(str::to_owned),
        }
    }

    fn instance_path_key(root: &str, path: &[String]) -> String {
        let mut key = root.to_string();
        for property in path {
            key.push('\u{1f}');
            key.push_str(property);
        }
        key
    }

    fn invalidate_instance_path(
        variables: &mut std::collections::HashMap<String, String>,
        key: &str,
    ) {
        let descendant = format!("{key}\u{1f}");
        variables.retain(|candidate, _| candidate != key && !candidate.starts_with(&descendant));
    }

    fn instance_receiver(expression: &Expr) -> Option<(String, String)> {
        match expression {
            Expr::Ident(identifier) => {
                let name = identifier.sym.to_string();
                Some((name.clone(), name))
            }
            Expr::Member(member) => {
                let (key, rendered) = instance_receiver(&member.obj)?;
                match &member.prop {
                    MemberProp::Ident(property) => Some((
                        format!("{key}\u{1f}{}", property.sym),
                        format!("{rendered}.{}", property.sym),
                    )),
                    MemberProp::Computed(computed) => {
                        let Expr::Lit(Lit::Str(property)) = computed.expr.as_ref() else {
                            return None;
                        };
                        let property = property.value.to_string_lossy().into_owned();
                        Some((
                            format!("{key}\u{1f}{property}"),
                            format!("{rendered}[{}]", serde_json::to_string(&property).ok()?),
                        ))
                    }
                    MemberProp::PrivateName(_) => None,
                }
            }
            Expr::Paren(parenthesized) => instance_receiver(&parenthesized.expr),
            Expr::TsAs(assertion) => instance_receiver(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => instance_receiver(&assertion.expr),
            _ => None,
        }
    }

    fn static_class_receiver(expression: &Expr) -> Option<(Option<String>, String)> {
        match expression {
            Expr::Ident(class) => Some((None, class.sym.to_string())),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                    Some((Some(qualifier.sym.to_string()), class.sym.to_string()))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn collect_object_instance_classes(
        expression: &Expr,
        prefix: &str,
        classes: &[ClassConstructorRewrite],
        factories: &[FactoryClassRewrite],
        variables: &std::collections::HashMap<String, String>,
        additions: &mut Vec<(String, String)>,
    ) {
        let Expr::Object(object) = expression else {
            return;
        };
        for property in &object.props {
            let PropOrSpread::Prop(property) = property else {
                continue;
            };
            let (name, value): (String, &Expr) = match property.as_ref() {
                Prop::KeyValue(property) => {
                    let Some(name) = (match &property.key {
                        PropName::Ident(identifier) => Some(identifier.sym.to_string()),
                        PropName::Str(value) => Some(value.value.to_string_lossy().into_owned()),
                        PropName::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(value)) => {
                                Some(value.value.to_string_lossy().into_owned())
                            }
                            _ => None,
                        },
                        _ => None,
                    }) else {
                        continue;
                    };
                    (name, property.value.as_ref())
                }
                Prop::Shorthand(identifier) => {
                    let key = format!("{prefix}\u{1f}{}", identifier.sym);
                    if let Some(class) = variables.get(identifier.sym.as_str()) {
                        additions.push((key, class.clone()));
                    }
                    continue;
                }
                _ => continue,
            };
            let key = format!("{prefix}\u{1f}{name}");
            if let Some(class) = source_instance_class(value, classes, factories, variables) {
                additions.push((key.clone(), class));
            }
            collect_object_instance_classes(value, &key, classes, factories, variables, additions);
        }
    }

    #[derive(Clone)]
    enum SourceFunctionResult {
        Fixed(thaw_hir::HirType),
        Argument(usize),
        CommonArguments(Vec<usize>),
    }

    fn source_collection_element_supported(ty: &thaw_hir::HirType) -> bool {
        match ty {
            thaw_hir::HirType::F64
            | thaw_hir::HirType::Str
            | thaw_hir::HirType::Bool
            | thaw_hir::HirType::Json => true,
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload) => {
                source_collection_element_supported(payload)
            }
            thaw_hir::HirType::Array(element) => source_collection_element_supported(element),
            thaw_hir::HirType::Tuple(elements) => {
                elements.iter().all(source_collection_element_supported)
            }
            thaw_hir::HirType::Object(fields) => fields
                .iter()
                .all(|(_, field)| source_collection_element_supported(field)),
            _ => false,
        }
    }

    fn source_expr_type(
        expression: &Expr,
        variables: &std::collections::HashMap<String, thaw_hir::HirType>,
        functions: &std::collections::HashMap<String, SourceFunctionResult>,
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        match expression {
            Expr::Lit(Lit::Num(_)) => Some(thaw_hir::HirType::F64),
            Expr::Lit(Lit::Str(_)) => Some(thaw_hir::HirType::Str),
            Expr::Lit(Lit::Bool(_)) => Some(thaw_hir::HirType::Bool),
            Expr::Lit(Lit::Null(_)) => Some(thaw_hir::HirType::Null),
            Expr::Array(array) => {
                let mut elements = array.elems.iter().map(|element| {
                    source_expr_type(
                        element.as_ref()?.expr.as_ref(),
                        variables,
                        functions,
                        named,
                    )
                });
                let first = elements.next().unwrap_or(Some(thaw_hir::HirType::F64))?;
                (source_collection_element_supported(&first)
                    && elements.all(|candidate| candidate.as_ref() == Some(&first)))
                .then_some(thaw_hir::HirType::Array(Box::new(first)))
            }
            Expr::Object(object) => {
                let mut fields = Vec::with_capacity(object.props.len());
                for property in &object.props {
                    if let PropOrSpread::Spread(spread) = property {
                        let Some(thaw_hir::HirType::Object(spread_fields)) =
                            source_expr_type(&spread.expr, variables, functions, named)
                        else {
                            return Some(thaw_hir::HirType::Object(Vec::new()));
                        };
                        for (name, value_type) in spread_fields {
                            fields.retain(|(existing, _)| existing != &name);
                            fields.push((name, value_type));
                        }
                        continue;
                    }
                    let PropOrSpread::Prop(property) = property else {
                        unreachable!();
                    };
                    let (name, value_type) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let name = match &property.key {
                                PropName::Ident(identifier) => identifier.sym.to_string(),
                                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => match computed.expr.as_ref() {
                                    Expr::Lit(Lit::Str(value)) => {
                                        value.value.to_string_lossy().into_owned()
                                    }
                                    _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                                },
                                _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                            };
                            let Some(value_type) =
                                source_expr_type(&property.value, variables, functions, named)
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (name, value_type)
                        }
                        Prop::Shorthand(identifier) => {
                            let Some(value_type) = variables.get(identifier.sym.as_str()).cloned()
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (identifier.sym.to_string(), value_type)
                        }
                        _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                    };
                    fields.retain(|(existing, _)| existing != &name);
                    fields.push((name, value_type));
                }
                Some(thaw_hir::HirType::Object(fields))
            }
            Expr::Ident(identifier) => variables
                .get(identifier.sym.as_str())
                .cloned()
                .or_else(|| {
                    (identifier.sym == *"undefined").then_some(thaw_hir::HirType::Undefined)
                }),
            Expr::Paren(parenthesized) => {
                source_expr_type(&parenthesized.expr, variables, functions, named)
            }
            Expr::TsAs(assertion) => source_ts_type(&assertion.type_ann, named)
                .or_else(|| source_expr_type(&assertion.expr, variables, functions, named)),
            Expr::TsTypeAssertion(assertion) => source_ts_type(&assertion.type_ann, named)
                .or_else(|| source_expr_type(&assertion.expr, variables, functions, named)),
            Expr::Tpl(_) => Some(thaw_hir::HirType::Str),
            Expr::Unary(unary) => match unary.op {
                UnaryOp::Plus | UnaryOp::Minus
                    if source_expr_type(&unary.arg, variables, functions, named)
                        == Some(thaw_hir::HirType::F64) =>
                {
                    Some(thaw_hir::HirType::F64)
                }
                UnaryOp::Tilde
                    if source_expr_type(&unary.arg, variables, functions, named)
                        == Some(thaw_hir::HirType::F64) =>
                {
                    Some(thaw_hir::HirType::F64)
                }
                UnaryOp::Bang => Some(thaw_hir::HirType::Bool),
                UnaryOp::TypeOf => Some(thaw_hir::HirType::Str),
                _ => None,
            },
            Expr::Bin(binary) => {
                let left = source_expr_type(&binary.left, variables, functions, named);
                let right = source_expr_type(&binary.right, variables, functions, named);
                match binary.op {
                    BinaryOp::Add
                        if matches!(left, Some(thaw_hir::HirType::Str))
                            && matches!(
                                right,
                                Some(
                                    thaw_hir::HirType::F64
                                        | thaw_hir::HirType::Str
                                        | thaw_hir::HirType::Bool
                                )
                            )
                            || matches!(right, Some(thaw_hir::HirType::Str))
                                && matches!(
                                    left,
                                    Some(
                                        thaw_hir::HirType::F64
                                            | thaw_hir::HirType::Str
                                            | thaw_hir::HirType::Bool
                                    )
                                ) =>
                    {
                        Some(thaw_hir::HirType::Str)
                    }
                    BinaryOp::Add
                    | BinaryOp::Sub
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
                        if left == Some(thaw_hir::HirType::F64)
                            && right == Some(thaw_hir::HirType::F64) =>
                    {
                        Some(thaw_hir::HirType::F64)
                    }
                    BinaryOp::EqEq
                    | BinaryOp::NotEq
                    | BinaryOp::EqEqEq
                    | BinaryOp::NotEqEq
                    | BinaryOp::Lt
                    | BinaryOp::LtEq
                    | BinaryOp::Gt
                    | BinaryOp::GtEq => Some(thaw_hir::HirType::Bool),
                    BinaryOp::LogicalAnd
                    | BinaryOp::LogicalOr
                    | BinaryOp::NullishCoalescing
                        if left.is_some() && left == right =>
                    {
                        left
                    }
                    _ => None,
                }
            }
            Expr::Cond(conditional) => {
                let consequent = source_expr_type(&conditional.cons, variables, functions, named);
                let alternate = source_expr_type(&conditional.alt, variables, functions, named);
                (consequent == alternate).then_some(consequent).flatten()
            }
            Expr::Call(call) => match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(identifier) if identifier.sym == *"Number" => {
                        Some(thaw_hir::HirType::F64)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"String" => {
                        Some(thaw_hir::HirType::Str)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"Boolean" => {
                        Some(thaw_hir::HirType::Bool)
                    }
                    Expr::Ident(identifier)
                        if matches!(identifier.sym.as_str(), "parseInt" | "parseFloat") =>
                    {
                        Some(thaw_hir::HirType::F64)
                    }
                    Expr::Ident(identifier) => match functions.get(identifier.sym.as_str())? {
                        SourceFunctionResult::Fixed(ty) => Some(ty.clone()),
                        SourceFunctionResult::Argument(index) => call.args.get(*index).and_then(
                            |argument| {
                                source_expr_type(
                                    argument.expr.as_ref(),
                                    variables,
                                    functions,
                                    named,
                                )
                            },
                        ),
                        SourceFunctionResult::CommonArguments(indices) => {
                            let mut types = indices.iter().map(|index| {
                                source_expr_type(
                                    call.args.get(*index)?.expr.as_ref(),
                                    variables,
                                    functions,
                                    named,
                                )
                            });
                            let first = types.next()??;
                            types
                                .all(|candidate| candidate.as_ref() == Some(&first))
                                .then_some(first)
                        }
                    },
                    Expr::Member(member) => {
                        let method = member_property_name(&member.prop)?;
                        if let Expr::Ident(namespace) = member.obj.as_ref() {
                            match (namespace.sym.as_str(), method.as_str()) {
                                ("JSON", "stringify")
                                    if call.args.first().is_some_and(|argument| {
                                        matches!(
                                            source_expr_type(
                                                argument.expr.as_ref(),
                                                variables,
                                                functions,
                                                named,
                                            ),
                                            Some(
                                                thaw_hir::HirType::F64
                                                    | thaw_hir::HirType::Str
                                                    | thaw_hir::HirType::Bool
                                                    | thaw_hir::HirType::Object(_)
                                                    | thaw_hir::HirType::Array(_)
                                            )
                                        )
                                    }) =>
                                {
                                    return Some(thaw_hir::HirType::Str);
                                }
                                ("Array", "isArray") => return Some(thaw_hir::HirType::Bool),
                                ("Math", _) => return Some(thaw_hir::HirType::F64),
                                _ => {}
                            }
                        }
                        let receiver =
                            source_expr_type(&member.obj, variables, functions, named)?;
                        match (&receiver, method.as_str()) {
                            (_, "toString")
                            | (
                                thaw_hir::HirType::Str,
                                "slice"
                                | "substring"
                                | "substr"
                                | "toUpperCase"
                                | "toLowerCase"
                                | "trim"
                                | "trimStart"
                                | "trimEnd"
                                | "concat"
                                | "replace"
                                | "replaceAll",
                            )
                            | (
                                thaw_hir::HirType::F64,
                                "toFixed" | "toExponential" | "toPrecision",
                            )
                            | (thaw_hir::HirType::Array(_), "join") => {
                                Some(thaw_hir::HirType::Str)
                            }
                            (
                                thaw_hir::HirType::Str,
                                "includes" | "startsWith" | "endsWith",
                            )
                            | (thaw_hir::HirType::Array(_), "includes") => {
                                Some(thaw_hir::HirType::Bool)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                },
                _ => None,
            },
            Expr::Member(member) => {
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                if property.sym == *"length" {
                    return Some(thaw_hir::HirType::F64);
                }
                let thaw_hir::HirType::Object(fields) =
                    source_expr_type(&member.obj, variables, functions, named)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find(|(name, _)| name == property.sym.as_str())
                    .map(|(_, ty)| ty)
            }
            _ => None,
        }
    }

    fn source_ts_type(
        ty: &TsType,
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        match ty {
            TsType::TsKeywordType(keyword) => match keyword.kind {
                TsKeywordTypeKind::TsNumberKeyword => Some(thaw_hir::HirType::F64),
                TsKeywordTypeKind::TsStringKeyword => Some(thaw_hir::HirType::Str),
                TsKeywordTypeKind::TsBooleanKeyword => Some(thaw_hir::HirType::Bool),
                TsKeywordTypeKind::TsVoidKeyword => Some(thaw_hir::HirType::Void),
                _ => None,
            },
            TsType::TsLitType(literal) => match &literal.lit {
                TsLit::Number(_) => Some(thaw_hir::HirType::F64),
                TsLit::Str(_) => Some(thaw_hir::HirType::Str),
                TsLit::Bool(_) => Some(thaw_hir::HirType::Bool),
                _ => None,
            },
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
                let mut payload = None;
                let mut has_null = false;
                let mut has_undefined = false;
                for element in &union.types {
                    match element.as_ref() {
                        TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsNullKeyword =>
                        {
                            has_null = true;
                        }
                        TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsUndefinedKeyword =>
                        {
                            has_undefined = true;
                        }
                        element => {
                            let candidate = source_ts_type(element, named)?;
                            if payload.as_ref().is_some_and(|payload| payload != &candidate) {
                                return None;
                            }
                            payload = Some(candidate);
                        }
                    }
                }
                let payload = payload?;
                Some(match (has_null, has_undefined) {
                    (true, true) => thaw_hir::HirType::Nullish(Box::new(payload)),
                    (true, false) => thaw_hir::HirType::Nullable(Box::new(payload)),
                    (false, true) => thaw_hir::HirType::Optional(Box::new(payload)),
                    (false, false) => payload,
                })
            }
            TsType::TsUnionOrIntersectionType(
                TsUnionOrIntersectionType::TsIntersectionType(intersection),
            ) => {
                let resolved = intersection
                    .types
                    .iter()
                    .map(|element| source_ts_type(element, named))
                    .collect::<Option<Vec<_>>>()?;
                let first = resolved.first()?.clone();
                if resolved.iter().all(|candidate| candidate == &first) {
                    return Some(first);
                }
                let mut fields = Vec::new();
                for ty in resolved {
                    let thaw_hir::HirType::Object(object_fields) = ty else {
                        return None;
                    };
                    for (name, field_type) in object_fields {
                        if let Some((_, existing)) =
                            fields.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &field_type {
                                return None;
                            }
                        } else {
                            fields.push((name, field_type));
                        }
                    }
                }
                Some(thaw_hir::HirType::Object(fields))
            }
            TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
                source_ts_type(&operator.type_ann, named)
            }
            TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
                matches!(
                    source_ts_type(&operator.type_ann, named),
                    Some(thaw_hir::HirType::Object(_))
                )
                .then_some(thaw_hir::HirType::Str)
            }
            TsType::TsIndexedAccessType(indexed) => {
                let thaw_hir::HirType::Object(fields) =
                    source_ts_type(&indexed.obj_type, named)?
                else {
                    return None;
                };
                let name = match indexed.index_type.as_ref() {
                    TsType::TsLitType(literal) => match &literal.lit {
                        TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
                        TsLit::Number(value) => value.value.to_string(),
                        _ => return None,
                    },
                    _ => return None,
                };
                fields
                    .into_iter()
                    .find(|(field, _)| field == &name)
                    .map(|(_, ty)| ty)
            }
            TsType::TsArrayType(array) => {
                let element = source_ts_type(&array.elem_type, named)?;
                source_collection_element_supported(&element)
                    .then_some(thaw_hir::HirType::Array(Box::new(element)))
            }
            TsType::TsTupleType(tuple) => tuple
                .elem_types
                .iter()
                .map(|element| source_ts_type(&element.ty, named))
                .collect::<Option<Vec<_>>>()
                .map(thaw_hir::HirType::Tuple),
            TsType::TsTypeRef(reference) => {
                let TsEntityName::Ident(name) = &reference.type_name else {
                    return None;
                };
                let parameters = reference
                    .type_params
                    .as_ref()
                    .map(|parameters| parameters.params.as_slice())
                    .unwrap_or_default();
                match (name.sym.as_str(), parameters) {
                    ("Array" | "ReadonlyArray", [element]) => {
                        let element = source_ts_type(element, named)?;
                        source_collection_element_supported(&element)
                            .then_some(thaw_hir::HirType::Array(Box::new(element)))
                    }
                    ("Readonly", [inner]) => source_ts_type(inner, named),
                    ("Partial", [inner]) => {
                        let thaw_hir::HirType::Object(fields) = source_ts_type(inner, named)? else {
                            return None;
                        };
                        Some(thaw_hir::HirType::Object(
                            fields
                                .into_iter()
                                .map(|(name, ty)| {
                                    let ty = match ty {
                                        thaw_hir::HirType::Optional(_)
                                        | thaw_hir::HirType::Nullish(_) => ty,
                                        thaw_hir::HirType::Nullable(inner) => {
                                            thaw_hir::HirType::Nullish(inner)
                                        }
                                        other => thaw_hir::HirType::Optional(Box::new(other)),
                                    };
                                    (name, ty)
                                })
                                .collect(),
                        ))
                    }
                    ("Required", [inner]) => {
                        let thaw_hir::HirType::Object(fields) = source_ts_type(inner, named)? else {
                            return None;
                        };
                        Some(thaw_hir::HirType::Object(
                            fields
                                .into_iter()
                                .map(|(name, ty)| {
                                    let ty = match ty {
                                        thaw_hir::HirType::Optional(inner) => *inner,
                                        thaw_hir::HirType::Nullish(inner) => {
                                            thaw_hir::HirType::Nullable(inner)
                                        }
                                        other => other,
                                    };
                                    (name, ty)
                                })
                                .collect(),
                        ))
                    }
                    ("Pick" | "Omit", [object, keys]) => {
                        let thaw_hir::HirType::Object(fields) = source_ts_type(object, named)? else {
                            return None;
                        };
                        let keys = source_utility_keys(keys, named)?;
                        let omit = name.sym == *"Omit";
                        Some(thaw_hir::HirType::Object(
                            fields
                                .into_iter()
                                .filter(|(field, _)| keys.contains(field) != omit)
                                .collect(),
                        ))
                    }
                    ("Record", [keys, value]) => {
                        let keys = source_utility_keys(keys, named)?;
                        let value = source_ts_type(value, named)?;
                        Some(thaw_hir::HirType::Object(
                            keys.into_iter().map(|key| (key, value.clone())).collect(),
                        ))
                    }
                    ("NonNullable", [inner]) => match source_ts_type(inner, named)? {
                        thaw_hir::HirType::Optional(inner)
                        | thaw_hir::HirType::Nullable(inner)
                        | thaw_hir::HirType::Nullish(inner) => Some(*inner),
                        other => Some(other),
                    },
                    // `JsValue` is thaw's own built-in dynamic-value type,
                    // not something a user's source ever declares itself
                    // -- without this, `x as JsValue` (a real, useful
                    // pattern for handing an already-dynamically-typed
                    // value to a call that needs one, real example:
                    // passing a registry function's own JsValue-typed
                    // result into another function's JsValue-typed
                    // parameter when its declared shape can't be locally
                    // inferred any other way) silently inferred as
                    // "unknown type" here, same as any other unrecognized
                    // reference.
                    ("JsValue", []) => Some(thaw_hir::HirType::JsValue),
                    (_, []) => named.get(name.sym.as_str()).cloned(),
                    _ => None,
                }
            }
            TsType::TsTypeLit(literal) => {
                source_type_members(&literal.members, named).map(thaw_hir::HirType::Object)
            }
            TsType::TsParenthesizedType(parenthesized) => {
                source_ts_type(&parenthesized.type_ann, named)
            }
            _ => None,
        }
    }

    fn source_utility_keys(
        ty: &TsType,
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<Vec<String>> {
        match ty {
            TsType::TsLitType(literal) => match &literal.lit {
                TsLit::Str(value) => Some(vec![value.value.to_string_lossy().into_owned()]),
                TsLit::Number(value) => Some(vec![value.value.to_string()]),
                _ => None,
            },
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
                let mut keys = Vec::new();
                for element in &union.types {
                    keys.extend(source_utility_keys(element, named)?);
                }
                Some(keys)
            }
            TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
                let thaw_hir::HirType::Object(fields) =
                    source_ts_type(&operator.type_ann, named)?
                else {
                    return None;
                };
                Some(fields.into_iter().map(|(name, _)| name).collect())
            }
            TsType::TsParenthesizedType(parenthesized) => {
                source_utility_keys(&parenthesized.type_ann, named)
            }
            _ => None,
        }
    }

    fn source_type_property_name(key: &Expr) -> Option<String> {
        match key {
            Expr::Ident(identifier) => Some(identifier.sym.to_string()),
            Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
            Expr::Lit(Lit::Num(value)) => Some(value.value.to_string()),
            _ => None,
        }
    }

    fn source_type_members(
        members: &[TsTypeElement],
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<Vec<(String, thaw_hir::HirType)>> {
        let mut fields = Vec::with_capacity(members.len());
        for member in members {
            let TsTypeElement::TsPropertySignature(property) = member else {
                return None;
            };
            let name = source_type_property_name(&property.key)?;
            if fields.iter().any(|(existing, _)| existing == &name) {
                return None;
            }
            let mut ty = source_ts_type(&property.type_ann.as_ref()?.type_ann, named)?;
            if property.optional {
                ty = thaw_hir::HirType::Optional(Box::new(ty));
            }
            fields.push((name, ty));
        }
        Some(fields)
    }

    fn source_interfaces_type(
        name: &str,
        declarations: &[TsInterfaceDecl],
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        let mut fields = Vec::new();
        for declaration in declarations
            .iter()
            .filter(|declaration| declaration.id.sym == *name)
        {
            if declaration.type_params.is_some() {
                return None;
            }
            for base in &declaration.extends {
                if base.type_args.is_some() {
                    return None;
                }
                let Expr::Ident(base_name) = base.expr.as_ref() else {
                    return None;
                };
                let thaw_hir::HirType::Object(base_fields) =
                    named.get(base_name.sym.as_str())?
                else {
                    return None;
                };
                for (field_name, ty) in base_fields {
                    if let Some((_, existing)) =
                        fields.iter().find(|(existing, _)| existing == field_name)
                    {
                        if existing != ty {
                            return None;
                        }
                    } else {
                        fields.push((field_name.clone(), ty.clone()));
                    }
                }
            }
            for (field_name, ty) in source_type_members(&declaration.body.body, named)? {
                if let Some((_, existing)) = fields
                    .iter()
                    .find(|(existing, _)| existing == &field_name)
                {
                    if existing != &ty {
                        return None;
                    }
                } else {
                    fields.push((field_name, ty));
                }
            }
        }
        Some(thaw_hir::HirType::Object(fields))
    }

    #[derive(Default)]
    struct NamedTypeDeclarationFinder {
        aliases: Vec<TsTypeAliasDecl>,
        interfaces: Vec<TsInterfaceDecl>,
    }

    impl Visit for NamedTypeDeclarationFinder {
        fn visit_ts_type_alias_decl(&mut self, declaration: &TsTypeAliasDecl) {
            self.aliases.push(declaration.clone());
        }

        fn visit_ts_interface_decl(&mut self, declaration: &TsInterfaceDecl) {
            self.interfaces.push(declaration.clone());
        }
    }

    fn returned_expression(body: &FunctionBody) -> Option<&Expr> {
        let [Stmt::Return(statement)] = body.stmts.as_slice() else {
            return None;
        };
        statement.arg.as_deref()
    }

    fn returned_identifier(body: &FunctionBody) -> Option<&str> {
        let Expr::Ident(identifier) = returned_expression(body)? else {
            return None;
        };
        Some(identifier.sym.as_str())
    }

    fn function_argument_result(function: &Function) -> Option<usize> {
        let returned = returned_identifier(function.body.as_ref()?)?;
        function.params.iter().position(
            |parameter| matches!(&parameter.pat, Pat::Ident(binding) if binding.id.sym == *returned),
        )
    }

    fn arrow_argument_result(arrow: &thaw_parser::ast::ArrowExpr) -> Option<usize> {
        let returned = match arrow.body.as_ref() {
            ArrowFunctionBody::Expr(expression) => {
                let Expr::Ident(identifier) = expression.as_ref() else {
                    return None;
                };
                identifier.sym.as_str()
            }
            ArrowFunctionBody::FunctionBody(body) => returned_identifier(body)?,
        };
        arrow
            .params
            .iter()
            .position(|parameter| matches!(parameter, Pat::Ident(binding) if binding.id.sym == *returned))
    }

    fn common_argument_result(expression: &Expr, parameters: &[Pat]) -> Option<Vec<usize>> {
        let expressions = match expression {
            Expr::Cond(conditional) => [&*conditional.cons, &*conditional.alt],
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::LogicalAnd
                        | BinaryOp::LogicalOr
                        | BinaryOp::NullishCoalescing
                ) =>
            {
                [&*binary.left, &*binary.right]
            }
            _ => return None,
        };
        let mut indices = Vec::new();
        for expression in expressions {
            let Expr::Ident(identifier) = expression else {
                return None;
            };
            let index = parameters.iter().position(
                |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.sym == identifier.sym),
            )?;
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
        (indices.len() > 1).then_some(indices)
    }

    fn function_common_argument_result(function: &Function) -> Option<Vec<usize>> {
        let parameters = function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        common_argument_result(returned_expression(function.body.as_ref()?)?, &parameters)
    }

    fn arrow_common_argument_result(
        arrow: &thaw_parser::ast::ArrowExpr,
    ) -> Option<Vec<usize>> {
        let expression = match arrow.body.as_ref() {
            ArrowFunctionBody::Expr(expression) => expression.as_ref(),
            ArrowFunctionBody::FunctionBody(body) => returned_expression(body)?,
        };
        common_argument_result(expression, &arrow.params)
    }

    fn block_argument_result(
        body: &FunctionBody,
        parameters: &[Pat],
    ) -> Option<SourceFunctionResult> {
        struct Returns<'a> {
            parameters: &'a [Pat],
            indices: Vec<usize>,
            valid: bool,
        }
        impl Visit for Returns<'_> {
            fn visit_return_stmt(&mut self, statement: &ReturnStmt) {
                let Some(expression) = statement.arg.as_deref() else {
                    self.valid = false;
                    return;
                };
                let indices = if let Expr::Ident(identifier) = expression {
                    self.parameters
                        .iter()
                        .position(
                            |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.sym == identifier.sym),
                        )
                        .map(|index| vec![index])
                } else {
                    common_argument_result(expression, self.parameters)
                };
                let Some(indices) = indices else {
                    self.valid = false;
                    return;
                };
                for index in indices {
                    if !self.indices.contains(&index) {
                        self.indices.push(index);
                    }
                }
            }

            fn visit_fn_decl(&mut self, _declaration: &FnDecl) {}

            fn visit_arrow_expr(&mut self, _expression: &thaw_parser::ast::ArrowExpr) {}

            fn visit_fn_expr(&mut self, _expression: &thaw_parser::ast::FnExpr) {}
        }
        let mut returns = Returns {
            parameters,
            indices: Vec::new(),
            valid: true,
        };
        body.visit_with(&mut returns);
        if !returns.valid {
            return None;
        }
        match returns.indices.as_slice() {
            [index] => Some(SourceFunctionResult::Argument(*index)),
            [] => None,
            _ => Some(SourceFunctionResult::CommonArguments(returns.indices)),
        }
    }

    fn function_block_argument_result(function: &Function) -> Option<SourceFunctionResult> {
        let parameters = function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        block_argument_result(function.body.as_ref()?, &parameters)
    }

    fn arrow_block_argument_result(
        arrow: &thaw_parser::ast::ArrowExpr,
    ) -> Option<SourceFunctionResult> {
        let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() else {
            return None;
        };
        block_argument_result(body, &arrow.params)
    }

    fn forwarded_argument_result(
        expression: &Expr,
        parameters: &[Pat],
        known: &std::collections::HashMap<String, SourceFunctionResult>,
    ) -> Option<SourceFunctionResult> {
        let Expr::Call(call) = expression else {
            return None;
        };
        let thaw_parser::ast::Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Ident(function) = callee.as_ref() else {
            return None;
        };
        let arguments = match known.get(function.sym.as_str())? {
            SourceFunctionResult::Argument(argument) => vec![*argument],
            SourceFunctionResult::CommonArguments(arguments) => arguments.clone(),
            SourceFunctionResult::Fixed(_) => return None,
        };
        let indices = arguments
            .into_iter()
            .map(|argument| {
                let Expr::Ident(returned) = call.args.get(argument)?.expr.as_ref() else {
                    return None;
                };
                parameters.iter().position(
                    |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.sym == returned.sym),
                )
            })
            .collect::<Option<Vec<_>>>()?;
        match indices.as_slice() {
            [index] => Some(SourceFunctionResult::Argument(*index)),
            _ => Some(SourceFunctionResult::CommonArguments(indices)),
        }
    }

    fn function_forwarded_argument(
        function: &Function,
        known: &std::collections::HashMap<String, SourceFunctionResult>,
    ) -> Option<SourceFunctionResult> {
        let parameters = function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        forwarded_argument_result(returned_expression(function.body.as_ref()?)?, &parameters, known)
    }

    fn arrow_forwarded_argument(
        arrow: &thaw_parser::ast::ArrowExpr,
        known: &std::collections::HashMap<String, SourceFunctionResult>,
    ) -> Option<SourceFunctionResult> {
        let expression = match arrow.body.as_ref() {
            ArrowFunctionBody::Expr(expression) => expression.as_ref(),
            ArrowFunctionBody::FunctionBody(body) => returned_expression(body)?,
        };
        forwarded_argument_result(expression, &arrow.params, known)
    }

    struct FunctionTypeFinder<'a> {
        named: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        types: std::collections::HashMap<String, SourceFunctionResult>,
    }

    impl Visit for FunctionTypeFinder<'_> {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if let Some(return_type) = declaration
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| source_ts_type(&annotation.type_ann, self.named))
            {
                self.types.insert(
                    declaration.ident.sym.to_string(),
                    SourceFunctionResult::Fixed(return_type),
                );
            } else if let Some(index) = function_argument_result(&declaration.function) {
                self.types.insert(
                    declaration.ident.sym.to_string(),
                    SourceFunctionResult::Argument(index),
                );
            } else if let Some(indices) =
                function_common_argument_result(&declaration.function)
            {
                self.types.insert(
                    declaration.ident.sym.to_string(),
                    SourceFunctionResult::CommonArguments(indices),
                );
            } else if let Some(result) =
                function_block_argument_result(&declaration.function)
            {
                self.types
                    .insert(declaration.ident.sym.to_string(), result);
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                let annotation = match initializer.as_ref() {
                    Expr::Arrow(arrow) => arrow.return_type.as_ref(),
                    Expr::Fn(function) => function.function.return_type.as_ref(),
                    _ => None,
                };
                if let Some(return_type) =
                    annotation
                        .and_then(|annotation| source_ts_type(&annotation.type_ann, self.named))
                {
                    self.types.insert(
                        binding.id.sym.to_string(),
                        SourceFunctionResult::Fixed(return_type),
                    );
                } else {
                    let argument = match initializer.as_ref() {
                        Expr::Arrow(arrow) => arrow_argument_result(arrow),
                        Expr::Fn(function) => function_argument_result(&function.function),
                        _ => None,
                    };
                    if let Some(index) = argument {
                        self.types.insert(
                            binding.id.sym.to_string(),
                            SourceFunctionResult::Argument(index),
                        );
                    } else {
                        let indices = match initializer.as_ref() {
                            Expr::Arrow(arrow) => arrow_common_argument_result(arrow),
                            Expr::Fn(function) => {
                                function_common_argument_result(&function.function)
                            }
                            _ => None,
                        };
                        if let Some(indices) = indices {
                            self.types.insert(
                                binding.id.sym.to_string(),
                                SourceFunctionResult::CommonArguments(indices),
                            );
                        } else {
                            let result = match initializer.as_ref() {
                                Expr::Arrow(arrow) => arrow_block_argument_result(arrow),
                                Expr::Fn(function) => {
                                    function_block_argument_result(&function.function)
                                }
                                _ => None,
                            };
                            if let Some(result) = result {
                                self.types.insert(binding.id.sym.to_string(), result);
                            }
                        }
                    }
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn inferred_block_return_type(
        block: &FunctionBody,
        functions: &std::collections::HashMap<String, SourceFunctionResult>,
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        struct Returns<'a> {
            functions: &'a std::collections::HashMap<String, SourceFunctionResult>,
            named: &'a std::collections::HashMap<String, thaw_hir::HirType>,
            types: Vec<Option<thaw_hir::HirType>>,
        }
        impl Visit for Returns<'_> {
            fn visit_return_stmt(&mut self, statement: &ReturnStmt) {
                self.types
                    .push(statement.arg.as_ref().and_then(|expression| {
                        source_expr_type(
                            expression,
                            &std::collections::HashMap::new(),
                            self.functions,
                            self.named,
                        )
                    }));
            }

            // Returns inside nested functions belong to those functions, not
            // to the block currently being inferred.
            fn visit_fn_decl(&mut self, _declaration: &FnDecl) {}

            fn visit_arrow_expr(&mut self, _expression: &thaw_parser::ast::ArrowExpr) {}

            fn visit_fn_expr(&mut self, _expression: &thaw_parser::ast::FnExpr) {}
        }

        let mut returns = Returns {
            functions,
            named,
            types: Vec::new(),
        };
        block.visit_with(&mut returns);
        let first = returns.types.first()?.clone()?;
        returns
            .types
            .iter()
            .all(|candidate| candidate.as_ref() == Some(&first))
            .then_some(first)
    }

    struct InferredFunctionTypeFinder<'a> {
        known: &'a std::collections::HashMap<String, SourceFunctionResult>,
        named: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        additions: std::collections::HashMap<String, SourceFunctionResult>,
    }

    impl Visit for InferredFunctionTypeFinder<'_> {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if !self.known.contains_key(declaration.ident.sym.as_str()) {
                if let Some(result) =
                    function_forwarded_argument(&declaration.function, self.known)
                {
                    self.additions
                        .insert(declaration.ident.sym.to_string(), result);
                } else if let Some(return_type) = declaration.function.body.as_ref().and_then(
                    |body| inferred_block_return_type(body, self.known, self.named),
                ) {
                    self.additions.insert(
                        declaration.ident.sym.to_string(),
                        SourceFunctionResult::Fixed(return_type),
                    );
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                if !self.known.contains_key(binding.id.sym.as_str()) {
                    let forwarded = match initializer.as_ref() {
                        Expr::Arrow(arrow) => arrow_forwarded_argument(arrow, self.known),
                        Expr::Fn(function) => {
                            function_forwarded_argument(&function.function, self.known)
                        }
                        _ => None,
                    };
                    if let Some(result) = forwarded {
                        self.additions.insert(binding.id.sym.to_string(), result);
                        declaration.visit_children_with(self);
                        return;
                    }
                    let return_type = match initializer.as_ref() {
                        Expr::Arrow(arrow) => match arrow.body.as_ref() {
                            ArrowFunctionBody::FunctionBody(block) => {
                                inferred_block_return_type(block, self.known, self.named)
                            }
                            ArrowFunctionBody::Expr(expression) => source_expr_type(
                                expression,
                                &std::collections::HashMap::new(),
                                self.known,
                                self.named,
                            ),
                        },
                        Expr::Fn(function) => function
                            .function
                            .body
                            .as_ref()
                            .and_then(|body| {
                                inferred_block_return_type(body, self.known, self.named)
                            }),
                        _ => None,
                    };
                    if let Some(return_type) = return_type {
                        self.additions.insert(
                            binding.id.sym.to_string(),
                            SourceFunctionResult::Fixed(return_type),
                        );
                    }
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn overload_type_score(declared: &thaw_hir::HirType, actual: &thaw_hir::HirType) -> Option<u8> {
        match (declared, actual) {
            (thaw_hir::HirType::Json, _) => Some(0),
            (thaw_hir::HirType::Union(elements), actual) => elements
                .iter()
                .filter_map(|element| overload_type_score(element, actual))
                .max(),
            (thaw_hir::HirType::Nullable(payload), thaw_hir::HirType::Null) => {
                source_collection_element_supported(payload).then_some(2)
            }
            (thaw_hir::HirType::Nullable(payload), actual) if payload.as_ref() == actual => Some(2),
            (thaw_hir::HirType::Optional(payload), thaw_hir::HirType::Undefined) => {
                source_collection_element_supported(payload).then_some(2)
            }
            (thaw_hir::HirType::Optional(payload), actual) if payload.as_ref() == actual => Some(2),
            (thaw_hir::HirType::Nullish(payload), thaw_hir::HirType::Null | thaw_hir::HirType::Undefined) => {
                source_collection_element_supported(payload).then_some(2)
            }
            (thaw_hir::HirType::Nullish(payload), actual) if payload.as_ref() == actual => Some(2),
            (thaw_hir::HirType::Object(declared), thaw_hir::HirType::Object(actual)) => {
                if declared.is_empty() || actual.is_empty() {
                    return Some(1);
                }
                let mut score = 2u8;
                for (name, declared_type) in declared {
                    let (_, actual_type) =
                        actual.iter().find(|(candidate, _)| candidate == name)?;
                    score = score.saturating_add(overload_type_score(declared_type, actual_type)?);
                }
                Some(score)
            }
            (left, right) if left == right => Some(2),
            _ => None,
        }
    }

    fn member_property_name(property: &MemberProp) -> Option<String> {
        match property {
            MemberProp::Ident(identifier) => Some(identifier.sym.to_string()),
            MemberProp::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                _ => None,
            },
            MemberProp::PrivateName(_) => None,
        }
    }

    fn member_assignment_path(
        member: &thaw_parser::ast::MemberExpr,
    ) -> Option<(String, Vec<String>)> {
        let mut path = vec![member_property_name(&member.prop)?];
        let mut object = member.obj.as_ref();
        loop {
            match object {
                Expr::Ident(identifier) => {
                    path.reverse();
                    return Some((identifier.sym.to_string(), path));
                }
                Expr::Member(parent) => {
                    path.push(member_property_name(&parent.prop)?);
                    object = parent.obj.as_ref();
                }
                _ => return None,
            }
        }
    }

    fn update_object_property_type(
        object: &mut thaw_hir::HirType,
        path: &[String],
        value: thaw_hir::HirType,
    ) -> bool {
        let thaw_hir::HirType::Object(fields) = object else {
            return false;
        };
        let Some((property, remaining)) = path.split_first() else {
            return false;
        };
        if remaining.is_empty() {
            if let Some((_, ty)) = fields.iter_mut().find(|(name, _)| name == property) {
                *ty = value;
            } else {
                fields.push((property.clone(), value));
            }
            return true;
        }
        fields
            .iter_mut()
            .find(|(name, _)| name == property)
            .is_some_and(|(_, nested)| update_object_property_type(nested, remaining, value))
    }

    fn annotate_generic_callback_arguments(
        call: &CallExpr,
        generic: &thaw_bridge::DtsGenericFunction,
        variables: &std::collections::HashMap<String, thaw_hir::HirType>,
        functions: &std::collections::HashMap<String, SourceFunctionResult>,
        named: &std::collections::HashMap<String, thaw_hir::HirType>,
        edits: &mut Vec<(u32, u32, String)>,
    ) -> bool {
        let edit_count = edits.len();
        let mut complete = true;
        fn type_text(ty: &thaw_hir::HirType) -> Option<String> {
            match ty {
                thaw_hir::HirType::F64 => Some("number".into()),
                thaw_hir::HirType::Str => Some("string".into()),
                thaw_hir::HirType::Bool => Some("boolean".into()),
                thaw_hir::HirType::Json => Some("Json".into()),
                thaw_hir::HirType::Array(inner) => Some(format!("{}[]", type_text(inner)?)),
                _ => None,
            }
        }
        let mut substitutions = std::collections::HashMap::new();
        for (argument, declared) in call.args.iter().zip(&generic.contextual_param_types) {
            let Some(actual) = source_expr_type(argument.expr.as_ref(), variables, functions, named)
            else {
                continue;
            };
            for (parameter, _) in &generic.type_params {
                if declared == parameter {
                    if let Some(actual) = type_text(&actual) {
                        substitutions.insert(parameter.clone(), actual);
                    }
                } else if declared == &format!("{parameter}[]") {
                    if let thaw_hir::HirType::Array(element) = &actual {
                        if let Some(element) = type_text(element) {
                            substitutions.insert(parameter.clone(), element);
                        }
                    }
                } else if declared.contains(&format!("<{parameter}>")) {
                    // ponytail: generic wrappers are treated as collections when the
                    // actual value is an array; expand alias bodies if this becomes ambiguous.
                    if let thaw_hir::HirType::Array(element) = &actual {
                        if let Some(element) = type_text(element) {
                            substitutions.insert(parameter.clone(), element);
                        }
                    }
                }
            }
        }
        for (argument, contextual) in call.args.iter().zip(&generic.contextual_param_types) {
            let Expr::Arrow(arrow) = argument.expr.as_ref() else {
                continue;
            };
            let Some(end) = contextual.find(") =>") else {
                continue;
            };
            let Some(parameters) = contextual.strip_prefix('(').map(|text| &text[..end - 1]) else {
                continue;
            };
            for (parameter, declared) in arrow.params.iter().zip(parameters.split(',')) {
                let Pat::Ident(binding) = parameter else {
                    continue;
                };
                if binding.type_ann.is_some() {
                    continue;
                }
                let Some((_, declared)) = declared.split_once(':') else {
                    continue;
                };
                let mut rendered = declared.trim().to_string();
                for (name, replacement) in &substitutions {
                    if rendered == *name {
                        rendered = replacement.clone();
                    } else if rendered == format!("{name}[]") {
                        rendered = format!("{replacement}[]");
                    } else if rendered == format!("{name}[number]") {
                        if let Some(element) = replacement.strip_suffix("[]") {
                            rendered = element.into();
                        }
                    }
                }
                if matches!(rendered.as_str(), "number" | "string" | "boolean" | "Json")
                    || rendered.ends_with("[]")
                {
                    let span = binding.id.span;
                    if arrow.params.len() == 1 && arrow.span.lo == span.lo {
                        edits.push((span.lo.0, span.lo.0, "(".into()));
                        edits.push((span.hi.0, span.hi.0, format!(": {rendered})")));
                    } else {
                        edits.push((span.hi.0, span.hi.0, format!(": {rendered}")));
                    }
                } else {
                    complete = false;
                }
            }
        }
        if !complete {
            edits.truncate(edit_count);
        }
        complete && edits.len() != edit_count
    }

    struct Finder<'a> {
        classes: &'a [ClassConstructorRewrite],
        factories: &'a [FactoryClassRewrite],
        functions: &'a [FallbackFunctionOverloadRewrite],
        methods: &'a [ClassMethodRewrite],
        static_methods: &'a [StaticClassMethodRewrite],
        getters: &'a [ClassGetterRewrite],
        setters: &'a [ClassSetterRewrite],
        static_getters: &'a [StaticClassGetterRewrite],
        static_setters: &'a [StaticClassSetterRewrite],
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        function_types: &'a std::collections::HashMap<String, SourceFunctionResult>,
        named_types: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
        edits: Vec<(u32, u32, String)>,
        switch_break_depth: usize,
        switch_break_exits: Vec<FlowState>,
    }

    #[derive(Clone)]
    struct FlowState {
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
    }

    impl Finder<'_> {
        fn flow_state(&self) -> FlowState {
            FlowState {
                variables: self.variables.clone(),
                value_types: self.value_types.clone(),
                callbacks: self.callbacks.clone(),
            }
        }

        fn restore_flow_state(&mut self, state: FlowState) {
            self.variables = state.variables;
            self.value_types = state.value_types;
            self.callbacks = state.callbacks;
        }

        fn join_current_flow_with(&mut self, other: &FlowState) {
            self.variables
                .retain(|name, class| other.variables.get(name) == Some(class));
            self.value_types
                .retain(|name, ty| other.value_types.get(name) == Some(ty));
            self.callbacks.retain(|name| other.callbacks.contains(name));
        }
    }

    impl Visit for Finder<'_> {
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let class = match expression.callee.as_ref() {
                Expr::Ident(class) => self
                    .classes
                    .iter()
                    .find(|(_, name, _)| name == class.sym.as_str()),
                Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                    (Expr::Ident(package), MemberProp::Ident(class)) => {
                        self.classes.iter().find(|(qualifier, name, _)| {
                            qualifier == package.sym.as_str() && name == class.sym.as_str()
                        })
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((_, _, helpers)) = class {
                let arguments = expression.args.as_deref().unwrap_or_default();
                let selected = helpers
                    .iter()
                    .filter(|(arity, _, _)| *arity == arguments.len())
                    .filter_map(|candidate| {
                        let mut score = 0u16;
                        for (argument, declared) in arguments.iter().zip(&candidate.2) {
                            if let Some(actual) = source_expr_type(
                                argument.expr.as_ref(),
                                &self.value_types,
                                self.function_types,
                                self.named_types,
                            ) {
                                score += u16::from(overload_type_score(declared, &actual)?);
                            }
                        }
                        Some((score, candidate))
                    })
                    .reduce(|best, candidate| {
                        if candidate.0 > best.0 {
                            candidate
                        } else {
                            best
                        }
                    })
                    .map(|(_, candidate)| candidate);
                if let Some((_, helper, _)) = selected {
                    self.edits.push((
                        expression.span().lo.0,
                        expression.callee.span().hi.0,
                        helper.clone(),
                    ));
                }
            }
            expression.visit_children_with(self);
        }

        fn visit_member_expr(&mut self, member: &thaw_parser::ast::MemberExpr) {
            if let (Some((qualifier, class)), MemberProp::Ident(property)) =
                (static_class_receiver(&member.obj), &member.prop)
            {
                if let Some((_, _, _, helper)) = self.static_getters.iter().find(
                    |(candidate_qualifier, candidate_class, candidate_property, _)| {
                        candidate_class == &class
                            && candidate_property == property.sym.as_str()
                            && qualifier
                                .as_ref()
                                .is_none_or(|qualifier| candidate_qualifier == qualifier)
                    },
                ) {
                    let span = member.span();
                    self.edits
                        .push((span.lo.0, span.hi.0, format!("{helper}()")));
                    return;
                }
            }
            if let (Some((receiver_key, receiver_source)), MemberProp::Ident(property)) =
                (instance_receiver(&member.obj), &member.prop)
            {
                if let Some(class) = self.variables.get(&receiver_key) {
                    if let Some((_, _, helper)) =
                        self.getters
                            .iter()
                            .find(|(candidate_class, candidate_property, _)| {
                                candidate_class == class
                                    && candidate_property == property.sym.as_str()
                            })
                    {
                        let span = member.span();
                        self.edits.push((
                            span.lo.0,
                            span.hi.0,
                            format!("{helper}({receiver_source})"),
                        ));
                        member.obj.visit_with(self);
                        return;
                    }
                }
            }
            member.visit_children_with(self);
        }

        fn visit_if_stmt(&mut self, statement: &IfStmt) {
            statement.test.visit_with(self);
            let base = self.flow_state();

            statement.cons.visit_with(self);
            let consequent = self.flow_state();

            self.restore_flow_state(base);
            if let Some(alternate) = &statement.alt {
                alternate.visit_with(self);
            }

            let alternate = self.flow_state();
            self.restore_flow_state(consequent);
            self.join_current_flow_with(&alternate);
        }

        fn visit_while_stmt(&mut self, statement: &WhileStmt) {
            statement.test.visit_with(self);
            let zero_iterations = self.flow_state();
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_for_stmt(&mut self, statement: &ForStmt) {
            if let Some(initializer) = &statement.init {
                initializer.visit_with(self);
            }
            if let Some(test) = &statement.test {
                test.visit_with(self);
            }
            let zero_iterations = self.flow_state();
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            if let Some(update) = &statement.update {
                update.visit_with(self);
            }
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_do_while_stmt(&mut self, statement: &DoWhileStmt) {
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            statement.test.visit_with(self);
        }

        fn visit_for_in_stmt(&mut self, statement: &ForInStmt) {
            statement.right.visit_with(self);
            statement.left.visit_with(self);
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
        }

        fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
            statement.right.visit_with(self);
            statement.left.visit_with(self);
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
        }

        fn visit_break_stmt(&mut self, statement: &BreakStmt) {
            if statement.label.is_none() && self.switch_break_depth == 1 {
                self.switch_break_exits.push(self.flow_state());
            }
        }

        fn visit_switch_stmt(&mut self, statement: &SwitchStmt) {
            let outer_break_depth = std::mem::replace(&mut self.switch_break_depth, 1);
            let outer_break_exits = std::mem::take(&mut self.switch_break_exits);
            statement.discriminant.visit_with(self);
            let base = self.flow_state();
            let mut direct_entries = vec![None; statement.cases.len()];

            // Case tests execute in source order until one matches. Visiting
            // each test once gives the exact incoming state for that direct
            // entry without duplicating source rewrites across possible paths.
            for (index, case) in statement.cases.iter().enumerate() {
                if let Some(test) = &case.test {
                    test.visit_with(self);
                    direct_entries[index] = Some(self.flow_state());
                }
            }
            let after_tests = self.flow_state();
            if let Some(default) = statement.cases.iter().position(|case| case.test.is_none()) {
                direct_entries[default] = Some(after_tests.clone());
            }

            let mut exits = Vec::new();
            if statement.cases.iter().all(|case| case.test.is_some()) {
                exits.push(after_tests);
            }
            let mut fallthrough: Option<FlowState> = None;

            for (case, direct) in statement.cases.iter().zip(direct_entries) {
                let mut incoming = direct.expect("every switch case has a direct entry");
                if let Some(previous) = fallthrough.take() {
                    self.restore_flow_state(incoming);
                    self.join_current_flow_with(&previous);
                    incoming = self.flow_state();
                }
                self.restore_flow_state(incoming);

                let mut terminated = false;
                for consequent in &case.cons {
                    consequent.visit_with(self);
                    if matches!(consequent, Stmt::Break(statement) if statement.label.is_none()) {
                        terminated = true;
                        break;
                    }
                }
                if !terminated {
                    fallthrough = Some(self.flow_state());
                }
            }
            if let Some(fallthrough) = fallthrough {
                exits.push(fallthrough);
            }
            exits.append(&mut self.switch_break_exits);
            self.switch_break_exits = outer_break_exits;
            self.switch_break_depth = outer_break_depth;

            let mut exits = exits.into_iter();
            let Some(first) = exits.next() else {
                self.restore_flow_state(base);
                return;
            };
            self.restore_flow_state(first);
            for exit in exits {
                self.join_current_flow_with(&exit);
            }
        }

        fn visit_try_stmt(&mut self, statement: &TryStmt) {
            let incoming = self.flow_state();
            statement.block.visit_with(self);
            let try_exit = self.flow_state();

            if let Some(handler) = &statement.handler {
                // A throw may occur after any prefix of the try block. Facts
                // that differ between entry and normal exit are therefore not
                // safe assumptions at catch entry.
                let mut catch_entry = try_exit.clone();
                catch_entry
                    .variables
                    .retain(|name, class| incoming.variables.get(name) == Some(class));
                catch_entry
                    .value_types
                    .retain(|name, ty| incoming.value_types.get(name) == Some(ty));
                catch_entry
                    .callbacks
                    .retain(|name| incoming.callbacks.contains(name));
                self.restore_flow_state(catch_entry);
                handler.body.visit_with(self);
                let catch_exit = self.flow_state();

                self.restore_flow_state(try_exit);
                self.join_current_flow_with(&catch_exit);
            }

            // `finally` executes on every path that leaves the construct, so
            // assignments made there can establish new facts after the join.
            if let Some(finalizer) = &statement.finalizer {
                finalizer.visit_with(self);
            }
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let Pat::Ident(binding) = &declaration.name {
                if let Some(ty) = binding
                    .type_ann
                    .as_ref()
                    .and_then(|annotation| {
                        source_ts_type(&annotation.type_ann, self.named_types)
                    })
                {
                    self.value_types.insert(binding.id.sym.to_string(), ty);
                }
            }
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                if let Some(class) = source_instance_class(
                    initializer,
                    self.classes,
                    self.factories,
                    &self.variables,
                ) {
                    self.variables.insert(binding.id.sym.to_string(), class);
                }
                let mut property_classes = Vec::new();
                collect_object_instance_classes(
                    initializer,
                    binding.id.sym.as_str(),
                    self.classes,
                    self.factories,
                    &self.variables,
                    &mut property_classes,
                );
                self.variables.extend(property_classes);
                if matches!(initializer.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                    self.callbacks.insert(binding.id.sym.to_string());
                }
                if binding.type_ann.is_none() {
                    if let Some(ty) =
                        source_expr_type(
                            initializer,
                            &self.value_types,
                            self.function_types,
                            self.named_types,
                        )
                    {
                        self.value_types.insert(binding.id.sym.to_string(), ty);
                    }
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    let static_target = match member.obj.as_ref() {
                        Expr::Ident(class) => Some((None, class.sym.as_str())),
                        Expr::Member(class_member) => {
                            match (class_member.obj.as_ref(), &class_member.prop) {
                                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                                    Some((Some(qualifier.sym.as_str()), class.sym.as_str()))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let (Some((qualifier, class)), MemberProp::Ident(method)) =
                        (static_target, &member.prop)
                    {
                        let has_callback = call.args.last().is_some_and(|argument| {
                            matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                        });
                        let selected = self
                            .static_methods
                            .iter()
                            .filter(|candidate| {
                                candidate.1 == class
                                    && candidate.2 == method.sym.as_str()
                                    && candidate.4 == call.args.len()
                                    && candidate.5 == has_callback
                                    && qualifier.is_none_or(|qualifier| candidate.0 == qualifier)
                            })
                            .filter_map(|candidate| {
                                let mut score = 0u16;
                                for (argument, declared) in call.args.iter().zip(&candidate.6) {
                                    if let Some(actual) = source_expr_type(
                                        argument.expr.as_ref(),
                                        &self.value_types,
                                        self.function_types,
                                        self.named_types,
                                    ) {
                                        score += u16::from(overload_type_score(declared, &actual)?);
                                    }
                                }
                                Some((score, candidate))
                            })
                            .reduce(|best, candidate| {
                                if candidate.0 > best.0 {
                                    candidate
                                } else {
                                    best
                                }
                            })
                            .map(|(_, candidate)| candidate);
                        if let Some(candidate) = selected {
                            let span = member.span();
                            self.edits.push((span.lo.0, span.hi.0, candidate.3.clone()));
                            call.visit_children_with(self);
                            return;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), MemberProp::Ident(method)) =
                        (instance_receiver(&member.obj), &member.prop)
                    {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            let has_callback = call.args.last().is_some_and(|argument| {
                                matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                    || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                            });
                            let selected = self
                                .methods
                                .iter()
                                .filter(
                                    |(
                                        candidate_class,
                                        candidate_method,
                                        _,
                                        argument_count,
                                        candidate_callback,
                                        _,
                                    )| {
                                        candidate_class == class
                                            && candidate_method == method.sym.as_str()
                                            && *argument_count == call.args.len()
                                            && *candidate_callback == has_callback
                                    },
                                )
                                .filter_map(|candidate| {
                                    let mut score = 0u16;
                                    for (argument, declared) in
                                        call.args.iter().zip(candidate.5.iter())
                                    {
                                        if let Some(actual) = source_expr_type(
                                            argument.expr.as_ref(),
                                            &self.value_types,
                                            self.function_types,
                                            self.named_types,
                                        ) {
                                            score +=
                                                u16::from(overload_type_score(declared, &actual)?);
                                        }
                                    }
                                    Some((score, candidate))
                                })
                                .reduce(|best, candidate| {
                                    if candidate.0 > best.0 {
                                        candidate
                                    } else {
                                        best
                                    }
                                })
                                .map(|(_, candidate)| candidate);
                            if let Some((_, _, helper, _, _, _)) = selected {
                                let span = member.span();
                                self.edits.push((span.lo.0, span.hi.0, helper.clone()));
                                let insertion = if call.args.is_empty() {
                                    receiver_source
                                } else {
                                    format!("{receiver_source}, ")
                                };
                                self.edits.push((span.hi.0 + 1, span.hi.0 + 1, insertion));
                            }
                        }
                    }
                } else if let Expr::Ident(name) = callee.as_ref() {
                    // A bare function call against a registry Fallback
                    // name with more than one `.d.ts` overload -- real
                    // example: uuid's `v4(options?): string` vs. its
                    // generic buffer-output `v4<TBuf extends Uint8Array =
                    // Uint8Array>(options, buf, offset?): TBuf`. Filters
                    // by arity range first (an overload set discriminated
                    // by argument *count*, like `v4`'s, needs no further
                    // scoring), then breaks a same-arity tie by comparing
                    // each argument's actual inferred type against the
                    // candidate's declared parameter types -- identical
                    // pattern to the static/instance-method branches
                    // above, just with no receiver to splice in.
                    let selected = self
                        .functions
                        .iter()
                        .filter(|(candidate_name, _, min_arity, max_arity, _, _)| {
                            candidate_name == name.sym.as_str()
                                && call.args.len() >= *min_arity
                                && call.args.len() <= *max_arity
                        })
                        .filter_map(|candidate| {
                            let mut score = 0u16;
                            for (argument, declared) in call.args.iter().zip(candidate.4.iter()) {
                                if let Some(actual) = source_expr_type(
                                    argument.expr.as_ref(),
                                    &self.value_types,
                                    self.function_types,
                                    self.named_types,
                                ) {
                                    score += u16::from(overload_type_score(declared, &actual)?);
                                }
                            }
                            Some((score, candidate))
                        })
                        .reduce(|best, candidate| {
                            if candidate.0 > best.0 {
                                candidate
                            } else {
                                best
                            }
                        })
                        .map(|(_, candidate)| candidate);
                    if let Some((_, symbol, _, _, _, generic)) = selected {
                        let annotated = generic.as_ref().is_some_and(|generic| {
                            annotate_generic_callback_arguments(
                                call,
                                generic,
                                &self.value_types,
                                self.function_types,
                                self.named_types,
                                &mut self.edits,
                            )
                        });
                        if !annotated {
                            for (candidate_name, _, min_arity, max_arity, _, generic) in
                                self.functions.iter()
                            {
                                if candidate_name != name.sym.as_str()
                                    || call.args.len() < *min_arity
                                    || call.args.len() > *max_arity
                                {
                                    continue;
                                }
                                if generic.as_ref().is_some_and(|generic| {
                                    annotate_generic_callback_arguments(
                                        call,
                                        generic,
                                        &self.value_types,
                                        self.function_types,
                                        self.named_types,
                                        &mut self.edits,
                                    )
                                }) {
                                    break;
                                }
                            }
                        }
                        let span = callee.span();
                        self.edits.push((span.lo.0, span.hi.0, symbol.clone()));
                    }
                }
            }
            call.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
            let mut setter_rewritten = false;
            if assignment.op == AssignOp::Assign {
                if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left {
                    if let (Some((qualifier, class)), Some(property)) = (
                        static_class_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some((_, _, _, helper, _)) = self.static_setters.iter().find(
                            |(
                                candidate_qualifier,
                                candidate_class,
                                candidate_property,
                                _,
                                declared,
                            )| {
                                candidate_class == &class
                                    && candidate_property == &property
                                    && qualifier
                                        .as_ref()
                                        .is_none_or(|qualifier| candidate_qualifier == qualifier)
                                    && source_expr_type(
                                        &assignment.right,
                                        &self.value_types,
                                        self.function_types,
                                        self.named_types,
                                    )
                                    .is_none_or(|actual| {
                                        overload_type_score(declared, &actual).is_some()
                                    })
                            },
                        ) {
                            let member_span = member.span();
                            let right_span = assignment.right.span();
                            let assignment_span = assignment.span();
                            self.edits
                                .push((member_span.lo.0, member_span.hi.0, helper.clone()));
                            self.edits
                                .push((member_span.hi.0, right_span.lo.0, "(".into()));
                            self.edits.push((
                                assignment_span.hi.0,
                                assignment_span.hi.0,
                                ")".into(),
                            ));
                            setter_rewritten = true;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), Some(property)) = (
                        instance_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            if let Some((_, _, helper, _)) =
                                self.setters.iter().find(
                                    |(candidate_class, candidate_property, _, declared)| {
                                        candidate_class == class
                                            && candidate_property == &property
                                            && source_expr_type(
                                                &assignment.right,
                                                &self.value_types,
                                                self.function_types,
                                                self.named_types,
                                            )
                                            .is_none_or(|actual| {
                                                overload_type_score(declared, &actual).is_some()
                                            })
                                    },
                                )
                            {
                                let member_span = member.span();
                                let right_span = assignment.right.span();
                                let assignment_span = assignment.span();
                                self.edits.push((
                                    member_span.lo.0,
                                    member_span.hi.0,
                                    helper.clone(),
                                ));
                                self.edits.push((
                                    member_span.hi.0,
                                    right_span.lo.0,
                                    format!("({receiver_source}, "),
                                ));
                                self.edits.push((
                                    assignment_span.hi.0,
                                    assignment_span.hi.0,
                                    ")".into(),
                                ));
                                setter_rewritten = true;
                            }
                        }
                    }
                }
            }
            if setter_rewritten {
                assignment.right.visit_with(self);
            } else {
                assignment.visit_children_with(self);
            }
            let inferred = (assignment.op == AssignOp::Assign)
                .then(|| {
                    source_expr_type(
                        &assignment.right,
                        &self.value_types,
                        self.function_types,
                        self.named_types,
                    )
                })
                .flatten();
            let assigned_class = (assignment.op == AssignOp::Assign)
                .then(|| {
                    source_instance_class(
                        &assignment.right,
                        self.classes,
                        self.factories,
                        &self.variables,
                    )
                })
                .flatten();
            let AssignTarget::Simple(target) = &assignment.left else {
                return;
            };
            match target {
                SimpleAssignTarget::Ident(binding) => {
                    if let Some(ty) = inferred {
                        self.value_types.insert(binding.id.sym.to_string(), ty);
                    } else {
                        self.value_types.remove(binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                        if let Some(class) = assigned_class {
                            self.variables.insert(binding.id.sym.to_string(), class);
                        }
                    } else {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign
                        && matches!(assignment.right.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                    {
                        self.callbacks.insert(binding.id.sym.to_string());
                    } else {
                        self.callbacks.remove(binding.id.sym.as_str());
                    }
                }
                SimpleAssignTarget::Member(member) => {
                    let (root, path) = match member_assignment_path(member) {
                        Some(path) => path,
                        None => return,
                    };
                    let instance_key = instance_path_key(&root, &path);
                    invalidate_instance_path(&mut self.variables, &instance_key);
                    if let Some(class) = assigned_class {
                        self.variables.insert(instance_key, class);
                    }
                    let Some(value) = inferred else {
                        self.value_types.remove(&root);
                        return;
                    };
                    if let Some(object) = self.value_types.get_mut(&root) {
                        if !update_object_property_type(object, &path, value) {
                            self.value_types.remove(&root);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut named_declarations = NamedTypeDeclarationFinder::default();
    module.visit_with(&mut named_declarations);
    let mut named_types = std::collections::HashMap::new();
    let interface_names = named_declarations
        .interfaces
        .iter()
        .map(|interface| interface.id.sym.to_string())
        .collect::<std::collections::HashSet<_>>();
    for _ in 0..=named_declarations.aliases.len() + named_declarations.interfaces.len() {
        for alias in &named_declarations.aliases {
            if alias.type_params.is_none() {
                if let Some(ty) = source_ts_type(&alias.type_ann, &named_types) {
                    named_types.insert(alias.id.sym.to_string(), ty);
                }
            }
        }
        for name in &interface_names {
            if let Some(ty) = source_interfaces_type(
                name,
                &named_declarations.interfaces,
                &named_types,
            ) {
                named_types.insert(name.clone(), ty);
            }
        }
    }
    let mut function_types = FunctionTypeFinder {
        named: &named_types,
        types: std::collections::HashMap::new(),
    };
    module.visit_with(&mut function_types);
    loop {
        let mut inferred = InferredFunctionTypeFinder {
            known: &function_types.types,
            named: &named_types,
            additions: std::collections::HashMap::new(),
        };
        module.visit_with(&mut inferred);
        inferred
            .additions
            .retain(|name, _| !function_types.types.contains_key(name));
        if inferred.additions.is_empty() {
            break;
        }
        function_types.types.extend(inferred.additions);
    }
    let mut finder = Finder {
        classes,
        factories,
        functions,
        methods,
        static_methods,
        getters,
        setters,
        static_getters,
        static_setters,
        variables: std::collections::HashMap::new(),
        value_types: std::collections::HashMap::new(),
        function_types: &function_types.types,
        named_types: &named_types,
        callbacks: std::collections::HashSet::new(),
        edits: Vec::new(),
        switch_break_depth: 0,
        switch_break_exits: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut edits = finder
        .edits
        .into_iter()
        .map(|(lo, hi, replacement)| {
            let lo = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(lo))
                .pos
                .0 as usize;
            let hi = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(hi))
                .pos
                .0 as usize;
            (lo, hi, replacement)
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(lo, hi, _)| (*lo, *hi));
    let mut output = source.to_string();
    for (lo, hi, replacement) in edits.into_iter().rev() {
        output.replace_range(lo..hi, &replacement);
    }
    Ok(output)
}
