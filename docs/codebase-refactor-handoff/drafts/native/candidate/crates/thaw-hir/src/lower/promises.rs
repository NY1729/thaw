#[derive(Clone, Copy, PartialEq, Eq)]
enum PromiseResolveKind {
    Value,
    Adopt,
    Mixed,
}

fn promise_resolved_argument(parameter: &HirType) -> (HirType, PromiseResolveKind) {
    if let HirType::Optional(inner) = parameter {
        if matches!(inner.as_ref(), HirType::Promise(value)
            if matches!(value.as_ref(), HirType::Void | HirType::Undefined)) {
            let HirType::Promise(value) = inner.as_ref() else { unreachable!() };
            return (value.as_ref().clone(), PromiseResolveKind::Mixed);
        }
    }
    if let HirType::Nullable(inner) = parameter {
        if matches!(inner.as_ref(), HirType::Promise(value)
            if value.as_ref() == &HirType::Null) {
            return (HirType::Null, PromiseResolveKind::Mixed);
        }
    }
    if let HirType::Union(parts) = parameter {
        if let [value, HirType::Promise(promised)] = parts.as_slice() {
            if value == promised.as_ref() {
                return (value.clone(), PromiseResolveKind::Mixed);
            }
        }
    }
    if let HirType::Promise(value) = parameter {
        return (value.as_ref().clone(), PromiseResolveKind::Adopt);
    }
    (parameter.clone(), PromiseResolveKind::Value)
}

fn promise_resolver_parameter(callback: &HirType) -> Result<(HirType, PromiseResolveKind), String> {
    let (params, ret) = match callback {
        HirType::Function(params, ret) => (params, ret),
        HirType::CallableFunction(params, _, None, ret) => (params, ret),
        _ => return Err("Promise resolve callback is not a function".into()),
    };
    if ret.as_ref() != &HirType::Void {
        return Err("Promise resolve callback must return void".into());
    }
    match params.as_slice() {
        [] => Ok((HirType::Void, PromiseResolveKind::Value)),
        [parameter] => Ok(promise_resolved_argument(parameter)),
        _ => Err("Promise resolve callback must take at most one value".into()),
    }
}

fn promise_mixed_resolver_type(resolved: &HirType, omittable: bool) -> HirType {
    let parameter = if matches!(resolved, HirType::Void | HirType::Undefined) {
        HirType::Optional(Box::new(HirType::Promise(Box::new(resolved.clone()))))
    } else if resolved == &HirType::Null {
        HirType::Nullable(Box::new(HirType::Promise(Box::new(HirType::Null))))
    } else {
        HirType::Union(vec![
            resolved.clone(), HirType::Promise(Box::new(resolved.clone())),
        ])
    };
    if omittable {
        HirType::CallableFunction(
            vec![parameter],
            HirOptionalMask::from_bools(&[true]),
            None,
            Box::new(HirType::Void),
        )
    } else {
        HirType::Function(vec![parameter], Box::new(HirType::Void))
    }
}

impl<'a> FnLowerer<'a> {
    fn promise_rejection_snapshot_name(binding: &str, field: &str) -> String {
        format!("@@thaw_promise_rejection:{binding}:{field}")
    }

    fn snapshot_promise_rejection(&mut self, binding: &str) -> Vec<HirStmt> {
        let fields = [
            ("original", HirType::Str, None),
            ("native_text", HirType::Str, Some("__thaw_pending_exception_native_text")),
            ("aggregate", HirType::Object(Vec::new()), Some("__thaw_pending_exception_aggregate")),
            ("tag", HirType::I64, Some("__thaw_pending_exception_tag")),
            ("f64", HirType::F64, Some("__thaw_pending_exception_f64")),
            ("i64", HirType::I64, Some("__thaw_pending_exception_i64")),
            ("bool", HirType::Bool, Some("__thaw_pending_exception_bool")),
            ("object", HirType::Object(Vec::new()), Some("__thaw_pending_exception_object")),
            ("native", HirType::NativeException, Some("__thaw_pending_exception_native")),
        ];
        fields.into_iter().map(|(field, ty, getter)| {
            let name = Self::promise_rejection_snapshot_name(binding, field);
            self.scope.insert(name.clone(), ty.clone());
            let value = if let Some(getter) = getter {
                HirExpr::Call(Box::new(HirExpr::Var(getter.to_string())), Vec::new())
            } else {
                HirExpr::Var(binding.to_string())
            };
            HirStmt::Let(name, ty, value)
        }).collect()
    }
    fn lower_promise_rejection_callback(
        &mut self,
        expr: &Expr,
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let parameter = match expr {
            Expr::Arrow(arrow) => arrow.params.first(),
            Expr::Fn(function) => function.function.params.first().map(|param| &param.pat),
            _ => None,
        }
        .and_then(|param| match param {
            Pat::Ident(binding) => Some(binding.id.sym.to_string()),
            _ => None,
        });
        let saved = std::mem::replace(&mut self.promise_catch_parameter, parameter);
        let result = self.lower_promise_callback(expr, &[HirType::Str], expected_return);
        self.promise_catch_parameter = saved;
        result
    }

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
                return self.lower_function_expression(function, Some((parameter_types, expected_return)));
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
                            None,
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
            _ => self.lower_expr(expr)?,
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
        self.normalize_promise_callback(callback, parameter_types)
    }

    fn validate_promise_callback_value(
        &mut self,
        callback: &HirExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<(), String> {
        let (params, rest, ret) = match self.infer_expr_type(callback)? {
            HirType::Function(mut params, ret) => {
                let rest = if let HirExpr::FunctionRef(symbol, _, _) = callback {
                    self.signatures.get(symbol).and_then(|signature| signature.native_rest.clone())
                } else { None };
                if rest.is_some() { params.pop(); }
                (params, rest.map(Box::new), ret)
            },
            HirType::CallableFunction(params, _, rest, ret) => (params, rest, ret),
            _ => return Err("Promise callback is not a function value".into()),
        };
        if params.len() > parameter_types.len()
            || !params.iter().zip(parameter_types).all(|(actual, supplied)| {
                callback_param_compatible(supplied, actual)
                    || match actual {
                        HirType::Optional(payload)
                        | HirType::Nullable(payload)
                        | HirType::Nullish(payload) => callback_param_compatible(supplied, payload),
                        _ => false,
                    }
            })
            || rest.as_ref().is_some_and(|element| {
                parameter_types[params.len()..].iter().any(|supplied| {
                    !callback_param_compatible(supplied, element)
                })
            })
        {
            return Err(format!(
                "Promise callback has parameters {params:?} and rest {rest:?}, expected a prefix of {parameter_types:?}"
            ));
        }
        if let Some(expected) = expected_return {
            if !callable_return_compatible(expected, &ret) {
                return Err(format!(
                    "Promise callback returns {:?}, expected {expected:?}",
                    ret
                ));
            }
        }
        Ok(())
    }

    /// Bind a callback expression once and expose the exact fixed Promise
    /// callback ABI. Callable optional/rest slots and zero-argument functions
    /// are invoked through their own original signature inside the wrapper.
    fn normalize_promise_callback(
        &mut self,
        callback: HirExpr,
        supplied: &[HirType],
    ) -> Result<HirExpr, String> {
        let original = self.infer_expr_type(&callback)?;
        let (fixed, rest, ret) = match &original {
            HirType::Function(fixed, ret) => {
                let rest = if let HirExpr::FunctionRef(symbol, _, _) = &callback {
                    self.signatures.get(symbol).and_then(|signature| signature.native_rest.clone())
                } else { None };
                let mut fixed = fixed.clone();
                if rest.is_some() { fixed.pop(); }
                (fixed, rest, ret.as_ref().clone())
            },
            HirType::CallableFunction(fixed, _, rest, ret) => {
                (fixed.clone(), rest.as_deref().cloned(), ret.as_ref().clone())
            }
            _ => return Err("Promise callback is not a function value".into()),
        };
        if matches!(&original, HirType::Function(..)) && rest.is_none() && fixed == supplied {
            return Ok(callback);
        }
        let callback_name = format!("__thaw_promise_callback_{}", self.next_binding);
        self.next_binding += 1;
        let input_names = (0..supplied.len())
            .map(|index| format!("__thaw_promise_argument_{index}_{}", self.next_binding))
            .collect::<Vec<_>>();
        let inputs = input_names.iter().zip(supplied).map(|(name, ty)| HirParam {
            name: name.clone(), ty: ty.clone(),
        }).collect::<Vec<_>>();
        for (name, ty) in input_names.iter().zip(supplied) {
            self.scope.insert(name.clone(), ty.clone());
        }
        let arguments = (|| -> Result<Vec<HirExpr>, String> {
            let mut arguments = Vec::new();
            for (index, expected) in fixed.iter().enumerate() {
                let input = HirExpr::Var(input_names[index].clone());
                arguments.push(self.coerce_to_declared(expected, input)?);
            }
            if let Some(element) = rest {
                let remaining = input_names.iter().skip(fixed.len())
                    .map(|name| self.coerce_to_declared(&element, HirExpr::Var(name.clone())))
                    .collect::<Result<Vec<_>, _>>()?;
                arguments.push(HirExpr::ArrayLit(remaining));
            }
            Ok(arguments)
        })();
        for name in &input_names { self.scope.remove(name); }
        let arguments = arguments?;
        let call = HirExpr::Call(Box::new(HirExpr::Var(callback_name.clone())), arguments);
        let body = if ret == HirType::Void {
            HirExpr::Block(vec![HirStmt::Expr(call), HirStmt::Return(None)])
        } else {
            HirExpr::Block(vec![HirStmt::Return(Some(call))])
        };
        let normalized = HirExpr::Lambda(
            vec![HirParam { name: callback_name.clone(), ty: original.clone() }],
            inputs,
            ret.clone(),
            Box::new(body),
        );
        let result_type = HirType::Function(supplied.to_vec(), Box::new(ret));
        let binder = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam { name: callback_name, ty: original }],
            result_type,
            Box::new(HirExpr::Block(vec![HirStmt::Return(Some(normalized))])),
        );
        Ok(HirExpr::Call(Box::new(binder), vec![callback]))
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

    fn declared_promise_resolver(
        &mut self,
        executor: &Expr,
    ) -> Result<Option<HirType>, String> {
        let annotation = match executor {
            Expr::Arrow(arrow) => arrow.params.first().and_then(|parameter| match parameter {
                Pat::Ident(binding) => binding.type_ann.as_ref(),
                Pat::Assign(assignment) => match assignment.left.as_ref() {
                    Pat::Ident(binding) => binding.type_ann.as_ref(),
                    _ => None,
                },
                _ => None,
            }),
            Expr::Fn(function) => function.function.params.first().and_then(|parameter| {
                match &parameter.pat {
                    Pat::Ident(binding) => binding.type_ann.as_ref(),
                    _ => None,
                }
            }),
            _ => None,
        };
        if let Some(annotation) = annotation {
            if matches!(annotation.type_ann.as_ref(),
                TsType::TsKeywordType(keyword)
                    if keyword.kind == TsKeywordTypeKind::TsAnyKeyword)
            {
                return Ok(None);
            }
            return lower_ts_type(
                &annotation.type_ann,
                self.interfaces,
                self.generic_interfaces,
            ).map(Some);
        }
        if let Expr::Ident(ident) = executor {
            let name = self.resolve_binding(ident.sym.as_ref());
            if let Some(ty) = self.scope.get(&name) {
                return Ok(match ty {
                    HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                        params.first().filter(|param| *param != &HirType::Dynamic).cloned()
                    }
                    _ => None,
                });
            }
            return Ok(self.signatures.get(&name).and_then(|signature| {
                signature.params.first().filter(|param| *param != &HirType::Dynamic).cloned()
            }));
        }
        Ok(None)
    }

    /// `new Function(...args)` -- the args are the parameter names followed
    /// by the body. An AOT compiler can't compile a runtime source string,
    /// so this is lowered to the JS realm's own `Function` constructor via
    /// `newDynamicFunction`, which caches the compiled function per source
    /// (`thaw_js_new_function`).
    fn lower_function_constructor(
        &mut self,
        new_expr: &swc_ecma_ast::NewExpr,
    ) -> Result<HirExpr, String> {
        let args = new_expr.args.clone().unwrap_or_default();
        if args.iter().any(|argument| argument.spread.is_some()) {
            return Err("`new Function()` does not support spread arguments".into());
        }
        // A: a dynamic source string goes to the JS realm's own `Function`
        // constructor (cached per source in `thaw_js_new_function`).
        let mut values = Vec::with_capacity(args.len());
        for argument in &args {
            let value = self.lower_expr(&argument.expr)?;
            let value = self.coerce_primitive_to_string(value)?;
            values.push(self.coerce_to_declared(&HirType::Json, value)?);
        }
        let args_json = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("newDynamicFunction".into())),
            vec![args_json],
        ))
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
        let explicit = if let Some(type_args) = &new_expr.type_args {
            let [resolved] = type_args.params.as_slice() else {
                return Err("`new Promise` requires exactly one type argument".into());
            };
            Some(lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?)
        } else {
            None
        };
        let declared_resolver = if let Some(executor) = &spread_executor {
            match self.infer_expr_type(executor)? {
                HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                    params.first().filter(|param| *param != &HirType::Dynamic).cloned()
                }
                _ => return Err("Promise executor is not a function value".into()),
            }
        } else {
            self.declared_promise_resolver(&executor.expr)?
        };
        let inferred_resolve = if let Some(executor) = &spread_executor {
            self.infer_promise_constructor_value_type(executor, explicit.as_ref())
                .map(|value| (value, true))
        } else {
            self.infer_promise_constructor_type(&executor.expr, explicit.as_ref())
        };
        if let Err(error) = &inferred_resolve {
            if explicit.is_none()
                || !error.starts_with(
                    "cannot infer Promise type because the executor has no resolvable",
                )
            {
                return Err(error.clone());
            }
        }
        let inferred = inferred_resolve.as_ref().ok()
            .map(|(result, observed)| (result.clone(), *observed));
        let legacy_unobserved_void = explicit.is_none()
            && matches!(inferred.as_ref(), Some((HirType::Void, false)));
        let resolved = if let Some(explicit) = explicit {
            if let Some((result, observed)) = inferred {
                let compatible = if declared_resolver.is_some() {
                    result == explicit
                } else {
                    promise_resolve_value_fits(&result, &explicit)
                };
                if observed && !compatible {
                    return Err(format!(
                        "Promise resolve value has type {result:?}, expected {explicit:?}"
                    ));
                }
            }
            explicit
        } else {
            inferred_resolve.map(|(result, _)| result)?
        };
        let kind = if let Some(declared) = &declared_resolver {
            let (declared_result, kind) = promise_resolver_parameter(declared)?;
            if declared_result != resolved {
                return Err(format!(
                    "Promise resolve callback has result {declared_result:?}, expected {resolved:?}"
                ));
            }
            kind
        } else if legacy_unobserved_void {
            PromiseResolveKind::Value
        } else {
            PromiseResolveKind::Mixed
        };
        let resolve = if kind == PromiseResolveKind::Mixed {
            declared_resolver.unwrap_or_else(|| {
                promise_mixed_resolver_type(&resolved, resolved == HirType::Void)
            })
        } else {
            let resolve_value = if kind == PromiseResolveKind::Adopt {
                vec![HirType::Promise(Box::new(resolved.clone()))]
            } else if resolved == HirType::Void {
                Vec::new()
            } else {
                vec![resolved.clone()]
            };
            HirType::Function(resolve_value, Box::new(HirType::Void))
        };
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
            self.normalize_promise_callback(executor, &available[..arity])?
        } else {
            self.lower_promise_callback(
                &executor.expr,
                &available[..arity],
                Some(&HirType::Void),
            )?
        };
        let result = if kind == PromiseResolveKind::Mixed {
            HirExpr::PromiseNewMixed(Box::new(executor), resolved, available[0].clone())
        } else {
            HirExpr::PromiseNew(
                Box::new(executor), resolved,
                kind == PromiseResolveKind::Adopt, false,
            )
        };
        self.wrap_call_argument_bindings(result, &spread_bindings)
    }

    fn infer_promise_constructor_value_type(
        &mut self,
        executor: &HirExpr,
        explicit: Option<&HirType>,
    ) -> Result<HirType, String> {
        let (params, _) = match self.infer_expr_type(executor)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(params, _, _, ret) => (params, ret),
            _ => return Err("cannot infer Promise type from executor function".into()),
        };
        let Some(resolve) = params.first() else {
            return Err("cannot infer Promise type from executor resolve parameter".into());
        };
        if resolve == &HirType::Dynamic {
            return explicit.cloned().ok_or(
                "cannot infer Promise type from a generic executor without a type argument".into(),
            );
        }
        Ok(promise_resolver_parameter(resolve)?.0)
    }

    fn infer_promise_constructor_type(
        &mut self,
        executor: &Expr,
        explicit: Option<&HirType>,
    ) -> Result<(HirType, bool), String> {
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
            let Some(HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _)) = callback_type else {
                return Err("cannot infer Promise type from executor function".into());
            };
            let Some(resolve) = params.first() else {
                return Err("cannot infer Promise type from executor resolve parameter".into());
            };
            if resolve == &HirType::Dynamic {
                return explicit.cloned().map(|resolved| (resolved, false))
                    .ok_or("cannot infer Promise type from a generic executor without a type argument".into());
            }
            return Ok((promise_resolver_parameter(resolve)?.0, true));
        }

        let resolve_binding = match executor {
            Expr::Arrow(arrow) => arrow.params.first(),
            Expr::Fn(function) => function.function.params.first().map(|param| &param.pat),
            _ => return Err("cannot infer Promise type from this executor".into()),
        };
        let Some(resolve_binding) = resolve_binding else {
            return Err("cannot infer Promise type without a resolve parameter".into());
        };
        let resolve_binding = match resolve_binding {
            Pat::Ident(binding) => binding,
            Pat::Assign(assignment) => match assignment.left.as_ref() {
                Pat::Ident(binding) => binding,
                _ => return Err("cannot infer Promise type without a resolve parameter".into()),
            },
            _ => return Err("cannot infer Promise type without a resolve parameter".into()),
        };
        if let Some(annotation) = &resolve_binding.type_ann {
            if !matches!(annotation.type_ann.as_ref(),
                TsType::TsKeywordType(keyword)
                    if keyword.kind == TsKeywordTypeKind::TsAnyKeyword)
            {
                let declared = lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )?;
                return Ok((promise_resolver_parameter(&declared)?.0, true));
            }
        }
        struct ResolveCalls {
            name: Symbol,
            values: Vec<(Expr, bool)>,
            zero_arg_calls: usize,
            locals: HashMap<Symbol, Expr>,
            // Whether `resolve` is referenced anywhere at all, in any
            // position -- including handed off *by reference* rather
            // than called directly (`setTimeout(resolve, ms)`, the
            // standard zero-dependency "sleep" idiom, or a one-shot
            // event listener/"done" callback). `values` alone can't see
            // this: `resolve` never appears in call-callee position at
            // all in that shape.
            referenced: bool,
        }
        impl ResolveCalls {
            fn shadows_resolve(&self, pattern: &Pat) -> bool {
                match pattern {
                    Pat::Ident(binding) => binding.id.sym == self.name,
                    Pat::Assign(assignment) => self.shadows_resolve(&assignment.left),
                    _ => false,
                }
            }
        }
        impl Visit for ResolveCalls {
            fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
                if !arrow.params.iter().any(|pattern| self.shadows_resolve(pattern)) {
                    arrow.visit_children_with(self);
                }
            }

            fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
                if !function.params.iter().any(|param| self.shadows_resolve(&param.pat)) {
                    function.visit_children_with(self);
                }
            }

            fn visit_call_expr(&mut self, call: &CallExpr) {
                if let Callee::Expr(callee) = &call.callee {
                    if let Expr::Ident(ident) = callee.as_ref() {
                        let mut current = ident.sym.to_string();
                        let mut seen = BTreeSet::new();
                        while current != self.name && seen.insert(current.clone()) {
                            let Some(Expr::Ident(next)) = self.locals.get(&current) else {
                                break;
                            };
                            current = next.sym.to_string();
                        }
                        if current == self.name {
                            if let [arg] = call.args.as_slice() {
                                self.values.push((arg.expr.as_ref().clone(), arg.spread.is_some()));
                            } else if call.args.is_empty() {
                                self.zero_arg_calls += 1;
                            }
                        }
                    }
                }
                call.visit_children_with(self);
            }

            fn visit_expr(&mut self, expr: &Expr) {
                if let Expr::Ident(ident) = expr {
                    if ident.sym == self.name {
                        self.referenced = true;
                    }
                }
                expr.visit_children_with(self);
            }

            fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
                if let (Pat::Ident(binding), Some(initializer)) =
                    (&declarator.name, &declarator.init)
                {
                    self.locals
                        .insert(binding.id.sym.to_string(), initializer.as_ref().clone());
                }
                declarator.visit_children_with(self);
            }

        }
        let mut calls = ResolveCalls {
            name: resolve_binding.id.sym.to_string(),
            values: Vec::new(),
            zero_arg_calls: 0,
            locals: HashMap::new(),
            referenced: false,
        };
        match executor {
            Expr::Arrow(arrow) => arrow.body.visit_with(&mut calls),
            Expr::Fn(function) => {
                if let Some(body) = &function.function.body {
                    body.visit_with(&mut calls);
                }
            }
            _ => unreachable!(),
        }

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
        for (mut value, mut spread) in calls.values {
            if spread {
                let single = match &value {
                    Expr::Array(array) => match array.elems.as_slice() {
                        [Some(element)] if element.spread.is_none() => {
                            Some(element.expr.as_ref().clone())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(single) = single {
                    value = single;
                    spread = false;
                }
            }
            value.visit_mut_with(&mut ExpandExecutorLocals {
                locals: &calls.locals,
                expanding: BTreeSet::new(),
            });
            let value = self.lower_expr(&value).map_err(|error| {
                format!("cannot infer Promise type from resolve argument: {error}")
            })?;
            let actual = self.infer_expr_type(&value)?;
            let actual = if spread {
                match actual {
                    HirType::Tuple(mut values) if values.len() == 1 => values.remove(0),
                    other => return Err(format!(
                        "Promise resolve spread must supply one static value, got {other:?}"
                    )),
                }
            } else {
                actual
            };
            let (actual, _) = promise_resolved_argument(&actual);
            if let Some(expected) = explicit {
                if !promise_resolve_value_fits(&actual, expected) {
                    return Err(format!(
                        "Promise resolve value has type {actual:?}, expected {expected:?}"
                    ));
                }
                inferred = Some(expected.clone());
                continue;
            }
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
        if calls.zero_arg_calls > 0 {
            if explicit.is_some_and(|expected| *expected != HirType::Void)
                || inferred.as_ref().is_some_and(|inferred| *inferred != HirType::Void)
            {
                return Err("Promise resolve() without a value requires Promise<void>".into());
            }
            return Ok((HirType::Void, true));
        }
        if let Some(inferred) = inferred {
            return Ok((inferred, true));
        }
        if calls.referenced {
            // `resolve` is handed off by reference (no direct
            // `resolve(value)` call anywhere in the visible source) --
            // defaults to `void`, matching the overwhelmingly common
            // reason to do this (a timer, a one-shot event listener, a
            // "done" callback signaling completion with no value). An
            // explicit `new Promise<T>(...)` type argument still
            // overrides this when the callback truly resolves with a
            // value some other way this static scan can't see.
            return Ok((HirType::Void, false));
        }
        Err("cannot infer Promise type because the executor has no resolvable `resolve(value)` call"
            .into())
    }
}
