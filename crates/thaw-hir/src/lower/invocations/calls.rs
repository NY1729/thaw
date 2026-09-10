impl<'a> FnLowerer<'a> {
    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        // Taken (not just read) as the very first action, before this
        // call's own arguments are lowered below -- a hint set for *this*
        // call must never still be visible if lowering one of its own
        // arguments recurses into another `lower_call` for a nested call
        // expression, which would otherwise see and wrongly consume a
        // hint meant for its parent. See `lower_expr_with_expected_type`.
        let expected_return_hint = self.expected_return_hint.take();
        if matches!(call.callee, Callee::Super(_)) {
            let (mut symbol, _base_type, base_name) = self
                .super_initializer
                .clone()
                .ok_or("`super(...)` is only valid in a derived class constructor")?;
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`), so there is no base initializer
            // function to call -- `super(message)` for a class directly
            // extending one of them instead assigns the inherited
            // `message` field itself. A class extending *that* class still
            // goes through the normal call-the-base-initializer path below
            // (its own generated initializer function already runs this
            // assignment as part of its body), so only a *direct* `extends
            // Error`-family base needs this.
            if matches!(
                base_name.as_str(),
                "Error"
                    | "TypeError"
                    | "RangeError"
                    | "SyntaxError"
                    | "ReferenceError"
                    | "EvalError"
                    | "URIError"
            ) {
                if call.type_args.is_some() {
                    return Err("native `super(...)` does not support type arguments".into());
                }
                let [message] = call.args.as_slice() else {
                    return Err(format!("`super(...)` for `{base_name}` expects a message argument"));
                };
                if message.spread.is_some() {
                    return Err("`super(...)` does not support a spread argument".into());
                }
                let message = self.lower_expr(&message.expr)?;
                let message = self.coerce_primitive_to_string(message)?;
                let this_name = self.resolve_binding("this");
                let this_type = self
                    .scope
                    .get(&this_name)
                    .cloned()
                    .ok_or("`super(...)` used outside a constructor")?;
                return Ok(HirExpr::PropAssign(
                    Box::new(HirExpr::Var(this_name)),
                    this_type,
                    "message".to_string(),
                    Box::new(message),
                ));
            }
            let mut signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .ok_or_else(|| format!("missing base class initializer `{symbol}`"))?;
            if call.type_args.is_some() {
                return Err("native `super(...)` does not support type arguments".into());
            }
            let this_name = self.resolve_binding("this");
            let mut args = vec![HirExpr::Var(this_name)];
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "base constructor")?;
            let arguments = self.select_omitted_class_arguments(
                &mut symbol,
                &mut signature,
                arguments,
                1,
                "base constructor",
            )?;
            args.extend(arguments);
            let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
            return self.wrap_call_argument_bindings(result, &bindings);
        }
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::SuperProp(member) = callee.as_ref() {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` member access is only valid in a derived class")?;
                let method_name = match &member.prop {
                    SuperProp::Ident(name) => name.sym.to_string(),
                    SuperProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                        _ => {
                            return Err(
                                "computed super methods require a string literal name".into()
                            )
                        }
                    },
                };
                let mut symbol = if self.class_static_context {
                    class_static_method_symbol(&base_name, &method_name)
                } else {
                    class_method_symbol(&base_name, &method_name)
                };
                let mut signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no method `{method_name}`")
                })?;
                if call.type_args.is_some() {
                    return Err("native super methods do not support type arguments".into());
                }
                let receiver_count = usize::from(!self.class_static_context);
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                let label = format!("super method `{base_name}.{method_name}`");
                let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                let arguments = self.select_omitted_class_arguments(
                    &mut symbol,
                    &mut signature,
                    arguments,
                    receiver_count,
                    &label,
                )?;
                args.extend(arguments);
                let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        if let Some(invoked) = self.lower_saved_native_method_call_or_apply(call)? {
            return Ok(invoked);
        }
        if self.unbound_this_context {
            if let Expr::Member(member) = callee_expr.as_ref() {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` method call requires a statically known property")?;
                    let HirType::Function(_, result) = self
                        .unbound_this_member_type(&property)
                        .ok_or_else(|| format!("class has no native method `{property}`"))?
                    else {
                        return Err(format!("native member `{property}` is not callable"));
                    };
                    return self.lower_unbound_this_error(&property, &result);
                }
            }
        }

        if let Some(invoked) = self.lower_immediately_invoked_class_bind(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_immediately_invoked_function_bind(call)? {
            return Ok(invoked);
        }

        // Expressions that produce typed functions are callable through the
        // same closure ABI as locals. In particular, support `make(x)(y)`.
        if !matches!(callee_expr.as_ref(), Expr::Ident(_) | Expr::Member(_)) {
            if call.type_args.is_some() {
                return Err("function-value invocation does not accept type arguments".into());
            }
            let callee = self.lower_expr(callee_expr)?;
            let (params, optional, rest) = match self.infer_expr_type(&callee)? {
                HirType::Function(params, _) => (params, HirOptionalMask::default(), None),
                HirType::CallableFunction(params, optional, rest, _) => {
                    (params, optional, rest.map(|element| *element))
                }
                _ => return Err("call target is not a function value".into()),
            };
            let (mut arguments, bindings) =
                self.lower_native_spread_values(&call.args, "function-value invocation")?;
            if rest.is_none() && arguments.len() > params.len() {
                return Err(format!(
                    "function value expects {} argument(s), got {}",
                    params.len(),
                    arguments.len()
                ));
            }
            if arguments.len() < params.len()
                && (arguments.len()..params.len())
                    .any(|index| !is_optional_parameter(&optional, index))
            {
                return Err(format!(
                    "function value expects at least {} argument(s), got {}",
                    params.len(),
                    arguments.len()
                ));
            }
            let rest_values = rest.as_ref().map(|element| {
                let values = arguments.split_off(params.len());
                (element, values)
            });
            for parameter in params.iter().skip(arguments.len()) {
                arguments.push(omitted_parameter_value(parameter)?);
            }
            let mut arguments = arguments
                .into_iter()
                .zip(&params)
                .map(|(argument, expected)| self.coerce_to_declared(expected, argument))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some((element, values)) = rest_values {
                let values = values
                    .into_iter()
                    .map(|value| self.coerce_to_declared(element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                arguments.push(native_rest_array(values, element));
            }
            return self.wrap_call_argument_bindings(
                HirExpr::Call(Box::new(callee), arguments),
                &bindings,
            );
        }

        if let Some(bound) = self.lower_native_class_bind(call)? {
            return Ok(bound);
        }
        if let Some(bound) = self.lower_native_static_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_native_class_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_native_static_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_this_parameter_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(bound) = self.lower_function_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_function_call_or_apply(call)? {
            return Ok(invoked);
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let Some(property) = member_property_name(&member.prop) {
                if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                    let class_name = self
                        .class_context
                        .as_deref()
                        .expect("static class lowering retains its class context");
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                if let Expr::Ident(class) = member.obj.as_ref() {
                    let class_name = class.sym.as_ref();
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                let receiver_type = self.infer_member_receiver_type(&member.obj);
                if let Some(class_receiver_type) =
                    receiver_type.clone().filter(|ty| class_name_from_type(ty).is_some())
                {
                    let class_name = class_name_from_type(&class_receiver_type)
                        .expect("the receiver was classified as a native class");
                    let symbol = class_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native class method `{class_name}.{property}` is not generic"
                            ));
                        }
                        let mut args = Vec::with_capacity(call.args.len() + 1);
                        args.push(swc_ecma_ast::ExprOrSpread {
                            spread: None,
                            expr: member.obj.clone(),
                        });
                        args.extend(call.args.iter().cloned());
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args,
                            type_args: None,
                        });
                    }
                    return Err(format!(
                        "class `{class_name}` has no native method `{property}`"
                    ));
                }
                // A receiver typed `JsValue` (an opaque handle -- a
                // Fallback function's return value that couldn't be
                // classified as anything JSON-representable, e.g. zod's
                // `z.object(...)` returning a live `ZodObject` schema
                // instance) has no known class/method table the way the
                // branch above needs, but calling a method on it by name
                // is still meaningful: the value really is a live JS
                // object on the other side of the QuickJS boundary, it's
                // just one thaw has no compiled knowledge of the shape
                // of. Real example: that same schema's own
                // `.safeParse(data)`. Routes through `callDynamicMethod`
                // (an existing low-level intrinsic, previously only a
                // manual escape hatch a user could call directly)
                // instead of erroring the way an unrecognized *class*
                // method above does, since there's no fixed method list
                // to have missed from.
                if matches!(receiver_type, Some(HirType::JsValue)) {
                    return self.lower_dynamic_value_method_call(
                        &member.obj,
                        &property,
                        &call.args,
                        expected_return_hint.as_ref(),
                    );
                }
            }
        }

        if matches!(callee_expr.as_ref(), Expr::Ident(identifier) if identifier.sym == *"__thaw_object_rest")
        {
            if call.args.is_empty() || call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err("object-rest lowering expects a source and static field names".into());
            }
            let source = self.lower_expr(&call.args[0].expr)?;
            let source_type = self.infer_expr_type(&source)?;
            if let HirType::Dictionary(element) = &source_type {
                let omitted = call.args[1..]
                    .iter()
                    .map(|argument| {
                        let key = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_string(key)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let copy = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                    vec![HirExpr::JsonObjectLit(Vec::new(), element.as_ref().clone()), source],
                );
                let parameter = format!("__thaw_object_rest_copy_{}", self.next_binding);
                self.next_binding += 1;
                let mut parameters = vec![HirParam {
                    name: parameter.clone(),
                    ty: source_type.clone(),
                }];
                let mut arguments = vec![copy];
                let mut body = Vec::with_capacity(omitted.len() + 1);
                for key in omitted {
                    let key_parameter =
                        format!("__thaw_object_rest_key_{}", self.next_binding);
                    self.next_binding += 1;
                    parameters.push(HirParam {
                        name: key_parameter.clone(),
                        ty: HirType::Str,
                    });
                    arguments.push(key);
                    body.push(HirStmt::Expr(HirExpr::JsonDelete(
                        Box::new(HirExpr::Var(parameter.clone())),
                        Box::new(HirExpr::Var(key_parameter)),
                    )));
                }
                body.push(HirStmt::Return(Some(HirExpr::Var(parameter.clone()))));
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        Vec::new(),
                        parameters,
                        source_type,
                        Box::new(HirExpr::Block(body)),
                    )),
                    arguments,
                ));
            }
            let HirType::Object(fields) = &source_type else {
                return Err(format!(
                    "object rest requires a fixed-shape object, got {source_type:?}"
                ));
            };
            let omitted = call.args[1..]
                .iter()
                .map(|argument| match argument.expr.as_ref() {
                    Expr::Lit(Lit::Str(key)) => Ok(key.value.to_string_lossy().into_owned()),
                    _ => Err("object-rest field names must be string literals".to_string()),
                })
                .collect::<Result<HashSet<_>, _>>()?;
            return Ok(HirExpr::ObjectLit(
                fields
                    .iter()
                    .filter(|(name, _)| !omitted.contains(name))
                    .map(|(name, _)| {
                        (
                            name.clone(),
                            HirExpr::PropAccess(
                                Box::new(source.clone()),
                                source_type.clone(),
                                name.clone(),
                            ),
                        )
                    })
                    .collect(),
            ));
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(property) = &member.prop {
                if let Expr::Ident(object) = member.obj.as_ref() {
                    if Self::is_static_builtin_call(object.sym.as_ref(), property.sym.as_ref()) {
                        return self.lower_static_builtin_call(object, property, call);
                    }
                }
                if Self::is_native_instance_builtin(property.sym.as_ref()) {
                    return self.lower_native_instance_builtin(member, property, call);
                }
                if matches!(
                    property.sym.as_ref(),
                    "get" | "set" | "has" | "delete" | "add" | "clear"
                ) && self.receiver_is_map_or_set(&member.obj)
                {
                    return self.lower_native_instance_builtin(member, property, call);
                }
                if matches!(property.sym.as_ref(), "finally" | "then" | "catch") {
                    return self.lower_promise_member_call(member, property, call);
                }
            }
        }

        // An object field with a function type is a callable value. Preserve
        // it as `Call(PropAccess(...), args)` instead of flattening it into a
        // synthetic `object.method` global symbol (the latter is reserved for
        // builtins such as `console.log` and `JSON.parse`).
        if let Expr::Member(member) = callee_expr.as_ref() {
            let property = member_property_name(&member.prop);
            if let Some(property) = property {
                let lowered_object = if let Expr::Ident(object) = member.obj.as_ref() {
                    let object_name = self.resolve_binding(object.sym.as_ref());
                    let object_ty = self
                        .narrowings
                        .get(&object_name)
                        .cloned()
                        .or_else(|| self.nullable_narrowings.get(&object_name).cloned())
                        .or_else(|| self.nullish_narrowings.get(&object_name).cloned())
                        .or_else(|| self.scope.get(&object_name).cloned());
                    object_ty
                        .map(|object_ty| {
                            self.lower_expr(&member.obj)
                                .map(|object_expr| (object_expr, object_ty, object.sym.to_string()))
                        })
                        .transpose()?
                } else {
                    let object_expr = self.lower_expr(&member.obj)?;
                    let object_ty = self.infer_expr_type(&object_expr)?;
                    Some((object_expr, object_ty, "<expression>".to_string()))
                };
                if let Some((object_expr, object_ty, object_label)) = lowered_object {
                    let requested_property = property.as_str();
                    let resolved_property = match (requested_property, call.args.len()) {
                        ("listen", 2) => "__listenWithCallback",
                        ("close", 1) => "__closeWithCallback",
                        ("on", 2)
                            if matches!(
                                call.args.first().map(|arg| arg.expr.as_ref()),
                                Some(Expr::Lit(Lit::Str(event))) if event.value == *"error"
                            ) =>
                        {
                            "__onError"
                        }
                        _ => requested_property,
                    };
                    let callable = match &object_ty {
                        HirType::Object(fields) => fields
                            .iter()
                            .find(|(name, _)| name == resolved_property)
                            .and_then(|(_, ty)| match ty {
                                HirType::Function(params, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    None,
                                    HirOptionalMask::default(),
                                )),
                                HirType::CallableFunction(params, optional, rest, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    rest.as_deref().cloned(),
                                    optional.clone(),
                                )),
                                _ => None,
                            }),
                        _ => None,
                    };
                    if let Some((params, _, rest, optional)) = callable {
                        // Compile-time marker: keep ordinary function-valued
                        // properties distinct from receiver-aware methods.
                        let receiver_parameter = params.first().is_some_and(|parameter| {
                            matches!(parameter, HirType::Object(fields)
                                if fields.iter().any(|(name, _)| name == "__thaw_object_method_receiver"))
                        });
                        let callee = HirExpr::PropAccess(
                            Box::new(object_expr.clone()),
                            object_ty.clone(),
                            resolved_property.to_string(),
                        );
                        let label = format!("method `{object_label}.{property}`");
                        let expected = if receiver_parameter {
                            &params[1..]
                        } else {
                            &params[..]
                        };
                        let (mut args, bindings) = self
                            .lower_native_spread_values_with_expected(
                                &call.args,
                                &label,
                                expected,
                                rest.as_ref(),
                            )?;
                        if receiver_parameter {
                            let HirType::Object(receiver_fields) = &params[0] else {
                                unreachable!()
                            };
                            args.insert(
                                0,
                                HirExpr::ObjectLit(
                                    receiver_fields
                                        .iter()
                                        .map(|(name, _)| {
                                            if name == "__thaw_object_method_receiver" {
                                                (name.clone(), HirExpr::Lit(HirLit::Undefined))
                                            } else {
                                                (
                                                    name.clone(),
                                                    HirExpr::PropAccess(
                                                        Box::new(object_expr.clone()),
                                                        object_ty.clone(),
                                                        name.clone(),
                                                    ),
                                                )
                                            }
                                        })
                                        .collect(),
                                ),
                            );
                        }
                        if args.len() < params.len()
                            && (args.len()..params.len())
                                .any(|index| !is_optional_parameter(&optional, index))
                        {
                            return Err(format!(
                                "method `{}.{}` expects at least {} argument(s), got {}",
                                object_label,
                                property,
                                params.len(),
                                args.len()
                            ));
                        }
                        let rest_values = rest.as_ref().map(|element| {
                            let values = args.split_off(params.len());
                            (element, values)
                        });
                        for parameter in params.iter().skip(args.len()) {
                            args.push(omitted_parameter_value(parameter)?);
                        }
                        let mut args = args
                            .into_iter()
                            .zip(&params)
                            .enumerate()
                            .map(|(index, (value, expected))| {
                                self.coerce_to_declared(expected, value).map_err(|error| {
                                    format!(
                                        "argument {} of `{}.{}` is invalid: {error}",
                                        index + 1,
                                        object_label,
                                        property
                                    )
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if let Some((element, values)) = rest_values {
                            let values = values
                                .into_iter()
                                .map(|value| self.coerce_to_declared(element, value))
                                .collect::<Result<Vec<_>, _>>()?;
                            args.push(native_rest_array(values, element));
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(Box::new(callee), args),
                            &bindings,
                        );
                    }
                }
            }
        }

        let mut callee_name = match callee_expr.as_ref() {
            Expr::Ident(ident) => self.resolve_binding(ident.sym.as_ref()),
            // Console methods have no dedicated HIR node; they are encoded as
            // synthetic names such as "console.log" and codegen special-cases them.
            Expr::Member(member) => {
                let Expr::Ident(obj) = member.obj.as_ref() else {
                    return Err(format!(
                        "unsupported member call target at source bytes {}..{}",
                        member.span.lo.0, member.span.hi.0
                    ));
                };
                let MemberProp::Ident(prop) = &member.prop else {
                    return Err("unsupported member call property".into());
                };
                format!("{}.{}", obj.sym, prop.sym)
            }
            _ => return Err(
                "unsupported call target (only plain identifiers and supported console methods are supported)"
                    .into(),
            ),
        };

        if matches!(
            callee_name.as_str(),
            "setTimeout"
                | "clearTimeout"
                | "setInterval"
                | "clearInterval"
                | "setImmediate"
                | "clearImmediate"
        ) && !self.scope.contains_key(&callee_name)
            && !self.signatures.contains_key(&callee_name)
        {
            let binding = format!("__thaw_global_timer_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(binding.clone(), HirType::JsValue);
            let timer_handle = matches!(
                callee_name.as_str(),
                "setTimeout" | "setInterval" | "setImmediate"
            )
            .then_some(HirType::JsValue);
            let result = self.lower_dynamic_value_call(
                &binding,
                &call.args,
                expected_return_hint.as_ref().or(timer_handle.as_ref()),
            )?;
            return self.wrap_call_argument_bindings(
                result,
                &[(
                    binding,
                    HirType::JsValue,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str(callee_name))],
                    ),
                )],
            );
        }

        if self.scope.get(&callee_name) == Some(&HirType::JsValue) {
            return self.lower_dynamic_value_call(
                &callee_name,
                &call.args,
                expected_return_hint.as_ref(),
            );
        }

        if let Some(arrow) = self.generic_arrows.get(&callee_name).cloned() {
            return self.lower_generic_arrow_call(&callee_name, &arrow, call);
        }
        if let Some(target) = self.generic_named_templates.get(&callee_name).cloned() {
            let mut forwarded = call.clone();
            forwarded.callee = Callee::Expr(Box::new(Expr::Ident(
                swc_ecma_ast::Ident::new_no_ctxt(target.into(), call.span),
            )));
            return self.lower_call(&forwarded);
        }

        if matches!(callee_name.as_str(), "isNaN" | "isFinite") {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_number(value.clone())?;
            let result = HirExpr::Call(
                Box::new(HirExpr::Var(
                    if callee_name == "isNaN" {
                        "__thaw_number_is_nan"
                    } else {
                        "__thaw_number_is_finite"
                    }
                    .to_string(),
                )),
                vec![value],
            );
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if callee_name == "structuredClone" {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "structuredClone")?;
            let [value] = arguments.as_slice() else {
                return Err("`structuredClone` expects exactly one argument".into());
            };
            let value = value.clone();
            let value_type = self.infer_expr_type(&value)?;
            let result = self.structured_clone_expr(value, value_type)?;
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if matches!(callee_name.as_str(), "encodeURIComponent" | "encodeURI") {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_string(value.clone())?;
            let intrinsic = if callee_name == "encodeURIComponent" {
                "__thaw_encode_uri_component"
            } else {
                "__thaw_encode_uri"
            };
            let result = HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), vec![value]);
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if matches!(callee_name.as_str(), "decodeURIComponent" | "decodeURI") {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_string(value.clone())?;
            let intrinsic = if callee_name == "decodeURIComponent" {
                "__thaw_decode_uri_component"
            } else {
                "__thaw_decode_uri"
            };
            let raw_name = format!("__thaw_decode_uri_raw_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(raw_name.clone(), HirType::Str);
            let body = HirExpr::Block(vec![
                HirStmt::Let(
                    raw_name.clone(),
                    HirType::Str,
                    HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), vec![value]),
                ),
                HirStmt::If(
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_is_null".to_string())),
                        vec![HirExpr::Var(raw_name.clone())],
                    ),
                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                        "URI malformed".into(),
                    )))],
                    Vec::new(),
                ),
                HirStmt::Return(Some(HirExpr::Var(raw_name))),
            ]);
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|captured| {
                    self.scope
                        .get(&captured)
                        .cloned()
                        .map(|ty| HirParam { name: captured, ty })
                })
                .collect();
            let result = HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    Vec::new(),
                    HirType::Str,
                    Box::new(body),
                )),
                Vec::new(),
            );
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if callee_name == "queueMicrotask" {
            let [callback] = call.args.as_slice() else {
                return Err("`queueMicrotask` expects exactly one callback".into());
            };
            if callback.spread.is_some() {
                return Err("`queueMicrotask` does not support a spread callback".into());
            }
            let callback = self.lower_promise_callback(&callback.expr, &[], None)?;
            let HirType::Function(_, callback_output) = self.infer_expr_type(&callback)? else {
                unreachable!()
            };
            let (output, flatten) = match callback_output.as_ref() {
                HirType::Promise(inner) => (inner.as_ref().clone(), true),
                output => (output.clone(), false),
            };
            let resolve_type = HirType::Function(Vec::new(), Box::new(HirType::Void));
            let source = HirExpr::PromiseNew(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: "__thaw_microtask_resolve".into(),
                        ty: resolve_type,
                    }],
                    HirType::Void,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_microtask_resolve".into())),
                        Vec::new(),
                    )),
                )),
                HirType::Void,
                false,
            );
            let pending = HirExpr::PromiseThen(
                Box::new(source),
                Box::new(callback),
                HirType::Void,
                output,
                false,
                flatten,
            );
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_detach_promise".into())),
                vec![pending],
            ));
        }

        if matches!(callee_name.as_str(), "Promise.all" | "Promise.allSettled" | "Promise.race" | "Promise.any") {
            return self.lower_promise_static_call(&callee_name, call);
        }

        if callee_name == "Promise.resolve" || callee_name == "Promise.reject" {
            // Desugars to the executor form (`new Promise((resolve, reject)
            // => resolve(value))` / `=> reject(message)`) instead of adding
            // a dedicated HIR node, reusing PromiseNew's already-tested
            // codegen untouched. `reject`'s payload is coerced through the
            // same string conversion `throw`/`String(value)` use, since the
            // rejection channel (like the exception channel it shares) is
            // still string-only; `resolve` keeps the value's own type,
            // assimilating an already-Promise value exactly as a bare
            // `return somePromise` inside an executor would.
            let is_reject = callee_name == "Promise.reject";
            if !is_reject && call.args.is_empty() {
                if call.type_args.as_ref().is_some_and(|args| {
                    !matches!(args.params.as_slice(), [] | [_])
                }) {
                    return Err("`Promise.resolve` accepts at most one type argument".into());
                }
                let resolve_ty = HirType::Function(Vec::new(), Box::new(HirType::Void));
                let executor = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: "__thaw_promise_resolve".into(),
                        ty: resolve_ty,
                    }],
                    HirType::Void,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_promise_resolve".into())),
                        Vec::new(),
                    )),
                );
                return Ok(HirExpr::PromiseNew(
                    Box::new(executor),
                    HirType::Void,
                    false,
                ));
            }
            let [argument] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if argument.spread.is_some() {
                return Err(format!("`{callee_name}` does not support a spread argument"));
            }
            let value = self.lower_expr(&argument.expr)?;
            let (resolved, assimilates, settled_value) = if is_reject {
                let resolved = match &call.type_args {
                    Some(type_args) => {
                        let [resolved] = type_args.params.as_slice() else {
                            return Err(
                                "`Promise.reject` accepts at most one type argument".into()
                            );
                        };
                        lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?
                    }
                    None => HirType::Void,
                };
                (resolved, false, self.coerce_primitive_to_string(value)?)
            } else {
                match self.infer_expr_type(&value)? {
                    HirType::Promise(inner) => (*inner, true, value),
                    other => (other, false, value),
                }
            };
            let resolve_params = if assimilates {
                vec![HirType::Promise(Box::new(resolved.clone()))]
            } else if resolved == HirType::Void {
                Vec::new()
            } else {
                vec![resolved.clone()]
            };
            let resolve_ty = HirType::Function(resolve_params, Box::new(HirType::Void));
            let reject_ty = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));

            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&settled_value, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    self.scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();

            let (params, callee) = if is_reject {
                (
                    vec![
                        HirParam {
                            name: "__thaw_promise_resolve".to_string(),
                            ty: resolve_ty,
                        },
                        HirParam {
                            name: "__thaw_promise_reject".to_string(),
                            ty: reject_ty,
                        },
                    ],
                    "__thaw_promise_reject",
                )
            } else {
                (
                    vec![HirParam {
                        name: "__thaw_promise_resolve".to_string(),
                        ty: resolve_ty,
                    }],
                    "__thaw_promise_resolve",
                )
            };
            let body = HirExpr::Call(Box::new(HirExpr::Var(callee.to_string())), vec![settled_value]);
            let executor = HirExpr::Lambda(captures, params, HirType::Void, Box::new(body));
            return Ok(HirExpr::PromiseNew(Box::new(executor), resolved, assimilates));
        }

        // `Number`/`String`/`Boolean` convert a `Json` leaf to a concrete
        // value. Unlike `console.log` (whose codegen can disambiguate its
        // argument by LLVM value shape -- f64 vs. pointer), `Str`/`Array`/
        if matches!(callee_name.as_str(), "parseFloat" | "parseInt") {
            return self.lower_parse_call(call, callee_name == "parseInt");
        }
        if callee_name == "Symbol" {
            if call.args.len() > 1 || call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err("`Symbol(...)` expects at most one non-spread argument".into());
            }
            let description = match call.args.first() {
                Some(argument) => {
                    let value = self.lower_expr(&argument.expr)?;
                    self.coerce_primitive_to_string(value)?
                }
                None => HirExpr::Lit(HirLit::Str(String::new())),
            };
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_symbol_new".into())),
                vec![description],
            ));
        }

        // `Object`/`Json` all share the same pointer representation, so
        // this has to be resolved here at lowering time using the
        // argument's inferred type, not deferred to codegen.
        if matches!(callee_name.as_str(), "Number" | "String" | "Boolean") {
            let [arg] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, &callee_name)?;
                let [value] = arguments.as_slice() else {
                    return Err(format!("`{callee_name}` expects exactly one argument"));
                };
                return self.lower_primitive_conversion(
                    &callee_name,
                    value.clone(),
                    bindings,
                );
            }
            let value = self.lower_expr(&arg.expr)?;
            let ty = self.infer_expr_type(&value)?;
            if callee_name == "String" && ty == HirType::Str {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_error_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::Bool {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::F64 {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::I64 {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_i64_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::Symbol {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_symbol_to_string".into())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::JsValue {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_js_handle_to_string".into())),
                    vec![value],
                ));
            }
            if callee_name == "String"
                && matches!(
                    ty,
                    HirType::Array(_)
                        | HirType::Tuple(_)
                        | HirType::Object(_)
                        | HirType::Optional(_)
                        | HirType::Nullable(_)
                        | HirType::Nullish(_)
                        | HirType::Union(_)
                        | HirType::Null
                        | HirType::Undefined
                )
            {
                return self.coerce_primitive_to_string(value);
            }
            if callee_name == "Boolean" && ty != HirType::Json {
                let name = format!("__thaw_boolean_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let converted = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
                return self.wrap_call_argument_bindings(converted, &[(name, ty, value)]);
            }
            if callee_name == "Number" && ty == HirType::F64 {
                return Ok(value);
            }
            if callee_name == "Number" && ty == HirType::Bool {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "Number" && ty == HirType::Str {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "Number" && ty == HirType::JsValue {
                return Ok(HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("readDynamicValue".to_string())),
                    vec![value],
                ))));
            }
            if callee_name == "Number"
                && matches!(
                    ty,
                    HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
                )
            {
                return self.coerce_primitive_to_number(value);
            }
            if ty != HirType::Json {
                return Err(format!(
                    "`{callee_name}(...)` is only supported on a JSON value for now (got {ty:?})"
                ));
            }
            return Ok(match callee_name.as_str() {
                "Number" => HirExpr::JsonAsNumber(Box::new(value)),
                "String" => HirExpr::JsonAsString(Box::new(value)),
                _ => HirExpr::JsonAsBool(Box::new(value)),
            });
        }

        let mut signature = self.signatures.get(&callee_name).cloned();
        let local_function = self.scope.get(&callee_name).and_then(|ty| match ty {
            HirType::Function(params, ret) => Some((
                params.clone(),
                ret.as_ref().clone(),
                None,
                HirOptionalMask::default(),
            )),
            HirType::CallableFunction(params, optional, rest, ret) => {
                let mut abi_params = params.clone();
                if let Some(rest) = rest {
                    abi_params.push(HirType::Array(rest.clone()));
                }
                Some((
                    abi_params,
                    ret.as_ref().clone(),
                    rest.as_deref().cloned(),
                    optional.clone(),
                ))
            }
            _ => None,
        });
        let mut param_types = signature
            .as_ref()
            .map(|sig| sig.params.clone())
            .or_else(|| {
                local_function
                    .as_ref()
                    .map(|(params, _, _, _)| params.clone())
            });

        let mut argument_bindings = Vec::new();
        let mut lowered_arguments = Vec::new();
        let mut forwarded_rest_array = false;
        let mut lowered = Vec::with_capacity(call.args.len());
        let generic_placeholder = signature
            .as_ref()
            .is_some_and(|signature| !signature.generic_type_params.is_empty());
        let mut generic_inferred = HashMap::new();
        for (index, argument) in call.args.iter().enumerate() {
            let generic_context = signature
                .as_ref()
                .filter(|_| argument.spread.is_none())
                .and_then(|signature| signature.generic_param_patterns.get(index))
                .and_then(|pattern| match pattern {
                    GenericTypePattern::Function(params, optional, rest, result) => {
                        let params = params
                            .iter()
                            .zip(optional)
                            .map(|(param, optional)| {
                                instantiate_generic_pattern(param, &generic_inferred).map(|ty| {
                                    if *optional {
                                        optional_parameter_type(ty)
                                    } else {
                                        ty
                                    }
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?;
                        let mut params = params;
                        if let Some(rest) = rest {
                            params.push(HirType::Array(Box::new(
                                instantiate_generic_pattern(rest, &generic_inferred).ok()?,
                            )));
                        }
                        Some((
                            params,
                            instantiate_generic_pattern(result, &generic_inferred).ok(),
                        ))
                    }
                    _ => None,
                });
            let contextual_function = generic_context.or_else(|| {
                if argument.spread.is_some() || generic_placeholder {
                    return None;
                }
                param_types
                    .as_ref()
                    .and_then(|params| params.get(index))
                    .and_then(|expected| match expected {
                        HirType::Function(params, ret) => {
                            Some((params.clone(), Some(ret.as_ref().clone())))
                        }
                        HirType::CallableFunction(params, _, rest, ret) => {
                            let mut abi_params = params.clone();
                            if let Some(rest) = rest {
                                let (argument_count, declares_rest) = match argument.expr.as_ref() {
                                    Expr::Arrow(arrow) => (
                                        arrow.params.len(),
                                        matches!(arrow.params.last(), Some(Pat::Rest(_))),
                                    ),
                                    Expr::Fn(function) => (
                                        function.function.params.len(),
                                        matches!(
                                            function.function.params.last(),
                                            Some(param) if matches!(param.pat, Pat::Rest(_))
                                        ),
                                    ),
                                    _ => (abi_params.len() + 1, true),
                                };
                                if declares_rest {
                                    abi_params.push(HirType::Array(rest.clone()));
                                } else {
                                    abi_params.resize(
                                        argument_count.max(abi_params.len()),
                                        rest.as_ref().clone(),
                                    );
                                }
                            }
                            Some((abi_params, Some(ret.as_ref().clone())))
                        }
                        _ => None,
                    })
            });
            let value = if let Some((params, ret)) = contextual_function {
                if matches!(
                    argument.expr.as_ref(),
                    Expr::Arrow(_) | Expr::Fn(_) | Expr::Ident(_)
                ) {
                    self.lower_promise_callback(&argument.expr, &params, ret.as_ref())?
                } else {
                    self.lower_expr(&argument.expr)?
                }
            } else {
                let expected = (!generic_placeholder)
                    .then(|| param_types.as_ref().and_then(|params| params.get(index)))
                    .flatten();
                match argument.expr.as_ref() {
                    Expr::Arrow(arrow)
                        if signature.as_ref().is_some_and(|signature| signature.is_extern)
                            && matches!(
                                expected,
                                Some(HirType::Dynamic | HirType::Json | HirType::JsValue)
                            ) =>
                    {
                        self.lower_contextual_arrow(
                            arrow,
                            &vec![HirType::JsValue; arrow.params.len()],
                            None,
                        )?
                    }
                    _ => self.lower_expr_with_expected_type(&argument.expr, expected)?,
                }
            };
            if let Some(pattern) = signature
                .as_ref()
                .and_then(|signature| signature.generic_param_patterns.get(index))
            {
                let actual = self.infer_expr_type(&value)?;
                let _ = match_generic_pattern(pattern, &actual, &mut generic_inferred);
            }
            lowered.push(value);
        }
        let preserve_argument_order =
            call.args.iter().any(|arg| arg.spread.is_some()) || lowered.iter().any(contains_await);
        for (arg, value) in call.args.iter().zip(lowered) {
            if !preserve_argument_order {
                lowered_arguments.push(value);
                continue;
            }
            if arg.spread.is_none() {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_call_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                argument_bindings.push((name.clone(), ty, value));
                lowered_arguments.push(HirExpr::Var(name));
                continue;
            }

            if let HirExpr::ArrayLit(values) = value {
                for value in values {
                    let ty = self.infer_expr_type(&value)?;
                    let name = format!("__thaw_call_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    argument_bindings.push((name.clone(), ty, value));
                    lowered_arguments.push(HirExpr::Var(name));
                }
                continue;
            }

            let source_type = self.infer_expr_type(&value)?;
            let rest_element = signature
                .as_ref()
                .and_then(|signature| signature.native_rest.as_ref())
                .or_else(|| {
                    local_function
                        .as_ref()
                        .and_then(|(_, _, rest, _)| rest.as_ref())
                });
            let fixed_count = param_types
                .as_ref()
                .map_or(0, |params| params.len() - usize::from(rest_element.is_some()));
            if let (HirType::Array(element), Some(rest_element)) = (&source_type, rest_element) {
                if lowered_arguments.len() == fixed_count && element.as_ref() == rest_element {
                    lowered_arguments.push(value);
                    forwarded_rest_array = true;
                    continue;
                }
            }
            let HirType::Tuple(elements) = &source_type else {
                return Err(format!(
                    "call spread source must have statically known tuple length, got {source_type:?}"
                ));
            };
            let elements = elements.clone();
            let name = format!("__thaw_call_spread_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), source_type.clone());
            argument_bindings.push((name.clone(), source_type, value));
            lowered_arguments.extend(elements.into_iter().enumerate().map(|(index, element)| {
                HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    element,
                )
            }));
        }

        let native_rest_values = if forwarded_rest_array {
            None
        } else {
            signature.as_ref().and_then(|signature| signature.native_rest.clone().map(|element| {
                let fixed_count = signature.params.len() - 1;
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
            }))
        };
        let local_rest_values = if forwarded_rest_array {
            None
        } else {
            local_function.as_ref().and_then(|(_, _, rest, _)| rest.clone()).map(|element| {
                let fixed_count = param_types
                    .as_ref()
                    .map_or(0, |params| params.len().saturating_sub(1));
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
            })
        };

        if let Some((fixed, _, _, optional)) = &local_function {
            let fixed_count = fixed.len() - usize::from(local_rest_values.is_some());
            if lowered_arguments.len() < fixed_count {
                for (index, parameter) in fixed
                    .iter()
                    .enumerate()
                    .take(fixed_count)
                    .skip(lowered_arguments.len())
                {
                    if !is_optional_parameter(optional, index) {
                        return Err(format!(
                            "function `{callee_name}` expects argument {}, but it was omitted",
                            index + 1
                        ));
                    }
                    lowered_arguments.push(omitted_parameter_value(parameter)?);
                }
            }
        }

        if let Some(full_signature) = signature.clone() {
            let logical_param_count =
                full_signature.params.len() - usize::from(full_signature.native_rest.is_some());
            if full_signature.variadic.is_none()
                && logical_param_count < usize::BITS as usize
                && lowered_arguments.len() <= logical_param_count
            {
                let mut omitted_mask = 0usize;
                for index in 0..logical_param_count {
                    let omitted = match lowered_arguments.get(index) {
                        None => true,
                        Some(value) => self.infer_expr_type(value)? == HirType::Undefined,
                    };
                    if omitted {
                        omitted_mask |= 1usize << index;
                    }
                }
                if omitted_mask != 0 {
                    let wrapper = omitted_parameter_symbol(&callee_name, omitted_mask);
                    if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                        lowered_arguments = lowered_arguments
                            .into_iter()
                            .enumerate()
                            .filter_map(|(index, argument)| {
                                (omitted_mask & (1usize << index) == 0).then_some(argument)
                            })
                            .collect();
                        callee_name = wrapper;
                        param_types = Some(wrapper_signature.params.clone());
                        signature = Some(wrapper_signature);
                    }
                }
            }
        }

        if let Some((element, values)) = native_rest_values {
            let packed = if element == HirType::Dynamic {
                let expected = signature
                    .as_ref()
                    .and_then(|signature| signature.params.last())
                    .ok_or("tuple rest parameter is missing its ABI slot")?;
                self.coerce_to_declared(expected, HirExpr::ArrayLit(values))?
            } else {
                let values = values
                    .into_iter()
                    .map(|value| self.coerce_to_declared(&element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                native_rest_array(values, &element)
            };
            lowered_arguments.push(packed);
            param_types = signature.as_ref().map(|signature| signature.params.clone());
        }
        if let Some((element, values)) = local_rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(&element, value))
                .collect::<Result<Vec<_>, _>>()?;
            lowered_arguments.push(native_rest_array(values, &element));
        }

        if signature.as_ref().is_some_and(|signature| {
            signature.variadic.is_none()
                && signature.native_rest.is_none()
                && lowered_arguments.len() != signature.params.len()
        }) {
            let wrapper = default_arity_symbol(&callee_name, lowered_arguments.len());
            if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                callee_name = wrapper;
                param_types = Some(wrapper_signature.params.clone());
                signature = Some(wrapper_signature);
            }
        }

        if let Some(params) = &param_types {
            let variadic = signature.as_ref().and_then(|sig| sig.variadic.as_ref());
            let generic_optional = signature.as_ref().filter(|signature| {
                !signature.generic_type_params.is_empty()
                    && signature.generic_param_optional.iter().any(|optional| *optional)
            });
            let wrong_count = if let Some(signature) = generic_optional {
                let required = signature
                    .generic_param_optional
                    .iter()
                    .take_while(|optional| !**optional)
                    .count();
                lowered_arguments.len() < required || lowered_arguments.len() > params.len()
            } else if variadic.is_some() {
                lowered_arguments.len() < params.len()
            } else {
                lowered_arguments.len() != params.len()
            };
            if wrong_count {
                return Err(format!(
                    "function `{callee_name}` expects {}{} argument(s), got {}",
                    if variadic.is_some() { "at least " } else { "" },
                    params.len(),
                    lowered_arguments.len()
                ));
            }
        }

        let mut args = lowered_arguments
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                match param_types
                    .as_ref()
                    .and_then(|p| p.get(i))
                    .or_else(|| signature.as_ref().and_then(|sig| sig.variadic.as_ref()))
                {
                    Some(_)
                        if signature
                            .as_ref()
                            .is_some_and(|sig| !sig.generic_type_params.is_empty()) =>
                    {
                        Ok(value)
                    }
                    Some(declared) => {
                        let value = if local_function.is_some()
                            && *declared == HirType::Json
                            && self.infer_expr_type(&value)? == HirType::JsValue
                        {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                                vec![value],
                            )
                        } else {
                            value
                        };
                        self.coerce_to_declared(declared, value).map_err(|error| {
                            format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                        })
                    }
                    None => Ok(value),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        if callee_name == "console.assert" {
            if args.is_empty() {
                args.push(HirExpr::Lit(HirLit::Bool(false)));
            } else {
                let condition_value = args.remove(0);
                let ty = self.infer_expr_type(&condition_value)?;
                let name = format!("__thaw_console_assert_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let condition = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
                argument_bindings.push((name, ty, condition_value));
                args.insert(0, condition);
            }
        }

        let generic_types = if let Some(signature) = signature
            .as_ref()
            .filter(|signature| !signature.generic_type_params.is_empty())
        {
            let preserves_key_literal = |index: usize| {
                let Some(GenericTypePattern::Variable(name)) =
                    signature.generic_param_patterns.get(index)
                else {
                    return false;
                };
                signature
                    .generic_type_params
                    .iter()
                    .position(|parameter| parameter == name)
                    .and_then(|index| signature.generic_type_constraints.get(index))
                    .and_then(Option::as_deref)
                    .is_some_and(|constraint| {
                        matches!(constraint, TsType::TsTypeOperator(operator) if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf)
                    })
            };
            let actual = args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    match (call.type_args.is_none() && preserves_key_literal(index))
                        .then(|| call.args.get(index))
                        .flatten()
                        .filter(|argument| argument.spread.is_none())
                    {
                        Some(argument) => match argument.expr.as_ref() {
                            Expr::Lit(Lit::Str(value)) => Ok(HirType::StrLiteral(
                                value.value.to_string_lossy().into_owned(),
                            )),
                            _ => self.infer_expr_type(arg),
                        },
                        None => self.infer_expr_type(arg),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            let types = if let Some(type_args) = &call.type_args {
                resolve_explicit_generic_type_tuple(
                    signature,
                    &type_args.params,
                    &actual,
                    self.interfaces,
                    self.generic_interfaces,
                )
            } else {
                infer_generic_type_tuple(
                    signature,
                    &actual,
                    self.interfaces,
                    self.generic_interfaces,
                    expected_return_hint.as_ref(),
                )
            }
            .map_err(|error| format!("call to generic function `{callee_name}`: {error}"))?;
            if !types.contains(&HirType::Dynamic) {
                for ty in &types {
                    if !(supports_generic_native_layout(ty)
                        || signature.is_extern && supports_generic_dynamic_layout(ty))
                    {
                        return Err(format!(
                            "generic function `{callee_name}` cannot specialize for native layout {ty:?}"
                        ));
                    }
                }
            }
            Some(types)
        } else {
            if call.type_args.is_some() {
                return Err(format!(
                    "non-generic function `{callee_name}` does not accept type arguments"
                ));
            }
            None
        };

        if let (Some(signature), Some(constraints)) = (&signature, self.call_constraints) {
            if let Some(types) = &generic_types {
                constraints
                    .borrow_mut()
                    .push(CallConstraint::Generic(callee_name.clone(), types.clone()));
            } else {
                for (index, (declared, value)) in signature.params.iter().zip(&args).enumerate() {
                    let actual = self.infer_expr_type(value)?;
                    if *declared == HirType::Dynamic
                        || (*declared == HirType::Json && actual == HirType::JsValue)
                    {
                        constraints.borrow_mut().push(CallConstraint::Parameter(
                            callee_name.clone(),
                            index,
                            actual,
                            (call.span.lo.0, call.span.hi.0),
                        ));
                    }
                }
            }
        }

        if let Some(sig) = signature.clone().filter(|sig| sig.is_extern) {
            if let Some((backend, symbol)) = dynamic_symbol(&callee_name) {
                let (params, ret) = if let Some(types) = &generic_types {
                    let substitution = sig
                        .generic_type_params
                        .iter()
                        .cloned()
                        .zip(types.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    let params = sig
                        .generic_param_patterns
                        .iter()
                        .zip(&sig.generic_param_optional)
                        .map(|(pattern, optional)| {
                            let ty = instantiate_generic_pattern(pattern, &substitution)?;
                            Ok(if *optional { optional_parameter_type(ty) } else { ty })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let ret = resolve_ts_type_with_substitution(
                        sig.generic_return_type
                            .as_ref()
                            .expect("generic function return type"),
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )?;
                    (params, ret)
                } else {
                    (sig.params, sig.ret)
                };
                // Argument coercion was skipped for every generic
                // function's call above (`param_types` there was still
                // the pre-substitution, `Dynamic`-placeholder version --
                // coercing against it would be meaningless), deferred to
                // here where `params` is finally the real, substituted
                // type. Needed for a value going into a parameter that's
                // optional only *after* substitution (e.g. `nanoid`'s
                // `size?: number`, `Optional(F64)` -- a raw `5.0` needs
                // wrapping into that Optional's own representation, not
                // just passing through unwrapped). Coerces only the
                // arguments actually present, up to `params.len()` --
                // never more (a variadic call's own trailing rest
                // arguments, beyond `params.len()`, pass through
                // unchanged instead of being truncated away), and never
                // fewer either (an optional trailing parameter, like
                // `size?` above, can legitimately be omitted; arity is
                // somebody else's job, not this coercion step's).
                let fixed_count = params.len().min(args.len());
                let mut args = args.into_iter();
                let mut coerced_args = params
                    .iter()
                    .take(fixed_count)
                    .enumerate()
                    .map(|(i, declared)| {
                        let value = args.next().expect("fixed_count <= args.len()");
                        self.coerce_to_declared(declared, value).map_err(|error| {
                            format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                coerced_args.extend(args);
                let args = coerced_args;
                let result = HirExpr::DynamicCall(
                    DynamicSignature {
                        backend,
                        symbol,
                        params,
                        ret,
                    },
                    args,
                );
                return self.wrap_call_argument_bindings(result, &argument_bindings);
            }
            // A plain (non-`__thaw_typed_*`) generic extern function's own
            // `params`/`ret` were collected using every type parameter
            // substituted with `Dynamic` (a placeholder so the signature
            // parses at all -- see `function_type_substitution`), not this
            // call's actual inferred types; re-derive them the same way
            // the `dynamic_symbol` branch above already does, or a
            // parameter/return that mentions a type parameter at all
            // (e.g. `nanoid<Type extends string>(size?: number): Type`'s
            // return) would stay `Dynamic` here regardless of `Type`
            // having been successfully inferred just above.
            let (params, ret) = if let Some(types) = &generic_types {
                let substitution = sig
                    .generic_type_params
                    .iter()
                    .cloned()
                    .zip(types.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let params = sig
                    .generic_param_patterns
                    .iter()
                    .zip(&sig.generic_param_optional)
                    .map(|(pattern, optional)| {
                        let ty = instantiate_generic_pattern(pattern, &substitution)?;
                        Ok(if *optional { optional_parameter_type(ty) } else { ty })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let ret = match &sig.generic_return_type {
                    Some(declared) => resolve_ts_type_with_substitution(
                        declared,
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )?,
                    None => sig.ret.clone(),
                };
                (params, ret)
            } else {
                (sig.params.clone(), sig.ret.clone())
            };
            // See the identical comment on the `dynamic_symbol` branch
            // above: argument coercion against the real, substituted
            // parameter types (rather than the pre-substitution
            // `Dynamic` placeholder) was deferred to here, touching only
            // the arguments actually present up to `params.len()` -- a
            // variadic call's own trailing rest arguments (beyond
            // `params.len()`) pass through unchanged rather than being
            // truncated away, and an omitted optional trailing
            // parameter is left for arity checking elsewhere to handle,
            // not an error here.
            let fixed_count = params.len().min(args.len());
            let mut args = args.into_iter();
            let mut coerced_args = params
                .iter()
                .take(fixed_count)
                .enumerate()
                .map(|(i, declared)| {
                    let value = args.next().expect("fixed_count <= args.len()");
                    self.coerce_to_declared(declared, value).map_err(|error| {
                        format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            coerced_args.extend(args);
            let args = coerced_args;
            let param_count = params.len();
            let ffi_signature = FfiSignature {
                symbol: callee_name,
                params,
                variadic: sig.variadic,
                variadic_abi: crate::FfiVariadicAbi::Native,
                ret,
                error_abi: FfiErrorAbi::Direct,
                return_ownership: FfiOwnership::Borrowed,
                error_ownership: FfiOwnership::Borrowed,
                param_string_abis: vec![FfiStringAbi::NullTerminated; param_count],
                return_string_abi: FfiStringAbi::NullTerminated,
                calling_convention: FfiCallingConvention::C,
                aggregate_return_abi: FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            };
            let result = HirExpr::FfiCall(Box::new(ffi_signature), args);
            return self.wrap_call_argument_bindings(result, &argument_bindings);
        }

        let lowered_name = if let Some(types) = generic_types
            .as_ref()
            .filter(|types| !types.contains(&HirType::Dynamic))
        {
            let param_types = args
                .iter()
                .map(|arg| self.infer_expr_type(arg))
                .collect::<Result<Vec<_>, _>>()?;
            let signature = signature
                .as_ref()
                .expect("generic types require a generic signature");
            let lowered_name =
                specialized_generic_function_name(&callee_name, &param_types, signature, types);
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(types.iter().cloned())
                .collect::<HashMap<_, _>>();
            let return_type = resolve_ts_type_with_substitution(
                signature
                    .generic_return_type
                    .as_ref()
                    .expect("generic return type"),
                &substitution,
                self.interfaces,
                self.generic_interfaces,
                &mut Vec::new(),
            )?;
            self.generic_call_returns
                .insert(lowered_name.clone(), return_type);
            lowered_name
        } else {
            callee_name
        };
        let result = HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), args);
        self.wrap_call_argument_bindings(result, &argument_bindings)
    }
}
