impl<'a> FnLowerer<'a> {
    fn validate_array_callback_value(
        &self,
        callback: HirExpr,
        available: &[HirType],
        expected_return: Option<&HirType>,
        label: &str,
    ) -> Result<HirExpr, String> {
        let (params, ret) = match self.infer_expr_type(&callback)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(params, _, None, ret) => (params, ret),
            _ => return Err(format!("{label} callback is not a function value")),
        };
        if params.len() > available.len() || params != available[..params.len()] {
            return Err(format!(
                "{label} callback has parameters {params:?}, expected a prefix of {available:?}"
            ));
        }
        if expected_return.is_some_and(|expected| ret.as_ref() != expected) {
            return Err(format!(
                "{label} callback returns {ret:?}, expected {:?}",
                expected_return.unwrap()
            ));
        }
        Ok(callback)
    }

    fn lower_spread_promise_combinator(
        &mut self,
        value: HirExpr,
        combinator: &str,
    ) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::Array(element) => {
                let HirType::Promise(resolved) = *element else {
                    return Err(format!(
                        "`Promise.{combinator}` expects an array of promises"
                    ));
                };
                if *resolved == HirType::Void {
                    return Err(format!(
                        "`Promise.{combinator}` elements must not resolve to void"
                    ));
                }
                Ok(match combinator {
                    "all" => HirExpr::PromiseAllArray(Box::new(value), *resolved),
                    "allSettled" => {
                        HirExpr::PromiseAllSettledArray(Box::new(value), *resolved)
                    }
                    "race" => HirExpr::PromiseRaceArray(Box::new(value), *resolved),
                    "any" => HirExpr::PromiseAnyArray(Box::new(value), *resolved),
                    _ => unreachable!(),
                })
            }
            HirType::Union(members) => {
                let mut resolved = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    let HirType::Array(element) = member else {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} is not an array: {member:?}"
                        ));
                    };
                    let HirType::Promise(payload) = element.as_ref() else {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} is not an array of promises: {member:?}"
                        ));
                    };
                    if payload.as_ref() == &HirType::Void {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} resolves to void"
                        ));
                    }
                    resolved.push(payload.as_ref().clone());
                }

                let branch_results = resolved
                    .iter()
                    .map(|payload| match combinator {
                        "all" => HirType::Array(Box::new(payload.clone())),
                        "allSettled" => HirType::Array(Box::new(
                            promise_settled_result_type(payload.clone()),
                        )),
                        "race" | "any" => payload.clone(),
                        _ => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                let mut result_members = Vec::new();
                for result in &branch_results {
                    let member = match result {
                        HirType::Array(element) => element.as_ref(),
                        other => other,
                    };
                    Self::flatten_property_union_members(member, &mut result_members)?;
                }
                let common_result = match result_members.as_slice() {
                    [single] => single.clone(),
                    _ => HirType::Union(result_members),
                };
                let result_type = match combinator {
                    "all" | "allSettled" => HirType::Array(Box::new(common_result)),
                    "race" | "any" => common_result,
                    _ => unreachable!(),
                };
                let source_type = HirType::Union(members.clone());
                let source_name = format!("__thaw_promise_union_array_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), source_type.clone());

                let mut branches = Vec::with_capacity(members.len());
                for (index, (payload, branch_result)) in resolved
                    .into_iter()
                    .zip(branch_results)
                    .enumerate()
                {
                    let array = HirExpr::UnionValue(
                        Box::new(HirExpr::Var(source_name.clone())),
                        index,
                        members.clone(),
                    );
                    let promise = match combinator {
                        "all" => HirExpr::PromiseAllArray(Box::new(array), payload),
                        "allSettled" => {
                            HirExpr::PromiseAllSettledArray(Box::new(array), payload)
                        }
                        "race" => HirExpr::PromiseRaceArray(Box::new(array), payload),
                        "any" => HirExpr::PromiseAnyArray(Box::new(array), payload),
                        _ => unreachable!(),
                    };
                    // Widen fulfilled values after the exact member combinator
                    // runs so its promise layout and rejection path stay intact.
                    if branch_result == result_type {
                        branches.push(promise);
                        continue;
                    }
                    let result_name = format!("__thaw_promise_union_result_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(result_name.clone(), branch_result.clone());
                    let widened = match (&branch_result, &result_type) {
                        (HirType::Array(source), HirType::Array(target)) => {
                            let element_name = format!(
                                "__thaw_promise_union_element_{}",
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope
                                .insert(element_name.clone(), source.as_ref().clone());
                            let converted = self.coerce_to_declared(
                                target,
                                HirExpr::Var(element_name.clone()),
                            )?;
                            self.lower_array_map(
                                HirExpr::Var(result_name.clone()),
                                branch_result.clone(),
                                source.as_ref().clone(),
                                source.as_ref().clone(),
                                HirExpr::Lambda(
                                    Vec::new(),
                                    vec![HirParam {
                                        name: element_name,
                                        ty: source.as_ref().clone(),
                                    }],
                                    target.as_ref().clone(),
                                    Box::new(converted),
                                ),
                                None,
                            )?
                        }
                        _ => self.coerce_to_declared(
                            &result_type,
                            HirExpr::Var(result_name.clone()),
                        )?,
                    };
                    branches.push(HirExpr::PromiseThen(
                        Box::new(promise),
                        Box::new(HirExpr::Lambda(
                            Vec::new(),
                            vec![HirParam {
                                name: result_name,
                                ty: branch_result.clone(),
                            }],
                            result_type.clone(),
                            Box::new(widened),
                        )),
                        branch_result,
                        result_type.clone(),
                        false,
                        false,
                    ));
                }
                let result = self.merge_union_array_method_branches(
                    &source_name,
                    &members,
                    branches,
                )?;
                self.wrap_call_argument_bindings(result, &[(source_name, source_type, value)])
            }
            HirType::Tuple(elements) => {
                let source_type = HirType::Tuple(elements.clone());
                let source_name = format!("__thaw_promise_tuple_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), source_type.clone());
                let mut resolved = Vec::with_capacity(elements.len());
                let mut promises = Vec::with_capacity(elements.len());
                for (index, element) in elements.into_iter().enumerate() {
                    let HirType::Promise(payload) = &element else {
                        return Err(format!(
                            "Promise.{combinator} element {index} must be a Promise, got {element:?}"
                        ));
                    };
                    if payload.as_ref() == &HirType::Void {
                        return Err(format!(
                            "Promise.{combinator} element {index} resolves to void"
                        ));
                    }
                    resolved.push(payload.as_ref().clone());
                    promises.push(HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(source_name.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element,
                    ));
                }
                let first = resolved.first().cloned().unwrap_or(HirType::F64);
                if combinator == "all" && resolved.iter().any(|element| element != &first) {
                    return self.wrap_call_argument_bindings(
                        HirExpr::PromiseAllTuple(promises, resolved),
                        &[(source_name, source_type, value)],
                    );
                }
                if let Some((index, actual)) = resolved
                    .iter()
                    .enumerate()
                    .find(|(_, element)| *element != &first)
                {
                    return Err(format!(
                        "Promise.{combinator} element {index} resolves to {actual:?}, expected {first:?}"
                    ));
                }
                let result = match combinator {
                    "all" => HirExpr::PromiseAll(promises, first),
                    "allSettled" => HirExpr::PromiseAllSettled(promises, first),
                    "race" => HirExpr::PromiseRace(promises, first),
                    "any" => HirExpr::PromiseAny(promises, first),
                    _ => unreachable!(),
                };
                self.wrap_call_argument_bindings(result, &[(source_name, source_type, value)])
            }
            other => Err(format!(
                "`Promise.{combinator}` expects an array of promises, got {other:?}"
            )),
        }
    }

    fn lower_parse_call(&mut self, call: &CallExpr, parse_int: bool) -> Result<HirExpr, String> {
        let label = if parse_int { "parseInt" } else { "parseFloat" };
        let expected = if parse_int { 1..=2 } else { 1..=1 };
        let (arguments, bindings) = self.lower_native_spread_values(&call.args, label)?;
        if !expected.contains(&arguments.len()) {
            return Err(format!(
                "`{label}` expects one{} argument",
                if parse_int { " or two" } else { "" }
            ));
        }
        let text = arguments[0].clone();
        let text = self.coerce_primitive_to_string(text)?;
        let mut lowered = vec![text];
        if parse_int {
            let radix = if let Some(argument) = arguments.get(1) {
                self.coerce_primitive_to_number(argument.clone())?
            } else {
                HirExpr::Lit(HirLit::F64(0.0))
            };
            lowered.push(radix);
        }
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(
                if parse_int {
                    "__thaw_parse_int"
                } else {
                    "__thaw_parse_float"
                }
                .to_string(),
            )),
            lowered,
        );
        self.wrap_call_argument_bindings(call, &bindings)
    }

    fn lower_array_sort_default(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        copy: bool,
    ) -> Result<HirExpr, String> {
        if matches!(element_type, HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Undefined | HirType::Null | HirType::Union(_) | HirType::Json) {
            return self.lower_array_sort_default_tagged(receiver, array_type, element_type, copy);
        }
        let prefix = match element_type {
            HirType::F64 => "number",
            HirType::Str => "string",
            HirType::Bool => "bool",
            HirType::Object(_) => "object",
            other => return Err(format!("default array sort does not support element type {other:?}")),
        };
        let suffix = if copy { "to_sorted" } else { "sort" };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
            vec![receiver],
        ))
    }

    // ponytail: reuses the existing O(n²) comparator sort; add a native tagged sorter if large arrays matter.
    fn lower_array_sort_default_tagged(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        copy: bool,
    ) -> Result<HirExpr, String> {
        let left = format!("__thaw_sort_default_left_{}", self.next_binding);
        self.next_binding += 1;
        let right = format!("__thaw_sort_default_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(left.clone(), element_type.clone());
        self.scope.insert(right.clone(), element_type.clone());
        let left_string = self.coerce_primitive_to_string(HirExpr::Var(left.clone()))?;
        let right_string = self.coerce_primitive_to_string(HirExpr::Var(right.clone()))?;
        let comparator = HirExpr::Lambda(
            Vec::new(),
            vec![
                HirParam { name: left, ty: element_type.clone() },
                HirParam { name: right, ty: element_type.clone() },
            ],
            HirType::F64,
            Box::new(HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_gt".into())),
                    vec![left_string, right_string],
                )),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                HirType::F64,
            )),
        );
        self.lower_array_sort_comparator(
            receiver,
            array_type,
            element_type.clone(),
            element_type,
            comparator,
            copy,
        )
    }

    fn lower_array_sort_comparator(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        comparator_element_type: HirType,
        comparator: HirExpr,
        copy: bool,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_sort_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_name = format!("__thaw_sort_comparator_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_type = HirType::Function(
            vec![
                comparator_element_type.clone(),
                comparator_element_type.clone(),
            ],
            Box::new(HirType::F64),
        );
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(comparator_name.clone(), comparator_type.clone());

        let array_name = format!("__thaw_sort_array_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_sort_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_name = format!("__thaw_sort_outer_{}", self.next_binding);
        self.next_binding += 1;
        let inner_name = format!("__thaw_sort_inner_{}", self.next_binding);
        self.next_binding += 1;
        let left_name = format!("__thaw_sort_left_{}", self.next_binding);
        self.next_binding += 1;
        let right_name = format!("__thaw_sort_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(outer_name.clone(), HirType::F64);
        self.scope.insert(inner_name.clone(), HirType::F64);
        self.scope.insert(left_name.clone(), element_type.clone());
        self.scope.insert(right_name.clone(), element_type.clone());

        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let variable = |name: &str| HirExpr::Var(name.to_string());
        let add_one =
            |value: HirExpr| HirExpr::BinOp(BinOp::Add, Box::new(value), Box::new(number(1.0)));
        let working_source = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_slice".into())),
            vec![variable(&receiver_name), number(0.0), number(f64::INFINITY)],
        );
        let inner_index = variable(&inner_name);
        let next_index = add_one(inner_index.clone());
        let left_value = HirExpr::TypedIndex(
            Box::new(variable(&array_name)),
            Box::new(inner_index.clone()),
            element_type.clone(),
        );
        let right_value = HirExpr::TypedIndex(
            Box::new(variable(&array_name)),
            Box::new(next_index.clone()),
            element_type.clone(),
        );
        let compare = HirExpr::Call(
            Box::new(variable(&comparator_name)),
            vec![
                self.coerce_to_declared(&comparator_element_type, variable(&left_name))?,
                self.coerce_to_declared(&comparator_element_type, variable(&right_name))?,
            ],
        );
        let should_swap = HirExpr::BinOp(BinOp::Gt, Box::new(compare), Box::new(number(0.0)));
        let inner_limit = HirExpr::BinOp(
            BinOp::Sub,
            Box::new(variable(&length_name)),
            Box::new(variable(&outer_name)),
        );
        let inner_condition = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(add_one(variable(&inner_name))),
            Box::new(inner_limit),
        );
        let outer_condition = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(variable(&outer_name)),
            Box::new(variable(&length_name)),
        );
        let result = if copy {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_densify".into())),
                vec![variable(&array_name)],
            )
        } else {
            variable(&receiver_name)
        };
        let mut statements = vec![
            HirStmt::Let(array_name.clone(), array_type.clone(), working_source),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_compact_for_sort".into())),
                    vec![variable(&array_name)],
                ),
            ),
            HirStmt::Let(outer_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                outer_condition,
                vec![
                    HirStmt::Let(inner_name.clone(), HirType::F64, number(0.0)),
                    HirStmt::While(
                        inner_condition,
                        vec![
                            HirStmt::Let(left_name.clone(), element_type.clone(), left_value),
                            HirStmt::Let(right_name.clone(), element_type.clone(), right_value),
                            HirStmt::If(
                                should_swap,
                                vec![
                                    HirStmt::Expr(HirExpr::IndexAssign(
                                        Box::new(variable(&array_name)),
                                        Box::new(variable(&inner_name)),
                                        Box::new(variable(&right_name)),
                                    )),
                                    HirStmt::Expr(HirExpr::IndexAssign(
                                        Box::new(variable(&array_name)),
                                        Box::new(add_one(variable(&inner_name))),
                                        Box::new(variable(&left_name)),
                                    )),
                                ],
                                Vec::new(),
                            ),
                            HirStmt::Expr(HirExpr::Assign(
                                inner_name.clone(),
                                Box::new(add_one(variable(&inner_name))),
                            )),
                        ],
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        outer_name.clone(),
                        Box::new(add_one(variable(&outer_name))),
                    )),
                ],
            ),
        ];
        if !copy {
            let write_index = format!("__thaw_sort_write_{}", self.next_binding);
            self.next_binding += 1;
            let write_state = format!("__thaw_sort_state_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(write_index.clone(), HirType::F64);
            self.scope.insert(write_state.clone(), HirType::F64);
            let delete_key = self.coerce_primitive_to_string(variable(&write_index))?;
            statements.push(HirStmt::Let(write_index.clone(), HirType::F64, number(0.0)));
            statements.push(HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(variable(&write_index)),
                    Box::new(HirExpr::ArrayLen(Box::new(variable(&array_name)))),
                ),
                vec![
                    HirStmt::Let(write_state.clone(), HirType::F64, HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                        vec![variable(&array_name), variable(&write_index)],
                    )),
                    HirStmt::If(
                        HirExpr::BinOp(BinOp::EqEqEq,
                            Box::new(variable(&write_state)), Box::new(number(0.0))),
                        vec![HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_delete_strict".into())),
                            vec![variable(&receiver_name), delete_key],
                        ))],
                        vec![HirStmt::If(
                            HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(variable(&write_state)), Box::new(number(2.0))),
                            vec![HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_set_undefined".into())),
                                vec![variable(&receiver_name), variable(&write_index)],
                            ))],
                            vec![HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(variable(&receiver_name)),
                                Box::new(variable(&write_index)),
                                Box::new(HirExpr::TypedIndex(
                                    Box::new(variable(&array_name)),
                                    Box::new(variable(&write_index)), element_type.clone(),
                                )),
                            ))],
                        )],
                    ),
                    HirStmt::Expr(HirExpr::Assign(write_index.clone(),
                        Box::new(add_one(variable(&write_index))))),
                ],
            ));
        }
        statements.push(HirStmt::Return(Some(result)));
        let body = HirExpr::Block(statements);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (comparator_name, comparator_type, comparator),
            ],
        )
    }

    /// Builds a call to an array iteration callback, coercing each
    /// argument to the callback's own declared parameter type first.
    ///
    /// The callback's parameter types come from its own signature (e.g. an
    /// explicit `(x: number) =>` annotation), which can be narrower than
    /// the contextual type used to build `available` -- a sparse array's
    /// element read is `T | undefined` (`Optional`) even when an explicit
    /// annotation declares plain `T`, since the compiler respects explicit
    /// annotations over the contextual type (`lower_contextual_arrow`).
    /// Without this coercion, `.map()`/`.filter()`/predicate/`reduce`/
    /// `.forEach()` on a possibly-sparse array pass an `Optional` value's
    /// runtime layout to a callback compiled to expect the bare payload,
    /// which LLVM's module verifier rejects as a parameter type mismatch.
    fn lower_array_callback_call(
        &mut self,
        callback_name: &str,
        params: &[HirType],
        available: &[HirExpr],
    ) -> Result<HirExpr, String> {
        let args = available[..params.len()]
            .iter()
            .cloned()
            .zip(params)
            .map(|(value, declared)| self.coerce_primitive_array_argument(value, declared))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.to_string())),
            args,
        ))
    }

    fn lower_array_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
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
                    .ok_or_else(|| format!("unknown array predicate `{name}`"))?
            }
            _ => return Err("array predicate must be an arrow or function value".into()),
        };
        if arity > 3 {
            return Err(format!(
                "array predicate accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        self.lower_promise_callback(expr, &available[..arity], expected_return)
    }

    fn array_callback_truthy(&mut self, callback: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&callback)?;
        if ty == HirType::Void {
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&callback, &mut referenced);
            let captures = referenced.into_iter().filter_map(|name| {
                self.scope.get(&name).cloned().map(|ty| HirParam { name, ty })
            }).collect();
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    Vec::new(),
                    HirType::Bool,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Expr(callback),
                        HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false)))),
                    ])),
                )),
                Vec::new(),
            ));
        }
        let name = format!("__thaw_array_predicate_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), ty.clone());
        let truthy = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
        self.wrap_call_argument_bindings(truthy, &[(name, ty, callback)])
    }

    fn array_void_to_undefined(&mut self, callback: HirExpr) -> Result<HirExpr, String> {
        Ok(HirExpr::Conditional(
            Box::new(self.array_callback_truthy(callback)?),
            Box::new(HirExpr::Lit(HirLit::Undefined)),
            Box::new(HirExpr::Lit(HirLit::Undefined)),
            HirType::Undefined,
        ))
    }

    fn lower_array_reducer_callback(
        &mut self,
        expr: &Expr,
        accumulator_type: &HirType,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
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
                    .ok_or_else(|| format!("unknown array reducer `{name}`"))?
            }
            _ => return Err("array reducer must be an arrow or function value".into()),
        };
        if arity > 4 {
            return Err(format!(
                "array reducer accepts at most four parameters, got {arity}"
            ));
        }
        let available = [
            accumulator_type.clone(),
            element_type.clone(),
            HirType::F64,
            array_type.clone(),
        ];
        self.lower_promise_callback(expr, &available[..arity], Some(accumulator_type))
    }

    fn lower_array_mapping_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
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
                    .ok_or_else(|| format!("unknown array mapper `{name}`"))?
            }
            _ => return Err("array mapper must be an arrow or function value".into()),
        };
        if arity > 3 {
            return Err(format!(
                "array mapper accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        self.sparse_mapping_result = true;
        let result = self.lower_promise_callback(expr, &available[..arity], None);
        self.sparse_mapping_result = false;
        result
    }

    fn lower_array_from_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
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
                    .ok_or_else(|| format!("unknown Array.from mapper `{name}`"))?
            }
            _ => return Err("Array.from mapper must be an arrow or function value".into()),
        };
        if arity > 2 {
            return Err(format!(
                "Array.from mapper accepts at most two parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64];
        self.sparse_mapping_result = true;
        let result = self.lower_promise_callback(expr, &available[..arity], None);
        self.sparse_mapping_result = false;
        result
    }

}
