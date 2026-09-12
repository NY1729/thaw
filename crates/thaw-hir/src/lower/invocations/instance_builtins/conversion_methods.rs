impl<'a> FnLowerer<'a> {
    fn lower_native_conversion_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"toJSON" {
                    if !call.args.is_empty() {
                        return Err("native `.toJSON()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.toJSON receiver")?;
                    let receiver_name = format!("__thaw_date_json_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_date_json_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let timestamp = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        date_type.clone(),
                        "timestamp".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_to_iso_string".into())),
                                vec![timestamp],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::NullableNone(HirType::Str)))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::NullableSome(
                            Box::new(var(&raw_name)),
                            HirType::Str,
                        ))),
                    ]);
                    let result_type = HirType::Nullable(Box::new(HirType::Str));
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
                    let bindings = vec![(receiver_name, date_type, receiver)];
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(
                    property.sym.as_ref(),
                    "toDateString" | "toTimeString" | "toUTCString"
                ) {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(
                        &date_type,
                        &receiver,
                        &format!("Date.{} receiver", property.sym),
                    )?;
                    let intrinsic = match property.sym.as_ref() {
                        "toDateString" => "__thaw_date_to_date_string",
                        "toTimeString" => "__thaw_date_to_time_string",
                        "toUTCString" => "__thaw_date_to_utc_string",
                        _ => unreachable!(),
                    };
                    let timestamp = HirExpr::PropAccess(
                        Box::new(receiver),
                        date_type,
                        "timestamp".to_string(),
                    );
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
                        vec![timestamp],
                    ));
                }
                if property.sym == *"equals" {
                    // `a.equals(b)` -- byte-for-byte equality, Buffer only
                    // (a plain array uses `===` / a loop). Both sides are
                    // read as raw byte arrays in the runtime.
                    let receiver = self.lower_expr(&member.obj)?;
                    if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
                        return Err(
                            "`.equals()` is only supported on a Buffer / Uint8Array".into(),
                        );
                    }
                    let [argument] = call.args.as_slice() else {
                        return Err("`Buffer.prototype.equals` expects exactly one argument".into());
                    };
                    let other = self.lower_expr(&argument.expr)?;
                    self.expect_type(
                        &HirType::Array(Box::new(HirType::F64)),
                        &other,
                        "`.equals()` argument",
                    )?;
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_bytes_equals".to_string())),
                        vec![receiver, other],
                    ));
                }
                if property.sym == *"toString" {
                    let receiver = self.lower_expr(&member.obj)?;
                    // A byte buffer decodes (`buf.toString("hex")`, default
                    // `utf8`) instead of comma-joining like a plain number
                    // array. `infer_expr_type` normalizes `Bytes` away, so
                    // ask for the un-normalized receiver type.
                    if self.infer_expr_type_inner(&receiver)? == HirType::Bytes {
                        let encoding = match call.args.first() {
                            Some(argument) => self.lower_expr(&argument.expr)?,
                            None => HirExpr::Lit(HirLit::Str("utf8".to_string())),
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_bytes_to_string".to_string())),
                            vec![receiver, encoding],
                        ));
                    }
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == HirType::Array(Box::new(HirType::F64))
                        && call.args.len() == 1
                    {
                        let encoding = self.lower_expr(&call.args[0].expr)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_bytes_to_string".to_string())),
                            vec![receiver, encoding],
                        ));
                    }
                    if receiver_type == HirType::F64 && !call.args.is_empty() {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Number.toString")?;
                        let [radix] = arguments.as_slice() else {
                            return Err("native `.toString()` expects zero or one argument".into());
                        };
                        let radix = self.coerce_primitive_to_number(radix.clone())?;
                        let receiver_name =
                            format!("__thaw_to_string_radix_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let radix_name =
                            format!("__thaw_to_string_radix_value_{}", self.next_binding);
                        self.next_binding += 1;
                        let normalized_name =
                            format!("__thaw_to_string_radix_normalized_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::F64);
                        self.scope.insert(radix_name.clone(), HirType::F64);
                        self.scope.insert(normalized_name.clone(), HirType::F64);
                        let number = |value| HirExpr::Lit(HirLit::F64(value));
                        let var = |name: &str| HirExpr::Var(name.into());
                        let assign = |value| {
                            HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                        };
                        let range_error = || {
                            HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "toString() radix argument must be between 2 and 36".into(),
                            )))
                        };
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(normalized_name.clone(), HirType::F64, var(&radix_name)),
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
                                    Box::new(number(2.0)),
                                ),
                                vec![range_error()],
                                Vec::new(),
                            ),
                            HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::Gt,
                                    Box::new(var(&normalized_name)),
                                    Box::new(number(36.0)),
                                ),
                                vec![range_error()],
                                Vec::new(),
                            ),
                            HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(var(&normalized_name)),
                                    Box::new(number(10.0)),
                                ),
                                vec![HirStmt::Return(Some(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_number_to_string".into())),
                                    vec![var(&receiver_name)],
                                )))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_number_to_radix_string".into())),
                                vec![var(&receiver_name), var(&normalized_name)],
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
                                HirType::Str,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        let mut bindings = vec![(receiver_name, HirType::F64, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((radix_name, HirType::F64, radix));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !call.args.is_empty() {
                        return Err("native `.toString()` does not accept arguments yet".into());
                    }
                    let date_type = date_object_type();
                    if receiver_type == date_type {
                        let timestamp = HirExpr::PropAccess(
                            Box::new(receiver),
                            date_type,
                            "timestamp".to_string(),
                        );
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_to_string".to_string())),
                            vec![timestamp],
                        ));
                    }
                    return self.coerce_primitive_to_string(receiver);
                }
                if property.sym == *"valueOf" {
                    if !call.args.is_empty() {
                        return Err("native `.valueOf()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == date_object_type() {
                        return Ok(HirExpr::PropAccess(
                            Box::new(receiver),
                            receiver_type,
                            "timestamp".to_string(),
                        ));
                    }
                    if !matches!(receiver_type, HirType::F64 | HirType::Str | HirType::Bool) {
                        return Err(format!(
                            "native `.valueOf()` requires a number, string or boolean receiver, got {receiver_type:?}"
                        ));
                    }
                    return Ok(receiver);
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}
