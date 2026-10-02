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
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                if matches!(
                    property.sym.as_ref(),
                    "toLocaleString" | "toLocaleDateString" | "toLocaleTimeString"
                ) {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == date_object_type() {
                        let timestamp = HirExpr::PropAccess(
                            Box::new(receiver),
                            receiver_type,
                            "timestamp".to_string(),
                        );
                        // thaw's calendar math is UTC-only and has no locale
                        // database, so the locale date/time spellings reuse
                        // the corresponding UTC rendering.
                        let intrinsic = match property.sym.as_ref() {
                            "toLocaleDateString" => "__thaw_date_to_date_string",
                            "toLocaleTimeString" => "__thaw_date_to_time_string",
                            _ => "__thaw_date_to_string",
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![timestamp],
                        ));
                    }
                    if property.sym != *"toLocaleString" {
                        return Err(format!(
                            "native `.{}()` is only supported on a Date",
                            property.sym
                        ));
                    }
                    // String/Number/Boolean/Array: approximate locale
                    // formatting with the locale-independent string form (no
                    // thousands separators or locale digits).
                    return self.coerce_primitive_to_string(receiver);
                }
                if property.sym == *"toString" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
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
                    // `RegExp.prototype.toString()` is `"/" + source + "/" +
                    // flags`, not the generic `[object Object]` an object
                    // receiver otherwise coerces to.
                    if receiver_type == regex_object_type() {
                        if !call.args.is_empty() {
                            return Err("native `.toString()` expects no arguments".into());
                        }
                        let receiver_name = format!("__thaw_regex_to_string_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), receiver_type.clone());
                        let bound = HirExpr::Var(receiver_name.clone());
                        let source = HirExpr::PropAccess(
                            Box::new(bound.clone()),
                            receiver_type.clone(),
                            "source".to_string(),
                        );
                        let flags = HirExpr::PropAccess(
                            Box::new(bound.clone()),
                            receiver_type.clone(),
                            "flags".to_string(),
                        );
                        let open = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".into())),
                            vec![HirExpr::Lit(HirLit::Str("/".to_string())), source],
                        );
                        let close = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".into())),
                            vec![open, HirExpr::Lit(HirLit::Str("/".to_string()))],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".into())),
                            vec![close, flags],
                        );
                        return self.wrap_call_argument_bindings(
                            result,
                            &[(receiver_name, receiver_type, receiver)],
                        );
                    }
                    // `Symbol.prototype.toString()` is `Symbol(description)`.
                    // `coerce_primitive_to_string`'s `Symbol` arm is
                    // deliberately identity (a symbol used as an object key
                    // needs its raw representation), so a *method call* needs
                    // the dedicated rendering instead.
                    if receiver_type == HirType::Symbol {
                        if !call.args.is_empty() {
                            return Err("native `.toString()` expects no arguments".into());
                        }
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_symbol_to_string".to_string())),
                            vec![receiver],
                        ));
                    }
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
                        let radix = if self.infer_expr_type(radix)? == HirType::Undefined {
                            HirExpr::Lit(HirLit::F64(10.0))
                        } else {
                            self.coerce_primitive_to_number(radix.clone())?
                        };
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
                    if receiver_type == HirType::I64 && !call.args.is_empty() {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "BigInt.toString")?;
                        let [radix] = arguments.as_slice() else {
                            return Err("native `.toString()` expects zero or one argument".into());
                        };
                        // Explicit undefined selects radix 10. Preserve the raw argument
                        // binding (including its side effects) before numeric coercion.
                        let radix_type = self.infer_expr_type(radix)?;
                        let missing = match &radix_type {
                            HirType::Undefined => Some(HirExpr::Lit(HirLit::Bool(true))),
                            HirType::Json => Some(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                                vec![radix.clone()],
                            )),
                            HirType::JsValue => Some(self.dynamic_value_is_undefined(radix.clone())),
                            HirType::Optional(payload) => Some(HirExpr::OptionalIsNone(
                                Box::new(radix.clone()), payload.as_ref().clone(),
                            )),
                            HirType::Nullish(payload) => Some(HirExpr::NullishIsUndefined(
                                Box::new(radix.clone()), payload.as_ref().clone(),
                            )),
                            _ => None,
                        };
                        let radix = if let HirType::Union(members) = &radix_type {
                            // A general union keeps its member tag. Project exactly
                            // the selected member; only undefined gets the default.
                            let mut branches = Vec::with_capacity(members.len());
                            for (index, member) in members.iter().enumerate() {
                                let selected = HirExpr::UnionValue(
                                    Box::new(radix.clone()), index, members.clone(),
                                );
                                branches.push(if *member == HirType::Undefined {
                                    HirExpr::Lit(HirLit::F64(10.0))
                                } else {
                                    self.coerce_primitive_to_number(selected)?
                                });
                            }
                            let mut result = branches.pop().expect("union has members");
                            for index in (0..branches.len()).rev() {
                                result = HirExpr::Conditional(
                                    Box::new(HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::UnionTag(
                                            Box::new(radix.clone()), members.clone(),
                                        )),
                                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                    )),
                                    Box::new(branches[index].clone()),
                                    Box::new(result),
                                    HirType::F64,
                                );
                            }
                            result
                        } else {
                            let converted = self.coerce_primitive_to_number(radix.clone())?;
                            if let Some(missing) = missing {
                                HirExpr::Conditional(
                                    Box::new(missing),
                                    Box::new(HirExpr::Lit(HirLit::F64(10.0))),
                                    Box::new(converted),
                                    HirType::F64,
                                )
                            } else {
                                converted
                            }
                        };
                        let receiver_name = format!("__thaw_bigint_radix_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::I64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_i64_to_radix_string".into())),
                            vec![HirExpr::Var(receiver_name.clone()), radix],
                        );
                        let mut bindings = vec![(receiver_name, HirType::I64, receiver)];
                        bindings.extend(spread_bindings);
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
                    // `coerce_primitive_to_string`'s own `HirType::Str`
                    // case is plain identity (correct for ordinary
                    // string coercion, e.g. `"" + str`, which must never
                    // rewrite an already-`Str` value) -- but a *method
                    // call* `.toString()` needs the error-aware
                    // rendering real JS gives every `Error.prototype.
                    // toString()` (`"Name: message"` for a tagged value,
                    // an untagged plain string unchanged), the same
                    // rendering `String(value)`'s own `HirType::Str`
                    // special case already applies (see
                    // `lower/invocations/calls.rs`) -- without this,
                    // `e.toString()` and `String(e)` would disagree,
                    // even though real JS guarantees they're identical.
                    if receiver_type == HirType::Str {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_to_string".to_string())),
                            vec![receiver],
                        ));
                    }
                    return self.coerce_primitive_to_string(receiver);
                }
                if property.sym == *"valueOf" {
                    if !call.args.is_empty() {
                        return Err("native `.valueOf()` expects no arguments".into());
                    }
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == date_object_type() {
                        return Ok(HirExpr::PropAccess(
                            Box::new(receiver),
                            receiver_type,
                            "timestamp".to_string(),
                        ));
                    }
                    if matches!(receiver_type, HirType::Array(_))
                        || matches!(&receiver_type, HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))))
                    {
                        return Ok(receiver);
                    }
                    if !matches!(receiver_type, HirType::F64 | HirType::Str | HirType::Bool) {
                        return Err(format!(
                            "native `.valueOf()` requires a primitive or array receiver, got {receiver_type:?}"
                        ));
                    }
                    return Ok(receiver);
                }
        unreachable!("instance builtin category was checked before lowering")
    }

    /// `obj.hasOwnProperty(key)` -- the instance-method spelling of
    /// `Object.hasOwn` (which static_builtins.rs already lowers the same
    /// way): a fixed native object compares the key against its known
    /// field names, while a `Json`/dictionary object defers to
    /// `__thaw_json_has_own` at runtime.
    fn lower_native_has_own_property(
        &mut self,
        member: &MemberExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (arguments, bindings) =
            self.lower_native_spread_values(&call.args, "hasOwnProperty")?;
        let [key_value] = arguments.as_slice() else {
            return Err("`hasOwnProperty` expects exactly one argument".into());
        };
        let mut receiver = self.lower_required_member_receiver(&member.obj, "hasOwnProperty")?;
        let mut receiver_type = self.infer_expr_type(&receiver)?;
        if let HirType::Union(members) = &receiver_type {
            if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                (receiver, receiver_type) = self.lower_union_array_sequence(receiver, members)?;
            }
        }
        let result = if matches!(&member.prop, MemberProp::Ident(property) if property.sym == "propertyIsEnumerable")
            && matches!(receiver_type, HirType::Array(_))
        {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_property_is_enumerable".into())),
                vec![receiver, self.coerce_primitive_to_string(key_value.clone())?],
            )
        } else {
            self.lower_has_own_value(receiver, key_value.clone())?
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// Whether `receiver` has `key` as an own property. A fixed native
    /// object compares the key against its known field names; a
    /// `Json`/dictionary object defers to `__thaw_json_has_own`. Shared by
    /// `obj.hasOwnProperty(key)`, static `Object.hasOwn`, and
    /// `Reflect.has`.
    fn lower_has_own_value(
        &mut self,
        receiver: HirExpr,
        key_value: HirExpr,
    ) -> Result<HirExpr, String> {
        let key_value = self.coerce_primitive_to_string(key_value)?;
        let mut receiver = receiver;
        let mut receiver_type = self.infer_expr_type(&receiver)?;
        if let HirType::Union(members) = &receiver_type {
            if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                (receiver, receiver_type) = self.lower_union_array_sequence(receiver, members)?;
            }
        }
        if matches!(receiver_type, HirType::Array(_)) {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_has_own".into())),
                vec![receiver, key_value],
            ));
        }
        if matches!(receiver_type, HirType::Json | HirType::Dictionary(_)) {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_has_own".to_string())),
                vec![receiver, key_value],
            ));
        }
        let HirType::Object(fields) = &receiver_type else {
            return Err(format!(
                "`hasOwn` currently requires a fixed object or dictionary, got {receiver_type:?}"
            ));
        };
        let field_names = fields
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let object_name = format!("__thaw_has_own_object_{}", self.next_binding);
        self.next_binding += 1;
        let key_name = format!("__thaw_has_own_key_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(object_name.clone(), receiver_type.clone());
        self.scope.insert(key_name.clone(), HirType::Str);
        let mut comparisons = field_names.into_iter().map(|field| {
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Var(key_name.clone())),
                Box::new(HirExpr::Lit(HirLit::Str(field))),
            )
        });
        let mut result = comparisons
            .next()
            .unwrap_or(HirExpr::Lit(HirLit::Bool(false)));
        for comparison in comparisons {
            result = self.lower_logical_expr(result, comparison, false)?;
        }
        self.wrap_call_argument_bindings(
            result,
            &[
                (object_name, receiver_type, receiver),
                (key_name, HirType::Str, key_value),
            ],
        )
    }
}
