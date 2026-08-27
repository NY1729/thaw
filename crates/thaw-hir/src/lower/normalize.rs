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
        Expr::Lit(Lit::Str(value)) => Ok(HirLit::Str(value.value.to_string_lossy().into_owned())),
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

    fn property_name(key: &PropName) -> Result<String, String> {
        match key {
            PropName::Ident(identifier) => Ok(identifier.sym.to_string()),
            PropName::Str(string) => Ok(string.value.to_string_lossy().into_owned()),
            PropName::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(string)) => Ok(string.value.to_string_lossy().into_owned()),
                _ => Err("top-level object rest requires static string keys".into()),
            },
            _ => Err("top-level object rest requires static string keys".into()),
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
                            used_keys.push(property.key.id.sym.to_string());
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
                            if has_rest {
                                used_keys.push(property_name(&property.key)?);
                            }
                            expand_pattern(
                                &property.value,
                                member(temporary_expr(), &property.key, span)?,
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
                            args.extend(used_keys.iter().map(|key| swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                                    span,
                                    value: key.clone().into(),
                                    raw: None,
                                }))),
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
                            || binding.id.sym.starts_with("__thaw_top_default_"))
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
