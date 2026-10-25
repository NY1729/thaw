/// A receiver whose runtime tags include a Host `Json` (a `Json` or a plain native
/// object, which crosses as a borrowed Host `Json`): a native `thisArg` of another
/// layout must be converted to `Json` before the tagged receiver entry decodes it.
fn receiver_wants_json(receiver: &HirType) -> bool {
    match receiver {
        HirType::Json => true,
        HirType::Object(fields) => !fields.first().is_some_and(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")),
        HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => receiver_wants_json(inner),
        _ => false,
    }
}

impl<'a> FnLowerer<'a> {
    pub(in crate::lower) fn adapt_selected_overload_argument(
        &mut self,
        physical: &HirType,
        selected: Option<&HirType>,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let Some(selected) = selected else {
            return Ok(value);
        };
        fn visible_callable(ty: &HirType) -> &HirType {
            match ty {
                HirType::FunctionWithThis(_, visible) => visible.as_ref(),
                other => other,
            }
        }
        let is_callable = |ty: &HirType| {
            matches!(
                visible_callable(ty),
                HirType::Function(..) | HirType::CallableFunction(..)
            )
        };
        let actual = self.infer_expr_type(&value)?;
        if !is_callable(selected) || !is_callable(physical) || !is_callable(&actual) {
            return Ok(value);
        }
        if visible_callable(physical) == visible_callable(&actual)
            || callable_abi_compatible(visible_callable(physical), visible_callable(&actual))
        {
            return Ok(value);
        }
        let (HirType::Function(public_params, public_ret), HirType::Function(physical_params, physical_ret)) =
            (visible_callable(selected), visible_callable(physical))
        else {
            return Err(format!(
                "selected callback ABI {selected:?} cannot be adapted to implementation ABI {physical:?}"
            ));
        };
        if public_params.len() != physical_params.len()
            || self
                .expect_type(selected, &value, "overload callback argument")
                .is_err()
        {
            return Err(format!(
                "selected callback ABI {selected:?} cannot be adapted to implementation ABI {physical:?}"
            ));
        }

        let callback_name = format!("__thaw_overload_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_previous = self.scope.insert(callback_name.clone(), selected.clone());
        let mut adapter_params = Vec::with_capacity(physical_params.len());
        let mut callback_args = Vec::with_capacity(physical_params.len());
        let mut parameter_previous = Vec::with_capacity(physical_params.len());
        for (index, (physical_param, public_param)) in
            physical_params.iter().zip(public_params).enumerate()
        {
            let name = format!("__thaw_overload_callback_arg_{}_{}", self.next_binding, index);
            self.next_binding += 1;
            let previous = self.scope.insert(name.clone(), physical_param.clone());
            parameter_previous.push((name.clone(), previous));
            adapter_params.push(HirParam {
                name: name.clone(),
                ty: physical_param.clone(),
            });
            let argument = if physical_param == public_param {
                HirExpr::Var(name)
            } else if matches!(physical_param, HirType::Union(members) if members.contains(public_param)) {
                self.project_overload_call_result(
                    HirExpr::Var(name),
                    physical_param,
                    public_param,
                )?
            } else {
                return Err(format!(
                    "callback parameter {physical_param:?} cannot be narrowed to selected public type {public_param:?}"
                ));
            };
            callback_args.push(argument);
        }
        let callback_result = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            callback_args,
        );
        let adapter_body = if physical_ret.as_ref() == public_ret.as_ref() {
            callback_result
        } else if let HirType::Union(members) = physical_ret.as_ref() {
            let Some(index) = members.iter().position(|member| member == public_ret.as_ref()) else {
                return Err(format!(
                    "selected callback return {:?} cannot be adapted to implementation return {:?}",
                    public_ret, physical_ret
                ));
            };
            HirExpr::UnionInject(Box::new(callback_result), index, members.clone())
        } else {
            return Err(format!(
                "selected callback return {:?} cannot be adapted to implementation return {:?}",
                public_ret, physical_ret
            ));
        };
        let adapter = HirExpr::Lambda(
            vec![HirParam {
                name: callback_name.clone(),
                ty: selected.clone(),
            }],
            adapter_params,
            physical_ret.as_ref().clone(),
            Box::new(adapter_body),
        );
        self.scope.insert(callback_name.clone(), selected.clone());
        let factory = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam {
                name: callback_name.clone(),
                ty: selected.clone(),
            }],
            physical.clone(),
            Box::new(adapter),
        );
        for (name, previous) in parameter_previous {
            match previous {
                Some(previous) => {
                    self.scope.insert(name, previous);
                }
                None => {
                    self.scope.remove(&name);
                }
            }
        }
        match callback_previous {
            Some(previous) => {
                self.scope.insert(callback_name, previous);
            }
            None => {
                self.scope.remove(&callback_name);
            }
        }
        Ok(HirExpr::Call(Box::new(factory), vec![value]))
    }

    pub(in crate::lower) fn select_public_overload(
        &self,
        overloads: &[HirType],
        arguments: &[HirExpr],
        call_receiver: Option<&HirExpr>,
        label: &str,
    ) -> Result<Option<HirType>, String> {
        for public in overloads {
            let (declared_receiver, callable) = match public {
                HirType::FunctionWithThis(receiver, callable) => {
                    (Some(receiver.as_ref()), callable.as_ref())
                }
                callable => (None, callable),
            };
            let (params, optional, rest, ret) = match callable {
                HirType::Function(params, ret) => (
                    params.as_slice(),
                    HirOptionalMask::default(),
                    None,
                    ret.as_ref(),
                ),
                HirType::CallableFunction(params, optional, rest, ret) => (
                    params.as_slice(),
                    optional.clone(),
                    rest.as_deref(),
                    ret.as_ref(),
                ),
                _ => continue,
            };
            let inline_receiver = declared_receiver.is_some() && call_receiver.is_none();
            let mut expected = Vec::with_capacity(params.len() + usize::from(inline_receiver));
            if let Some(receiver) = declared_receiver.filter(|_| inline_receiver) {
                expected.push(receiver.clone());
            }
            expected.extend(params.iter().cloned());
            let receiver_offset = usize::from(inline_receiver);
            if let (Some(receiver), Some(value)) = (declared_receiver, call_receiver) {
                if self.expect_type(receiver, value, "overload receiver").is_err() {
                    continue;
                }
            } else if declared_receiver.is_some() && call_receiver.is_none() && arguments.is_empty() {
                continue;
            }
            let required = (0..expected.len())
                .filter(|index| {
                    *index < receiver_offset || !optional.contains(*index - receiver_offset)
                })
                .count();
            if arguments.len() < required
                || (rest.is_none() && arguments.len() > expected.len())
            {
                continue;
            }
            if arguments.iter().enumerate().all(|(index, argument)| {
                expected
                    .get(index)
                    .or(rest)
                    .is_some_and(|declared| {
                        self.expect_public_overload_argument(declared, argument).is_ok()
                    })
            }) {
                return Ok(Some(ret.clone()));
            }
        }
        if overloads.is_empty() {
            Ok(None)
        } else {
            Err(format!("no public overload of `{label}` accepts the supplied arguments"))
        }
    }

    pub(in crate::lower) fn select_public_overload_for_source(
        &self,
        overloads: &[HirType],
        arguments: &[swc_ecma_ast::ExprOrSpread],
        call_receiver: Option<&HirExpr>,
        label: &str,
    ) -> Result<Option<HirType>, String> {
        for public in overloads {
            let (declared_receiver, callable) = match public {
                HirType::FunctionWithThis(receiver, callable) => {
                    (Some(receiver.as_ref()), callable.as_ref())
                }
                callable => (None, callable),
            };
            let (params, optional, rest) = match callable {
                HirType::Function(params, _) => (
                    params.as_slice(),
                    HirOptionalMask::default(),
                    None,
                ),
                HirType::CallableFunction(params, optional, rest, _) => (
                    params.as_slice(),
                    optional.clone(),
                    rest.as_deref(),
                ),
                _ => continue,
            };
            let inline_receiver = declared_receiver.is_some() && call_receiver.is_none();
            let mut expected = Vec::with_capacity(params.len() + usize::from(inline_receiver));
            if let Some(receiver) = declared_receiver.filter(|_| inline_receiver) {
                expected.push(receiver.clone());
            }
            expected.extend(params.iter().cloned());
            let receiver_offset = usize::from(inline_receiver);
            if let (Some(receiver), Some(value)) = (declared_receiver, call_receiver) {
                if self.expect_type(receiver, value, "overload receiver").is_err() {
                    continue;
                }
            } else if declared_receiver.is_some() && call_receiver.is_none() && arguments.is_empty() {
                continue;
            }

            let state = self
                .call_constraints
                .map(|constraints| constraints.borrow().len());
            let mut trial = self.clone();
            let mut argument_count = 0;
            let applicable = arguments.iter().all(|argument| {
                if argument.spread.is_some()
                    && !matches!(argument.expr.as_ref(), swc_ecma_ast::Expr::Array(_))
                {
                    let Ok(value) = trial.lower_expr_with_expected_type(&argument.expr, None) else {
                        return false;
                    };
                    let Ok(HirType::Tuple(elements)) = trial.infer_expr_type(&value) else {
                        return false;
                    };
                    return elements.iter().all(|actual| {
                        let Some(declared) = expected.get(argument_count).or(rest) else {
                            return false;
                        };
                        let typed = HirExpr::TypedIndex(
                            Box::new(HirExpr::Lit(HirLit::Undefined)),
                            Box::new(HirExpr::Lit(HirLit::F64(argument_count as f64))),
                            actual.clone(),
                        );
                        argument_count += 1;
                        trial.expect_type(declared, &typed, "overload argument").is_ok()
                    });
                }
                let mut check = |expr: &swc_ecma_ast::Expr| {
                    let Some(declared) = expected.get(argument_count).or(rest) else {
                        return false;
                    };
                    let Ok(value) = trial.lower_expr_with_expected_type(expr, Some(declared))
                    else {
                        return false;
                    };
                    argument_count += 1;
                    trial.expect_public_overload_argument(declared, &value).is_ok()
                };
                if argument.spread.is_none() {
                    return check(&argument.expr);
                }
                let swc_ecma_ast::Expr::Array(array) = argument.expr.as_ref() else {
                    unreachable!("nonliteral spread handled before contextual closure")
                };
                if array.elems.iter().any(Option::is_none) {
                    return false;
                }
                array.elems.iter().flatten().all(|element| {
                    if element.spread.is_some() {
                        return false;
                    }
                    check(&element.expr)
                })
            });
            let required = (0..expected.len())
                .filter(|index| {
                    *index < receiver_offset || !optional.contains(*index - receiver_offset)
                })
                .count();
            let applicable = applicable
                && argument_count >= required
                && (rest.is_some() || argument_count <= expected.len());
            if let (Some(constraints), Some(length)) = (self.call_constraints, state) {
                constraints.borrow_mut().truncate(length);
            }
            if applicable {
                return Ok(Some(public.clone()));
            }
        }
        if overloads.is_empty() {
            Ok(None)
        } else {
            Err(format!("no public overload of `{label}` accepts the supplied arguments"))
        }
    }

    pub(in crate::lower) fn public_overload_argument_type(
        &self,
        public: &HirType,
        index: usize,
    ) -> Option<HirType> {
        let (receiver, callable) = match public {
            HirType::FunctionWithThis(receiver, callable) => {
                (Some(receiver.as_ref()), callable.as_ref())
            }
            callable => (None, callable),
        };
        let (params, rest) = match callable {
            HirType::Function(params, _) | HirType::CallableFunction(params, _, None, _) => {
                (params.as_slice(), None)
            }
            HirType::CallableFunction(params, _, Some(rest), _) => {
                (params.as_slice(), Some(rest.as_ref()))
            }
            _ => return None,
        };
        let offset = usize::from(receiver.is_some());
        if index < offset {
            receiver.cloned()
        } else {
            params
                .get(index - offset)
                .or(rest)
                .cloned()
        }
    }

    fn expect_public_overload_argument(
        &self,
        declared: &HirType,
        value: &HirExpr,
    ) -> Result<(), String> {
        match declared {
            HirType::Optional(_) if self.infer_expr_type(value)? == HirType::Undefined => Ok(()),
            HirType::Optional(payload) => self
                .expect_type(declared, value, "overload argument")
                .or_else(|_| self.expect_type(payload, value, "overload argument")),
            _ => self.expect_type(declared, value, "overload argument"),
        }
    }

    pub(in crate::lower) fn public_overload_visible_argument_type(
        &self,
        public: &HirType,
        index: usize,
    ) -> Option<HirType> {
        let callable = match public {
            HirType::FunctionWithThis(_, callable) => callable.as_ref(),
            callable => callable,
        };
        match callable {
            HirType::Function(params, _) => params.get(index).cloned(),
            HirType::CallableFunction(params, _, rest, _) => params
                .get(index)
                .or(rest.as_deref())
                .cloned(),
            _ => None,
        }
    }

    pub(in crate::lower) fn public_overload_return_type(&self, public: &HirType) -> Option<HirType> {
        let callable = match public {
            HirType::FunctionWithThis(_, callable) => callable.as_ref(),
            callable => callable,
        };
        match callable {
            HirType::Function(_, ret) | HirType::CallableFunction(_, _, _, ret) => {
                Some(ret.as_ref().clone())
            }
            _ => None,
        }
    }

    pub(in crate::lower) fn project_overload_call_result(
        &mut self,
        call: HirExpr,
        physical: &HirType,
        selected: &HirType,
    ) -> Result<HirExpr, String> {
        if physical == selected {
            return Ok(call);
        }
        let HirType::Union(members) = physical else {
            return Err(format!(
                "overload return projection from {physical:?} to {selected:?} is unsupported"
            ));
        };
        let fallback = Self::unreachable_value(selected)?;
        let name = format!("__thaw_overload_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), physical.clone());
        let value = HirExpr::Var(name.clone());
        let throw = || {
            HirExpr::ThrowValue(
                Box::new(HirExpr::Lit(HirLit::Str(
                    "implementation returned a value outside the selected overload".into(),
                ))),
                Box::new(fallback.clone()),
            )
        };
        let checked = if let HirType::Union(selected_members) = selected {
            let mut checked = throw();
            for (selected_index, member) in selected_members.iter().enumerate().rev() {
                let Some(physical_index) = members.iter().position(|candidate| candidate == member)
                else {
                    return Err(format!(
                        "overload return member {member:?} is not in implementation return {physical:?}"
                    ));
                };
                checked = HirExpr::Conditional(
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(value.clone()), members.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(physical_index as f64))),
                    )),
                    Box::new(HirExpr::UnionInject(
                        Box::new(HirExpr::UnionValue(
                            Box::new(value.clone()),
                            physical_index,
                            members.clone(),
                        )),
                        selected_index,
                        selected_members.clone(),
                    )),
                    Box::new(checked),
                    selected.clone(),
                );
            }
            checked
        } else {
            let Some(index) = members.iter().position(|member| member == selected) else {
                return Err(format!(
                    "overload return {selected:?} is not a member of implementation return {physical:?}"
                ));
            };
            HirExpr::Conditional(
                Box::new(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::UnionTag(Box::new(value.clone()), members.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                )),
                Box::new(HirExpr::UnionValue(Box::new(value.clone()), index, members.clone())),
                Box::new(throw()),
                selected.clone(),
            )
        };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                vec![HirParam {
                    name,
                    ty: physical.clone(),
                }],
                selected.clone(),
                Box::new(checked),
            )),
            vec![call],
        ))
    }

    /// Converts an explicit `thisArg` to the declared receiver of a `this`-taking
    /// closure: a native aggregate of another layout crosses as a Host `Json`.
    fn coerce_this_for_receiver(
        &mut self,
        receiver: Option<&HirType>,
        this_value: HirExpr,
    ) -> Result<HirExpr, String> {
        let Some(receiver) = receiver else { return Ok(this_value) };
        let actual = self.infer_expr_type(&this_value)?;
        if let HirType::Object(fields) = &actual {
            let class_projection = self.live_projection_supported_with_classes(&actual);
            if class_projection
                || receiver_wants_json(receiver)
                    && Self::fixed_object_supports_live_projection(&actual)
            {
                let this_value = self.retain_optional_native_this_projection(this_value, fields)?;
                if actual == *receiver || !receiver_wants_json(receiver) {
                    return Ok(this_value);
                }
                return self.coerce_to_declared(&HirType::Json, this_value);
            }
        }
        if receiver_wants_json(receiver)
            && actual != *receiver
            && matches!(actual, HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_)
                | HirType::Bytes | HirType::Map(..) | HirType::Set(_)
                | HirType::WeakMap(..) | HirType::WeakSet(_))
        {
            return self.coerce_to_declared(&HirType::Json, this_value);
        }
        Ok(this_value)
    }
}

impl<'a> FnLowerer<'a> {
    fn is_unshadowed_class_identifier(&self, class: &str) -> bool {
        !self.scope.contains_key(&self.resolve_binding(class))
    }

    fn lower_this_parameter_call_or_apply(
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
        if operation_name != "call" && operation_name != "apply" && operation_name != "bind" {
            return Ok(None);
        }
        let Expr::Ident(target) = operation.obj.as_ref() else {
            return Ok(None);
        };
        let symbol = self.resolve_binding(target.sym.as_ref());
        if !self
            .signatures
            .get(&symbol)
            .is_some_and(|signature| signature.uses_this)
        {
            return Ok(None);
        }
        if operation_name == "bind" {
            let mut args = vec![swc_ecma_ast::ExprOrSpread {
                spread: None,
                expr: Box::new(Expr::Lit(Lit::Bool(swc_ecma_ast::Bool {
                    span: call.span,
                    value: false,
                }))),
            }];
            args.extend_from_slice(&call.args);
            return self.lower_function_bind(&CallExpr {
                span: call.span,
                ctxt: call.ctxt,
                callee: call.callee.clone(),
                args,
                type_args: call.type_args.clone(),
            });
        }
        let Some((this_argument, supplied)) = call.args.split_first() else {
            return Err(format!("function `{symbol}.{operation_name}` expects a thisArg"));
        };
        let mut args = vec![this_argument.clone()];
        if operation_name == "apply" {
            let [arguments] = supplied else {
                return Err(format!(
                    "function `{symbol}.apply` expects a thisArg and one argument tuple"
                ));
            };
            args.push(swc_ecma_ast::ExprOrSpread {
                spread: Some(call.span),
                expr: arguments.expr.clone(),
            });
        } else {
            args.extend_from_slice(supplied);
        }
        self.lower_call(&CallExpr {
            span: call.span,
            ctxt: call.ctxt,
            callee: Callee::Expr(Box::new(Expr::Ident(target.clone()))),
            args,
            type_args: call.type_args.clone(),
        })
        .map(Some)
    }

    /// `Object.prototype.toString.call(<value>)` -- the single most common
    /// real-world use of `Object`/`.prototype` (feature-detecting a
    /// value's "internal class" tag string, e.g. `Object.prototype.
    /// toString.call([]) === '[object Array]'`). Neither `Object` as a
    /// bare identifier nor a general `.prototype` object graph exists in
    /// this compiler, so this recognizes the one exact syntactic call
    /// shape instead of building either, computing the same tag real
    /// ECMA-262's `Object.prototype.toString` algorithm would (`Null`/
    /// `Undefined`/`Array`/`Function` first, then a small built-in table
    /// for the remaining native types, then a class's own `[Symbol.
    /// toStringTag]` getter if it has one -- round32's own class-getter
    /// lookup, also reused by `coerce_primitive_to_string`'s `String
    /// (value)` fallback -- finally defaulting to `"Object"`).
    fn lower_object_prototype_to_string_call(
        &mut self,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let Callee::Expr(callee) = &call.callee else {
            return Ok(None);
        };
        let Expr::Member(call_member) = callee.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&call_member.prop).as_deref() != Some("call") {
            return Ok(None);
        }
        let Expr::Member(to_string_member) = call_member.obj.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&to_string_member.prop).as_deref() != Some("toString") {
            return Ok(None);
        }
        let Expr::Member(prototype_member) = to_string_member.obj.as_ref() else {
            return Ok(None);
        };
        if member_property_name(&prototype_member.prop).as_deref() != Some("prototype") {
            return Ok(None);
        }
        let Expr::Ident(root) = prototype_member.obj.as_ref() else {
            return Ok(None);
        };
        if root.sym != *"Object" {
            return Ok(None);
        }
        let [argument] = call.args.as_slice() else {
            return Err("Object.prototype.toString.call expects exactly one argument".into());
        };
        if argument.spread.is_some() {
            return Err("Object.prototype.toString.call does not support a spread argument".into());
        }
        let value = self.lower_expr(&argument.expr)?;
        let ty = self.infer_expr_type(&value)?;
        self.object_prototype_to_string_tag(value, &ty).map(Some)
    }

    fn object_prototype_to_string_tag(
        &mut self,
        value: HirExpr,
        ty: &HirType,
    ) -> Result<HirExpr, String> {
        if matches!(ty, HirType::Optional(_) | HirType::Nullable(_)
            | HirType::Nullish(_) | HirType::Union(_)) {
            let name = format!("__thaw_object_tagged_to_string_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            let bound = HirExpr::Var(name.clone());
            let tag = |text: &str| HirExpr::Lit(HirLit::Str(format!("[object {text}]")));
            let choose = |test, yes, no| HirExpr::Conditional(
                Box::new(test), Box::new(yes), Box::new(no), HirType::Str,
            );
            let result = match ty {
                HirType::Optional(payload) => choose(
                    HirExpr::OptionalIsNone(Box::new(bound.clone()), payload.as_ref().clone()),
                    tag("Undefined"), self.object_prototype_to_string_tag(
                        HirExpr::OptionalValue(Box::new(bound), payload.as_ref().clone()), payload)?,
                ),
                HirType::Nullable(payload) => choose(
                    HirExpr::NullableIsNone(Box::new(bound.clone()), payload.as_ref().clone()),
                    tag("Null"), self.object_prototype_to_string_tag(
                        HirExpr::NullableValue(Box::new(bound), payload.as_ref().clone()), payload)?,
                ),
                HirType::Nullish(payload) => choose(
                    HirExpr::NullishIsUndefined(Box::new(bound.clone()), payload.as_ref().clone()),
                    tag("Undefined"), choose(
                        HirExpr::NullishIsNull(Box::new(bound.clone()), payload.as_ref().clone()),
                        tag("Null"), self.object_prototype_to_string_tag(
                            HirExpr::NullishValue(Box::new(bound), payload.as_ref().clone()), payload)?,
                    ),
                ),
                HirType::Union(members) => {
                    let mut result = tag("Object");
                    for (index, member) in members.iter().enumerate().rev() {
                        result = choose(
                            HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(bound.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64)))),
                            self.object_prototype_to_string_tag(
                                HirExpr::UnionValue(Box::new(bound.clone()), index, members.clone()), member)?,
                            result,
                        );
                    }
                    result
                }
                _ => unreachable!(),
            };
            return self.wrap_call_argument_bindings(result, &[(name, ty.clone(), value)]);
        }
        if let HirType::Object(_) = ty {
            if ty != &date_object_type() && ty != &regex_object_type() {
                let tag_property = well_known_symbol_key("toStringTag");
                if let Some(owner) = self.class_instance_accessor_owner(ty, &tag_property) {
                    let symbol = class_getter_symbol(&owner, &tag_property, false);
                    if self.signatures.get(&symbol).is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str())) {
                        let tag = HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![
                            self.assert_class_accessor_receiver(value, &owner),
                        ]);
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![
                                HirExpr::Lit(HirLit::Str("[object ".to_string())),
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                    vec![tag, HirExpr::Lit(HirLit::Str("]".to_string()))],
                                ),
                            ],
                        ));
                    }
                }
            }
        }
        if *ty == date_object_type() {
            let name = format!("__thaw_date_object_tag_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            let result = HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_date_has_native_identity".into())),
                    vec![HirExpr::Var(name.clone())],
                )),
                Box::new(HirExpr::Lit(HirLit::Str("[object Date]".into()))),
                Box::new(HirExpr::Lit(HirLit::Str("[object Object]".into()))),
                HirType::Str,
            );
            return self.wrap_call_argument_bindings(result, &[(name, ty.clone(), value)]);
        }
        let static_tag = match ty {
            HirType::Undefined => "Undefined",
            HirType::Null => "Null",
            HirType::Array(_) | HirType::Tuple(_) => "Array",
            HirType::Function(_, _) | HirType::CallableFunction(_, _, _, _) => "Function",
            _ if *ty == date_object_type() => "Date",
            _ if *ty == regex_object_type() => "RegExp",
            HirType::F64 => "Number",
            HirType::Str | HirType::StrLiteral(_) => "String",
            HirType::Bool => "Boolean",
            HirType::Map(_, _) => "Map",
            HirType::WeakMap(_, _) => "WeakMap",
            HirType::Set(_) => "Set",
            HirType::WeakSet(_) => "WeakSet",
            HirType::Bytes => "Uint8Array",
            HirType::Symbol => "Symbol",
            HirType::Promise(_) => "Promise",
            HirType::I64 => "BigInt",
            _ if native_base_tag(ty).is_some() => native_base_tag(ty).unwrap_or("Object"),
            _ => "Object",
        };
        let name = format!("__thaw_object_prototype_to_string_{}", self.next_binding);
        self.next_binding += 1;
        self.wrap_call_argument_bindings(
            HirExpr::Lit(HirLit::Str(format!("[object {static_tag}]"))),
            &[(name, ty.clone(), value)],
        )
    }

    fn class_receiver_has_own_field(ty: &HirType, property: &str) -> bool {
        matches!(ty, HirType::Object(fields) if fields.iter().any(|(name, _)| name == property))
    }

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
                            if self.is_unshadowed_class_identifier(class.sym.as_ref()) {
                                if let Some(signature) = self.signatures.get(&symbol) {
                                    return Some(signature.ret.clone());
                                }
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
        let receiver_count = 1;
        let fixed_param_count = signature
            .params
            .len()
            .saturating_sub(receiver_count + usize::from(signature.native_rest.is_some()));
        let fixed_params = &signature.params[receiver_count..receiver_count + fixed_param_count];
        let rest = signature.native_rest.clone();
        if rest.is_none() && leading_values.len() > fixed_params.len() {
            return Err(format!(
                "native class method `{class_name}.{method_name}.bind` binds {} leading argument(s), but the method accepts {}",
                leading_values.len(),
                fixed_params.len()
            ));
        }
        let fixed_bound_count = leading_values.len().min(fixed_params.len());

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
        let mut bound_argument_types = Vec::with_capacity(leading_values.len());
        for (index, value) in leading_values.iter().enumerate() {
            let expected = fixed_params.get(index).or(rest.as_ref()).ok_or_else(|| {
                format!("native class method `{class_name}.{method_name}.bind` has no parameter {}", index + 1)
            })?;
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
            bound_argument_types.push(expected.clone());
            bindings.push((name, expected.clone(), value));
        }
        let remaining_fixed = fixed_params[fixed_bound_count..].to_vec();
        let mut parameters = remaining_fixed
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_bound_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        let invocation_rest_name = rest.as_ref().map(|element| {
            let name = format!("__thaw_bound_rest_{}", self.next_binding);
            self.next_binding += 1;
            parameters.push(HirParam {
                name: name.clone(),
                ty: HirType::Array(Box::new(element.clone())),
            });
            name
        });
        let mut arguments = vec![HirExpr::Var(bound_name.clone())];
        arguments.extend(
            bound_argument_names[..fixed_bound_count]
                .iter()
                .cloned()
                .map(HirExpr::Var),
        );
        arguments.extend(
            parameters[..remaining_fixed.len()]
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        if let (Some(element), Some(invocation_rest_name)) = (&rest, invocation_rest_name) {
            let bound_rest = bound_argument_names[fixed_bound_count..]
                .iter()
                .cloned()
                .map(HirExpr::Var)
                .collect::<Vec<_>>();
            let rest_argument = if bound_rest.is_empty() {
                HirExpr::Var(invocation_rest_name)
            } else {
                HirExpr::ArrayConcat(
                    vec![
                        HirExpr::ArrayLit(bound_rest),
                        HirExpr::Var(invocation_rest_name),
                    ],
                    element.clone(),
                )
            };
            arguments.push(rest_argument);
        }
        let mut captures = vec![HirParam {
            name: bound_name.clone(),
            ty: target_type.clone(),
        }];
        captures.extend(
            bound_argument_names
                .iter()
                .cloned()
                .zip(bound_argument_types)
                .map(|(name, ty)| HirParam { name, ty }),
        );
        let closure_return = if signature.is_async && !matches!(signature.ret, HirType::Promise(_))
        {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let closure_type = HirType::CallableFunction(
            remaining_fixed,
            signature.source_omittable.shifted(receiver_count + fixed_bound_count),
            rest.map(Box::new),
            Box::new(closure_return.clone()),
        );
        let closure = HirExpr::TypedClosure(closure_type, Box::new(HirExpr::Lambda(
            captures,
            parameters,
            closure_return,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments)),
        )));
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
        if !self.is_unshadowed_class_identifier(class.sym.as_ref()) {
            return Ok(None);
        }
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
        let fixed_param_count = signature
            .params
            .len()
            .saturating_sub(usize::from(signature.native_rest.is_some()));
        let fixed_params = &signature.params[..fixed_param_count];
        let rest = signature.native_rest.clone();
        if rest.is_none() && leading_values.len() > fixed_params.len() {
            return Err(format!(
                "native static method `{}.{method_name}.bind` binds {} leading argument(s), but the method accepts {}",
                class.sym,
                leading_values.len(),
                fixed_params.len()
            ));
        }
        let fixed_bound_count = leading_values.len().min(fixed_params.len());
        let this_name = format!("__thaw_static_bind_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        let mut bindings = vec![(this_name, this_type, this_value)];
        bindings.extend(leading_bindings);
        let mut captures = Vec::new();
        let mut bound_names = Vec::with_capacity(leading_values.len());
        let mut bound_types = Vec::with_capacity(leading_values.len());
        for (index, value) in leading_values.iter().enumerate() {
            let expected = fixed_params.get(index).or(rest.as_ref()).ok_or_else(|| {
                format!("native static method `{}.{method_name}.bind` has no parameter {}", class.sym, index + 1)
            })?;
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
            bound_names.push(name);
            bound_types.push(expected.clone());
        }
        let remaining_fixed = fixed_params[fixed_bound_count..].to_vec();
        let mut parameters = remaining_fixed
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_static_bound_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        let invocation_rest_name = rest.as_ref().map(|element| {
            let name = format!("__thaw_static_bound_rest_{}", self.next_binding);
            self.next_binding += 1;
            parameters.push(HirParam {
                name: name.clone(),
                ty: HirType::Array(Box::new(element.clone())),
            });
            name
        });
        let mut arguments = bound_names[..fixed_bound_count]
            .iter()
            .cloned()
            .map(HirExpr::Var)
            .collect::<Vec<_>>();
        arguments.extend(
            parameters[..remaining_fixed.len()]
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone())),
        );
        if let (Some(element), Some(invocation_rest_name)) = (&rest, invocation_rest_name) {
            let bound_rest = bound_names[fixed_bound_count..]
                .iter()
                .cloned()
                .map(HirExpr::Var)
                .collect::<Vec<_>>();
            let rest_argument = if bound_rest.is_empty() {
                HirExpr::Var(invocation_rest_name)
            } else {
                HirExpr::ArrayConcat(
                    vec![
                        HirExpr::ArrayLit(bound_rest),
                        HirExpr::Var(invocation_rest_name),
                    ],
                    element.clone(),
                )
            };
            arguments.push(rest_argument);
        }
        captures.extend(
            bound_names
                .into_iter()
                .zip(bound_types)
                .map(|(name, ty)| HirParam { name, ty }),
        );
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let closure_type = HirType::CallableFunction(
            remaining_fixed,
            signature.source_omittable.shifted(fixed_bound_count),
            rest.map(Box::new),
            Box::new(result.clone()),
        );
        let closure = HirExpr::TypedClosure(closure_type, Box::new(HirExpr::Lambda(
            captures,
            parameters,
            result,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments)),
        )));
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
            if let Some(signature) = self.signatures.get(&symbol)
                .filter(|_| self.is_unshadowed_class_identifier(class.sym.as_ref())) {
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
                        None,
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
        if Self::class_receiver_has_own_field(&receiver_type, method_name) {
            return Ok(None);
        }
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
                Some(class_name.to_string()),
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
        let call = HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments);
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        let mut closure = HirExpr::Lambda(
            vec![HirParam {
                name: receiver_name.clone(),
                ty: receiver_type.clone(),
            }],
            parameters,
            result,
            Box::new(call),
        );
        // An unannotated method's signature still defers its return type (`Dynamic`); the
        // closure's inferred type resolves it from the lowered body.
        if signature.ret == HirType::Dynamic {
            if let HirType::Function(_, resolved) = self.infer_expr_type(&closure)? {
                if let HirExpr::Lambda(_, _, result, _) = &mut closure {
                    *result = *resolved;
                }
            }
        }
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
            if self.is_unshadowed_class_identifier(class.sym.as_ref())
                && self.signatures.get(&symbol)
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
        if Self::class_receiver_has_own_field(&receiver, &method) {
            return None;
        }
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
            HirExpr::TypedClosure(
                HirType::CallableFunction(
                    parameters.iter().map(|parameter| parameter.ty.clone()).collect(),
                    signature.source_omittable.shifted(receiver_count + leading_values.len()),
                    None,
                    Box::new(result.clone()),
                ),
                Box::new(HirExpr::Lambda(
                captures,
                parameters,
                result,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var(method.symbol.clone())),
                    arguments,
                )),
                )),
            ),
            &bindings,
        )
    }

    // A function operation reads `.call`/`.apply`/`.bind` before it evaluates
    // thisArg or arguments. This block is a binding-wrapper lambda body, not
    // a standalone expression block.
    pub(crate) fn guard_function_value(
        &mut self, value: HirExpr, ty: &HirType, message: &str, body: HirExpr,
    ) -> Result<HirExpr, String> {
        let result_type = self.infer_expr_type(&body)?;
        let mut statements = vec![HirStmt::If(
            Self::function_pointer_is_undefined(value, ty),
            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                format!("\u{1}TypeError\u{1}{message}"),
            )))],
            Vec::new(),
        )];
        if result_type == HirType::Void {
            statements.push(HirStmt::Expr(body));
        } else {
            statements.push(HirStmt::Return(Some(body)));
        }
        Ok(HirExpr::Block(statements))
    }

    pub(in crate::lower) fn logical_function_ref_type(
        &self,
        symbol: &str,
        physical_params: &[HirType],
        ret: &HirType,
    ) -> HirType {
        let Some(signature) = self.signatures.get(symbol) else {
            return HirType::Function(physical_params.to_vec(), Box::new(ret.clone()));
        };
        if !signature.generic_type_params.is_empty() {
            return HirType::Function(physical_params.to_vec(), Box::new(ret.clone()));
        }
        let rest = signature.native_rest.clone();
        if rest.as_ref().is_some_and(|element| {
            physical_params.last() != Some(&HirType::Array(Box::new(element.clone())))
        }) {
            return HirType::Function(physical_params.to_vec(), Box::new(ret.clone()));
        }
        let fixed_count = physical_params.len().saturating_sub(usize::from(rest.is_some()));
        let mut optional = HirOptionalMask::default();
        let mut fixed = physical_params[..fixed_count].to_vec();
        for (index, parameter) in fixed.iter_mut().enumerate() {
            if signature.source_omittable.contains(index) {
                optional.insert(index);
                *parameter = optional_parameter_type(parameter.clone());
            }
        }
        if optional.is_empty() && rest.is_none() {
            HirType::Function(physical_params.to_vec(), Box::new(ret.clone()))
        } else {
            HirType::CallableFunction(fixed, optional, rest.map(Box::new), Box::new(ret.clone()))
        }
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
        let (public_overloads, target_type) = match target_type {
            HirType::OverloadedFunction(public, physical) => (Some(public), *physical),
            other => (None, other),
        };
        let (target_type, declared_receiver) = match target_type {
            HirType::FunctionWithThis(receiver, visible) => (*visible, Some(*receiver)),
            other => (other, None),
        };
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
        let this_value = self.coerce_this_for_receiver(declared_receiver.as_ref(), this_value)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let mut residual_overloads = Vec::new();
        let mut overload_argument_contexts = Vec::new();
        if let Some(overloads) = public_overloads.as_deref() {
            let mut viable = Vec::new();
            for public in overloads {
                let (public_receiver, callable) = match public {
                    HirType::FunctionWithThis(receiver, callable) => {
                        (Some(receiver.as_ref()), callable.as_ref())
                    }
                    callable => (None, callable),
                };
                if public_receiver.is_some_and(|receiver| {
                    self.expect_type(receiver, &this_value, "overload bind receiver").is_err()
                }) {
                    continue;
                }
                let (public_params, public_rest) = match callable {
                    HirType::Function(params, _) => (
                        params.as_slice(), None,
                    ),
                    HirType::CallableFunction(params, _, rest, _) => (
                        params.as_slice(), rest.as_deref(),
                    ),
                    _ => continue,
                };
                let state = self.call_constraints.map(|constraints| constraints.borrow().len());
                let mut trial = self.clone();
                let mut contexts = Vec::new();
                let mut logical_count = 0usize;
                let mut applicable = true;
                for argument in leading {
                    if argument.spread.is_none() {
                        let Some(declared) = public_params.get(logical_count).or(public_rest) else {
                            applicable = false;
                            break;
                        };
                        let Ok(value) = trial.lower_expr_with_expected_type(&argument.expr, Some(declared)) else {
                            applicable = false;
                            break;
                        };
                        if trial.expect_public_overload_argument(declared, &value).is_err() {
                            applicable = false;
                            break;
                        }
                        contexts.push(declared.clone());
                        logical_count += 1;
                        continue;
                    }
                    if let swc_ecma_ast::Expr::Array(array) = argument.expr.as_ref() {
                        if array.elems.iter().any(|element| {
                            element.as_ref().is_none_or(|element| element.spread.is_some())
                        }) {
                            applicable = false;
                            break;
                        }
                        for element in array.elems.iter().flatten() {
                            let Some(declared) = public_params.get(logical_count).or(public_rest) else {
                                applicable = false;
                                break;
                            };
                            let Ok(value) = trial.lower_expr_with_expected_type(&element.expr, Some(declared)) else {
                                applicable = false;
                                break;
                            };
                            if trial.expect_public_overload_argument(declared, &value).is_err() {
                                applicable = false;
                                break;
                            }
                            contexts.push(declared.clone());
                            logical_count += 1;
                        }
                        if !applicable { break; }
                        continue;
                    }
                    let Ok(value) = trial.lower_expr_with_expected_type(&argument.expr, None) else {
                        applicable = false;
                        break;
                    };
                    let Ok(HirType::Tuple(elements)) = trial.infer_expr_type(&value) else {
                        applicable = false;
                        break;
                    };
                    for actual in elements {
                        let Some(declared) = public_params.get(logical_count).or(public_rest) else {
                            applicable = false;
                            break;
                        };
                        let typed = HirExpr::TypedClosure(actual, Box::new(HirExpr::Lit(HirLit::Undefined)));
                        if trial.expect_public_overload_argument(declared, &typed).is_err() {
                            applicable = false;
                            break;
                        }
                        contexts.push(declared.clone());
                        logical_count += 1;
                    }
                    if !applicable { break; }
                }
                applicable &= public_rest.is_some() || logical_count <= public_params.len();
                if let (Some(constraints), Some(length)) = (self.call_constraints, state) {
                    constraints.borrow_mut().truncate(length);
                }
                if !applicable {
                    continue;
                }
                let bound_fixed = logical_count.min(public_params.len());
                let residual = match callable {
                    HirType::Function(params, ret) => {
                        HirType::Function(params[bound_fixed..].to_vec(), ret.clone())
                    }
                    HirType::CallableFunction(params, optional, rest, ret) => {
                        HirType::CallableFunction(
                            params[bound_fixed..].to_vec(),
                            optional.shifted(bound_fixed),
                            rest.clone(),
                            ret.clone(),
                        )
                    }
                    _ => unreachable!(),
                };
                viable.push((residual, contexts));
            }
            if viable.is_empty() {
                return Err("no public overload accepts the supplied bind receiver and prefix".into());
            }
            for (residual, _) in &viable {
                residual_overloads.push(residual.clone());
            }
            let context_count = viable.iter().map(|candidate| candidate.1.len()).max().unwrap_or(0);
            for argument_index in 0..context_count {
                let first = viable[0].1.get(argument_index).cloned();
                let agreed = first.filter(|context| viable.iter().all(|candidate| {
                    candidate.1.get(argument_index) == Some(context)
                }));
                overload_argument_contexts.push(agreed);
            }
        }
        let (leading, spread_bindings) =
            if public_overloads.is_some() {
                let expected = if overload_argument_contexts.iter().all(Option::is_some) {
                    overload_argument_contexts.iter().filter_map(Clone::clone).collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                self.lower_native_spread_values_with_overload_expected(
                    leading, "function .bind()", &expected, None,
                )?
            } else {
                self.lower_native_spread_values(leading, "function .bind()")?
            };
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
            let mut argument_bindings = vec![(this_name.clone(), this_type.clone(), this_value)];
            argument_bindings.extend(spread_bindings);

            let mut bound_names = Vec::with_capacity(leading.len());
            for (index, value) in leading.into_iter().enumerate() {
                let expected = params
                    .get(index)
                    .or(rest.as_ref())
                    .expect("leading callable argument has a fixed or rest type");
                let selected = overload_argument_contexts
                    .get(index)
                    .and_then(Option::as_ref);
                let value = self.adapt_selected_overload_argument(expected, selected, value)?;
                let value = self.coerce_to_declared(expected, value)?;
                let name = format!("__thaw_rest_bound_argument_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), expected.clone());
                argument_bindings.push((name.clone(), expected.clone(), value));
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
                    name: target_name.clone(),
                    ty: HirType::CallableFunction(
                        params.clone(),
                        optional.clone(),
                        rest.clone().map(Box::new),
                        Box::new(ret.clone()),
                    ),
                },
                HirParam {
                    name: this_name,
                    ty: this_type,
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
            let logical = if public_overloads.is_some() {
                HirType::OverloadedFunction(residual_overloads.clone(), Box::new(logical))
            } else {
                logical
            };
            let bound = self.wrap_call_argument_bindings(
                HirExpr::TypedClosure(logical, Box::new(closure)),
                &argument_bindings,
            )?;
            let guarded = self.guard_function_value(
                HirExpr::Var(target_name.clone()), &target_type,
                "Cannot read properties of undefined (reading 'bind')", bound,
            )?;
            return self.wrap_call_argument_bindings(
                guarded, &[(target_name, target_type, target)],
            ).map(Some);
        }
        if leading.len() > params.len() {
            return Err(format!(
                "function .bind() binds {} leading argument(s), but the function accepts {}",
                leading.len(),
                params.len()
            ));
        }
        let bound_count = leading.len();
        let overload_physical_bound = HirType::Function(
            params[bound_count..].to_vec(),
            Box::new(ret.clone()),
        );
        let leading = leading
            .into_iter()
            .zip(&params)
            .enumerate()
            .map(|(index, (value, expected))| {
                let selected = overload_argument_contexts
                    .get(index)
                    .and_then(Option::as_ref);
                let value = self.adapt_selected_overload_argument(expected, selected, value)?;
                self.coerce_to_declared(expected, value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let target_name = format!("__thaw_function_bind_target_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(target_name.clone(), target_type.clone());
        let this_name = format!("__thaw_function_bind_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        let mut argument_bindings = vec![(this_name.clone(), this_type, this_value)];
        argument_bindings.extend(spread_bindings);
        let bound = self.wrap_call_argument_bindings(
            HirExpr::FunctionBindThis(
                Box::new(HirExpr::Var(target_name.clone())),
                Box::new(HirExpr::Var(this_name)),
                leading,
                params,
                ret,
            ),
            &argument_bindings,
        )?;
        let bound = if public_overloads.is_some() {
            HirExpr::TypedClosure(
                HirType::OverloadedFunction(residual_overloads, Box::new(overload_physical_bound)),
                Box::new(bound),
            )
        } else {
            bound
        };
        let guarded = self.guard_function_value(
            HirExpr::Var(target_name.clone()), &target_type,
            "Cannot read properties of undefined (reading 'bind')", bound,
        )?;
        self.wrap_call_argument_bindings(guarded, &[(target_name, target_type, target)])
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
        // `Reflect.apply(...)` (a static builtin whose own name is `apply`)
        // isn't a function value's `.apply` -- leave it to
        // `is_static_builtin_call`'s dispatch below instead of trying to
        // lower `Reflect` as a variable.
        if let Expr::Ident(object) = operation.obj.as_ref() {
            if Self::is_static_builtin_call(object.sym.as_ref(), &operation_name) {
                return Ok(None);
            }
        }
        let target = self.lower_expr(&operation.obj)?;
        let target_type = self.infer_expr_type(&target)?;
        let (public_overloads, target_type) = match target_type {
            HirType::OverloadedFunction(public, physical) => (Some(public), *physical),
            other => (None, other),
        };
        // A `this`-taking closure keeps its declared receiver type: the explicit
        // `thisArg` is converted to it before the tagged receiver entry decodes it.
        let (target_type, declared_receiver) = match target_type {
            HirType::FunctionWithThis(receiver, visible) => (*visible, Some(*receiver)),
            other => (other, None),
        };
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
            match supplied {
                [] => Vec::new(),
                [arguments] => {
                    if arguments.spread.is_some() {
                        return Err("function .apply() tuple cannot itself be spread".into());
                    }
                    vec![swc_ecma_ast::ExprOrSpread {
                        spread: Some(call.span),
                        expr: arguments.expr.clone(),
                    }]
                }
                _ => {
                    return Err(
                        "function .apply() expects exactly a thisArg and an argument tuple".into(),
                    )
                }
            }
        } else {
            supplied.to_vec()
        };
        let quickjs_function_value = self
            .validate_quickjs_function_value_arguments(&operation.obj, &forwarded)?;
        let this_value = self.lower_expr(&this_argument.expr)?;
        let this_value = self.coerce_this_for_receiver(declared_receiver.as_ref(), this_value)?;
        let this_type = self.infer_expr_type(&this_value)?;
        let label = format!("function .{operation_name}()");
        let selected_public_signature = public_overloads
            .as_deref()
            .map(|overloads| {
                self.select_public_overload_for_source(
                    overloads,
                    &forwarded,
                    Some(&this_value),
                    &label,
                )
            })
            .transpose()?
            .flatten();
        let selected_public_return = selected_public_signature
            .as_ref()
            .and_then(|public| self.public_overload_return_type(public));
        let public_rest = selected_public_signature.as_ref().and_then(|public| match public {
            HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
                _ => None,
            },
            HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
            _ => None,
        });
        let public_params = selected_public_signature.as_ref().map(|public| {
            (0..params.len())
                .filter_map(|index| self.public_overload_visible_argument_type(public, index))
                .collect::<Vec<_>>()
        });
        let (arguments, spread_bindings) = if let Some(public_params) = &public_params {
            self.lower_native_spread_values_with_overload_expected(
                &forwarded,
                &label,
                public_params,
                public_rest,
            )?
        } else {
            self.lower_native_spread_values(&forwarded, &label)?
        };
        let mut arguments = arguments;
        if let Some(public_fixed_count) = selected_public_signature.as_ref().and_then(|public| match public {
            HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                HirType::CallableFunction(params, _, Some(_), _) => Some(params.len()),
                _ => None,
            },
            HirType::CallableFunction(params, _, Some(_), _) => Some(params.len()),
            _ => None,
        }).filter(|count| *count < params.len()) {
            if arguments.len() > public_fixed_count {
                let public_rest_values = arguments.split_off(public_fixed_count);
                for parameter in params.iter().skip(public_fixed_count) {
                    arguments.push(omitted_parameter_value(parameter)?);
                }
                arguments.extend(public_rest_values);
            }
        }
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
            let values = arguments.split_off(arguments.len().min(params.len()));
            (element, values)
        });
        for parameter in params.iter().skip(arguments.len()) {
            arguments.push(omitted_parameter_value(parameter)?);
        }
        let mut arguments = arguments
            .into_iter()
            .zip(&params)
            .enumerate()
            .map(|(index, (argument, expected))| {
                let public_expected = selected_public_signature
                    .as_ref()
                    .and_then(|public| self.public_overload_visible_argument_type(public, index));
                let argument = self.adapt_selected_overload_argument(
                    expected,
                    public_expected.as_ref(),
                    argument,
                )?;
                if quickjs_function_value {
                    self.coerce_primitive_array_argument_aliasing(argument, expected)
                } else {
                    self.coerce_to_declared(expected, argument)
                }
            })
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
        let mut argument_bindings = vec![(this_name.clone(), this_type, this_value)];
        argument_bindings.extend(spread_bindings);
        let invoked = HirExpr::FunctionCallWithThis(
                Box::new(HirExpr::Var(target_name.clone())),
                Box::new(HirExpr::Var(this_name)),
                arguments,
                abi_params,
                ret.clone(),
            );
        let invoked = if let Some(selected) = selected_public_return {
            self.project_overload_call_result(invoked, &ret, &selected)?
        } else {
            invoked
        };
        let invoked = self.wrap_call_argument_bindings(
            invoked,
            &argument_bindings,
        )?;
        let guarded = self.guard_function_value(
            HirExpr::Var(target_name.clone()), &target_type,
            &format!("Cannot read properties of undefined (reading '{operation_name}')"),
            invoked,
        )?;
        self.wrap_call_argument_bindings(guarded, &[(target_name, target_type, target)])
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
            // A saved symbol is only an optimization hint. Capture the selected
            // function before arguments, but report a missing call after them.
            let selected = HirExpr::Var(binding.clone());
            let selected_type = self.infer_expr_type(&selected)?;
            let selected_name = format!("__thaw_saved_selected_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(selected_name.clone(), selected_type.clone());
            let label = format!("unbound native method `{}`", target.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
            let symbol = unbound_class_method_symbol(&method.symbol);
            let signature = self.signatures.get(&symbol).cloned().unwrap_or_else(|| {
                let mut signature = self.signatures[&method.symbol].clone();
                if method.receiver.is_some() {
                    signature.params.remove(0);
                }
                signature
            });
            let arguments = self.pad_source_mask_arguments(
                &signature,
                arguments,
                0,
                &label,
            )?;
            let guarded = self.guard_function_value(
                HirExpr::Var(selected_name.clone()), &selected_type,
                "Value is not a function",
                HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments),
            )?;
            let after_arguments = self.wrap_call_argument_bindings(guarded, &bindings)?;
            return self.wrap_call_argument_bindings(
                after_arguments, &[(selected_name, selected_type, selected)],
            ).map(Some);
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
        let selected = HirExpr::Var(binding.clone());
        let selected_type = self.infer_expr_type(&selected)?;
        let selected_name = format!("__thaw_saved_operation_selected_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(selected_name.clone(), selected_type.clone());
        if operation_name == "bind" {
            let bound = self.lower_saved_native_method_bind(target.sym.as_ref(), &method, call)?;
            let guarded = self.guard_function_value(
                HirExpr::Var(selected_name.clone()), &selected_type,
                "Cannot read properties of undefined (reading 'bind')", bound,
            )?;
            return self.wrap_call_argument_bindings(
                guarded, &[(selected_name, selected_type, selected)],
            ).map(Some);
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
            let symbol = method.symbol.clone();
            let signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .expect("saved static method retains its signature");
            let arguments = self.pad_source_mask_arguments(
                &signature,
                arguments,
                0,
                &label,
            )?;
            bindings.insert(0, (this_name, this_type, this_value));
            let invoked = self.wrap_call_argument_bindings(
                HirExpr::Call(Box::new(HirExpr::Var(symbol)), arguments),
                &bindings,
            )?;
            let guarded = self.guard_function_value(
                HirExpr::Var(selected_name.clone()), &selected_type,
                &format!("Cannot read properties of undefined (reading '{operation_name}')"),
                invoked,
            )?;
            return self.wrap_call_argument_bindings(
                guarded, &[(selected_name, selected_type, selected)],
            ).map(Some);
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
        let guarded = self.guard_function_value(
            HirExpr::Var(selected_name.clone()), &selected_type,
            &format!("Cannot read properties of undefined (reading '{operation_name}')"),
            lowered,
        )?;
        self.wrap_call_argument_bindings(
            guarded, &[(selected_name, selected_type, selected)],
        ).map(Some)
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
        if !self.is_unshadowed_class_identifier(class.sym.as_ref()) {
            return Ok(None);
        }
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
            let values = arguments.split_off(arguments.len().min(params.len()));
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
