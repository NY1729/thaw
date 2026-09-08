impl<'a> FnLowerer<'a> {
    fn lower_native_scalar_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"codePointAt" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "codePointAt receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.codePointAt")?;
                    if arguments.len() > 1 {
                        return Err("native `.codePointAt()` expects zero or one argument".into());
                    }
                    let index = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name =
                        format!("__thaw_code_point_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_code_point_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_code_point_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_code_point_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    self.scope.insert(raw_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |name: String, value| {
                        HirStmt::Expr(HirExpr::Assign(name, Box::new(value)))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&index_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(normalized_name.clone(), number(0.0))],
                        ),
                        assign(
                            normalized_name.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                                vec![var(&normalized_name)],
                            ),
                        ),
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::F64,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_code_point_at".into())),
                                vec![var(&receiver_name), var(&normalized_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&raw_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(HirType::F64)))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&raw_name)),
                            HirType::F64,
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(HirType::F64));
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
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((index_name, HirType::F64, index));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"concat" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "native concat")?;
                    if let HirType::Array(element) = receiver_type {
                        let element = element.as_ref().clone();
                        let array_type = HirType::Array(Box::new(element.clone()));
                        let receiver_name = format!("__thaw_concat_part_0_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(receiver_name.clone(), array_type.clone());
                        let mut bindings = vec![(receiver_name.clone(), array_type, receiver)];
                        bindings.extend(spread_bindings);
                        let mut ordered = vec![HirExpr::Var(receiver_name)];
                        for (position, value) in arguments.into_iter().enumerate() {
                            let actual = self.infer_expr_type(&value)?;
                            let part = if actual == HirType::Array(Box::new(element.clone())) {
                                value
                            } else if actual == element {
                                HirExpr::ArrayLit(vec![value])
                            } else {
                                return Err(format!(
                                    "array concat argument has type {actual:?}, expected {element:?} or an array of it"
                                ));
                            };
                            let ty = self.infer_expr_type(&part)?;
                            let name = format!(
                                "__thaw_concat_part_{}_{}",
                                position + 1,
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            bindings.push((name.clone(), ty, part));
                            ordered.push(HirExpr::Var(name));
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayConcat(ordered, element),
                            &bindings,
                        );
                    }
                    self.expect_type(&HirType::Str, &receiver, "string concat receiver")?;
                    let receiver_name =
                        format!("__thaw_string_concat_part_0_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    let mut bindings =
                        vec![(receiver_name.clone(), HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    let mut values = vec![HirExpr::Var(receiver_name)];
                    for (position, source) in arguments.into_iter().enumerate() {
                        let source = self.coerce_primitive_to_string(source)?;
                        let name = format!(
                            "__thaw_string_concat_part_{}_{}",
                            position + 1,
                            self.next_binding
                        );
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        bindings.push((name.clone(), HirType::Str, source));
                        values.push(HirExpr::Var(name));
                    }
                    let mut values = values.into_iter();
                    let mut result = values.next().expect("concat always has a receiver");
                    for value in values {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![result, value],
                        );
                    }
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "trim" | "trimStart" | "trimEnd") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string trim receiver")?;
                    let suffix = match property.sym.as_ref() {
                        "trim" => "trim",
                        "trimStart" => "trim_start",
                        "trimEnd" => "trim_end",
                        _ => unreachable!(),
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if property.sym == *"repeat" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string repeat receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.repeat")?;
                    let [count] = arguments.as_slice() else {
                        return Err("native `.repeat()` expects exactly one count".into());
                    };
                    let count = self.coerce_primitive_to_number(count.clone())?;
                    let receiver_name = format!("__thaw_repeat_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let count_name = format!("__thaw_repeat_count_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name = format!("__thaw_repeat_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(count_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "Invalid count value for String.prototype.repeat".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&count_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(number(f64::INFINITY)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_repeat".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((count_name, HirType::F64, count));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if matches!(property.sym.as_ref(), "padStart" | "padEnd") {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string pad receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.padStart/padEnd")?;
                    if arguments.is_empty() || arguments.len() > 2 {
                        return Err(format!(
                            "native `.{}()` expects one or two arguments",
                            property.sym
                        ));
                    }
                    let target_length = self.coerce_primitive_to_number(arguments[0].clone())?;
                    let pad = match arguments.get(1) {
                        Some(pad) => self.coerce_primitive_to_string(pad.clone())?,
                        None => HirExpr::Lit(HirLit::Str(" ".into())),
                    };
                    let suffix = if property.sym == *"padStart" {
                        "pad_start"
                    } else {
                        "pad_end"
                    };
                    let receiver_name = format!("__thaw_pad_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let length_name = format!("__thaw_pad_length_{}", self.next_binding);
                    self.next_binding += 1;
                    let pad_name = format!("__thaw_pad_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(length_name.clone(), HirType::F64);
                    self.scope.insert(pad_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(pad_name.clone()),
                            HirExpr::Var(length_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((length_name, HirType::F64, target_length));
                    bindings.push((pad_name, HirType::Str, pad));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"toFixed" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::F64, &receiver, "toFixed receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Number.toFixed")?;
                    if arguments.len() > 1 {
                        return Err("native `.toFixed()` expects zero or one argument".into());
                    }
                    let digits = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_to_fixed_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let digits_name = format!("__thaw_to_fixed_digits_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_to_fixed_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::F64);
                    self.scope.insert(digits_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "toFixed() digits argument must be between 0 and 100".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&digits_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Gt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(100.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_fixed".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::F64, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((digits_name, HirType::F64, digits));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if property.sym == *"toPrecision" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::F64, &receiver, "toPrecision receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Number.toPrecision")?;
                    if arguments.len() > 1 {
                        return Err("native `.toPrecision()` expects zero or one argument".into());
                    }
                    let receiver_name =
                        format!("__thaw_to_precision_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::F64);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let Some(argument) = arguments.first() else {
                        let mut bindings = vec![(receiver_name.clone(), HirType::F64, receiver)];
                        bindings.extend(spread_bindings);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_string".into())),
                            vec![var(&receiver_name)],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    };
                    let precision = self.coerce_primitive_to_number(argument.clone())?;
                    let precision_name =
                        format!("__thaw_to_precision_digits_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_to_precision_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(precision_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "toPrecision() argument must be between 1 and 100".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&precision_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(1.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Gt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(100.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_precision".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::F64, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((precision_name, HirType::F64, precision));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if matches!(property.sym.as_ref(), "toLowerCase" | "toUpperCase") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string case receiver")?;
                    let suffix = if property.sym == *"toLowerCase" {
                        "to_lower_case"
                    } else {
                        "to_upper_case"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(property.sym.as_ref(), "isWellFormed" | "toWellFormed") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(
                        &HirType::Str,
                        &receiver,
                        &format!("string {} receiver", property.sym),
                    )?;
                    if property.sym == *"toWellFormed" {
                        return Ok(receiver);
                    }
                    let receiver_name =
                        format!("__thaw_well_formed_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(true)),
                        &[(receiver_name, HirType::Str, receiver)],
                    );
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}

