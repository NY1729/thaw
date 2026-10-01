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
        if let ModuleItem::Stmt(Stmt::Expr(statement)) = item {
            if statement.span == swc_common::DUMMY_SP {
                if let Expr::Lit(Lit::Str(value)) = statement.expr.as_ref() {
                    if let Some(mode) = value.value.as_str().and_then(|value| value.strip_prefix("__thaw_internal_execution:")) {
                        let execute = match mode {
                            "0" => false,
                            "1" => true,
                            _ => return Err("invalid internal execution boundary".into()),
                        };
                        steps.push(HirInitStep::ExecutionBoundary(execute));
                        continue;
                    }
                    if let Some(encoded) = value.value.as_str().and_then(|value| value.strip_prefix("__thaw_internal_module:")) {
                        let fields = encoded.split(':').collect::<Vec<_>>();
                        if fields.len() != 4 {
                            return Err("invalid internal module boundary".into());
                        }
                        let index = fields[0].parse().map_err(|_| "invalid internal module index")?;
                        let eager = fields[1] == "1";
                        let runtime = fields[2] == "1";
                        let static_dependencies = if fields[3].is_empty() {
                            Vec::new()
                        } else {
                            fields[3].split(',').map(str::parse).collect::<Result<Vec<_>, _>>()
                                .map_err(|_| "invalid internal module dependency")?
                        };
                        steps.push(HirInitStep::ModuleBoundary { index, eager, runtime, static_dependencies });
                        continue;
                    }
                }
            }
        }
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
                    let token_value = match class_value_js_script(declaration) {
                        Some((global, script)) => {
                            // Register one native bridge per instance method
                            // before evaluating the class, so the generated
                            // JavaScript method bodies (which delegate to
                            // these globals) can reach the native
                            // implementations -- including the native
                            // functions those bodies call.
                            for (property, _kind, symbol) in class_bridgeable_members(declaration) {
                                let bridge = class_method_bridge_expr(&mut lowerer, &symbol)?;
                                let name = class_method_bridge_name(class_name, &property);
                                steps.push(HirInitStep::Statement(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var("setDynamicProperty".to_string())),
                                    vec![
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                            vec![HirExpr::Lit(HirLit::Str("globalThis".into()))],
                                        ),
                                        HirExpr::Lit(HirLit::Str(name)),
                                        bridge,
                                    ],
                                ))));
                            }
                            steps.push(HirInitStep::Statement(HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("loadScript".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(script))],
                            ))));
                            HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(global))],
                            )
                        }
                        None => class_decorator_token_init(&mut lowerer)?,
                    };
                    steps.push(HirInitStep::StoreGlobal(token_symbol.clone(), token_value));
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

/// Removes every decorator (class, member, parameter) from a cloned class
/// so its JavaScript codegen can be evaluated by QuickJS, which has no
/// decorator syntax. thaw applies decorators natively.
fn strip_class_decorators(class: &mut swc_ecma_ast::Class) {
    struct Stripper;
    impl swc_ecma_visit::VisitMut for Stripper {
        fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
            class.decorators.clear();
            class.visit_mut_children_with(self);
        }
        fn visit_mut_class_member(&mut self, member: &mut swc_ecma_ast::ClassMember) {
            match member {
                swc_ecma_ast::ClassMember::Method(method) => {
                    method.function.decorators.clear();
                }
                swc_ecma_ast::ClassMember::PrivateMethod(method) => {
                    method.function.decorators.clear();
                }
                swc_ecma_ast::ClassMember::ClassProp(property) => property.decorators.clear(),
                swc_ecma_ast::ClassMember::PrivateProp(property) => property.decorators.clear(),
                _ => {}
            }
            member.visit_mut_children_with(self);
        }
        fn visit_mut_param(&mut self, param: &mut swc_ecma_ast::Param) {
            param.decorators.clear();
            param.visit_mut_children_with(self);
        }
    }
    class.visit_mut_with(&mut Stripper);
}

/// The realm global a bridged method's generated JavaScript body calls to
/// reach the real native implementation (see `class_method_bridge_expr`).
fn class_method_bridge_name(class_name: &str, property: &str) -> String {
    format!("__thaw_method_bridge_{class_name}_{property}")
}

/// The instance methods/getters/setters of a class that get a native bridge
/// -- sync, non-static ones (an async/generator body can't be written as a
/// synchronous delegation). Returns `(property, kind, native symbol)`.
fn class_bridgeable_members(
    declaration: &swc_ecma_ast::ClassDecl,
) -> Vec<(Symbol, swc_ecma_ast::MethodKind, Symbol)> {
    let class_name = declaration.ident.sym.as_ref();
    let mut members = Vec::new();
    for member in &declaration.class.body {
        let swc_ecma_ast::ClassMember::Method(method) = member else {
            continue;
        };
        if method.is_static || method.function.is_async || method.function.is_generator {
            continue;
        }
        let Ok(property) = class_property_name(&method.key) else {
            continue;
        };
        let Ok(symbol) = class_member_symbol(class_name, method) else {
            continue;
        };
        members.push((property, method.kind, symbol));
    }
    members
}

/// Whether a native value of `ty` supports the `===` used for write-back
/// change detection. Scalar and pointer-shaped types do (value or pointer
/// compare); an `Optional`/`Nullable`/`Nullish`/`Union`'s tag+payload layout
/// has no single LLVM value `===` can compare, so those fields are written
/// back unconditionally instead.
fn bridge_field_comparable(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::F64
            | HirType::I64
            | HirType::Bool
            | HirType::Str
            | HirType::Symbol
            | HirType::Null
            | HirType::Undefined
            | HirType::Json
            | HirType::Object(_)
            | HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Dictionary(_)
            | HirType::Function(_, _)
            | HirType::CallableFunction(_, _, _, _)
            // Tagged-absence wrappers and unions compare through the
            // equality lowering's dedicated nodes (see
            // `lower_optional_undefined_equality`), not a raw `===`.
            | HirType::Optional(_)
            | HirType::Nullable(_)
            | HirType::Nullish(_)
            | HirType::Union(_)
    )
}

/// Builds the native bridge for one method. The generated JavaScript body
/// calls it as `bridge(this, this, ...args)`: the first `this` is decoded by
/// the `registerNativeCallback` adapter into a native receiver copy for the
/// call, the second is the live JS handle. After the call, each non-identity
/// receiver field that actually changed (see `bridge_field_comparable`) is
/// written back onto that handle (`setDynamicPropertyJson`), so a native
/// method's mutations to `this` are observable from the JavaScript object --
/// an untouched field is left exactly as it was.
///
/// A field with no JSON encoding is skipped rather than failing the whole
/// bridge.
fn class_method_bridge_expr(
    lowerer: &mut FnLowerer<'_>,
    symbol: &Symbol,
) -> Result<HirExpr, String> {
    let signature = lowerer
        .signatures
        .get(symbol)
        .cloned()
        .ok_or_else(|| format!("bridge target `{symbol}` is not declared"))?;
    let Some((receiver_type, method_params)) = signature.params.split_first() else {
        return Err(format!("bridge target `{symbol}` has no receiver"));
    };
    let receiver_type = receiver_type.clone();

    let self_name: Symbol = "__thaw_bridge_self".to_string();
    let handle_name: Symbol = "__thaw_bridge_handle".to_string();
    let mut params = vec![
        HirParam {
            name: self_name.clone(),
            ty: receiver_type.clone(),
        },
        HirParam {
            name: handle_name.clone(),
            ty: HirType::JsValue,
        },
    ];
    let mut call_args = vec![HirExpr::Var(self_name.clone())];
    for (index, ty) in method_params.iter().enumerate() {
        let name = format!("__thaw_bridge_arg_{index}");
        params.push(HirParam {
            name: name.clone(),
            ty: ty.clone(),
        });
        call_args.push(HirExpr::Var(name));
    }
    // The lambda's own parameters are only added to the type scope when the
    // `HirExpr::Lambda` node itself is inferred, but the equality helper
    // below infers eagerly while the body is being built -- register them up
    // front so `self`/the handle/args resolve.
    for param in &params {
        lowerer.scope.insert(param.name.clone(), param.ty.clone());
    }

    let native_call = HirExpr::Call(Box::new(HirExpr::Var(symbol.clone())), call_args);
    let result_name: Symbol = "__thaw_bridge_result".to_string();

    // Snapshot every encodable receiver field *before* the call, then write
    // it back only if it actually changed. An untouched reference field is
    // left alone, so the JavaScript object keeps the exact value (and
    // identity) it had -- reassigning every field unconditionally used to
    // replace an untouched object with a fresh copy and an untouched
    // function with a native-callback wrapper. A `===` on a native
    // reference type compiles to a pointer compare, `Str`/`Symbol` to a
    // content compare, and a number to a value compare (see
    // `operators.rs`'s `EqEqEq`), so this detects a direct reassignment and
    // a change made through a nested native call (`this.inc()`), while a
    // field with no reliable `===` (an `Optional`/`Union` tag+payload) is
    // still written back unconditionally.
    let mut pre_stmts = Vec::new();
    let mut post_stmts = Vec::new();
    if let HirType::Object(fields) = &receiver_type {
        for (index, (field, field_type)) in fields.iter().enumerate() {
            if field.starts_with("__thaw_class_identity_") {
                continue;
            }
            let current = HirExpr::PropAccess(
                Box::new(HirExpr::Var(self_name.clone())),
                receiver_type.clone(),
                field.clone(),
            );
            let Ok(encoded) = lowerer.coerce_to_declared(&HirType::Json, current.clone()) else {
                continue;
            };
            let write_back = HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("setDynamicPropertyJson".to_string())),
                vec![
                    HirExpr::Var(handle_name.clone()),
                    HirExpr::Lit(HirLit::Str(field.clone())),
                    encoded,
                ],
            ));
            if !bridge_field_comparable(field_type) {
                post_stmts.push(write_back);
                continue;
            }
            let previous: Symbol = format!("__thaw_bridge_previous_{index}");
            lowerer.scope.insert(previous.clone(), field_type.clone());
            pre_stmts.push(HirStmt::Let(
                previous.clone(),
                field_type.clone(),
                current.clone(),
            ));
            // Prefer the equality lowering's own node for tagged types
            // (`Optional`/`Nullable`/`Nullish`/`Union`); a raw `===` can't
            // compare their tag+payload layout. Scalars/references fall back
            // to the direct `===` (value/pointer compare).
            let equal = match lowerer.lower_optional_undefined_equality(
                HirExpr::Var(previous.clone()),
                current.clone(),
            )? {
                Some(equal) => equal,
                None => HirExpr::BinOp(
                    crate::BinOp::EqEqEq,
                    Box::new(HirExpr::Var(previous)),
                    Box::new(current),
                ),
            };
            post_stmts.push(HirStmt::If(equal, Vec::new(), vec![write_back]));
        }
    }

    let mut stmts = pre_stmts;
    if signature.ret == HirType::Void {
        stmts.push(HirStmt::Expr(native_call));
    } else {
        stmts.push(HirStmt::Let(
            result_name.clone(),
            signature.ret.clone(),
            native_call,
        ));
    }
    stmts.extend(post_stmts);
    stmts.push(HirStmt::Return(if signature.ret == HirType::Void {
        None
    } else {
        Some(HirExpr::Var(result_name))
    }));

    let body = HirExpr::Block(stmts);
    let lambda = HirExpr::Lambda(Vec::new(), params.clone(), signature.ret.clone(), Box::new(body));
    let callback_type = HirType::Function(
        params.iter().map(|param| param.ty.clone()).collect(),
        Box::new(signature.ret.clone()),
    );
    Ok(HirExpr::Call(
        Box::new(HirExpr::Var("registerNativeCallback".to_string())),
        vec![HirExpr::TypedClosure(callback_type, Box::new(lambda))],
    ))
}

/// Builds a JavaScript method body that delegates to the native method's
/// realm-registered bridge: `globalThis["<bridge>"](this, ...arguments)`
/// (a getter passes only `this`, a setter drops the result).
fn method_delegate_body(
    bridge: &str,
    kind: swc_ecma_ast::MethodKind,
) -> swc_ecma_ast::FunctionBody {
    use swc_common::DUMMY_SP;
    let global_this = swc_ecma_ast::Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
        "globalThis".into(),
        DUMMY_SP,
    ));
    let callee = swc_ecma_ast::Expr::Member(swc_ecma_ast::MemberExpr {
        span: DUMMY_SP,
        obj: Box::new(global_this),
        prop: swc_ecma_ast::MemberProp::Computed(swc_ecma_ast::ComputedPropName {
            span: DUMMY_SP,
            expr: Box::new(swc_ecma_ast::Expr::Lit(swc_ecma_ast::Lit::Str(
                swc_ecma_ast::Str {
                    span: DUMMY_SP,
                    value: bridge.into(),
                    raw: None,
                },
            ))),
        }),
    });
    let this_arg = || swc_ecma_ast::ExprOrSpread {
        spread: None,
        expr: Box::new(swc_ecma_ast::Expr::This(swc_ecma_ast::ThisExpr { span: DUMMY_SP })),
    };
    // `this` twice: the first is decoded into a native receiver copy for the
    // call, the second is the live handle the bridge writes mutations back to.
    let mut args = vec![this_arg(), this_arg()];
    if !matches!(kind, swc_ecma_ast::MethodKind::Getter) {
        args.push(swc_ecma_ast::ExprOrSpread {
            spread: Some(DUMMY_SP),
            expr: Box::new(swc_ecma_ast::Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                "arguments".into(),
                DUMMY_SP,
            ))),
        });
    }
    let call = swc_ecma_ast::Expr::Call(swc_ecma_ast::CallExpr {
        span: DUMMY_SP,
        ctxt: Default::default(),
        callee: swc_ecma_ast::Callee::Expr(Box::new(callee)),
        args,
        type_args: None,
    });
    let stmt = if matches!(kind, swc_ecma_ast::MethodKind::Setter) {
        swc_ecma_ast::Stmt::Expr(swc_ecma_ast::ExprStmt {
            span: DUMMY_SP,
            expr: Box::new(call),
        })
    } else {
        swc_ecma_ast::Stmt::Return(swc_ecma_ast::ReturnStmt {
            span: DUMMY_SP,
            arg: Some(Box::new(call)),
        })
    };
    swc_ecma_ast::FunctionBody {
        span: DUMMY_SP,
        stmts: vec![stmt],
    }
}

/// Replaces each bridgeable instance method's JavaScript body with a
/// delegation to its native bridge (see `method_delegate_body`), so a
/// dynamically-called method reaches the real native implementation --
/// including any native functions its body calls.
fn rewrite_instance_methods_to_bridges(class: &mut swc_ecma_ast::Class, class_name: &str) {
    for member in &mut class.body {
        let swc_ecma_ast::ClassMember::Method(method) = member else {
            continue;
        };
        if method.is_static || method.function.is_async || method.function.is_generator {
            continue;
        }
        let Ok(property) = class_property_name(&method.key) else {
            continue;
        };
        let bridge = class_method_bridge_name(class_name, &property);
        method.function.body = Some(method_delegate_body(&bridge, method.kind));
    }
}

/// Emits a bare ECMAScript module (`class C { ... }`) as JavaScript source.
fn emit_module_js(module: &swc_ecma_ast::Module) -> Option<String> {
    use swc_common::sync::Lrc;
    use swc_ecma_codegen::{text_writer::JsWriter, Config, Emitter};
    let source_map: Lrc<swc_common::SourceMap> = Default::default();
    let mut buffer = Vec::new();
    {
        let writer = JsWriter::new(source_map.clone(), "\n", &mut buffer, None);
        let mut emitter = Emitter {
            cfg: Config::default(),
            cm: source_map,
            comments: None,
            wr: writer,
        };
        emitter.emit_module(module).ok()?;
    }
    String::from_utf8(buffer).ok()
}

/// The user-facing name of a class for JavaScript codegen: strips the
/// module-flattening prefix (`__thawmod<N>_`, added to every top-level
/// declaration for cross-file uniqueness) so the evaluated class reports its
/// real `name` (`A`, not `__thawmod0_A`).
fn display_class_name(class_name: &str) -> &str {
    if let Some(rest) = class_name.strip_prefix("__thawmod") {
        if let Some((digits, original)) = rest.split_once('_') {
            if !digits.is_empty()
                && digits.chars().all(|character| character.is_ascii_digit())
                && !original.is_empty()
            {
                return original;
            }
        }
    }
    class_name
}

/// Renders one class as JavaScript with its TypeScript-only syntax
/// (annotations, `readonly`/`declare`/accessibility modifiers, type params)
/// stripped, for evaluation by QuickJS. Decorators are dropped (applied
/// natively instead). Returns `None` if the class can't be codegen'd, in
/// which case the caller falls back to a `new Function()` token.
fn class_value_js_source(declaration: &swc_ecma_ast::ClassDecl) -> Option<String> {
    let mut class = declaration.class.clone();
    strip_class_decorators(&mut class);
    rewrite_instance_methods_to_bridges(&mut class, declaration.ident.sym.as_ref());
    let mut class_ident = declaration.ident.clone();
    class_ident.sym = display_class_name(declaration.ident.sym.as_ref()).into();
    let class_decl = swc_ecma_ast::ClassDecl {
        ident: class_ident,
        declare: false,
        class,
    };
    let module = swc_ecma_ast::Module {
        span: swc_common::DUMMY_SP,
        body: vec![swc_ecma_ast::ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(
            swc_ecma_ast::Decl::Class(class_decl),
        ))],
        shebang: None,
    };
    let mut program = swc_ecma_ast::Program::Module(module);
    // `Mark::new()` reads swc's scoped `GLOBALS` TLS, which the parser sets
    // during parsing but lowering has since left; open a fresh scope for
    // the strip pass.
    let globals = swc_common::Globals::new();
    swc_common::GLOBALS.set(&globals, || {
        let mut pass = swc_ecma_transforms_typescript::strip(
            swc_common::Mark::new(),
            swc_common::Mark::new(),
        );
        use swc_ecma_ast::Pass;
        pass.process(&mut program);
    });
    let swc_ecma_ast::Program::Module(module) = program else {
        return None;
    };
    emit_module_js(&module)
}

/// Builds the class's own real JavaScript class (TypeScript stripped) and
/// the realm global name it is stored under, plus the `loadScript` source
/// that defines it. The definition is wrapped so any evaluation failure
/// (e.g. an `extends` of another native-only class, or a static initializer
/// touching a native helper) degrades to an empty `Function()` token instead
/// of aborting module init. Returns `None` if codegen isn't possible, in
/// which case the caller keeps the plain `new Function()` token.
///
/// ponytail: method *bodies* are the class's real JavaScript, so a method
/// that calls a native-only function (an import, another compiled function)
/// will fail when invoked dynamically -- only `this`/built-in logic works.
/// Static `instance.method()` still uses the native method, unaffected.
fn class_value_js_script(declaration: &swc_ecma_ast::ClassDecl) -> Option<(String, String)> {
    let source = class_value_js_source(declaration)?;
    let class_name = declaration.ident.sym.as_ref();
    let display = display_class_name(class_name);
    let global = format!("__thaw_class_value_{class_name}");
    let script = format!(
        "globalThis[\"{global}\"] = (function () {{ try {{ {source}\nreturn {display}; }} catch (e) {{ return Function(); }} }})();"
    );
    Some((global, script))
}

/// Placeholder `HirGlobal.init` for a class token. AOT codegen never reads a
/// `HirGlobal`'s own init -- the real value comes from the
/// `HirInitStep::StoreGlobal` in `lower_top_level_initializers` (see the
/// comment there), which stores the evaluated class (`class_value_js_script`)
/// when available. This `constructDynamicValue` expression is only a
/// type-correct `JsValue` fallback.
fn class_decorator_token_init(lowerer: &mut FnLowerer<'_>) -> Result<HirExpr, String> {
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

/// Gives every class that is used dynamically (decorated, referenced as a
/// bare value, or constructed) one real, live `JsValue` "class token" --
/// a genuine distinct QuickJS object -- stored as an ordinary global
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
