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

    fn lower_promise_array_value(
        &mut self,
        expr: &Expr,
        combinator: &str,
    ) -> Result<(HirExpr, HirType), String> {
        let values = self.lower_expr(expr)?;
        let HirType::Array(element) = self.infer_expr_type(&values)? else {
            return Err(format!(
                "`Promise.{combinator}` expects an array of promises"
            ));
        };
        let HirType::Promise(element) = *element else {
            return Err(format!(
                "`Promise.{combinator}` expects an array of promises"
            ));
        };
        if *element == HirType::Void {
            return Err(format!(
                "`Promise.{combinator}` elements must not resolve to void"
            ));
        }
        Ok((values, *element))
    }

    fn lower_spread_promise_combinator(
        &self,
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
            HirType::Tuple(elements) => {
                if elements.is_empty() && matches!(combinator, "race" | "any") {
                    return Err(format!(
                        "`Promise.{combinator}` requires at least one promise"
                    ));
                }
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
                        Box::new(value.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element,
                    ));
                }
                let first = resolved.first().cloned().unwrap_or(HirType::F64);
                if combinator == "all" && resolved.iter().any(|element| element != &first) {
                    return Ok(HirExpr::PromiseAllTuple(promises, resolved));
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
                Ok(match combinator {
                    "all" => HirExpr::PromiseAll(promises, first),
                    "allSettled" => HirExpr::PromiseAllSettled(promises, first),
                    "race" => HirExpr::PromiseRace(promises, first),
                    "any" => HirExpr::PromiseAny(promises, first),
                    _ => unreachable!(),
                })
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

    fn lower_array_sort_comparator(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        comparator: HirExpr,
        copy: bool,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_sort_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_name = format!("__thaw_sort_comparator_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_type = HirType::Function(
            vec![element_type.clone(), element_type.clone()],
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
        let working_source = if copy {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                vec![variable(&receiver_name), number(0.0), number(f64::INFINITY)],
            )
        } else {
            variable(&receiver_name)
        };
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
            vec![variable(&left_name), variable(&right_name)],
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
        let body = HirExpr::Block(vec![
            HirStmt::Let(array_name.clone(), array_type.clone(), working_source),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(variable(&array_name))),
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
            HirStmt::Return(Some(variable(&array_name))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (comparator_name, comparator_type, comparator),
            ],
        )
    }

    fn lower_array_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
        expected_return: &HirType,
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
        self.lower_promise_callback(expr, &available[..arity], Some(expected_return))
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
        self.lower_promise_callback(expr, &available[..arity], None)
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
        self.lower_promise_callback(expr, &available[..arity], None)
    }

}
