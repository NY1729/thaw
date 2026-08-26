impl<'a> FnLowerer<'a> {
    fn native_class_expression_type(&self, expression: &Expr) -> Option<HirType> {
        match expression {
            Expr::Ident(identifier) => self
                .scope
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::This(_) => self.scope.get(&self.resolve_binding("this")).cloned(),
            Expr::New(construction) => construction
                .callee
                .as_ident()
                .and_then(|class| self.interfaces.get(class.sym.as_ref()).cloned()),
            Expr::Paren(parenthesized) => self.native_class_expression_type(&parenthesized.expr),
            Expr::TsAs(assertion) => self.native_class_expression_type(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => self.native_class_expression_type(&assertion.expr),
            Expr::TsConstAssertion(assertion) => self.native_class_expression_type(&assertion.expr),
            Expr::Seq(sequence) => sequence
                .exprs
                .last()
                .and_then(|expression| self.native_class_expression_type(expression)),
            Expr::Cond(conditional) => {
                let consequent = self.native_class_expression_type(&conditional.cons)?;
                let alternate = self.native_class_expression_type(&conditional.alt)?;
                (consequent == alternate).then_some(consequent)
            }
            Expr::Member(member) => {
                let property = member_property_name(&member.prop)?;
                let HirType::Object(fields) = self.native_class_expression_type(&member.obj)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find_map(|(name, ty)| (name == property).then_some(ty))
            }
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                match callee.as_ref() {
                    Expr::Ident(function) => self
                        .signatures
                        .get(function.sym.as_ref())
                        .map(|signature| signature.ret.clone()),
                    Expr::Member(member) => {
                        let method = member_property_name(&member.prop)?;
                        if let Expr::Ident(class) = member.obj.as_ref() {
                            let symbol = class_static_method_symbol(class.sym.as_ref(), &method);
                            if let Some(signature) = self.signatures.get(&symbol) {
                                return Some(signature.ret.clone());
                            }
                        }
                        let receiver = self.native_class_expression_type(&member.obj)?;
                        let class = class_name_from_type(&receiver)?;
                        self.signatures
                            .get(&class_method_symbol(class, &method))
                            .map(|signature| signature.ret.clone())
                    }
                    _ => None,
                }
            }
            _ => infer_generic_constructor_expr_type(
                expression,
                self.interfaces,
                self.generic_interfaces,
                std::slice::from_ref(&self.scope),
                &self.generic_call_returns,
            )
            .ok(),
        }
    }

    fn lower_native_class_bind(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(bind) = callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&bind.prop).as_deref() != Some("bind") {
            return Ok(None);
        }
        let Expr::Member(method) = bind.obj.as_ref() else {
            return Ok(None);
        };
        let Some(method_name) = member_property_name(&method.prop) else {
            return Ok(None);
        };
        let Some(target_type) = self.native_class_expression_type(&method.obj) else {
            return Ok(None);
        };
        let Some(class_name) = class_name_from_type(&target_type) else {
            return Ok(None);
        };
        let symbol = class_method_symbol(class_name, &method_name);
        let Some(signature) = self.signatures.get(&symbol).cloned() else {
            return Ok(None);
        };
        if call.type_args.is_some() {
            return Err("native class method `.bind()` does not accept type arguments".into());
        }
        let Some((this_argument, leading_arguments)) = call.args.split_first() else {
            return Err(format!(
                "native class method `{class_name}.{method_name}.bind` expects a `thisArg`"
            ));
        };
        if this_argument.spread.is_some() {
            return Err("native class method `.bind()` cannot spread its `thisArg`".into());
        }
        let target = self.lower_expr(&method.obj)?;
        self.expect_type(&target_type, &target, "method bind target")?;
        let bound = self.lower_expr(&this_argument.expr)?;
        self.expect_type(&target_type, &bound, "method bind `thisArg`")?;
        let label = format!("native class method `{class_name}.{method_name}.bind`");
        let (leading_values, leading_bindings) =
            self.lower_native_spread_values(leading_arguments, &label)?;
        if leading_values.len() > signature.params.len().saturating_sub(1) {
            return Err(format!(
                "native class method `{class_name}.{method_name}.bind` binds {} leading argument(s), but the method accepts {}",
                leading_values.len(),
                signature.params.len().saturating_sub(1)
            ));
        }

        let target_name = format!("__thaw_bind_target_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), target_type.clone());
        let bound_name = format!("__thaw_bound_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(bound_name.clone(), target_type.clone());
        let mut bindings = vec![
            (target_name, target_type.clone(), target),
            (bound_name.clone(), target_type.clone(), bound),
        ];
        bindings.extend(leading_bindings);
        let mut bound_argument_names = Vec::new();
        for (index, (value, expected)) in leading_values
            .iter()
            .zip(signature.params[1..].iter())
            .enumerate()
        {
            let value = self
                .coerce_to_declared(expected, value.clone())
                .map_err(|error| {
                    format!(
                        "bound argument {} of `{class_name}.{method_name}` is invalid: {error}",
                        index + 1
                    )
                })?;
            let name = format!("__thaw_bound_leading_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), expected.clone());
            bound_argument_names.push(name.clone());
            bindings.push((name, expected.clone(), value));
        }
        let parameters = signature.params[1 + leading_values.len()..]
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_bound_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        let mut arguments = vec![HirExpr::Var(bound_name.clone())];
        arguments.extend(bound_argument_names.iter().cloned().map(HirExpr::Var));
        arguments.extend(
            parameters
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        let mut captures = vec![HirParam {
            name: bound_name.clone(),
            ty: target_type.clone(),
        }];
        captures.extend(
            bound_argument_names
                .iter()
                .cloned()
                .zip(
                    signature.params[1..1 + leading_values.len()]
                        .iter()
                        .cloned(),
                )
                .map(|(name, ty)| HirParam { name, ty }),
        );
        let closure_return = if signature.is_async && !matches!(signature.ret, HirType::Promise(_))
        {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let closure = HirExpr::Lambda(
            captures,
            parameters,
            closure_return,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments)),
        );
        self.wrap_call_argument_bindings(closure, &bindings)
            .map(Some)
    }

    fn lower_native_static_bind(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(bind) = callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&bind.prop).as_deref() != Some("bind") {
            return Ok(None);
        }
        let Expr::Member(method) = bind.obj.as_ref() else {
            return Ok(None);
        };
        let Expr::Ident(class) = method.obj.as_ref() else {
            return Ok(None);
        };
        let Some(method_name) = member_property_name(&method.prop) else {
            return Ok(None);
        };
        let symbol = class_static_method_symbol(class.sym.as_ref(), &method_name);
        let Some(signature) = self.signatures.get(&symbol).cloned() else {
            return Ok(None);
        };
        let Some((this_argument, leading_arguments)) = call.args.split_first() else {
            return Err(format!(
                "native static method `{}.{method_name}.bind` expects a `thisArg`",
                class.sym
            ));
        };
        if this_argument.spread.is_some() {
            return Err("native static method `.bind()` cannot spread its `thisArg`".into());
        }
        let this_value = self.lower_expr(&this_argument.expr)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let label = format!("native static method `{}.{method_name}.bind`", class.sym);
        let (leading_values, leading_bindings) =
            self.lower_native_spread_values(leading_arguments, &label)?;
        if leading_values.len() > signature.params.len() {
            return Err(format!(
                "native static method `{}.{method_name}.bind` binds {} leading argument(s), but the method accepts {}",
                class.sym,
                leading_values.len(),
                signature.params.len()
            ));
        }
        let this_name = format!("__thaw_static_bind_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        let mut bindings = vec![(this_name, this_type, this_value)];
        bindings.extend(leading_bindings);
        let mut captures = Vec::new();
        let mut arguments = Vec::new();
        for (index, (value, expected)) in leading_values
            .iter()
            .zip(signature.params.iter())
            .enumerate()
        {
            let value = self
                .coerce_to_declared(expected, value.clone())
                .map_err(|error| {
                    format!(
                        "bound argument {} of `{}.{method_name}` is invalid: {error}",
                        index + 1,
                        class.sym
                    )
                })?;
            let name = format!("__thaw_static_bound_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), expected.clone());
            bindings.push((name.clone(), expected.clone(), value));
            captures.push(HirParam {
                name: name.clone(),
                ty: expected.clone(),
            });
            arguments.push(HirExpr::Var(name));
        }
        let parameters = signature.params[leading_values.len()..]
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_static_bound_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        arguments.extend(
            parameters
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let closure = HirExpr::Lambda(
            captures,
            parameters,
            result,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments)),
        );
        self.wrap_call_argument_bindings(closure, &bindings)
            .map(Some)
    }

    fn lower_native_method_reference(
        &mut self,
        member: &MemberExpr,
        method_name: &str,
    ) -> Result<Option<HirExpr>, String> {
        if let Expr::Ident(class) = member.obj.as_ref() {
            let symbol = class_static_method_symbol(class.sym.as_ref(), method_name);
            if let Some(signature) = self.signatures.get(&symbol) {
                if signature.uses_this {
                    let result =
                        if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        };
                    return Ok(Some(HirExpr::MethodRef(
                        unbound_class_method_symbol(&symbol),
                        symbol,
                        signature.params.clone(),
                        result,
                        true,
                    )));
                }
                let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_))
                {
                    HirType::Promise(Box::new(signature.ret.clone()))
                } else {
                    signature.ret.clone()
                };
                return Ok(Some(HirExpr::FunctionRef(
                    symbol,
                    signature.params.clone(),
                    result,
                )));
            }
        }

        let Some(receiver_type) = self.native_class_expression_type(&member.obj) else {
            return Ok(None);
        };
        let Some(class_name) = class_name_from_type(&receiver_type) else {
            return Ok(None);
        };
        let symbol = class_method_symbol(class_name, method_name);
        let Some(signature) = self.signatures.get(&symbol).cloned() else {
            return Ok(None);
        };
        if signature.uses_this {
            let receiver = self.lower_expr(&member.obj)?;
            self.expect_type(&receiver_type, &receiver, "method reference receiver")?;
            let receiver_name = format!("__thaw_unbound_method_receiver_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(receiver_name.clone(), receiver_type.clone());
            let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
                HirType::Promise(Box::new(signature.ret.clone()))
            } else {
                signature.ret.clone()
            };
            let unbound = HirExpr::MethodRef(
                unbound_class_method_symbol(&symbol),
                symbol,
                signature.params[1..].to_vec(),
                result,
                false,
            );
            return self
                .wrap_call_argument_bindings(unbound, &[(receiver_name, receiver_type, receiver)])
                .map(Some);
        }
        let receiver = self.lower_expr(&member.obj)?;
        self.expect_type(&receiver_type, &receiver, "method reference receiver")?;
        let receiver_name = format!("__thaw_method_reference_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), receiver_type.clone());
        let parameters = signature.params[1..]
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_method_reference_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        let mut arguments = vec![HirExpr::Var(receiver_name.clone())];
        arguments.extend(
            parameters
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let closure = HirExpr::Lambda(
            vec![HirParam {
                name: receiver_name.clone(),
                ty: receiver_type.clone(),
            }],
            parameters,
            result,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments)),
        );
        self.wrap_call_argument_bindings(closure, &[(receiver_name, receiver_type, receiver)])
            .map(Some)
    }

    fn native_instance_method_value(&self, expression: &Expr) -> Option<NativeMethodValue> {
        let Expr::Member(member) = expression else {
            return None;
        };
        let method = member_property_name(&member.prop)?;
        if let Expr::Ident(class) = member.obj.as_ref() {
            let symbol = class_static_method_symbol(class.sym.as_ref(), &method);
            if self
                .signatures
                .get(&symbol)
                .is_some_and(|signature| signature.uses_this)
            {
                return Some(NativeMethodValue {
                    symbol,
                    receiver: None,
                });
            }
        }
        let receiver = self.native_class_expression_type(&member.obj)?;
        let class = class_name_from_type(&receiver)?;
        let symbol = class_method_symbol(class, &method);
        self.signatures
            .get(&symbol)
            .is_some_and(|signature| signature.uses_this)
            .then_some(NativeMethodValue {
                symbol,
                receiver: Some(receiver),
            })
    }

    fn lower_saved_native_method_bind(
        &mut self,
        target: &str,
        method: &NativeMethodValue,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let signature = self
            .signatures
            .get(&method.symbol)
            .cloned()
            .expect("saved native method retains its signature");
        let Some((this_argument, leading_arguments)) = call.args.split_first() else {
            return Err(format!(
                "unbound native method `{target}.bind` expects a `thisArg`"
            ));
        };
        if this_argument.spread.is_some() {
            return Err("unbound native method `.bind()` cannot spread its `thisArg`".into());
        }
        let bound = self.lower_expr(&this_argument.expr)?;
        let bound_type = if let Some(receiver) = &method.receiver {
            self.expect_type(receiver, &bound, "unbound method bind `thisArg`")?;
            receiver.clone()
        } else {
            self.infer_expr_type(&bound)?
        };
        let receiver_count = usize::from(method.receiver.is_some());
        let label = format!("unbound native method `{target}.bind`");
        let (leading_values, leading_bindings) =
            self.lower_native_spread_values(leading_arguments, &label)?;
        if leading_values.len() > signature.params.len().saturating_sub(receiver_count) {
            return Err(format!(
                "unbound native method `{target}.bind` binds {} leading argument(s), but the method accepts {}",
                leading_values.len(),
                signature.params.len().saturating_sub(receiver_count)
            ));
        }
        let bound_name = format!("__thaw_saved_bound_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(bound_name.clone(), bound_type.clone());
        let mut bindings = vec![(bound_name.clone(), bound_type, bound)];
        bindings.extend(leading_bindings);
        let mut bound_argument_names = Vec::new();
        for (index, (value, expected)) in leading_values
            .iter()
            .zip(signature.params[receiver_count..].iter())
            .enumerate()
        {
            let value = self
                .coerce_to_declared(expected, value.clone())
                .map_err(|error| {
                    format!(
                        "bound argument {} of `{target}` is invalid: {error}",
                        index + 1
                    )
                })?;
            let name = format!("__thaw_saved_bound_argument_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), expected.clone());
            bound_argument_names.push(name.clone());
            bindings.push((name, expected.clone(), value));
        }
        let parameters = signature.params[receiver_count + leading_values.len()..]
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_saved_bound_parameter_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        let mut arguments = method
            .receiver
            .as_ref()
            .map(|_| vec![HirExpr::Var(bound_name.clone())])
            .unwrap_or_default();
        arguments.extend(bound_argument_names.iter().cloned().map(HirExpr::Var));
        arguments.extend(
            parameters
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        let mut captures = method
            .receiver
            .as_ref()
            .map(|receiver| {
                vec![HirParam {
                    name: bound_name,
                    ty: receiver.clone(),
                }]
            })
            .unwrap_or_default();
        captures.extend(
            bound_argument_names
                .into_iter()
                .zip(
                    signature.params[receiver_count..receiver_count + leading_values.len()]
                        .iter()
                        .cloned(),
                )
                .map(|(name, ty)| HirParam { name, ty }),
        );
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        self.wrap_call_argument_bindings(
            HirExpr::Lambda(
                captures,
                parameters,
                result,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var(method.symbol.clone())),
                    arguments,
                )),
            ),
            &bindings,
        )
    }

    fn lower_function_bind(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(operation) = callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&operation.prop).as_deref() != Some("bind") {
            return Ok(None);
        }
        let target = self.lower_expr(&operation.obj)?;
        let target_type = self.infer_expr_type(&target)?;
        let (params, optional, rest, ret) = match &target_type {
            HirType::Function(params, ret) => (
                params.clone(),
                HirOptionalMask::default(),
                None,
                ret.as_ref().clone(),
            ),
            HirType::CallableFunction(params, optional, rest, ret) => (
                params.clone(),
                optional.clone(),
                rest.as_deref().cloned(),
                ret.as_ref().clone(),
            ),
            _ => return Ok(None),
        };
        let Some((this_argument, leading)) = call.args.split_first() else {
            return Err("function .bind() expects a thisArg".into());
        };
        if this_argument.spread.is_some() {
            return Err("function .bind() cannot spread its thisArg".into());
        }
        let this_value = self.lower_expr(&this_argument.expr)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let (leading, spread_bindings) =
            self.lower_native_spread_values(leading, "function .bind()")?;
        if !optional.is_empty() || rest.is_some() {
            if rest.is_none() && leading.len() > params.len() {
                return Err(format!(
                    "function .bind() binds {} leading argument(s), but the function accepts {}",
                    leading.len(),
                    params.len()
                ));
            }
            let fixed_bound_count = leading.len().min(params.len());
            let target_name = format!("__thaw_rest_bind_target_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(target_name.clone(), target_type.clone());
            let this_name = format!("__thaw_rest_bind_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(this_name.clone(), this_type.clone());
            let mut bindings = vec![
                (target_name.clone(), target_type, target),
                (this_name.clone(), this_type, this_value),
            ];
            bindings.extend(spread_bindings);

            let mut bound_names = Vec::with_capacity(leading.len());
            for (index, value) in leading.into_iter().enumerate() {
                let expected = params
                    .get(index)
                    .or(rest.as_ref())
                    .expect("leading callable argument has a fixed or rest type");
                let value = self.coerce_to_declared(expected, value)?;
                let name = format!("__thaw_rest_bound_argument_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), expected.clone());
                bindings.push((name.clone(), expected.clone(), value));
                bound_names.push((name, expected.clone()));
            }

            let remaining_fixed = params[fixed_bound_count..].to_vec();
            let mut closure_params = remaining_fixed
                .iter()
                .enumerate()
                .map(|(index, ty)| HirParam {
                    name: format!("__thaw_rest_bind_parameter_{index}"),
                    ty: ty.clone(),
                })
                .collect::<Vec<_>>();
            let invocation_rest_name = rest.as_ref().map(|rest| {
                let name = format!("__thaw_rest_bind_invocation_{}", self.next_binding);
                self.next_binding += 1;
                closure_params.push(HirParam {
                    name: name.clone(),
                    ty: HirType::Array(Box::new(rest.clone())),
                });
                name
            });

            let mut arguments = bound_names[..fixed_bound_count]
                .iter()
                .map(|(name, _)| HirExpr::Var(name.clone()))
                .collect::<Vec<_>>();
            arguments.extend(
                closure_params[..remaining_fixed.len()]
                    .iter()
                    .map(|parameter| HirExpr::Var(parameter.name.clone())),
            );
            let mut source_abi = params.clone();
            if let (Some(rest), Some(invocation_rest_name)) = (&rest, invocation_rest_name) {
                let bound_rest = bound_names[fixed_bound_count..]
                    .iter()
                    .map(|(name, _)| HirExpr::Var(name.clone()))
                    .collect::<Vec<_>>();
                let rest_argument = if bound_rest.is_empty() {
                    HirExpr::Var(invocation_rest_name)
                } else {
                    HirExpr::ArrayConcat(
                        vec![
                            HirExpr::ArrayLit(bound_rest),
                            HirExpr::Var(invocation_rest_name),
                        ],
                        rest.clone(),
                    )
                };
                arguments.push(rest_argument);
                source_abi.push(HirType::Array(Box::new(rest.clone())));
            }
            let body = HirExpr::FunctionCallWithThis(
                Box::new(HirExpr::Var(target_name.clone())),
                Box::new(HirExpr::Var(this_name.clone())),
                arguments,
                source_abi,
                ret.clone(),
            );
            let mut captures = vec![
                HirParam {
                    name: target_name,
                    ty: HirType::CallableFunction(
                        params.clone(),
                        optional.clone(),
                        rest.clone().map(Box::new),
                        Box::new(ret.clone()),
                    ),
                },
                HirParam {
                    name: this_name,
                    ty: bindings[1].1.clone(),
                },
            ];
            captures.extend(
                bound_names
                    .into_iter()
                    .map(|(name, ty)| HirParam { name, ty }),
            );
            let closure = HirExpr::Lambda(captures, closure_params, ret.clone(), Box::new(body));
            let logical = HirType::CallableFunction(
                remaining_fixed.clone(),
                optional.shifted(fixed_bound_count),
                rest.map(Box::new),
                Box::new(ret),
            );
            return self
                .wrap_call_argument_bindings(
                    HirExpr::TypedClosure(logical, Box::new(closure)),
                    &bindings,
                )
                .map(Some);
        }
        if leading.len() > params.len() {
            return Err(format!(
                "function .bind() binds {} leading argument(s), but the function accepts {}",
                leading.len(),
                params.len()
            ));
        }
        let leading = leading
            .into_iter()
            .zip(&params)
            .map(|(value, expected)| self.coerce_to_declared(expected, value))
            .collect::<Result<Vec<_>, _>>()?;
        let target_name = format!("__thaw_function_bind_target_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), target_type.clone());
        let this_name = format!("__thaw_function_bind_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        let mut bindings = vec![
            (target_name.clone(), target_type, target),
            (this_name.clone(), this_type, this_value),
        ];
        bindings.extend(spread_bindings);
        self.wrap_call_argument_bindings(
            HirExpr::FunctionBindThis(
                Box::new(HirExpr::Var(target_name)),
                Box::new(HirExpr::Var(this_name)),
                leading,
                params,
                ret,
            ),
            &bindings,
        )
        .map(Some)
    }

    fn lower_function_call_or_apply(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(operation) = callee.as_ref() else {
            return Ok(None);
        };
        let Some(operation_name) = member_property_name(&operation.prop) else {
            return Ok(None);
        };
        if operation_name != "call" && operation_name != "apply" {
            return Ok(None);
        }
        let target = self.lower_expr(&operation.obj)?;
        let target_type = self.infer_expr_type(&target)?;
        let (params, optional, rest, ret) = match &target_type {
            HirType::Function(params, ret) => (
                params.clone(),
                HirOptionalMask::default(),
                None,
                ret.as_ref().clone(),
            ),
            HirType::CallableFunction(params, optional, rest, ret) => (
                params.clone(),
                optional.clone(),
                rest.as_deref().cloned(),
                ret.as_ref().clone(),
            ),
            _ => return Ok(None),
        };
        let Some((this_argument, supplied)) = call.args.split_first() else {
            return Err(format!("function .{operation_name}() expects a thisArg"));
        };
        if this_argument.spread.is_some() {
            return Err(format!(
                "function .{operation_name}() cannot spread its thisArg"
            ));
        }
        let forwarded = if operation_name == "apply" {
            let [arguments] = supplied else {
                return Err(
                    "function .apply() expects exactly a thisArg and an argument tuple".into(),
                );
            };
            if arguments.spread.is_some() {
                return Err("function .apply() tuple cannot itself be spread".into());
            }
            vec![swc_ecma_ast::ExprOrSpread {
                spread: Some(call.span),
                expr: arguments.expr.clone(),
            }]
        } else {
            supplied.to_vec()
        };
        let this_value = self.lower_expr(&this_argument.expr)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let label = format!("function .{operation_name}()");
        let (arguments, spread_bindings) = self.lower_native_spread_values(&forwarded, &label)?;
        let mut arguments = arguments;
        if arguments.len() > params.len() && rest.is_none() {
            return Err(format!(
                "function .{operation_name}() expects {} argument(s), got {}",
                params.len(),
                arguments.len()
            ));
        }
        if arguments.len() < params.len()
            && (arguments.len()..params.len()).any(|index| !is_optional_parameter(&optional, index))
        {
            return Err(format!(
                "function .{operation_name}() expects at least {} argument(s), got {}",
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
        let mut abi_params = params;
        if let Some((element, values)) = rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(element, value))
                .collect::<Result<Vec<_>, _>>()?;
            arguments.push(native_rest_array(values, element));
            abi_params.push(HirType::Array(Box::new(element.clone())));
        }
        let target_name = format!("__thaw_function_operation_target_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), target_type.clone());
        let this_name = format!("__thaw_function_operation_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        let mut bindings = vec![
            (target_name.clone(), target_type, target),
            (this_name.clone(), this_type, this_value),
        ];
        bindings.extend(spread_bindings);
        self.wrap_call_argument_bindings(
            HirExpr::FunctionCallWithThis(
                Box::new(HirExpr::Var(target_name)),
                Box::new(HirExpr::Var(this_name)),
                arguments,
                abi_params,
                ret,
            ),
            &bindings,
        )
        .map(Some)
    }

    fn lower_saved_native_method_call_or_apply(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        if let Expr::Ident(target) = callee.as_ref() {
            let binding = self.resolve_binding(target.sym.as_ref());
            let Some(method) = self.native_method_values.get(&binding).cloned() else {
                return Ok(None);
            };
            let label = format!("unbound native method `{}`", target.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
            let mut symbol = unbound_class_method_symbol(&method.symbol);
            let mut signature = self.signatures.get(&symbol).cloned().unwrap_or_else(|| {
                let mut signature = self.signatures[&method.symbol].clone();
                if method.receiver.is_some() {
                    signature.params.remove(0);
                }
                signature
            });
            let arguments = self.select_omitted_class_arguments(
                &mut symbol,
                &mut signature,
                arguments,
                0,
                &label,
            )?;
            return self
                .wrap_call_argument_bindings(
                    HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments),
                    &bindings,
                )
                .map(Some);
        }
        let Expr::Member(operation) = callee.as_ref() else {
            return Ok(None);
        };
        let Some(operation_name) = member_property_name(&operation.prop) else {
            return Ok(None);
        };
        if operation_name != "call" && operation_name != "apply" && operation_name != "bind" {
            return Ok(None);
        }
        let Expr::Ident(target) = operation.obj.as_ref() else {
            return Ok(None);
        };
        let binding = self.resolve_binding(target.sym.as_ref());
        let Some(method) = self.native_method_values.get(&binding).cloned() else {
            return Ok(None);
        };
        if operation_name == "bind" {
            return self
                .lower_saved_native_method_bind(target.sym.as_ref(), &method, call)
                .map(Some);
        }
        let Some((this_argument, supplied)) = call.args.split_first() else {
            return Err(format!(
                "unbound native method `{}.{operation_name}` expects a `thisArg`",
                target.sym
            ));
        };
        if this_argument.spread.is_some() {
            return Err(format!(
                "unbound native method `.{operation_name}()` cannot spread its `thisArg`"
            ));
        }
        let forwarded = if operation_name == "apply" {
            let [arguments] = supplied else {
                return Err(format!(
                    "unbound native method `{}.apply` expects exactly a `thisArg` and an argument tuple",
                    target.sym
                ));
            };
            if arguments.spread.is_some() {
                return Err("unbound native method `.apply()` tuple cannot be spread".into());
            }
            vec![swc_ecma_ast::ExprOrSpread {
                spread: Some(call.span),
                expr: arguments.expr.clone(),
            }]
        } else {
            supplied.to_vec()
        };
        if method.receiver.is_none() {
            let this_value = self.lower_expr(&this_argument.expr)?;
            let this_type = self.infer_expr_type(&this_value)?;
            let this_name = format!("__thaw_saved_static_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(this_name.clone(), this_type.clone());
            let label = format!("saved static method `{}.{operation_name}`", target.sym);
            let (arguments, mut bindings) = self.lower_native_spread_values(&forwarded, &label)?;
            let mut symbol = method.symbol.clone();
            let mut signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .expect("saved static method retains its signature");
            let arguments = self.select_omitted_class_arguments(
                &mut symbol,
                &mut signature,
                arguments,
                0,
                &label,
            )?;
            bindings.insert(0, (this_name, this_type, this_value));
            return self
                .wrap_call_argument_bindings(
                    HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments),
                    &bindings,
                )
                .map(Some);
        }
        let mut arguments = Vec::with_capacity(forwarded.len() + 1);
        arguments.push(this_argument.clone());
        arguments.extend(forwarded);
        let lowered = self.lower_call(&CallExpr {
            span: call.span,
            ctxt: call.ctxt,
            callee: Callee::Expr(Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                method.symbol.into(),
                call.span,
            )))),
            args: arguments,
            type_args: None,
        })?;
        Ok(Some(lowered))
    }

    fn lower_native_class_call_or_apply(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(operation) = callee.as_ref() else {
            return Ok(None);
        };
        let Some(operation_name) = member_property_name(&operation.prop) else {
            return Ok(None);
        };
        if operation_name != "call" && operation_name != "apply" {
            return Ok(None);
        }
        let Expr::Member(method) = operation.obj.as_ref() else {
            return Ok(None);
        };
        let Some(method_name) = member_property_name(&method.prop) else {
            return Ok(None);
        };
        let Some(target_type) = self.native_class_expression_type(&method.obj) else {
            return Ok(None);
        };
        let Some(class_name) = class_name_from_type(&target_type) else {
            return Ok(None);
        };
        if !self
            .signatures
            .contains_key(&class_method_symbol(class_name, &method_name))
        {
            return Ok(None);
        }
        if call.type_args.is_some() {
            return Err(format!(
                "native class method `.{operation_name}()` does not accept type arguments"
            ));
        }
        let Some((this_argument, supplied)) = call.args.split_first() else {
            return Err(format!(
                "native class method `{class_name}.{method_name}.{operation_name}` expects a `thisArg`"
            ));
        };
        if this_argument.spread.is_some() {
            return Err(format!(
                "native class method `.{operation_name}()` cannot spread its `thisArg`"
            ));
        }
        let forwarded = if operation_name == "apply" {
            let [arguments] = supplied else {
                return Err(format!(
                    "native class method `{class_name}.{method_name}.apply` expects exactly a `thisArg` and an argument tuple"
                ));
            };
            if arguments.spread.is_some() {
                return Err(
                    "native class method `.apply()` argument tuple cannot be spread".into(),
                );
            }
            vec![swc_ecma_ast::ExprOrSpread {
                spread: Some(call.span),
                expr: arguments.expr.clone(),
            }]
        } else {
            supplied.to_vec()
        };
        let target = self.lower_expr(&method.obj)?;
        self.expect_type(&target_type, &target, "method call target")?;
        let forwarded_call = CallExpr {
            span: call.span,
            ctxt: call.ctxt,
            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                span: call.span,
                obj: this_argument.expr.clone(),
                prop: method.prop.clone(),
            }))),
            args: forwarded,
            type_args: None,
        };
        let result = self.lower_call(&forwarded_call)?;
        let target_name = format!("__thaw_method_target_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), target_type.clone());
        self.wrap_call_argument_bindings(result, &[(target_name, target_type, target)])
            .map(Some)
    }

    fn lower_native_static_call_or_apply(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(operation) = callee.as_ref() else {
            return Ok(None);
        };
        let Some(operation_name) = member_property_name(&operation.prop) else {
            return Ok(None);
        };
        if operation_name != "call" && operation_name != "apply" {
            return Ok(None);
        }
        let Expr::Member(method) = operation.obj.as_ref() else {
            return Ok(None);
        };
        let Expr::Ident(class) = method.obj.as_ref() else {
            return Ok(None);
        };
        let Some(method_name) = member_property_name(&method.prop) else {
            return Ok(None);
        };
        let symbol = class_static_method_symbol(class.sym.as_ref(), &method_name);
        if !self.signatures.contains_key(&symbol) {
            return Ok(None);
        }
        let Some((this_argument, supplied)) = call.args.split_first() else {
            return Err(format!(
                "native static method `{}.{method_name}.{operation_name}` expects a `thisArg`",
                class.sym
            ));
        };
        if this_argument.spread.is_some() {
            return Err(format!(
                "native static method `.{operation_name}()` cannot spread its `thisArg`"
            ));
        }
        let forwarded = if operation_name == "apply" {
            let [arguments] = supplied else {
                return Err(format!(
                    "native static method `{}.{method_name}.apply` expects exactly a `thisArg` and an argument tuple",
                    class.sym
                ));
            };
            if arguments.spread.is_some() {
                return Err(
                    "native static method `.apply()` argument tuple cannot be spread".into(),
                );
            }
            vec![swc_ecma_ast::ExprOrSpread {
                spread: Some(call.span),
                expr: arguments.expr.clone(),
            }]
        } else {
            supplied.to_vec()
        };
        let this_value = self.lower_expr(&this_argument.expr)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let forwarded_call = CallExpr {
            span: call.span,
            ctxt: call.ctxt,
            callee: Callee::Expr(Box::new(Expr::Member(method.clone()))),
            args: forwarded,
            type_args: None,
        };
        let result = self.lower_call(&forwarded_call)?;
        let this_name = format!("__thaw_static_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        self.wrap_call_argument_bindings(result, &[(this_name, this_type, this_value)])
            .map(Some)
    }

    fn lower_immediately_invoked_function_bind(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Call(binding) = callee.as_ref() else {
            return Ok(None);
        };
        let Callee::Expr(bind_callee) = &binding.callee else {
            return Ok(None);
        };
        let Expr::Member(bind) = bind_callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&bind.prop).as_deref() != Some("bind") {
            return Ok(None);
        }
        if binding.type_args.is_some() || call.type_args.is_some() {
            return Err("function bind invocation does not accept type arguments".into());
        }
        let Some(bound) = self.lower_function_bind(binding)? else {
            return Ok(None);
        };
        let bound_type = self.infer_expr_type(&bound)?;
        let (params, optional, rest) = match &bound_type {
            HirType::Function(params, _) => (params.clone(), HirOptionalMask::default(), None),
            HirType::CallableFunction(params, optional, rest, _) => {
                (params.clone(), optional.clone(), rest.as_deref().cloned())
            }
            _ => return Ok(None),
        };
        let (mut arguments, argument_bindings) =
            self.lower_native_spread_values(&call.args, "bound function invocation")?;
        if rest.is_none() && arguments.len() > params.len() {
            return Err(format!(
                "bound function invocation expects {} argument(s), got {}",
                params.len(),
                arguments.len()
            ));
        }
        if arguments.len() < params.len()
            && (arguments.len()..params.len()).any(|index| !is_optional_parameter(&optional, index))
        {
            return Err(format!(
                "bound function invocation expects at least {} argument(s), got {}",
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
        let target_name = format!("__thaw_immediate_bound_function_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), bound_type.clone());
        let mut bindings = vec![(target_name.clone(), bound_type, bound)];
        bindings.extend(argument_bindings);
        self.wrap_call_argument_bindings(
            HirExpr::Call(Box::new(HirExpr::Var(target_name)), arguments),
            &bindings,
        )
        .map(Some)
    }

    fn lower_immediately_invoked_class_bind(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Call(binding) = callee.as_ref() else {
            return Ok(None);
        };
        let Callee::Expr(bind_callee) = &binding.callee else {
            return Ok(None);
        };
        let Expr::Member(bind) = bind_callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&bind.prop).as_deref() != Some("bind") {
            return Ok(None);
        }
        if !matches!(bind.obj.as_ref(), Expr::Member(_)) {
            return Ok(None);
        }
        if binding.type_args.is_some() || call.type_args.is_some() {
            return Err(
                "native class method bind invocation does not accept outer type arguments".into(),
            );
        }
        let mut arguments = binding.args.clone();
        arguments.extend(call.args.iter().cloned());
        let forwarded = CallExpr {
            span: call.span,
            ctxt: call.ctxt,
            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                span: bind.span,
                obj: bind.obj.clone(),
                prop: MemberProp::Ident(IdentName::new("call".into(), bind.span)),
            }))),
            args: arguments,
            type_args: None,
        };
        self.lower_native_class_call_or_apply(&forwarded)
    }

}
