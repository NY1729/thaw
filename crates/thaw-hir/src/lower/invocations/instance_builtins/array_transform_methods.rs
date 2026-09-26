impl<'a> FnLowerer<'a> {
    fn lower_native_array_transform_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"toReversed" {
                    if !call.args.is_empty() {
                        return Err("native `.toReversed()` expects no arguments".into());
                    }
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.toReversed()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_to_reversed".to_string())),
                        vec![receiver],
                    ));
                }
                if matches!(property.sym.as_ref(), "sort" | "toSorted") {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_sort_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), receiver_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if arguments.len() > 1 {
                            return Err(format!(
                                "native `.{}()` expects zero or one comparator",
                                property.sym
                            ));
                        }
                        let has_comparator = arguments.first()
                            .map(|argument| self.infer_expr_type(argument).map(|ty| ty != HirType::Undefined))
                            .transpose()?
                            .unwrap_or(false);
                        let result = if has_comparator {
                            let comparator = arguments.first().unwrap();
                            let available = [element_type.clone(), element_type.clone()];
                            let comparator = self.validate_array_callback_value(
                                comparator.clone(),
                                &available,
                                Some(&HirType::F64),
                                "array comparator",
                            )?;
                            self.lower_array_sort_comparator(
                                HirExpr::Var(source_name.clone()),
                                receiver_type.clone(),
                                element_type,
                                comparator,
                                property.sym == *"toSorted",
                            )?
                        } else {
                            self.lower_array_sort_default(
                                HirExpr::Var(source_name.clone()),
                                receiver_type.clone(),
                                element_type,
                                property.sym == *"toSorted",
                            )?
                        };
                        let mut bindings = vec![(source_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if call.args.len() > 1 {
                        return Err(format!(
                            "native `.{}()` expects zero or one comparator",
                            property.sym
                        ));
                    }
                    if let Some(argument) = call.args.first() {
                        if !matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                            let lowered = self.lower_expr(&argument.expr)?;
                            if self.infer_expr_type(&lowered)? == HirType::Undefined {
                                let source_name = format!("__thaw_sort_source_{}", self.next_binding);
                                self.next_binding += 1;
                                let argument_name = format!("__thaw_sort_argument_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(source_name.clone(), receiver_type.clone());
                                self.scope.insert(argument_name.clone(), HirType::Undefined);
                                let result = self.lower_array_sort_default(
                                    HirExpr::Var(source_name.clone()),
                                    receiver_type.clone(),
                                    element_type,
                                    property.sym == *"toSorted",
                                )?;
                                return self.wrap_call_argument_bindings(result, &[
                                    (source_name, receiver_type, receiver),
                                    (argument_name, HirType::Undefined, lowered),
                                ]);
                            }
                        }
                        let comparator = self.lower_promise_callback(
                            &argument.expr,
                            &[element_type.clone(), element_type.clone()],
                            Some(&HirType::F64),
                        )?;
                        return self.lower_array_sort_comparator(
                            receiver,
                            receiver_type,
                            element_type,
                            comparator,
                            property.sym == *"toSorted",
                        );
                    }
                    return self.lower_array_sort_default(
                        receiver,
                        receiver_type,
                        element_type,
                        property.sym == *"toSorted",
                    );
                }
                if matches!(
                    property.sym.as_ref(),
                    "some" | "every" | "find" | "findIndex" | "findLast" | "findLastIndex"
                ) {
                    let mode = match property.sym.as_ref() {
                        "some" => ArrayPredicateMode::Some,
                        "every" => ArrayPredicateMode::Every,
                        "find" => ArrayPredicateMode::Find,
                        "findIndex" => ArrayPredicateMode::FindIndex,
                        "findLast" => ArrayPredicateMode::FindLast,
                        "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                        _ => unreachable!(),
                    };
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Union(elements) = &array_type {
                        if elements.iter().all(|element| matches!(element, HirType::Array(_)))
                            && call.args.iter().all(|argument| argument.spread.is_none())
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err(format!(
                                    "native `.{}()` expects a predicate and optional thisArg",
                                    property.sym
                                ));
                            }
                            let mut callback_members = Vec::new();
                            for element in elements {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                let ty = if sparse_callback {
                                    Self::array_read_type(inner)
                                } else {
                                    inner.as_ref().clone()
                                };
                                Self::flatten_property_union_members(&ty, &mut callback_members)?;
                            }
                            let callback_element_type = match callback_members.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(callback_members),
                            };
                            let callback = self.lower_array_callback(
                                &call.args[0].expr,
                                &callback_element_type,
                                &array_type,
                                None,
                            )?;
                            let this_arg = call
                                .args
                                .get(1)
                                .map(|argument| self.lower_expr(&argument.expr))
                                .transpose()?;
                            let name = format!("__thaw_union_predicate_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut branches = Vec::with_capacity(elements.len());
                            let mut result_members = Vec::new();
                            for (index, element) in elements.iter().enumerate() {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                let result = self.lower_array_predicate_method(
                                    HirExpr::UnionValue(
                                        Box::new(bound.clone()),
                                        index,
                                        elements.clone(),
                                    ),
                                    element.clone(),
                                    inner.as_ref().clone(),
                                    callback.clone(),
                                    this_arg.clone(),
                                    mode,
                                    sparse_callback,
                                )?;
                                let result_type = self.infer_expr_type(&result)?;
                                Self::flatten_property_union_members(
                                    &result_type,
                                    &mut result_members,
                                )?;
                                branches.push((result, result_type));
                            }
                            let result_type = match result_members.as_slice() {
                                [] => return Err("cannot search an empty union".into()),
                                [single] => single.clone(),
                                _ => HirType::Union(result_members),
                            };
                            let mut result = None;
                            for (index, (branch, branch_type)) in
                                branches.into_iter().enumerate().rev()
                            {
                                let branch = if branch_type == result_type {
                                    branch
                                } else {
                                    self.coerce_to_declared(&result_type, branch)?
                                };
                                result = Some(match result {
                                    None => branch,
                                    Some(rest) => HirExpr::Conditional(
                                        Box::new(HirExpr::BinOp(
                                            BinOp::EqEqEq,
                                            Box::new(HirExpr::UnionTag(
                                                Box::new(bound.clone()),
                                                elements.clone(),
                                            )),
                                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                        )),
                                        Box::new(branch),
                                        Box::new(rest),
                                        result_type.clone(),
                                    ),
                                });
                            }
                            return self.wrap_call_argument_bindings(
                                result.ok_or("cannot search an empty union")?,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_predicate_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(format!(
                                "native `.{}()` expects a predicate and optional thisArg",
                                property.sym
                            ));
                        }
                        let available = [
                            callback_element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array predicate",
                        )?;
                        let result = self.lower_array_predicate_method(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                            mode,
                            sparse_callback,
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a predicate and optional thisArg",
                            property.sym
                        ));
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &callback_element_type,
                        &array_type,
                        None,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_predicate_method(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                        mode,
                        sparse_callback,
                    );
                }
                if matches!(property.sym.as_ref(), "reduce" | "reduceRight") {
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Union(elements) = &array_type {
                        if elements.iter().all(|element| matches!(element, HirType::Array(_)))
                            && call.args.iter().all(|argument| argument.spread.is_none())
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err(format!(
                                    "native `.{}()` expects a reducer and optional initial value",
                                    property.sym
                                ));
                            }
                            let initial = call.args.get(1).map(|argument| {
                                let value = self.lower_expr(&argument.expr)?;
                                let ty = self.infer_expr_type(&value)?;
                                Ok::<_, String>((value, ty))
                            }).transpose()?;
                            let mut member_types = Vec::new();
                            for element in elements {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let ty = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                Self::flatten_property_union_members(&ty, &mut member_types)?;
                            }
                            let element_type = match member_types.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(member_types),
                            };
                            let accumulator_type = initial.as_ref().map(|(_, ty)| ty).unwrap_or(&element_type);
                            let callback = self.lower_array_reducer_callback(
                                &call.args[0].expr, accumulator_type, &element_type, &array_type,
                            )?;
                            let name = format!("__thaw_union_reduce_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut result = None;
                            for (index, element) in elements.iter().enumerate().rev() {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let callback_element_type = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                let reduced = self.lower_array_reduce(
                                    HirExpr::UnionValue(Box::new(bound.clone()), index, elements.clone()),
                                    element.clone(), inner.as_ref().clone(), callback_element_type,
                                    callback.clone(), initial.clone(), property.sym == *"reduceRight",
                                )?;
                                let reduced = if self.infer_expr_type(&reduced)? == *accumulator_type {
                                    reduced
                                } else {
                                    self.coerce_to_declared(accumulator_type, reduced)?
                                };
                                result = Some(match result {
                                    None => reduced,
                                    Some(rest) => HirExpr::Conditional(
                                        Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                                            Box::new(HirExpr::UnionTag(Box::new(bound.clone()), elements.clone())),
                                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))))),
                                        Box::new(reduced), Box::new(rest), accumulator_type.clone(),
                                    ),
                                });
                            }
                            return self.wrap_call_argument_bindings(
                                result.ok_or("cannot reduce an empty union")?,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_reduce_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(format!(
                                "native `.{}()` expects a reducer and optional initial value",
                                property.sym
                            ));
                        }
                        let initial = arguments.get(1).cloned().map(|value| {
                            let ty = self.infer_expr_type(&value)?;
                            Ok::<_, String>((value, ty))
                        }).transpose()?;
                        let sparse_element_type = Self::array_read_type(&element_type);
                        let accumulator_type = initial.as_ref().map(|(_, ty)| ty).unwrap_or(
                            if sparse_callback { &sparse_element_type } else { &element_type }
                        );
                        let callback_element_type = if sparse_callback {
                            Self::array_read_type(&element_type)
                        } else {
                            element_type.clone()
                        };
                        let available = [
                            accumulator_type.clone(),
                            callback_element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(accumulator_type),
                            "array reducer",
                        )?;
                        let result = self.lower_array_reduce(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback_element_type,
                            callback,
                            initial,
                            property.sym == *"reduceRight",
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a reducer and optional initial value",
                            property.sym
                        ));
                    }
                    let initial = call
                        .args
                        .get(1)
                        .map(|argument| {
                            let value = self.lower_expr(&argument.expr)?;
                            let ty = self.infer_expr_type(&value)?;
                            Ok::<_, String>((value, ty))
                        })
                        .transpose()?;
                    let sparse_element_type = Self::array_read_type(&element_type);
                    let accumulator_type = initial.as_ref().map(|(_, ty)| ty).unwrap_or(
                        if sparse_callback { &sparse_element_type } else { &element_type }
                    );
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
                    let callback = self.lower_array_reducer_callback(
                        &call.args[0].expr,
                        accumulator_type,
                        &callback_element_type,
                        &array_type,
                    )?;
                    return self.lower_array_reduce(
                        receiver,
                        array_type,
                        element_type,
                        callback_element_type,
                        callback,
                        initial,
                        property.sym == *"reduceRight",
                    );
                }
                if property.sym == *"toSpliced" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.toSpliced()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let source_name = format!("__thaw_to_spliced_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_array_values(&call.args, "Array.toSpliced")?;
                    let arguments = arguments
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| {
                            if index < 2 {
                                self.coerce_primitive_to_number(value)
                            } else {
                                self.coerce_array_insert_value(value, &element_type)
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = self.lower_array_to_spliced(
                        HirExpr::Var(source_name.clone()),
                        array_type.clone(),
                        element_type,
                        arguments,
                    )?;
                    let mut bindings = vec![(source_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"at" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == HirType::Str {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "String.at")?;
                        let [index] = arguments.as_slice() else {
                            return Err("native `.at()` expects exactly one index".into());
                        };
                        let index = self.coerce_primitive_to_number(index.clone())?;
                        let receiver_name =
                            format!("__thaw_string_at_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let index_name = format!("__thaw_string_at_index_{}", self.next_binding);
                        self.next_binding += 1;
                        let length_name = format!("__thaw_string_at_length_{}", self.next_binding);
                        self.next_binding += 1;
                        let actual_index_name =
                            format!("__thaw_string_at_actual_index_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(index_name.clone(), HirType::F64);
                        self.scope.insert(length_name.clone(), HirType::F64);
                        self.scope.insert(actual_index_name.clone(), HirType::F64);
                        let number = |value| HirExpr::Lit(HirLit::F64(value));
                        let var = |name: &str| HirExpr::Var(name.into());
                        let assign = |value| {
                            HirStmt::Expr(HirExpr::Assign(actual_index_name.clone(), Box::new(value)))
                        };
                        let none = || HirStmt::Return(Some(HirExpr::OptionalNone(HirType::Str)));
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(
                                length_name.clone(),
                                HirType::F64,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_length".into())),
                                    vec![var(&receiver_name)],
                                ),
                            ),
                            HirStmt::Let(
                                actual_index_name.clone(),
                                HirType::F64,
                                var(&index_name),
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
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_at".into())),
                                    vec![var(&receiver_name), var(&actual_index_name)],
                                )),
                                HirType::Str,
                            ))),
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
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                HirType::Optional(Box::new(HirType::Str)),
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((index_name, HirType::F64, index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let array_type = receiver_type;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "array `.at()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let receiver_name = format!("__thaw_at_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.at")?;
                    let [index] = arguments.as_slice() else {
                        return Err("native array `.at()` expects exactly one index".into());
                    };
                    let index = self.coerce_primitive_to_number(index.clone())?;
                    let result = self.lower_array_at(
                        HirExpr::Var(receiver_name.clone()),
                        array_type.clone(),
                        element_type,
                        index,
                    )?;
                    let mut bindings = vec![(receiver_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"with" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.with()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let receiver_name = format!("__thaw_with_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_array_values(&call.args, "Array.with")?;
                    let [index, value] = arguments.as_slice() else {
                        return Err("native `.with()` expects an index and value".into());
                    };
                    let index = self.coerce_primitive_to_number(index.clone())?;
                    let value = self.coerce_array_insert_value(value.clone(), &element_type)?;
                    let result = self.lower_array_with(
                        HirExpr::Var(receiver_name.clone()),
                        array_type.clone(),
                        element_type,
                        index,
                        value,
                    )?;
                    let mut bindings = vec![(receiver_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"flat" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let mut current_type = self.infer_expr_type(&receiver)?;
                    if !matches!(current_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.flat()` requires a homogeneous array, got {current_type:?}"
                        ));
                    }
                    let source_type = current_type.clone();
                    let source_name = format!("__thaw_flat_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), current_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.flat")?;
                    if arguments.len() > 1 {
                        return Err("native `.flat()` expects zero or one depth".into());
                    }
                    let depth = if let Some(argument) = arguments.first() {
                        let value = argument.clone();
                        self.expect_type(&HirType::F64, &value, "array flat depth")?;
                        let constant = match value {
                            HirExpr::Lit(HirLit::F64(value)) => Some(value),
                            HirExpr::BinOp(BinOp::Sub, left, right) => match (*left, *right) {
                                (HirExpr::Lit(HirLit::F64(0.0)), HirExpr::Lit(HirLit::F64(value))) => {
                                    Some(-value)
                                }
                                _ => None,
                            },
                            HirExpr::Call(callee, arguments)
                                if matches!(
                                    callee.as_ref(),
                                    HirExpr::Var(name) if name == "__thaw_number_neg"
                                ) => match arguments.as_slice() {
                                    [HirExpr::Lit(HirLit::F64(value))] => Some(-value),
                                    _ => None,
                                },
                            _ => None,
                        }
                        .ok_or(
                            "native `.flat()` depth must be a numeric literal so its result layout is static",
                        )?;
                        if constant.is_nan() || constant <= 0.0 {
                            0usize
                        } else if constant.is_infinite() {
                            usize::MAX
                        } else {
                            constant.trunc() as usize
                        }
                    } else {
                        1
                    };
                    let HirType::Array(receiver_element) = &current_type else {
                        unreachable!()
                    };
                    if receiver_element.as_ref() == &HirType::Json {
                        // Whether a `Json`-typed element is itself an
                        // array is only knowable at runtime -- the loop
                        // below unwraps a *statically* nested `T[][]`
                        // shape one level at a time, which can't express
                        // this, so it silently never flattens (`element`
                        // is `Json`, never `Array(_)`, so it `break`s
                        // immediately regardless of `depth`). Delegate to
                        // a native helper that walks the boxed JSON tree
                        // instead; see `thaw_any_array_flat`'s doc
                        // comment (thaw-std's `json.rs`) for why only
                        // this element type needs it.
                        let depth_value = if depth == usize::MAX {
                            f64::INFINITY
                        } else {
                            depth as f64
                        };
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_any_array_flat".to_string())),
                            vec![
                                HirExpr::Var(source_name.clone()),
                                HirExpr::Lit(HirLit::F64(depth_value)),
                            ],
                        );
                        let mut bindings = vec![(source_name, source_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let mut result = HirExpr::Var(source_name.clone());
                    let mut flattened = false;
                    for _ in 0..depth {
                        let HirType::Array(element) = &current_type else {
                            unreachable!()
                        };
                        let HirType::Array(inner) = element.as_ref() else {
                            break;
                        };
                        let inner = inner.as_ref().clone();
                        result = self.lower_array_flat_one(result, current_type, inner.clone())?;
                        current_type = HirType::Array(Box::new(inner));
                        flattened = true;
                    }
                    if !flattened {
                        let HirType::Array(element) = &current_type else {
                            unreachable!()
                        };
                        result = self.lower_array_filter(
                            result,
                            current_type.clone(),
                            element.as_ref().clone(),
                            element.as_ref().clone(),
                            HirExpr::Lambda(
                                Vec::new(),
                                Vec::new(),
                                HirType::Bool,
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                            ),
                            None,
                        )?;
                    }
                    let mut bindings = vec![(source_name, source_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"flatMap" {
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Union(elements) = &array_type {
                        if elements.iter().all(|element| matches!(element, HirType::Array(_)))
                            && call.args.iter().all(|argument| argument.spread.is_none())
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err("native `.flatMap()` expects a callback and optional thisArg".into());
                            }
                            let mut member_types = Vec::new();
                            for element in elements {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let ty = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                Self::flatten_property_union_members(&ty, &mut member_types)?;
                            }
                            let callback_element_type = match member_types.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(member_types),
                            };
                            let callback = self.lower_array_mapping_callback(
                                &call.args[0].expr, &callback_element_type, &array_type,
                            )?;
                            let this_arg = call.args.get(1).map(|argument| self.lower_expr(&argument.expr)).transpose()?;
                            let name = format!("__thaw_union_flat_map_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut branches = Vec::new();
                            let mut result_members = Vec::new();
                            for (index, element) in elements.iter().enumerate() {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let member_callback_type = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                let mapped = self.lower_array_map(
                                    HirExpr::UnionValue(Box::new(bound.clone()), index, elements.clone()),
                                    element.clone(), inner.as_ref().clone(), member_callback_type,
                                    callback.clone(), this_arg.clone(),
                                )?;
                                let branch = self.lower_array_flat_map_result(mapped)?;
                                let ty = self.infer_expr_type(&branch)?;
                                Self::flatten_property_union_members(&ty, &mut result_members)?;
                                branches.push((branch, ty));
                            }
                            let result_type = match result_members.as_slice() {
                                [single] => single.clone(),
                                [] => return Err("cannot flatMap an empty union".into()),
                                _ => HirType::Union(result_members),
                            };
                            let mut result = None;
                            for (index, (branch, ty)) in branches.into_iter().enumerate().rev() {
                                let branch = if ty == result_type { branch } else { self.coerce_to_declared(&result_type, branch)? };
                                result = Some(match result {
                                    None => branch,
                                    Some(rest) => HirExpr::Conditional(
                                        Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                                            Box::new(HirExpr::UnionTag(Box::new(bound.clone()), elements.clone())),
                                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))))),
                                        Box::new(branch), Box::new(rest), result_type.clone(),
                                    ),
                                });
                            }
                            return self.wrap_call_argument_bindings(
                                result.ok_or("cannot flatMap an empty union")?,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.flatMap()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_flat_map_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.flatMap")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.flatMap()` expects a callback and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            callback_element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array mapper",
                        )?;
                        let mapped = self.lower_array_map(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback_element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let result = self.lower_array_flat_map_result(mapped)?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.flatMap()` expects a callback and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &callback_element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    let mapped = self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback_element_type,
                        callback,
                        this_arg,
                    )?;
                    return self.lower_array_flat_map_result(mapped);
                }
                if property.sym == *"map" {
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Union(elements) = &array_type {
                        if elements.iter().all(|element| matches!(element, HirType::Array(_)))
                            && call.args.iter().all(|argument| argument.spread.is_none())
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err(
                                    "native `.map()` expects a callback and optional thisArg".into()
                                );
                            }
                            let mut callback_members = Vec::new();
                            for element in elements {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                let ty = if sparse_callback {
                                    Self::array_read_type(inner)
                                } else {
                                    inner.as_ref().clone()
                                };
                                Self::flatten_property_union_members(&ty, &mut callback_members)?;
                            }
                            let callback_element_type = match callback_members.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(callback_members),
                            };
                            let callback = self.lower_array_mapping_callback(
                                &call.args[0].expr,
                                &callback_element_type,
                                &array_type,
                            )?;
                            let this_arg = call
                                .args
                                .get(1)
                                .map(|argument| self.lower_expr(&argument.expr))
                                .transpose()?;
                            let name = format!("__thaw_union_map_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut result: Option<(HirExpr, HirType)> = None;
                            for (index, element) in elements.iter().enumerate().rev() {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                let member_callback_type = if sparse_callback {
                                    Self::array_read_type(inner)
                                } else {
                                    inner.as_ref().clone()
                                };
                                let mapped = self.lower_array_map(
                                    HirExpr::UnionValue(
                                        Box::new(bound.clone()),
                                        index,
                                        elements.clone(),
                                    ),
                                    element.clone(),
                                    inner.as_ref().clone(),
                                    member_callback_type,
                                    callback.clone(),
                                    this_arg.clone(),
                                )?;
                                let mapped_type = self.infer_expr_type(&mapped)?;
                                result = Some(match result {
                                    None => (mapped, mapped_type),
                                    Some((rest, result_type)) => (
                                        HirExpr::Conditional(
                                            Box::new(HirExpr::BinOp(
                                                BinOp::EqEqEq,
                                                Box::new(HirExpr::UnionTag(
                                                    Box::new(bound.clone()),
                                                    elements.clone(),
                                                )),
                                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                            )),
                                            Box::new(mapped),
                                            Box::new(rest),
                                            result_type.clone(),
                                        ),
                                        result_type,
                                    ),
                                });
                            }
                            let (result, _) = result.ok_or("cannot map an empty union")?;
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.map()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_map_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.map")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.map()` expects a callback and optional thisArg".into(),
                            );
                        }
                        let available = [
                            callback_element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array mapper",
                        )?;
                        let this_arg = arguments.get(1).cloned();
                        let result = self.lower_array_map(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback_element_type,
                            callback,
                            this_arg,
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.map()` expects a callback and optional thisArg".into()
                        );
                    }
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &callback_element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback_element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"filter" {
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Union(elements) = &array_type {
                        if elements.iter().all(|element| matches!(element, HirType::Array(_)))
                            && call.args.iter().all(|argument| argument.spread.is_none())
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err("native `.filter()` expects a predicate and optional thisArg".into());
                            }
                            let mut member_types = Vec::new();
                            for element in elements {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let ty = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                Self::flatten_property_union_members(&ty, &mut member_types)?;
                            }
                            let callback_element_type = match member_types.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(member_types),
                            };
                            let callback = self.lower_array_callback(
                                &call.args[0].expr, &callback_element_type, &array_type, None,
                            )?;
                            let this_arg = call.args.get(1).map(|argument| self.lower_expr(&argument.expr)).transpose()?;
                            let name = format!("__thaw_union_filter_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut result = None;
                            for (index, element) in elements.iter().enumerate().rev() {
                                let HirType::Array(inner) = element else { unreachable!("checked above") };
                                let member_callback_type = if sparse_callback { Self::array_read_type(inner) } else { inner.as_ref().clone() };
                                let filtered = self.lower_array_filter(
                                    HirExpr::UnionValue(Box::new(bound.clone()), index, elements.clone()),
                                    element.clone(), inner.as_ref().clone(), member_callback_type,
                                    callback.clone(), this_arg.clone(),
                                )?;
                                let filtered = self.coerce_to_declared(&array_type, filtered)?;
                                result = Some(match result {
                                    None => filtered,
                                    Some(rest) => HirExpr::Conditional(
                                        Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                                            Box::new(HirExpr::UnionTag(Box::new(bound.clone()), elements.clone())),
                                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))))),
                                        Box::new(filtered), Box::new(rest), array_type.clone(),
                                    ),
                                });
                            }
                            return self.wrap_call_argument_bindings(
                                result.ok_or("cannot filter an empty union")?,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.filter()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_filter_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.filter")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.filter()` expects a predicate and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            callback_element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array predicate",
                        )?;
                        let result = self.lower_array_filter(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback_element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.filter()` expects a predicate and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &callback_element_type,
                        &array_type,
                        None,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_filter(
                        receiver,
                        array_type,
                        element_type,
                        callback_element_type,
                        callback,
                        this_arg,
                    );
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}
