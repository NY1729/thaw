impl<'a> FnLowerer<'a> {
    /// `Array.from({ length }, mapfn?)` -- the array-like-object overload,
    /// as opposed to the real-array/string overload `lower_array_map`
    /// handles. There is no underlying element storage to read (a plain
    /// `{ length }` object has no indexed properties in this compiler's
    /// fixed-layout object model), so every per-index value the spec would
    /// read from the source is always exactly `undefined`, matching
    /// `Array.from({length: 3})` producing `[undefined, undefined,
    /// undefined]` for real JavaScript too. `mapfn` may still ignore that
    /// value and use only the index, which is the overload's common use
    /// (`Array.from({length: n}, (_, i) => ...)` to build a range).
    fn lower_array_from_length(
        &mut self,
        length: HirExpr,
        callback: Option<HirExpr>,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let length_name = format!("__thaw_array_from_length_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        let mut bindings = vec![(length_name.clone(), HirType::F64, length)];
        let Some(callback) = callback else {
            let result = HirExpr::ArrayAlloc(
                Box::new(HirExpr::Var(length_name)),
                HirType::Undefined,
            );
            return self.wrap_call_argument_bindings(result, &bindings);
        };
        let callback_name = format!("__thaw_array_from_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let HirType::Function(params, output_type) = &callback_type else {
            unreachable!("Array.from mapper was validated as a function")
        };
        if **output_type == HirType::Void {
            return Err("Array.from mapper must return a value".into());
        }
        let output_type = output_type.as_ref().clone();
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let result_name = format!("__thaw_array_from_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_array_from_index_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Array(Box::new(output_type.clone()));
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        let available = [
            HirExpr::Lit(HirLit::Undefined),
            HirExpr::Var(index_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let body = HirExpr::Block(vec![
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
        bindings.push((callback_name, callback_type, callback));
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_array_from_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
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

    /// `Array.prototype.keys`: a real, eagerly-built `number[]` of indices
    /// `0..length` rather than the specification's lazy iterator, the same
    /// simplification `Map`/`Set`'s own `.keys()`/`.values()`/`.entries()`
    /// already make.
    fn lower_array_keys(&mut self, receiver: HirExpr, array_type: HirType) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_array_keys_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_array_keys_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_array_keys_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_array_keys_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        let result_type = HirType::Array(Box::new(HirType::F64));
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        let var = |name: &str| HirExpr::Var(name.into());
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&length_name)), HirType::F64),
            ),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&index_name)),
                        Box::new(var(&index_name)),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&body, &mut referenced);
        let captures = referenced
            .into_iter()
            .filter_map(|captured| {
                self.scope
                    .get(&captured)
                    .cloned()
                    .map(|ty| HirParam { name: captured, ty })
            })
            .collect();
        let result = HirExpr::Call(
            Box::new(HirExpr::Lambda(captures, Vec::new(), result_type, Box::new(body))),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(result, &[(receiver_name, array_type, receiver)])
    }

    /// `Array.prototype.entries`: a real, eagerly-built array of `[index,
    /// value]` pairs rather than the specification's lazy iterator (same
    /// simplification as `lower_array_keys`). Building each pair via
    /// `ArrayLit` (the same construction a literal `[i, element]` tuple
    /// expression itself would use) keeps this correct regardless of the
    /// element type's own storage width -- unlike a native implementation
    /// that assumed a fixed per-element byte stride, which would silently
    /// misread a wider element type such as `T | undefined`.
    fn lower_array_entries(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_array_entries_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_array_entries_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_array_entries_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_array_entries_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_array_entries_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        let pair_type = HirType::Tuple(vec![HirType::F64, element_type.clone()]);
        let result_type = HirType::Array(Box::new(pair_type.clone()));
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(element_name.clone(), element_type.clone());
        let var = |name: &str| HirExpr::Var(name.into());
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&length_name)), pair_type),
            ),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name.clone(),
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&receiver_name)),
                            Box::new(var(&index_name)),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&index_name)),
                        Box::new(HirExpr::ArrayLit(vec![var(&index_name), var(&element_name)])),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&body, &mut referenced);
        let captures = referenced
            .into_iter()
            .filter_map(|captured| {
                self.scope
                    .get(&captured)
                    .cloned()
                    .map(|ty| HirParam { name: captured, ty })
            })
            .collect();
        let result = HirExpr::Call(
            Box::new(HirExpr::Lambda(captures, Vec::new(), result_type, Box::new(body))),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(result, &[(receiver_name, array_type, receiver)])
    }

}
