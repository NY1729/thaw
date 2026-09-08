impl<'a> FnLowerer<'a> {
    fn lower_native_date_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"getTime" {
                    if !call.args.is_empty() {
                        return Err("native `.getTime()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.getTime receiver")?;
                    return Ok(HirExpr::PropAccess(
                        Box::new(receiver),
                        date_type,
                        "timestamp".to_string(),
                    ));
                }
                if property.sym == *"setTime" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.setTime receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Date.setTime")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.setTime()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_number(value.clone())?;
                    let receiver_name =
                        format!("__thaw_date_set_time_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_date_set_time_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    self.scope.insert(value_name.clone(), HirType::F64);
                    let result = HirExpr::PropAssign(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        date_type.clone(),
                        "timestamp".to_string(),
                        Box::new(HirExpr::Var(value_name.clone())),
                    );
                    let mut bindings = vec![(receiver_name, date_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::F64, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                // Each entry pairs a setter's native intrinsic with, per
                // parameter position, the getter intrinsic that supplies
                // that position's default when the caller omits it -- `None`
                // marks the one leading parameter every setter requires.
                let date_setter = match property.sym.as_ref() {
                    "setFullYear" | "setUTCFullYear" => Some((
                        "__thaw_date_set_full_year",
                        vec![None, Some("__thaw_date_get_month"), Some("__thaw_date_get_date")],
                    )),
                    "setMonth" | "setUTCMonth" => Some((
                        "__thaw_date_set_month",
                        vec![None, Some("__thaw_date_get_date")],
                    )),
                    "setDate" | "setUTCDate" => Some(("__thaw_date_set_date", vec![None])),
                    "setHours" | "setUTCHours" => Some((
                        "__thaw_date_set_hours",
                        vec![
                            None,
                            Some("__thaw_date_get_minutes"),
                            Some("__thaw_date_get_seconds"),
                            Some("__thaw_date_get_milliseconds"),
                        ],
                    )),
                    "setMinutes" | "setUTCMinutes" => Some((
                        "__thaw_date_set_minutes",
                        vec![
                            None,
                            Some("__thaw_date_get_seconds"),
                            Some("__thaw_date_get_milliseconds"),
                        ],
                    )),
                    "setSeconds" | "setUTCSeconds" => Some((
                        "__thaw_date_set_seconds",
                        vec![None, Some("__thaw_date_get_milliseconds")],
                    )),
                    "setMilliseconds" | "setUTCMilliseconds" => {
                        Some(("__thaw_date_set_milliseconds", vec![None]))
                    }
                    _ => None,
                };
                if let Some((intrinsic, param_defaults)) = date_setter {
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(
                        &date_type,
                        &receiver,
                        &format!("Date.{} receiver", property.sym),
                    )?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Date.{}", property.sym),
                    )?;
                    if arguments.is_empty() || arguments.len() > param_defaults.len() {
                        return Err(format!(
                            "native `.{}()` expects one to {} argument(s)",
                            property.sym,
                            param_defaults.len()
                        ));
                    }
                    let receiver_name =
                        format!("__thaw_date_set_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), date_type.clone(), receiver)];
                    bindings.extend(spread_bindings);
                    let mut param_vars = Vec::new();
                    for (index, getter) in param_defaults.iter().enumerate() {
                        let value = if let Some(argument) = arguments.get(index) {
                            self.coerce_primitive_to_number(argument.clone())?
                        } else {
                            let getter =
                                getter.expect("the leading parameter is always required");
                            let timestamp = HirExpr::PropAccess(
                                Box::new(HirExpr::Var(receiver_name.clone())),
                                date_type.clone(),
                                "timestamp".to_string(),
                            );
                            HirExpr::Call(
                                Box::new(HirExpr::Var(getter.to_string())),
                                vec![timestamp],
                            )
                        };
                        let param_name =
                            format!("__thaw_date_set_arg_{index}_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(param_name.clone(), HirType::F64);
                        bindings.push((param_name.clone(), HirType::F64, value));
                        param_vars.push(HirExpr::Var(param_name));
                    }
                    let timestamp = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        date_type.clone(),
                        "timestamp".to_string(),
                    );
                    let mut call_args = vec![timestamp];
                    call_args.extend(param_vars);
                    let new_timestamp =
                        HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), call_args);
                    let result = HirExpr::PropAssign(
                        Box::new(HirExpr::Var(receiver_name)),
                        date_type,
                        "timestamp".to_string(),
                        Box::new(new_timestamp),
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                let date_getter_intrinsic = match property.sym.as_ref() {
                    "getFullYear" | "getUTCFullYear" => Some("__thaw_date_get_full_year"),
                    "getMonth" | "getUTCMonth" => Some("__thaw_date_get_month"),
                    "getDate" | "getUTCDate" => Some("__thaw_date_get_date"),
                    "getDay" | "getUTCDay" => Some("__thaw_date_get_day"),
                    "getHours" | "getUTCHours" => Some("__thaw_date_get_hours"),
                    "getMinutes" | "getUTCMinutes" => Some("__thaw_date_get_minutes"),
                    "getSeconds" | "getUTCSeconds" => Some("__thaw_date_get_seconds"),
                    "getMilliseconds" | "getUTCMilliseconds" => {
                        Some("__thaw_date_get_milliseconds")
                    }
                    _ => None,
                };
                if let Some(intrinsic) = date_getter_intrinsic {
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
                if property.sym == *"toISOString" {
                    if !call.args.is_empty() {
                        return Err("native `.toISOString()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.toISOString receiver")?;
                    let receiver_name =
                        format!("__thaw_date_iso_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_date_iso_raw_{}", self.next_binding);
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
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "Invalid time value".into(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(var(&raw_name))),
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
                    let bindings = vec![(receiver_name, date_type, receiver)];
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}

