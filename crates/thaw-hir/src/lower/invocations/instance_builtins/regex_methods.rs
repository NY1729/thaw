impl<'a> FnLowerer<'a> {
    fn lower_native_regex_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
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
        unreachable!("instance builtin category was checked before lowering")
    }
}

