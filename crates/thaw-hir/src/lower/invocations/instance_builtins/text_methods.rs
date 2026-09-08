impl<'a> FnLowerer<'a> {
    fn lower_native_text_method(
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
        unreachable!("instance builtin category was checked before lowering")
    }
}

