impl<'a> FnLowerer<'a> {
    fn lower_native_array_mutation_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if matches!(property.sym.as_ref(), "next" | "throw" | "return") {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    let (resume, resume_binding) = if property.sym == *"throw" {
                        let [argument] = call.args.as_slice() else {
                            return Err("generator `.throw()` expects exactly one value".into());
                        };
                        if argument.spread.is_some() {
                            return Err("generator `.throw()` does not accept a spread value".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let value = self.coerce_primitive_to_string(value)?;
                        let name = format!("__thaw_generator_throw_{}", self.next_binding);
                        self.next_binding += 1;
                        (vec![
                            HirExpr::Lit(HirLit::I64(2)),
                            HirExpr::Var(name.clone()),
                            generator_placeholder(input_type).ok_or_else(|| {
                                format!(
                                    "generator input type {input_type:?} has no default value"
                                )
                            })?,
                            HirExpr::Var(completion_name.clone()),
                            HirExpr::Var(request_name.clone()),
                            HirExpr::Var(forced_name.clone()),
                        ], Some((name, HirType::Str, value)))
                    } else if property.sym == *"return" {
                        (vec![
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
                        ], None)
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
                        let name = format!("__thaw_generator_input_{}", self.next_binding);
                        self.next_binding += 1;
                        (vec![
                            HirExpr::Lit(HirLit::I64(0)),
                            HirExpr::Lit(HirLit::Str(String::new())),
                            HirExpr::Var(name.clone()),
                            HirExpr::Var(completion_name.clone()),
                            HirExpr::Var(request_name.clone()),
                            HirExpr::Var(forced_name.clone()),
                        ], Some((name, input_type.clone(), input)))
                    };
                    let return_is_optional_element = matches!(
                        &return_element,
                        HirType::Optional(payload) if payload.as_ref() == &element
                    );
                    let return_union_members = match &return_element {
                        HirType::Union(members) if members.contains(&element) => {
                            Some(members.clone())
                        }
                        _ => None,
                    };
                    let value_type = if return_element == HirType::Undefined
                        && matches!(&element, HirType::Optional(_))
                    {
                        element.clone()
                    } else if return_element == HirType::Undefined {
                        HirType::Optional(Box::new(element.clone()))
                    } else if return_is_optional_element {
                        return_element.clone()
                    } else if let Some(mut members) = return_union_members.clone() {
                        if !members.contains(&HirType::Undefined) {
                            members.push(HirType::Undefined);
                        }
                        HirType::Union(members)
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
                    let (undefined, completion, yielded, forced) = match (&value_type, &element) {
                        (HirType::Optional(payload), HirType::Optional(_))
                            if return_element == HirType::Undefined =>
                        {
                            (
                                HirExpr::OptionalNone(payload.as_ref().clone()),
                                HirExpr::OptionalNone(payload.as_ref().clone()),
                                yielded_shift,
                                HirExpr::OptionalNone(payload.as_ref().clone()),
                            )
                        }
                        (HirType::Optional(_), _) if return_element == HirType::Undefined => (
                            HirExpr::OptionalNone(element.clone()),
                            HirExpr::OptionalNone(element.clone()),
                            HirExpr::OptionalSome(Box::new(yielded_shift), element.clone()),
                            HirExpr::OptionalNone(element.clone()),
                        ),
                        (HirType::Optional(_), _) => (
                            HirExpr::OptionalNone(element.clone()),
                            completion_shift,
                            HirExpr::OptionalSome(Box::new(yielded_shift), element.clone()),
                            forced_shift,
                        ),
                        (HirType::Union(members), _) => (
                            HirExpr::UnionInject(
                                Box::new(HirExpr::Lit(HirLit::Undefined)),
                                members.len() - 1,
                                members.clone(),
                            ),
                            if return_union_members.is_some() {
                                completion_shift
                            } else {
                                HirExpr::UnionInject(
                                    Box::new(completion_shift),
                                    members
                                        .iter()
                                        .position(|member| member == &return_element)
                                        .expect("generator return type is a result union member"),
                                    members.clone(),
                                )
                            },
                            HirExpr::UnionInject(
                                Box::new(yielded_shift),
                                members
                                    .iter()
                                    .position(|member| member == &element)
                                    .expect("generator yield type is a result union member"),
                                members.clone(),
                            ),
                            if return_union_members.is_some() {
                                forced_shift
                            } else {
                                HirExpr::UnionInject(
                                    Box::new(forced_shift),
                                    members
                                        .iter()
                                        .position(|member| member == &return_element)
                                        .expect("generator return type is a result union member"),
                                    members.clone(),
                                )
                            },
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
                    let mut wrapper_params = vec![
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
                    let mut wrapper_args = vec![
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
                    if let Some((name, ty, value)) = resume_binding {
                        wrapper_params.push(HirParam { name, ty });
                        wrapper_args.push(value);
                    }
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
                // `.forEach()` on a `Map`/`Set` value stored in `any` --
                // same rationale as `.get`/`.has`'s own `Json` branches
                // (`map_set_methods.rs`): no static K/V to decode into,
                // so this unwraps to the uniform `[key, value]` pairs
                // `.entries()` already produces
                // (`__thaw_json_map_or_set_entries_view` -- a Set's
                // pairs are `[v, v]`) and invokes the callback once per
                // pair with plain `any`/`Json` arguments, matching what
                // a real dynamic `.forEach()` genuinely passes.
                if property.sym == *"forEach"
                    && self.peek_type_without_lowering(&member.obj) == Some(HirType::Json)
                {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                                "Map/Set forEach callback must be an arrow or function value".into(),
                            )
                        }
                    };
                    if arity > 3 {
                        return Err(format!(
                            "Map/Set forEach callback accepts at most three parameters, got {arity}"
                        ));
                    }
                    let available = [HirType::Json, HirType::Json, HirType::Json];
                    let callback = self.lower_promise_callback(
                        &argument.expr,
                        &available[..arity],
                        Some(&HirType::Void),
                    )?;
                    let callback_type = self.infer_expr_type(&callback)?;
                    let HirType::Function(params, _) = &callback_type else {
                        unreachable!("Map/Set forEach callback was validated as a function")
                    };
                    let params_len = params.len();

                    let receiver_name = format!("__thaw_map_for_each_any_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let callback_name = format!("__thaw_map_for_each_any_callback_{}", self.next_binding);
                    self.next_binding += 1;
                    let pairs_name = format!("__thaw_map_for_each_any_pairs_{}", self.next_binding);
                    self.next_binding += 1;
                    let length_name = format!("__thaw_map_for_each_any_length_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_map_for_each_any_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let pair_name = format!("__thaw_map_for_each_any_pair_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_for_each_any_key_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_map_for_each_any_value_{}", self.next_binding);
                    self.next_binding += 1;

                    let receiver_type = HirType::Json;
                    let pairs_type = HirType::Array(Box::new(HirType::Json));
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(callback_name.clone(), callback_type.clone());
                    self.scope.insert(pairs_name.clone(), pairs_type.clone());
                    self.scope.insert(length_name.clone(), HirType::F64);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(pair_name.clone(), HirType::Json);
                    self.scope.insert(key_name.clone(), HirType::Json);
                    self.scope.insert(value_name.clone(), HirType::Json);

                    let var = |name: &str| HirExpr::Var(name.into());
                    let available_vars = [var(&value_name), var(&key_name), var(&receiver_name)];
                    let callback_call = HirExpr::Call(
                        Box::new(var(&callback_name)),
                        available_vars[..params_len].to_vec(),
                    );

                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            pairs_name.clone(),
                            pairs_type.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    "__thaw_json_map_or_set_entries_view".to_string(),
                                )),
                                vec![var(&receiver_name)],
                            ),
                        ),
                        HirStmt::Let(
                            length_name.clone(),
                            HirType::F64,
                            HirExpr::ArrayLen(Box::new(var(&pairs_name))),
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
                                    pair_name.clone(),
                                    HirType::Json,
                                    HirExpr::TypedIndex(
                                        Box::new(var(&pairs_name)),
                                        Box::new(var(&index_name)),
                                        HirType::Json,
                                    ),
                                ),
                                HirStmt::Let(
                                    key_name.clone(),
                                    HirType::Json,
                                    HirExpr::JsonIndex(
                                        Box::new(var(&pair_name)),
                                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                    ),
                                ),
                                HirStmt::Let(
                                    value_name.clone(),
                                    HirType::Json,
                                    HirExpr::JsonIndex(
                                        Box::new(var(&pair_name)),
                                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
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
                    let bindings = vec![
                        (receiver_name, receiver_type, receiver),
                        (callback_name, callback_type, callback),
                    ];
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if property.sym == *"forEach" && self.receiver_is_map_or_set(&member.obj) {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (key_type, value_type, keys_double_as_values) = match &receiver_type {
                        HirType::Map(key_type, value_type) => {
                            (key_type.as_ref().clone(), value_type.as_ref().clone(), false)
                        }
                        HirType::Set(element_type) => {
                            let element_type = element_type.as_ref().clone();
                            (element_type.clone(), element_type, true)
                        }
                        HirType::WeakMap(_, _) | HirType::WeakSet(_) => {
                            return Err("WeakMap/WeakSet are not enumerable and do not support `.forEach()`".into())
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
                    let sparse_callback = self.expression_may_be_sparse_array(&member.obj);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    // A union whose members are all arrays (`number[] |
                    // string[]`): the member layouts differ, so dispatch on
                    // the runtime tag and run the loop for the selected
                    // member, coercing its element into the callback's
                    // (union) parameter type.
                    if let HirType::Union(union_elements) = &array_type {
                        if union_elements.iter().all(|element| matches!(element, HirType::Array(_)))
                        {
                            if !(1..=2).contains(&call.args.len()) {
                                return Err(
                                    "native `.forEach()` expects a callback and optional thisArg"
                                        .into(),
                                );
                            }
                            let mut element_types = Vec::new();
                            for element in union_elements {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                if !element_types.contains(inner.as_ref()) {
                                    element_types.push(inner.as_ref().clone());
                                }
                            }
                            let union_element_type = match element_types.as_slice() {
                                [single] => single.clone(),
                                _ => HirType::Union(element_types),
                            };
                            let callback = self.lower_array_callback(
                                &call.args[0].expr,
                                &union_element_type,
                                &array_type,
                                Some(&HirType::Void),
                            )?;
                            let this_arg = call
                                .args
                                .get(1)
                                .map(|argument| self.lower_expr(&argument.expr))
                                .transpose()?;
                            let name = format!("__thaw_union_for_each_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), array_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let mut statements: Vec<HirStmt> = Vec::new();
                            for (index, element) in union_elements.iter().enumerate().rev() {
                                let HirType::Array(inner) = element else {
                                    unreachable!("checked above")
                                };
                                let member_value = HirExpr::UnionValue(
                                    Box::new(bound.clone()),
                                    index,
                                    union_elements.clone(),
                                );
                                let loop_expr = self.lower_array_for_each(
                                    member_value,
                                    HirType::Array(inner.clone()),
                                    inner.as_ref().clone(),
                                    inner.as_ref().clone(),
                                    callback.clone(),
                                    this_arg.clone(),
                                )?;
                                if statements.is_empty() {
                                    statements = vec![HirStmt::Expr(loop_expr)];
                                } else {
                                    statements = vec![HirStmt::If(
                                        HirExpr::BinOp(
                                            BinOp::EqEqEq,
                                            Box::new(HirExpr::UnionTag(
                                                Box::new(bound.clone()),
                                                union_elements.clone(),
                                            )),
                                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                        ),
                                        vec![HirStmt::Expr(loop_expr)],
                                        statements,
                                    )];
                                }
                            }
                            let result = HirExpr::Block(statements);
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(name, array_type.clone(), receiver)],
                            );
                        }
                    }
                    let element_type = match &array_type {
                        HirType::Array(element) => element.as_ref().clone(),
                        HirType::Tuple(elements) => elements
                            .iter()
                            .find(|element| **element != HirType::Undefined)
                            .cloned()
                            .unwrap_or(HirType::Undefined),
                        _ => {
                            return Err(format!(
                                "`.forEach()` requires an array, got {array_type:?}"
                            ));
                        }
                    };
                    let callback_element_type = if sparse_callback {
                        Self::array_read_type(&element_type)
                    } else {
                        element_type.clone()
                    };
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
                            callback_element_type.clone(),
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
                            "native `.forEach()` expects a callback and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &callback_element_type,
                        &array_type,
                        Some(&HirType::Void),
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
                        callback_element_type,
                        callback,
                        this_arg,
                    );
                }
                if matches!(
                    property.sym.as_ref(),
                    "slice" | "subarray" | "substring" | "substr"
                ) {
                    let is_subarray = property.sym == *"subarray";
                    let is_substring = property.sym == *"substring" || property.sym == *"substr";
                    let mut receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    if is_subarray
                        && !matches!(receiver_type, HirType::Array(_) | HirType::Union(_))
                    {
                        return Err(format!(
                            "`.subarray()` is only supported on a Buffer / array, got {receiver_type:?}"
                        ));
                    }
                    if is_substring && receiver_type != HirType::Str {
                        return Err(format!(
                            "`.{}()` requires a string receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    if receiver_type == HirType::Str {
                        let label = match property.sym.as_ref() {
                            "substring" => "String.substring",
                            "substr" => "String.substr",
                            _ => "String.slice",
                        };
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, label)?;
                        if arguments.len() > 2 {
                            return Err(format!("native `{label}` expects zero to two arguments"));
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
                        let intrinsic = match property.sym.as_ref() {
                            "substring" => "__thaw_string_substring",
                            "substr" => "__thaw_string_substr",
                            _ => "__thaw_string_slice",
                        };
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.into())),
                            vec![receiver, start, end],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !matches!(receiver_type, HirType::Array(_) | HirType::Union(_)) {
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
                    if let HirType::Union(members) = &receiver_type {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!(
                                "`.slice()` requires an array union, got {receiver_type:?}"
                            ));
                        }
                        let receiver_name =
                            format!("__thaw_union_slice_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), receiver_type.clone());
                        let mut bindings = vec![(
                            receiver_name.clone(),
                            receiver_type.clone(),
                            receiver,
                        )];
                        bindings.extend(spread_bindings);
                        let mut index_names = Vec::with_capacity(2);
                        for (position, index) in indices.into_iter().enumerate() {
                            let name = format!(
                                "__thaw_union_slice_index_{position}_{}",
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), HirType::F64);
                            bindings.push((name.clone(), HirType::F64, index));
                            index_names.push(name);
                        }
                        let branches = members
                            .iter()
                            .enumerate()
                            .map(|(index, _)| {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                    vec![
                                        HirExpr::UnionValue(
                                            Box::new(HirExpr::Var(receiver_name.clone())),
                                            index,
                                            members.clone(),
                                        ),
                                        HirExpr::Var(index_names[0].clone()),
                                        HirExpr::Var(index_names[1].clone()),
                                    ],
                                )
                            })
                            .collect();
                        let result = self.merge_union_array_method_branches(
                            &receiver_name,
                            members,
                            branches,
                        )?;
                        return self.wrap_call_argument_bindings(result, &bindings);
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
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_) | HirType::Union(_)) {
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
                    let mut arguments = Vec::with_capacity(4);
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_copy_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = if let HirType::Union(members) = &bindings[0].1 {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!("`.copyWithin()` requires an array union, got {:?}", bindings[0].1));
                        }
                        let branches = members.iter().enumerate().map(|(index, _)| {
                            let mut branch_arguments = vec![HirExpr::UnionValue(
                                Box::new(HirExpr::Var(receiver_name.clone())), index, members.clone(),
                            )];
                            branch_arguments.extend(arguments.clone());
                            HirExpr::Call(Box::new(HirExpr::Var("__thaw_array_copy_within".into())), branch_arguments)
                        }).collect();
                        self.merge_union_array_method_branches(&receiver_name, members, branches)?
                    } else {
                        let mut call_arguments = vec![HirExpr::Var(receiver_name)];
                        call_arguments.extend(arguments);
                        HirExpr::Call(Box::new(HirExpr::Var("__thaw_array_copy_within".into())), call_arguments)
                    };
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"fill" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let element_types = match &receiver_type {
                        HirType::Array(element) => vec![element.as_ref().clone()],
                        HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))) => members.iter().map(|member| {
                            let HirType::Array(element) = member else { unreachable!() };
                            element.as_ref().clone()
                        }).collect(),
                        _ => {
                        return Err(format!(
                            "`.fill()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                        }
                    };
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_array_values(&call.args, "Array.fill")?;
                    if !(1..=3).contains(&arguments.len()) {
                        return Err("native `.fill()` expects one to three arguments".into());
                    }
                    let value = arguments[0].clone();
                    for element in &element_types {
                        self.coerce_array_insert_value(value.clone(), element).map_err(|error| {
                            format!("`.fill()` value is not safe for every array union member: {error}")
                        })?;
                    }
                    let value_type = self.infer_expr_type(&value)?;
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
                    let receiver_name = format!("__thaw_fill_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let value_name = format!("__thaw_fill_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), value_type.clone());
                    let mut bindings = vec![
                        (receiver_name.clone(), receiver_type, receiver),
                    ];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name.clone(), value_type, value));
                    let mut index_arguments = Vec::with_capacity(2);
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_fill_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        index_arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let mut branch = |array: HirExpr, element: &HirType| {
                        let coerced = self.coerce_array_insert_value(HirExpr::Var(value_name.clone()), element)?;
                        let coerced_type = self.infer_expr_type(&coerced)?;
                        let runtime = match element {
                            _ if coerced_type != *element => "__thaw_array_fill",
                            HirType::F64 => "__thaw_number_array_fill",
                            HirType::Bool => "__thaw_bool_array_fill",
                            HirType::Str | HirType::Array(_) | HirType::Object(_) => "__thaw_pointer_array_fill",
                            _ => "__thaw_array_fill",
                        };
                        let mut arguments = vec![array, coerced];
                        arguments.extend(index_arguments.clone());
                        Ok::<_, String>(HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), arguments))
                    };
                    let result = if let HirType::Union(members) = &bindings[0].1 {
                        let branches = members.iter().enumerate().map(|(index, member)| {
                            let HirType::Array(element) = member else { unreachable!() };
                            branch(HirExpr::UnionValue(Box::new(HirExpr::Var(receiver_name.clone())), index, members.clone()), element)
                        }).collect::<Result<Vec<_>, _>>()?;
                        self.merge_union_array_method_branches(&receiver_name, members, branches)?
                    } else {
                        branch(HirExpr::Var(receiver_name), &element_types[0])?
                    };
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"reverse" {
                    if !call.args.is_empty() {
                        return Err("native `.reverse()` expects no arguments".into());
                    }
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_) | HirType::Union(_)) {
                        return Err(format!(
                            "`.reverse()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    if let HirType::Union(members) = &receiver_type {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!("`.reverse()` requires an array union, got {receiver_type:?}"));
                        }
                        let name = format!("__thaw_union_reverse_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), receiver_type.clone());
                        let branches = members.iter().enumerate().map(|(index, _)| HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_reverse".into())),
                            vec![HirExpr::UnionValue(Box::new(HirExpr::Var(name.clone())), index, members.clone())],
                        )).collect();
                        let result = self.merge_union_array_method_branches(&name, members, branches)?;
                        return self.wrap_call_argument_bindings(result, &[(name, receiver_type, receiver)]);
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
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_) | HirType::Union(_)) {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    let runtime = if property.sym == *"pop" {
                        "__thaw_array_pop_optional"
                    } else {
                        "__thaw_array_shift_optional"
                    };
                    if let HirType::Union(members) = &receiver_type {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!("`.{}()` requires an array union, got {receiver_type:?}", property.sym));
                        }
                        let name = format!("__thaw_union_{}_{}", property.sym, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), receiver_type.clone());
                        let branches = members.iter().enumerate().map(|(index, _)| HirExpr::Call(
                            Box::new(HirExpr::Var(runtime.into())),
                            vec![HirExpr::UnionValue(Box::new(HirExpr::Var(name.clone())), index, members.clone())],
                        )).collect();
                        let result = self.merge_union_array_method_branches(&name, members, branches)?;
                        return self.wrap_call_argument_bindings(result, &[(name, receiver_type, receiver)]);
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(runtime.to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"push" || property.sym == *"unshift" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let element_types = match &receiver_type {
                        HirType::Array(element) => vec![element.as_ref().clone()],
                        HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))) => members.iter().map(|member| {
                            let HirType::Array(element) = member else { unreachable!() };
                            element.as_ref().clone()
                        }).collect(),
                        _ => return Err(format!("`.{}()` requires an array or array union, got {receiver_type:?}", property.sym)),
                    };
                    let label = if property.sym == *"push" { "push" } else { "unshift" };
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_array_values(&call.args, &format!("Array.{label}"))?;
                    // `coerce_to_declared`, not the stricter `expect_type`
                    // this replaced: a `Json`/`JsValue` value pushed into a
                    // *typed* array (real trigger: a for-of loop element
                    // decoded from a dynamic iterator, e.g. `lru-cache`'s
                    // `Generator<K>`-returning `keys()`) used to reject
                    // outright ("array push/unshift value has type Json,
                    // expected Str") with no way to push it at all except
                    // decoding it (`String(k)`) into a fresh local first.
                    // `coerce_to_declared` already falls back to this same
                    // strict `expect_type` check for anything it can't
                    // coerce, so genuinely incompatible pushes are still
                    // rejected exactly as before.
                    for value in &arguments {
                        for element in &element_types {
                            self.coerce_array_insert_value(value.clone(), element).map_err(|error| {
                                format!("`.{label}()` value is not safe for every array union member: {error}")
                            })?;
                        }
                    }
                    let receiver_name = format!("__thaw_{label}_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut value_names = Vec::with_capacity(arguments.len());
                    for (position, value) in arguments.into_iter().enumerate() {
                        let name = format!("__thaw_{label}_value_{position}_{}", self.next_binding);
                        self.next_binding += 1;
                        let value_type = self.infer_expr_type(&value)?;
                        self.scope.insert(name.clone(), value_type.clone());
                        value_names.push(name.clone());
                        bindings.push((name, value_type, value));
                    }
                    let runtime = if property.sym == *"push" {
                        "__thaw_array_push"
                    } else {
                        "__thaw_array_unshift"
                    };
                    let mut make_branch = |array: HirExpr, element: &HirType| {
                        let mut call_arguments = vec![array];
                        for name in &value_names {
                            call_arguments.push(self.coerce_array_insert_value(HirExpr::Var(name.clone()), element)?);
                        }
                        Ok::<_, String>(HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), call_arguments))
                    };
                    let result = if let HirType::Union(members) = &bindings[0].1 {
                        let branches = members.iter().enumerate().map(|(index, member)| {
                            let HirType::Array(element) = member else { unreachable!() };
                            make_branch(HirExpr::UnionValue(Box::new(HirExpr::Var(receiver_name.clone())), index, members.clone()), element)
                        }).collect::<Result<Vec<_>, _>>()?;
                        self.merge_union_array_method_branches(&receiver_name, members, branches)?
                    } else {
                        make_branch(HirExpr::Var(receiver_name), &element_types[0])?
                    };
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"splice" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let element_types = match &receiver_type {
                        HirType::Array(element) => vec![element.as_ref().clone()],
                        HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))) => members.iter().map(|member| {
                            let HirType::Array(element) = member else { unreachable!() };
                            element.as_ref().clone()
                        }).collect(),
                        _ => return Err(format!("`.splice()` requires an array or array union, got {receiver_type:?}")),
                    };
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_array_values(&call.args, "Array.splice")?;
                    let has_start = !arguments.is_empty();
                    let mut arguments = arguments.into_iter();
                    let start = match arguments.next() {
                        Some(value) => self.coerce_primitive_to_number(value)?,
                        None => HirExpr::Lit(HirLit::F64(0.0)),
                    };
                    let delete_count = match arguments.next() {
                        Some(value) => self.coerce_primitive_to_number(value)?,
                        None => HirExpr::Lit(HirLit::F64(if has_start { f64::INFINITY } else { 0.0 })),
                    };
                    let items = arguments.collect::<Vec<_>>();
                    for item in &items {
                        for element in &element_types {
                            self.coerce_array_insert_value(item.clone(), element).map_err(|error| {
                                format!("`.splice()` item is not safe for every array union member: {error}")
                            })?;
                        }
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
                    let mut item_names = Vec::with_capacity(items.len());
                    for (position, item) in items.into_iter().enumerate() {
                        let name = format!("__thaw_splice_item_{position}_{}", self.next_binding);
                        self.next_binding += 1;
                        let item_type = self.infer_expr_type(&item)?;
                        self.scope.insert(name.clone(), item_type.clone());
                        item_names.push(name.clone());
                        bindings.push((name, item_type, item));
                    }
                    let mut make_branch = |array: HirExpr, element: &HirType| {
                        let mut call_arguments = vec![array, HirExpr::Var(start_name.clone()), HirExpr::Var(delete_name.clone())];
                        for name in &item_names {
                            call_arguments.push(self.coerce_array_insert_value(HirExpr::Var(name.clone()), element)?);
                        }
                        Ok::<_, String>(HirExpr::Call(Box::new(HirExpr::Var("__thaw_array_splice".into())), call_arguments))
                    };
                    let result = if let HirType::Union(members) = &bindings[0].1 {
                        let branches = members.iter().enumerate().map(|(index, member)| {
                            let HirType::Array(element) = member else { unreachable!() };
                            make_branch(HirExpr::UnionValue(Box::new(HirExpr::Var(receiver_name.clone())), index, members.clone()), element)
                        }).collect::<Result<Vec<_>, _>>()?;
                        self.merge_union_array_method_branches(&receiver_name, members, branches)?
                    } else {
                        make_branch(HirExpr::Var(receiver_name), &element_types[0])?
                    };
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"join" {
                    let receiver = self.lower_required_member_receiver(&member.obj, "join")?;
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
                    if let HirType::Union(members) = &receiver_type {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!(
                                "`.join()` requires an array union, got {receiver_type:?}"
                            ));
                        }
                        let separator_name =
                            format!("__thaw_union_join_separator_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(separator_name.clone(), HirType::Str);
                        let mut branches = Vec::with_capacity(members.len());
                        for (index, member) in members.iter().enumerate() {
                            let HirType::Array(element) = member else { unreachable!() };
                            let builtin = match element.as_ref() {
                                HirType::F64 => "__thaw_number_array_join",
                                HirType::Str => "__thaw_string_array_join",
                                HirType::Bool => "__thaw_bool_array_join",
                                HirType::Object(_) => "__thaw_object_array_join",
                                HirType::Optional(_)
                                | HirType::Nullable(_)
                                | HirType::Nullish(_)
                                | HirType::Undefined
                                | HirType::Null
                                | HirType::Union(_) => "__thaw_tagged_array_join",
                                other => {
                                    return Err(format!(
                                        "array join does not support element type {other:?}"
                                    ))
                                }
                            };
                            branches.push(HirExpr::Call(
                                Box::new(HirExpr::Var(builtin.into())),
                                vec![
                                    HirExpr::UnionValue(
                                        Box::new(HirExpr::Var(source_name.clone())),
                                        index,
                                        members.clone(),
                                    ),
                                    HirExpr::Var(separator_name.clone()),
                                ],
                            ));
                        }
                        let result = self.merge_union_array_method_branches(
                            &source_name,
                            members,
                            branches,
                        )?;
                        let mut bindings = vec![(source_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((separator_name, HirType::Str, separator));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let result = match receiver_type.clone() {
                        HirType::Array(element) => {
                            let array_type = HirType::Array(element.clone());
                            let receiver_name =
                                format!("__thaw_join_receiver_{}", self.next_binding);
                            self.next_binding += 1;
                            let separator_name =
                                format!("__thaw_join_separator_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(receiver_name.clone(), array_type.clone());
                            self.scope.insert(separator_name.clone(), HirType::Str);
                            let result = if let HirType::Json = element.as_ref() {
                                // No native `__thaw_..._array_join` intrinsic
                                // exists for a generic `Json` element (real
                                // trigger: `Array.from(cache.keys())` on a
                                // real npm class's `Generator<K>`-returning
                                // method, e.g. `lru-cache`) -- built directly
                                // out of existing HIR nodes instead of adding
                                // a new one: a native `while` loop that reads
                                // each element out via `TypedIndex`, converts
                                // it the same way a template literal already
                                // does (`coerce_primitive_to_string`, which
                                // already handles `Json` via `JsonAsString`
                                // -- real JS `String()` semantics, not just
                                // "this JSON value happens to already be a
                                // string"), and concatenates via the same
                                // `__thaw_string_concat` a template literal's
                                // own lowering already uses.
                                let result_name =
                                    format!("__thaw_join_result_{}", self.next_binding);
                                self.next_binding += 1;
                                let index_name =
                                    format!("__thaw_join_index_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(result_name.clone(), HirType::Str);
                                self.scope.insert(index_name.clone(), HirType::F64);
                                let element_str = self.coerce_primitive_to_string(
                                    HirExpr::TypedIndex(
                                        Box::new(HirExpr::Var(receiver_name.clone())),
                                        Box::new(HirExpr::Var(index_name.clone())),
                                        HirType::Json,
                                    ),
                                )?;
                                let concat = |lhs: HirExpr, rhs: HirExpr| {
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                        vec![lhs, rhs],
                                    )
                                };
                                let loop_body = vec![
                                    HirStmt::If(
                                        HirExpr::BinOp(
                                            BinOp::Gt,
                                            Box::new(HirExpr::Var(index_name.clone())),
                                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                        ),
                                        vec![HirStmt::Expr(HirExpr::Assign(
                                            result_name.clone(),
                                            Box::new(concat(
                                                HirExpr::Var(result_name.clone()),
                                                HirExpr::Var(separator_name.clone()),
                                            )),
                                        ))],
                                        Vec::new(),
                                    ),
                                    HirStmt::Expr(HirExpr::Assign(
                                        result_name.clone(),
                                        Box::new(concat(
                                            HirExpr::Var(result_name.clone()),
                                            element_str,
                                        )),
                                    )),
                                    HirStmt::Expr(HirExpr::Assign(
                                        index_name.clone(),
                                        Box::new(HirExpr::BinOp(
                                            BinOp::Add,
                                            Box::new(HirExpr::Var(index_name.clone())),
                                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                                        )),
                                    )),
                                ];
                                HirExpr::Block(vec![
                                    HirStmt::Let(
                                        result_name.clone(),
                                        HirType::Str,
                                        HirExpr::Lit(HirLit::Str(String::new())),
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
                                            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                                                receiver_name.clone(),
                                            )))),
                                        ),
                                        loop_body,
                                    ),
                                    HirStmt::Return(Some(HirExpr::Var(result_name))),
                                ])
                            } else {
                                let builtin = match element.as_ref() {
                                    HirType::F64 => "__thaw_number_array_join",
                                    HirType::Str => "__thaw_string_array_join",
                                    HirType::Bool => "__thaw_bool_array_join",
                                    HirType::Object(_) => "__thaw_object_array_join",
                                    HirType::Optional(_)
                                    | HirType::Nullable(_)
                                    | HirType::Nullish(_)
                                    | HirType::Undefined
                                    | HirType::Null
                                    | HirType::Union(_) => "__thaw_tagged_array_join",
                                    other => {
                                        return Err(format!(
                                            "array join does not support element type {other:?}"
                                        ))
                                    }
                                };
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(builtin.to_string())),
                                    vec![
                                        HirExpr::Var(receiver_name.clone()),
                                        HirExpr::Var(separator_name.clone()),
                                    ],
                                )
                            };
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
                        // A dynamically-obtained (`Json`) array -- e.g. a
                        // dynamic call's result like
                        // `Array.prototype.map.call(...)` -- joins through the
                        // runtime's JS-semantics join.
                        HirType::Json | HirType::Dictionary(_) => Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_array_join".to_string())),
                            vec![HirExpr::Var(source_name.clone()), separator],
                        )),
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
                    let mut receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    if let HirType::Union(members) = &receiver_type {
                        if !members.iter().all(|member| matches!(member, HirType::Array(_))) {
                            return Err(format!("`.{}` requires an array union, got {receiver_type:?}", property.sym));
                        }
                        let needle = arguments[0].clone();
                        let needle_type = self.infer_expr_type(&needle)?;
                        let from_index = if let Some(argument) = arguments.get(1) {
                            self.coerce_primitive_to_number(argument.clone())?
                        } else if property.sym == *"lastIndexOf" {
                            HirExpr::Lit(HirLit::F64(f64::INFINITY))
                        } else {
                            HirExpr::Lit(HirLit::F64(0.0))
                        };
                        let receiver_name = format!("__thaw_union_search_array_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name = format!("__thaw_union_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let start_name = format!("__thaw_union_search_start_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(needle_name.clone(), needle_type.clone());
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let suffix = match property.sym.as_ref() {
                            "includes" => "includes",
                            "lastIndexOf" => "last_index_of",
                            _ => "index_of",
                        };
                        let mut branches = Vec::with_capacity(members.len());
                        for (index, member) in members.iter().enumerate() {
                            let HirType::Array(element) = member else { unreachable!() };
                            let array = HirExpr::UnionValue(Box::new(HirExpr::Var(receiver_name.clone())), index, members.clone());
                            let branch = if needle_type == HirType::Undefined {
                                let found = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_undefined_index_of".into())),
                                    vec![
                                        array,
                                        HirExpr::Var(start_name.clone()),
                                        HirExpr::Lit(HirLit::Bool(property.sym == *"lastIndexOf")),
                                        HirExpr::Lit(HirLit::F64(0.0)),
                                        HirExpr::Lit(HirLit::Bool(property.sym == *"includes")),
                                        HirExpr::Lit(HirLit::F64(0.0)),
                                    ],
                                );
                                if property.sym == *"includes" {
                                    HirExpr::BinOp(
                                        BinOp::GtEq,
                                        Box::new(found),
                                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                    )
                                } else {
                                    found
                                }
                            } else if element.as_ref() != &needle_type {
                                if property.sym == *"includes" {
                                    HirExpr::Lit(HirLit::Bool(false))
                                } else {
                                    HirExpr::Lit(HirLit::F64(-1.0))
                                }
                            } else {
                                let prefix = match element.as_ref() {
                                    HirType::F64 => "number",
                                    HirType::Str => "string",
                                    HirType::Bool => "bool",
                                    HirType::Object(_) => "object",
                                    other => return Err(format!("array search does not support element type {other:?}")),
                                };
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                                    vec![
                                        array,
                                        HirExpr::Var(needle_name.clone()),
                                        HirExpr::Var(start_name.clone()),
                                    ],
                                )
                            };
                            branches.push(branch);
                        }
                        let result = self.merge_union_array_method_branches(&receiver_name, members, branches)?;
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, needle_type, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
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
                    if *element == HirType::Json {
                        // A `Json` (`any`-typed) element can hold any
                        // primitive shape, so a plain-typed needle
                        // (`arr.includes(1)`) needs the same declared-
                        // type coercion any other call argument gets
                        // before it's comparable -- previously this
                        // fell through to the type-mismatch fast path
                        // below (silently `false`/`-1` whenever the
                        // needle's own natural type wasn't already
                        // `Json`) or, if it *was* `Json` already, the
                        // "array search does not support element type
                        // Json" error further down (`Json` was missing
                        // from the element-type dispatch entirely).
                        // Comparison itself is real Strict Equality
                        // (`__thaw_any_array_*`, `thaw_json_strict_
                        // equal` in thaw-std) instead of the pointer-
                        // identity search every other element type uses
                        // below, since a `Json` slot can hold a
                        // primitive value.
                        let needle = self.coerce_to_declared(&HirType::Json, needle)?;
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
                        self.scope.insert(needle_name.clone(), HirType::Json);
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_any_array_{suffix}"))),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(needle_name.clone()),
                                HirExpr::Var(start_name.clone()),
                            ],
                        );
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, HirType::Json, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if needle_type == HirType::Undefined
                        || (needle_type == HirType::Null
                            && (matches!(element.as_ref(), HirType::Null | HirType::Nullable(_) | HirType::Nullish(_))
                                || matches!(element.as_ref(), HirType::Union(members) if members.contains(&HirType::Null))))
                    {
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
                        let (kind, union_tag) = match (needle_type.clone(), element.as_ref()) {
                            (HirType::Null, HirType::Nullable(_)) => (4.0, 0.0),
                            (HirType::Null, HirType::Nullish(_)) => (5.0, 0.0),
                            (HirType::Null, HirType::Null) => (6.0, 0.0),
                            (HirType::Null, HirType::Union(members)) => (
                                8.0,
                                members.iter().position(|ty| ty == &HirType::Null)
                                    .expect("null union member") as f64,
                            ),
                            (_, HirType::Undefined) => (1.0, 0.0),
                            (_, HirType::Optional(_)) => (2.0, 0.0),
                            (_, HirType::Nullish(_)) => (3.0, 0.0),
                            (_, HirType::Union(members)) => members.iter()
                                .position(|ty| ty == &HirType::Undefined)
                                .map_or((0.0, 0.0), |index| (7.0, index as f64)),
                            _ => (0.0, 0.0),
                        };
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_undefined_index_of".into())),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(start_name.clone()),
                                HirExpr::Lit(HirLit::Bool(property.sym == *"lastIndexOf")),
                                HirExpr::Lit(HirLit::F64(kind)),
                                HirExpr::Lit(HirLit::Bool(
                                    property.sym == *"includes" && needle_type == HirType::Undefined,
                                )),
                                HirExpr::Lit(HirLit::F64(union_tag)),
                            ],
                        );
                        let result = if property.sym == *"includes" {
                            HirExpr::BinOp(
                                BinOp::GtEq,
                                Box::new(result),
                                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                            )
                        } else {
                            result
                        };
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, needle_type, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if matches!(element.as_ref(), HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) if payload.as_ref() == &needle_type)
                        || matches!(element.as_ref(), HirType::Union(members) if members.contains(&needle_type))
                    {
                        let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let start_name = format!("__thaw_search_start_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(needle_name.clone(), needle_type.clone());
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_tagged_array_index_of".into())),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(needle_name.clone()),
                                HirExpr::Var(start_name.clone()),
                                HirExpr::Lit(HirLit::Bool(property.sym == *"lastIndexOf")),
                                HirExpr::Lit(HirLit::Bool(property.sym == *"includes")),
                            ],
                        );
                        let result = if property.sym == *"includes" {
                            HirExpr::BinOp(
                                BinOp::GtEq,
                                Box::new(result),
                                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                            )
                        } else {
                            result
                        };
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, needle_type, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
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
