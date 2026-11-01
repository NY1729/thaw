type EnumValues = HashMap<(Symbol, Symbol), HirLit>;
type EnumReverseValues = HashMap<Symbol, Vec<(f64, Symbol)>>;

fn enum_to_uint32(value: f64) -> u32 {
    if !value.is_finite() || value == 0.0 {
        0
    } else {
        value.trunc().rem_euclid(4_294_967_296.0) as u32
    }
}

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
                UnaryOp::Tilde => Ok(HirLit::F64((!(enum_to_uint32(value) as i32)) as f64)),
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
                BinaryOp::BitOr => ((enum_to_uint32(left) as i32) | (enum_to_uint32(right) as i32)) as f64,
                BinaryOp::BitXor => ((enum_to_uint32(left) as i32) ^ (enum_to_uint32(right) as i32)) as f64,
                BinaryOp::BitAnd => ((enum_to_uint32(left) as i32) & (enum_to_uint32(right) as i32)) as f64,
                BinaryOp::LShift => ((enum_to_uint32(left) as i32) << (enum_to_uint32(right) & 31)) as f64,
                BinaryOp::RShift => ((enum_to_uint32(left) as i32) >> (enum_to_uint32(right) & 31)) as f64,
                BinaryOp::ZeroFillRShift => (enum_to_uint32(left) >> (enum_to_uint32(right) & 31)) as f64,
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
        // `this.x` (read or assignment target) is the constructor shape; a bare `this`
        // (`typeof this`, `() => this`) does not make a function a constructor.
        fn visit_member_expr(&mut self, member: &swc_ecma_ast::MemberExpr) {
            if matches!(member.obj.as_ref(), Expr::This(_)) {
                self.0 = true;
            }
            member.visit_children_with(self);
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
    let mut declared_static_fields = std::collections::HashSet::new();
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
        } else if declared_static_fields.insert(name.clone()) {
            // Keep each assignment at its original module position.  The
            // declaration only reserves storage; it must not evaluate RHS.
            let values: Vec<&Expr> = statics.iter()
                .filter(|(member, value)| member == name && resolve_function(value).is_none())
                .map(|(_, value)| *value)
                .collect();
            let inferred: Vec<TsType> = values.iter()
                .map(|value| inferred_field_type(value, &param_types).unwrap_or_else(ts_any_type))
                .collect();
            let same_keyword = |kind: TsKeywordTypeKind| inferred.iter().all(|ty|
                matches!(ty, TsType::TsKeywordType(keyword) if keyword.kind == kind));
            let ty = if inferred.len() == 1
                || same_keyword(TsKeywordTypeKind::TsNumberKeyword)
                || same_keyword(TsKeywordTypeKind::TsStringKeyword)
                || same_keyword(TsKeywordTypeKind::TsBooleanKeyword) {
                inferred.into_iter().next().unwrap()
            } else {
                // Distinct inferred value kinds need one shared writable
                // slot, rather than duplicate class members.
                ts_any_type()
            };
            body.push(ClassMember::ClassProp(ClassProp {
                span: swc_common::DUMMY_SP,
                key: property(name),
                value: None,
                type_ann: Some(Box::new(TsTypeAnn {
                    span: swc_common::DUMMY_SP,
                    type_ann: Box::new(ty),
                })),
                is_static: true,
                decorators: Vec::new(),
                accessibility: None,
                is_abstract: false,
                is_optional: true,
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
    ast_break_escapes(statement, nested, None)
}

/// `own` is a label whose `break` stays inside the statement being asked about.
fn ast_break_escapes(statement: &swc_ecma_ast::Stmt, nested: usize, own: Option<&str>) -> bool {
    use swc_ecma_ast::Stmt;
    match statement {
        Stmt::Break(value) => match &value.label {
            Some(label) => own != Some(label.sym.as_ref()),
            None => nested == 0,
        },
        Stmt::Block(block) => block.stmts.iter().any(|stmt| ast_break_escapes(stmt, nested, own)),
        Stmt::If(branch) => ast_break_escapes(&branch.cons, nested, own)
            || branch.alt.as_ref().is_some_and(|alt| ast_break_escapes(alt, nested, own)),
        Stmt::Switch(switch) => switch.cases.iter().any(|case| case.cons.iter()
            .any(|stmt| ast_break_escapes(stmt, nested + 1, own))),
        Stmt::While(loop_stmt) => ast_break_escapes(&loop_stmt.body, nested + 1, own),
        Stmt::DoWhile(loop_stmt) => ast_break_escapes(&loop_stmt.body, nested + 1, own),
        Stmt::For(loop_stmt) => ast_break_escapes(&loop_stmt.body, nested + 1, own),
        Stmt::ForIn(loop_stmt) => ast_break_escapes(&loop_stmt.body, nested + 1, own),
        Stmt::ForOf(loop_stmt) => ast_break_escapes(&loop_stmt.body, nested + 1, own),
        Stmt::Try(try_stmt) => try_stmt.block.stmts.iter().any(|stmt| ast_break_escapes(stmt, nested, own))
            || try_stmt.handler.as_ref().is_some_and(|handler| handler.body.stmts.iter()
                .any(|stmt| ast_break_escapes(stmt, nested, own)))
            || try_stmt.finalizer.as_ref().is_some_and(|finalizer| finalizer.stmts.iter()
                .any(|stmt| ast_break_escapes(stmt, nested, own))),
        Stmt::Labeled(labeled) => ast_break_escapes(&labeled.body, nested, own),
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
        let own = match statement {
            swc_ecma_ast::Stmt::Labeled(labeled) => Some(labeled.label.sym.as_ref()),
            _ => None,
        };
        if ast_break_escapes(statement, 0, own) {
            return false;
        }
        if ast_statement_terminates(statement) {
            return true;
        }
    }
    false
}

/// True if a callback written inline (arrow or function expression with a block
/// body) ends in `throw` on every path and never returns a value, so its result
/// type may be taken from the surrounding Promise chain instead of `void`.
fn ast_callback_always_throws(expr: &swc_ecma_ast::Expr) -> bool {
    use swc_ecma_ast::*;
    let stmts = match expr {
        Expr::Arrow(arrow) => match arrow.body.as_ref() {
            ArrowFunctionBody::FunctionBody(block) => &block.stmts,
            _ => return false,
        },
        Expr::Fn(function) => match &function.function.body {
            Some(block) => &block.stmts,
            None => return false,
        },
        _ => return false,
    };
    ast_block_terminates(stmts) && !ast_block_has_value_return(stmts)
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
            // `let n: number;` starts from the type's zero value; the
            // runtime can't hold `undefined` in a native scalar.
            let kind = binding.type_ann.as_ref().and_then(|a| match a.type_ann.as_ref() {
                TsType::TsKeywordType(k) => Some(k.kind),
                _ => None,
            });
            let sp = swc_common::DUMMY_SP;
            // `let m: T | null;` (no `undefined` member) starts as `null`.
            let nullable_only = binding.type_ann.as_ref().is_some_and(|a| match a.type_ann.as_ref() {
                TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
                    let has = |wanted: TsKeywordTypeKind| union.types.iter().any(|member| {
                        matches!(member.as_ref(), TsType::TsKeywordType(k) if k.kind == wanted)
                    });
                    has(TsKeywordTypeKind::TsNullKeyword)
                        && !has(TsKeywordTypeKind::TsUndefinedKeyword)
                        && !has(TsKeywordTypeKind::TsAnyKeyword)
                        && !has(TsKeywordTypeKind::TsUnknownKeyword)
                }
                _ => false,
            });
            if nullable_only {
                declarator.init = Some(Box::new(Expr::Lit(Lit::Null(Null { span: sp }))));
                continue;
            }
            declarator.init = Some(Box::new(match kind {
                Some(TsKeywordTypeKind::TsNumberKeyword) => {
                    Expr::Lit(Lit::Num(Number { span: sp, value: 0.0, raw: None }))
                }
                Some(TsKeywordTypeKind::TsStringKeyword) => {
                    Expr::Lit(Lit::Str(Str { span: sp, value: "".into(), raw: None }))
                }
                Some(TsKeywordTypeKind::TsBooleanKeyword) => {
                    Expr::Lit(Lit::Bool(Bool { span: sp, value: false }))
                }
                _ => Expr::Ident(Ident::new_no_ctxt("undefined".into(), sp)),
            }));
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

/// Gives a function that uses `arguments` a trailing `...__thaw_arguments_rest: any[]`
/// parameter (when it has no rest of its own) so surplus call arguments are received.
fn add_arguments_rest(function: &mut swc_ecma_ast::Function) {
    use swc_ecma_ast::*;
    if function.params.iter().any(|p| matches!(p.pat, Pat::Rest(_))) {
        return;
    }
    let sp = swc_common::DUMMY_SP;
    function.params.push(Param {
        span: sp,
        decorators: vec![],
        pat: Pat::Rest(RestPat {
            span: sp,
            dot3_token: sp,
            arg: Box::new(Pat::Ident(BindingIdent {
                id: Ident::new_no_ctxt("__thaw_arguments_rest".into(), sp),
                type_ann: None,
            })),
            type_ann: Some(Box::new(TsTypeAnn {
                span: sp,
                type_ann: Box::new(TsType::TsArrayType(TsArrayType {
                    span: sp,
                    elem_type: Box::new(TsType::TsKeywordType(TsKeywordType {
                        span: sp,
                        kind: TsKeywordTypeKind::TsAnyKeyword,
                    })),
                })),
            })),
        }),
    });
}

/// `const arguments: any[] = [p1, p2, ...];` for a function whose body uses
/// `arguments`, bound to the declared parameters plus (once
/// `add_arguments_rest` runs) the surplus arguments. `None` when a parameter
/// is itself named `arguments` or isn't a simple identifier.
fn arguments_binding(function: &swc_ecma_ast::Function) -> Option<Vec<Stmt>> {
    use swc_ecma_ast::*;
    // A function's own trailing rest parameter (`...r`) holds the surplus arguments, so it
    // is spread in place of the synthesized `__thaw_arguments_rest`.
    let (fixed, own_rest) = match function.params.split_last() {
        Some((last, leading)) if matches!(last.pat, Pat::Rest(_)) => (leading, Some(&last.pat)),
        _ => (&function.params[..], None),
    };
    let rest_name = match own_rest {
        Some(Pat::Rest(RestPat { arg, .. })) => {
            let Pat::Ident(binding) = arg.as_ref() else {
                return None;
            };
            if binding.id.sym == *"arguments" {
                return None;
            }
            binding.id.sym.to_string()
        }
        _ => "__thaw_arguments_rest".to_string(),
    };
    let mut elements = Vec::new();
    for parameter in fixed {
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
    elements.push(Some(ExprOrSpread {
        spread: Some(swc_common::DUMMY_SP),
        expr: Box::new(Expr::Ident(Ident::new_no_ctxt(rest_name.as_str().into(), swc_common::DUMMY_SP))),
    }));
    let any = || TsType::TsKeywordType(TsKeywordType {
        span: swc_common::DUMMY_SP,
        kind: TsKeywordTypeKind::TsAnyKeyword,
    });
    let declaration = Stmt::Decl(Decl::Var(Box::new(VarDecl {
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
    })));
    // Call sites do not pass an argument count, so omitted trailing arguments
    // (which arrive as `undefined`) are trimmed when no surplus was passed:
    // `while (rest.length === 0 && arguments.length > 0 && arguments[arguments.length - 1] === undefined) arguments.pop()`.
    // ponytail: an explicit trailing `undefined` argument is also dropped; exact counts need a call-site arity channel.
    let sp = swc_common::DUMMY_SP;
    let ident = |name: &str| Box::new(Expr::Ident(Ident::new_no_ctxt(name.into(), sp)));
    let member = |obj: Box<Expr>, prop: &str| {
        Box::new(Expr::Member(MemberExpr {
            span: sp,
            obj,
            prop: MemberProp::Ident(IdentName::new(prop.into(), sp)),
        }))
    };
    let number = |value: f64| Box::new(Expr::Lit(Lit::Num(Number { span: sp, value, raw: None })));
    let bin = |op: BinaryOp, left: Box<Expr>, right: Box<Expr>| {
        Box::new(Expr::Bin(BinExpr { span: sp, op, left, right }))
    };
    let length = || member(ident("arguments"), "length");
    let last = Box::new(Expr::Member(MemberExpr {
        span: sp,
        obj: ident("arguments"),
        prop: MemberProp::Computed(ComputedPropName {
            span: sp,
            expr: bin(BinaryOp::Sub, length(), number(1.0)),
        }),
    }));
    let test = bin(
        BinaryOp::LogicalAnd,
        bin(
            BinaryOp::LogicalAnd,
            bin(BinaryOp::EqEqEq, member(ident(&rest_name), "length"), number(0.0)),
            bin(BinaryOp::Gt, length(), number(0.0)),
        ),
        bin(BinaryOp::EqEqEq, last, ident("undefined")),
    );
    let pop = Stmt::Expr(ExprStmt {
        span: sp,
        expr: Box::new(Expr::Call(CallExpr {
            span: sp,
            ctxt: Default::default(),
            callee: Callee::Expr(member(ident("arguments"), "pop")),
            args: vec![],
            type_args: None,
        })),
    });
    let trim = Stmt::While(WhileStmt { span: sp, test, body: Box::new(pop) });
    Some(vec![declaration, trim])
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

/// Whether any call in `statements` has the plain identifier `name` as its callee.
fn arrow_calls(statements: &[swc_ecma_ast::Stmt], name: &str) -> bool {
    use swc_ecma_visit::{Visit, VisitWith};
    struct Finder<'a> {
        name: &'a str,
        found: bool,
    }
    impl Visit for Finder<'_> {
        fn visit_call_expr(&mut self, call: &swc_ecma_ast::CallExpr) {
            if matches!(&call.callee, swc_ecma_ast::Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Ident(ident) if ident.sym == *self.name))
            {
                self.found = true;
            }
            call.visit_children_with(self);
        }
    }
    let mut finder = Finder { name, found: false };
    statements.visit_with(&mut finder);
    finder.found
}

pub fn normalize_implicit_returns(module: &Module) -> Module {
    use swc_ecma_ast::*;
    struct Rewriter;
    impl swc_ecma_visit::VisitMut for Rewriter {
        fn visit_mut_function(&mut self, function: &mut Function) {
            function.visit_mut_children_with(self);
            if function_references_arguments(function) {
                if let Some(binding) = arguments_binding(function) {
                    add_arguments_rest(function);
                    if let Some(body) = &mut function.body {
                        body.stmts.splice(0..0, binding);
                    }
                }
            }
            if function.return_type.is_none() && !function.is_generator {
                if let Some(body) = &mut function.body {
                    normalize_inferred_return_body(&mut body.stmts);
                }
            }
        }

        // `const visit = (n) => { ...; visit(next); }`: a self-recursive arrow that never returns
        // a value is `void`; its own call needs that signature before its body is typed.
        fn visit_mut_var_declarator(&mut self, declarator: &mut VarDeclarator) {
            declarator.visit_mut_children_with(self);
            let (Pat::Ident(name), Some(init)) = (&declarator.name, declarator.init.as_deref_mut()) else { return };
            let Expr::Arrow(arrow) = init else { return };
            if arrow.return_type.is_some() || arrow.is_async || arrow.is_generator {
                return;
            }
            let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() else { return };
            if ast_block_has_value_return(&body.stmts) || !arrow_calls(&body.stmts, name.id.sym.as_ref()) {
                return;
            }
            arrow.return_type = Some(Box::new(TsTypeAnn {
                span: swc_common::DUMMY_SP,
                type_ann: Box::new(TsType::TsKeywordType(TsKeywordType {
                    span: swc_common::DUMMY_SP,
                    kind: TsKeywordTypeKind::TsVoidKeyword,
                })),
            }));
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

/// `const p: any = asyncFn()`: a native Promise has no Json form, so an immutable `any` alias of
/// a direct call to a top-level async function drops the annotation and keeps the Promise type
/// (`await p` / `p.then(...)` then work like any other Promise binding).
pub fn normalize_any_async_call_aliases(module: &Module) -> Module {
    use swc_ecma_ast::*;
    struct Rewriter(HashSet<Symbol>);
    impl swc_ecma_visit::VisitMut for Rewriter {
        fn visit_mut_var_decl(&mut self, var: &mut VarDecl) {
            var.visit_mut_children_with(self);
            if var.kind != VarDeclKind::Const {
                return;
            }
            for declarator in &mut var.decls {
                let Pat::Ident(binding) = &mut declarator.name else { continue };
                let is_any = matches!(binding.type_ann.as_deref(), Some(TsTypeAnn { type_ann, .. })
                    if matches!(type_ann.as_ref(), TsType::TsKeywordType(keyword)
                        if keyword.kind == TsKeywordTypeKind::TsAnyKeyword));
                let calls_async = matches!(declarator.init.as_deref(), Some(Expr::Call(CallExpr {
                    callee: Callee::Expr(callee), ..
                })) if matches!(callee.as_ref(), Expr::Ident(id) if self.0.contains(&id.sym.to_string())));
                if is_any && calls_async {
                    binding.type_ann = None;
                }
            }
        }
    }
    let asyncs = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function)))
            | ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                decl: Decl::Fn(function), ..
            })) if function.function.is_async => Some(function.ident.sym.to_string()),
            _ => None,
        })
        .collect();
    let mut module = module.clone();
    module.visit_mut_with(&mut Rewriter(asyncs));
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
        let callable_statics: std::collections::HashSet<Symbol> = list.iter()
            .filter(|(_, on_prototype, _, right)| !*on_prototype &&
                (rhs_function(right).is_some() || matches!(right, Expr::Ident(ident)
                    if functions.contains_key(ident.sym.as_ref()))))
            .map(|(_, _, member, _)| member.clone())
            .collect();
        for (item_index, on_prototype, member, _) in &list {
            // Callable and prototype members still use the legacy class
            // method conversion.  Ordinary static writes stay in place.
            if *on_prototype || callable_statics.contains(member) {
                consumed.insert(*item_index);
            }
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

/// A default expression that creates a closure must be evaluated in the callee's
/// own scope: a closure built by the arity wrappers captures the wrappers' copy
/// of every parameter, not the cell the body reads and writes. Such defaults
/// (and the defaults after them) move into the body as
/// `let p = p__thaw_arg === undefined ? <default> : p__thaw_arg`, the parameter
/// itself becoming `p__thaw_arg?: T`. Needs an annotation or a literal default
/// to name `T`; any other shape keeps the wrapper evaluation.
pub fn normalize_default_parameter_closures(module: &Module) -> Module {
    use swc_ecma_ast::*;
    struct ClosureFinder(bool);
    impl swc_ecma_visit::Visit for ClosureFinder {
        fn visit_function(&mut self, _: &swc_ecma_ast::Function) {
            self.0 = true;
        }
        fn visit_arrow_expr(&mut self, _: &ArrowExpr) {
            self.0 = true;
        }
    }
    fn creates_closure(expr: &Expr) -> bool {
        let mut finder = ClosureFinder(false);
        expr.visit_with(&mut finder);
        finder.0
    }
    fn keyword(kind: TsKeywordTypeKind) -> Box<TsTypeAnn> {
        Box::new(TsTypeAnn {
            span: swc_common::DUMMY_SP,
            type_ann: Box::new(TsType::TsKeywordType(TsKeywordType {
                span: swc_common::DUMMY_SP,
                kind,
            })),
        })
    }
    fn parameter_type(binding: &BindingIdent, default: &Expr) -> Option<Box<TsTypeAnn>> {
        if let Some(annotation) = &binding.type_ann {
            return Some(annotation.clone());
        }
        match default {
            Expr::Lit(Lit::Num(_)) => Some(keyword(TsKeywordTypeKind::TsNumberKeyword)),
            Expr::Lit(Lit::Str(_)) => Some(keyword(TsKeywordTypeKind::TsStringKeyword)),
            Expr::Lit(Lit::Bool(_)) => Some(keyword(TsKeywordTypeKind::TsBooleanKeyword)),
            _ => None,
        }
    }
    /// The body statements that evaluate the moved defaults, or `None` (patterns untouched).
    fn move_defaults(patterns: &mut [&mut Pat]) -> Option<Vec<Stmt>> {
        let start = patterns.iter().position(
            |pattern| matches!(&**pattern, Pat::Assign(assign) if creates_closure(&assign.right)),
        )?;
        let mut moved = Vec::new();
        for pattern in &patterns[start..] {
            let Pat::Assign(assign) = &**pattern else {
                return None;
            };
            let Pat::Ident(binding) = assign.left.as_ref() else {
                return None;
            };
            let annotation = parameter_type(binding, &assign.right)?;
            moved.push((binding.id.clone(), annotation, assign.right.clone()));
        }
        let mut statements = Vec::new();
        for (pattern, (id, annotation, default)) in patterns[start..].iter_mut().zip(moved) {
            let supplied = Ident::new_no_ctxt(format!("{}__thaw_arg", id.sym).into(), swc_common::DUMMY_SP);
            **pattern = Pat::Ident(BindingIdent {
                id: Ident {
                    optional: true,
                    ..supplied.clone()
                },
                type_ann: Some(annotation.clone()),
            });
            let undefined = Box::new(Expr::Ident(Ident::new_no_ctxt(
                "undefined".into(),
                swc_common::DUMMY_SP,
            )));
            statements.push(Stmt::Decl(Decl::Var(Box::new(VarDecl {
                span: swc_common::DUMMY_SP,
                ctxt: Default::default(),
                kind: VarDeclKind::Let,
                declare: false,
                decls: vec![VarDeclarator {
                    span: swc_common::DUMMY_SP,
                    name: Pat::Ident(BindingIdent {
                        id: Ident::new_no_ctxt(id.sym.clone(), swc_common::DUMMY_SP),
                        type_ann: Some(annotation),
                    }),
                    init: Some(Box::new(Expr::Cond(CondExpr {
                        span: swc_common::DUMMY_SP,
                        test: Box::new(Expr::Bin(BinExpr {
                            span: swc_common::DUMMY_SP,
                            op: BinaryOp::EqEqEq,
                            left: Box::new(Expr::Ident(supplied.clone())),
                            right: undefined,
                        })),
                        cons: default,
                        alt: Box::new(Expr::Ident(supplied)),
                    }))),
                    definite: false,
                }],
            }))));
        }
        Some(statements)
    }
    struct Rewriter;
    impl swc_ecma_visit::VisitMut for Rewriter {
        fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
            function.visit_mut_children_with(self);
            if function_references_arguments(function) {
                return;
            }
            let Some(body) = &mut function.body else {
                return;
            };
            let mut patterns: Vec<&mut Pat> =
                function.params.iter_mut().map(|param| &mut param.pat).collect();
            if let Some(mut statements) = move_defaults(&mut patterns) {
                statements.append(&mut body.stmts);
                body.stmts = statements;
            }
        }

        fn visit_mut_constructor(&mut self, constructor: &mut Constructor) {
            constructor.visit_mut_children_with(self);
            let Some(body) = &mut constructor.body else {
                return;
            };
            let mut patterns = Vec::new();
            for parameter in &mut constructor.params {
                let ParamOrTsParamProp::Param(parameter) = parameter else {
                    return;
                };
                patterns.push(&mut parameter.pat);
            }
            if let Some(mut statements) = move_defaults(&mut patterns) {
                statements.append(&mut body.stmts);
                body.stmts = statements;
            }
        }

        fn visit_mut_arrow_expr(&mut self, arrow: &mut ArrowExpr) {
            arrow.visit_mut_children_with(self);
            let mut patterns: Vec<&mut Pat> = arrow.params.iter_mut().collect();
            let Some(mut statements) = move_defaults(&mut patterns) else {
                return;
            };
            match arrow.body.as_mut() {
                ArrowFunctionBody::FunctionBody(body) => {
                    statements.append(&mut body.stmts);
                    body.stmts = statements;
                }
                ArrowFunctionBody::Expr(expr) => {
                    statements.push(Stmt::Return(ReturnStmt {
                        span: swc_common::DUMMY_SP,
                        arg: Some(expr.clone()),
                    }));
                    *arrow.body = ArrowFunctionBody::FunctionBody(FunctionBody {
                        span: swc_common::DUMMY_SP,
                        stmts: statements,
                    });
                }
            }
        }
    }
    let mut module = module.clone();
    module.visit_mut_with(&mut Rewriter);
    module
}

/// A parameter with a default and no type annotation takes the default's
/// static type, as TypeScript infers it: literals, arithmetic on number
/// operands, string concatenation, and earlier keyword-typed parameters
/// (`function f(a: number, b = a * 2)`). Anything else is left unannotated,
/// so the existing annotation error still applies.
pub fn annotate_untyped_default_parameters(module: &Module) -> Module {
    use swc_ecma_ast::*;
    fn keyword_annotation(kind: TsKeywordTypeKind) -> Box<TsTypeAnn> {
        Box::new(TsTypeAnn {
            span: swc_common::DUMMY_SP,
            type_ann: Box::new(TsType::TsKeywordType(TsKeywordType {
                span: swc_common::DUMMY_SP,
                kind,
            })),
        })
    }
    fn keyword_of(annotation: &TsTypeAnn) -> Option<TsKeywordTypeKind> {
        match annotation.type_ann.as_ref() {
            TsType::TsKeywordType(keyword) => Some(keyword.kind),
            _ => None,
        }
    }
    fn expr_type(
        expr: &Expr,
        earlier: &HashMap<String, TsKeywordTypeKind>,
        fields: &HashMap<String, TsKeywordTypeKind>,
    ) -> Option<TsKeywordTypeKind> {
        use TsKeywordTypeKind::*;
        match expr {
            Expr::Paren(paren) => expr_type(&paren.expr, earlier, fields),
            Expr::Member(member) if matches!(member.obj.as_ref(), Expr::This(_)) => {
                match &member.prop {
                    MemberProp::Ident(name) => fields.get(&name.sym.to_string()).cloned(),
                    _ => None,
                }
            }
            Expr::Lit(Lit::Num(_)) => Some(TsNumberKeyword),
            Expr::Lit(Lit::Str(_)) => Some(TsStringKeyword),
            Expr::Lit(Lit::Bool(_)) => Some(TsBooleanKeyword),
            Expr::Ident(ident) => earlier.get(&ident.sym.to_string()).cloned(),
            Expr::Bin(bin) => {
                let left = expr_type(&bin.left, earlier, fields);
                let right = expr_type(&bin.right, earlier, fields);
                let both_numbers = left == Some(TsNumberKeyword) && right == Some(TsNumberKeyword);
                match bin.op {
                    BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Exp
                        if both_numbers =>
                    {
                        Some(TsNumberKeyword)
                    }
                    BinaryOp::Add if both_numbers => Some(TsNumberKeyword),
                    BinaryOp::Add
                        if left == Some(TsStringKeyword) || right == Some(TsStringKeyword) =>
                    {
                        Some(TsStringKeyword)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    fn annotate_params<'a>(params: impl Iterator<Item = &'a mut Pat>) {
        let mut earlier: HashMap<String, TsKeywordTypeKind> = HashMap::new();
        for pat in params {
            let binding = match pat {
                Pat::Ident(binding) => binding,
                Pat::Assign(assign) => match assign.left.as_mut() {
                    Pat::Ident(binding) => {
                        if binding.type_ann.is_none() {
                            if let Some(kind) = expr_type(&assign.right, &earlier, &HashMap::new()) {
                                binding.type_ann = Some(keyword_annotation(kind));
                            }
                        }
                        binding
                    }
                    _ => continue,
                },
                _ => continue,
            };
            if let Some(kind) = binding.type_ann.as_deref().and_then(keyword_of) {
                earlier.insert(binding.id.sym.to_string(), kind);
            }
        }
    }
    struct Annotator;
    impl swc_ecma_visit::VisitMut for Annotator {
        fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
            function.visit_mut_children_with(self);
            annotate_params(function.params.iter_mut().map(|param| &mut param.pat));
        }
        fn visit_mut_arrow_expr(&mut self, arrow: &mut ArrowExpr) {
            arrow.visit_mut_children_with(self);
            annotate_params(arrow.params.iter_mut());
        }
        fn visit_mut_class(&mut self, class: &mut Class) {
            class.visit_mut_children_with(self);
            // `handler = () => this.n`: a field without an annotation takes its arrow's
            // signature, with a return type from the arrow, its body, or `void`.
            let mut fields: HashMap<String, TsKeywordTypeKind> = HashMap::new();
            for member in &class.body {
                let ClassMember::ClassProp(prop) = member else { continue };
                let (PropName::Ident(name), false) = (&prop.key, prop.is_static) else { continue };
                let kind = match (&prop.type_ann, &prop.value) {
                    (Some(annotation), _) => keyword_of(annotation),
                    (None, Some(value)) => expr_type(value, &HashMap::new(), &fields),
                    _ => None,
                };
                if let Some(kind) = kind {
                    fields.insert(name.sym.to_string(), kind);
                }
            }
            for member in &mut class.body {
                let ClassMember::ClassProp(prop) = member else { continue };
                if prop.is_static || prop.type_ann.is_some() {
                    continue;
                }
                let Some(Expr::Arrow(arrow)) = prop.value.as_deref() else { continue };
                if arrow.is_async || arrow.is_generator || arrow.type_params.is_some() {
                    continue;
                }
                let mut params = Vec::new();
                let mut known: HashMap<String, TsKeywordTypeKind> = HashMap::new();
                for param in &arrow.params {
                    let Pat::Ident(binding) = param else { break };
                    let Some(annotation) = binding.type_ann.as_ref() else { break };
                    if let Some(kind) = keyword_of(annotation) {
                        known.insert(binding.id.sym.to_string(), kind);
                    }
                    params.push(TsFnParam::Ident(binding.clone()));
                }
                if params.len() != arrow.params.len() {
                    continue;
                }
                // The value of the last `return <expr>` in a block (not inside a closure).
                fn returned_value(statements: &[Stmt]) -> Option<&Expr> {
                    let mut found = None;
                    for statement in statements {
                        match statement {
                            Stmt::Return(ret) => found = ret.arg.as_deref().or(found),
                            Stmt::Block(block) => found = returned_value(&block.stmts).or(found),
                            Stmt::If(branch) => {
                                found = returned_value(std::slice::from_ref(&branch.cons)).or(found);
                                if let Some(alt) = branch.alt.as_deref() {
                                    found = returned_value(std::slice::from_ref(alt)).or(found);
                                }
                            }
                            _ => {}
                        }
                    }
                    found
                }
                let return_type = if let Some(annotation) = arrow.return_type.clone() {
                    annotation
                } else {
                    let kind = match arrow.body.as_ref() {
                        ArrowFunctionBody::Expr(body) => expr_type(body, &known, &fields),
                        ArrowFunctionBody::FunctionBody(body) => match returned_value(&body.stmts) {
                            None => Some(TsKeywordTypeKind::TsVoidKeyword),
                            Some(value) => expr_type(value, &known, &fields),
                        },
                    };
                    let Some(kind) = kind else { continue };
                    keyword_annotation(kind)
                };
                prop.type_ann = Some(Box::new(TsTypeAnn {
                    span: swc_common::DUMMY_SP,
                    type_ann: Box::new(TsType::TsFnOrConstructorType(
                        TsFnOrConstructorType::TsFnType(TsFnType {
                            span: swc_common::DUMMY_SP,
                            params,
                            type_params: None,
                            type_ann: return_type,
                        }),
                    )),
                }));
            }
        }
        fn visit_mut_constructor(&mut self, constructor: &mut Constructor) {
            constructor.visit_mut_children_with(self);
            // `constructor(private z = 3)`: a parameter property takes its default's type.
            for param in &mut constructor.params {
                let ParamOrTsParamProp::TsParamProp(property) = param else { continue };
                let TsParamPropParam::Assign(assign) = &mut property.param else { continue };
                if let Pat::Ident(binding) = assign.left.as_mut() {
                    if binding.type_ann.is_none() {
                        if let Some(kind) = expr_type(&assign.right, &HashMap::new(), &HashMap::new()) {
                            binding.type_ann = Some(keyword_annotation(kind));
                        }
                    }
                }
            }
            annotate_params(constructor.params.iter_mut().filter_map(|param| match param {
                ParamOrTsParamProp::Param(param) => Some(&mut param.pat),
                ParamOrTsParamProp::TsParamProp(_) => None,
            }));
        }
    }
    let mut module = module.clone();
    module.visit_mut_with(&mut Annotator);
    module
}

/// The object real JS builds for an enum: members and (numeric) reverse entries, keys as given.
fn enum_object_expr(
    name: &str,
    keys: impl Iterator<Item = String>,
    values: &EnumValues,
    reverse: &EnumReverseValues,
) -> Expr {
    use swc_ecma_ast::*;
    let text = |value: &str| Expr::Lit(Lit::Str(Str { span: swc_common::DUMMY_SP, value: value.into(), raw: None }));
    let props = keys
        .map(|key| {
            let value = match values.get(&(name.to_string(), key.clone())) {
                Some(HirLit::F64(number)) => Expr::Lit(Lit::Num(Number { span: swc_common::DUMMY_SP, value: *number, raw: None })),
                Some(HirLit::Str(value)) => text(value),
                _ => text(
                    reverse
                        .get(name)
                        .and_then(|entries| entries.iter().find(|(number, _)| format!("{number}") == key))
                        .map_or("", |(_, member)| member.as_str()),
                ),
            };
            PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                key: PropName::Str(Str { span: swc_common::DUMMY_SP, value: key.as_str().into(), raw: None }),
                value: Box::new(value),
            })))
        })
        .collect();
    Expr::Object(ObjectLit { span: swc_common::DUMMY_SP, props })
}

/// `E[name]` with a string-typed `name`: the same lookup against the enum object.
fn enum_dynamic_index(
    name: &str,
    index: &Expr,
    values: &EnumValues,
    reverse: &EnumReverseValues,
) -> Expr {
    use swc_ecma_ast::*;
    let mut keys: Vec<String> = values.keys().filter(|(n, _)| n == name).map(|(_, m)| m.clone()).collect();
    keys.sort();
    keys.extend(reverse.get(name).into_iter().flatten().map(|(number, _)| format!("{number}")));
    Expr::Member(MemberExpr {
        span: swc_common::DUMMY_SP,
        obj: Box::new(enum_object_expr(name, keys.into_iter(), values, reverse)),
        prop: MemberProp::Computed(ComputedPropName { span: swc_common::DUMMY_SP, expr: Box::new(index.clone()) }),
    })
}

/// Makes a top-level enum usable as a value: a bare `E` (not `E.A` / `E[x]`) becomes the object
/// `{ "0": "A", ..., A: 0, ... }` real JS builds (integer keys first), and `typeof E` becomes the
/// matching object type. Shadowing of the enum name by a local is not tracked.
// ponytail: only top-level non-exported enums (the ones `collect_enums` sees).
pub fn enums_as_values(module: &Module) -> Module {
    use swc_ecma_ast::*;
    use swc_ecma_visit::{VisitMut, VisitMutWith};

    let Ok((values, reverse, _)) = collect_enums(module) else {
        return module.clone();
    };
    // name -> ordered (key, is_numeric_value) list, integer reverse keys first.
    let mut shapes: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(declaration))) = item else { continue };
        let name = declaration.id.sym.to_string();
        let shape = shapes.entry(name.clone()).or_default();
        let mut ints: Vec<(f64, String)> = Vec::new();
        let mut rest: Vec<(String, bool)> = Vec::new();
        for member in &declaration.members {
            let member = enum_member_name(&member.id);
            let numeric = matches!(values.get(&(name.clone(), member.clone())), Some(HirLit::F64(_)));
            rest.push((member, numeric));
        }
        for (number, _) in reverse.get(&name).into_iter().flatten() {
            if number.fract() == 0.0 && *number >= 0.0 && *number < 4294967295.0 {
                ints.push((*number, format!("{number}")));
            } else {
                rest.push((format!("{number}"), false));
            }
        }
        ints.sort_by(|a, b| a.0.total_cmp(&b.0));
        shape.extend(ints.into_iter().map(|(_, key)| (key, false)));
        shape.extend(rest);
    }
    if shapes.is_empty() {
        return module.clone();
    }
    fn matches_name<'a>(shapes: &'a HashMap<String, Vec<(String, bool)>>, name: &str) -> Option<&'a str> {
        // Types keep their bare names while module flattening mangles value names.
        shapes.keys().map(String::as_str).find(|key| {
            *key == name
                || key.strip_prefix("__thawmod").and_then(|rest| rest.split_once('_')).is_some_and(
                    |(digits, original)| digits.bytes().all(|b| b.is_ascii_digit()) && original == name,
                )
        })
    }
    struct Rewrite<'a> {
        shapes: &'a HashMap<String, Vec<(String, bool)>>,
        values: &'a EnumValues,
        reverse: &'a EnumReverseValues,
    }
    impl Rewrite<'_> {
        fn object(&self, name: &str) -> Expr {
            enum_object_expr(name, self.shapes[name].iter().map(|(key, _)| key.clone()), self.values, self.reverse)
        }
        fn type_literal(&self, name: &str) -> TsType {
            let members = self.shapes[name]
                .iter()
                .map(|(key, numeric)| {
                    let is_member = self.values.contains_key(&(name.to_string(), key.clone()));
                    let kind = if is_member && *numeric { TsKeywordTypeKind::TsNumberKeyword } else { TsKeywordTypeKind::TsStringKeyword };
                    TsTypeElement::TsPropertySignature(TsPropertySignature {
                        span: swc_common::DUMMY_SP,
                        readonly: false,
                        key: Box::new(Expr::Lit(Lit::Str(Str { span: swc_common::DUMMY_SP, value: key.as_str().into(), raw: None }))),
                        computed: false,
                        optional: false,
                        type_ann: Some(Box::new(TsTypeAnn {
                            span: swc_common::DUMMY_SP,
                            type_ann: Box::new(TsType::TsKeywordType(TsKeywordType { span: swc_common::DUMMY_SP, kind })),
                        })),
                    })
                })
                .collect();
            TsType::TsTypeLit(TsTypeLit { span: swc_common::DUMMY_SP, members })
        }
    }
    impl VisitMut for Rewrite<'_> {
        fn visit_mut_expr(&mut self, expr: &mut Expr) {
            match expr {
                // `E.A` / `E[x]` keep their compile-time lowering; only the index is visited.
                Expr::Member(member) if matches!(member.obj.as_ref(), Expr::Ident(id) if self.shapes.contains_key(id.sym.as_ref())) => {
                    member.prop.visit_mut_with(self);
                }
                Expr::Ident(id) if self.shapes.contains_key(id.sym.as_ref()) => {
                    *expr = self.object(id.sym.as_ref());
                }
                _ => expr.visit_mut_children_with(self),
            }
        }
        fn visit_mut_ts_type(&mut self, ty: &mut TsType) {
            if let TsType::TsTypeQuery(query) = ty {
                if let TsTypeQueryExpr::TsEntityName(TsEntityName::Ident(id)) = &query.expr_name {
                    if let Some(name) = matches_name(self.shapes, id.sym.as_ref()) {
                        *ty = self.type_literal(name);
                        return;
                    }
                }
            }
            ty.visit_mut_children_with(self);
        }
    }
    let mut module = module.clone();
    module.visit_mut_with(&mut Rewrite { shapes: &shapes, values: &values, reverse: &reverse });
    module
}

/// Flattens `namespace NS { export const a = 1; export function f() {} }` into top-level
/// declarations named `NS__a`/`NS__f`, and rewrites `NS.a` (value and type position) and the
/// namespace body's own unqualified references to match. Types stay under their bare names, as
/// `collect_type_declarations` already hoists them. A namespace is left untouched (and so
/// rejected as before) when it is exported, ambient, merged-in-body unsupported, or a local
/// binding shadows one of its members.
pub fn flatten_namespaces(module: &Module) -> Module {
    use swc_ecma_ast::*;
    use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};

    #[derive(Clone)]
    enum Binding {
        Flat(String),
        Namespace(String),
    }
    type Members = HashMap<String, Binding>;

    fn block_of(declaration: &TsModuleDecl) -> Option<&TsModuleBlock> {
        match (&declaration.id, &declaration.body) {
            (TsModuleName::Ident(_), Some(TsNamespaceBody::TsModuleBlock(block)))
                if !declaration.declare && !declaration.global =>
            {
                Some(block)
            }
            _ => None,
        }
    }
    fn declared(item: &ModuleItem) -> Option<(&Decl, bool)> {
        match item {
            ModuleItem::Stmt(Stmt::Decl(decl)) => Some((decl, false)),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some((&export.decl, true)),
            _ => None,
        }
    }
    fn has_value(block: &TsModuleBlock) -> bool {
        block.body.iter().any(|item| match declared(item) {
            Some((Decl::TsInterface(_) | Decl::TsTypeAlias(_), _)) => false,
            Some((Decl::TsModule(inner), _)) => block_of(inner).is_some_and(has_value),
            _ => true,
        })
    }
    /// How often each name is bound (variable, parameter, function, class) anywhere in a body.
    #[derive(Default)]
    struct Bindings(HashMap<String, usize>);
    impl Visit for Bindings {
        fn visit_binding_ident(&mut self, binding: &BindingIdent) {
            *self.0.entry(binding.id.sym.to_string()).or_default() += 1;
        }
        // `x = 1` / `x += 1` assigns to an existing binding, it does not declare one.
        fn visit_simple_assign_target(&mut self, target: &SimpleAssignTarget) {
            if !matches!(target, SimpleAssignTarget::Ident(_)) {
                target.visit_children_with(self);
            }
        }
        fn visit_fn_decl(&mut self, function: &FnDecl) {
            *self.0.entry(function.ident.sym.to_string()).or_default() += 1;
            function.function.visit_with(self);
        }
        fn visit_class_decl(&mut self, class: &ClassDecl) {
            *self.0.entry(class.ident.sym.to_string()).or_default() += 1;
            class.class.visit_with(self);
        }
        fn visit_ts_enum_decl(&mut self, declaration: &TsEnumDecl) {
            *self.0.entry(declaration.id.sym.to_string()).or_default() += 1;
        }
    }

    struct Hoist {
        exported: HashMap<String, Members>,
        env: HashMap<String, Members>,
        items: Vec<(String, ModuleItem)>,
        /// Exported names per namespace in declaration order, and the type of each value member
        /// (when its declaration spells one out) for `typeof NS`.
        order: HashMap<String, Vec<String>>,
        types: HashMap<String, TsType>,
        failed: bool,
    }
    impl Hoist {
        /// Records the declared type of a (renamed) value declaration, for `typeof NS`.
        fn record_type(&mut self, decl: &Decl) {
            let ann = |ann: &Option<Box<TsTypeAnn>>| ann.as_ref().map(|a| a.type_ann.clone());
            match decl {
                Decl::Fn(function) if function.function.type_params.is_none() => {
                    let (params, ret) = (
                        function
                            .function
                            .params
                            .iter()
                            .map(|param| match &param.pat {
                                Pat::Ident(binding) if binding.type_ann.is_some() => Some(TsFnParam::Ident(binding.clone())),
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>(),
                        function.function.return_type.clone(),
                    );
                    if let (Some(params), Some(ret)) = (params, ret) {
                        self.types.insert(
                            function.ident.sym.to_string(),
                            TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(TsFnType {
                                span: swc_common::DUMMY_SP,
                                params,
                                type_params: None,
                                type_ann: ret,
                            })),
                        );
                    }
                }
                Decl::Var(var) => {
                    for declarator in &var.decls {
                        let Pat::Ident(binding) = &declarator.name else { continue };
                        let kind = match declarator.init.as_deref() {
                            _ if binding.type_ann.is_some() => None,
                            Some(Expr::Lit(Lit::Num(_))) => Some(TsKeywordTypeKind::TsNumberKeyword),
                            Some(Expr::Lit(Lit::Str(_))) => Some(TsKeywordTypeKind::TsStringKeyword),
                            Some(Expr::Lit(Lit::Bool(_))) => Some(TsKeywordTypeKind::TsBooleanKeyword),
                            _ => continue,
                        };
                        let ty = match kind {
                            Some(kind) => Box::new(TsType::TsKeywordType(TsKeywordType { span: swc_common::DUMMY_SP, kind })),
                            None => ann(&binding.type_ann).expect("checked above"),
                        };
                        self.types.insert(binding.id.sym.to_string(), *ty);
                    }
                }
                _ => {}
            }
        }
        fn block(&mut self, path: &str, parent: &Members, block: &TsModuleBlock) {
            let mut counts = Bindings::default();
            block.visit_with(&mut counts);
            let mut members = parent.clone();
            let mut exported = Members::new();
            let mut order = Vec::new();
            let mut hoisted = Vec::new();
            for item in &block.body {
                let Some((decl, is_export)) = declared(item) else {
                    if matches!(item, ModuleItem::Stmt(_)) {
                        hoisted.push(item.clone());
                    } else {
                        self.failed = true;
                    }
                    continue;
                };
                let mut decl = decl.clone();
                let mut names: Vec<(String, Binding)> = Vec::new();
                let rename = |ident: &mut Ident, names: &mut Vec<(String, Binding)>| {
                    let flat = format!("{path}__{}", ident.sym);
                    names.push((ident.sym.to_string(), Binding::Flat(flat.clone())));
                    ident.sym = flat.into();
                };
                match &mut decl {
                    Decl::Var(var) => {
                        for declarator in &mut var.decls {
                            match &mut declarator.name {
                                Pat::Ident(binding) => rename(&mut binding.id, &mut names),
                                _ => self.failed = true,
                            }
                        }
                    }
                    Decl::Fn(function) => rename(&mut function.ident, &mut names),
                    Decl::Class(class) => rename(&mut class.ident, &mut names),
                    Decl::TsEnum(declaration) => rename(&mut declaration.id, &mut names),
                    // Types keep their bare names; recording them lets `NS.Shape` resolve to `Shape`.
                    Decl::TsInterface(declaration) => {
                        names.push((declaration.id.sym.to_string(), Binding::Flat(declaration.id.sym.to_string())));
                    }
                    Decl::TsTypeAlias(declaration) => {
                        names.push((declaration.id.sym.to_string(), Binding::Flat(declaration.id.sym.to_string())));
                    }
                    Decl::TsModule(inner) => match (block_of(inner), &inner.id) {
                        (Some(inner_block), TsModuleName::Ident(name)) if has_value(inner_block) => {
                            let inner_path = format!("{path}__{}", name.sym);
                            names.push((name.sym.to_string(), Binding::Namespace(inner_path.clone())));
                            let mut inner_members = members.clone();
                            inner_members.insert(name.sym.to_string(), Binding::Namespace(inner_path.clone()));
                            self.block(&inner_path, &inner_members, inner_block);
                            for (name, binding) in &names {
                                members.insert(name.clone(), binding.clone());
                                if is_export {
                                    exported.insert(name.clone(), binding.clone());
                                    order.push(name.clone());
                                }
                            }
                            continue;
                        }
                        _ => self.failed = true,
                    },
                    _ => self.failed = true,
                }
                for (name, binding) in names {
                    if counts.0.get(&name).copied().unwrap_or(0) > 1 {
                        self.failed = true;
                    }
                    members.insert(name.clone(), binding.clone());
                    if is_export {
                        order.push(name.clone());
                        exported.insert(name, binding);
                    }
                }
                self.record_type(&decl);
                hoisted.push(ModuleItem::Stmt(Stmt::Decl(decl)));
            }
            // Reopened namespaces merge their exports.
            self.order.entry(path.to_string()).or_default().extend(order);
            self.exported.entry(path.to_string()).or_default().extend(exported);
            self.env.entry(path.to_string()).or_default().extend(members);
            for item in hoisted {
                self.items.push((path.to_string(), item));
            }
        }
    }

    struct Rewrite<'a> {
        env: &'a Members,
        exported: &'a HashMap<String, Members>,
        order: &'a HashMap<String, Vec<String>>,
        types: &'a HashMap<String, TsType>,
        /// Namespaces merged with a same-named class or enum: the bare name stays the class/enum.
        merged: &'a HashSet<String>,
    }
    impl Rewrite<'_> {
        /// `NS` used as a value: an object of its exported values (types are skipped; a type
        /// keeps its bare name, so it is the only kind whose flat name equals its own).
        fn namespace_object(&self, path: &str) -> Expr {
            let mut props = Vec::new();
            for name in self.order.get(path).into_iter().flatten() {
                let value = match self.exported.get(path).and_then(|m| m.get(name)) {
                    Some(Binding::Flat(flat)) if flat != name => Expr::Ident(Ident::new_no_ctxt(flat.as_str().into(), swc_common::DUMMY_SP)),
                    Some(Binding::Namespace(inner)) => self.namespace_object(inner),
                    _ => continue,
                };
                props.push(PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                    key: PropName::Ident(IdentName::new(name.as_str().into(), swc_common::DUMMY_SP)),
                    value: Box::new(value),
                }))));
            }
            Expr::Object(ObjectLit { span: swc_common::DUMMY_SP, props })
        }
        /// `typeof NS` as an object type, or `None` when a member's type is not spelled out.
        fn namespace_type(&self, path: &str) -> Option<TsType> {
            let mut members = Vec::new();
            for name in self.order.get(path).into_iter().flatten() {
                let ty = match self.exported.get(path).and_then(|m| m.get(name)) {
                    Some(Binding::Flat(flat)) if flat != name => self.types.get(flat)?.clone(),
                    Some(Binding::Namespace(inner)) => self.namespace_type(inner)?,
                    _ => continue,
                };
                members.push(TsTypeElement::TsPropertySignature(TsPropertySignature {
                    span: swc_common::DUMMY_SP,
                    readonly: false,
                    key: Box::new(Expr::Ident(Ident::new_no_ctxt(name.as_str().into(), swc_common::DUMMY_SP))),
                    computed: false,
                    optional: false,
                    type_ann: Some(Box::new(TsTypeAnn { span: swc_common::DUMMY_SP, type_ann: Box::new(ty) })),
                }));
            }
            Some(TsType::TsTypeLit(TsTypeLit { span: swc_common::DUMMY_SP, members }))
        }
        fn namespace_of_expr(&self, expr: &Expr) -> Option<&str> {
            match expr {
                Expr::Ident(ident) => match self.env.get(ident.sym.as_ref()) {
                    Some(Binding::Namespace(path)) => Some(path),
                    _ => None,
                },
                Expr::Member(MemberExpr { obj, prop: MemberProp::Ident(name), .. }) => {
                    let outer = self.namespace_of_expr(obj)?;
                    match self.exported.get(outer)?.get(name.sym.as_ref()) {
                        Some(Binding::Namespace(path)) => Some(path),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        fn namespace_of_entity(&self, name: &TsEntityName) -> Option<&str> {
            match name {
                // The module-flattening rename leaves `NS` in type positions untouched.
                TsEntityName::Ident(ident) => match self.env.get(ident.sym.as_ref()).or_else(|| {
                    self.env.iter().find_map(|(key, binding)| {
                        let (digits, original) = key.strip_prefix("__thawmod")?.split_once('_')?;
                        (digits.bytes().all(|b| b.is_ascii_digit()) && original == ident.sym.as_ref())
                            .then_some(binding)
                    })
                }) {
                    Some(Binding::Namespace(path)) => Some(path),
                    _ => None,
                },
                TsEntityName::TsQualifiedName(qualified) => {
                    let outer = self.namespace_of_entity(&qualified.left)?;
                    match self.exported.get(outer)?.get(qualified.right.sym.as_ref()) {
                        Some(Binding::Namespace(path)) => Some(path),
                        _ => None,
                    }
                }
            }
        }
        fn flat(&self, ident: &Ident) -> Option<String> {
            match self.env.get(ident.sym.as_ref()) {
                Some(Binding::Flat(flat)) => Some(flat.clone()),
                _ => None,
            }
        }
    }
    impl VisitMut for Rewrite<'_> {
        fn visit_mut_expr(&mut self, expr: &mut Expr) {
            if let Expr::Member(member) = expr {
                if let MemberProp::Ident(name) = &member.prop {
                    if let Some(path) = self.namespace_of_expr(&member.obj) {
                        if let Some(Binding::Flat(flat)) =
                            self.exported.get(path).and_then(|m| m.get(name.sym.as_ref()))
                        {
                            *expr = Expr::Ident(Ident::new_no_ctxt(flat.as_str().into(), member.span));
                            return;
                        }
                    }
                }
            }
            if let Expr::Ident(ident) = expr {
                if let Some(Binding::Namespace(path)) = self.env.get(ident.sym.as_ref()) {
                    if !self.merged.contains(ident.sym.as_ref()) {
                        *expr = self.namespace_object(path);
                    }
                    return;
                }
                if let Some(flat) = self.flat(ident) {
                    ident.sym = flat.into();
                }
                return;
            }
            expr.visit_mut_children_with(self);
        }
        fn visit_mut_prop(&mut self, prop: &mut Prop) {
            if let Prop::Shorthand(ident) = prop {
                if let Some(flat) = self.flat(ident) {
                    *prop = Prop::KeyValue(KeyValueProp {
                        key: PropName::Ident(IdentName::new(ident.sym.clone(), ident.span)),
                        value: Box::new(Expr::Ident(Ident::new_no_ctxt(flat.into(), ident.span))),
                    });
                }
                return;
            }
            prop.visit_mut_children_with(self);
        }
        fn visit_mut_simple_assign_target(&mut self, target: &mut SimpleAssignTarget) {
            if let SimpleAssignTarget::Ident(binding) = target {
                if let Some(flat) = self.flat(&binding.id) {
                    binding.id.sym = flat.into();
                }
                return;
            }
            target.visit_mut_children_with(self);
        }
        fn visit_mut_ts_type(&mut self, ty: &mut TsType) {
            if let TsType::TsTypeQuery(query) = ty {
                if let TsTypeQueryExpr::TsEntityName(entity) = &query.expr_name {
                    if let Some(object) = self.namespace_of_entity(entity).and_then(|path| self.namespace_type(path)) {
                        *ty = object;
                        return;
                    }
                }
            }
            ty.visit_mut_children_with(self);
        }
        fn visit_mut_ts_entity_name(&mut self, name: &mut TsEntityName) {
            match name {
                TsEntityName::TsQualifiedName(qualified) => {
                    if let Some(path) = self.namespace_of_entity(&qualified.left) {
                        if let Some(Binding::Flat(flat)) =
                            self.exported.get(path).and_then(|m| m.get(qualified.right.sym.as_ref()))
                        {
                            *name = TsEntityName::Ident(Ident::new_no_ctxt(
                                flat.as_str().into(),
                                qualified.span,
                            ));
                        }
                    }
                }
                TsEntityName::Ident(ident) => {
                    if let Some(flat) = self.flat(ident) {
                        ident.sym = flat.into();
                    }
                }
            }
        }
    }

    // Top-level namespaces (plain statements) that declare at least one value.
    let mut top = Members::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(declaration))) = item {
            if let (Some(block), TsModuleName::Ident(name)) = (block_of(declaration), &declaration.id) {
                if has_value(block) {
                    top.insert(name.sym.to_string(), Binding::Namespace(name.sym.to_string()));
                }
            }
        }
    }
    if top.is_empty() {
        return module.clone();
    }
    let mut hoist = Hoist {
        exported: HashMap::new(),
        env: HashMap::new(),
        items: Vec::new(),
        order: HashMap::new(),
        types: HashMap::new(),
        failed: false,
    };
    let mut layout: Vec<Result<ModuleItem, usize>> = Vec::new(); // namespace slots keep source order
    let mut namespaces: Vec<(String, usize, usize)> = Vec::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(declaration))) = item {
            if let (Some(block), TsModuleName::Ident(name)) = (block_of(declaration), &declaration.id) {
                if top.contains_key(name.sym.as_ref()) {
                    let start = hoist.items.len();
                    hoist.block(name.sym.as_ref(), &top, block);
                    namespaces.push((name.sym.to_string(), start, hoist.items.len()));
                    layout.push(Err(namespaces.len() - 1));
                    continue;
                }
            }
        }
        layout.push(Ok(item.clone()));
    }
    if hoist.failed {
        return module.clone();
    }
    let exported = std::mem::take(&mut hoist.exported);
    let (order, types) = (std::mem::take(&mut hoist.order), std::mem::take(&mut hoist.types));
    let merged: HashSet<String> = module.body.iter().filter_map(|item| match declared(item) {
        Some((Decl::Class(class), _)) if top.contains_key(class.ident.sym.as_ref()) => Some(class.ident.sym.to_string()),
        Some((Decl::TsEnum(declaration), _)) if top.contains_key(declaration.id.sym.as_ref()) => Some(declaration.id.sym.to_string()),
        _ => None,
    }).collect();
    let rewrite_item = |env: &Members, item: &mut ModuleItem| {
        item.visit_mut_with(&mut Rewrite { env, exported: &exported, order: &order, types: &types, merged: &merged });
    };
    let mut body = Vec::new();
    for slot in layout {
        match slot {
            Ok(mut item) => {
                rewrite_item(&top, &mut item);
                body.push(item);
            }
            Err(index) => {
                let (_, start, end) = &namespaces[index];
                for (path, item) in &hoist.items[*start..*end] {
                    let mut item = item.clone();
                    rewrite_item(&hoist.env[path], &mut item);
                    body.push(item);
                }
            }
        }
    }
    Module { body, ..module.clone() }
}

/// `new Map()`/`new Set()` have no type arguments of their own; copy them from
/// the annotation the value flows into, like TypeScript's contextual typing:
/// `const m: Map<K, V> = new Map()`, `f: Map<K, V> = new Map()` class fields,
/// `this.f = new Map()` / `x = new Map()` assignments to an annotated field or
/// variable, and `return new Map()` from a function with a declared return type.
/// Names annotated differently in different places are left alone.
pub fn infer_collection_type_args(module: &Module) -> Module {
    use swc_ecma_ast::*;
    type Annotations = HashMap<String, Option<TsType>>;
    /// The `Map<..>`/`Set<..>` reference inside `T`, `T | undefined`, `T | null`.
    fn collection_ref(ty: &TsType) -> Option<(String, Box<TsTypeParamInstantiation>)> {
        match ty {
            TsType::TsTypeRef(reference) => {
                let TsEntityName::Ident(name) = &reference.type_name else { return None };
                matches!(name.sym.as_ref(), "Map" | "Set" | "WeakMap" | "WeakSet")
                    .then(|| reference.type_params.clone().map(|params| (name.sym.to_string(), params)))
                    .flatten()
            }
            TsType::TsParenthesizedType(paren) => collection_ref(&paren.type_ann),
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
                let mut found = None;
                for member in &union.types {
                    match member.as_ref() {
                        TsType::TsKeywordType(keyword)
                            if matches!(keyword.kind, TsKeywordTypeKind::TsUndefinedKeyword | TsKeywordTypeKind::TsNullKeyword) => {}
                        other => {
                            if found.is_some() { return None; }
                            found = Some(collection_ref(other)?);
                        }
                    }
                }
                found
            }
            _ => None,
        }
    }
    fn fill(expression: &mut Expr, ty: Option<&TsType>) {
        let Some((class, params)) = ty.and_then(collection_ref) else { return };
        let mut current = &mut *expression;
        while let Expr::Paren(paren) = current { current = &mut paren.expr; }
        if let Expr::New(new) = current {
            if new.type_args.is_none()
                && matches!(new.callee.as_ref(), Expr::Ident(name) if name.sym.as_ref() == class)
            {
                new.type_args = Some(params);
            }
        }
    }
    struct Collect { names: Annotations, fields: Annotations }
    fn note(map: &mut Annotations, name: String, ty: &TsType) {
        match map.get(&name) {
            Some(Some(existing)) if !swc_common::EqIgnoreSpan::eq_ignore_span(existing, ty) => { map.insert(name, None); }
            Some(_) => {}
            None => { map.insert(name, Some(ty.clone())); }
        }
    }
    impl Visit for Collect {
        fn visit_binding_ident(&mut self, binding: &BindingIdent) {
            if let Some(annotation) = &binding.type_ann {
                note(&mut self.names, binding.id.sym.to_string(), &annotation.type_ann);
            }
        }
        fn visit_class_prop(&mut self, property: &ClassProp) {
            if let (PropName::Ident(name), Some(annotation)) = (&property.key, &property.type_ann) {
                note(&mut self.fields, name.sym.to_string(), &annotation.type_ann);
            }
            property.visit_children_with(self);
        }
    }
    struct Fill { names: Annotations, fields: Annotations, returns: Vec<Option<TsType>> }
    impl Fill {
        fn lookup<'a>(map: &'a Annotations, name: &str) -> Option<&'a TsType> {
            map.get(name).and_then(Option::as_ref)
        }
    }
    impl VisitMut for Fill {
        fn visit_mut_var_decl(&mut self, declaration: &mut VarDecl) {
            if !declaration.declare {
                declaration.visit_mut_children_with(self);
            }
        }
        fn visit_mut_var_declarator(&mut self, declarator: &mut VarDeclarator) {
            declarator.visit_mut_children_with(self);
            if let (Pat::Ident(binding), Some(init)) = (&declarator.name, declarator.init.as_deref_mut()) {
                fill(init, binding.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref()));
            }
            // `let m: Map<K, V>;` assigned later: a collection has no
            // `undefined` representation, so start it as an empty one.
            if let (Pat::Ident(binding), None) = (&declarator.name, &declarator.init) {
                let annotation = binding.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref());
                if let Some(TsType::TsTypeRef(reference)) = annotation {
                    if let (Some((class, params)), TsEntityName::Ident(name)) =
                        (collection_ref(&TsType::TsTypeRef(reference.clone())), &reference.type_name)
                    {
                        declarator.init = Some(Box::new(Expr::New(NewExpr {
                            span: name.span,
                            ctxt: Default::default(),
                            callee: Box::new(Expr::Ident(Ident::new_no_ctxt(class.into(), name.span))),
                            args: Some(Vec::new()),
                            type_args: Some(params),
                        })));
                    }
                }
            }
        }
        fn visit_mut_class_prop(&mut self, property: &mut ClassProp) {
            property.visit_mut_children_with(self);
            if let Some(value) = property.value.as_deref_mut() {
                fill(value, property.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref()));
            }
        }
        fn visit_mut_assign_expr(&mut self, assign: &mut AssignExpr) {
            assign.visit_mut_children_with(self);
            if assign.op != AssignOp::Assign { return; }
            let ty = match &assign.left {
                AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => {
                    Self::lookup(&self.names, binding.id.sym.as_ref())
                }
                AssignTarget::Simple(SimpleAssignTarget::Member(member))
                    if matches!(member.obj.as_ref(), Expr::This(_)) =>
                {
                    match &member.prop {
                        MemberProp::Ident(name) => Self::lookup(&self.fields, name.sym.as_ref()),
                        _ => None,
                    }
                }
                _ => None,
            };
            let ty = ty.cloned();
            fill(&mut assign.right, ty.as_ref());
        }
        fn visit_mut_function(&mut self, function: &mut Function) {
            self.returns.push(function.return_type.as_ref().map(|annotation| (*annotation.type_ann).clone()));
            function.visit_mut_children_with(self);
            self.returns.pop();
        }
        fn visit_mut_arrow_expr(&mut self, arrow: &mut ArrowExpr) {
            self.returns.push(arrow.return_type.as_ref().map(|annotation| (*annotation.type_ann).clone()));
            arrow.visit_mut_children_with(self);
            if let (Some(Some(ty)), ArrowFunctionBody::Expr(body)) = (self.returns.last(), arrow.body.as_mut()) {
                fill(body, Some(&ty.clone()));
            }
            self.returns.pop();
        }
        fn visit_mut_return_stmt(&mut self, statement: &mut ReturnStmt) {
            statement.visit_mut_children_with(self);
            if let (Some(argument), Some(Some(ty))) = (statement.arg.as_deref_mut(), self.returns.last()) {
                fill(argument, Some(&ty.clone()));
            }
        }
    }
    let mut collect = Collect { names: HashMap::new(), fields: HashMap::new() };
    module.visit_with(&mut collect);
    let mut module = module.clone();
    module.visit_mut_with(&mut Fill { names: collect.names, fields: collect.fields, returns: Vec::new() });
    module
}

/// `Record<E, T>` / `{ [K in E]: T }` with an enum `E` as the key set: spell the keys as the enum's
/// literal values, which is what the finite-key utility types resolve.
pub fn enum_keys_as_literals(module: &Module) -> Module {
    use swc_ecma_ast::*;
    let Ok((values, _, _)) = collect_enums(module) else { return module.clone() };
    if values.is_empty() {
        return module.clone();
    }
    let literal_union = |name: &str, span: swc_common::Span| -> Option<TsType> {
        let mut members: Vec<(String, HirLit)> = values
            .iter()
            .filter(|((enum_name, _), _)| enum_name == name)
            .map(|((_, member), value)| (member.clone(), value.clone()))
            .collect();
        if members.is_empty() {
            return None;
        }
        members.sort_by(|a, b| match (&a.1, &b.1) {
            (HirLit::F64(x), HirLit::F64(y)) => x.total_cmp(y),
            _ => a.0.cmp(&b.0),
        });
        let mut types = Vec::new();
        for (_, value) in members {
            let lit = match value {
                HirLit::F64(number) => TsLit::Number(Number { span, value: number, raw: None }),
                HirLit::Str(text) => TsLit::Str(Str { span, value: text.as_str().into(), raw: None }),
                _ => return None,
            };
            let ty = Box::new(TsType::TsLitType(TsLitType { span, lit }));
            if !types.iter().any(|existing| swc_common::EqIgnoreSpan::eq_ignore_span(existing, &ty)) {
                types.push(ty);
            }
        }
        Some(TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(TsUnionType { span, types })))
    };
    struct Rewrite<'a> {
        literal_union: &'a dyn Fn(&str, swc_common::Span) -> Option<TsType>,
        values: &'a EnumValues,
    }
    impl Rewrite<'_> {
        fn replace(&self, ty: &mut TsType) {
            if let TsType::TsTypeRef(reference) = ty {
                if let (TsEntityName::Ident(name), None) = (&reference.type_name, &reference.type_params) {
                    if let Some(union) = (self.literal_union)(name.sym.as_ref(), reference.span) {
                        *ty = union;
                    }
                }
            }
        }
    }
    impl VisitMut for Rewrite<'_> {
        fn visit_mut_ts_type_ref(&mut self, reference: &mut TsTypeRef) {
            reference.visit_mut_children_with(self);
            let is_record = matches!(&reference.type_name, TsEntityName::Ident(name) if name.sym == *"Record");
            if is_record {
                if let Some(params) = reference.type_params.as_mut() {
                    if let Some(key) = params.params.first_mut() {
                        self.replace(key);
                    }
                }
            }
        }
        /// `{ [E.A]: v }`: a constant enum key is an ordinary named property.
        fn visit_mut_prop_name(&mut self, name: &mut PropName) {
            name.visit_mut_children_with(self);
            if let PropName::Computed(computed) = name {
                // An enum member read already folded to its literal value.
                match computed.expr.as_ref() {
                    Expr::Lit(Lit::Num(number)) => {
                        *name = PropName::Num(number.clone());
                        return;
                    }
                    Expr::Lit(Lit::Str(text)) => {
                        *name = PropName::Str(text.clone());
                        return;
                    }
                    _ => {}
                }
            }
            if let PropName::Computed(computed) = name {
                if let Expr::Member(member) = computed.expr.as_ref() {
                    if let (Expr::Ident(owner), MemberProp::Ident(property)) = (member.obj.as_ref(), &member.prop) {
                        let key = (owner.sym.to_string(), property.sym.to_string());
                        match self.values.get(&key) {
                            Some(HirLit::F64(number)) => {
                                *name = PropName::Num(Number { span: member.span, value: *number, raw: None });
                            }
                            Some(HirLit::Str(text)) => {
                                *name = PropName::Str(Str { span: member.span, value: text.as_str().into(), raw: None });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        fn visit_mut_ts_mapped_type(&mut self, mapped: &mut TsMappedType) {
            mapped.visit_mut_children_with(self);
            if let Some(constraint) = mapped.type_param.constraint.as_mut() {
                self.replace(constraint);
            }
        }
    }
    let mut result = module.clone();
    result.visit_mut_with(&mut Rewrite { literal_union: &literal_union, values: &values });
    result
}
