impl<'a> FnLowerer<'a> {
    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
                s.value.to_string_lossy().into_owned(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Lit(Lit::Null(_)) => Ok(HirExpr::Lit(HirLit::Null)),
            Expr::Lit(Lit::Regex(regex)) => Ok(HirExpr::ObjectLit(vec![
                ("source".to_string(), HirExpr::Lit(HirLit::Str(regex.exp.to_string()))),
                (
                    "flags".to_string(),
                    HirExpr::Lit(HirLit::Str(regex.flags.to_string())),
                ),
                ("lastIndex".to_string(), HirExpr::Lit(HirLit::F64(0.0))),
            ])),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                if !self.scope.contains_key(&name) {
                    if let Some(signature) = self.signatures.get(&name) {
                        if signature.is_extern || !signature.generic_type_params.is_empty() {
                            return Err(format!(
                                "function value `{name}` needs a monomorphic native implementation"
                            ));
                        }
                        let ret = if signature.is_async {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        };
                        return Ok(HirExpr::FunctionRef(
                            name,
                            signature.params.clone(),
                            ret,
                        ));
                    }
                }
                if !self.scope.contains_key(&name) && !self.signatures.contains_key(&name) {
                    match ident.sym.as_ref() {
                        "NaN" => return Ok(HirExpr::Lit(HirLit::F64(f64::NAN))),
                        "Infinity" => return Ok(HirExpr::Lit(HirLit::F64(f64::INFINITY))),
                        "undefined" => return Ok(HirExpr::Lit(HirLit::Undefined)),
                        _ => {}
                    }
                }
                if let Some((allowed, elements)) = self.union_narrowings.get(&name) {
                    if let [index] = allowed.as_slice() {
                        return Ok(HirExpr::UnionValue(
                            Box::new(HirExpr::Var(name)),
                            *index,
                            elements.clone(),
                        ));
                    }
                }
                match self
                    .narrowings
                    .get(&name)
                    .map(|payload| (payload, 0))
                    .or_else(|| {
                        self.nullable_narrowings
                            .get(&name)
                            .map(|payload| (payload, 1))
                    })
                    .or_else(|| {
                        self.nullish_narrowings
                            .get(&name)
                            .map(|payload| (payload, 2))
                    })
                {
                    Some((payload, 1)) => Ok(HirExpr::NullableValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some((payload, 0)) => Ok(HirExpr::OptionalValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some((payload, 2)) => Ok(HirExpr::NullishValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some(_) => unreachable!(),
                    None => Ok(HirExpr::Var(name)),
                }
            }

            Expr::This(_) => {
                if self.unbound_this_context {
                    return Ok(HirExpr::Lit(HirLit::Undefined));
                }
                if self.class_static_context {
                    let class = self
                        .class_context
                        .as_deref()
                        .ok_or("static `this` is missing its class context")?;
                    let constructor = class_constructor_symbol(class);
                    let signature = self.signatures.get(&constructor).ok_or_else(|| {
                        format!("static `this` constructor `{constructor}` is not declared")
                    })?;
                    let result = self
                        .interfaces
                        .get(class)
                        .cloned()
                        .ok_or_else(|| format!("static `this` class `{class}` has no layout"))?;
                    return Ok(HirExpr::FunctionRef(
                        constructor,
                        signature.params.clone(),
                        result,
                    ));
                }
                let name = self.resolve_binding("this");
                if self.scope.contains_key(&name) {
                    Ok(HirExpr::Var(name))
                } else {
                    Err("`this` is only available inside a native class constructor or method".into())
                }
            }
            Expr::Paren(paren) => self.lower_expr(&paren.expr),
            Expr::TsAs(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsSatisfies(satisfies) => self.lower_satisfies(satisfies),
            Expr::TsNonNull(assertion) => self.lower_non_null_assertion(assertion),
            Expr::TsConstAssertion(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsInstantiation(instantiation) => {
                self.lower_generic_instantiation_expression(instantiation)
            }

            Expr::Seq(sequence) => {
                let mut values = sequence
                    .exprs
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect::<Result<Vec<_>, _>>()?;
                let last = values
                    .pop()
                    .ok_or("sequence expression must contain at least one value")?;
                let result_type = self.infer_expr_type(&last)?;
                let mut statements = values
                    .into_iter()
                    .map(HirStmt::Expr)
                    .collect::<Vec<_>>();
                if result_type == HirType::Void {
                    statements.push(HirStmt::Expr(last));
                } else {
                    statements.push(HirStmt::Return(Some(last)));
                }
                let body = HirExpr::Block(statements);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter_map(|name| {
                        self.scope
                            .get(&name)
                            .cloned()
                            .map(|ty| HirParam { name, ty })
                    })
                    .collect();
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        Vec::new(),
                        result_type,
                        Box::new(body),
                    )),
                    Vec::new(),
                ))
            }

            Expr::Tpl(template) => {
                let mut parts = Vec::with_capacity(template.quasis.len() + template.exprs.len());
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|cooked| cooked.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    if !text.is_empty() {
                        parts.push(HirExpr::Lit(HirLit::Str(text)));
                    }
                    if let Some(expression) = template.exprs.get(index) {
                        let mut value = self.lower_expr(expression)?;
                        value = self.coerce_primitive_to_string(value)?;
                        parts.push(value);
                    }
                }
                let mut parts = parts.into_iter();
                let Some(mut result) = parts.next() else {
                    return Ok(HirExpr::Lit(HirLit::Str(String::new())));
                };
                for part in parts {
                    result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![result, part],
                    );
                }
                Ok(result)
            }

            Expr::TaggedTpl(tagged) => {
                if let Expr::Member(member) = tagged.tag.as_ref() {
                    if let (Expr::Ident(object), MemberProp::Ident(property)) =
                        (member.obj.as_ref(), &member.prop)
                    {
                        if object.sym == *"String" && property.sym == *"raw" {
                            let template = &tagged.tpl;
                            let mut parts = Vec::with_capacity(
                                template.quasis.len() + template.exprs.len(),
                            );
                            for (index, quasi) in template.quasis.iter().enumerate() {
                                let text = quasi.raw.to_string();
                                if !text.is_empty() {
                                    parts.push(HirExpr::Lit(HirLit::Str(text)));
                                }
                                if let Some(expression) = template.exprs.get(index) {
                                    let mut value = self.lower_expr(expression)?;
                                    value = self.coerce_primitive_to_string(value)?;
                                    parts.push(value);
                                }
                            }
                            let mut parts = parts.into_iter();
                            let Some(mut result) = parts.next() else {
                                return Ok(HirExpr::Lit(HirLit::Str(String::new())));
                            };
                            for part in parts {
                                result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                    vec![result, part],
                                );
                            }
                            return Ok(result);
                        }
                    }
                }
                let template = &tagged.tpl;
                let mut strings = Vec::with_capacity(template.quasis.len());
                for quasi in &template.quasis {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|cooked| cooked.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    strings.push(HirExpr::Lit(HirLit::Str(text)));
                }
                let mut args = vec![HirExpr::ArrayLit(strings)];
                for expression in &template.exprs {
                    args.push(self.lower_expr(expression)?);
                }
                let tag = self.lower_expr(&tagged.tag)?;
                Ok(HirExpr::Call(Box::new(tag), args))
            }

            Expr::Bin(bin) => {
                if bin.op == BinaryOp::InstanceOf {
                    let Expr::Ident(class) = bin.right.as_ref() else {
                        return Err(
                            "native `instanceof` requires a class identifier on the right"
                                .into(),
                        );
                    };
                    if !self
                        .signatures
                        .contains_key(&class_constructor_symbol(class.sym.as_ref()))
                    {
                        return Err(format!(
                            "native `instanceof` right operand `{}` is not a known class",
                            class.sym
                        ));
                    }
                    let value = self.lower_expr(&bin.left)?;
                    let value_type = self.infer_expr_type(&value)?;
                    let result = class_type_has_identity(&value_type, class.sym.as_ref());
                    let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), value_type.clone());
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(result)),
                        &[(name, value_type, value)],
                    );
                }
                let mut lhs = self.lower_expr(&bin.left)?;
                let rhs_narrowing = self
                    .optional_undefined_narrowing(&bin.left)
                    .filter(|(_, _, present, _)| {
                        (bin.op == BinaryOp::LogicalAnd && *present)
                            || (bin.op == BinaryOp::LogicalOr && !*present)
                    })
                    .map(|(name, payload, _, nullable)| (name, payload, nullable));
                let rhs_union_narrowing = self.union_narrowing(&bin.left).and_then(
                    |(targets, equal, complement)| {
                        let required_truth = match bin.op {
                            BinaryOp::LogicalAnd => true,
                            BinaryOp::LogicalOr => false,
                            _ => return None,
                        };
                        Some(
                            targets
                                .into_iter()
                                .map(|target| UnionNarrowingTarget {
                                    matching: if required_truth == equal {
                                        target.matching.clone()
                                    } else if complement {
                                        target
                                            .allowed
                                            .iter()
                                            .filter(|index| !target.matching.contains(index))
                                            .copied()
                                            .collect()
                                    } else {
                                        target.allowed.clone()
                                    },
                                    ..target
                                })
                                .collect::<Vec<_>>(),
                        )
                    },
                );
                let mut rhs = self
                    .lower_expr_with_union_narrowing(
                        &bin.right,
                        rhs_union_narrowing.as_deref(),
                        rhs_narrowing.as_ref(),
                    )?;
                let mut bindings = Vec::new();
                if !matches!(
                    bin.op,
                    BinaryOp::In
                        | BinaryOp::LogicalAnd
                        | BinaryOp::LogicalOr
                        | BinaryOp::NullishCoalescing
                ) && contains_await(&rhs)
                {
                    let lhs_type = self.infer_expr_type(&lhs)?;
                    let lhs_name = format!("__thaw_binary_left_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(lhs_name.clone(), lhs_type.clone());
                    bindings.push((lhs_name.clone(), lhs_type, lhs));
                    lhs = HirExpr::Var(lhs_name);

                    let rhs_type = self.infer_expr_type(&rhs)?;
                    let rhs_name = format!("__thaw_binary_right_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(rhs_name.clone(), rhs_type.clone());
                    bindings.push((rhs_name.clone(), rhs_type, rhs));
                    rhs = HirExpr::Var(rhs_name);
                }
                let value = match bin.op {
                    BinaryOp::In => {
                        let right_type = self.infer_expr_type(&rhs)?;
                        if matches!(right_type, HirType::Json | HirType::Dictionary(_)) {
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            let left_name = format!("__thaw_in_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(left_name.clone(), HirType::Str);
                            let right_name = format!("__thaw_in_object_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(right_name.clone(), right_type.clone());
                            self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_has_own".into())),
                                    vec![
                                        HirExpr::Var(right_name.clone()),
                                        HirExpr::Var(left_name.clone()),
                                    ],
                                ),
                                &[
                                    (left_name, HirType::Str, lhs),
                                    (right_name, right_type, rhs),
                                ],
                            )?
                        } else {
                            let HirType::Object(fields) = right_type else {
                                return Err("`in` currently requires a fixed-shape object".into());
                            };
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            let field_names = fields
                                .iter()
                                .map(|(field, _)| field.clone())
                                .collect::<Vec<_>>();
                            let left_name = format!("__thaw_in_key_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(left_name.clone(), HirType::Str);
                        let right_type = HirType::Object(fields);
                        let right_name = format!("__thaw_in_object_{}", self.next_binding);
                        self.next_binding += 1;
                            self.scope.insert(right_name.clone(), right_type.clone());
                            let mut comparisons = field_names.into_iter().map(|field| {
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Var(left_name.clone())),
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
                                (left_name, HirType::Str, lhs),
                                (right_name, right_type, rhs),
                            ],
                        )?
                        }
                    }
                    BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                        self.lower_logical_expr(
                            lhs,
                            rhs,
                            bin.op == BinaryOp::LogicalAnd,
                        )?
                    }
                    BinaryOp::NullishCoalescing => self.lower_nullish_coalescing(lhs, rhs)?,
                    BinaryOp::Lt => self.lower_relational(lhs, rhs, BinOp::Lt)?,
                    BinaryOp::Gt => self.lower_relational(lhs, rhs, BinOp::Gt)?,
                    BinaryOp::LtEq => self.lower_relational(lhs, rhs, BinOp::LtEq)?,
                    BinaryOp::GtEq => self.lower_relational(lhs, rhs, BinOp::GtEq)?,
                    BinaryOp::EqEqEq => self
                        .lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                        .unwrap_or(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(lhs),
                            Box::new(rhs),
                        )),
                    BinaryOp::NotEqEq => {
                        let equality = self
                            .lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                            .unwrap_or(HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(lhs),
                                Box::new(rhs),
                            ));
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(equality),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )
                    }
                    BinaryOp::EqEq => self.lower_loose_equality(lhs, rhs)?,
                    BinaryOp::NotEq => HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(self.lower_loose_equality(lhs, rhs)?),
                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                    ),
                    BinaryOp::Add
                        if self.infer_expr_type(&lhs)? == HirType::Str
                            || self.infer_expr_type(&rhs)? == HirType::Str =>
                    {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![
                                self.coerce_primitive_to_string(lhs)?,
                                self.coerce_primitive_to_string(rhs)?,
                            ],
                        )
                    }
                    other => HirExpr::BinOp(
                        lower_bin_op(other)?,
                        Box::new(lhs),
                        Box::new(rhs),
                    ),
                };
                self.infer_expr_type(&value)?;
                self.wrap_call_argument_bindings(value, &bindings)
            }

            Expr::Unary(unary) => {
                if unary.op == UnaryOp::Delete {
                    let Expr::Member(member) = unary.arg.as_ref() else {
                        return Err("native `delete` requires a JSON or dictionary property".into());
                    };
                    let object = self.lower_expr(&member.obj)?;
                    let object_type = self.infer_expr_type(&object)?;
                    if !matches!(object_type, HirType::Json | HirType::Dictionary(_)) {
                        return Err(format!(
                            "native `delete` requires a JSON or dictionary receiver, got {object_type:?}"
                        ));
                    }
                    let key = match &member.prop {
                        MemberProp::Ident(property) => {
                            HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                        }
                        MemberProp::Computed(computed) => {
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                        MemberProp::PrivateName(_) => {
                            return Err("native `delete` does not support private properties".into())
                        }
                    };
                    return Ok(HirExpr::JsonDelete(Box::new(object), Box::new(key)));
                }
                let value = self.lower_expr(&unary.arg)?;
                let lowered = match unary.op {
                    UnaryOp::Minus => {
                        self.expect_type(&HirType::F64, &value, "unary minus")?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_neg".to_string())),
                            vec![value],
                        )
                    }
                    UnaryOp::Plus => {
                        self.expect_type(&HirType::F64, &value, "unary plus")?;
                        value
                    }
                    UnaryOp::Bang => {
                        self.expect_type(&HirType::Bool, &value, "logical not")?;
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )
                    }
                    UnaryOp::Tilde => {
                        self.expect_type(&HirType::F64, &value, "bitwise not")?;
                        HirExpr::BinOp(
                            BinOp::BitXor,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )
                    }
                    UnaryOp::TypeOf => {
                        let operand_type = if let HirExpr::Var(name) = &value {
                            self.generic_arrows
                                .get(name)
                                .map(|arrow| {
                                    HirType::Function(
                                        vec![HirType::Dynamic; arrow.params.len()],
                                        Box::new(HirType::Dynamic),
                                    )
                                })
                                .or_else(|| {
                                    self.generic_named_templates.get(name).and_then(|target| {
                                        self.signatures.get(target).map(|signature| {
                                            HirType::Function(
                                                signature.params.clone(),
                                                Box::new(signature.ret.clone()),
                                            )
                                        })
                                    })
                                })
                                .or_else(|| {
                                    self.signatures.get(name).map(|signature| {
                                        let ret = if signature.is_async {
                                            HirType::Promise(Box::new(signature.ret.clone()))
                                        } else {
                                            signature.ret.clone()
                                        };
                                        HirType::Function(signature.params.clone(), Box::new(ret))
                                    })
                                })
                        } else {
                            None
                        }
                        .map(Ok)
                        .unwrap_or_else(|| self.infer_expr_type(&value))?;
                        if let HirType::Union(elements) = &operand_type {
                            let parameter = format!("__thaw_typeof_union_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let mut branches = vec![HirStmt::Return(Some(HirExpr::Lit(
                                HirLit::Str(
                                    native_typeof_name(elements.last().ok_or(
                                        "`typeof` cannot inspect an empty union",
                                    )?)
                                    .ok_or_else(|| {
                                        format!(
                                            "`typeof` union member has no runtime category: {:?}",
                                            elements.last().unwrap()
                                        )
                                    })?
                                    .into(),
                                ),
                            )))];
                            for (index, member) in elements
                                .iter()
                                .enumerate()
                                .rev()
                                .skip(1)
                            {
                                let type_name = native_typeof_name(member).ok_or_else(|| {
                                    format!(
                                        "`typeof` union member has no runtime category: {member:?}"
                                    )
                                })?;
                                branches = vec![HirStmt::If(
                                    HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::UnionTag(
                                            Box::new(bound.clone()),
                                            elements.clone(),
                                        )),
                                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        type_name.into(),
                                    ))))],
                                    branches,
                                )];
                            }
                            let result = HirExpr::Block(branches);
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            );
                        }
                        if let HirType::Nullish(payload) = &operand_type {
                            let Some(type_name) = native_typeof_name(payload) else {
                                return Err(format!(
                                    "`typeof` nullish payload has no supported runtime category: {payload:?}"
                                ));
                            };
                            let parameter =
                                format!("__thaw_typeof_nullish_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let result = HirExpr::Block(vec![HirStmt::If(
                                HirExpr::NullishIsUndefined(
                                    Box::new(bound.clone()),
                                    payload.as_ref().clone(),
                                ),
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    "undefined".into(),
                                ))))],
                                vec![HirStmt::If(
                                    HirExpr::NullishIsNull(
                                        Box::new(bound),
                                        payload.as_ref().clone(),
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        "object".into(),
                                    ))))],
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        type_name.into(),
                                    ))))],
                                )],
                            )]);
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            );
                        }
                        if let Some((payload, absent, nullable)) = match &operand_type {
                            HirType::Optional(payload) => {
                                Some((payload.as_ref(), "undefined", false))
                            }
                            HirType::Nullable(payload) => {
                                Some((payload.as_ref(), "object", true))
                            }
                            _ => None,
                        } {
                            let Some(type_name) = native_typeof_name(payload) else {
                                return Err(format!(
                                    "`typeof` optional payload has no supported runtime category: {payload:?}"
                                ));
                            };
                            let parameter = format!("__thaw_typeof_optional_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let is_none = if nullable {
                                HirExpr::NullableIsNone(Box::new(bound), payload.clone())
                            } else {
                                HirExpr::OptionalIsNone(Box::new(bound), payload.clone())
                            };
                            let result = HirExpr::Block(vec![HirStmt::If(
                                is_none,
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    absent.into(),
                                ))))],
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    type_name.into(),
                                ))))],
                            )]);
                            self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            )?
                        } else {
                        let Some(type_name) = native_typeof_name(&operand_type) else {
                            return Err(format!(
                                "`typeof` requires one statically known runtime category, got {operand_type:?}"
                            ));
                        };
                        if matches!(&value, HirExpr::Var(_) | HirExpr::FunctionRef(..))
                            && matches!(operand_type, HirType::Function(_, _))
                        {
                            HirExpr::Lit(HirLit::Str(type_name.into()))
                        } else {
                        let parameter = format!("__thaw_typeof_{}", self.next_binding);
                        self.next_binding += 1;
                        HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                Vec::new(),
                                vec![HirParam {
                                    name: parameter,
                                    ty: operand_type,
                                }],
                                HirType::Str,
                                Box::new(HirExpr::Lit(HirLit::Str(type_name.into()))),
                            )),
                            vec![value],
                            )
                        }
                        }
                    }
                    UnaryOp::Void => {
                        let body = HirExpr::Block(vec![HirStmt::Expr(value)]);
                        let mut referenced = BTreeSet::new();
                        collect_referenced_bindings(&body, &mut referenced);
                        let captures = referenced
                            .into_iter()
                            .filter_map(|name| {
                                self.scope
                                    .get(&name)
                                    .cloned()
                                    .map(|ty| HirParam { name, ty })
                            })
                            .collect();
                        HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                HirType::Void,
                                Box::new(body),
                            )),
                            Vec::new(),
                        )
                    }
                    other => return Err(format!("unsupported unary operator {other:?}")),
                };
                self.infer_expr_type(&lowered)?;
                Ok(lowered)
            }

            Expr::Cond(conditional) => {
                let undefined_default_binding = if let Expr::Bin(test) = conditional.test.as_ref() {
                    if test.op == BinaryOp::EqEqEq && !self.scope.contains_key("undefined") {
                        match (test.left.as_ref(), test.right.as_ref(), conditional.alt.as_ref()) {
                            (Expr::Ident(value), Expr::Ident(undefined), Expr::Ident(alternate))
                                if undefined.sym == *"undefined"
                                    && value.sym == alternate.sym =>
                            {
                                Some(alternate)
                            }
                            (Expr::Ident(undefined), Expr::Ident(value), Expr::Ident(alternate))
                                if undefined.sym == *"undefined"
                                    && value.sym == alternate.sym =>
                            {
                                Some(alternate)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(binding) = undefined_default_binding {
                    let lhs = self.lower_expr(&Expr::Ident(binding.clone()))?;
                    let rhs = self.lower_expr(&conditional.cons)?;
                    return self.lower_undefined_default(lhs, rhs);
                }
                let test = self.lower_expr(&conditional.test)?;
                self.expect_type(&HirType::Bool, &test, "conditional expression test")?;
                let mut consequent = self.lower_expr(&conditional.cons)?;
                let mut alternate = self.lower_expr(&conditional.alt)?;
                let consequent_type = self.infer_expr_type(&consequent)?;
                let alternate_type = self.infer_expr_type(&alternate)?;
                let result_type = if consequent_type == alternate_type {
                    consequent_type
                } else if !matches!(consequent_type, HirType::Union(_))
                    && !matches!(alternate_type, HirType::Union(_))
                {
                    let result = match (&consequent_type, &alternate_type) {
                        (HirType::Undefined, payload) | (payload, HirType::Undefined) => {
                            HirType::Optional(Box::new(payload.clone()))
                        }
                        (HirType::Null, payload) | (payload, HirType::Null) => {
                            HirType::Nullable(Box::new(payload.clone()))
                        }
                        _ => HirType::Union(vec![consequent_type, alternate_type]),
                    };
                    consequent = self.coerce_to_declared(&result, consequent)?;
                    alternate = self.coerce_to_declared(&result, alternate)?;
                    result
                } else {
                    return Err(format!(
                        "conditional expression branches have incompatible types {consequent_type:?} and {alternate_type:?}"
                    ));
                };
                let body = HirExpr::Block(vec![HirStmt::If(
                    test,
                    vec![HirStmt::Return(Some(consequent))],
                    vec![HirStmt::Return(Some(alternate))],
                )]);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter_map(|name| {
                        self.scope
                            .get(&name)
                            .cloned()
                            .map(|ty| HirParam { name, ty })
                    })
                    .collect();
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        Vec::new(),
                        result_type,
                        Box::new(body),
                    )),
                    Vec::new(),
                ))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => self.lower_optional_member_read(member),
                OptChainBase::Call(call) => self.lower_optional_call(call),
            },

            Expr::Arrow(arrow) => self.lower_arrow(arrow),
            Expr::Fn(function) => {
                let arrow = function_expression_as_arrow(function)?;
                self.lower_arrow(&arrow)
            }

            Expr::Array(array_lit) => {
                if array_lit
                    .elems
                    .iter()
                    .all(|element| element.as_ref().is_some_and(|element| element.spread.is_none()))
                {
                    let values = array_lit
                        .elems
                        .iter()
                        .map(|element| {
                            self.lower_expr(
                                &element.as_ref().expect("checked array element").expr,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let preserve_order = values.iter().any(contains_await);
                    let mut bindings = Vec::new();
                    let values = if preserve_order {
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(position, value)| {
                                let ty = self.infer_expr_type(&value)?;
                                let name = format!(
                                    "__thaw_array_element_{}_{}",
                                    position, self.next_binding
                                );
                                self.next_binding += 1;
                                self.scope.insert(name.clone(), ty.clone());
                                bindings.push((name.clone(), ty, value));
                                Ok(HirExpr::Var(name))
                            })
                            .collect::<Result<Vec<_>, String>>()?
                    } else {
                        values
                    };
                    let value = HirExpr::ArrayLit(values);
                    self.infer_expr_type(&value)?;
                    return self.wrap_call_argument_bindings(value, &bindings);
                }
                let mut parts = Vec::new();
                let mut pending = Vec::new();
                let mut element_type: Option<HirType> = None;
                for element in &array_lit.elems {
                    let Some(element) = element else {
                        return Err("elisions are not supported in array literals".into());
                    };
                    let mut value = self.lower_expr(&element.expr)?;
                    if element.spread.is_some() {
                        if !pending.is_empty() {
                            parts.push(HirExpr::ArrayLit(std::mem::take(&mut pending)));
                        }
                        let HirType::Array(spread_element) = self.infer_expr_type(&value)? else {
                            return Err("array spread source must be a typed array".into());
                        };
                        if let Some(expected) = &element_type {
                            if expected != spread_element.as_ref() {
                                return Err(format!(
                                    "array spread element type {:?} does not match {expected:?}",
                                    spread_element
                                ));
                            }
                        } else {
                            element_type = Some(spread_element.as_ref().clone());
                        }
                        parts.push(value);
                    } else {
                        let actual = self.infer_expr_type(&value)?;
                        if let Some(expected) = &element_type {
                            if expected != &actual {
                                if matches!(expected, HirType::Union(members) if members.contains(&actual))
                                {
                                    value = self.coerce_to_declared(expected, value)?;
                                } else {
                                    return Err(format!(
                                        "array element type {actual:?} does not match {expected:?}"
                                    ));
                                }
                            }
                        } else {
                            element_type = Some(actual);
                        }
                        pending.push(value);
                    }
                }
                if !pending.is_empty() {
                    parts.push(HirExpr::ArrayLit(pending));
                }
                let element_type = element_type.unwrap_or(HirType::F64);
                if !parts.iter().any(contains_await) {
                    return Ok(HirExpr::ArrayConcat(parts, element_type));
                }
                let parts = parts
                    .into_iter()
                    .flat_map(|part| match part {
                        HirExpr::ArrayLit(values) => values
                            .into_iter()
                            .map(|value| HirExpr::ArrayLit(vec![value]))
                            .collect(),
                        other => vec![other],
                    })
                    .collect::<Vec<_>>();
                let mut bindings = Vec::with_capacity(parts.len());
                let mut ordered = Vec::with_capacity(parts.len());
                for (position, part) in parts.into_iter().enumerate() {
                    let ty = self.infer_expr_type(&part)?;
                    let name = format!("__thaw_array_part_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, part));
                    ordered.push(HirExpr::Var(name));
                }
                self.wrap_call_argument_bindings(
                    HirExpr::ArrayConcat(ordered, element_type),
                    &bindings,
                )
            }

            Expr::Object(obj_lit) => self.lower_object_lit(obj_lit),

            Expr::Member(member) => self.lower_member_read(member),

            Expr::SuperProp(member) => {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property access is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                if self.class_static_context {
                    let storage = class_static_field_symbol(&base_name, &property);
                    if self.scope.contains_key(&storage) {
                        return Ok(HirExpr::Var(storage));
                    }
                }
                let symbol = class_getter_symbol(
                    &base_name,
                    &property,
                    self.class_static_context,
                );
                if !self.signatures.contains_key(&symbol) {
                    return Err(format!(
                        "base class `{base_name}` has no getter `{property}`"
                    ));
                }
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var(symbol)),
                    if self.class_static_context {
                        Vec::new()
                    } else {
                        vec![HirExpr::Var(self.resolve_binding("this"))]
                    },
                ))
            }

            Expr::Assign(assign) => self.lower_assign(assign),

            Expr::Update(update) => self.lower_update(update),

            Expr::Await(await_expr) => {
                let value = self.lower_expr(&await_expr.arg)?;
                if let HirType::Promise(resolved) = self.infer_expr_type(&value)? {
                    if *resolved == HirType::Void {
                        Ok(HirExpr::Await(Box::new(value)))
                    } else {
                        Ok(HirExpr::AwaitPromise(Box::new(value), *resolved))
                    }
                } else {
                    Ok(HirExpr::Await(Box::new(value)))
                }
            }

            Expr::New(new_expr) => {
                if let Expr::Ident(class) = new_expr.callee.as_ref() {
                    if class.sym == *"RegExp" {
                        let args = new_expr.args.clone().unwrap_or_default();
                        if !(1..=2).contains(&args.len()) {
                            return Err("`new RegExp()` expects one or two arguments".into());
                        }
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(
                                "`new RegExp()` does not support spread arguments".into()
                            );
                        }
                        let source = self.lower_expr(&args[0].expr)?;
                        let source = self.coerce_primitive_to_string(source)?;
                        let flags = if let Some(argument) = args.get(1) {
                            let flags = self.lower_expr(&argument.expr)?;
                            self.coerce_primitive_to_string(flags)?
                        } else {
                            HirExpr::Lit(HirLit::Str(String::new()))
                        };
                        return Ok(HirExpr::ObjectLit(vec![
                            ("source".to_string(), source),
                            ("flags".to_string(), flags),
                            ("lastIndex".to_string(), HirExpr::Lit(HirLit::F64(0.0))),
                        ]));
                    }
                    if matches!(class.sym.as_ref(), "Map" | "Set" | "WeakMap" | "WeakSet") {
                        // `new Map()`/`new Set()` are ambiguous on their
                        // own -- unlike `RegExp`/`Date`, `Map<K, V>`/
                        // `Set<T>` are genuinely generic, and this
                        // compiler has no contextual (expected-type)
                        // inference to recover `K`/`V` from an
                        // assignment target the way TypeScript itself
                        // does. Explicit type arguments are required.
                        //
                        // `WeakMap`/`WeakSet` reuse `HirType::Map`/`Set`
                        // outright rather than introducing distinct types
                        // (the same "avoid a new exhaustive-match blast
                        // radius" tradeoff `RegExp`/`Date` already made by
                        // reusing `Object`) -- the only real difference is
                        // that their key/element type must be a reference
                        // type (`weak_key_intrinsic_suffix` rejects
                        // `number`/`string`, matching the specification).
                        // Two things this does NOT enforce, unlike the
                        // specification: `.size`/`.keys()`/`.values()`/
                        // `.entries()`/`.forEach()`/`for...of` all still
                        // work (a real `WeakMap`/`WeakSet` isn't
                        // enumerable at all), and a key's entry is never
                        // actually reclaimed early just because it became
                        // otherwise unreachable -- this runtime has no
                        // fine-grained GC to do that with; every
                        // allocation lives until the whole arena resets at
                        // the next Lambda invocation regardless.
                        let is_weak = matches!(class.sym.as_ref(), "WeakMap" | "WeakSet");
                        let key_validator: fn(&HirType) -> Result<&'static str, String> =
                            if is_weak {
                                weak_key_intrinsic_suffix
                            } else {
                                map_key_intrinsic_suffix
                            };
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(format!(
                                "`new {}()` does not support spread arguments",
                                class.sym
                            ));
                        }
                        if args.len() > 1 {
                            return Err(format!(
                                "`new {}()` expects zero or one argument",
                                class.sym
                            ));
                        }
                        let params = new_expr
                            .type_args
                            .as_ref()
                            .map(|type_args| type_args.params.as_slice())
                            .unwrap_or_default();
                        if matches!(class.sym.as_ref(), "Map" | "WeakMap") {
                            let [key, value] = params else {
                                return Err(format!(
                                    "`new {}<K, V>()` requires explicit type arguments",
                                    class.sym
                                ));
                            };
                            let key_type =
                                lower_ts_type(key, self.interfaces, self.generic_interfaces)?;
                            key_validator(&key_type)?;
                            let value_type =
                                lower_ts_type(value, self.interfaces, self.generic_interfaces)?;
                            let map_type =
                                HirType::Map(Box::new(key_type.clone()), Box::new(value_type.clone()));
                            let Some(argument) = args.first() else {
                                return Ok(HirExpr::TypedClosure(
                                    map_type,
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                                        Vec::new(),
                                    )),
                                ));
                            };
                            let entries = self.lower_expr(&argument.expr)?;
                            let pair_type =
                                HirType::Tuple(vec![key_type.clone(), value_type.clone()]);
                            let entries_type = HirType::Array(Box::new(pair_type.clone()));
                            self.expect_type(
                                &entries_type,
                                &entries,
                                &format!("{} constructor entries", class.sym),
                            )?;
                            return self.lower_map_or_set_from_iterable(
                                entries,
                                entries_type,
                                map_type,
                                key_validator(&key_type)?,
                                key_type,
                                Some((value_type, pair_type)),
                            );
                        }
                        let [element] = params else {
                            return Err(format!(
                                "`new {}<T>()` requires an explicit type argument",
                                class.sym
                            ));
                        };
                        let element_type =
                            lower_ts_type(element, self.interfaces, self.generic_interfaces)?;
                        key_validator(&element_type)?;
                        let set_type = HirType::Set(Box::new(element_type.clone()));
                        let Some(argument) = args.first() else {
                            return Ok(HirExpr::TypedClosure(
                                set_type,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                                    Vec::new(),
                                )),
                            ));
                        };
                        let iterable = self.lower_expr(&argument.expr)?;
                        let iterable_type = HirType::Array(Box::new(element_type.clone()));
                        self.expect_type(
                            &iterable_type,
                            &iterable,
                            &format!("{} constructor iterable", class.sym),
                        )?;
                        return self.lower_map_or_set_from_iterable(
                            iterable,
                            iterable_type,
                            set_type,
                            key_validator(&element_type)?,
                            element_type,
                            None,
                        );
                    }
                    if class.sym == *"Date" {
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("`new Date()` does not support spread arguments".into());
                        }
                        if args.len() > 7 {
                            return Err("`new Date()` expects zero to seven arguments".into());
                        }
                        let timestamp = if args.len() >= 2 {
                            // `new Date(year, month, date?, hours?, minutes?,
                            // seconds?, ms?)`: identical to `Date.UTC` since
                            // "local" time is UTC here too, just wrapped as
                            // a `Date` instead of returned as a bare number.
                            const DEFAULTS: [f64; 7] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
                            let mut call_args = Vec::with_capacity(7);
                            for (index, default) in DEFAULTS.iter().enumerate() {
                                let value = if let Some(argument) = args.get(index) {
                                    let value = self.lower_expr(&argument.expr)?;
                                    self.coerce_primitive_to_number(value)?
                                } else {
                                    HirExpr::Lit(HirLit::F64(*default))
                                };
                                call_args.push(value);
                            }
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_utc".to_string())),
                                call_args,
                            )
                        } else if let Some(argument) = args.first() {
                            let value = self.lower_expr(&argument.expr)?;
                            if self.infer_expr_type(&value)? == HirType::Str {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_date_parse".to_string())),
                                    vec![value],
                                )
                            } else {
                                self.coerce_primitive_to_number(value)?
                            }
                        } else {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_now".to_string())),
                                Vec::new(),
                            )
                        };
                        return Ok(HirExpr::ObjectLit(vec![(
                            "timestamp".to_string(),
                            timestamp,
                        )]));
                    }
                    if matches!(
                        class.sym.as_ref(),
                        "Error"
                            | "TypeError"
                            | "RangeError"
                            | "SyntaxError"
                            | "ReferenceError"
                            | "EvalError"
                            | "URIError"
                    ) {
                        // `throw` already only ever unwinds a plain string
                        // (`catch` binds it as `HirType::Str`, see
                        // `lower/statements/lowering.rs`) -- there is no
                        // `Error` object, stack trace, or `.name`/`.message`
                        // field, and no support for a class extending one of
                        // these. `new Error(message)`/`new TypeError(...)`/
                        // etc. all just become `message` itself (defaulting
                        // to `""` when omitted), so `throw new Error("x")`
                        // works exactly like the already-supported
                        // `throw "x"`, without a new exception
                        // representation to plumb through every catch site.
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(format!(
                                "`new {}()` does not support spread arguments",
                                class.sym
                            ));
                        }
                        if args.len() > 1 {
                            return Err(format!(
                                "`new {}()` expects zero or one message argument",
                                class.sym
                            ));
                        }
                        return match args.first() {
                            Some(argument) => {
                                let message = self.lower_expr(&argument.expr)?;
                                self.coerce_primitive_to_string(message)
                            }
                            None => Ok(HirExpr::Lit(HirLit::Str(String::new()))),
                        };
                    }
                    let constructor = class_constructor_symbol(class.sym.as_ref());
                    if let Some(signature) = self.signatures.get(&constructor) {
                        if signature.abstract_class_constructor {
                            return Err(format!(
                                "cannot construct abstract class `{}`",
                                class.sym
                            ));
                        }
                        let mut callee = class.clone();
                        callee.sym = constructor.into();
                        return self.lower_call(&CallExpr {
                            span: new_expr.span,
                            ctxt: new_expr.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(callee))),
                            args: new_expr.args.clone().unwrap_or_default(),
                            type_args: new_expr.type_args.clone(),
                        });
                    }
                }
                self.lower_promise_new(new_expr)
            }

            other => Err(format!(
                "unsupported expression {other:?} (Phase 0/1/2 support literals, identifiers, binary ops, calls, arrays, objects, member access, assignment, ++/--)"
            )),
        }
    }

    /// Builds `new Map<K, V>(entries)`/`new Set<T>(iterable)`: a
    /// Lambda-IIFE that allocates an empty map (`__thaw_map_new`, the same
    /// as the no-argument constructor), loops over `source` by index, and
    /// calls the matching `__thaw_map_{suffix}_set` intrinsic once per
    /// element -- for a `Map`, each element is itself a `[key, value]`
    /// pair (`value_info` carries the value/pair types needed to read
    /// both halves); for a `Set`, each element IS the key, with the value
    /// half fixed at `0.0` (a dummy word, exactly like `.add()`).
    fn lower_map_or_set_from_iterable(
        &mut self,
        source: HirExpr,
        source_type: HirType,
        result_type: HirType,
        key_suffix: &'static str,
        key_type: HirType,
        value_info: Option<(HirType, HirType)>,
    ) -> Result<HirExpr, String> {
        let source_name = format!("__thaw_map_ctor_source_{}", self.next_binding);
        self.next_binding += 1;
        let map_name = format!("__thaw_map_ctor_map_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_map_ctor_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_map_ctor_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), source_type.clone());
        self.scope.insert(map_name.clone(), result_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        let var = |name: &str| HirExpr::Var(name.into());

        let (set_key_expr, set_value_expr, mut loop_body) =
            if let Some((value_type, pair_type)) = value_info {
                let pair_name = format!("__thaw_map_ctor_pair_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(pair_name.clone(), pair_type.clone());
                let pair_let = HirStmt::Let(
                    pair_name.clone(),
                    pair_type.clone(),
                    HirExpr::TypedIndex(
                        Box::new(var(&source_name)),
                        Box::new(var(&index_name)),
                        pair_type,
                    ),
                );
                let key_expr = HirExpr::TypedIndex(
                    Box::new(var(&pair_name)),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    key_type.clone(),
                );
                let value_expr = HirExpr::TypedIndex(
                    Box::new(var(&pair_name)),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    value_type,
                );
                (key_expr, value_expr, vec![pair_let])
            } else {
                let element_expr = HirExpr::TypedIndex(
                    Box::new(var(&source_name)),
                    Box::new(var(&index_name)),
                    key_type,
                );
                (element_expr, HirExpr::Lit(HirLit::F64(0.0)), Vec::new())
            };
        loop_body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
            vec![var(&map_name), set_key_expr, set_value_expr],
        )));
        loop_body.push(HirStmt::Expr(HirExpr::Assign(
            index_name.clone(),
            Box::new(HirExpr::BinOp(
                BinOp::Add,
                Box::new(var(&index_name)),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
            )),
        )));

        let body = HirExpr::Block(vec![
            HirStmt::Let(
                map_name.clone(),
                result_type.clone(),
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_map_new".to_string())), Vec::new()),
            ),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&source_name))),
            ),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&index_name)),
                    Box::new(var(&length_name)),
                ),
                loop_body,
            ),
            HirStmt::Return(Some(var(&map_name))),
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
                result_type,
                Box::new(body),
            )),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(result, &[(source_name, source_type, source)])
    }

}
