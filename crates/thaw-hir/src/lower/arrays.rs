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

    fn lower_array_map(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_map_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_map_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let HirType::Function(params, output_type) = &callback_type else {
            unreachable!("array mapper was validated as a function")
        };
        if **output_type == HirType::Void {
            return Err("array mapper must return a value".into());
        }
        let output_type = output_type.as_ref().clone();
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_map_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_map_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_map_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_map_element_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Array(Box::new(output_type.clone()));
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(HirExpr::Var(length_name.clone())), output_type),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(HirExpr::Var(result_name.clone())),
                        Box::new(HirExpr::Var(index_name.clone())),
                        Box::new(callback_call),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(result_name))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_map_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_filter(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_filter_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_filter_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_filter_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_filter_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_filter_index_{}", self.next_binding);
        self.next_binding += 1;
        let output_index_name = format!("__thaw_filter_output_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_filter_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(output_index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array filter was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let increment = |name: &str| {
            HirStmt::Expr(HirExpr::Assign(
                name.into(),
                Box::new(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(HirExpr::Var(name.into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
            ))
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(
                    Box::new(HirExpr::Var(length_name.clone())),
                    element_type.clone(),
                ),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::Let(
                output_index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name.clone(),
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type.clone(),
                        ),
                    ),
                    HirStmt::If(
                        callback_call,
                        vec![
                            HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(HirExpr::Var(result_name.clone())),
                                Box::new(HirExpr::Var(output_index_name.clone())),
                                Box::new(HirExpr::Var(element_name)),
                            )),
                            increment(&output_index_name),
                        ],
                        Vec::new(),
                    ),
                    increment(&index_name),
                ],
            ),
            HirStmt::Return(Some(HirExpr::ArraySetLen(
                Box::new(HirExpr::Var(result_name)),
                Box::new(HirExpr::Var(output_index_name)),
                element_type,
            ))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_filter_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_flat_one(
        &mut self,
        receiver: HirExpr,
        nested_array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_flat_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), nested_array_type.clone());
        let outer_length_name = format!("__thaw_flat_outer_length_{}", self.next_binding);
        self.next_binding += 1;
        let total_length_name = format!("__thaw_flat_total_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_index_name = format!("__thaw_flat_outer_index_{}", self.next_binding);
        self.next_binding += 1;
        let inner_array_name = format!("__thaw_flat_inner_array_{}", self.next_binding);
        self.next_binding += 1;
        let inner_length_name = format!("__thaw_flat_inner_length_{}", self.next_binding);
        self.next_binding += 1;
        let inner_index_name = format!("__thaw_flat_inner_index_{}", self.next_binding);
        self.next_binding += 1;
        let destination_name = format!("__thaw_flat_destination_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_flat_result_{}", self.next_binding);
        self.next_binding += 1;
        for name in [
            &outer_length_name,
            &total_length_name,
            &outer_index_name,
            &inner_length_name,
            &inner_index_name,
            &destination_name,
        ] {
            self.scope.insert(name.clone(), HirType::F64);
        }
        let inner_array_type = HirType::Array(Box::new(element_type.clone()));
        let result_type = inner_array_type.clone();
        self.scope
            .insert(inner_array_name.clone(), inner_array_type.clone());
        self.scope.insert(result_name.clone(), result_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let add = |left, right| HirExpr::BinOp(BinOp::Add, Box::new(left), Box::new(right));
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let increment = |name: &str| assign(name, add(var(name), number(1.0)));
        let load_inner = || {
            HirExpr::TypedIndex(
                Box::new(var(&receiver_name)),
                Box::new(var(&outer_index_name)),
                inner_array_type.clone(),
            )
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                outer_length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(total_length_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(outer_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::Let(
                        inner_array_name.clone(),
                        inner_array_type.clone(),
                        load_inner(),
                    ),
                    assign(
                        &total_length_name,
                        add(
                            var(&total_length_name),
                            HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                        ),
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(var(&total_length_name)), element_type.clone()),
            ),
            assign(&outer_index_name, number(0.0)),
            HirStmt::Let(destination_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::Let(
                        inner_array_name.clone(),
                        inner_array_type.clone(),
                        load_inner(),
                    ),
                    HirStmt::Let(
                        inner_length_name.clone(),
                        HirType::F64,
                        HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                    ),
                    HirStmt::Let(inner_index_name.clone(), HirType::F64, number(0.0)),
                    HirStmt::While(
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(var(&inner_index_name)),
                            Box::new(var(&inner_length_name)),
                        ),
                        vec![
                            HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(var(&result_name)),
                                Box::new(var(&destination_name)),
                                Box::new(HirExpr::TypedIndex(
                                    Box::new(var(&inner_array_name)),
                                    Box::new(var(&inner_index_name)),
                                    element_type.clone(),
                                )),
                            )),
                            increment(&inner_index_name),
                            increment(&destination_name),
                        ],
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(body, &[(receiver_name, nested_array_type, receiver)])
    }

    fn lower_array_with(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_with_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_with_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        let value_name = format!("__thaw_with_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        self.scope.insert(value_name.clone(), element_type.clone());
        let length_name = format!("__thaw_with_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_with_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        let copy_index_name = format!("__thaw_with_copy_index_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_with_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        self.scope.insert(copy_index_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let range_error = || {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                "Invalid index for Array.prototype.with".into(),
            )))
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            // ToIntegerOrInfinity maps NaN to +0; finite fractional indices
            // are truncated by the typed element-address conversion.
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(&actual_index_name, number(0.0))],
            ),
            assign(
                &actual_index_name,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                    vec![var(&actual_index_name)],
                ),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(
                    &actual_index_name,
                    HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(var(&length_name)),
                        Box::new(var(&actual_index_name)),
                    ),
                )],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&length_name)), element_type.clone()),
            ),
            HirStmt::Let(copy_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&copy_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(var(&copy_index_name)),
                            Box::new(var(&actual_index_name)),
                        ),
                        vec![HirStmt::Expr(HirExpr::IndexAssign(
                            Box::new(var(&result_name)),
                            Box::new(var(&copy_index_name)),
                            Box::new(var(&value_name)),
                        ))],
                        vec![HirStmt::Expr(HirExpr::IndexAssign(
                            Box::new(var(&result_name)),
                            Box::new(var(&copy_index_name)),
                            Box::new(HirExpr::TypedIndex(
                                Box::new(var(&receiver_name)),
                                Box::new(var(&copy_index_name)),
                                element_type.clone(),
                            )),
                        ))],
                    ),
                    assign(
                        &copy_index_name,
                        HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&copy_index_name)),
                            Box::new(number(1.0)),
                        ),
                    ),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
                (value_name, element_type, value),
            ],
        )
    }

    fn lower_array_at(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_at_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_at_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        let length_name = format!("__thaw_at_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_at_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |value| HirStmt::Expr(HirExpr::Assign(actual_index_name.clone(), Box::new(value)));
        let none = || HirStmt::Return(Some(HirExpr::OptionalNone(element_type.clone())));
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(number(0.0))],
            ),
            assign(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![var(&actual_index_name)],
            )),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(var(&length_name)),
                    Box::new(var(&actual_index_name)),
                ))],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::Return(Some(HirExpr::OptionalSome(
                Box::new(HirExpr::TypedIndex(
                    Box::new(var(&receiver_name)),
                    Box::new(var(&actual_index_name)),
                    element_type.clone(),
                )),
                element_type,
            ))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
            ],
        )
    }

    fn lower_array_to_spliced(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        arguments: Vec<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_to_spliced_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        let mut bindings = vec![(receiver_name.clone(), array_type.clone(), receiver)];
        let mut argument_names = Vec::with_capacity(arguments.len());
        for (index, argument) in arguments.into_iter().enumerate() {
            let ty = if index < 2 {
                HirType::F64
            } else {
                element_type.clone()
            };
            let name = format!("__thaw_to_spliced_argument_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            argument_names.push(name.clone());
            bindings.push((name, ty, argument));
        }
        let length_name = format!("__thaw_to_spliced_length_{}", self.next_binding);
        self.next_binding += 1;
        let start_name = format!("__thaw_to_spliced_start_{}", self.next_binding);
        self.next_binding += 1;
        let delete_name = format!("__thaw_to_spliced_delete_{}", self.next_binding);
        self.next_binding += 1;
        let result_length_name = format!("__thaw_to_spliced_result_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_to_spliced_result_{}", self.next_binding);
        self.next_binding += 1;
        let source_index_name = format!("__thaw_to_spliced_source_{}", self.next_binding);
        self.next_binding += 1;
        let destination_index_name = format!("__thaw_to_spliced_destination_{}", self.next_binding);
        self.next_binding += 1;
        for name in [
            &length_name,
            &start_name,
            &delete_name,
            &result_length_name,
            &source_index_name,
            &destination_index_name,
        ] {
            self.scope.insert(name.clone(), HirType::F64);
        }
        self.scope.insert(result_name.clone(), array_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let add = |left, right| HirExpr::BinOp(BinOp::Add, Box::new(left), Box::new(right));
        let sub = |left, right| HirExpr::BinOp(BinOp::Sub, Box::new(left), Box::new(right));
        let increment = |name: &str| assign(name, add(var(name), number(1.0)));
        let trunc = |value| {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![value],
            )
        };
        let initial_start = argument_names
            .first()
            .map(|name| var(name))
            .unwrap_or_else(|| number(0.0));
        let mut statements = vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(start_name.clone(), HirType::F64, initial_start),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&start_name)),
                    Box::new(var(&start_name)),
                ),
                Vec::new(),
                vec![assign(&start_name, number(0.0))],
            ),
            assign(&start_name, trunc(var(&start_name))),
            HirStmt::If(
                HirExpr::BinOp(BinOp::Lt, Box::new(var(&start_name)), Box::new(number(0.0))),
                vec![
                    assign(&start_name, add(var(&length_name), var(&start_name))),
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(var(&start_name)),
                            Box::new(number(0.0)),
                        ),
                        vec![assign(&start_name, number(0.0))],
                        Vec::new(),
                    ),
                ],
                vec![HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::Gt,
                        Box::new(var(&start_name)),
                        Box::new(var(&length_name)),
                    ),
                    vec![assign(&start_name, var(&length_name))],
                    Vec::new(),
                )],
            ),
        ];
        let initial_delete = match argument_names.len() {
            0 => number(0.0),
            1 => sub(var(&length_name), var(&start_name)),
            _ => var(&argument_names[1]),
        };
        statements.extend([
            HirStmt::Let(delete_name.clone(), HirType::F64, initial_delete),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&delete_name)),
                    Box::new(var(&delete_name)),
                ),
                Vec::new(),
                vec![assign(&delete_name, number(0.0))],
            ),
            assign(&delete_name, trunc(var(&delete_name))),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&delete_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(&delete_name, number(0.0))],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Gt,
                    Box::new(var(&delete_name)),
                    Box::new(sub(var(&length_name), var(&start_name))),
                ),
                vec![assign(
                    &delete_name,
                    sub(var(&length_name), var(&start_name)),
                )],
                Vec::new(),
            ),
            HirStmt::Let(
                result_length_name.clone(),
                HirType::F64,
                add(
                    sub(var(&length_name), var(&delete_name)),
                    number(argument_names.len().saturating_sub(2) as f64),
                ),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&result_length_name)), element_type.clone()),
            ),
            HirStmt::Let(source_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(destination_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&source_index_name)),
                    Box::new(var(&start_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&destination_index_name)),
                        Box::new(HirExpr::TypedIndex(
                            Box::new(var(&receiver_name)),
                            Box::new(var(&source_index_name)),
                            element_type.clone(),
                        )),
                    )),
                    increment(&source_index_name),
                    increment(&destination_index_name),
                ],
            ),
        ]);
        for item_name in argument_names.iter().skip(2) {
            statements.push(HirStmt::Expr(HirExpr::IndexAssign(
                Box::new(var(&result_name)),
                Box::new(var(&destination_index_name)),
                Box::new(var(item_name)),
            )));
            statements.push(increment(&destination_index_name));
        }
        statements.extend([
            assign(&source_index_name, add(var(&start_name), var(&delete_name))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&source_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&destination_index_name)),
                        Box::new(HirExpr::TypedIndex(
                            Box::new(var(&receiver_name)),
                            Box::new(var(&source_index_name)),
                            element_type,
                        )),
                    )),
                    increment(&source_index_name),
                    increment(&destination_index_name),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings)
    }

    fn lower_array_reduce(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        initial: Option<(HirExpr, HirType)>,
        reverse: bool,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_reduce_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_reduce_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let accumulator_type = initial
            .as_ref()
            .map(|(_, ty)| ty.clone())
            .unwrap_or_else(|| element_type.clone());
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_reduce_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_reduce_index_{}", self.next_binding);
        self.next_binding += 1;
        let accumulator_name = format!("__thaw_reduce_accumulator_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_reduce_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(accumulator_name.clone(), accumulator_type.clone());
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let one = || HirExpr::Lit(HirLit::F64(1.0));
        let length = || HirExpr::Var(length_name.clone());
        let index = || HirExpr::Var(index_name.clone());
        let receiver_var = || HirExpr::Var(receiver_name.clone());
        let (initial_value, initial_index, empty_guard, initial_name) = if initial.is_some() {
            let initial_name = format!("__thaw_reduce_initial_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(initial_name.clone(), accumulator_type.clone());
            (
                HirExpr::Var(initial_name.clone()),
                if reverse {
                    HirExpr::BinOp(BinOp::Sub, Box::new(length()), Box::new(one()))
                } else {
                    HirExpr::Lit(HirLit::F64(0.0))
                },
                None,
                Some(initial_name),
            )
        } else {
            (
                HirExpr::TypedIndex(
                    Box::new(receiver_var()),
                    Box::new(if reverse {
                        HirExpr::BinOp(BinOp::Sub, Box::new(length()), Box::new(one()))
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    }),
                    element_type.clone(),
                ),
                if reverse {
                    HirExpr::BinOp(
                        BinOp::Sub,
                        Box::new(length()),
                        Box::new(HirExpr::Lit(HirLit::F64(2.0))),
                    )
                } else {
                    one()
                },
                Some(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(length()),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                        "Reduce of empty array with no initial value".into(),
                    )))],
                    Vec::new(),
                )),
                None,
            )
        };
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array reducer was validated as a function")
        };
        let available = [
            HirExpr::Var(accumulator_name.clone()),
            HirExpr::Var(element_name.clone()),
            index(),
            receiver_var(),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let mut statements = vec![HirStmt::Let(
            length_name.clone(),
            HirType::F64,
            HirExpr::ArrayLen(Box::new(receiver_var())),
        )];
        if let Some(empty_guard) = empty_guard {
            statements.push(empty_guard);
        }
        statements.extend([
            HirStmt::Let(accumulator_name.clone(), accumulator_type, initial_value),
            HirStmt::Let(index_name.clone(), HirType::F64, initial_index),
            HirStmt::While(
                HirExpr::BinOp(
                    if reverse { BinOp::GtEq } else { BinOp::Lt },
                    Box::new(index()),
                    Box::new(if reverse {
                        HirExpr::Lit(HirLit::F64(0.0))
                    } else {
                        length()
                    }),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(receiver_var()),
                            Box::new(index()),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        accumulator_name.clone(),
                        Box::new(callback_call),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            if reverse { BinOp::Sub } else { BinOp::Add },
                            Box::new(index()),
                            Box::new(one()),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(accumulator_name))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some((initial, ty)) = initial {
            bindings.push((
                initial_name.expect("initial accumulator binding must be retained"),
                ty,
                initial,
            ));
        }
        self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings)
    }

    fn lower_array_predicate_method(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
        mode: ArrayPredicateMode,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_predicate_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_predicate_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_predicate_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_predicate_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_predicate_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array predicate was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let stop_condition = if matches!(
            mode,
            ArrayPredicateMode::Some
                | ArrayPredicateMode::Find
                | ArrayPredicateMode::FindIndex
                | ArrayPredicateMode::FindLast
                | ArrayPredicateMode::FindLastIndex
        ) {
            callback_call
        } else {
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(callback_call),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
            )
        };
        let stop_result = match mode {
            ArrayPredicateMode::Some => HirExpr::Lit(HirLit::Bool(true)),
            ArrayPredicateMode::Every => HirExpr::Lit(HirLit::Bool(false)),
            ArrayPredicateMode::Find | ArrayPredicateMode::FindLast => HirExpr::OptionalSome(
                Box::new(HirExpr::Var(element_name.clone())),
                element_type.clone(),
            ),
            ArrayPredicateMode::FindIndex | ArrayPredicateMode::FindLastIndex => {
                HirExpr::Var(index_name.clone())
            }
        };
        let final_result = match mode {
            ArrayPredicateMode::Some => HirExpr::Lit(HirLit::Bool(false)),
            ArrayPredicateMode::Every => HirExpr::Lit(HirLit::Bool(true)),
            ArrayPredicateMode::Find | ArrayPredicateMode::FindLast => {
                HirExpr::OptionalNone(element_type.clone())
            }
            ArrayPredicateMode::FindIndex | ArrayPredicateMode::FindLastIndex => {
                HirExpr::Lit(HirLit::F64(-1.0))
            }
        };
        let one = || HirExpr::Lit(HirLit::F64(1.0));
        let reverse = matches!(
            mode,
            ArrayPredicateMode::FindLast | ArrayPredicateMode::FindLastIndex
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                if reverse {
                    HirExpr::BinOp(
                        BinOp::Sub,
                        Box::new(HirExpr::Var(length_name.clone())),
                        Box::new(one()),
                    )
                } else {
                    HirExpr::Lit(HirLit::F64(0.0))
                },
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    if reverse { BinOp::GtEq } else { BinOp::Lt },
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(if reverse {
                        HirExpr::Lit(HirLit::F64(0.0))
                    } else {
                        HirExpr::Var(length_name)
                    }),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type.clone(),
                        ),
                    ),
                    HirStmt::If(
                        stop_condition,
                        vec![HirStmt::Return(Some(stop_result))],
                        Vec::new(),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            if reverse { BinOp::Sub } else { BinOp::Add },
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(one()),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(final_result)),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_predicate_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_for_each(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_for_each_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_for_each_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_for_each_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_for_each_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_for_each_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array callback was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(callback_call),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(None),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_for_each_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }
}
