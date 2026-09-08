impl<'a> FnLowerer<'a> {
    fn lower_native_primitive_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"charAt" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "charAt receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.charAt")?;
                    if arguments.len() > 1 {
                        return Err("native `.charAt()` expects zero or one argument".into());
                    }
                    let index = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_char_at_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_char_at_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_char_at_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_char_at_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let empty = || HirExpr::Lit(HirLit::Str(String::new()));
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
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![HirStmt::Return(Some(empty()))],
                            Vec::new(),
                        ),
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_at".into())),
                                vec![var(&receiver_name), var(&normalized_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(empty()))],
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
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((index_name, HirType::F64, index));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"charCodeAt" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "charCodeAt receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.charCodeAt")?;
                    if arguments.len() > 1 {
                        return Err("native `.charCodeAt()` expects zero or one argument".into());
                    }
                    let index = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_char_code_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_char_code_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_char_code_at".to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(index_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((index_name, HirType::F64, index));
                    return self.wrap_call_argument_bindings(
                        result,
                        &bindings,
                    );
                }
                if property.sym == *"localeCompare" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "localeCompare receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.localeCompare")?;
                    let [other] = arguments.as_slice() else {
                        return Err("native `.localeCompare()` expects exactly one argument".into());
                    };
                    let other = self.coerce_primitive_to_string(other.clone())?;
                    let receiver_name =
                        format!("__thaw_locale_compare_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let other_name = format!("__thaw_locale_compare_other_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(other_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_locale_compare".to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(other_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((other_name, HirType::Str, other));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"normalize" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "normalize receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.normalize")?;
                    if arguments.len() > 1 {
                        return Err("native `.normalize()` expects zero or one argument".into());
                    }
                    let form = match arguments.first() {
                        Some(argument) => self.coerce_primitive_to_string(argument.clone())?,
                        None => HirExpr::Lit(HirLit::Str("NFC".into())),
                    };
                    let receiver_name =
                        format!("__thaw_normalize_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let form_name = format!("__thaw_normalize_form_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_normalize_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(form_name.clone(), HirType::Str);
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_normalize".into())),
                                vec![var(&receiver_name), var(&form_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "The normalization form should be one of NFC, NFD, NFKC, NFKD."
                                    .into(),
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
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((form_name, HirType::Str, form));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"split" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "split receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.split")?;
                    if arguments.len() > 2 {
                        return Err(
                            "native `.split()` expects zero, one or two arguments".into()
                        );
                    }
                    if arguments.is_empty() {
                        let receiver_name =
                            format!("__thaw_split_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayLit(vec![HirExpr::Var(receiver_name.clone())]),
                            &[(receiver_name, HirType::Str, receiver)],
                        );
                    }
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(&arguments[0])? == regex_type {
                        let pattern = arguments[0].clone();
                        let limit = match arguments.get(1) {
                            Some(argument) => {
                                self.coerce_primitive_to_number(argument.clone())?
                            }
                            None => HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                        };
                        let receiver_name =
                            format!("__thaw_regex_split_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let pattern_name =
                            format!("__thaw_regex_split_pattern_{}", self.next_binding);
                        self.next_binding += 1;
                        let limit_name =
                            format!("__thaw_regex_split_limit_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(pattern_name.clone(), regex_type.clone());
                        self.scope.insert(limit_name.clone(), HirType::F64);
                        let source = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "source".to_string(),
                        );
                        let flags = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "flags".to_string(),
                        );
                        let array_type = HirType::Array(Box::new(HirType::Str));
                        let raw_name = format!("__thaw_regex_split_raw_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(raw_name.clone(), array_type.clone());
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(
                                raw_name.clone(),
                                array_type.clone(),
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_regex_split".to_string())),
                                    vec![
                                        HirExpr::Var(receiver_name.clone()),
                                        source,
                                        flags,
                                        HirExpr::Var(limit_name.clone()),
                                    ],
                                ),
                            ),
                            HirStmt::If(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                    vec![HirExpr::Var(raw_name.clone())],
                                ),
                                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                    "invalid regular expression".into(),
                                )))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::Var(raw_name))),
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
                                array_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((pattern_name, regex_type, pattern));
                        bindings.push((limit_name, HirType::F64, limit));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let separator = self.coerce_primitive_to_string(arguments[0].clone())?;
                    let limit = match arguments.get(1) {
                        Some(argument) => self.coerce_primitive_to_number(argument.clone())?,
                        None => HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                    };
                    let receiver_name = format!("__thaw_split_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let separator_name =
                        format!("__thaw_split_separator_{}", self.next_binding);
                    self.next_binding += 1;
                    let limit_name = format!("__thaw_split_limit_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(separator_name.clone(), HirType::Str);
                    self.scope.insert(limit_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_split".into())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(separator_name.clone()),
                            HirExpr::Var(limit_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((separator_name, HirType::Str, separator));
                    bindings.push((limit_name, HirType::F64, limit));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "replace" | "replaceAll") {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "replace receiver")?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("String.{}", property.sym),
                    )?;
                    let [search, replacement] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly two arguments",
                            property.sym
                        ));
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(search)? == regex_type {
                        let pattern = search.clone();
                        let replacement = self.coerce_primitive_to_string(replacement.clone())?;
                        let intrinsic = if property.sym == *"replace" {
                            "__thaw_regex_replace"
                        } else {
                            "__thaw_regex_replace_all"
                        };
                        let receiver_name =
                            format!("__thaw_regex_replace_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let pattern_name =
                            format!("__thaw_regex_replace_pattern_{}", self.next_binding);
                        self.next_binding += 1;
                        let replacement_name =
                            format!("__thaw_regex_replace_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(pattern_name.clone(), regex_type.clone());
                        self.scope.insert(replacement_name.clone(), HirType::Str);
                        let source = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "source".to_string(),
                        );
                        let flags = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "flags".to_string(),
                        );
                        let raw_name = format!("__thaw_regex_replace_raw_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(raw_name.clone(), HirType::Str);
                        let error_message = if property.sym == *"replace" {
                            "invalid regular expression"
                        } else {
                            "replaceAll must be called with a global RegExp"
                        };
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(
                                raw_name.clone(),
                                HirType::Str,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(intrinsic.to_string())),
                                    vec![
                                        HirExpr::Var(receiver_name.clone()),
                                        source,
                                        flags,
                                        HirExpr::Var(replacement_name.clone()),
                                    ],
                                ),
                            ),
                            HirStmt::If(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                    vec![HirExpr::Var(raw_name.clone())],
                                ),
                                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                    error_message.into(),
                                )))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::Var(raw_name))),
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
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((pattern_name, regex_type, pattern));
                        bindings.push((replacement_name, HirType::Str, replacement));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let search = self.coerce_primitive_to_string(search.clone())?;
                    let replacement = self.coerce_primitive_to_string(replacement.clone())?;
                    let intrinsic = if property.sym == *"replace" {
                        "__thaw_string_replace"
                    } else {
                        "__thaw_string_replace_all"
                    };
                    let receiver_name = format!("__thaw_replace_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let search_name = format!("__thaw_replace_search_{}", self.next_binding);
                    self.next_binding += 1;
                    let replacement_name =
                        format!("__thaw_replace_replacement_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(search_name.clone(), HirType::Str);
                    self.scope.insert(replacement_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(search_name.clone()),
                            HirExpr::Var(replacement_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((search_name, HirType::Str, search));
                    bindings.push((replacement_name, HirType::Str, replacement));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"test" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let regex_type = regex_object_type();
                    self.expect_type(&regex_type, &receiver, "RegExp.test receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "RegExp.test")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.test()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_string(value.clone())?;
                    let receiver_name = format!("__thaw_regex_test_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_regex_test_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), regex_type.clone());
                    self.scope.insert(value_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let last_index = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "lastIndex".to_string(),
                    );
                    let set_last_index = |value: HirExpr| {
                        HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(var(&receiver_name)),
                            regex_type.clone(),
                            "lastIndex".to_string(),
                            Box::new(value),
                        ))
                    };
                    let is_global = HirExpr::Call(
                        Box::new(var("__thaw_string_includes")),
                        vec![
                            flags.clone(),
                            HirExpr::Lit(HirLit::Str("g".to_string())),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    let is_sticky = HirExpr::Call(
                        Box::new(var("__thaw_string_includes")),
                        vec![
                            flags.clone(),
                            HirExpr::Lit(HirLit::Str("y".to_string())),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    // Like `.exec()`, `.test()` only threads `lastIndex`
                    // state for a global or sticky pattern -- it delegates
                    // to the same `RegExpExec` algorithm in the
                    // specification, just discarding the match array for a
                    // boolean. A non-global, non-sticky pattern keeps using
                    // the cheaper `thaw_regex_test` (no array allocation)
                    // and never touches `lastIndex`.
                    let array_type = HirType::Array(Box::new(HirType::Str));
                    let raw_name = format!("__thaw_regex_test_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(raw_name.clone(), array_type.clone());
                    let start_name = format!("__thaw_regex_test_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let next_name = format!("__thaw_regex_test_next_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(next_name.clone(), HirType::F64);
                    let stateful_branch = vec![
                        HirStmt::Let(start_name.clone(), HirType::F64, last_index.clone()),
                        HirStmt::Let(
                            raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(var("__thaw_regex_exec")),
                                vec![
                                    source.clone(),
                                    flags.clone(),
                                    var(&value_name),
                                    var(&start_name),
                                ],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(var("__thaw_array_is_null")),
                                vec![var(&raw_name)],
                            ),
                            vec![
                                set_last_index(HirExpr::Lit(HirLit::F64(0.0))),
                                HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false)))),
                            ],
                            vec![
                                HirStmt::Let(
                                    next_name.clone(),
                                    HirType::F64,
                                    HirExpr::Call(
                                        Box::new(var("__thaw_regex_exec_advance")),
                                        vec![
                                            var(&value_name),
                                            source.clone(),
                                            flags.clone(),
                                            var(&start_name),
                                        ],
                                    ),
                                ),
                                set_last_index(var(&next_name)),
                                HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                            ],
                        ),
                    ];
                    let non_stateful_branch = vec![HirStmt::Return(Some(HirExpr::Call(
                        Box::new(var("__thaw_regex_test")),
                        vec![source.clone(), flags.clone(), var(&value_name)],
                    )))];
                    let body = HirExpr::Block(vec![HirStmt::If(
                        is_global,
                        stateful_branch.clone(),
                        vec![HirStmt::If(is_sticky, stateful_branch, non_stateful_branch)],
                    )]);
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
                            HirType::Bool,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, regex_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::Str, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"exec" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let regex_type = regex_object_type();
                    self.expect_type(&regex_type, &receiver, "RegExp.exec receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "RegExp.exec")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.exec()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_string(value.clone())?;
                    let receiver_name = format!("__thaw_regex_exec_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_regex_exec_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), regex_type.clone());
                    self.scope.insert(value_name.clone(), HirType::Str);
                    let array_type = HirType::Array(Box::new(HirType::Str));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let last_index = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "lastIndex".to_string(),
                    );
                    let set_last_index = |value: HirExpr| {
                        HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(var(&receiver_name)),
                            regex_type.clone(),
                            "lastIndex".to_string(),
                            Box::new(value),
                        ))
                    };
                    let is_global = HirExpr::Call(
                        Box::new(var("__thaw_string_includes")),
                        vec![
                            flags.clone(),
                            HirExpr::Lit(HirLit::Str("g".to_string())),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    let is_sticky = HirExpr::Call(
                        Box::new(var("__thaw_string_includes")),
                        vec![
                            flags.clone(),
                            HirExpr::Lit(HirLit::Str("y".to_string())),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    // `exec` only threads `lastIndex` state for a global or
                    // sticky pattern (matching the specification's
                    // `RegExpBuiltinExec`) -- otherwise it always searches
                    // from the start and leaves `lastIndex` untouched, same
                    // as `RegExp.prototype.test`'s own simplification.
                    let raw_name = format!("__thaw_regex_exec_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(raw_name.clone(), array_type.clone());
                    let start_name = format!("__thaw_regex_exec_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let next_name = format!("__thaw_regex_exec_next_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(next_name.clone(), HirType::F64);
                    let stateful_branch = vec![
                        HirStmt::Let(start_name.clone(), HirType::F64, last_index.clone()),
                        HirStmt::Let(
                            raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(var("__thaw_regex_exec")),
                                vec![
                                    source.clone(),
                                    flags.clone(),
                                    var(&value_name),
                                    var(&start_name),
                                ],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(var("__thaw_array_is_null")),
                                vec![var(&raw_name)],
                            ),
                            vec![
                                set_last_index(HirExpr::Lit(HirLit::F64(0.0))),
                                HirStmt::Return(Some(HirExpr::OptionalNone(array_type.clone()))),
                            ],
                            vec![
                                HirStmt::Let(
                                    next_name.clone(),
                                    HirType::F64,
                                    HirExpr::Call(
                                        Box::new(var("__thaw_regex_exec_advance")),
                                        vec![
                                            var(&value_name),
                                            source.clone(),
                                            flags.clone(),
                                            var(&start_name),
                                        ],
                                    ),
                                ),
                                set_last_index(var(&next_name)),
                                HirStmt::Return(Some(HirExpr::OptionalSome(
                                    Box::new(var(&raw_name)),
                                    array_type.clone(),
                                ))),
                            ],
                        ),
                    ];
                    let non_stateful_raw_name =
                        format!("__thaw_regex_exec_raw_ns_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(non_stateful_raw_name.clone(), array_type.clone());
                    let non_stateful_branch = vec![
                        HirStmt::Let(
                            non_stateful_raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(var("__thaw_regex_exec")),
                                vec![
                                    source.clone(),
                                    flags.clone(),
                                    var(&value_name),
                                    HirExpr::Lit(HirLit::F64(0.0)),
                                ],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(var("__thaw_array_is_null")),
                                vec![var(&non_stateful_raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                array_type.clone(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&non_stateful_raw_name)),
                            array_type.clone(),
                        ))),
                    ];
                    let body = HirExpr::Block(vec![HirStmt::If(
                        is_global,
                        stateful_branch.clone(),
                        vec![HirStmt::If(is_sticky, stateful_branch, non_stateful_branch)],
                    )]);
                    let result_type = HirType::Optional(Box::new(array_type));
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
                    let mut bindings = vec![(receiver_name, regex_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::Str, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"match" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "match receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.match")?;
                    let [pattern] = arguments.as_slice() else {
                        return Err("native `.match()` expects exactly one argument".into());
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(pattern)? != regex_type {
                        return Err("native `.match()` requires a RegExp argument".into());
                    }
                    let pattern = pattern.clone();
                    let receiver_name = format!("__thaw_match_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let pattern_name = format!("__thaw_match_pattern_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_match_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(pattern_name.clone(), regex_type.clone());
                    let array_type = HirType::Array(Box::new(HirType::Str));
                    self.scope.insert(raw_name.clone(), array_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_match".into())),
                                vec![var(&receiver_name), source, flags],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                array_type.clone(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&raw_name)),
                            array_type.clone(),
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(array_type));
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
                    bindings.push((pattern_name, regex_type, pattern));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"matchAll" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "matchAll receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.matchAll")?;
                    let [pattern] = arguments.as_slice() else {
                        return Err("native `.matchAll()` expects exactly one argument".into());
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(pattern)? != regex_type {
                        return Err("native `.matchAll()` requires a RegExp argument".into());
                    }
                    let pattern = pattern.clone();
                    let receiver_name = format!("__thaw_match_all_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let pattern_name = format!("__thaw_match_all_pattern_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_match_all_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(pattern_name.clone(), regex_type.clone());
                    let matches_type = HirType::Array(Box::new(HirType::Array(Box::new(HirType::Str))));
                    self.scope.insert(raw_name.clone(), matches_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            matches_type.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_match_all".into())),
                                vec![var(&receiver_name), source, flags],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "String.prototype.matchAll must be called with a global RegExp"
                                    .into(),
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
                            matches_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((pattern_name, regex_type, pattern));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"search" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "search receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.search")?;
                    let [pattern] = arguments.as_slice() else {
                        return Err("native `.search()` expects exactly one argument".into());
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(pattern)? != regex_type {
                        return Err("native `.search()` requires a RegExp argument".into());
                    }
                    let pattern = pattern.clone();
                    let receiver_name = format!("__thaw_search_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let pattern_name = format!("__thaw_search_pattern_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(pattern_name.clone(), regex_type.clone());
                    let source = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(pattern_name.clone())),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(pattern_name.clone())),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_regex_search".to_string())),
                        vec![HirExpr::Var(receiver_name.clone()), source, flags],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((pattern_name, regex_type, pattern));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
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

