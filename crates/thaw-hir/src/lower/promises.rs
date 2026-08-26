impl<'a> FnLowerer<'a> {
    fn lower_promise_callback(
        &mut self,
        expr: &Expr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let callback = match expr {
            Expr::Arrow(arrow) => {
                return self.lower_contextual_arrow(arrow, parameter_types, expected_return)
            }
            Expr::Fn(function) => {
                let arrow = function_expression_as_arrow(function)?;
                return self.lower_contextual_arrow(&arrow, parameter_types, expected_return);
            }
            Expr::Ident(ident) => {
                let mut name = self.resolve_binding(ident.sym.as_ref());
                if let Some(arrow) = self.generic_arrows.get(&name).cloned() {
                    return self.lower_stored_generic_arrow(
                        &name,
                        &arrow,
                        parameter_types,
                        expected_return,
                    );
                }
                if let Some(target) = self.generic_named_templates.get(&name) {
                    name = target.clone();
                }
                if self.scope.contains_key(&name) {
                    self.lower_expr(expr)?
                } else {
                    let signature = self
                        .signatures
                        .get(&name)
                        .ok_or_else(|| format!("unknown Promise callback `{name}`"))?;
                    if !signature.generic_type_params.is_empty() {
                        if signature.params.len() != parameter_types.len() {
                            return Err(format!(
                                "generic callback `{name}` expects {} argument(s), contextual call provides {}",
                                signature.params.len(),
                                parameter_types.len()
                            ));
                        }
                        let types = infer_generic_type_tuple(
                            signature,
                            parameter_types,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                        .map_err(|error| {
                            format!("cannot specialize generic callback `{name}`: {error}")
                        })?;
                        if let Some(constraints) = self.call_constraints {
                            constraints
                                .borrow_mut()
                                .push(CallConstraint::Generic(name.clone(), types.clone()));
                        }
                        let substitution = signature
                            .generic_type_params
                            .iter()
                            .cloned()
                            .zip(types.iter().cloned())
                            .collect::<HashMap<_, _>>();
                        let mut ret = resolve_ts_type_with_substitution(
                            signature
                                .generic_return_type
                                .as_ref()
                                .expect("generic callback return type"),
                            &substitution,
                            self.interfaces,
                            self.generic_interfaces,
                            &mut Vec::new(),
                        )?;
                        if signature.is_async && !matches!(ret, HirType::Promise(_)) {
                            ret = HirType::Promise(Box::new(ret));
                        }
                        let specialized = specialized_generic_function_name(
                            &name,
                            parameter_types,
                            signature,
                            &types,
                        );
                        return Ok(HirExpr::FunctionRef(
                            specialized,
                            parameter_types.to_vec(),
                            ret,
                        ));
                    }
                    let ret = if signature.is_async {
                        HirType::Promise(Box::new(signature.ret.clone()))
                    } else {
                        signature.ret.clone()
                    };
                    HirExpr::FunctionRef(name, signature.params.clone(), ret)
                }
            }
            _ => return Err("Promise callback must be an arrow or function value".into()),
        };
        let callback = if parameter_types
            .iter()
            .any(|ty| matches!(ty, HirType::Optional(_) | HirType::Nullish(_)))
        {
            if let Some(expected_return) = expected_return {
                let optional = parameter_types
                    .iter()
                    .map(|ty| matches!(ty, HirType::Optional(_) | HirType::Nullish(_)))
                    .collect::<Vec<_>>();
                let declared = HirType::CallableFunction(
                    parameter_types.to_vec(),
                    optional_parameter_mask(&optional),
                    None,
                    Box::new(expected_return.clone()),
                );
                self.adapt_named_function_to_callable(&declared, &callback)?
                    .unwrap_or(callback)
            } else {
                callback
            }
        } else {
            callback
        };
        self.validate_promise_callback_value(&callback, parameter_types, expected_return)?;
        Ok(callback)
    }

    fn validate_promise_callback_value(
        &mut self,
        callback: &HirExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<(), String> {
        let (params, ret) = match self.infer_expr_type(callback)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(mut params, _, rest, ret) => {
                if let Some(rest) = rest {
                    params.push(HirType::Array(rest));
                }
                (params, ret)
            }
            _ => return Err("Promise callback is not a function value".into()),
        };
        if params != parameter_types {
            return Err(format!(
                "Promise callback has parameters {params:?}, expected {parameter_types:?}"
            ));
        }
        if let Some(expected) = expected_return {
            if *ret != *expected {
                return Err(format!(
                    "Promise callback returns {:?}, expected {expected:?}",
                    ret
                ));
            }
        }
        Ok(())
    }

    fn callback_parameter_count(&self, expr: &Expr, label: &str) -> Result<usize, String> {
        match expr {
            Expr::Arrow(arrow) => Ok(arrow.params.len()),
            Expr::Fn(function) => Ok(function.function.params.len()),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _)
                        | HirType::CallableFunction(params, _, _, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown {label} `{name}`"))
            }
            _ => Err(format!("{label} must be an arrow or function value")),
        }
    }

    fn lower_promise_new(&mut self, new_expr: &swc_ecma_ast::NewExpr) -> Result<HirExpr, String> {
        let Expr::Ident(callee) = new_expr.callee.as_ref() else {
            return Err("only `new Promise<T>(...)` is supported".into());
        };
        if callee.sym != *"Promise" {
            return Err("only `new Promise<T>(...)` is supported".into());
        }
        let args = new_expr.args.as_deref().unwrap_or_default();
        let [executor] = args else {
            return Err("`new Promise<T>` expects exactly one executor".into());
        };
        let (spread_executor, spread_bindings) = if executor.spread.is_some() {
            let (executors, bindings) =
                self.lower_native_spread_values(args, "Promise executor")?;
            let [executor] = executors.as_slice() else {
                return Err(
                    "`new Promise<T>` expects exactly one executor after spread expansion".into(),
                );
            };
            (Some(executor.clone()), bindings)
        } else {
            (None, Vec::new())
        };
        let inferred_resolve = if let Some(executor) = &spread_executor {
            self.infer_promise_constructor_value_type(executor)
        } else {
            self.infer_promise_constructor_type(&executor.expr)
        };
        let (resolved, assimilates) = if let Some(type_args) = &new_expr.type_args {
            let [resolved] = type_args.params.as_slice() else {
                return Err("`new Promise` requires exactly one type argument".into());
            };
            let resolved = lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?;
            let assimilates = matches!(
                inferred_resolve.as_ref().ok(),
                Some(HirType::Promise(inner)) if inner.as_ref() == &resolved
            );
            (resolved, assimilates)
        } else {
            match inferred_resolve? {
                HirType::Promise(inner) => (*inner, true),
                resolved => (resolved, false),
            }
        };
        let resolve_value = if assimilates {
            vec![HirType::Promise(Box::new(resolved.clone()))]
        } else if resolved == HirType::Void {
            Vec::new()
        } else {
            vec![resolved.clone()]
        };
        let resolve = HirType::Function(resolve_value, Box::new(HirType::Void));
        let reject = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
        let arity = if let Some(executor) = &spread_executor {
            match self.infer_expr_type(executor)? {
                HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                    params.len()
                }
                _ => return Err("Promise executor is not a function value".into()),
            }
        } else {
            self.callback_parameter_count(&executor.expr, "Promise executor")?
        };
        if arity > 2 {
            return Err(format!(
                "Promise executor accepts at most two parameters, got {arity}"
            ));
        }
        let available = [resolve, reject];
        let executor = if let Some(executor) = spread_executor {
            self.validate_promise_callback_value(
                &executor,
                &available[..arity],
                Some(&HirType::Void),
            )?;
            executor
        } else {
            self.lower_promise_callback(
                &executor.expr,
                &available[..arity],
                Some(&HirType::Void),
            )?
        };
        let result = HirExpr::PromiseNew(
            Box::new(executor),
            resolved,
            assimilates,
        );
        self.wrap_call_argument_bindings(result, &spread_bindings)
    }

    fn infer_promise_constructor_value_type(
        &mut self,
        executor: &HirExpr,
    ) -> Result<HirType, String> {
        let (params, _) = match self.infer_expr_type(executor)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(params, _, _, ret) => (params, ret),
            _ => return Err("cannot infer Promise type from executor function".into()),
        };
        let Some(HirType::Function(resolve_params, _)) = params.first() else {
            return Err("cannot infer Promise type from executor resolve parameter".into());
        };
        let [resolved] = resolve_params.as_slice() else {
            return Err("Promise resolve callback must take exactly one value".into());
        };
        Ok(resolved.clone())
    }

    fn infer_promise_constructor_type(&mut self, executor: &Expr) -> Result<HirType, String> {
        use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};

        if let Expr::Ident(ident) = executor {
            let name = self.resolve_binding(ident.sym.as_ref());
            let callback_type = self.scope.get(&name).cloned().or_else(|| {
                self.signatures.get(&name).map(|signature| {
                    HirType::Function(
                        signature.params.clone(),
                        Box::new(if signature.is_async {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        }),
                    )
                })
            });
            let Some(HirType::Function(params, _)) = callback_type else {
                return Err("cannot infer Promise type from executor function".into());
            };
            let Some(HirType::Function(resolve_params, _)) = params.first() else {
                return Err("cannot infer Promise type from executor resolve parameter".into());
            };
            let [resolved] = resolve_params.as_slice() else {
                return Err("Promise resolve callback must take exactly one value".into());
            };
            return Ok(resolved.clone());
        }

        let Expr::Arrow(arrow) = executor else {
            return Err("cannot infer Promise type from this executor".into());
        };
        let Some(Pat::Ident(resolve_binding)) = arrow.params.first() else {
            return Err("cannot infer Promise type without a resolve parameter".into());
        };
        struct ResolveCalls {
            name: Symbol,
            values: Vec<Expr>,
            locals: HashMap<Symbol, Expr>,
        }
        impl Visit for ResolveCalls {
            fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
                if let (Pat::Ident(binding), Some(initializer)) =
                    (&declarator.name, &declarator.init)
                {
                    self.locals
                        .insert(binding.id.sym.to_string(), initializer.as_ref().clone());
                }
                declarator.visit_children_with(self);
            }

            fn visit_call_expr(&mut self, call: &CallExpr) {
                if let Callee::Expr(callee) = &call.callee {
                    if matches!(callee.as_ref(), Expr::Ident(ident) if ident.sym == self.name) {
                        if let [arg] = call.args.as_slice() {
                            if arg.spread.is_none() {
                                self.values.push(arg.expr.as_ref().clone());
                            }
                        }
                    }
                }
                call.visit_children_with(self);
            }
        }
        let mut calls = ResolveCalls {
            name: resolve_binding.id.sym.to_string(),
            values: Vec::new(),
            locals: HashMap::new(),
        };
        arrow.body.visit_with(&mut calls);

        struct ExpandExecutorLocals<'a> {
            locals: &'a HashMap<Symbol, Expr>,
            expanding: BTreeSet<Symbol>,
        }
        impl VisitMut for ExpandExecutorLocals<'_> {
            fn visit_mut_expr(&mut self, expr: &mut Expr) {
                if let Expr::Ident(ident) = expr {
                    let name = ident.sym.to_string();
                    if let Some(initializer) = self.locals.get(&name) {
                        if self.expanding.insert(name.clone()) {
                            let mut replacement = initializer.clone();
                            replacement.visit_mut_with(self);
                            self.expanding.remove(&name);
                            *expr = replacement;
                        }
                        return;
                    }
                }
                expr.visit_mut_children_with(self);
            }
        }
        let mut inferred = None;
        for mut value in calls.values {
            value.visit_mut_with(&mut ExpandExecutorLocals {
                locals: &calls.locals,
                expanding: BTreeSet::new(),
            });
            let value = self.lower_expr(&value).map_err(|error| {
                format!("cannot infer Promise type from resolve argument: {error}")
            })?;
            let actual = self.infer_expr_type(&value)?;
            if let Some(expected) = &inferred {
                if expected != &actual {
                    return Err(format!(
                        "conflicting Promise resolve types: {expected:?} and {actual:?}"
                    ));
                }
            } else {
                inferred = Some(actual);
            }
        }
        inferred.ok_or_else(|| {
            "cannot infer Promise type because the executor has no resolvable `resolve(value)` call"
                .into()
        })
    }
}
