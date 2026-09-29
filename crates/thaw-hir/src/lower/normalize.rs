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
) -> Result<swc_ecma_ast::ClassDecl, String> {
    use swc_ecma_ast::*;

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
    body.push(ClassMember::Constructor(Constructor {
        span: swc_common::DUMMY_SP,
        ctxt: Default::default(),
        key: property(&"constructor".to_string()),
        params: function
            .function
            .params
            .iter()
            .cloned()
            .map(ParamOrTsParamProp::Param)
            .collect(),
        body: function.function.body.clone(),
        accessibility: None,
        is_optional: false,
    }));
    for (name, right) in &instance_methods {
        let Some(method_function) = rhs_function(right) else {
            return Err(format!(
                "constructor-function prototype member `{name}` is not a function"
            ));
        };
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
        if let Some(function) = rhs_function(right) {
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
                type_ann: inferred_field_type(right, &param_types).map(|ty| {
                    Box::new(TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(ty),
                    })
                }),
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

    let mut candidates: HashMap<Symbol, &FnDecl> = HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item {
            let name = function.ident.sym.to_string();
            if !function.function.is_async
                && !function.function.is_generator
                && function.function.body.is_some()
                && function.function.this_param.is_none()
                && !rebound.contains(&name)
                && constructor_function_uses_this(&function.function)
            {
                candidates.insert(name, function);
            }
        }
    }
    if candidates.is_empty() {
        return Ok(module.clone());
    }

    let mut assignments: HashMap<Symbol, Vec<(usize, bool, Symbol, &Expr)>> = HashMap::new();
    for (index, item) in module.body.iter().enumerate() {
        let ModuleItem::Stmt(Stmt::Expr(ExprStmt { expr, .. })) = item else {
            continue;
        };
        if let Some((owner, on_prototype, name, right)) = constructor_assignment_target(expr) {
            if candidates.contains_key(&owner) {
                assignments
                    .entry(owner)
                    .or_default()
                    .push((index, on_prototype, name, right));
            }
        }
    }

    let mut class_for: HashMap<usize, ClassDecl> = HashMap::new();
    let mut consumed: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (index, item) in module.body.iter().enumerate() {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
            continue;
        };
        if !candidates.contains_key(function.ident.sym.as_ref()) {
            continue;
        }
        let list = assignments
            .remove(function.ident.sym.as_ref())
            .unwrap_or_default();
        for (item_index, ..) in &list {
            consumed.insert(*item_index);
        }
        let refs: Vec<(bool, Symbol, &Expr)> = list
            .iter()
            .map(|(_, on_prototype, name, right)| (*on_prototype, name.clone(), *right))
            .collect();
        class_for.insert(index, constructor_class(function, &refs)?);
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
    Ok(normalized)
}
