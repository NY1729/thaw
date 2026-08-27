impl<'a> FnLowerer<'a> {
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
