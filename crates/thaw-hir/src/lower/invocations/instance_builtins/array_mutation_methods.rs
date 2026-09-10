impl<'a> FnLowerer<'a> {
    fn lower_native_array_mutation_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if matches!(property.sym.as_ref(), "next" | "throw" | "return") {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Function(params, result) = &receiver_type else {
                        return Err(format!(
                            "`.{}` requires a generator, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let [
                        HirType::I64,
                        HirType::Str,
                        input_type,
                        return_channel @ HirType::Array(_),
                        return_request @ HirType::Array(_),
                        forced_channel @ HirType::Array(_),
                    ] = params.as_slice()
                    else {
                        return Err(format!(
                            "`.{}` requires a generator, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let HirType::Array(return_element) = return_channel else {
                        unreachable!("generator return channel was matched as an array")
                    };
                    let return_element = return_element.as_ref().clone();
                    let completion_name =
                        format!("__thaw_generator_completion_{}", self.next_binding);
                    self.next_binding += 1;
                    let request_name =
                        format!("__thaw_generator_return_request_{}", self.next_binding);
                    self.next_binding += 1;
                    let forced_name =
                        format!("__thaw_generator_forced_return_{}", self.next_binding);
                    self.next_binding += 1;
                    let (generated_type, async_generator) = match result.as_ref() {
                        HirType::Array(_) => (result.as_ref().clone(), false),
                        HirType::Promise(generated)
                            if matches!(generated.as_ref(), HirType::Array(_)) =>
                        {
                            (generated.as_ref().clone(), true)
                        }
                        _ => {
                            return Err(format!(
                                "`.{}` requires a generator, got {receiver_type:?}",
                                property.sym
                            ))
                        }
                    };
                    let HirType::Array(element) = &generated_type else {
                        return Err(format!(
                            "`.{}` requires a generator, got {generated_type:?}",
                            property.sym
                        ));
                    };
                    let element = element.as_ref().clone();
                    let return_value = if property.sym == *"return" {
                        if call.args.len() > 1
                            || call.args.iter().any(|argument| argument.spread.is_some())
                        {
                            return Err("generator `.return()` accepts at most one value".into());
                        }
                        Some(match call.args.first() {
                            Some(argument) => HirExpr::TypedClosure(
                                return_request.clone(),
                                Box::new(HirExpr::ArrayLit(vec![
                                    self.lower_expr_with_expected_type(
                                        &argument.expr,
                                        Some(&return_element),
                                    )?,
                                ])),
                            ),
                            None => HirExpr::TypedClosure(
                                return_request.clone(),
                                Box::new(HirExpr::ArrayLit(Vec::new())),
                            ),
                        })
                    } else {
                        None
                    };
                    let request_value = return_value.unwrap_or_else(|| {
                        HirExpr::TypedClosure(
                            return_request.clone(),
                            Box::new(HirExpr::ArrayLit(Vec::new())),
                        )
                    });
                    let resume = if property.sym == *"throw" {
                        let [argument] = call.args.as_slice() else {
                            return Err("generator `.throw()` expects exactly one value".into());
                        };
                        if argument.spread.is_some() {
                            return Err("generator `.throw()` does not accept a spread value".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        vec![
                            HirExpr::Lit(HirLit::I64(2)),
                            self.coerce_primitive_to_string(value)?,
                            generator_placeholder(input_type).ok_or_else(|| {
                                format!(
                                    "generator input type {input_type:?} has no default value"
                                )
                            })?,
                            HirExpr::Var(completion_name.clone()),
                            HirExpr::Var(request_name.clone()),
                            HirExpr::Var(forced_name.clone()),
                        ]
                    } else if property.sym == *"return" {
                        vec![
                            HirExpr::Lit(HirLit::I64(1)),
                            HirExpr::Lit(HirLit::Str(String::new())),
                            generator_placeholder(input_type).ok_or_else(|| {
                                format!(
                                    "generator input type {input_type:?} has no default value"
                                )
                            })?,
                            HirExpr::Var(completion_name.clone()),
                            HirExpr::Var(request_name.clone()),
                            HirExpr::Var(forced_name.clone()),
                        ]
                    } else {
                        if call.args.len() > 1
                            || call.args.iter().any(|argument| argument.spread.is_some())
                        {
                            return Err("generator `.next()` accepts at most one value".into());
                        }
                        let input = match call.args.first() {
                            Some(argument) => self.lower_expr_with_expected_type(
                                &argument.expr,
                                Some(input_type),
                            )?,
                            None => generator_placeholder(input_type).ok_or_else(|| {
                                format!(
                                    "generator input type {input_type:?} has no default value"
                                )
                            })?,
                        };
                        vec![
                            HirExpr::Lit(HirLit::I64(0)),
                            HirExpr::Lit(HirLit::Str(String::new())),
                            input,
                            HirExpr::Var(completion_name.clone()),
                            HirExpr::Var(request_name.clone()),
                            HirExpr::Var(forced_name.clone()),
                        ]
                    };
                    let value_type = if return_element == HirType::Undefined {
                        HirType::Optional(Box::new(element.clone()))
                    } else {
                        let mut members = vec![element.clone()];
                        if !members.contains(&return_element) {
                            members.push(return_element.clone());
                        }
                        if !members.contains(&HirType::Undefined) {
                            members.push(HirType::Undefined);
                        }
                        HirType::Union(members)
                    };
                    let result_type = HirType::Object(vec![
                        ("value".into(), value_type.clone()),
                        ("done".into(), HirType::Bool),
                    ]);
                    let producer_name =
                        format!("__thaw_generator_producer_{}", self.next_binding);
                    self.next_binding += 1;
                    let array_name = format!("__thaw_generator_next_{}", self.next_binding);
                    self.next_binding += 1;
                    let empty = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                            array_name.clone(),
                        )))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    );
                    let no_completion = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                            completion_name.clone(),
                        )))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    );
                    let no_forced_return = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                            forced_name.clone(),
                        )))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    );
                    let completion_shift = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_shift".into())),
                        vec![HirExpr::Var(completion_name.clone())],
                    );
                    let yielded_shift = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_shift".into())),
                        vec![HirExpr::Var(array_name.clone())],
                    );
                    let forced_shift = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_shift".into())),
                        vec![HirExpr::Var(forced_name.clone())],
                    );
                    let (undefined, completion, yielded, forced) = match &value_type {
                        HirType::Optional(_) => (
                            HirExpr::OptionalNone(element.clone()),
                            HirExpr::OptionalNone(element.clone()),
                            HirExpr::OptionalSome(Box::new(yielded_shift), element.clone()),
                            HirExpr::OptionalNone(element.clone()),
                        ),
                        HirType::Union(members) => (
                            HirExpr::UnionInject(
                                Box::new(HirExpr::Lit(HirLit::Undefined)),
                                members.len() - 1,
                                members.clone(),
                            ),
                            HirExpr::UnionInject(
                                Box::new(completion_shift),
                                members
                                    .iter()
                                    .position(|member| member == &return_element)
                                    .expect("generator return type is a result union member"),
                                members.clone(),
                            ),
                            HirExpr::UnionInject(
                                Box::new(yielded_shift),
                                0,
                                members.clone(),
                            ),
                            HirExpr::UnionInject(
                                Box::new(forced_shift),
                                members
                                    .iter()
                                    .position(|member| member == &return_element)
                                    .expect("generator return type is a result union member"),
                                members.clone(),
                            ),
                        ),
                        _ => unreachable!("generator result value is optional or a union"),
                    };
                    let done_without_value = HirExpr::ObjectLit(vec![
                        ("value".into(), undefined),
                        ("done".into(), HirExpr::Lit(HirLit::Bool(true))),
                    ]);
                    let done_with_value = HirExpr::ObjectLit(vec![
                        ("value".into(), completion),
                        ("done".into(), HirExpr::Lit(HirLit::Bool(true))),
                    ]);
                    let done_with_forced_value = HirExpr::ObjectLit(vec![
                        ("value".into(), forced),
                        ("done".into(), HirExpr::Lit(HirLit::Bool(true))),
                    ]);
                    let next = HirExpr::ObjectLit(vec![
                        ("value".into(), yielded),
                        ("done".into(), HirExpr::Lit(HirLit::Bool(false))),
                    ]);
                    let mapper_captures = vec![
                        HirParam {
                            name: completion_name.clone(),
                            ty: return_channel.clone(),
                        },
                        HirParam {
                            name: forced_name.clone(),
                            ty: forced_channel.clone(),
                        },
                    ];
                    let mapper = HirExpr::Lambda(
                        mapper_captures,
                        vec![HirParam {
                            name: array_name,
                            ty: generated_type.clone(),
                        }],
                        result_type.clone(),
                        Box::new(HirExpr::Block(vec![HirStmt::If(
                            empty,
                            vec![HirStmt::If(
                                no_forced_return,
                                vec![HirStmt::If(
                                    no_completion,
                                    vec![HirStmt::Return(Some(done_without_value))],
                                    vec![HirStmt::Return(Some(done_with_value))],
                                )],
                                vec![HirStmt::Return(Some(done_with_forced_value))],
                            )],
                            vec![HirStmt::Return(Some(next))],
                        )])),
                    );
                    let producer_call = HirExpr::Call(
                        Box::new(HirExpr::Var(producer_name.clone())),
                        resume,
                    );
                    let (wrapper_result, wrapper_body) = if async_generator {
                        (
                            HirType::Promise(Box::new(result_type.clone())),
                            HirExpr::PromiseThen(
                                Box::new(producer_call),
                                Box::new(mapper),
                                generated_type,
                                result_type,
                                false,
                                false,
                            ),
                        )
                    } else {
                        (
                            result_type,
                            HirExpr::Call(Box::new(mapper), vec![producer_call]),
                        )
                    };
                    let wrapper_params = vec![
                        HirParam {
                            name: producer_name.clone(),
                            ty: receiver_type.clone(),
                        },
                        HirParam {
                            name: completion_name.clone(),
                            ty: return_channel.clone(),
                        },
                        HirParam {
                            name: request_name.clone(),
                            ty: return_request.clone(),
                        },
                        HirParam {
                            name: forced_name.clone(),
                            ty: forced_channel.clone(),
                        },
                    ];
                    let wrapper_args = vec![
                        receiver,
                        HirExpr::TypedClosure(
                            return_channel.clone(),
                            Box::new(HirExpr::ArrayLit(Vec::new())),
                        ),
                        request_value,
                        HirExpr::TypedClosure(
                            forced_channel.clone(),
                            Box::new(HirExpr::ArrayLit(Vec::new())),
                        ),
                    ];
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            Vec::new(),
                            wrapper_params,
                            wrapper_result,
                            Box::new(wrapper_body),
                        )),
                        wrapper_args,
                    ));
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
                if property.sym == *"slice" || property.sym == *"subarray" {
                    let is_subarray = property.sym == *"subarray";
                    let mut receiver = self.lower_expr(&member.obj)?;
                    // `slice`/`subarray` keep the byte-buffer identity: a
                    // sliced `Buffer` is still a `Buffer`, so a chained
                    // `buf.slice(0, 4).toString("hex")` decodes rather than
                    // comma-joining. `infer_expr_type` normalizes `Bytes`
                    // away, so ask for the raw receiver type.
                    let receiver_is_bytes =
                        self.infer_expr_type_inner(&receiver)? == HirType::Bytes;
                    let mut receiver_type = self.infer_expr_type(&receiver)?;
                    if matches!(receiver_type, HirType::Json | HirType::JsValue) {
                        receiver = self.coerce_primitive_to_string(receiver)?;
                        receiver_type = HirType::Str;
                    }
                    if is_subarray && !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.subarray()` is only supported on a Buffer / array, got {receiver_type:?}"
                        ));
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
                    let intrinsic = if receiver_is_bytes {
                        "__thaw_bytes_slice"
                    } else {
                        "__thaw_array_slice"
                    };
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
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
                    // `buf.indexOf` / `includes` / `lastIndexOf` on a byte
                    // buffer with a string or sub-buffer needle is a
                    // *subsequence* search, not the element search a plain
                    // array does. (A numeric needle stays on the array
                    // path -- it's already an element match.)
                    if matches!(property.sym.as_ref(), "indexOf" | "includes" | "lastIndexOf")
                        && self.infer_expr_type_inner(&receiver)? == HirType::Bytes
                        && !matches!(
                            self.infer_expr_type(&arguments[0])?,
                            HirType::F64
                        )
                    {
                        return self.lower_native_bytes_search(
                            receiver,
                            property.sym.as_ref(),
                            arguments,
                            spread_bindings,
                        );
                    }
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
