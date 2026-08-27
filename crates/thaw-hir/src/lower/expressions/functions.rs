impl<'a> FnLowerer<'a> {
    fn lower_satisfies(
        &mut self,
        satisfies: &swc_ecma_ast::TsSatisfiesExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&satisfies.expr)?;
        let expected = lower_ts_type(
            &satisfies.type_ann,
            self.interfaces,
            self.generic_interfaces,
        )?;
        self.coerce_to_declared(&expected, value.clone())?;
        Ok(value)
    }

    fn lower_non_null_assertion(
        &mut self,
        assertion: &swc_ecma_ast::TsNonNullExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&assertion.expr)?;
        match self.infer_expr_type(&value)? {
            HirType::Optional(payload) => Ok(HirExpr::OptionalValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullable(payload) => Ok(HirExpr::NullableValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullish(payload) => Ok(HirExpr::NullishValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Null | HirType::Undefined => Err(
                "non-null assertion cannot produce a native value from a statically null or undefined expression"
                    .into(),
            ),
            _ => Ok(value),
        }
    }

    fn lower_recursive_function_expression(
        &mut self,
        outer_name: &str,
        expression: &swc_ecma_ast::FnExpr,
        annotated: Option<&HirType>,
    ) -> Result<Option<(Symbol, HirType, HirExpr)>, String> {
        if !named_function_is_recursive(expression) {
            return Ok(None);
        }
        if expression.function.type_params.is_some() {
            return Ok(None);
        }
        let internal = expression
            .ident
            .as_ref()
            .expect("recursive named function expression")
            .sym
            .to_string();
        let arrow = function_expression_as_arrow(expression)?;
        let ty = if let Some(annotated) = annotated {
            annotated.clone()
        } else {
            let params = arrow
                .params
                .iter()
                .map(|parameter| {
                    lower_param(
                        parameter,
                        self.interfaces,
                        self.generic_interfaces,
                        false,
                        &HashMap::new(),
                    )
                    .map(|parameter| parameter.ty)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let result = arrow
                .return_type
                .as_ref()
                .ok_or_else(|| "recursive local function values need a return annotation or an outer function type".to_string())
                .and_then(|annotation| {
                    lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )
                })?;
            HirType::Function(params, Box::new(result))
        };
        let HirType::Function(params, ret) = &ty else {
            return Err("recursive named function expression needs a function type".into());
        };
        let hir_name = self.bind_local(outer_name, ty.clone());
        let saved_internal = (internal != outer_name)
            .then(|| self.bindings.get(&internal).cloned())
            .flatten();
        if internal != outer_name {
            self.bindings
                .entry(internal.clone())
                .or_default()
                .push(hir_name.clone());
        }
        let lowered = self.lower_contextual_arrow(&arrow, params, Some(ret));
        if internal != outer_name {
            if let Some(saved) = saved_internal {
                self.bindings.insert(internal, saved);
            } else {
                self.bindings.remove(&internal);
            }
        }
        let closure = self.coerce_to_declared(&ty, lowered?)?;
        Ok(Some((
            hir_name.clone(),
            ty.clone(),
            HirExpr::RecursiveClosure(hir_name, ty, Box::new(closure)),
        )))
    }

    fn lower_arrow(&mut self, arrow: &swc_ecma_ast::ArrowExpr) -> Result<HirExpr, String> {
        if arrow.is_generator || arrow.type_params.is_some() {
            return Err("generator and generic arrow functions are not supported yet".into());
        }
        let source_params = arrow
            .params
            .iter()
            .map(|param| {
                lower_param(
                    param,
                    self.interfaces,
                    self.generic_interfaces,
                    false,
                    &HashMap::new(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let declared_return = arrow
            .return_type
            .as_ref()
            .map(|ann| lower_ts_type(&ann.type_ann, self.interfaces, self.generic_interfaces))
            .transpose()?;

        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(source_params.len());
            let mut destructuring = Vec::new();
            for (pattern, param) in arrow.params.iter().zip(source_params) {
                let name = self.bind_local(&param.name, param.ty.clone());
                if !matches!(pattern, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pattern, name.clone(), param.ty.clone()));
                }
                params.push(HirParam { name, ty: param.ty });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            let declared_async_result = if arrow.is_async {
                match &declared_return {
                    Some(HirType::Promise(result)) => Some(result.as_ref().clone()),
                    Some(other) => {
                        return Err(format!(
                            "async arrow return annotation must be Promise<T>, got {other:?}"
                        ))
                    }
                    None => None,
                }
            } else {
                None
            };
            self.ret_type = declared_async_result
                .clone()
                .or_else(|| declared_return.clone())
                .unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let mut expression = self.lower_expr(expr)?;
                    let mut inferred = self.infer_expr_type(&expression)?;
                    if arrow.is_async {
                        if let HirExpr::AwaitPromise(promise, resolved) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else if let HirExpr::Await(promise) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(inferred));
                        }
                        let resolved = declared_async_result.clone().unwrap_or_else(|| {
                            if let HirType::Promise(inner) = &inferred {
                                inner.as_ref().clone()
                            } else {
                                inferred.clone()
                            }
                        });
                        if contains_await(&expression) {
                            expression = self.coerce_to_declared(&resolved, expression)?;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else {
                            let assimilates = matches!(
                                &inferred,
                                HirType::Promise(inner) if inner.as_ref() == &resolved
                            );
                            if !assimilates {
                                expression = self.coerce_to_declared(&resolved, expression)?;
                            }
                            let resolve_type = HirType::Function(
                                vec![if assimilates {
                                    HirType::Promise(Box::new(resolved.clone()))
                                } else {
                                    resolved.clone()
                                }],
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&expression, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name.clone(),
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name)),
                                    vec![expression],
                                )),
                            );
                            expression = HirExpr::PromiseNew(
                                Box::new(executor),
                                resolved.clone(),
                                assimilates,
                            );
                            inferred = HirType::Promise(Box::new(resolved));
                        }
                    } else if let Some(expected) = &declared_return {
                        expression = self.coerce_to_declared(expected, expression)?;
                        inferred = self.infer_expr_type(&expression)?;
                    }
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    if arrow.is_async {
                        let has_await = stmts.iter().any(stmt_contains_await);
                        let only_tail_awaits =
                            has_await && async_arrow_has_only_tail_await_returns(&stmts);
                        let resolved = declared_async_result.clone().unwrap_or(inferred);
                        if has_await && !only_tail_awaits {
                            (HirExpr::Block(stmts), HirType::Promise(Box::new(resolved)))
                        } else {
                            let assimilates = if only_tail_awaits {
                                stmts = strip_async_arrow_tail_awaits(stmts);
                                true
                            } else {
                                false
                            };
                            let resolve_type = HirType::Function(
                                if assimilates {
                                    vec![HirType::Promise(Box::new(resolved.clone()))]
                                } else if resolved == HirType::Void {
                                    Vec::new()
                                } else {
                                    vec![resolved.clone()]
                                },
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut executor_body =
                                rewrite_async_arrow_returns(stmts, &resolve_name, &resolved)?;
                            if resolved == HirType::Void && !assimilates {
                                executor_body.push(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name.clone())),
                                    Vec::new(),
                                )));
                            }
                            let executor_body = HirExpr::Block(executor_body);
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&executor_body, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter(|name| name != &resolve_name)
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name,
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(executor_body),
                            );
                            (
                                HirExpr::PromiseNew(
                                    Box::new(executor),
                                    resolved.clone(),
                                    assimilates,
                                ),
                                HirType::Promise(Box::new(resolved)),
                            )
                        }
                    } else {
                        (HirExpr::Block(stmts), inferred)
                    }
                }
            };
            let return_type = declared_return.clone().unwrap_or(inferred_return);
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    saved_scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();
            Ok(HirExpr::Lambda(
                captures,
                params,
                return_type,
                Box::new(body),
            ))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }

    fn lower_generic_instantiation_expression(
        &mut self,
        instantiation: &swc_ecma_ast::TsInstantiation,
    ) -> Result<HirExpr, String> {
        let Expr::Ident(identifier) = instantiation.expr.as_ref() else {
            return Err(
                "generic instantiation expressions require a named top-level function".into(),
            );
        };
        let name = identifier.sym.to_string();
        let signature = self
            .signatures
            .get(&name)
            .ok_or_else(|| format!("unknown function `{name}` in instantiation expression"))?;
        if signature.generic_type_params.is_empty() {
            return Err(format!(
                "non-generic function `{name}` cannot be used in an instantiation expression"
            ));
        }
        let types = resolve_explicit_generic_type_tuple(
            signature,
            &instantiation.type_args.params,
            &[],
            self.interfaces,
            self.generic_interfaces,
        )?;
        for ty in &types {
            if !supports_generic_native_layout(ty) {
                return Err(format!(
                    "generic function `{name}` cannot specialize for native layout {ty:?}"
                ));
            }
        }
        let substitution = signature
            .generic_type_params
            .iter()
            .cloned()
            .zip(types.iter().cloned())
            .collect::<HashMap<_, _>>();
        let params = signature
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ret = resolve_ts_type_with_substitution(
            signature
                .generic_return_type
                .as_ref()
                .expect("generic instantiation return type"),
            &substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        if signature.is_async && !matches!(ret, HirType::Promise(_)) {
            ret = HirType::Promise(Box::new(ret));
        }
        if let Some(constraints) = self.call_constraints {
            constraints
                .borrow_mut()
                .push(CallConstraint::Generic(name.clone(), types.clone()));
        }
        let specialized = specialized_generic_function_name(&name, &params, signature, &types);
        Ok(HirExpr::FunctionRef(specialized, params, ret))
    }

    fn lower_stored_generic_arrow(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let Some(internal) = self.generic_arrow_self_names.get(name).cloned() else {
            return self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        };
        let signature = self.generic_arrow_signature(arrow)?;
        let concrete_types = infer_generic_type_tuple(
            &signature,
            parameter_types,
            self.interfaces,
            self.generic_interfaces,
        )?;
        let return_type =
            if let Some(expected) = expected_return.filter(|ty| **ty != HirType::Dynamic) {
                expected.clone()
            } else {
                let declared = signature.generic_return_type.as_ref().ok_or_else(|| {
                    format!("recursive generic local function `{name}` needs a return annotation")
                })?;
                let substitution = signature
                    .generic_type_params
                    .iter()
                    .cloned()
                    .zip(concrete_types)
                    .collect::<HashMap<_, _>>();
                resolve_ts_type_with_substitution(
                    declared,
                    &substitution,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )?
            };
        let self_type = HirType::Function(parameter_types.to_vec(), Box::new(return_type));
        let self_name = format!("__thaw_recursive_generic_{}", self.next_binding);
        self.next_binding += 1;
        let saved_binding = self.bindings.get(&internal).cloned();
        self.scope.insert(self_name.clone(), self_type.clone());
        self.bindings
            .entry(internal.clone())
            .or_default()
            .push(self_name.clone());
        let lowered = self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        self.scope.remove(&self_name);
        if let Some(saved) = saved_binding {
            self.bindings.insert(internal.clone(), saved);
        } else {
            self.bindings.remove(&internal);
        }
        lowered.map(|closure| HirExpr::RecursiveClosure(self_name, self_type, Box::new(closure)))
    }

    fn lower_contextual_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if arrow.params.len() != parameter_types.len() {
            return Err(format!(
                "Promise callback expects {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
        }
        if arrow.is_async {
            if arrow.is_generator {
                return Err("async generator arrow functions are not supported".into());
            }
            let mut contextual = arrow.clone();
            for (parameter, ty) in contextual.params.iter_mut().zip(parameter_types) {
                let annotation = match parameter {
                    Pat::Ident(binding) => &mut binding.type_ann,
                    Pat::Rest(rest) => &mut rest.type_ann,
                    _ => return Err(
                        "contextual async arrows currently require identifier or rest parameters"
                            .into(),
                    ),
                };
                if annotation.is_none() {
                    *annotation = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    }));
                }
            }
            if contextual.return_type.is_none() {
                if let Some(expected) = expected_return {
                    contextual.return_type = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(expected)?),
                    }));
                }
            }
            return self.lower_arrow(&contextual);
        }
        if arrow.is_generator {
            return Err("generator Promise callbacks are not supported".into());
        }
        let generic_return = if let Some(type_params) = &arrow.type_params {
            validate_trailing_type_parameter_defaults(
                "generic arrow function",
                "<anonymous>",
                type_params,
            )?;
            let generic_type_params = type_params
                .params
                .iter()
                .map(|parameter| parameter.name.sym.to_string())
                .collect::<Vec<_>>();
            let substitutions = generic_type_params
                .iter()
                .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                .collect::<HashMap<_, _>>();
            let generic_param_patterns = arrow
                .params
                .iter()
                .map(|parameter| {
                    let Pat::Ident(binding) = parameter else {
                        return Err(
                            "generic contextual arrows require identifier parameters".into()
                        );
                    };
                    let annotation = binding
                        .type_ann
                        .as_ref()
                        .ok_or("generic contextual arrow parameters need type annotations")?;
                    generic_type_pattern(
                        &annotation.type_ann,
                        &substitutions,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            let signature = FnSignature {
                params: Vec::new(),
                variadic: None,
                native_rest: None,
                abstract_class_constructor: false,
                ret: HirType::Dynamic,
                is_async: false,
                uses_this: false,
                is_extern: false,
                source_range: (arrow.span.lo.0, arrow.span.hi.0),
                generic_type_params,
                generic_type_constraints: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.constraint.clone())
                    .collect(),
                generic_type_defaults: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.default.clone())
                    .collect(),
                generic_param_patterns,
                generic_param_optional: arrow
                    .params
                    .iter()
                    .map(
                        |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.optional),
                    )
                    .collect(),
                generic_return_type: arrow
                    .return_type
                    .as_ref()
                    .map(|annotation| annotation.type_ann.clone()),
            };
            let types = infer_generic_type_tuple(
                &signature,
                parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )?;
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(types)
                .collect::<HashMap<_, _>>();
            arrow
                .return_type
                .as_ref()
                .map(|annotation| {
                    resolve_ts_type_with_substitution(
                        &annotation.type_ann,
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        if let (Some(declared), Some(contextual)) = (&generic_return, expected_return) {
            if contextual != &HirType::Dynamic && declared != contextual {
                return Err(format!(
                    "generic callback declares return type {declared:?}, expected {contextual:?}"
                ));
            }
        }
        let expected_return = generic_return.as_ref().or(expected_return);
        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(parameter_types.len());
            let mut destructuring = Vec::new();
            for (pat, ty) in arrow.params.iter().zip(parameter_types) {
                let source_name = match pat {
                    Pat::Ident(binding) => binding.id.sym.to_string(),
                    Pat::Assign(assignment) => match assignment.left.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "default callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Rest(rest) => match rest.arg.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "rest callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Object(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    Pat::Array(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    _ => return Err("unsupported Promise callback parameter pattern".into()),
                };
                let name = self.bind_local(&source_name, ty.clone());
                if !matches!(pat, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pat, name.clone(), ty.clone()));
                }
                params.push(HirParam {
                    name,
                    ty: ty.clone(),
                });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            self.ret_type = expected_return.cloned().unwrap_or(HirType::Dynamic);
            let (body, inferred) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let expression = self.lower_expr(expr)?;
                    let inferred = self.infer_expr_type(&expression)?;
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    (HirExpr::Block(stmts), inferred)
                }
            };
            if let Some(expected) = expected_return {
                if inferred != *expected && inferred != HirType::Dynamic {
                    return Err(format!(
                        "Promise callback returns {inferred:?}, expected {expected:?}"
                    ));
                }
            }
            let ret = expected_return.cloned().unwrap_or(inferred);
            if let Some(expected) = expected_return {
                debug_assert_eq!(&ret, expected);
            }
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    saved_scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();
            Ok(HirExpr::Lambda(captures, params, ret, Box::new(body)))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }
}
