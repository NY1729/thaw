struct GenericArrowThisOperation {
    receiver: (HirExpr, HirType),
    bind: bool,
    ignored: Vec<LoweredBinding>,
    prelowered: Option<Vec<HirExpr>>,
}

impl<'a> FnLowerer<'a> {
    fn lower_generic_arrow_operation(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else { return Ok(None); };
        let Expr::Member(operation) = callee.as_ref() else { return Ok(None); };
        let Some(method) = member_property_name(&operation.prop) else { return Ok(None); };
        if !matches!(method.as_str(), "call" | "apply" | "bind") { return Ok(None); }
        let Expr::Ident(target) = operation.obj.as_ref() else { return Ok(None); };
        let name = self.resolve_binding(target.sym.as_ref());
        if !self.generic_non_arrow_names.contains(&name) { return Ok(None); }
        let Some(arrow) = self.generic_arrows.get(&name).cloned() else { return Ok(None); };
        let (this_value, this_type, supplied) = if let Some((this_arg, supplied)) = call.args.split_first() {
            if this_arg.spread.is_some() {
                return Err(format!("generic function .{method}() cannot spread its thisArg"));
            }
            let value = self.lower_expr(&this_arg.expr)?;
            let ty = self.infer_expr_type(&value)?;
            (value, ty, supplied)
        } else {
            (HirExpr::Lit(HirLit::Undefined), HirType::Undefined, &[][..])
        };
        let mut discarded_apply_arguments = Vec::new();
        let mut prelowered_apply_arguments = None;
        let forwarded = if method == "apply" {
            if let Some((values, extras)) = supplied.split_first() {
                if values.spread.is_some() {
                    return Err("generic function .apply() cannot spread its argument tuple".into());
                }
                let value = self.lower_expr(&values.expr)?;
                let ty = self.infer_expr_type(&value)?;
                let nullable_tuple = match &ty {
                    HirType::Optional(payload) if matches!(payload.as_ref(), HirType::Tuple(_)) =>
                        Some((payload.as_ref().clone(), 0_u8)),
                    HirType::Nullable(payload) if matches!(payload.as_ref(), HirType::Tuple(_)) =>
                        Some((payload.as_ref().clone(), 1_u8)),
                    HirType::Nullish(payload) if matches!(payload.as_ref(), HirType::Tuple(_)) =>
                        Some((payload.as_ref().clone(), 2_u8)),
                    _ => None,
                };
                if let Some((payload, absence_kind)) = nullable_tuple {
                    let this_name = format!("__thaw_apply_this_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(this_name.clone(), this_type.clone());
                    let tuple_name = format!("__thaw_apply_tuple_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(tuple_name.clone(), ty.clone());
                    let mut bindings = vec![
                        (this_name.clone(), this_type.clone(), this_value),
                        (tuple_name.clone(), ty.clone(), value),
                    ];
                    bindings.extend(self.lower_ignored_apply_extras(extras)?);
                    let tuple_var = HirExpr::Var(tuple_name.clone());
                    let missing = match absence_kind {
                        0 => HirExpr::OptionalIsNone(Box::new(tuple_var.clone()), payload.clone()),
                        1 => HirExpr::NullableIsNone(Box::new(tuple_var.clone()), payload.clone()),
                        _ => HirExpr::NullishIsNone(Box::new(tuple_var.clone()), payload.clone()),
                    };
                    let tuple = match absence_kind {
                        0 => HirExpr::OptionalValue(Box::new(tuple_var), payload.clone()),
                        1 => HirExpr::NullableValue(Box::new(tuple_var), payload.clone()),
                        _ => HirExpr::NullishValue(Box::new(tuple_var), payload.clone()),
                    };
                    let mut present_call = call.clone();
                    present_call.args = vec![swc_ecma_ast::ExprOrSpread {
                        spread: Some(call.span), expr: values.expr.clone(),
                    }];
                    let present = self.lower_generic_arrow_call_with_this(&name, &arrow,
                        &present_call, Some(GenericArrowThisOperation {
                            receiver: (HirExpr::Var(this_name.clone()), this_type.clone()),
                            bind: false, ignored: Vec::new(), prelowered: Some(vec![tuple]),
                        }))?;
                    let mut absent_call = call.clone();
                    absent_call.args.clear();
                    let absent = self.lower_generic_arrow_call_with_this(&name, &arrow,
                        &absent_call, Some(GenericArrowThisOperation {
                            receiver: (HirExpr::Var(this_name), this_type),
                            bind: false, ignored: Vec::new(), prelowered: None,
                        }))?;
                    let result_ty = self.infer_expr_type(&present)?;
                    if result_ty != self.infer_expr_type(&absent)? {
                        return Err("generic .apply() nullish tuple branches infer different return types".into());
                    }
                    let (absent_stmts, present_stmts) = if result_ty == HirType::Void {
                        (vec![HirStmt::Expr(absent)], vec![HirStmt::Expr(present)])
                    } else {
                        (vec![HirStmt::Return(Some(absent))],
                         vec![HirStmt::Return(Some(present))])
                    };
                    let branch = HirExpr::Block(vec![HirStmt::If(missing,
                        absent_stmts, present_stmts)]);
                    return self.wrap_call_argument_bindings(branch, &bindings).map(Some);
                }
                let forwarded = if matches!(ty, HirType::Null | HirType::Undefined) {
                    let name = format!("__thaw_unused_apply_argument_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    discarded_apply_arguments.push((name, ty, value));
                    Vec::new()
                } else {
                    prelowered_apply_arguments = Some(vec![value]);
                    vec![swc_ecma_ast::ExprOrSpread {
                        spread: Some(call.span), expr: values.expr.clone(),
                    }]
                };
                discarded_apply_arguments.extend(self.lower_ignored_apply_extras(extras)?);
                forwarded
            } else { Vec::new() }
        } else { supplied.to_vec() };
        let mut invocation = call.clone();
        invocation.args = forwarded;
        self.lower_generic_arrow_call_with_this(&name, &arrow, &invocation,
            Some(GenericArrowThisOperation {
                receiver: (this_value, this_type),
                bind: method == "bind",
                ignored: discarded_apply_arguments,
                prelowered: prelowered_apply_arguments,
            })).map(Some)
    }

    fn lower_ignored_apply_extras(
        &mut self,
        extras: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<Vec<LoweredBinding>, String> {
        let mut ignored = Vec::new();
        for extra in extras {
            let (values, sources) = self.lower_native_spread_values(
                std::slice::from_ref(extra), "generic function .apply() extra argument")?;
            ignored.extend(sources);
            if extra.spread.is_some() {
                for value in values {
                    let ty = self.infer_expr_type(&value)?;
                    let name = format!("__thaw_unused_apply_argument_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    ignored.push((name, ty, value));
                }
            }
        }
        Ok(ignored)
    }

    fn lower_generic_arrow_call(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        self.lower_generic_arrow_call_with_this(name, arrow, call, None)
    }

    fn lower_generic_arrow_call_with_this(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        call: &CallExpr,
        operation: Option<GenericArrowThisOperation>,
    ) -> Result<HirExpr, String> {
        let (this_argument, bind, discarded_apply_arguments, prelowered_arguments) =
            match operation {
                Some(operation) => (Some(operation.receiver), operation.bind,
                    operation.ignored, operation.prelowered),
                None => (None, false, Vec::new(), None),
            };
        let lowered = match prelowered_arguments {
            Some(values) => values,
            None => call.args.iter()
                .map(|argument| self.lower_expr(&argument.expr))
                .collect::<Result<Vec<_>, _>>()?,
        };
        let preserve_order = call.args.iter().any(|argument| argument.spread.is_some())
            || lowered.iter().any(contains_await);
        let mut bindings = Vec::new();
        let this_name = this_argument.as_ref().map(|(_, ty)| {
            let name = format!("__thaw_generic_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            name
        });
        if let (Some((value, ty)), Some(this_name)) = (&this_argument, &this_name) {
            bindings.push((this_name.clone(), ty.clone(), value.clone()));
        }
        let mut arguments = Vec::new();
        for (source, value) in call.args.iter().zip(lowered) {
            if source.spread.is_none() && !preserve_order {
                arguments.push(value);
                continue;
            }
            if source.spread.is_some() {
                if let HirExpr::ArrayLit(elements) = value {
                    for element in elements {
                        let ty = self.infer_expr_type(&element)?;
                        let temporary = format!("__thaw_generic_arrow_arg_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(temporary.clone(), ty.clone());
                        bindings.push((temporary.clone(), ty, element));
                        arguments.push(HirExpr::Var(temporary));
                    }
                    continue;
                }
                let source_type = self.infer_expr_type(&value)?;
                let HirType::Tuple(elements) = &source_type else {
                    return Err(format!(
                        "generic arrow `{name}` spread source must have a statically known tuple length, got {source_type:?}"
                    ));
                };
                let elements = elements.clone();
                let temporary = format!("__thaw_generic_arrow_spread_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(temporary.clone(), source_type.clone());
                bindings.push((temporary.clone(), source_type, value));
                arguments.extend(elements.into_iter().enumerate().map(|(index, ty)| {
                    HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(temporary.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ty,
                    )
                }));
                continue;
            }
            let ty = self.infer_expr_type(&value)?;
            let temporary = format!("__thaw_generic_arrow_arg_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            bindings.push((temporary.clone(), ty, value));
            arguments.push(HirExpr::Var(temporary));
        }
        bindings.extend(discarded_apply_arguments);
        let signature = self.generic_arrow_signature(arrow)?;
        let visible_count = signature.generic_param_patterns.len();
        let has_rest = matches!(arrow.params.last(), Some(Pat::Rest(_)));
        let fixed_count = visible_count - usize::from(has_rest);
        if (!has_rest && arguments.len() > fixed_count) || (!bind
            && (arguments.len()..fixed_count)
                .any(|index| !signature.generic_param_optional[index])) {
            return Err(format!(
                "generic arrow `{name}` expects {} argument(s), got {} after spread expansion",
                visible_count,
                arguments.len()
            ));
        }
        let parameter_types = arguments
            .iter()
            .map(|argument| self.infer_expr_type(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let mut inference_signature = signature.clone();
        let mut inference_types = parameter_types.clone();
        if has_rest {
            let Some(GenericTypePattern::Array(element)) = signature.generic_param_patterns.last() else {
                return Err("generic rest parameter needs an array type annotation".into());
            };
            inference_signature.generic_param_patterns.truncate(fixed_count);
            inference_signature.generic_param_patterns.extend(
                parameter_types.iter().skip(fixed_count).map(|_| element.as_ref().clone()));
        }
        if let (Some(template), Some((_, this_ty))) = (
            self.generic_non_arrow_receiver_templates.get(name), this_argument.as_ref())
        {
            let variables = signature.generic_type_params.iter().cloned()
                .map(|name| (name.clone(), GenericTypePattern::Variable(name)))
                .collect::<HashMap<_, _>>();
            let pattern = generic_type_pattern(template, &variables,
                self.interfaces, self.generic_interfaces, &mut Vec::new())?;
            inference_signature.generic_param_patterns.insert(0, pattern);
            inference_types.insert(0, this_ty.clone());
        }
        let concrete_types = if let Some(type_args) = &call.type_args {
            resolve_explicit_generic_type_tuple(
                &inference_signature,
                &type_args.params,
                &inference_types,
                self.interfaces,
                self.generic_interfaces,
            )
            .map_err(|error| {
                format!("cannot explicitly specialize generic arrow `{name}`: {error}")
            })?
        } else {
            infer_generic_type_tuple(
                &inference_signature,
                &inference_types,
                self.interfaces,
                self.generic_interfaces,
                None,
            )
            .map_err(|error| format!("cannot specialize generic arrow `{name}`: {error}"))?
        };
        let receiver = self.specialized_generic_receiver(name, &signature, &concrete_types)?;
        let concrete = signature.generic_type_params.iter().cloned()
            .zip(concrete_types.iter().cloned()).collect::<HashMap<_, _>>();
        let runtime_parameter_types = if bind || has_rest || signature.generic_param_optional.iter().any(|optional| *optional) {
            signature.generic_param_patterns.iter().enumerate()
                .map(|(index, pattern)| {
                    let ty = instantiate_generic_pattern(pattern, &concrete)?;
                    Ok(if signature.generic_param_optional[index] { optional_parameter_type(ty) } else { ty })
                })
                .collect::<Result<Vec<_>, String>>()?
        } else { parameter_types.clone() };
        if !bind {
            let rest_values = if has_rest {
                arguments.split_off(fixed_count.min(arguments.len()))
            } else { Vec::new() };
            arguments = arguments.into_iter().zip(&runtime_parameter_types)
                .map(|(value, expected)| self.coerce_to_declared(expected, value))
                .collect::<Result<Vec<_>, _>>()?;
            for expected in runtime_parameter_types.iter().take(fixed_count).skip(arguments.len()) {
                arguments.push(omitted_parameter_value(expected)?);
            }
            if has_rest {
                let HirType::Array(element) = &runtime_parameter_types[fixed_count] else {
                    return Err("specialized generic rest parameter is not an array".into());
                };
                let rest = rest_values.into_iter()
                    .map(|value| self.coerce_to_declared(element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                arguments.push(native_rest_array(rest, element));
            }
        }
        let recursive = self.generic_arrow_self_names.get(name).cloned();
        let mut recursive_state = None;
        if let Some(internal) = recursive {
            let return_type = signature.generic_return_type.as_ref().ok_or_else(|| {
                format!("recursive generic local function `{name}` needs a return annotation")
            })?;
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(concrete_types.iter().cloned())
                .collect::<HashMap<_, _>>();
            let return_type = resolve_ts_type_with_substitution(
                return_type,
                &substitution,
                self.interfaces,
                self.generic_interfaces,
                &mut Vec::new(),
            )?;
            let self_type = HirType::Function(runtime_parameter_types.clone(), Box::new(return_type));
            let self_name = format!("__thaw_recursive_generic_{}", self.next_binding);
            self.next_binding += 1;
            let saved_binding = self.bindings.get(&internal).cloned();
            self.scope.insert(self_name.clone(), self_type.clone());
            self.bindings
                .entry(internal.clone())
                .or_default()
                .push(self_name.clone());
            recursive_state = Some((internal, self_name, self_type, saved_binding));
        }
        let substitutions = signature
            .generic_type_params
            .iter()
            .zip(&concrete_types)
            .map(|(name, ty)| Ok((name.clone(), Box::new(hir_type_as_ts_type(ty)?))))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let mut specialized_arrow = arrow.clone();
        specialized_arrow.type_params = None;
        specialized_arrow.visit_mut_with(&mut GenericClassTypeSubstituter {
            substitutions: &substitutions,
        });
        let previous_receiver = std::mem::replace(
            &mut self.non_arrow_receiver,
            receiver,
        );
        let lowered =
            self.lower_contextual_arrow(&specialized_arrow, &runtime_parameter_types, None);
        self.non_arrow_receiver = previous_receiver;
        if let Some((internal, self_name, _, saved_binding)) = &recursive_state {
            self.scope.remove(self_name);
            if let Some(saved) = saved_binding {
                self.bindings.insert(internal.clone(), saved.clone());
            } else {
                self.bindings.remove(internal);
            }
        }
        let mut lambda = lowered
            .map_err(|error| format!("cannot specialize generic arrow `{name}`: {error}"))?;
        if self.generic_non_arrow_names.contains(name) {
            lambda = HirExpr::NonArrowFunction(Box::new(lambda));
        }
        if let Some((_, self_name, self_type, _)) = recursive_state {
            lambda = HirExpr::RecursiveClosure(self_name, self_type, Box::new(lambda));
        }
        let result = if let Some(this_name) = this_name {
            let result_type = self.infer_expr_type(&lambda)?;
            let return_type = match &result_type {
                HirType::Function(_, ret) | HirType::CallableFunction(_, _, _, ret) => ret.as_ref().clone(),
                other => return Err(format!("generic function has non-callable type {other:?}")),
            };
            let this_arg = Box::new(HirExpr::Var(this_name.clone()));
            if bind {
                if has_rest && arguments.len() > fixed_count {
                    let HirType::Array(element) = &runtime_parameter_types[fixed_count] else {
                        return Err("specialized generic rest parameter is not an array".into());
                    };
                    let element = element.as_ref().clone();
                    let target_name = format!("__thaw_generic_rest_target_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(target_name.clone(), result_type.clone());
                    bindings.push((target_name.clone(), result_type.clone(), lambda));
                    let mut bound_names = Vec::new();
                    for (index, value) in arguments.into_iter().enumerate() {
                        let expected = if index < fixed_count {
                            &runtime_parameter_types[index]
                        } else { &element };
                        let value = self.coerce_to_declared(expected, value)?;
                        let name = format!("__thaw_generic_rest_bound_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), expected.clone());
                        bindings.push((name.clone(), expected.clone(), value));
                        bound_names.push((name, expected.clone()));
                    }
                    let remaining_fixed = runtime_parameter_types[bound_names.len().min(fixed_count)..fixed_count].to_vec();
                    let optional = optional_parameter_mask(
                        &signature.generic_param_optional[bound_names.len().min(fixed_count)..fixed_count]);
                    let rest_name = format!("__thaw_generic_rest_invocation_{}", self.next_binding);
                    self.next_binding += 1;
                    let mut closure_params = remaining_fixed.iter().enumerate().map(|(index, ty)| HirParam {
                        name: format!("__thaw_generic_rest_param_{index}"), ty: ty.clone(),
                    }).collect::<Vec<_>>();
                    closure_params.push(HirParam {
                        name: rest_name.clone(), ty: HirType::Array(Box::new(element.clone())),
                    });
                    let mut call_args = bound_names[..fixed_count].iter()
                        .map(|(name, _)| HirExpr::Var(name.clone())).collect::<Vec<_>>();
                    call_args.extend(closure_params[..remaining_fixed.len()].iter()
                        .map(|param| HirExpr::Var(param.name.clone())));
                    call_args.push(HirExpr::ArrayConcat(vec![
                        HirExpr::ArrayLit(bound_names[fixed_count..].iter()
                            .map(|(name, _)| HirExpr::Var(name.clone())).collect()),
                        HirExpr::Var(rest_name),
                    ], element.clone()));
                    let body = HirExpr::FunctionCallWithThis(
                        Box::new(HirExpr::Var(target_name.clone())), this_arg,
                        call_args, runtime_parameter_types, return_type.clone());
                    let mut captures = vec![
                        HirParam { name: target_name, ty: result_type },
                        HirParam { name: this_name, ty: this_argument.as_ref().unwrap().1.clone() },
                    ];
                    captures.extend(bound_names.into_iter()
                        .map(|(name, ty)| HirParam { name, ty }));
                    let closure = HirExpr::Lambda(captures, closure_params,
                        return_type.clone(), Box::new(body));
                    let bound = HirExpr::TypedClosure(HirType::CallableFunction(
                        remaining_fixed, optional, Some(Box::new(element)), Box::new(return_type)),
                        Box::new(closure));
                    return self.wrap_call_argument_bindings(bound, &bindings);
                }
                let bound = arguments.into_iter().zip(&runtime_parameter_types)
                    .map(|(argument, expected)| self.coerce_to_declared(expected, argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let bound_count = bound.len();
                let remaining = runtime_parameter_types[bound_count..fixed_count].to_vec();
                let optional = optional_parameter_mask(
                    &signature.generic_param_optional[bound_count..fixed_count]);
                let rest = if has_rest {
                    let HirType::Array(element) = &runtime_parameter_types[fixed_count] else { unreachable!() };
                    Some(element.clone())
                } else { None };
                HirExpr::TypedClosure(HirType::CallableFunction(remaining, optional,
                    rest, Box::new(return_type.clone())), Box::new(
                    HirExpr::FunctionBindThis(Box::new(lambda), this_arg,
                        bound, runtime_parameter_types, return_type)))
            } else {
                HirExpr::FunctionCallWithThis(Box::new(lambda), this_arg,
                    arguments, runtime_parameter_types, return_type)
            }
        } else { HirExpr::Call(Box::new(lambda), arguments) };
        self.wrap_call_argument_bindings(result, &bindings)
    }

    fn generic_arrow_signature(
        &self,
        arrow: &swc_ecma_ast::ArrowExpr,
    ) -> Result<FnSignature, String> {
        let type_params = arrow.type_params.as_ref().ok_or("arrow is not generic")?;
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
                let annotation = match parameter {
                    Pat::Ident(binding) => binding.type_ann.as_ref(),
                    Pat::Rest(rest) => rest.type_ann.as_ref(),
                    _ => return Err("generic arrows require identifier or rest parameters".into()),
                }.ok_or("generic arrow parameters need type annotations")?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            returns_sparse_array: false,
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
                .map(|parameter| matches!(parameter, Pat::Ident(binding) if binding.id.optional))
                .collect(),
            generic_return_type: arrow
                .return_type
                .as_ref()
                .map(|annotation| annotation.type_ann.clone())
                .or_else(|| inferred_generic_arrow_return_type(arrow)),
            generic_return_pattern: None,
            type_predicate: None,
        })
    }

    fn generic_callable_annotation_signature(
        &self,
        ty: &TsType,
    ) -> Result<Option<(FnSignature, Symbol)>, String> {
        let ty = strip_parenthesized_ts_type(ty);
        if let TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) = ty {
            if let Some(type_params) = &function.type_params {
                let label = "inline generic function type";
                return Ok(Some((
                    self.generic_function_type_signature(
                        label,
                        type_params,
                        &function.params,
                        &function.type_ann.type_ann,
                        (function.span.lo.0, function.span.hi.0),
                    )?,
                    label.into(),
                )));
            }
        }
        let TsType::TsTypeRef(reference) = ty else {
            return Ok(None);
        };
        let swc_ecma_ast::TsEntityName::Ident(identifier) = &reference.type_name else {
            return Ok(None);
        };
        if reference.type_params.is_some() {
            return Ok(None);
        }
        let callable_name = identifier.sym.to_string();
        let mut target = callable_name.clone();
        let mut seen = BTreeSet::new();
        loop {
            if let Some(alias) = self.generic_interfaces.function_aliases.get(&target) {
                return Ok(Some((
                    self.generic_function_alias_signature(alias)?,
                    callable_name,
                )));
            }
            if let Some(interface) = self.generic_interfaces.function_interfaces.get(&target) {
                return Ok(Some((
                    self.generic_function_interface_signature(interface)?,
                    callable_name,
                )));
            }
            let Some(next) = self
                .generic_interfaces
                .function_alias_chains
                .get(&target)
                .or_else(|| {
                    self.generic_interfaces
                        .function_interface_chains
                        .get(&target)
                })
                .cloned()
            else {
                return Ok(None);
            };
            if !seen.insert(target.clone()) {
                return Err(format!(
                    "cyclic generic callable type alias `{callable_name}`"
                ));
            }
            target = next;
        }
    }

    fn generic_function_interface_signature(
        &self,
        interface: &TsInterfaceDecl,
    ) -> Result<FnSignature, String> {
        let [TsTypeElement::TsCallSignatureDecl(call)] = interface.body.body.as_slice() else {
            return Err(format!(
                "callable interface `{}` must contain exactly one call signature",
                interface.id.sym
            ));
        };
        let type_params = call.type_params.as_ref().ok_or_else(|| {
            format!(
                "callable interface `{}` does not have a generic call signature",
                interface.id.sym
            )
        })?;
        validate_trailing_type_parameter_defaults(
            "generic callable interface",
            interface.id.sym.as_ref(),
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
        let generic_param_patterns = call
            .params
            .iter()
            .map(|parameter| {
                let TsFnParam::Ident(parameter) = parameter else {
                    return Err(format!(
                        "generic callable interface `{}` requires identifier parameters",
                        interface.id.sym
                    ));
                };
                let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                    format!(
                        "generic callable interface `{}` parameter `{}` needs an annotation",
                        interface.id.sym, parameter.id.sym
                    )
                })?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let return_type = call.type_ann.as_ref().ok_or_else(|| {
            format!(
                "generic callable interface `{}` needs a return type",
                interface.id.sym
            )
        })?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            returns_sparse_array: false,
            uses_this: false,
            is_extern: false,
            source_range: (interface.span.lo.0, interface.span.hi.0),
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
            generic_param_optional: call
                .params
                .iter()
                .map(|parameter| {
                    matches!(parameter, TsFnParam::Ident(binding) if binding.id.optional)
                })
                .collect(),
            generic_return_type: Some(return_type.type_ann.clone()),
            generic_return_pattern: None,
            type_predicate: None,
        })
    }

    fn generic_function_alias_signature(
        &self,
        alias: &swc_ecma_ast::TsTypeAliasDecl,
    ) -> Result<FnSignature, String> {
        let (type_params, params, return_type) = match strip_parenthesized_ts_type(&alias.type_ann)
        {
            TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => (
                function.type_params.as_ref(),
                function.params.as_slice(),
                Some(function.type_ann.type_ann.as_ref()),
            ),
            TsType::TsTypeLit(literal) => {
                let [TsTypeElement::TsCallSignatureDecl(call)] = literal.members.as_slice() else {
                    return Err(format!(
                        "type alias `{}` is not a generic callable type",
                        alias.id.sym
                    ));
                };
                (
                    call.type_params.as_ref(),
                    call.params.as_slice(),
                    call.type_ann
                        .as_ref()
                        .map(|annotation| annotation.type_ann.as_ref()),
                )
            }
            _ => {
                return Err(format!(
                    "type alias `{}` is not a generic callable type",
                    alias.id.sym
                ))
            }
        };
        let type_params = type_params
            .ok_or_else(|| format!("callable type alias `{}` is not generic", alias.id.sym))?;
        let return_type = return_type.ok_or_else(|| {
            format!(
                "generic callable type alias `{}` needs a return type",
                alias.id.sym
            )
        })?;
        self.generic_function_type_signature(
            &format!("generic function type alias `{}`", alias.id.sym),
            type_params,
            params,
            return_type,
            (alias.span.lo.0, alias.span.hi.0),
        )
    }

    fn generic_function_type_signature(
        &self,
        label: &str,
        type_params: &swc_ecma_ast::TsTypeParamDecl,
        params: &[TsFnParam],
        return_type: &TsType,
        source_range: (u32, u32),
    ) -> Result<FnSignature, String> {
        validate_trailing_type_parameter_defaults(
            label,
            label,
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
        let generic_param_patterns = params
            .iter()
            .map(|parameter| {
                let TsFnParam::Ident(parameter) = parameter else {
                    return Err(format!("{label} requires identifier parameters"));
                };
                let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                    format!("{label} parameter `{}` needs an annotation", parameter.id.sym)
                })?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            returns_sparse_array: false,
            uses_this: false,
            is_extern: false,
            source_range,
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
            generic_param_optional: params
                .iter()
                .map(|parameter| {
                    matches!(parameter, TsFnParam::Ident(binding) if binding.id.optional)
                })
                .collect(),
            generic_return_type: Some(Box::new(return_type.clone())),
            generic_return_pattern: None,
            type_predicate: None,
        })
    }

    fn validate_generic_callable_shape(
        &self,
        expected: &FnSignature,
        actual: &FnSignature,
        alias_name: &str,
    ) -> Result<(), String> {
        if actual.generic_return_type.is_none() {
            return Err(format!(
                "generic callable assigned to function type alias `{alias_name}` needs an explicit return type"
            ));
        }
        if expected.generic_type_params.len() != actual.generic_type_params.len()
            || expected.generic_param_patterns.len() != actual.generic_param_patterns.len()
        {
            return Err(format!(
                "generic arrow does not match function type alias `{alias_name}` arity"
            ));
        }
        if expected.generic_param_optional != actual.generic_param_optional {
            return Err(format!(
                "generic callable optional parameters do not match function type alias `{alias_name}`"
            ));
        }
        let canonical = (0..expected.generic_type_params.len())
            .map(|index| {
                HirType::Object(vec![(format!("__generic_parameter_{index}"), HirType::F64)])
            })
            .collect::<Vec<_>>();
        let expected_substitution = expected
            .generic_type_params
            .iter()
            .cloned()
            .zip(canonical.iter().cloned())
            .collect::<HashMap<_, _>>();
        let actual_substitution = actual
            .generic_type_params
            .iter()
            .cloned()
            .zip(canonical.iter().cloned())
            .collect::<HashMap<_, _>>();
        let expected_params = expected
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &expected_substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let actual_params = actual
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &actual_substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let expected_return = resolve_ts_type_with_substitution(
            expected
                .generic_return_type
                .as_ref()
                .expect("generic alias return type"),
            &expected_substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        let mut actual_return = resolve_ts_type_with_substitution(
            actual
                .generic_return_type
                .as_ref()
                .expect("generic callable return type was validated"),
            &actual_substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        if actual.is_async && !matches!(actual_return, HirType::Promise(_)) {
            actual_return = HirType::Promise(Box::new(actual_return));
        }
        if expected_params != actual_params || expected_return != actual_return {
            return Err(format!(
                "generic arrow has signature {actual_params:?} -> {actual_return:?}, incompatible with function type alias `{alias_name}` {expected_params:?} -> {expected_return:?}"
            ));
        }
        for (expected_constraint, actual_constraint) in expected
            .generic_type_constraints
            .iter()
            .zip(&actual.generic_type_constraints)
        {
            let expected_constraint = expected_constraint
                .as_ref()
                .map(|constraint| {
                    resolve_ts_type_with_substitution(
                        constraint,
                        &expected_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            let actual_constraint = actual_constraint
                .as_ref()
                .map(|constraint| {
                    resolve_ts_type_with_substitution(
                        constraint,
                        &actual_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            if expected_constraint != actual_constraint {
                return Err(format!(
                    "generic arrow constraints do not match function type alias `{alias_name}`"
                ));
            }
        }
        for (expected_default, actual_default) in expected
            .generic_type_defaults
            .iter()
            .zip(&actual.generic_type_defaults)
        {
            let expected_default = expected_default
                .as_ref()
                .map(|default| {
                    resolve_ts_type_with_substitution(
                        default,
                        &expected_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            let actual_default = actual_default
                .as_ref()
                .map(|default| {
                    resolve_ts_type_with_substitution(
                        default,
                        &actual_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            if expected_default != actual_default {
                return Err(format!(
                    "generic arrow defaults do not match function type alias `{alias_name}`"
                ));
            }
        }
        Ok(())
    }

    fn lower_optional_call(&mut self, call: &swc_ecma_ast::OptCall) -> Result<HirExpr, String> {
        let optional_member = match call.callee.as_ref() {
            Expr::Member(member) => Some(member),
            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => Some(member),
                OptChainBase::Call(_) => None,
            },
            _ => None,
        };
        if let Some(member) = optional_member {
            let receiver = self.lower_expr(&member.obj)?;
            let receiver_type = self.infer_expr_type(&receiver)?;
            if let Some((payload, absence_kind)) = match receiver_type.clone() {
                HirType::Optional(payload) => Some((payload, 0)),
                HirType::Nullable(payload) => Some((payload, 1)),
                HirType::Nullish(payload) => Some((payload, 2)),
                _ => None,
            } {
                let name = format!("__thaw_optional_method_receiver_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), receiver_type.clone());
                match absence_kind {
                    0 => {
                        self.narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    1 => {
                        self.nullable_narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    2 => {
                        self.nullish_narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    _ => unreachable!(),
                }

                let mut rebound = member.clone();
                rebound.obj = Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    name.clone().into(),
                    member.span,
                )));
                let mut ordinary = CallExpr::from(call.clone());
                ordinary.callee = Callee::Expr(Box::new(Expr::Member(rebound)));
                let invoked = self.lower_call(&ordinary);
                self.narrowings.remove(&name);
                self.nullable_narrowings.remove(&name);
                self.nullish_narrowings.remove(&name);
                let invoked = invoked?;
                let return_type = self.infer_expr_type(&invoked)?;
                let bound = HirExpr::Var(name.clone());
                let is_none = |bound: HirExpr| match absence_kind {
                    0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
                    1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
                    2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
                    _ => unreachable!(),
                };
                let result = if return_type == HirType::Void {
                    HirExpr::Block(vec![HirStmt::If(
                        is_none(bound),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))],
                        vec![
                            HirStmt::Expr(invoked),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                        ],
                    )])
                } else {
                    let (result_payload, present) = match &return_type {
                        HirType::Optional(inner) => (inner.as_ref().clone(), invoked),
                        output => (
                            output.clone(),
                            HirExpr::OptionalSome(Box::new(invoked), output.clone()),
                        ),
                    };
                    HirExpr::Block(vec![HirStmt::If(
                        is_none(bound),
                        vec![HirStmt::Return(Some(HirExpr::OptionalNone(result_payload)))],
                        vec![HirStmt::Return(Some(present))],
                    )])
                };
                return self
                    .wrap_call_argument_bindings(result, &[(name, receiver_type, receiver)]);
            }
            // The receiver is present, but the *property* itself may be an
            // optional function (`o.f?.()`). Reuse the receiver we already
            // lowered (so `new C().f?.()` constructs once) and let the
            // shared target path conditionally invoke the property value;
            // otherwise this is an ordinary member call.
            let property_is_optional = match (&receiver_type, &member.prop) {
                (HirType::Object(fields), MemberProp::Ident(prop)) => fields
                    .iter()
                    .find(|(name, _)| name == prop.sym.as_str())
                    .is_some_and(|(_, ty)| {
                        matches!(
                            ty,
                            HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)
                        )
                    }),
                _ => false,
            };
            if property_is_optional {
                let property = member_property_name(&member.prop)
                    .ok_or("optional call requires a statically known property")?;
                let callee = HirExpr::PropAccess(Box::new(receiver), receiver_type, property);
                return self.lower_optional_call_target(callee, call);
            }
            let mut ordinary = CallExpr::from(call.clone());
            ordinary.callee = Callee::Expr(Box::new(Expr::Member(member.clone())));
            return self.lower_call(&ordinary);
        }

        let callee = self.lower_expr(&call.callee)?;
        self.lower_optional_call_target(callee, call)
    }

    /// Shared tail of `lower_optional_call`: given an already-lowered callee
    /// value, conditionally invoke it when it is
    /// `Optional`/`Nullable`/`Nullish` of a function, else an ordinary call.
    fn lower_optional_call_target(
        &mut self,
        callee: HirExpr,
        call: &swc_ecma_ast::OptCall,
    ) -> Result<HirExpr, String> {
        let callee_type = self.infer_expr_type(&callee)?;
        let (payload, absence_kind) = match callee_type.clone() {
            HirType::Optional(payload) => (payload, 0),
            HirType::Nullable(payload) => (payload, 1),
            HirType::Nullish(payload) => (payload, 2),
            _ => return self.lower_call(&CallExpr::from(call.clone())),
        };
        let (params, optional, rest, return_type) = match payload.as_ref() {
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
            _ => {
                return Err(format!(
                    "optional call requires a function payload, got {payload:?}"
                ))
            }
        };
        if call.type_args.is_some() {
            return Err("optional native calls do not accept type arguments".into());
        }
        let (mut arguments, spread_bindings) =
            self.lower_native_spread_values(&call.args, "optional native call")?;
        if arguments.len() > params.len() && rest.is_none() {
            return Err(format!(
                "optional function accepts {} argument(s), got {}",
                params.len(),
                arguments.len()
            ));
        }
        if arguments.len() < params.len()
            && (arguments.len()..params.len())
                .any(|index| !is_optional_parameter(&optional, index))
        {
            return Err(format!(
                "optional function requires at least {} argument(s), got {}",
                optional.first_at_or_after(0).unwrap_or(params.len()),
                arguments.len()
            ));
        }
        let rest_values = rest.as_ref().map(|element| {
            let values = if arguments.len() > params.len() {
                arguments.split_off(params.len())
            } else {
                Vec::new()
            };
            (element, values)
        });
        for parameter in params.iter().skip(arguments.len()) {
            arguments.push(omitted_parameter_value(parameter)?);
        }
        let mut arguments = arguments
            .into_iter()
            .zip(&params)
            .map(|(value, expected)| self.coerce_to_declared(expected, value))
            .collect::<Result<Vec<_>, String>>()?;
        if let Some((element, values)) = rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(element, value))
                .collect::<Result<Vec<_>, _>>()?;
            arguments.push(native_rest_array(values, element));
        }

        let name = format!("__thaw_optional_callee_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), callee_type.clone());
        let bound = HirExpr::Var(name.clone());
        let function = match absence_kind {
            0 => HirExpr::OptionalValue(Box::new(bound.clone()), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(bound.clone()), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(bound.clone()), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let invoked = HirExpr::Call(Box::new(function), arguments);
        let invoked = self.wrap_call_argument_bindings(invoked, &spread_bindings)?;
        let is_none = |bound: HirExpr| match absence_kind {
            0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
            1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
            2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = if return_type == HirType::Void {
            HirExpr::Block(vec![HirStmt::If(
                is_none(bound),
                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))],
                vec![
                    HirStmt::Expr(invoked),
                    HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                ],
            )])
        } else {
            let (result_payload, present) = match &return_type {
                HirType::Optional(inner) => (inner.as_ref().clone(), invoked),
                output => (
                    output.clone(),
                    HirExpr::OptionalSome(Box::new(invoked), output.clone()),
                ),
            };
            HirExpr::Block(vec![HirStmt::If(
                is_none(bound),
                vec![HirStmt::Return(Some(HirExpr::OptionalNone(result_payload)))],
                vec![HirStmt::Return(Some(present))],
            )])
        };
        self.wrap_call_argument_bindings(result, &[(name, callee_type, callee)])
    }
}
