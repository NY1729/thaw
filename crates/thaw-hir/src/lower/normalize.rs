type EnumValues = HashMap<(Symbol, Symbol), HirLit>;
type EnumReverseValues = HashMap<Symbol, Vec<(f64, Symbol)>>;

fn enum_member_name(id: &swc_ecma_ast::TsEnumMemberId) -> Symbol {
    match id {
        swc_ecma_ast::TsEnumMemberId::Ident(id) => id.sym.to_string(),
        swc_ecma_ast::TsEnumMemberId::Str(value) => value.value.to_string_lossy().into_owned(),
    }
}

fn eval_enum_initializer(
    expr: &Expr,
    enum_name: &str,
    values: &EnumValues,
) -> Result<HirLit, String> {
    match expr {
        Expr::Lit(Lit::Num(value)) => Ok(HirLit::F64(value.value)),
        Expr::Lit(Lit::Str(value)) => {
            Ok(hir_string_literal_from_wtf8(value.value.as_wtf8().as_bytes()))
        }
        Expr::Paren(value) => eval_enum_initializer(&value.expr, enum_name, values),
        Expr::Unary(unary) => {
            let HirLit::F64(value) = eval_enum_initializer(&unary.arg, enum_name, values)? else {
                return Err(format!(
                    "enum `{enum_name}` unary initializer requires a numeric operand"
                ));
            };
            match unary.op {
                UnaryOp::Plus => Ok(HirLit::F64(value)),
                UnaryOp::Minus => Ok(HirLit::F64(-value)),
                UnaryOp::Tilde => Ok(HirLit::F64((!(value as i32)) as f64)),
                other => Err(format!(
                    "enum `{enum_name}` has unsupported unary initializer {other:?}"
                )),
            }
        }
        Expr::Ident(member) => values
            .get(&(enum_name.to_string(), member.sym.to_string()))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "enum `{enum_name}` initializer references unknown preceding member `{}`",
                    member.sym
                )
            }),
        Expr::Member(member) => {
            let Expr::Ident(target_enum) = member.obj.as_ref() else {
                return Err(format!(
                    "enum `{enum_name}` initializer has unsupported member reference"
                ));
            };
            let member_name = match &member.prop {
                MemberProp::Ident(member) => member.sym.to_string(),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(member)) => member.value.to_string_lossy().into_owned(),
                    _ => {
                        return Err(format!(
                        "enum `{enum_name}` computed initializer member must be a string literal"
                    ))
                    }
                },
                _ => {
                    return Err(format!(
                        "enum `{enum_name}` has unsupported initializer member"
                    ))
                }
            };
            values
                .get(&(target_enum.sym.to_string(), member_name.clone()))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "enum `{enum_name}` initializer references unknown member `{}.{member_name}`",
                        target_enum.sym
                    )
                })
        }
        Expr::Bin(binary) => {
            let HirLit::F64(left) = eval_enum_initializer(&binary.left, enum_name, values)? else {
                return Err(format!(
                    "enum `{enum_name}` binary initializer requires numeric operands"
                ));
            };
            let HirLit::F64(right) = eval_enum_initializer(&binary.right, enum_name, values)?
            else {
                return Err(format!(
                    "enum `{enum_name}` binary initializer requires numeric operands"
                ));
            };
            let value = match binary.op {
                BinaryOp::Add => left + right,
                BinaryOp::Sub => left - right,
                BinaryOp::Mul => left * right,
                BinaryOp::Div => left / right,
                BinaryOp::Mod => left % right,
                BinaryOp::Exp if right.is_nan() || (right.is_infinite() && left.abs() == 1.0) => {
                    f64::NAN
                }
                BinaryOp::Exp => left.powf(right),
                BinaryOp::BitOr => ((left as i32) | (right as i32)) as f64,
                BinaryOp::BitXor => ((left as i32) ^ (right as i32)) as f64,
                BinaryOp::BitAnd => ((left as i32) & (right as i32)) as f64,
                BinaryOp::LShift => ((left as i32) << ((right as u32) & 31)) as f64,
                BinaryOp::RShift => ((left as i32) >> ((right as u32) & 31)) as f64,
                BinaryOp::ZeroFillRShift => ((left as u32) >> ((right as u32) & 31)) as f64,
                other => {
                    return Err(format!(
                        "enum `{enum_name}` has unsupported binary initializer {other:?}"
                    ))
                }
            };
            Ok(HirLit::F64(value))
        }
        other => Err(format!(
            "enum `{enum_name}` has unsupported initializer expression {other:?}"
        )),
    }
}

fn collect_enums(
    module: &Module,
) -> Result<(EnumValues, EnumReverseValues, HashMap<Symbol, HirType>), String> {
    let mut values = EnumValues::new();
    let mut reverse_values = EnumReverseValues::new();
    let mut types = HashMap::new();
    let mut enum_names = BTreeSet::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(declaration))) = item else {
            continue;
        };
        let name = declaration.id.sym.to_string();
        enum_names.insert(name.clone());
        let mut enum_type = types.get(&name).cloned();
        let mut next_number = Some(0.0);
        for member in &declaration.members {
            let member_name = enum_member_name(&member.id);
            let value = if let Some(initializer) = &member.init {
                eval_enum_initializer(initializer, &name, &values)?
            } else {
                HirLit::F64(next_number.ok_or_else(|| {
                    format!(
                        "enum `{name}` member `{member_name}` needs an initializer after a string member"
                    )
                })?)
            };
            let ty = match &value {
                HirLit::F64(number) => {
                    next_number = Some(number + 1.0);
                    let reverse = reverse_values.entry(name.clone()).or_default();
                    reverse.retain(|(existing, _)| existing != number);
                    reverse.push((*number, member_name.clone()));
                    HirType::F64
                }
                HirLit::Str(_) => {
                    next_number = None;
                    HirType::Str
                }
                _ => unreachable!("enum evaluator only produces number or string literals"),
            };
            if enum_type.as_ref().is_some_and(|existing| existing != &ty) {
                return Err(format!(
                    "enum `{name}` mixes numeric and string members, which has no single native layout"
                ));
            }
            enum_type.get_or_insert(ty);
            if values
                .insert((name.clone(), member_name.clone()), value)
                .is_some()
            {
                return Err(format!(
                    "enum `{name}` contains duplicate member `{member_name}`"
                ));
            }
        }
        if let Some(ty) = enum_type {
            types.insert(name, ty);
        }
    }
    for name in enum_names {
        if !types.contains_key(&name) {
            return Err(format!(
                "enum `{name}` must contain at least one member across its declarations"
            ));
        }
    }
    Ok((values, reverse_values, types))
}

pub fn normalize_top_level_destructuring(module: &Module) -> Result<Module, String> {
    fn binding_annotation(pattern: &Pat) -> Option<Box<swc_ecma_ast::TsTypeAnn>> {
        match pattern {
            Pat::Ident(binding) => binding.type_ann.clone(),
            Pat::Array(pattern) => pattern.type_ann.clone(),
            Pat::Object(pattern) => pattern.type_ann.clone(),
            _ => None,
        }
    }

    fn member(object: Expr, key: &PropName, span: swc_common::Span) -> Result<Expr, String> {
        let prop = match key {
            PropName::Ident(identifier) => MemberProp::Ident(identifier.clone()),
            PropName::Str(string) => MemberProp::Computed(ComputedPropName {
                span,
                expr: Box::new(Expr::Lit(Lit::Str(string.clone()))),
            }),
            PropName::Computed(computed) => MemberProp::Computed(computed.clone()),
            _ => return Err("unsupported top-level destructuring property key".into()),
        };
        Ok(Expr::Member(MemberExpr {
            span,
            obj: Box::new(object),
            prop,
        }))
    }

    fn property_expression(key: &PropName) -> Result<Expr, String> {
        match key {
            PropName::Ident(identifier) => Ok(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                span: identifier.span,
                value: identifier.sym.clone().into(),
                raw: None,
            }))),
            PropName::Str(string) => Ok(Expr::Lit(Lit::Str(string.clone()))),
            PropName::Num(number) => Ok(Expr::Lit(Lit::Num(number.clone()))),
            PropName::Computed(computed) => Ok(computed.expr.as_ref().clone()),
            _ => Err("unsupported top-level object rest key".into()),
        }
    }

    fn undefined_default(
        value: Expr,
        default: Box<Expr>,
        span: swc_common::Span,
        counter: &mut usize,
        used: &mut HashSet<String>,
        out: &mut Vec<swc_ecma_ast::VarDecl>,
    ) -> Expr {
        let temporary = loop {
            let candidate = format!("__thaw_top_default_{}", *counter);
            *counter += 1;
            if used.insert(candidate.clone()) {
                break candidate;
            }
        };
        let identifier = swc_ecma_ast::Ident::new_no_ctxt(temporary.into(), span);
        out.push(swc_ecma_ast::VarDecl {
            span,
            ctxt: Default::default(),
            kind: swc_ecma_ast::VarDeclKind::Const,
            declare: false,
            decls: vec![swc_ecma_ast::VarDeclarator {
                span,
                name: Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: identifier.clone(),
                    type_ann: None,
                }),
                init: Some(Box::new(value)),
                definite: false,
            }],
        });
        let temporary = || Expr::Ident(identifier.clone());
        Expr::Cond(swc_ecma_ast::CondExpr {
            span,
            test: Box::new(Expr::Bin(swc_ecma_ast::BinExpr {
                span,
                op: BinaryOp::EqEqEq,
                left: Box::new(temporary()),
                right: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    "undefined".into(),
                    span,
                ))),
            })),
            cons: default,
            alt: Box::new(temporary()),
        })
    }

    fn expand_pattern(
        pattern: &Pat,
        value: Expr,
        kind: swc_ecma_ast::VarDeclKind,
        span: swc_common::Span,
        counter: &mut usize,
        used: &mut HashSet<String>,
        out: &mut Vec<swc_ecma_ast::VarDecl>,
    ) -> Result<(), String> {
        if let Pat::Assign(assign) = pattern {
            let value = undefined_default(value, assign.right.clone(), span, counter, used, out);
            return expand_pattern(&assign.left, value, kind, span, counter, used, out);
        }
        if let Pat::Ident(binding) = pattern {
            out.push(swc_ecma_ast::VarDecl {
                span,
                ctxt: Default::default(),
                kind,
                declare: false,
                decls: vec![swc_ecma_ast::VarDeclarator {
                    span,
                    name: Pat::Ident(binding.clone()),
                    init: Some(Box::new(value)),
                    definite: false,
                }],
            });
            return Ok(());
        }

        let temporary = loop {
            let candidate = format!("__thaw_top_destructure_{}", *counter);
            *counter += 1;
            if used.insert(candidate.clone()) {
                break candidate;
            }
        };
        let temporary_ident = swc_ecma_ast::Ident::new_no_ctxt(temporary.clone().into(), span);
        out.push(swc_ecma_ast::VarDecl {
            span,
            ctxt: Default::default(),
            kind: swc_ecma_ast::VarDeclKind::Const,
            declare: false,
            decls: vec![swc_ecma_ast::VarDeclarator {
                span,
                name: Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: temporary_ident.clone(),
                    type_ann: binding_annotation(pattern),
                }),
                init: Some(Box::new(value)),
                definite: false,
            }],
        });
        let temporary_expr = || Expr::Ident(temporary_ident.clone());

        match pattern {
            Pat::Object(object) => {
                let mut used_keys = Vec::new();
                let has_rest = object
                    .props
                    .iter()
                    .any(|property| matches!(property, ObjectPatProp::Rest(_)));
                for property in &object.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = PropName::Ident(swc_ecma_ast::IdentName::new(
                                property.key.id.sym.clone(),
                                property.key.id.span,
                            ));
                            used_keys.push(property_expression(&key)?);
                            let mut value = member(temporary_expr(), &key, span)?;
                            if let Some(default) = &property.value {
                                value = undefined_default(
                                    value,
                                    default.clone(),
                                    span,
                                    counter,
                                    used,
                                    out,
                                );
                            }
                            expand_pattern(
                                &Pat::Ident(property.key.clone()),
                                value,
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let mut member_key = property.key.clone();
                            if has_rest {
                                let mut omitted_key = property_expression(&property.key)?;
                                if matches!(property.key, PropName::Computed(_)) {
                                    let temporary = loop {
                                        let candidate = format!("__thaw_top_key_{}", *counter);
                                        *counter += 1;
                                        if used.insert(candidate.clone()) {
                                            break candidate;
                                        }
                                    };
                                    let identifier = swc_ecma_ast::Ident::new_no_ctxt(
                                        temporary.into(),
                                        span,
                                    );
                                    out.push(swc_ecma_ast::VarDecl {
                                        span,
                                        ctxt: Default::default(),
                                        kind: swc_ecma_ast::VarDeclKind::Const,
                                        declare: false,
                                        decls: vec![swc_ecma_ast::VarDeclarator {
                                            span,
                                            name: Pat::Ident(swc_ecma_ast::BindingIdent {
                                                id: identifier.clone(),
                                                type_ann: None,
                                            }),
                                            init: Some(Box::new(omitted_key)),
                                            definite: false,
                                        }],
                                    });
                                    omitted_key = Expr::Ident(identifier.clone());
                                    member_key = PropName::Computed(ComputedPropName {
                                        span,
                                        expr: Box::new(Expr::Ident(identifier)),
                                    });
                                }
                                used_keys.push(omitted_key);
                            }
                            expand_pattern(
                                &property.value,
                                member(temporary_expr(), &member_key, span)?,
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let mut args = vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(temporary_expr()),
                            }];
                            args.extend(used_keys.iter().cloned().map(|key| swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(key),
                            }));
                            expand_pattern(
                                &rest.arg,
                                Expr::Call(CallExpr {
                                    span,
                                    ctxt: Default::default(),
                                    callee: Callee::Expr(Box::new(Expr::Ident(
                                        swc_ecma_ast::Ident::new_no_ctxt(
                                            "__thaw_object_rest".into(),
                                            span,
                                        ),
                                    ))),
                                    args,
                                    type_args: None,
                                }),
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                    }
                }
            }
            Pat::Array(array) => {
                for (index, element) in array.elems.iter().enumerate() {
                    let Some(element) = element else { continue };
                    if let Pat::Rest(rest) = element {
                        let slice = Expr::Call(CallExpr {
                            span,
                            ctxt: Default::default(),
                            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                span,
                                obj: Box::new(temporary_expr()),
                                prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                                    "slice".into(),
                                    span,
                                )),
                            }))),
                            args: vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                                    span,
                                    value: index as f64,
                                    raw: None,
                                }))),
                            }],
                            type_args: None,
                        });
                        expand_pattern(&rest.arg, slice, kind, span, counter, used, out)?;
                        continue;
                    }
                    let value = Expr::Member(MemberExpr {
                        span,
                        obj: Box::new(temporary_expr()),
                        prop: MemberProp::Computed(ComputedPropName {
                            span,
                            expr: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                                span,
                                value: index as f64,
                                raw: None,
                            }))),
                        }),
                    });
                    expand_pattern(element, value, kind, span, counter, used, out)?;
                }
            }
            _ => return Err("unsupported top-level destructuring pattern".into()),
        }
        Ok(())
    }

    fn expand_declaration(
        declaration: &VarDecl,
        exported: bool,
        counter: &mut usize,
        used: &mut HashSet<String>,
    ) -> Result<Vec<ModuleItem>, String> {
        let mut items = Vec::new();
        for declarator in &declaration.decls {
            let init = declarator
                .init
                .as_deref()
                .ok_or("top-level destructuring declarations need an initializer")?
                .clone();
            let mut declarations = Vec::new();
            expand_pattern(
                &declarator.name,
                init,
                declaration.kind,
                declaration.span,
                counter,
                used,
                &mut declarations,
            )?;
            for declaration in declarations {
                let is_temporary = declaration.decls.first().is_some_and(|declarator| {
                    matches!(&declarator.name, Pat::Ident(binding)
                        if binding.id.sym.starts_with("__thaw_top_destructure_")
                            || binding.id.sym.starts_with("__thaw_top_default_")
                            || binding.id.sym.starts_with("__thaw_top_key_"))
                });
                if exported && !is_temporary {
                    items.push(ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(
                        swc_ecma_ast::ExportDecl {
                            span: declaration.span,
                            decl: Decl::Var(Box::new(declaration)),
                        },
                    )));
                } else {
                    items.push(ModuleItem::Stmt(Stmt::Decl(Decl::Var(Box::new(
                        declaration,
                    )))));
                }
            }
        }
        Ok(items)
    }

    let mut used = HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(declaration)) => {
                used.extend(declaration_names_for_normalization(declaration));
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                used.extend(declaration_names_for_normalization(&export.decl));
            }
            _ => {}
        }
    }
    let mut counter = 0;
    let mut body = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration)))
                if declaration
                    .decls
                    .iter()
                    .any(|declarator| !matches!(declarator.name, Pat::Ident(_))) =>
            {
                body.extend(expand_declaration(
                    declaration.as_ref(),
                    false,
                    &mut counter,
                    &mut used,
                )?);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) if matches!(&export.decl, Decl::Var(declaration) if declaration.decls.iter().any(|declarator| !matches!(declarator.name, Pat::Ident(_)))) =>
            {
                let Decl::Var(declaration) = &export.decl else {
                    unreachable!()
                };
                body.extend(expand_declaration(
                    declaration.as_ref(),
                    true,
                    &mut counter,
                    &mut used,
                )?);
            }
            _ => body.push(item.clone()),
        }
    }
    let mut normalized = module.clone();
    normalized.body = body;
    Ok(normalized)
}

/// True if a function's own body uses `this` -- the classic constructor
/// shape (`function F(...) { this.x = ...; }`).
fn constructor_function_uses_this(function: &swc_ecma_ast::Function) -> bool {
    struct Finder(bool);
    impl swc_ecma_visit::Visit for Finder {
        fn visit_expr(&mut self, expr: &Expr) {
            if matches!(expr, Expr::This(_)) {
                self.0 = true;
            }
            expr.visit_children_with(self);
        }
        // A nested (non-arrow) function has its own `this`; a `this` inside
        // it doesn't make the *enclosing* function a constructor. An arrow
        // keeps the enclosing `this`, so it is still descended into.
        fn visit_function(&mut self, _function: &swc_ecma_ast::Function) {}
    }
    let mut finder = Finder(false);
    if let Some(body) = &function.body {
        body.visit_with(&mut finder);
    }
    finder.0
}

/// The `(owner, on_prototype, name, right)` of an assignment expression whose
/// target is `F.name` (static) or `F.prototype.name` (instance).
fn constructor_assignment_target(expr: &Expr) -> Option<(Symbol, bool, Symbol, &Expr)> {
    let Expr::Assign(assign) = expr else {
        return None;
    };
    if assign.op != swc_ecma_ast::AssignOp::Assign {
        return None;
    }
    let swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Member(member)) =
        &assign.left
    else {
        return None;
    };
    let name: Symbol = match &member.prop {
        swc_ecma_ast::MemberProp::Ident(ident) => ident.sym.to_string(),
        _ => return None,
    };
    match member.obj.as_ref() {
        Expr::Ident(owner) => (name != "prototype").then_some((
            owner.sym.to_string(),
            false,
            name,
            assign.right.as_ref(),
        )),
        Expr::Member(inner) => {
            let Expr::Ident(owner) = inner.obj.as_ref() else {
                return None;
            };
            if !matches!(&inner.prop, swc_ecma_ast::MemberProp::Ident(prop) if prop.sym == *"prototype")
            {
                return None;
            }
            Some((owner.sym.to_string(), true, name, assign.right.as_ref()))
        }
        _ => None,
    }
}

fn ts_any_type() -> swc_ecma_ast::TsType {
    use swc_ecma_ast::*;
    TsType::TsKeywordType(TsKeywordType {
        span: swc_common::DUMMY_SP,
        kind: TsKeywordTypeKind::TsAnyKeyword,
    })
}

/// A best-effort TS type for a field from its assigned RHS (a string/number/
/// boolean literal, or `a || b`/`a + b`/a ternary of those). `None` means the
/// caller uses `any`.
fn inferred_field_type(
    right: &Expr,
    param_types: &HashMap<Symbol, swc_ecma_ast::TsType>,
) -> Option<swc_ecma_ast::TsType> {
    use swc_ecma_ast::*;
    let keyword = |kind| {
        Some(TsType::TsKeywordType(TsKeywordType {
            span: swc_common::DUMMY_SP,
            kind,
        }))
    };
    match right {
        Expr::Lit(Lit::Str(_)) => keyword(TsKeywordTypeKind::TsStringKeyword),
        Expr::Lit(Lit::Num(_)) => keyword(TsKeywordTypeKind::TsNumberKeyword),
        Expr::Lit(Lit::Bool(_)) => keyword(TsKeywordTypeKind::TsBooleanKeyword),
        // `this.x = x` where `x` is a typed constructor parameter: reuse the
        // parameter's own annotation as the field type.
        Expr::Ident(ident) => param_types.get(&ident.sym.to_string()).cloned(),
        Expr::Paren(paren) => inferred_field_type(&paren.expr, param_types),
        Expr::Bin(bin) => inferred_field_type(&bin.left, param_types)
            .or_else(|| inferred_field_type(&bin.right, param_types)),
        Expr::Cond(cond) => inferred_field_type(&cond.cons, param_types)
            .or_else(|| inferred_field_type(&cond.alt, param_types)),
        Expr::TsAs(assertion) => inferred_field_type(&assertion.expr, param_types),
        _ => None,
    }
}

/// Collects the `this.<field>` names (and a best-effort type) a constructor
/// body uses, excluding prototype-assigned method names. Any `this.p` read
/// but never assigned becomes an `any` field, so a method that reads it still
/// compiles.
fn constructor_fields(
    function: &swc_ecma_ast::Function,
    methods: &std::collections::HashSet<Symbol>,
    param_types: &HashMap<Symbol, swc_ecma_ast::TsType>,
) -> Vec<(Symbol, swc_ecma_ast::TsType)> {
    struct Collect<'a> {
        methods: std::collections::HashSet<Symbol>,
        param_types: &'a HashMap<Symbol, swc_ecma_ast::TsType>,
        assigned: Vec<(Symbol, swc_ecma_ast::TsType)>,
        read: Vec<Symbol>,
    }
    impl swc_ecma_visit::Visit for Collect<'_> {
        fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
            if let swc_ecma_ast::AssignTarget::Simple(
                swc_ecma_ast::SimpleAssignTarget::Member(member),
            ) = &assign.left
            {
                if let Expr::This(_) = member.obj.as_ref() {
                    if let swc_ecma_ast::MemberProp::Ident(ident) = &member.prop {
                        let name: Symbol = ident.sym.to_string();
                        if !self.methods.contains(&name)
                            && !self.assigned.iter().any(|(existing, _)| *existing == name)
                        {
                            let ty = inferred_field_type(&assign.right, self.param_types)
                                .unwrap_or_else(ts_any_type);
                            self.assigned.push((name, ty));
                        }
                    }
                }
            }
            assign.visit_children_with(self);
        }
        fn visit_member_expr(&mut self, member: &swc_ecma_ast::MemberExpr) {
            if let Expr::This(_) = member.obj.as_ref() {
                if let swc_ecma_ast::MemberProp::Ident(ident) = &member.prop {
                    self.read.push(ident.sym.to_string());
                }
            }
            member.visit_children_with(self);
        }
    }
    let mut collect = Collect {
        methods: methods.clone(),
        param_types,
        assigned: Vec::new(),
        read: Vec::new(),
    };
    function.visit_with(&mut collect);
    for name in collect.read {
        if !methods.contains(&name) && !collect.assigned.iter().any(|(existing, _)| *existing == name) {
            collect.assigned.push((name, ts_any_type()));
        }
    }
    collect.assigned
}

/// Gives every plain parameter a `= undefined` default, matching
/// JavaScript's "a missing argument is `undefined`" for the converted
/// constructor-function members (e.g. `assert.throws(Ctor, fn)` omitting the
/// optional `message`). A default -- unlike an `?` optional -- leaves the
/// declared type unchanged, so the body still sees `any`/`number`, not
/// `T | undefined`.
fn make_params_default_undefined(params: &mut [swc_ecma_ast::Param]) {
    use swc_ecma_ast::*;
    for parameter in params {
        let Pat::Ident(binding) = &parameter.pat else {
            continue;
        };
        // Only an untyped/`any`/`unknown` parameter tolerates a `= undefined`
        // default; a concretely typed one (e.g. `boolean`) would then reject
        // the default at the assignment site.
        let permissive = match &binding.type_ann {
            None => true,
            Some(annotation) => matches!(
                annotation.type_ann.as_ref(),
                TsType::TsKeywordType(TsKeywordType {
                    kind: TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword,
                    ..
                })
            ),
        };
        if !permissive {
            continue;
        }
        parameter.pat = Pat::Assign(AssignPat {
            span: swc_common::DUMMY_SP,
            left: Box::new(Pat::Ident(binding.clone())),
            right: Box::new(Expr::Ident(Ident::new_no_ctxt(
                "undefined".into(),
                swc_common::DUMMY_SP,
            ))),
        });
    }
}

/// Converts a prototype-assigned RHS (a function expression or arrow) into a
/// method's `Function`, or `None` if it isn't callable.
fn rhs_function(right: &Expr) -> Option<swc_ecma_ast::Function> {
    use swc_ecma_ast::*;
    match right {
        Expr::Fn(function) => Some((*function.function).clone()),
        Expr::Arrow(arrow) => {
            let params = arrow
                .params
                .iter()
                .cloned()
                .map(|pat| Param {
                    span: swc_common::DUMMY_SP,
                    decorators: Vec::new(),
                    pat,
                })
                .collect();
            let body = match arrow.body.as_ref() {
                ArrowFunctionBody::FunctionBody(block) => Some(block.clone()),
                ArrowFunctionBody::Expr(expr) => Some(FunctionBody {
                    span: swc_common::DUMMY_SP,
                    stmts: vec![Stmt::Return(ReturnStmt {
                        span: swc_common::DUMMY_SP,
                        arg: Some(expr.clone()),
                    })],
                }),
            };
            Some(Function {
                params,
                decorators: Vec::new(),
                span: swc_common::DUMMY_SP,
                ctxt: Default::default(),
                body,
                is_generator: arrow.is_generator,
                is_async: arrow.is_async,
                type_params: arrow.type_params.clone(),
                return_type: arrow.return_type.clone(),
                this_param: None,
            })
        }
        Expr::Paren(paren) => rhs_function(&paren.expr),
        _ => None,
    }
}

/// Builds the class that replaces a constructor-function declaration.
fn constructor_class(
    function: &swc_ecma_ast::FnDecl,
    assignments: &[(bool, Symbol, &Expr)],
    is_constructor: bool,
    prototype_replacement: Option<&Expr>,
    functions: &HashMap<Symbol, &swc_ecma_ast::Function>,
) -> Result<swc_ecma_ast::ClassDecl, String> {
    use swc_ecma_ast::*;

    // A member assigned an existing function's *name* (`assert._toString =
    // formatSimpleValue`) becomes a method that reuses that function.
    let resolve_function = |right: &Expr| -> Option<Function> {
        match right {
            Expr::Ident(ident) => functions.get(&ident.sym.to_string()).map(|f| (*f).clone()),
            _ => rhs_function(right),
        }
    };

    let mut instance_methods = Vec::new();
    let mut statics = Vec::new();
    for (on_prototype, name, right) in assignments {
        if *on_prototype {
            instance_methods.push((name.clone(), *right));
        } else {
            statics.push((name.clone(), *right));
        }
    }
    let method_names: std::collections::HashSet<Symbol> =
        instance_methods.iter().map(|(name, _)| name.clone()).collect();
    let mut param_types: HashMap<Symbol, TsType> = HashMap::new();
    for parameter in &function.function.params {
        if let Pat::Ident(binding) = &parameter.pat {
            if let Some(annotation) = &binding.type_ann {
                param_types.insert(
                    binding.id.sym.to_string(),
                    annotation.type_ann.as_ref().clone(),
                );
            }
        }
    }
    let fields = constructor_fields(&function.function, &method_names, &param_types);

    let property = |name: &Symbol| {
        PropName::Ident(IdentName::new(name.as_str().into(), swc_common::DUMMY_SP))
    };
    let mut body: Vec<ClassMember> = Vec::new();
    for (name, ty) in fields {
        body.push(ClassMember::ClassProp(ClassProp {
            span: swc_common::DUMMY_SP,
            key: property(&name),
            value: None,
            type_ann: Some(Box::new(TsTypeAnn {
                span: swc_common::DUMMY_SP,
                type_ann: Box::new(ty),
            })),
            is_static: false,
            decorators: Vec::new(),
            accessibility: None,
            is_abstract: false,
            is_optional: false,
            is_override: false,
            readonly: false,
            declare: false,
            definite: false,
        }));
    }
    if is_constructor {
        let mut params = function.function.params.clone();
        make_params_default_undefined(&mut params);
        body.push(ClassMember::Constructor(Constructor {
            span: swc_common::DUMMY_SP,
            ctxt: Default::default(),
            key: property(&"constructor".to_string()),
            params: params.into_iter().map(ParamOrTsParamProp::Param).collect(),
            body: function.function.body.clone(),
            accessibility: None,
            is_optional: false,
        }));
    } else {
        // A namespace-shaped function (`assert`): keep its body as the static
        // `__call__`, and rewrite its plain call sites to `F.__call__(...)`.
        let mut call_function = (*function.function).clone();
        make_params_default_undefined(&mut call_function.params);
        body.push(ClassMember::Method(ClassMethod {
            span: swc_common::DUMMY_SP,
            key: property(&"__call__".to_string()),
            function: Box::new(call_function),
            kind: MethodKind::Method,
            is_static: true,
            accessibility: None,
            is_abstract: false,
            is_optional: false,
            is_override: false,
        }));
    }
    for (name, right) in &instance_methods {
        let Some(mut method_function) = resolve_function(right) else {
            return Err(format!(
                "constructor-function prototype member `{name}` is not a function"
            ));
        };
        make_params_default_undefined(&mut method_function.params);
        body.push(ClassMember::Method(ClassMethod {
            span: swc_common::DUMMY_SP,
            key: property(name),
            function: Box::new(method_function),
            kind: MethodKind::Method,
            is_static: false,
            accessibility: None,
            is_abstract: false,
            is_optional: false,
            is_override: false,
        }));
    }
    for (name, right) in &statics {
        if let Some(mut function) = resolve_function(right) {
            make_params_default_undefined(&mut function.params);
            body.push(ClassMember::Method(ClassMethod {
                span: swc_common::DUMMY_SP,
                key: property(name),
                function: Box::new(function),
                kind: MethodKind::Method,
                is_static: true,
                accessibility: None,
                is_abstract: false,
                is_optional: false,
                is_override: false,
            }));
        } else {
            body.push(ClassMember::ClassProp(ClassProp {
                span: swc_common::DUMMY_SP,
                key: property(name),
                value: Some(Box::new((**right).clone())),
                type_ann: Some(Box::new(TsTypeAnn {
                    span: swc_common::DUMMY_SP,
                    type_ann: Box::new(
                        inferred_field_type(right, &param_types).unwrap_or_else(ts_any_type),
                    ),
                })),
                is_static: true,
                decorators: Vec::new(),
                accessibility: None,
                is_abstract: false,
                is_optional: false,
                is_override: false,
                readonly: false,
                declare: false,
                definite: false,
            }));
        }
    }
    // `F.prototype = { m() {...}, x: 1 }` -- fold the object literal's
    // identifier-keyed members into the class as instance methods/fields
    // (numeric/string keys and a non-literal RHS are left unsupported).
    if let Some(Expr::Object(object)) = prototype_replacement {
        for prop in &object.props {
            let PropOrSpread::Prop(prop) = prop else {
                continue;
            };
            match prop.as_ref() {
                Prop::Method(method) => {
                    let PropName::Ident(key) = &method.key else {
                        continue;
                    };
                    let mut method_function = (*method.function).clone();
                    make_params_default_undefined(&mut method_function.params);
                    body.push(ClassMember::Method(ClassMethod {
                        span: swc_common::DUMMY_SP,
                        key: PropName::Ident(key.clone()),
                        function: Box::new(method_function),
                        kind: MethodKind::Method,
                        is_static: false,
                        accessibility: None,
                        is_abstract: false,
                        is_optional: false,
                        is_override: false,
                    }));
                }
                Prop::KeyValue(pair) => {
                    let PropName::Ident(key) = &pair.key else {
                        continue;
                    };
                    // `{ greet: function () {...} }` is a method, not a field.
                    if let Some(mut method_function) = resolve_function(&pair.value) {
                        make_params_default_undefined(&mut method_function.params);
                        body.push(ClassMember::Method(ClassMethod {
                            span: swc_common::DUMMY_SP,
                            key: PropName::Ident(key.clone()),
                            function: Box::new(method_function),
                            kind: MethodKind::Method,
                            is_static: false,
                            accessibility: None,
                            is_abstract: false,
                            is_optional: false,
                            is_override: false,
                        }));
                        continue;
                    }
                    let ty = inferred_field_type(&pair.value, &param_types)
                        .unwrap_or_else(ts_any_type);
                    body.push(ClassMember::ClassProp(ClassProp {
                        span: swc_common::DUMMY_SP,
                        key: PropName::Ident(key.clone()),
                        value: Some(pair.value.clone()),
                        type_ann: Some(Box::new(TsTypeAnn {
                            span: swc_common::DUMMY_SP,
                            type_ann: Box::new(ty),
                        })),
                        is_static: false,
                        decorators: Vec::new(),
                        accessibility: None,
                        is_abstract: false,
                        is_optional: false,
                        is_override: false,
                        readonly: false,
                        declare: false,
                        definite: false,
                    }));
                }
                _ => {}
            }
        }
    }
    Ok(ClassDecl {
        ident: function.ident.clone(),
        declare: false,
        class: Box::new(Class {
            span: swc_common::DUMMY_SP,
            ctxt: Default::default(),
            decorators: Vec::new(),
            body,
            super_class: None,
            is_abstract: false,
            type_params: None,
            super_type_params: None,
            implements: Vec::new(),
        }),
    })
}

/// A break at the current loop level can reach the statement after a
/// `while (true)`. Breaks inside nested loops or switches target those
/// constructs instead; a labeled break may leave the outer loop.
fn ast_loop_body_may_break(statement: &swc_ecma_ast::Stmt, nested: usize) -> bool {
    use swc_ecma_ast::Stmt;
    match statement {
        Stmt::Break(value) => value.label.is_some() || nested == 0,
        Stmt::Block(block) => block.stmts.iter().any(|stmt| ast_loop_body_may_break(stmt, nested)),
        Stmt::If(branch) => ast_loop_body_may_break(&branch.cons, nested)
            || branch.alt.as_ref().is_some_and(|alt| ast_loop_body_may_break(alt, nested)),
        Stmt::Switch(switch) => switch.cases.iter().any(|case| case.cons.iter()
            .any(|stmt| ast_loop_body_may_break(stmt, nested + 1))),
        Stmt::While(loop_stmt) => ast_loop_body_may_break(&loop_stmt.body, nested + 1),
        Stmt::DoWhile(loop_stmt) => ast_loop_body_may_break(&loop_stmt.body, nested + 1),
        Stmt::For(loop_stmt) => ast_loop_body_may_break(&loop_stmt.body, nested + 1),
        Stmt::ForIn(loop_stmt) => ast_loop_body_may_break(&loop_stmt.body, nested + 1),
        Stmt::ForOf(loop_stmt) => ast_loop_body_may_break(&loop_stmt.body, nested + 1),
        Stmt::Try(try_stmt) => try_stmt.block.stmts.iter().any(|stmt| ast_loop_body_may_break(stmt, nested))
            || try_stmt.handler.as_ref().is_some_and(|handler| handler.body.stmts.iter()
                .any(|stmt| ast_loop_body_may_break(stmt, nested)))
            || try_stmt.finalizer.as_ref().is_some_and(|finalizer| finalizer.stmts.iter()
                .any(|stmt| ast_loop_body_may_break(stmt, nested))),
        Stmt::Labeled(labeled) => ast_loop_body_may_break(&labeled.body, nested),
        _ => false,
    }
}

/// True if `statement` always transfers control away (a `return`/`throw` on
/// every path) -- a conservative AST-level approximation, enough to decide
/// whether a function body can fall off its end.
fn ast_statement_terminates(statement: &swc_ecma_ast::Stmt) -> bool {
    use swc_ecma_ast::*;
    match statement {
        Stmt::Return(_) | Stmt::Throw(_) => true,
        Stmt::Block(block) => ast_block_terminates(&block.stmts),
        Stmt::If(if_statement) => {
            if_statement.alt.is_some()
                && ast_statement_terminates(&if_statement.cons)
                && ast_statement_terminates(if_statement.alt.as_ref().unwrap())
        }
        Stmt::Switch(switch) => {
            switch.cases.iter().any(|case| case.test.is_none())
                && switch
                    .cases
                    .iter()
                    .all(|case| ast_block_terminates(&case.cons))
        }
        Stmt::Try(try_statement) => {
            try_statement.finalizer.as_ref().is_some_and(|finalizer|
                ast_block_terminates(&finalizer.stmts))
                || (ast_block_terminates(&try_statement.block.stmts)
                    && try_statement.handler.as_ref().is_none_or(|handler|
                        ast_block_terminates(&handler.body.stmts)))
        }
        Stmt::While(while_statement) => {
            matches!(while_statement.test.as_ref(), Expr::Lit(Lit::Bool(b)) if b.value)
                && !ast_loop_body_may_break(&while_statement.body, 0)
        }
        Stmt::For(for_statement) => {
            for_statement.test.as_ref().is_none_or(|test|
                matches!(test.as_ref(), Expr::Lit(Lit::Bool(b)) if b.value))
                && !ast_loop_body_may_break(&for_statement.body, 0)
        }
        Stmt::DoWhile(do_while) => ast_statement_terminates(&do_while.body),
        _ => false,
    }
}

fn ast_block_terminates(statements: &[swc_ecma_ast::Stmt]) -> bool {
    for statement in statements {
        // A break before an apparent later return reaches the enclosing
        // switch/loop instead; that later return cannot prove termination.
        if ast_loop_body_may_break(statement, 0) {
            return false;
        }
        if ast_statement_terminates(statement) {
            return true;
        }
    }
    false
}

/// True if the block contains a `return <value>` somewhere (so the function
/// is value-returning, not `void`).
fn ast_block_has_value_return(statements: &[swc_ecma_ast::Stmt]) -> bool {
    use swc_ecma_ast::*;
    statements.iter().any(|statement| match statement {
        Stmt::Return(ret) => ret.arg.is_some(),
        Stmt::Block(block) => ast_block_has_value_return(&block.stmts),
        Stmt::If(if_statement) => {
            ast_block_has_value_return(std::slice::from_ref(&if_statement.cons))
                || if_statement
                    .alt
                    .as_ref()
                    .is_some_and(|alt| ast_block_has_value_return(std::slice::from_ref(alt)))
        }
        Stmt::Switch(switch) => switch
            .cases
            .iter()
            .any(|case| ast_block_has_value_return(&case.cons)),
        Stmt::Try(try_statement) => {
            ast_block_has_value_return(&try_statement.block.stmts)
                || try_statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| ast_block_has_value_return(&handler.body.stmts))
                || try_statement.finalizer.as_ref().is_some_and(|finalizer|
                    ast_block_has_value_return(&finalizer.stmts))
        }
        Stmt::While(while_statement) => ast_block_has_value_return(std::slice::from_ref(&*while_statement.body)),
        Stmt::DoWhile(do_while) => ast_block_has_value_return(std::slice::from_ref(&*do_while.body)),
        Stmt::For(for_statement) => ast_block_has_value_return(std::slice::from_ref(&*for_statement.body)),
        Stmt::ForIn(for_in) => ast_block_has_value_return(std::slice::from_ref(&*for_in.body)),
        Stmt::ForOf(for_of) => ast_block_has_value_return(std::slice::from_ref(&*for_of.body)),
        Stmt::Labeled(labeled) => ast_block_has_value_return(std::slice::from_ref(&labeled.body)),
        _ => false,
    })
}

/// Gives a module-level `var x;`/`let x;` (no initializer) an
/// `: any = undefined` type and value, so it can be declared and assigned
/// later (real trigger: test262's `testTypedArray.js` `var makeIterable;`).
/// A `const` without an initializer stays an error.
pub fn normalize_uninitialized_globals(module: &Module) -> Module {
    use swc_ecma_ast::*;
    let mut normalized = module.clone();
    for item in &mut normalized.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) = item else {
            continue;
        };
        if var.kind == VarDeclKind::Const {
            continue;
        }
        for declarator in &mut var.decls {
            if declarator.init.is_some() {
                continue;
            }
            let Pat::Ident(binding) = &mut declarator.name else {
                continue;
            };
            if binding.type_ann.is_none() {
                binding.type_ann = Some(Box::new(TsTypeAnn {
                    span: swc_common::DUMMY_SP,
                    type_ann: Box::new(TsType::TsKeywordType(TsKeywordType {
                        span: swc_common::DUMMY_SP,
                        kind: TsKeywordTypeKind::TsAnyKeyword,
                    })),
                }));
            }
            declarator.init = Some(Box::new(Expr::Ident(Ident::new_no_ctxt(
                "undefined".into(),
                swc_common::DUMMY_SP,
            ))));
        }
    }
    normalized
}

/// True if `function`'s body (including nested arrows, which inherit it, but
/// not nested non-arrow functions, which have their own) references
/// `arguments`.
fn function_references_arguments(function: &swc_ecma_ast::Function) -> bool {
    struct Finder(bool);
    impl swc_ecma_visit::Visit for Finder {
        fn visit_expr(&mut self, expr: &Expr) {
            if matches!(expr, Expr::Ident(ident) if ident.sym == *"arguments") {
                self.0 = true;
            }
            expr.visit_children_with(self);
        }
        fn visit_function(&mut self, _function: &swc_ecma_ast::Function) {}
    }
    let mut finder = Finder(false);
    if let Some(body) = &function.body {
        body.visit_with(&mut finder);
    }
    finder.0
}

/// `const arguments: any[] = [p1, p2, ...];` for a function whose body uses
/// `arguments` -- an approximation binding it to the declared parameters (JS
/// `arguments` also holds extra args, which thaw's fixed-arity functions
/// cannot receive). `None` when a parameter is itself named `arguments` or
/// isn't a simple identifier.
fn arguments_binding(function: &swc_ecma_ast::Function) -> Option<Stmt> {
    use swc_ecma_ast::*;
    let mut elements = Vec::new();
    for parameter in &function.params {
        let Pat::Ident(binding) = &parameter.pat else {
            return None;
        };
        if binding.id.sym == *"arguments" {
            return None;
        }
        elements.push(Some(ExprOrSpread {
            spread: None,
            expr: Box::new(Expr::Ident(binding.id.clone())),
        }));
    }
    let any = || TsType::TsKeywordType(TsKeywordType {
        span: swc_common::DUMMY_SP,
        kind: TsKeywordTypeKind::TsAnyKeyword,
    });
    Some(Stmt::Decl(Decl::Var(Box::new(VarDecl {
        span: swc_common::DUMMY_SP,
        ctxt: Default::default(),
        kind: VarDeclKind::Const,
        declare: false,
        decls: vec![VarDeclarator {
            span: swc_common::DUMMY_SP,
            name: Pat::Ident(BindingIdent {
                id: Ident::new_no_ctxt("arguments".into(), swc_common::DUMMY_SP),
                type_ann: Some(Box::new(TsTypeAnn {
                    span: swc_common::DUMMY_SP,
                    type_ann: Box::new(TsType::TsArrayType(TsArrayType {
                        span: swc_common::DUMMY_SP,
                        elem_type: Box::new(any()),
                    })),
                })),
            }),
            init: Some(Box::new(Expr::Array(ArrayLit {
                span: swc_common::DUMMY_SP,
                elems: elements,
            }))),
            definite: false,
        }],
    }))))
}

/// Normalize inferred value-returning callable bodies before signatures are
/// inferred. A bare return and a path that reaches the end both yield
/// `undefined`; generators use a separate completion channel.
fn normalize_inferred_return_body(statements: &mut Vec<Stmt>) {
    use swc_ecma_ast::*;
    if !ast_block_has_value_return(statements) {
        return;
    }
    fn undefined_expr() -> Box<Expr> {
        // The inserted value must not resolve a user binding named `undefined`.
        // This synthetic void-zero is recognized as an Undefined value during
        // lowering; ordinary unary-void expressions keep their existing ABI.
        Box::new(Expr::Unary(UnaryExpr {
            span: swc_common::DUMMY_SP,
            op: UnaryOp::Void,
            arg: Box::new(Expr::Lit(Lit::Num(Number {
                span: swc_common::DUMMY_SP,
                value: 0.0,
                raw: None,
            }))),
        }))
    }
    struct BareReturns;
    impl swc_ecma_visit::VisitMut for BareReturns {
        fn visit_mut_function(&mut self, _: &mut Function) {}
        fn visit_mut_arrow_expr(&mut self, _: &mut ArrowExpr) {}
        fn visit_mut_return_stmt(&mut self, ret: &mut ReturnStmt) {
            if ret.arg.is_none() {
                ret.arg = Some(undefined_expr());
            }
        }
    }
    statements.visit_mut_with(&mut BareReturns);
    if !ast_block_terminates(statements) {
        statements.push(Stmt::Return(ReturnStmt {
            span: swc_common::DUMMY_SP,
            arg: Some(undefined_expr()),
        }));
    }
}

pub fn normalize_implicit_returns(module: &Module) -> Module {
    use swc_ecma_ast::*;
    struct Rewriter;
    impl swc_ecma_visit::VisitMut for Rewriter {
        fn visit_mut_function(&mut self, function: &mut Function) {
            function.visit_mut_children_with(self);
            if function_references_arguments(function) {
                if let Some(binding) = arguments_binding(function) {
                    if let Some(body) = &mut function.body {
                        body.stmts.insert(0, binding);
                    }
                }
            }
            if function.return_type.is_none() && !function.is_generator {
                if let Some(body) = &mut function.body {
                    normalize_inferred_return_body(&mut body.stmts);
                }
            }
        }

        fn visit_mut_arrow_expr(&mut self, arrow: &mut ArrowExpr) {
            arrow.visit_mut_children_with(self);
            if arrow.return_type.is_none() && !arrow.is_generator {
                if let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_mut() {
                    normalize_inferred_return_body(&mut body.stmts);
                }
            }
        }
    }
    let mut module = module.clone();
    module.visit_mut_with(&mut Rewriter);
    module
}

/// `(owner, object)` for a whole-prototype assignment `F.prototype = <expr>`.
fn constructor_prototype_replacement(expr: &Expr) -> Option<(Symbol, &Expr)> {
    let Expr::Assign(assign) = expr else {
        return None;
    };
    if assign.op != swc_ecma_ast::AssignOp::Assign {
        return None;
    }
    let swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Member(member)) =
        &assign.left
    else {
        return None;
    };
    let Expr::Ident(owner) = member.obj.as_ref() else {
        return None;
    };
    if !matches!(&member.prop, swc_ecma_ast::MemberProp::Ident(prop) if prop.sym == *"prototype") {
        return None;
    }
    Some((owner.sym.to_string(), assign.right.as_ref()))
}

/// `(name, function)` for a single-declarator `var F = function () {...}`
/// binding, else `None`.
fn var_decl_function(var: &swc_ecma_ast::VarDecl) -> Option<(Symbol, &swc_ecma_ast::Function)> {
    let [declarator] = var.decls.as_slice() else {
        return None;
    };
    let swc_ecma_ast::Pat::Ident(binding) = &declarator.name else {
        return None;
    };
    let swc_ecma_ast::Expr::Fn(function) = declarator.init.as_deref()? else {
        return None;
    };
    Some((binding.id.sym.to_string(), &function.function))
}

/// Rewrites the classic constructor-function pattern into a class, so it
/// reuses thaw's native class machinery (`this`, `new F()`,
/// `instanceof F`, static dispatch) instead of failing on `this` in a plain
/// function. Real trigger: test262's own harness (`Test262Error`).
///
/// Only *module-level* function declarations whose body uses `this` are
/// converted; their `F.prototype.m = ...` / `F.s = ...` assignments (wherever
/// they appear in the module) become instance methods / static members.
pub fn normalize_constructor_functions(module: &Module) -> Result<Module, String> {
    use swc_ecma_ast::*;

    // A function explicitly declared with a `this` parameter, or used via
    // `.call`/`.apply`/`.bind`, is a call-with-receiver function, not a
    // constructor -- leave it alone.
    let rebound: std::collections::HashSet<Symbol> = {
        struct Finder(std::collections::HashSet<Symbol>);
        impl swc_ecma_visit::Visit for Finder {
            fn visit_member_expr(&mut self, member: &swc_ecma_ast::MemberExpr) {
                if let (Expr::Ident(object), swc_ecma_ast::MemberProp::Ident(property)) =
                    (member.obj.as_ref(), &member.prop)
                {
                    if matches!(property.sym.as_ref(), "call" | "apply" | "bind") {
                        self.0.insert(object.sym.to_string());
                    }
                }
                member.visit_children_with(self);
            }
        }
        let mut finder = Finder(std::collections::HashSet::new());
        module.visit_with(&mut finder);
        finder.0
    };

    // Collect every top-level `F.prototype.m = ...` / `F.s = ...` first, so a
    // function can be recognized as a namespace (`assert`) even without
    // `this`.
    let mut assignments: HashMap<Symbol, Vec<(usize, bool, Symbol, &Expr)>> = HashMap::new();
    let mut prototype_replacements: HashMap<Symbol, (usize, &Expr)> = HashMap::new();
    for (index, item) in module.body.iter().enumerate() {
        let ModuleItem::Stmt(Stmt::Expr(ExprStmt { expr, .. })) = item else {
            continue;
        };
        if let Some((owner, on_prototype, name, right)) = constructor_assignment_target(expr) {
            assignments
                .entry(owner)
                .or_default()
                .push((index, on_prototype, name, right));
        } else if let Some((owner, object)) = constructor_prototype_replacement(expr) {
            prototype_replacements.insert(owner, (index, object));
        }
    }

    let eligible = |function: &Function, name: &str| {
        !function.is_async
            && !function.is_generator
            && function.body.is_some()
            && function.this_param.is_none()
            && !rebound.contains(name)
    };
    let mut candidates: HashMap<Symbol, bool> = HashMap::new();
    for item in &module.body {
        let (name, function) = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) => {
                (function.ident.sym.to_string(), &*function.function)
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) => match var_decl_function(var) {
                Some(found) => found,
                None => continue,
            },
            _ => continue,
        };
        if !eligible(function, &name) {
            continue;
        }
        let uses_this = constructor_function_uses_this(function);
        // A whole-prototype assignment (`Con.prototype = {...}`) also marks a
        // constructor, even without `this`.
        let is_constructor = uses_this || prototype_replacements.contains_key(&name);
        if is_constructor || assignments.contains_key(&name) {
            candidates.insert(name, is_constructor);
        }
    }
    if candidates.is_empty() {
        return Ok(module.clone());
    }

    let functions: HashMap<Symbol, &Function> = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) => {
                Some((function.ident.sym.to_string(), &*function.function))
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) => var_decl_function(var),
            _ => None,
        })
        .collect();

    let mut namespaces: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    let mut class_for: HashMap<usize, ClassDecl> = HashMap::new();
    let mut consumed: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (index, item) in module.body.iter().enumerate() {
        let (name, ident, function) = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) => (
                function.ident.sym.to_string(),
                function.ident.clone(),
                (*function.function).clone(),
            ),
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) => {
                let Some((name, function)) = var_decl_function(var) else {
                    continue;
                };
                let Pat::Ident(binding) = &var.decls[0].name else {
                    continue;
                };
                (name, binding.id.clone(), function.clone())
            }
            _ => continue,
        };
        let Some(is_constructor) = candidates.get(&name) else {
            continue;
        };
        let list = assignments.remove(&name).unwrap_or_default();
        for (item_index, ..) in &list {
            consumed.insert(*item_index);
        }
        let replacement_object = match prototype_replacements.remove(&name) {
            Some((replacement_index, object)) => {
                consumed.insert(replacement_index);
                Some(object)
            }
            None => None,
        };
        let refs: Vec<(bool, Symbol, &Expr)> = list
            .iter()
            .map(|(_, on_prototype, member, right)| (*on_prototype, member.clone(), *right))
            .collect();
        if !*is_constructor {
            namespaces.insert(name.clone());
        }
        let declaration = FnDecl {
            ident,
            declare: false,
            function: Box::new(function),
        };
        class_for.insert(
            index,
            constructor_class(
                &declaration,
                &refs,
                *is_constructor,
                replacement_object,
                &functions,
            )?,
        );
    }

    let mut body = Vec::new();
    for (index, item) in module.body.iter().enumerate() {
        if let Some(class) = class_for.remove(&index) {
            body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))));
            continue;
        }
        if consumed.contains(&index) {
            continue;
        }
        body.push(item.clone());
    }
    let mut normalized = module.clone();
    normalized.body = body;

    // A namespace function is now a class, so rewrite its plain calls
    // (`assert(x)`) to the static `assert.__call__(x)`.
    if !namespaces.is_empty() {
        struct CallRewriter<'a> {
            names: &'a std::collections::HashSet<Symbol>,
        }
        impl swc_ecma_visit::VisitMut for CallRewriter<'_> {
            fn visit_mut_call_expr(&mut self, call: &mut CallExpr) {
                if let Callee::Expr(callee) = &call.callee {
                    if let Expr::Ident(ident) = callee.as_ref() {
                        if self.names.contains(ident.sym.as_ref()) {
                            call.callee = Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                span: swc_common::DUMMY_SP,
                                obj: Box::new(Expr::Ident(ident.clone())),
                                prop: MemberProp::Ident(IdentName::new(
                                    "__call__".into(),
                                    swc_common::DUMMY_SP,
                                )),
                            })));
                        }
                    }
                }
                call.visit_mut_children_with(self);
            }
        }
        normalized
            .body
            .visit_mut_with(&mut CallRewriter { names: &namespaces });
    }
    Ok(normalized)
}
