impl<'a> FnLowerer<'a> {
    fn lower_native_array_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"toReversed" {
                    if !call.args.is_empty() {
                        return Err("native `.toReversed()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
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
                    let receiver = self.lower_expr(&member.obj)?;
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
                        let result = if let Some(comparator) = arguments.first() {
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
                            let prefix = match &element_type {
                                HirType::F64 => "number",
                                HirType::Str => "string",
                                HirType::Bool => "bool",
                                HirType::Object(_) => "object",
                                other => {
                                    return Err(format!(
                                        "default array sort does not support element type {other:?}"
                                    ))
                                }
                            };
                            let suffix = if property.sym == *"sort" {
                                "sort"
                            } else {
                                "to_sorted"
                            };
                            HirExpr::Call(
                                Box::new(HirExpr::Var(format!(
                                    "__thaw_{prefix}_array_{suffix}"
                                ))),
                                vec![HirExpr::Var(source_name.clone())],
                            )
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
                    let prefix = match &element_type {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "default array sort does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = if property.sym == *"sort" {
                        "sort"
                    } else {
                        "to_sorted"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(
                    property.sym.as_ref(),
                    "some" | "every" | "find" | "findIndex" | "findLast" | "findLastIndex"
                ) {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
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
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Bool),
                            "array predicate",
                        )?;
                        let result = self.lower_array_predicate_method(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                            match property.sym.as_ref() {
                                "some" => ArrayPredicateMode::Some,
                                "every" => ArrayPredicateMode::Every,
                                "find" => ArrayPredicateMode::Find,
                                "findIndex" => ArrayPredicateMode::FindIndex,
                                "findLast" => ArrayPredicateMode::FindLast,
                                "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                                _ => unreachable!(),
                            },
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
                        &element_type,
                        &array_type,
                        &HirType::Bool,
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
                        match property.sym.as_ref() {
                            "some" => ArrayPredicateMode::Some,
                            "every" => ArrayPredicateMode::Every,
                            "find" => ArrayPredicateMode::Find,
                            "findIndex" => ArrayPredicateMode::FindIndex,
                            "findLast" => ArrayPredicateMode::FindLast,
                            "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                            _ => unreachable!(),
                        },
                    );
                }
                if matches!(property.sym.as_ref(), "reduce" | "reduceRight") {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
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
                        let accumulator_type = initial
                            .as_ref()
                            .map(|(_, ty)| ty)
                            .unwrap_or(&element_type);
                        let available = [
                            accumulator_type.clone(),
                            element_type.clone(),
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
                    let accumulator_type =
                        initial.as_ref().map(|(_, ty)| ty).unwrap_or(&element_type);
                    let callback = self.lower_array_reducer_callback(
                        &call.args[0].expr,
                        accumulator_type,
                        &element_type,
                        &array_type,
                    )?;
                    return self.lower_array_reduce(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        initial,
                        property.sym == *"reduceRight",
                    );
                }
                if property.sym == *"toSpliced" {
                    let receiver = self.lower_expr(&member.obj)?;
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
                        self.lower_native_spread_values(&call.args, "Array.toSpliced")?;
                    for (index, value) in arguments.iter().enumerate() {
                        let expected = if index < 2 {
                            &HirType::F64
                        } else {
                            &element_type
                        };
                        self.expect_type(expected, value, "array toSpliced argument")?;
                    }
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
                    let receiver = self.lower_expr(&member.obj)?;
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
                    let receiver = self.lower_expr(&member.obj)?;
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
                        self.lower_native_spread_values(&call.args, "Array.with")?;
                    let [index, value] = arguments.as_slice() else {
                        return Err("native `.with()` expects an index and value".into());
                    };
                    let index = index.clone();
                    self.expect_type(&HirType::F64, &index, "array with index")?;
                    let value = value.clone();
                    self.expect_type(&element_type, &value, "array with value")?;
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
                    let receiver = self.lower_expr(&member.obj)?;
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
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_slice".into())),
                            vec![
                                result,
                                HirExpr::Lit(HirLit::F64(0.0)),
                                HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                            ],
                        );
                    }
                    let mut bindings = vec![(source_name, source_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"flatMap" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.flatMap()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
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
                            element_type.clone(),
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
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mapped_type = self.infer_expr_type(&mapped)?;
                        let HirType::Array(mapped_element) = &mapped_type else {
                            unreachable!("array map always returns an array")
                        };
                        let HirType::Array(flat_element) = mapped_element.as_ref() else {
                            return Err(format!(
                                "native `.flatMap()` callback must return a homogeneous array, got {mapped_element:?}"
                            ));
                        };
                        let flat_element = flat_element.as_ref().clone();
                        let result = self.lower_array_flat_one(
                            mapped,
                            mapped_type,
                            flat_element,
                        )?;
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
                        &element_type,
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
                        callback,
                        this_arg,
                    )?;
                    let mapped_type = self.infer_expr_type(&mapped)?;
                    let HirType::Array(mapped_element) = &mapped_type else {
                        unreachable!("array map always returns an array")
                    };
                    let HirType::Array(flat_element) = mapped_element.as_ref() else {
                        return Err(format!(
                            "native `.flatMap()` callback must return a homogeneous array, got {mapped_element:?}"
                        ));
                    };
                    let flat_element = flat_element.as_ref().clone();
                    return self.lower_array_flat_one(mapped, mapped_type, flat_element);
                }
                if property.sym == *"map" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.map()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
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
                            element_type.clone(),
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
                        &element_type,
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
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"filter" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.filter()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
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
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Bool),
                            "array predicate",
                        )?;
                        let result = self.lower_array_filter(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
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
                        &element_type,
                        &array_type,
                        &HirType::Bool,
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
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"forEach" && self.receiver_is_map_or_set(&member.obj) {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (key_type, value_type, keys_double_as_values) = match &receiver_type {
                        HirType::Map(key_type, value_type) => {
                            (key_type.as_ref().clone(), value_type.as_ref().clone(), false)
                        }
                        HirType::Set(element_type) => {
                            let element_type = element_type.as_ref().clone();
                            (element_type.clone(), element_type, true)
                        }
                        _ => unreachable!("receiver_is_map_or_set confirmed this above"),
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err(
                            "native `.forEach()` does not support spread arguments on a Map/Set"
                                .into(),
                        );
                    }
                    let [argument] = call.args.as_slice() else {
                        return Err("native `.forEach()` expects exactly one argument".into());
                    };
                    let arity = match argument.expr.as_ref() {
                        Expr::Arrow(arrow) => arrow.params.len(),
                        Expr::Fn(function) => function.function.params.len(),
                        Expr::Ident(ident) => {
                            let name = self.resolve_binding(ident.sym.as_ref());
                            let HirType::Function(params, _) = self
                                .scope
                                .get(&name)
                                .ok_or_else(|| format!("unknown Map/Set forEach callback `{name}`"))?
                            else {
                                return Err(format!(
                                    "Map/Set forEach callback `{name}` is not a function value"
                                ));
                            };
                            params.len()
                        }
                        _ => {
                            return Err(
                                "Map/Set forEach callback must be an arrow or function value"
                                    .into(),
                            )
                        }
                    };
                    if arity > 3 {
                        return Err(format!(
                            "Map/Set forEach callback accepts at most three parameters, got {arity}"
                        ));
                    }
                    let available = [value_type.clone(), key_type.clone(), receiver_type.clone()];
                    let callback = self.lower_promise_callback(
                        &argument.expr,
                        &available[..arity],
                        Some(&HirType::Void),
                    )?;
                    let callback_type = self.infer_expr_type(&callback)?;
                    let HirType::Function(params, _) = &callback_type else {
                        unreachable!("Map/Set forEach callback was validated as a function")
                    };
                    let receiver_name = format!("__thaw_map_for_each_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let callback_name = format!("__thaw_map_for_each_callback_{}", self.next_binding);
                    self.next_binding += 1;
                    let keys_name = format!("__thaw_map_for_each_keys_{}", self.next_binding);
                    self.next_binding += 1;
                    let values_name = format!("__thaw_map_for_each_values_{}", self.next_binding);
                    self.next_binding += 1;
                    let length_name = format!("__thaw_map_for_each_length_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_map_for_each_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_var_name = format!("__thaw_map_for_each_key_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_var_name = format!("__thaw_map_for_each_value_{}", self.next_binding);
                    self.next_binding += 1;
                    let keys_array_type = HirType::Array(Box::new(key_type.clone()));
                    let values_array_type = HirType::Array(Box::new(value_type.clone()));
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(callback_name.clone(), callback_type.clone());
                    self.scope.insert(keys_name.clone(), keys_array_type.clone());
                    self.scope.insert(values_name.clone(), values_array_type.clone());
                    self.scope.insert(length_name.clone(), HirType::F64);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(key_var_name.clone(), key_type.clone());
                    self.scope.insert(value_var_name.clone(), value_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let available_vars = [
                        var(&value_var_name),
                        var(&key_var_name),
                        var(&receiver_name),
                    ];
                    let callback_call = HirExpr::Call(
                        Box::new(var(&callback_name)),
                        available_vars[..params.len()].to_vec(),
                    );
                    let values_source_name = if keys_double_as_values {
                        keys_name.clone()
                    } else {
                        values_name.clone()
                    };
                    let mut body_stmts = vec![HirStmt::Let(
                        keys_name.clone(),
                        keys_array_type,
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                            vec![var(&receiver_name)],
                        ),
                    )];
                    if !keys_double_as_values {
                        body_stmts.push(HirStmt::Let(
                            values_name,
                            values_array_type,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_map_snapshot_values".to_string())),
                                vec![var(&receiver_name)],
                            ),
                        ));
                    }
                    body_stmts.extend([
                        HirStmt::Let(
                            length_name.clone(),
                            HirType::F64,
                            HirExpr::ArrayLen(Box::new(var(&keys_name))),
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
                                    key_var_name,
                                    key_type.clone(),
                                    HirExpr::TypedIndex(
                                        Box::new(var(&keys_name)),
                                        Box::new(var(&index_name)),
                                        key_type,
                                    ),
                                ),
                                HirStmt::Let(
                                    value_var_name,
                                    value_type.clone(),
                                    HirExpr::TypedIndex(
                                        Box::new(var(&values_source_name)),
                                        Box::new(var(&index_name)),
                                        value_type,
                                    ),
                                ),
                                HirStmt::Expr(callback_call),
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
                        HirStmt::Return(None),
                    ]);
                    let body = HirExpr::Block(body_stmts);
                    let bindings = vec![
                        (receiver_name, receiver_type, receiver),
                        (callback_name, callback_type, callback),
                    ];
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if property.sym == *"forEach" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.forEach()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_for_each_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.forEach")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.forEach()` expects a callback and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Void),
                            "array callback",
                        )?;
                        let result = self.lower_array_for_each(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.forEach()` expects a callback and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Void,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_for_each(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"slice" {
                    let mut receiver = self.lower_expr(&member.obj)?;
                    let mut receiver_type = self.infer_expr_type(&receiver)?;
                    if matches!(receiver_type, HirType::Json | HirType::JsValue) {
                        receiver = self.coerce_primitive_to_string(receiver)?;
                        receiver_type = HirType::Str;
                    }
                    if receiver_type == HirType::Str {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "String.slice")?;
                        if arguments.len() > 2 {
                            return Err("native string `.slice()` expects zero to two arguments".into());
                        }
                        let start = arguments
                            .first()
                            .cloned()
                            .map(|value| self.coerce_primitive_to_number(value))
                            .transpose()?
                            .unwrap_or(HirExpr::Lit(HirLit::F64(0.0)));
                        let end = arguments
                            .get(1)
                            .cloned()
                            .map(|value| self.coerce_primitive_to_number(value))
                            .transpose()?
                            .unwrap_or(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_slice".into())),
                            vec![receiver, start, end],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.slice()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.slice")?;
                    if arguments.len() > 2 {
                        return Err("native `.slice()` expects zero to two arguments".into());
                    }
                    let mut indices = Vec::with_capacity(2);
                    for value in arguments {
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_slice_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_slice_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_slice".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"copyWithin" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.copyWithin()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.copyWithin")?;
                    if !(2..=3).contains(&arguments.len()) {
                        return Err("native `.copyWithin()` expects two or three arguments".into());
                    }
                    let mut indices = Vec::with_capacity(3);
                    for value in arguments {
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.len() == 2 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_copy_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_copy_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_copy_within".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"fill" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.fill()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    };
                    let element = element.as_ref().clone();
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.fill")?;
                    if !(1..=3).contains(&arguments.len()) {
                        return Err("native `.fill()` expects one to three arguments".into());
                    }
                    let value = arguments[0].clone();
                    self.expect_type(&element, &value, "fill value")?;
                    let mut indices = Vec::with_capacity(2);
                    for argument in arguments.into_iter().skip(1) {
                        indices.push(self.coerce_primitive_to_number(argument)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let runtime = match &element {
                        HirType::F64 => "__thaw_number_array_fill",
                        HirType::Bool => "__thaw_bool_array_fill",
                        HirType::Str | HirType::Array(_) | HirType::Object(_) => {
                            "__thaw_pointer_array_fill"
                        }
                        _ => "__thaw_array_fill",
                    };
                    let receiver_name = format!("__thaw_fill_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let value_name = format!("__thaw_fill_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), element.clone());
                    let mut bindings = vec![
                        (receiver_name.clone(), receiver_type, receiver),
                    ];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name.clone(), element, value));
                    let mut arguments = vec![HirExpr::Var(receiver_name), HirExpr::Var(value_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_fill_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), arguments);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"reverse" {
                    if !call.args.is_empty() {
                        return Err("native `.reverse()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.reverse()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_reverse".to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"pop" || property.sym == *"shift" {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    let runtime = if property.sym == *"pop" {
                        "__thaw_array_pop"
                    } else {
                        "__thaw_array_shift"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(runtime.to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"push" || property.sym == *"unshift" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element = element.as_ref().clone();
                    let label = if property.sym == *"push" { "push" } else { "unshift" };
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, &format!("Array.{label}"))?;
                    for value in &arguments {
                        self.expect_type(&element, value, "array push/unshift value")?;
                    }
                    let receiver_name = format!("__thaw_{label}_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut call_arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, value) in arguments.into_iter().enumerate() {
                        let name = format!("__thaw_{label}_value_{position}_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), element.clone());
                        call_arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, element.clone(), value));
                    }
                    let runtime = if property.sym == *"push" {
                        "__thaw_array_push"
                    } else {
                        "__thaw_array_unshift"
                    };
                    let result =
                        HirExpr::Call(Box::new(HirExpr::Var(runtime.to_string())), call_arguments);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"splice" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.splice()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    };
                    let element = element.as_ref().clone();
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.splice")?;
                    let mut arguments = arguments.into_iter();
                    let start = match arguments.next() {
                        Some(value) => self.coerce_primitive_to_number(value)?,
                        None => HirExpr::Lit(HirLit::F64(0.0)),
                    };
                    let delete_count = match arguments.next() {
                        Some(value) => self.coerce_primitive_to_number(value)?,
                        None => HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                    };
                    let items: Vec<HirExpr> = arguments.collect();
                    for item in &items {
                        self.expect_type(&element, item, "splice item")?;
                    }
                    let receiver_name = format!("__thaw_splice_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let start_name = format!("__thaw_splice_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let delete_name = format!("__thaw_splice_delete_count_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(delete_name.clone(), HirType::F64);
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((start_name.clone(), HirType::F64, start));
                    bindings.push((delete_name.clone(), HirType::F64, delete_count));
                    let mut call_arguments = vec![
                        HirExpr::Var(receiver_name),
                        HirExpr::Var(start_name),
                        HirExpr::Var(delete_name),
                    ];
                    for (position, item) in items.into_iter().enumerate() {
                        let name = format!("__thaw_splice_item_{position}_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), element.clone());
                        call_arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, element.clone(), item));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_splice".to_string())),
                        call_arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"join" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let source_name = format!("__thaw_join_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), receiver_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.join")?;
                    if arguments.len() > 1 {
                        return Err("native `.join()` expects zero or one argument".into());
                    }
                    let separator = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_string(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::Str(",".to_string()))
                    };
                    let result = match receiver_type.clone() {
                        HirType::Array(element) => {
                            let array_type = HirType::Array(element.clone());
                            let builtin = match element.as_ref() {
                                HirType::F64 => "__thaw_number_array_join",
                                HirType::Str => "__thaw_string_array_join",
                                HirType::Bool => "__thaw_bool_array_join",
                                HirType::Object(_) => "__thaw_object_array_join",
                                other => {
                                    return Err(format!(
                                        "array join does not support element type {other:?}"
                                    ))
                                }
                            };
                            let receiver_name =
                                format!("__thaw_join_receiver_{}", self.next_binding);
                            self.next_binding += 1;
                            let separator_name =
                                format!("__thaw_join_separator_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(receiver_name.clone(), array_type.clone());
                            self.scope.insert(separator_name.clone(), HirType::Str);
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(builtin.to_string())),
                                vec![
                                    HirExpr::Var(receiver_name.clone()),
                                    HirExpr::Var(separator_name.clone()),
                                ],
                            );
                            self.wrap_call_argument_bindings(
                                result,
                                &[
                                    (
                                        receiver_name,
                                        array_type,
                                        HirExpr::Var(source_name.clone()),
                                    ),
                                    (separator_name, HirType::Str, separator),
                                ],
                            )
                        }
                        HirType::Tuple(elements) => self.join_tuple(
                            HirExpr::Var(source_name.clone()),
                            elements,
                            separator,
                        ),
                        other => Err(format!(
                            "`.join()` requires an array receiver, got {other:?}"
                        )),
                    }?;
                    let mut bindings = vec![(source_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(
                    property.sym.as_ref(),
                    "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith"
                ) {
                    let label = format!("native .{}", property.sym);
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, &label)?;
                    if !(1..=2).contains(&arguments.len()) {
                        return Err(format!(
                            "native `.{}` expects one or two arguments",
                            property.sym
                        ));
                    }
                    let mut receiver = self.lower_expr(&member.obj)?;
                    let mut receiver_type = self.infer_expr_type(&receiver)?;
                    if matches!(property.sym.as_ref(), "startsWith" | "endsWith")
                        && matches!(receiver_type, HirType::Json | HirType::JsValue)
                    {
                        receiver = self.coerce_primitive_to_string(receiver)?;
                        receiver_type = HirType::Str;
                    }
                    if receiver_type == HirType::Str {
                        let needle = self.coerce_primitive_to_string(arguments[0].clone())?;
                        let position = if let Some(argument) = arguments.get(1) {
                            self.coerce_primitive_to_number(argument.clone())?
                        } else if property.sym == *"endsWith" || property.sym == *"lastIndexOf" {
                            HirExpr::Lit(HirLit::F64(f64::INFINITY))
                        } else {
                            HirExpr::Lit(HirLit::F64(0.0))
                        };
                        let suffix = match property.sym.as_ref() {
                            "indexOf" => "index_of",
                            "lastIndexOf" => "last_index_of",
                            "includes" => "includes",
                            "startsWith" => "starts_with",
                            "endsWith" => "ends_with",
                            _ => unreachable!(),
                        };
                        let receiver_name =
                            format!("__thaw_string_search_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name =
                            format!("__thaw_string_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let position_name =
                            format!("__thaw_string_search_position_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(needle_name.clone(), HirType::Str);
                        self.scope.insert(position_name.clone(), HirType::F64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(needle_name.clone()),
                                HirExpr::Var(position_name.clone()),
                            ],
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, HirType::Str, needle));
                        bindings.push((position_name, HirType::F64, position));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if matches!(property.sym.as_ref(), "startsWith" | "endsWith") {
                        return Err(format!(
                            "`.{}` requires a string receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    let HirType::Array(element) = receiver_type.clone() else {
                        return Err(format!(
                            "`.{}` requires a homogeneous array receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let needle = arguments[0].clone();
                    let needle_type = self.infer_expr_type(&needle)?;
                    let from_index = if let Some(argument) = arguments.get(1) {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else if property.sym == *"lastIndexOf" {
                        HirExpr::Lit(HirLit::F64(f64::INFINITY))
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    if needle_type != *element {
                        let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let start_name = format!("__thaw_search_start_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(needle_name.clone(), needle_type.clone());
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let result = if property.sym == *"includes" {
                            HirExpr::Lit(HirLit::Bool(false))
                        } else {
                            HirExpr::Lit(HirLit::F64(-1.0))
                        };
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, needle_type, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let prefix = match element.as_ref() {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "array search does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = match property.sym.as_ref() {
                        "includes" => "includes",
                        "lastIndexOf" => "last_index_of",
                        _ => "index_of",
                    };
                    let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                    self.next_binding += 1;
                    let start_name = format!("__thaw_search_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(needle_name.clone(), needle_type.clone());
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(needle_name.clone()),
                            HirExpr::Var(start_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((needle_name, needle_type, needle));
                    bindings.push((start_name, HirType::F64, from_index));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}

