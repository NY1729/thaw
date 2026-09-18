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
    value_referenced_classes: &HashSet<Symbol>,
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
                if class_has_decorators(&declaration.class)
                    || value_referenced_classes.contains(class_name)
                {
                    let token_symbol = class_decorator_token_symbol(class_name);
                    // `lower_class_decorator_tokens` also emits this same
                    // class token as a `crate::HirGlobal` (needed so
                    // `declare_globals`, thaw-llvm, allocates it real
                    // storage) but thaw-llvm's AOT codegen never reads a
                    // `HirGlobal`'s own `.init` -- every global's *real*
                    // first value comes from an `HirInitStep::StoreGlobal`
                    // here in `initializers` instead (see
                    // `emit_top_level_init`), the same as an ordinary
                    // top-level `const`/static field. Skipping this would
                    // leave the token's storage permanently zero (an
                    // invalid `JsValue` handle) the moment anything reads
                    // it.
                    steps.push(HirInitStep::StoreGlobal(
                        token_symbol.clone(),
                        class_decorator_token_init(&mut lowerer)?,
                    ));
                    // Only a *decorated* class actually has decorators to
                    // invoke; a merely value-referenced one just needs the
                    // token above.
                    if class_has_decorators(&declaration.class) {
                        for member in &declaration.class.body {
                            let (decorators, key) = match member {
                                ClassMember::ClassProp(property) => {
                                    (&property.decorators, class_property_name(&property.key)?)
                                }
                                ClassMember::Method(method)
                                    if method.kind == MethodKind::Method =>
                                {
                                    (
                                        &method.function.decorators,
                                        class_property_name(&method.key)?,
                                    )
                                }
                                _ => continue,
                            };
                            for decorator in decorators {
                                let call = lower_member_decorator_call(
                                    &mut lowerer,
                                    decorator,
                                    &token_symbol,
                                    &key,
                                )?;
                                steps.push(HirInitStep::Statement(HirStmt::Expr(call)));
                            }
                        }
                        for decorator in &declaration.class.decorators {
                            let call = lower_class_decorator_call(
                                &mut lowerer,
                                decorator,
                                &token_symbol,
                            )?;
                            steps.push(HirInitStep::Statement(HirStmt::Expr(call)));
                        }
                    }
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

/// Builds `new Function()` (via the same `getDynamicValue`/
/// `constructDynamicValue` pair `new Intl.DateTimeFormat(...)` etc. use) --
/// a genuine, distinct, live QuickJS object with no ties to thaw's own
/// (non-existent) prototype-chain machinery, used as a decorated class's
/// "class token" (see `lower_class_decorator_tokens`/
/// `class_decorator_token_symbol`).
fn class_decorator_token_init(lowerer: &mut FnLowerer) -> Result<HirExpr, String> {
    let constructor = HirExpr::Call(
        Box::new(HirExpr::Var("getDynamicValue".to_string())),
        vec![HirExpr::Lit(HirLit::Str("Function".to_string()))],
    );
    let no_args = lowerer.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(Vec::new()))?;
    Ok(HirExpr::Call(
        Box::new(HirExpr::Var("constructDynamicValue".to_string())),
        vec![constructor, no_args],
    ))
}

/// Gives every decorated class (`class_has_decorators`) one real, live
/// `JsValue` "class token" -- literally `new Function()`, a genuine
/// distinct QuickJS object -- stored as an ordinary global
/// (`class_decorator_token_symbol`). A decorator's `target` argument (see
/// `lower_class_decorator_call`/`lower_member_decorator_call` below) is
/// built from this same token, and so is the `constructor` field
/// `coerce_to_declared`'s `HirType::Json` branch now adds when marshaling
/// an instance of that same class across a dynamic-call boundary
/// (`inference/coercions.rs`) -- giving real code like class-validator's
/// `object.constructor`-keyed metadata storage a genuinely stable,
/// shared identity to key off, the way real Node's own class/prototype
/// objects would.
fn lower_class_decorator_tokens(
    declarations: &[&ClassDecl],
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    value_referenced_classes: &HashSet<Symbol>,
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
    let mut globals = Vec::new();
    for declaration in declarations {
        let class_name = declaration.ident.sym.as_ref();
        if !class_has_decorators(&declaration.class)
            && !value_referenced_classes.contains(class_name)
        {
            continue;
        }
        globals.push(crate::HirGlobal {
            name: class_decorator_token_symbol(class_name),
            ty: HirType::JsValue,
            init: class_decorator_token_init(&mut lowerer)?,
            mutable: false,
        });
    }
    Ok(globals)
}

/// A class's own `Ident` symbol is already `__thawmod{N}_`-prefixed by
/// `thaw-cli`'s module-flattening pass (every top-level declaration is
/// renamed for cross-file uniqueness, entry file included) by the time
/// thaw-hir ever sees it. A decorator receiving that raw compiler symbol
/// as "the class's name" would be a leak of an internal implementation
/// detail, not the source-level name real Node would hand it -- so strip
/// it back off for anything a decorator call actually observes.
/// A decorator (`@expr`) is a real function value, possibly produced by a
/// factory call (`@IsEmail()`); either way it is evaluated once and then
/// called with a real target identity plus (for member decorators) the
/// real property key, matching TC39/TS legacy decorator call order
/// (member decorators, top to bottom, then class decorators) and legacy
/// argument shape: a class decorator receives just its class's own
/// "class token" (`token_symbol`, see `lower_class_decorator_tokens`); a
/// property/method decorator receives that same token's `.prototype` --
/// a real object whose own `.constructor` is the token, exactly the
/// relationship real JS gives an instance member decorator's `target` --
/// plus the property key. Reusing `lower_call` here -- rather than a
/// bespoke dynamic-value dispatcher -- means a decorator that resolves to
/// a plain compiled function and one that resolves to a `JsValue`
/// imported from an npm package both get dispatched correctly for free.
///
/// A method/property decorator does not receive a `descriptor` argument:
/// thaw's natively-compiled methods have no first-class callable JS
/// representation to put in `descriptor.value`, and hand-waving a
/// non-functional placeholder object would silently break any decorator
/// that actually calls it. Decorators that only register metadata by
/// target+key (the pattern class-validator/TypeORM/NestJS's own decorators
/// use) work regardless -- and, since `coerce_to_declared` gives an
/// instance of this same class a `constructor` field pointing at this
/// exact token when it crosses a dynamic-call boundary, `object.
/// constructor`-keyed metadata (real class-validator's own storage key)
/// round-trips correctly too.
fn lower_member_decorator_call(
    lowerer: &mut FnLowerer,
    decorator: &Decorator,
    token_symbol: &str,
    property_key: &str,
) -> Result<HirExpr, String> {
    lower_decorator_call(
        lowerer,
        decorator,
        vec![
            decorator_token_prototype_arg(decorator.span, token_symbol),
            decorator_string_arg(decorator.span, property_key),
        ],
    )
}

fn lower_class_decorator_call(
    lowerer: &mut FnLowerer,
    decorator: &Decorator,
    token_symbol: &str,
) -> Result<HirExpr, String> {
    lower_decorator_call(
        lowerer,
        decorator,
        vec![decorator_token_arg(decorator.span, token_symbol)],
    )
}

fn lower_decorator_call(
    lowerer: &mut FnLowerer,
    decorator: &Decorator,
    args: Vec<swc_ecma_ast::ExprOrSpread>,
) -> Result<HirExpr, String> {
    lowerer.lower_call(&CallExpr {
        span: decorator.span,
        ctxt: Default::default(),
        callee: Callee::Expr(Box::new(decorator.expr.as_ref().clone())),
        args,
        type_args: None,
    })
}

fn decorator_token_ident(span: swc_common::Span, token_symbol: &str) -> Expr {
    Expr::Ident(swc_ecma_ast::Ident {
        span,
        ctxt: Default::default(),
        sym: token_symbol.into(),
        optional: false,
    })
}

fn decorator_token_arg(span: swc_common::Span, token_symbol: &str) -> swc_ecma_ast::ExprOrSpread {
    swc_ecma_ast::ExprOrSpread {
        spread: None,
        expr: Box::new(decorator_token_ident(span, token_symbol)),
    }
}

fn decorator_token_prototype_arg(
    span: swc_common::Span,
    token_symbol: &str,
) -> swc_ecma_ast::ExprOrSpread {
    swc_ecma_ast::ExprOrSpread {
        spread: None,
        expr: Box::new(Expr::Member(MemberExpr {
            span,
            obj: Box::new(decorator_token_ident(span, token_symbol)),
            prop: MemberProp::Ident(IdentName::new("prototype".into(), span)),
        })),
    }
}

fn decorator_string_arg(span: swc_common::Span, value: &str) -> swc_ecma_ast::ExprOrSpread {
    swc_ecma_ast::ExprOrSpread {
        spread: None,
        expr: Box::new(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
            span,
            value: value.into(),
            raw: None,
        }))),
    }
}
