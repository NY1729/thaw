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
        if arrow.type_params.is_some() {
            return Err("generic arrow functions are not supported yet".into());
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
        if arrow.is_generator {
            return self.lower_generator_arrow(arrow, source_params, None);
        }
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
                // A callback parameter annotated `any`/`unknown` that the
                // body *mutates* receives a live JavaScript value, not a
                // JSON snapshot: a package hands the closure its own
                // mutable object (real trigger: immer's own
                // `produce(value, draft => { draft.x = ... })`, whose
                // `draft` is a Proxy). Typing it `JsValue` routes the
                // body's `.x` writes through `setDynamicProperty` on the
                // real handle instead of a `Json` snapshot's local
                // `JsonSet`, which never reaches the caller. A merely-read
                // `any` parameter (a callback that just consumes JSON data)
                // stays `Json`, so no existing callback changes shape.
                let untyped =
                    parameter_is_any_annotated(pattern) && arrow_body_mutates(&param.name, arrow);
                let ty = if untyped && matches!(param.ty, HirType::Dynamic | HirType::Json) {
                    HirType::JsValue
                } else {
                    param.ty.clone()
                };
                let name = self.bind_local(&param.name, ty.clone());
                if !matches!(pattern, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pattern, name.clone(), ty.clone()));
                }
                params.push(HirParam { name, ty });
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
            // One-shot, taken regardless of `is_async` so it never leaks
            // into a nested arrow's own lowering -- only actually used
            // below when this arrow is a plain (non-async) callback with
            // no declared return annotation.
            let arrow_return_hint = self.expected_arrow_return_hint.take();
            self.ret_type = declared_async_result
                .clone()
                .or_else(|| declared_return.clone())
                .or(arrow_return_hint)
                .unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    // An implicit-return arrow (`(c) => c.text(...)`, no
                    // braces) is this same body's own tail return
                    // expression -- give it the identical hint
                    // `Stmt::Return` gets for the braced form, or a bare
                    // dynamic method call in tail position here defaults
                    // to the untyped JSON-snapshot path the exact same
                    // way.
                    let ret_type = self.ret_type.clone();
                    let mut expression = self.lower_expr_with_expected_type(expr, Some(&ret_type))?;
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
            Ok(rest_aware_closure(
                arrow,
                HirExpr::Lambda(captures, params, return_type, Box::new(body)),
            ))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }

    fn lower_generator_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        source_params: Vec<HirParam>,
        inferred_return: Option<HirType>,
    ) -> Result<HirExpr, String> {
        let ArrowFunctionBody::FunctionBody(block) = arrow.body.as_ref() else {
            return Err("generator function expression needs a body".into());
        };
        let declared_return = match inferred_return {
            Some(inferred) => inferred,
            None => lower_generator_return_type(
                arrow.is_async,
                &arrow.return_type,
                "<anonymous>",
                self.interfaces,
                self.generic_interfaces,
                &HashMap::new(),
            )?,
        };
        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let saved_generator_yields = self.generator_yields.clone();
        let saved_generator_finalizers = self.generator_finalizers.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(source_params.len());
            let mut body = Vec::new();
            for (pattern, param) in arrow.params.iter().zip(source_params.clone()) {
                let name = self.bind_local(&param.name, param.ty.clone());
                if !matches!(pattern, Pat::Ident(_) | Pat::Rest(_)) {
                    self.lower_binding_pattern(
                        pattern,
                        HirExpr::Var(name.clone()),
                        &param.ty,
                        &mut body,
                    )?;
                }
                params.push(HirParam { name, ty: param.ty });
            }
            self.ret_type = declared_return.clone();
            lower_function_statements(
                self,
                &block.stmts,
                &declared_return,
                true,
                &mut body,
            )?;
            let expression = HirExpr::Block(body);
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&expression, &mut referenced);
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
                declared_return.clone(),
                Box::new(expression),
            ))
        })();
        let inferred_return = self.ret_type.clone();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        self.generator_yields = saved_generator_yields;
        self.generator_finalizers = saved_generator_finalizers;
        if result.is_ok()
            && hir_type_contains_dynamic(&declared_return)
            && inferred_return != declared_return
        {
            return self.lower_generator_arrow(arrow, source_params, Some(inferred_return));
        }
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
            expected_return,
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
        if arrow.params.len() > parameter_types.len() {
            return Err(format!(
                "contextual callback accepts at most {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
        }
        if arrow.is_generator {
            let mut contextual = arrow.clone();
            for (parameter, ty) in contextual.params.iter_mut().zip(parameter_types) {
                let annotation = match parameter {
                    Pat::Ident(binding) => &mut binding.type_ann,
                    Pat::Rest(rest) => &mut rest.type_ann,
                    Pat::Object(object) => &mut object.type_ann,
                    Pat::Array(array) => &mut array.type_ann,
                    _ => return Err("unsupported contextual generator parameter pattern".into()),
                };
                if annotation.is_none() {
                    *annotation = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    }));
                }
            }
            return self.lower_arrow(&contextual);
        }
        if arrow.is_async {
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
                if annotation.as_ref().is_none_or(|annotation| {
                    matches!(
                        annotation.type_ann.as_ref(),
                        TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsAnyKeyword
                    )
                }) {
                    *annotation = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    }));
                }
            }
            for ty in parameter_types.iter().skip(contextual.params.len()) {
                let name = format!("__thaw_unused_{}", self.next_binding);
                self.next_binding += 1;
                contextual.params.push(Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: swc_ecma_ast::Ident::new_no_ctxt(
                        name.into(),
                        swc_common::DUMMY_SP,
                    ),
                    type_ann: Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    })),
                }));
            }
            if contextual.return_type.is_none() {
                if let Some(expected) = expected_return {
                    self.expected_arrow_return_hint = Some(match expected {
                        HirType::Promise(resolved) => resolved.as_ref().clone(),
                        other => other.clone(),
                    });
                }
            }
            return self.lower_arrow(&contextual);
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
                generic_return_pattern: None,
                type_predicate: None,
            };
            let types = infer_generic_type_tuple(
                &signature,
                parameter_types,
                self.interfaces,
                self.generic_interfaces,
                None,
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
        let declared_parameter_types = if arrow.type_params.is_none() {
            arrow
                .params
                .iter()
                .zip(parameter_types)
                .map(|(parameter, contextual)| {
                    if matches!(parameter, Pat::Assign(_)) {
                        return Ok(contextual.clone());
                    }
                    let annotation = match parameter {
                        Pat::Ident(binding) => binding.type_ann.as_ref(),
                        Pat::Rest(rest) => rest.type_ann.as_ref(),
                        Pat::Assign(assignment) => match assignment.left.as_ref() {
                            Pat::Ident(binding) => binding.type_ann.as_ref(),
                            _ => None,
                        },
                        _ => None,
                    };
                    annotation.map_or_else(
                        || Ok(contextual.clone()),
                        |annotation| {
                            if matches!(
                                annotation.type_ann.as_ref(),
                                TsType::TsKeywordType(keyword)
                                    if keyword.kind == TsKeywordTypeKind::TsAnyKeyword
                            ) {
                                return Ok(contextual.clone());
                            }
                            lower_ts_type(
                                &annotation.type_ann,
                                self.interfaces,
                                self.generic_interfaces,
                            )
                        },
                    )
                })
                .chain(parameter_types.iter().skip(arrow.params.len()).cloned().map(Ok))
                .collect::<Result<Vec<_>, String>>()?
        } else {
            parameter_types.to_vec()
        };
        let parameter_types = declared_parameter_types.as_slice();
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
            for ty in parameter_types.iter().skip(arrow.params.len()) {
                let name = format!("__thaw_unused_{}", self.next_binding);
                self.next_binding += 1;
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
                    let expression = if let Some(expected) = expected_return {
                        let expression = self.lower_expr_with_expected_type(expr, Some(expected))?;
                        if matches!(expected, HirType::Void | HirType::JsValue) {
                            expression
                        } else {
                            self.coerce_to_declared(expected, expression)?
                        }
                    } else {
                        self.lower_expr(expr)?
                    };
                    let inferred = self.infer_expr_type(&expression)?;
                    if expected_return == Some(&HirType::Void) {
                        prefix.push(HirStmt::Expr(expression));
                        (HirExpr::Block(prefix), HirType::Void)
                    } else {
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                    }
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    (HirExpr::Block(stmts), inferred)
                }
            };
            if let Some(expected) = expected_return {
                if inferred != *expected
                    && inferred != HirType::Dynamic
                    && *expected != HirType::JsValue
                {
                    return Err(format!(
                        "Promise callback returns {inferred:?}, expected {expected:?}"
                    ));
                }
            }
            let ret = match expected_return {
                Some(HirType::JsValue) if inferred != HirType::Dynamic => inferred,
                Some(expected) => expected.clone(),
                None => inferred,
            };
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

/// A JavaScript `(...rest) => ...` function's own logical type is a
/// `CallableFunction` with a rest parameter -- the flattened trailing
/// `Array(element)` parameter is only its *native* ABI, produced by
/// `lower_param` packing `Pat::Rest`. When an arrow is lowered without a
/// declared callback type to carry that distinction (the bare call site
/// of `lower_expr`, e.g. a `callDynamicMethod` argument object's own
/// field -- unlike a typed Fallback signature slot, which reaches
/// `coerce_to_declared`'s `CallableFunction` branch and is wrapped there),
/// the rest would otherwise disappear into a plain `Function([Array(..)])`.
/// A native-to-JS callback adapter built from that fixed type then passes
/// each real JS argument through positionally instead of collecting the
/// trailing ones into the array the native ABI expects -- the callback
/// reads an uninitialized array (real trigger: `new Widget().use({
/// renderer: { heading(...args) { ... } } })`, the modern marked API,
/// where the class method call is compiled through the untyped
/// `callDynamicMethod` JSON path). Re-tagging the closure's own inferred
/// type here keeps `has_rest` intact through every downstream consumer
/// (`compile_json_object_set_native_with_undefined`'s `CallableFunction`
/// arm, `compile_register_native_callback_from_closure_with_rest`).
fn rest_aware_closure(arrow: &swc_ecma_ast::ArrowExpr, closure: HirExpr) -> HirExpr {
    if !matches!(arrow.params.last(), Some(Pat::Rest(_))) {
        return closure;
    }
    let HirExpr::Lambda(_, params, ret, _) = &closure else {
        return closure;
    };
    let Some((last, fixed)) = params.split_last() else {
        return closure;
    };
    let HirType::Array(element) = &last.ty else {
        return closure;
    };
    let optional = arrow.params[..fixed.len()]
        .iter()
        .map(|parameter| match parameter {
            Pat::Assign(_) => true,
            Pat::Ident(binding) => binding.id.optional,
            _ => false,
        })
        .collect::<Vec<_>>();
    let callable = HirType::CallableFunction(
        fixed.iter().map(|parameter| parameter.ty.clone()).collect(),
        optional_parameter_mask(&optional),
        Some(element.clone()),
        Box::new(ret.clone()),
    );
    HirExpr::TypedClosure(callable, Box::new(closure))
}

/// Whether an arrow/function body mutates the parameter `name` -- a direct
/// assignment to one of its members (`draft.x = ...`, `draft[i] = ...`), a
/// mutating method call on it (`draft.push(...)`, `draft.set(...)`), or
/// `++`/`--`/`delete` on a member. Such a parameter must be a *live*
/// `JsValue` so the mutation reaches the real object (immer's Proxy draft);
/// a merely-read parameter keeps its `Json` snapshot shape.
fn arrow_body_mutates(name: &str, arrow: &swc_ecma_ast::ArrowExpr) -> bool {
    use swc_ecma_visit::{Visit, VisitWith};

    struct Finder<'a> {
        name: &'a str,
        mutated: bool,
    }

    /// The base identifier of a member/assignment target chain
    /// (`a.b[c]` -> `Some("a")`), following the object side down.
    fn root_ident(expr: &swc_ecma_ast::Expr) -> Option<&str> {
        match expr {
            swc_ecma_ast::Expr::Ident(ident) => Some(ident.sym.as_ref()),
            swc_ecma_ast::Expr::Member(member) => root_ident(member.obj.as_ref()),
            swc_ecma_ast::Expr::Paren(paren) => root_ident(paren.expr.as_ref()),
            _ => None,
        }
    }

    fn is_mutating_method(name: &str) -> bool {
        matches!(
            name,
            "push" | "pop" | "shift" | "unshift" | "splice" | "sort" | "reverse" | "fill" | "copyWithin"
                | "set" | "delete" | "clear" | "add"
        )
    }

    fn pattern_binds_name(pattern: &Pat, name: &str) -> bool {
        match pattern {
            Pat::Ident(binding) => binding.id.sym == name,
            Pat::Array(array) => array
                .elems
                .iter()
                .flatten()
                .any(|element| pattern_binds_name(element, name)),
            Pat::Rest(rest) => pattern_binds_name(&rest.arg, name),
            Pat::Object(object) => object.props.iter().any(|property| match property {
                swc_ecma_ast::ObjectPatProp::KeyValue(property) => {
                    pattern_binds_name(&property.value, name)
                }
                swc_ecma_ast::ObjectPatProp::Assign(property) => property.key.sym == name,
                swc_ecma_ast::ObjectPatProp::Rest(rest) => pattern_binds_name(&rest.arg, name),
            }),
            Pat::Assign(assignment) => pattern_binds_name(&assignment.left, name),
            Pat::Invalid(_) | Pat::Expr(_) => false,
        }
    }

    fn block_shadows_name(block: &swc_ecma_ast::BlockStmt, name: &str) -> bool {
        block.stmts.iter().any(|statement| {
            let swc_ecma_ast::Stmt::Decl(declaration) = statement else {
                return false;
            };
            match declaration {
                swc_ecma_ast::Decl::Var(var) if var.kind != swc_ecma_ast::VarDeclKind::Var => var
                    .decls
                    .iter()
                    .any(|declaration| pattern_binds_name(&declaration.name, name)),
                swc_ecma_ast::Decl::Fn(function) => function.ident.sym == name,
                swc_ecma_ast::Decl::Class(class) => class.ident.sym == name,
                _ => false,
            }
        })
    }

    impl Visit for Finder<'_> {
        fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
            if arrow
                .params
                .iter()
                .any(|parameter| pattern_binds_name(parameter, self.name))
            {
                return;
            }
            arrow.visit_children_with(self);
        }

        fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
            if function
                .params
                .iter()
                .any(|parameter| pattern_binds_name(&parameter.pat, self.name))
            {
                return;
            }
            function.visit_children_with(self);
        }

        fn visit_catch_clause(&mut self, catch: &swc_ecma_ast::CatchClause) {
            if catch
                .param
                .as_ref()
                .is_some_and(|parameter| pattern_binds_name(parameter, self.name))
            {
                return;
            }
            catch.visit_children_with(self);
        }

        fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
            if block_shadows_name(block, self.name) {
                return;
            }
            block.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &swc_ecma_ast::AssignExpr) {
            if let swc_ecma_ast::AssignTarget::Simple(simple) = &assignment.left {
                let target = match simple {
                    swc_ecma_ast::SimpleAssignTarget::Member(member) => {
                        Some(member.obj.as_ref())
                    }
                    _ => None,
                };
                if target.and_then(root_ident) == Some(self.name) {
                    self.mutated = true;
                }
            }
            assignment.visit_children_with(self);
        }

        fn visit_update_expr(&mut self, update: &swc_ecma_ast::UpdateExpr) {
            if root_ident(update.arg.as_ref()) == Some(self.name) {
                self.mutated = true;
            }
            update.visit_children_with(self);
        }

        fn visit_unary_expr(&mut self, unary: &swc_ecma_ast::UnaryExpr) {
            if unary.op == swc_ecma_ast::UnaryOp::Delete
                && root_ident(unary.arg.as_ref()) == Some(self.name)
            {
                self.mutated = true;
            }
            unary.visit_children_with(self);
        }

        fn visit_call_expr(&mut self, call: &swc_ecma_ast::CallExpr) {
            if let swc_ecma_ast::Callee::Expr(callee) = &call.callee {
                if let swc_ecma_ast::Expr::Member(member) = callee.as_ref() {
                    if root_ident(member.obj.as_ref()) == Some(self.name) {
                        if let swc_ecma_ast::MemberProp::Ident(property) = &member.prop {
                            if is_mutating_method(property.sym.as_ref()) {
                                self.mutated = true;
                            }
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    // Follow nested closures that capture this parameter, but do not count
    // writes below a scope that binds the same spelling to a different value.
    let mut finder = Finder {
        name,
        mutated: false,
    };
    arrow.body.visit_with(&mut finder);
    finder.mutated
}

/// Whether an arrow/function parameter's own annotation is `any`/`unknown`
/// (or entirely absent) -- see the `lower_arrow` call site.
fn parameter_is_any_annotated(pattern: &Pat) -> bool {
    let annotation = match pattern {
        Pat::Ident(binding) => binding.type_ann.as_ref(),
        Pat::Rest(rest) => rest.type_ann.as_ref(),
        Pat::Object(object) => object.type_ann.as_ref(),
        Pat::Array(array) => array.type_ann.as_ref(),
        Pat::Assign(assignment) => return parameter_is_any_annotated(&assignment.left),
        _ => return false,
    };
    let Some(annotation) = annotation else {
        return true;
    };
    matches!(
        annotation.type_ann.as_ref(),
        TsType::TsKeywordType(keyword)
            if matches!(
                keyword.kind,
                TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword
            )
    )
}

#[cfg(test)]
mod mutation_detection_tests {
    use super::*;

    fn parsed_arrow(source: &str) -> swc_ecma_ast::ArrowExpr {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let swc_ecma_ast::ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(
            swc_ecma_ast::Decl::Var(declaration),
        )) = &module.body[0]
        else {
            panic!("expected variable declaration");
        };
        let Some(swc_ecma_ast::Expr::Arrow(arrow)) = declaration.decls[0]
            .init
            .as_deref()
        else {
            panic!("expected arrow initializer");
        };
        arrow.clone()
    }

    #[test]
    fn mutation_detection_respects_nested_shadowing_and_captures() {
        let shadowed = parsed_arrow(
            "const callback = (draft: any) => { const nested = (draft: any) => { draft.x++; }; nested({}); };",
        );
        assert!(!arrow_body_mutates("draft", &shadowed));

        let captured = parsed_arrow(
            "const callback = (draft: any) => { const nested = () => { draft.x++; }; nested(); };",
        );
        assert!(arrow_body_mutates("draft", &captured));
    }
}
