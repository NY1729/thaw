#[allow(clippy::too_many_arguments)]
fn lower_top_level_initializers(
    module: &Module,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<Vec<HirInitStep>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        call_constraints,
    );
    for (name, ty) in global_types {
        if is_class_static_field_symbol(name) {
            lowerer.scope.insert(name.clone(), ty.clone());
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
            if immutable_globals.contains(name) {
                lowerer.immutable_bindings.insert(name.clone());
            }
        }
    }
    let mut steps = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => {
                for declarator in &declaration.decls {
                    let Pat::Ident(binding) = &declarator.name else {
                        unreachable!("top-level patterns were validated above")
                    };
                    let name = binding.id.sym.to_string();
                    let expected = global_types[&name].clone();
                    let init = lowerer.lower_expr(
                        declarator
                            .init
                            .as_deref()
                            .expect("top-level initializers were validated above"),
                    )?;
                    let ty = if expected == HirType::Dynamic {
                        lowerer.infer_expr_type(&init)?
                    } else {
                        expected
                    };
                    let init = if ty == HirType::Dynamic {
                        init
                    } else {
                        lowerer.coerce_to_declared(&ty, init)?
                    };
                    steps.push(HirInitStep::StoreGlobal(name.clone(), init));
                    lowerer.scope.insert(name.clone(), ty);
                    lowerer
                        .bindings
                        .entry(name.clone())
                        .or_default()
                        .push(name.clone());
                    if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                        lowerer.immutable_bindings.insert(name);
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                let class_name = declaration.ident.sym.as_ref();
                let saved_super = lowerer.super_initializer.clone();
                let saved_static_context = lowerer.class_static_context;
                let saved_class_context = lowerer.class_context.clone();
                lowerer.class_static_context = true;
                lowerer.class_context = Some(class_name.to_string());
                lowerer.super_initializer =
                    declaration
                        .class
                        .super_class
                        .as_ref()
                        .and_then(|base| match base.as_ref() {
                            Expr::Ident(base) => Some((
                                class_initializer_symbol(base.sym.as_ref()),
                                interfaces.get(base.sym.as_ref()).cloned().unwrap_or(HirType::Void),
                                base.sym.to_string(),
                            )),
                            _ => None,
                        });
                for member in &declaration.class.body {
                    if let ClassMember::StaticBlock(block) = member {
                        steps.extend(
                            lowerer
                                .lower_stmts(&block.body.stmts)?
                                .into_iter()
                                .map(HirInitStep::Statement),
                        );
                        continue;
                    }
                    let ClassMember::ClassProp(property) = member else {
                        continue;
                    };
                    if !property.is_static {
                        continue;
                    }
                    let field = class_property_name(&property.key)?;
                    let symbol = class_static_field_symbol(class_name, &field);
                    let expected = global_types[&symbol].clone();
                    let init = match property.value.as_deref() {
                        Some(value) => {
                            let init = lowerer.lower_expr(value)?;
                            lowerer.coerce_to_declared(&expected, init)?
                        }
                        None => match &expected {
                            HirType::Optional(payload) => {
                                HirExpr::OptionalNone(payload.as_ref().clone())
                            }
                            HirType::Nullish(payload) => {
                                HirExpr::NullishUndefined(payload.as_ref().clone())
                            }
                            _ => unreachable!("uninitialized static fields were validated above"),
                        },
                    };
                    steps.push(HirInitStep::StoreGlobal(symbol, init));
                }
                lowerer.super_initializer = saved_super;
                lowerer.class_static_context = saved_static_context;
                lowerer.class_context = saved_class_context;
            }
            ModuleItem::Stmt(Stmt::Decl(
                Decl::Fn(_)
                | Decl::TsInterface(_)
                | Decl::TsEnum(_)
                | Decl::TsTypeAlias(_)
                | Decl::TsModule(_),
            )) => {}
            ModuleItem::Stmt(statement) => {
                steps.extend(
                    lowerer
                        .lower_stmt_seq(statement)?
                        .into_iter()
                        .map(HirInitStep::Statement),
                );
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }
    Ok(steps)
}

#[allow(clippy::too_many_arguments)]
fn lower_global_decls(
    declarations: &[&VarDecl],
    global_types: &HashMap<Symbol, HirType>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<Vec<crate::HirGlobal>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        call_constraints,
    );
    for (name, ty) in global_types {
        if is_class_static_field_symbol(name) {
            lowerer.scope.insert(name.clone(), ty.clone());
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
        }
    }
    let mut globals = Vec::new();
    for declaration in declarations {
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                unreachable!("top-level patterns were validated above")
            };
            let name = binding.id.sym.to_string();
            let init = lowerer.lower_expr(
                declarator
                    .init
                    .as_deref()
                    .expect("top-level initializers were validated above"),
            )?;
            let expected = global_types[&name].clone();
            let inferred = lowerer.infer_expr_type(&init)?;
            let ty = if expected == HirType::Dynamic {
                inferred
            } else {
                expected
            };
            let init = if ty == HirType::Dynamic {
                init
            } else {
                lowerer.coerce_to_declared(&ty, init)?
            };
            let scope_type = ty.clone();
            globals.push(crate::HirGlobal {
                name: name.clone(),
                ty,
                init,
                mutable: declaration.kind != swc_ecma_ast::VarDeclKind::Const,
            });
            lowerer.scope.insert(name.clone(), scope_type);
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
            if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                lowerer.immutable_bindings.insert(name);
            }
        }
    }
    Ok(globals)
}

#[allow(clippy::too_many_arguments)]
fn lower_static_class_globals(
    declarations: &[&ClassDecl],
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
) -> Result<Vec<crate::HirGlobal>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        None,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    let mut globals = Vec::new();
    for declaration in declarations {
        let class_name = declaration.ident.sym.as_ref();
        let saved_super = lowerer.super_initializer.clone();
        let saved_static_context = lowerer.class_static_context;
        let saved_class_context = lowerer.class_context.clone();
        lowerer.class_static_context = true;
        lowerer.class_context = Some(class_name.to_string());
        lowerer.super_initializer =
            declaration
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some((
                        class_initializer_symbol(base.sym.as_ref()),
                        interfaces.get(base.sym.as_ref()).cloned().unwrap_or(HirType::Void),
                        base.sym.to_string(),
                    )),
                    _ => None,
                });
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if !property.is_static {
                continue;
            }
            let field = class_property_name(&property.key)?;
            let symbol = class_static_field_symbol(class_name, &field);
            let ty = global_types[&symbol].clone();
            let init = match property.value.as_deref() {
                Some(value) => {
                    let init = lowerer.lower_expr(value)?;
                    lowerer.coerce_to_declared(&ty, init)?
                }
                None => match &ty {
                    HirType::Optional(payload) => HirExpr::OptionalNone(payload.as_ref().clone()),
                    HirType::Nullish(payload) => {
                        HirExpr::NullishUndefined(payload.as_ref().clone())
                    }
                    _ => unreachable!("uninitialized static fields were validated above"),
                },
            };
            globals.push(crate::HirGlobal {
                name: symbol.clone(),
                ty,
                init,
                mutable: !immutable_globals.contains(&symbol),
            });
        }
        lowerer.super_initializer = saved_super;
        lowerer.class_static_context = saved_static_context;
        lowerer.class_context = saved_class_context;
    }
    Ok(globals)
}
