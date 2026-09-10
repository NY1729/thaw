impl<'a> FnLowerer<'a> {
    fn truthiness_expr(&self, value: HirExpr, ty: &HirType) -> Result<HirExpr, String> {
        let false_lit = || HirExpr::Lit(HirLit::Bool(false));
        match ty {
            HirType::Bool => Ok(value),
            HirType::F64 => {
                let is_zero = HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(value.clone()),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                );
                let not_nan =
                    HirExpr::BinOp(BinOp::EqEqEq, Box::new(value.clone()), Box::new(value));
                Ok(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(is_zero),
                        Box::new(not_nan),
                    )),
                    Box::new(false_lit()),
                ))
            }
            HirType::Str => Ok(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(value),
                    Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
                )),
                Box::new(false_lit()),
            )),
            HirType::Json => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::JsValue => Ok(HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".into())),
                vec![value],
            )))),
            HirType::Null | HirType::Undefined => Ok(false_lit()),
            // Only ever seen here for a still-unresolved generic type
            // parameter placeholder during signature analysis, never a
            // real runtime value inside a fully-lowered statement body --
            // matches `expect_type`'s own long-standing `actual ==
            // HirType::Dynamic` bypass (this function's callers used to
            // route through that check instead, before `if`/`while`/
            // `do`/`for` conditions started calling this directly).
            HirType::Dynamic => Ok(value),
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Dictionary(_)
            | HirType::Promise(_)
            | HirType::Function(_, _)
            | HirType::CallableFunction(..) => Ok(HirExpr::Lit(HirLit::Bool(true))),
            // `T | null` / `T | undefined` (`T | null | undefined`):
            // falsy when absent, otherwise the payload's own truthiness.
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                let param = "__thaw_truthy".to_string();
                let is_none = match ty {
                    HirType::Optional(_) => HirExpr::OptionalIsNone(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                    HirType::Nullable(_) => HirExpr::NullableIsNone(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                    _ => HirExpr::NullishIsNone(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                };
                let payload_value = match ty {
                    HirType::Optional(_) => HirExpr::OptionalValue(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                    HirType::Nullable(_) => HirExpr::NullableValue(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                    _ => HirExpr::NullishValue(
                        Box::new(HirExpr::Var(param.clone())),
                        payload.as_ref().clone(),
                    ),
                };
                let payload_truthy = self.truthiness_expr(payload_value, payload)?;
                let adapter = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: param,
                        ty: ty.clone(),
                    }],
                    HirType::Bool,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::If(
                            is_none,
                            vec![HirStmt::Return(Some(false_lit()))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(payload_truthy)),
                    ])),
                );
                Ok(HirExpr::Call(Box::new(adapter), vec![value]))
            }
            // A tagged union: each member contributes its own truthiness.
            HirType::Union(members) => {
                let param = "__thaw_union_truthy".to_string();
                let mut statements = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    let member_truthy = self.truthiness_expr(
                        HirExpr::UnionValue(
                            Box::new(HirExpr::Var(param.clone())),
                            index,
                            members.clone(),
                        ),
                        member,
                    )?;
                    if index + 1 == members.len() {
                        statements.push(HirStmt::Return(Some(member_truthy)));
                    } else {
                        statements.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(HirExpr::Var(param.clone())),
                                    members.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Return(Some(member_truthy))],
                            Vec::new(),
                        ));
                    }
                }
                let adapter = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: param,
                        ty: ty.clone(),
                    }],
                    HirType::Bool,
                    Box::new(HirExpr::Block(statements)),
                );
                Ok(HirExpr::Call(Box::new(adapter), vec![value]))
            }
            other => Err(format!(
                "logical truthiness is not defined for native type {other:?}"
            )),
        }
    }

    fn lower_logical_expr(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        is_and: bool,
    ) -> Result<HirExpr, String> {
        let mut lhs = lhs;
        let mut lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type == HirType::JsValue && rhs_type != HirType::JsValue {
            lhs = HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                vec![lhs],
            );
            lhs_type = HirType::Json;
        }
        // `x && rhs` where `x: T | null` (`| undefined`): result is
        // `rhs | null` -- `null` when `x` is absent, otherwise `rhs`
        // (evaluated with `x` narrowed to `T`, as the caller already
        // lowered it). A present-but-falsy payload (`""`, `0`) is folded
        // into the absent case.
        if let HirType::Nullable(_) | HirType::Optional(_) | HirType::Nullish(_) = &lhs_type {
            return if is_and {
                self.lower_nullish_and(lhs, lhs_type, rhs, rhs_type)
            } else {
                self.lower_nullish_or(lhs, lhs_type, rhs, rhs_type)
            };
        }
        if lhs_type != rhs_type && lhs_type != HirType::Json {
            return Err(format!(
                "logical operands have incompatible types {lhs_type:?} and {rhs_type:?}"
            ));
        }
        let name = format!("__thaw_logical_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let condition = self.truthiness_expr(left.clone(), &lhs_type)?;
        let left_value = if lhs_type == HirType::Json && rhs_type != HirType::Json {
            self.coerce_to_declared(&rhs_type, left.clone())?
        } else {
            left
        };
        let (then_value, else_value) = if is_and {
            (rhs, left_value)
        } else {
            (left_value, rhs)
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(then_value))],
            vec![HirStmt::Return(Some(else_value))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
    }

    /// `x && rhs` for an `Optional`/`Nullable`/`Nullish` `x`: an IIFE
    /// that returns `rhs` (wrapped in `x`'s absent-form kind, payload =
    /// `rhs`'s type) when `x` is present-and-truthy, and the absent form
    /// otherwise.
    fn lower_nullish_and(
        &mut self,
        lhs: HirExpr,
        lhs_type: HirType,
        rhs: HirExpr,
        rhs_type: HirType,
    ) -> Result<HirExpr, String> {
        let name = format!("__thaw_nullish_and_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let condition = self.truthiness_expr(left, &lhs_type)?;
        let rhs = Box::new(rhs);
        let (present, none) = match &lhs_type {
            HirType::Optional(_) => (
                HirExpr::OptionalSome(rhs, rhs_type.clone()),
                HirExpr::OptionalNone(rhs_type),
            ),
            HirType::Nullable(_) => (
                HirExpr::NullableSome(rhs, rhs_type.clone()),
                HirExpr::NullableNone(rhs_type),
            ),
            _ => (
                HirExpr::NullishSome(rhs, rhs_type.clone()),
                HirExpr::NullishNull(rhs_type),
            ),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(present))],
            vec![HirStmt::Return(Some(none))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
    }

    /// `x || rhs` for an `Optional`/`Nullable`/`Nullish` `x`: an IIFE
    /// returning `x`'s payload when `x` is present-and-truthy, else `rhs`.
    /// Result type is the payload when `rhs` has that same type,
    /// otherwise the union of the two.
    fn lower_nullish_or(
        &mut self,
        lhs: HirExpr,
        lhs_type: HirType,
        rhs: HirExpr,
        rhs_type: HirType,
    ) -> Result<HirExpr, String> {
        let payload = match &lhs_type {
            HirType::Optional(p) | HirType::Nullable(p) | HirType::Nullish(p) => p.as_ref().clone(),
            _ => unreachable!(),
        };
        let name = format!("__thaw_nullish_or_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let payload_value = match &lhs_type {
            HirType::Optional(_) => {
                HirExpr::OptionalValue(Box::new(HirExpr::Var(name.clone())), payload.clone())
            }
            HirType::Nullable(_) => {
                HirExpr::NullableValue(Box::new(HirExpr::Var(name.clone())), payload.clone())
            }
            _ => HirExpr::NullishValue(Box::new(HirExpr::Var(name.clone())), payload.clone()),
        };
        let condition = self.truthiness_expr(HirExpr::Var(name.clone()), &lhs_type)?;
        let result_type = if rhs_type == payload {
            payload
        } else {
            HirType::Union(vec![payload, rhs_type])
        };
        let present = self.coerce_to_declared(&result_type, payload_value)?;
        let fallback = self.coerce_to_declared(&result_type, rhs)?;
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(present))],
            vec![HirStmt::Return(Some(fallback))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
    }

    fn lower_undefined_default(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        if lhs_type == HirType::Undefined {
            return Ok(rhs);
        }
        if let HirType::Union(elements) = &lhs_type {
            if !elements.contains(&HirType::Undefined) {
                return Ok(lhs);
            }
            let rhs_type = self.infer_expr_type(&rhs)?;
            let mut result_members = elements
                .iter()
                .filter(|element| element != &&HirType::Undefined)
                .cloned()
                .collect::<Vec<_>>();
            if !result_members.contains(&rhs_type) {
                result_members.push(rhs_type);
            }
            let result_type = match result_members.as_slice() {
                [member] => member.clone(),
                members => HirType::Union(members.to_vec()),
            };
            let name = format!("__thaw_default_union_left_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), lhs_type.clone());
            let left = HirExpr::Var(name.clone());
            let mut body = Vec::new();
            for (index, element) in elements.iter().enumerate() {
                let value = if element == &HirType::Undefined {
                    self.coerce_to_declared(&result_type, rhs.clone())?
                } else {
                    self.coerce_to_declared(
                        &result_type,
                        HirExpr::UnionValue(Box::new(left.clone()), index, elements.clone()),
                    )?
                };
                if index + 1 == elements.len() {
                    body.push(HirStmt::Return(Some(value)));
                } else {
                    body.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(left.clone()), elements.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        vec![HirStmt::Return(Some(value))],
                        Vec::new(),
                    ));
                }
            }
            return self
                .wrap_call_argument_bindings(HirExpr::Block(body), &[(name, lhs_type, lhs)]);
        }
        match lhs_type.clone() {
            HirType::Optional(payload) => {
                self.expect_type(payload.as_ref(), &rhs, "destructuring default")?;
                let name = format!("__thaw_default_left_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), lhs_type.clone());
                let left = HirExpr::Var(name.clone());
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(left.clone()), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(rhs))],
                    vec![HirStmt::Return(Some(HirExpr::OptionalValue(
                        Box::new(left),
                        payload.as_ref().clone(),
                    )))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
            }
            HirType::Nullish(payload) => {
                self.expect_type(payload.as_ref(), &rhs, "destructuring default")?;
                let result_type = HirType::Nullable(payload.clone());
                let name = format!("__thaw_default_left_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), lhs_type.clone());
                let left = HirExpr::Var(name.clone());
                let result = HirExpr::Block(vec![
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(
                            Box::new(left.clone()),
                            payload.as_ref().clone(),
                        ),
                        vec![HirStmt::Return(Some(HirExpr::NullableSome(
                            Box::new(rhs),
                            payload.as_ref().clone(),
                        )))],
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsNull(Box::new(left.clone()), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::NullableNone(
                            payload.as_ref().clone(),
                        )))],
                        Vec::new(),
                    ),
                    HirStmt::Return(Some(HirExpr::NullableSome(
                        Box::new(HirExpr::NullishValue(
                            Box::new(left),
                            payload.as_ref().clone(),
                        )),
                        payload.as_ref().clone(),
                    ))),
                ]);
                let result = self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])?;
                self.expect_type(&result_type, &result, "destructuring default result")?;
                Ok(result)
            }
            _ => Ok(lhs),
        }
    }

    fn lower_nullish_coalescing(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        if let HirType::Union(elements) = &lhs_type {
            if !elements
                .iter()
                .any(|element| matches!(element, HirType::Null | HirType::Undefined))
            {
                return Ok(lhs);
            }
            let rhs_type = self.infer_expr_type(&rhs)?;
            let mut result_members = elements
                .iter()
                .filter(|element| !matches!(element, HirType::Null | HirType::Undefined))
                .cloned()
                .collect::<Vec<_>>();
            if !result_members.contains(&rhs_type) {
                result_members.push(rhs_type);
            }
            let result_type = match result_members.as_slice() {
                [member] => member.clone(),
                members => HirType::Union(members.to_vec()),
            };
            let name = format!("__thaw_nullish_union_left_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), lhs_type.clone());
            let left = HirExpr::Var(name.clone());
            let mut body = Vec::new();
            for (index, element) in elements.iter().enumerate() {
                let value = if matches!(element, HirType::Null | HirType::Undefined) {
                    self.coerce_to_declared(&result_type, rhs.clone())?
                } else {
                    self.coerce_to_declared(
                        &result_type,
                        HirExpr::UnionValue(Box::new(left.clone()), index, elements.clone()),
                    )?
                };
                if index + 1 == elements.len() {
                    body.push(HirStmt::Return(Some(value)));
                } else {
                    body.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(left.clone()), elements.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        vec![HirStmt::Return(Some(value))],
                        Vec::new(),
                    ));
                }
            }
            return self
                .wrap_call_argument_bindings(HirExpr::Block(body), &[(name, lhs_type, lhs)]);
        }
        let (payload, is_none, value) = match lhs_type.clone() {
            HirType::Optional(payload) => {
                let is_none = HirExpr::OptionalIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 0)
            }
            HirType::Nullable(payload) => {
                let is_none = HirExpr::NullableIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 1)
            }
            HirType::Nullish(payload) => {
                let is_none = HirExpr::NullishIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 2)
            }
            _ => return Ok(lhs),
        };
        self.expect_type(payload.as_ref(), &rhs, "nullish fallback")?;
        let name = format!("__thaw_nullish_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let is_none = match is_none {
            HirExpr::OptionalIsNone(_, payload) => {
                HirExpr::OptionalIsNone(Box::new(left.clone()), payload)
            }
            HirExpr::NullableIsNone(_, payload) => {
                HirExpr::NullableIsNone(Box::new(left.clone()), payload)
            }
            HirExpr::NullishIsNone(_, payload) => {
                HirExpr::NullishIsNone(Box::new(left.clone()), payload)
            }
            _ => unreachable!(),
        };
        let present = match value {
            0 => HirExpr::OptionalValue(Box::new(left), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(left), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(left), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            is_none,
            vec![HirStmt::Return(Some(rhs))],
            vec![HirStmt::Return(Some(present))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
    }

    fn coerce_primitive_to_string(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::Str => Ok(value),
            HirType::Symbol => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_symbol_key".to_string())),
                vec![value],
            )),
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                vec![value],
            )),
            HirType::F64 => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                vec![value],
            )),
            HirType::I64 => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_i64_to_string".to_string())),
                vec![value],
            )),
            HirType::Json => Ok(HirExpr::JsonAsString(Box::new(value))),
            // Mirrors `coerce_primitive_to_number`'s `JsValue` arm: read
            // the handle back as JSON, then stringify. Lets `"" + x` /
            // `x + ","` work on an opaque handle (a dynamic property read
            // such as p-limit's `limit.activeCount`), not just a `Json`.
            HirType::JsValue => Ok(HirExpr::JsonAsString(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                vec![value],
            )))),
            HirType::Null => Ok(HirExpr::Lit(HirLit::Str("null".to_string()))),
            HirType::Undefined => Ok(HirExpr::Lit(HirLit::Str("undefined".to_string()))),
            HirType::Optional(payload) => {
                let optional_type = HirType::Optional(payload.clone());
                let name = format!("__thaw_string_optional_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), optional_type.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_string(HirExpr::OptionalValue(
                    Box::new(bound.clone()),
                    payload.as_ref().clone(),
                ))?;
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                        "undefined".to_string(),
                    ))))],
                    vec![HirStmt::Return(Some(present))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, optional_type, value)])
            }
            HirType::Nullable(payload) => {
                let nullable_type = HirType::Nullable(payload.clone());
                let name = format!("__thaw_string_nullable_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), nullable_type.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_string(HirExpr::NullableValue(
                    Box::new(bound.clone()),
                    payload.as_ref().clone(),
                ))?;
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                        "null".to_string(),
                    ))))],
                    vec![HirStmt::Return(Some(present))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, nullable_type, value)])
            }
            HirType::Nullish(payload) => {
                let nullish_type = HirType::Nullish(payload.clone());
                let name = format!("__thaw_string_nullish_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), nullish_type.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_string(HirExpr::NullishValue(
                    Box::new(bound.clone()),
                    payload.as_ref().clone(),
                ))?;
                let result = HirExpr::Block(vec![
                    HirStmt::If(
                        HirExpr::NullishIsNull(Box::new(bound.clone()), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                            "null".to_string(),
                        ))))],
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(Box::new(bound), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                            "undefined".to_string(),
                        ))))],
                        vec![HirStmt::Return(Some(present))],
                    ),
                ]);
                self.wrap_call_argument_bindings(result, &[(name, nullish_type, value)])
            }
            HirType::Union(members) => {
                let union_type = HirType::Union(members.clone());
                let name = format!("__thaw_string_union_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), union_type.clone());
                let mut statements = Vec::with_capacity(members.len());
                for index in 0..members.len() {
                    let part = self.coerce_primitive_to_string(HirExpr::UnionValue(
                        Box::new(HirExpr::Var(name.clone())),
                        index,
                        members.clone(),
                    ))?;
                    if index + 1 == members.len() {
                        statements.push(HirStmt::Return(Some(part)));
                    } else {
                        statements.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(HirExpr::Var(name.clone())),
                                    members.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Return(Some(part))],
                            Vec::new(),
                        ));
                    }
                }
                self.wrap_call_argument_bindings(
                    HirExpr::Block(statements),
                    &[(name, union_type, value)],
                )
            }
            HirType::Object(fields) => {
                // A class extending `Error`/`TypeError`/etc. (see
                // `lower/module/classes.rs`) is a real object with an
                // inherited `message: Str` field, not the tagged string
                // `new Error(...)` produces directly -- but `throw`ing one
                // (or any other string coercion: `String(value)`, `+`,
                // template literals) needs to recover that same tagged
                // form (`\u{1}<Name>[$<AncestorName>...]\u{1}<message>`, the
                // full identity chain rather than just the most-derived
                // name, so `instanceof` on an intermediate ancestor still
                // matches -- see `thaw_error_is_instance`) so every
                // existing exception reader still understands it, instead
                // of falling into the generic `[object Object]` conversion.
                let error_chain = object_type_is_error_family(&HirType::Object(fields.clone()))
                    .then(|| {
                        let (marker, _) = &fields[0];
                        marker
                            .strip_prefix("__thaw_class_identity_")
                            .expect("checked by object_type_is_error_family")
                            .to_string()
                    });
                match error_chain {
                    Some(name)
                        if fields
                            .iter()
                            .any(|(field, ty)| field == "message" && *ty == HirType::Str) =>
                    {
                        let message = HirExpr::PropAccess(
                            Box::new(value),
                            HirType::Object(fields),
                            "message".to_string(),
                        );
                        let tag = HirExpr::Lit(HirLit::Str(format!("\u{1}{name}\u{1}")));
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![tag, message],
                        ))
                    }
                    _ => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_to_string".to_string())),
                        vec![value],
                    )),
                }
            }
            HirType::Array(element) => {
                let builtin = match element.as_ref() {
                    HirType::F64 => "__thaw_number_array_to_string",
                    HirType::Str => "__thaw_string_array_to_string",
                    HirType::Bool => "__thaw_bool_array_to_string",
                    HirType::Object(_) => "__thaw_object_array_to_string",
                    other => {
                        return Err(format!(
                            "array string conversion does not support element type {other:?}"
                        ))
                    }
                };
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var(builtin.to_string())),
                    vec![value],
                ))
            }
            HirType::Tuple(elements) => {
                let tuple_type = HirType::Tuple(elements.clone());
                let (tuple, binding) = if matches!(value, HirExpr::Var(_) | HirExpr::TypedIndex(..))
                {
                    (value, None)
                } else {
                    let name = format!("__thaw_string_tuple_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), tuple_type.clone());
                    (
                        HirExpr::Var(name.clone()),
                        Some((name, tuple_type.clone(), value)),
                    )
                };
                let mut result = HirExpr::Lit(HirLit::Str(String::new()));
                for (index, element) in elements.iter().enumerate() {
                    if index != 0 {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![result, HirExpr::Lit(HirLit::Str(",".to_string()))],
                        );
                    }
                    let part = self.coerce_primitive_to_string(HirExpr::TypedIndex(
                        Box::new(tuple.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element.clone(),
                    ))?;
                    result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![result, part],
                    );
                }
                match binding {
                    Some(binding) => self.wrap_call_argument_bindings(result, &[binding]),
                    None => Ok(result),
                }
            }
            other => Err(format!(
                "string concatenation cannot convert native type {other:?}"
            )),
        }
    }

    fn join_tuple(
        &mut self,
        value: HirExpr,
        elements: Vec<HirType>,
        separator: HirExpr,
    ) -> Result<HirExpr, String> {
        let tuple_type = HirType::Tuple(elements.clone());
        let tuple_name = format!("__thaw_join_tuple_{}", self.next_binding);
        self.next_binding += 1;
        let separator_name = format!("__thaw_join_separator_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(tuple_name.clone(), tuple_type.clone());
        self.scope.insert(separator_name.clone(), HirType::Str);
        let tuple = HirExpr::Var(tuple_name.clone());
        let separator_var = HirExpr::Var(separator_name.clone());
        let mut result = HirExpr::Lit(HirLit::Str(String::new()));
        for (index, element) in elements.into_iter().enumerate() {
            if index != 0 {
                result = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                    vec![result, separator_var.clone()],
                );
            }
            let part = self.coerce_primitive_to_string(HirExpr::TypedIndex(
                Box::new(tuple.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                element,
            ))?;
            result = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                vec![result, part],
            );
        }
        self.wrap_call_argument_bindings(
            result,
            &[
                (tuple_name, tuple_type, value),
                (separator_name, HirType::Str, separator),
            ],
        )
    }

    fn lower_loose_equality(
        &mut self,
        mut lhs: HirExpr,
        mut rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type == rhs_type {
            return Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)));
        }
        let nullish_check =
            match (&lhs_type, &rhs_type) {
                (HirType::Optional(payload), HirType::Null)
                | (HirType::Optional(payload), HirType::Undefined) => Some(
                    HirExpr::OptionalIsNone(Box::new(lhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Null, HirType::Optional(payload))
                | (HirType::Undefined, HirType::Optional(payload)) => Some(
                    HirExpr::OptionalIsNone(Box::new(rhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Nullable(payload), HirType::Null)
                | (HirType::Nullable(payload), HirType::Undefined) => Some(
                    HirExpr::NullableIsNone(Box::new(lhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Null, HirType::Nullable(payload))
                | (HirType::Undefined, HirType::Nullable(payload)) => Some(
                    HirExpr::NullableIsNone(Box::new(rhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Nullish(payload), HirType::Null)
                | (HirType::Nullish(payload), HirType::Undefined) => Some(HirExpr::NullishIsNone(
                    Box::new(lhs.clone()),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullish(payload))
                | (HirType::Undefined, HirType::Nullish(payload)) => Some(HirExpr::NullishIsNone(
                    Box::new(rhs.clone()),
                    payload.as_ref().clone(),
                )),
                _ => None,
            };
        if let Some(check) = nullish_check {
            return Ok(check);
        }
        if matches!(
            (&lhs_type, &rhs_type),
            (HirType::Null, HirType::Undefined) | (HirType::Undefined, HirType::Null)
        ) {
            let lhs_name = format!("__thaw_loose_nullish_left_{}", self.next_binding);
            self.next_binding += 1;
            let rhs_name = format!("__thaw_loose_nullish_right_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(lhs_name.clone(), lhs_type.clone());
            self.scope.insert(rhs_name.clone(), rhs_type.clone());
            return self.wrap_call_argument_bindings(
                HirExpr::Lit(HirLit::Bool(true)),
                &[(lhs_name, lhs_type, lhs), (rhs_name, rhs_type, rhs)],
            );
        }
        lhs = self.coerce_primitive_to_number(lhs)?;
        rhs = self.coerce_primitive_to_number(rhs)?;
        Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)))
    }

    fn coerce_primitive_to_number(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::F64 => Ok(value),
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                vec![value],
            )),
            HirType::Str => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                vec![value],
            )),
            HirType::Json => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::JsValue => Ok(HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                vec![value],
            )))),
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) => {
                let string = self.coerce_primitive_to_string(value)?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                    vec![string],
                ))
            }
            other => Err(format!(
                "numeric conversion is not defined for native type {other:?}"
            )),
        }
    }

    fn lower_relational(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        op: BinOp,
    ) -> Result<HirExpr, String> {
        if self.infer_expr_type(&lhs)? == HirType::I64
            && self.infer_expr_type(&rhs)? == HirType::I64
        {
            return Ok(HirExpr::BinOp(op, Box::new(lhs), Box::new(rhs)));
        }
        if self.infer_expr_type(&lhs)? == HirType::Str
            && self.infer_expr_type(&rhs)? == HirType::Str
        {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var(
                    match op {
                        BinOp::Lt => "__thaw_string_lt",
                        BinOp::Gt => "__thaw_string_gt",
                        BinOp::LtEq => "__thaw_string_lte",
                        BinOp::GtEq => "__thaw_string_gte",
                        _ => unreachable!(),
                    }
                    .to_string(),
                )),
                vec![lhs, rhs],
            ));
        }
        Ok(HirExpr::BinOp(
            op,
            Box::new(self.coerce_primitive_to_number(lhs)?),
            Box::new(self.coerce_primitive_to_number(rhs)?),
        ))
    }

    fn lower_optional_undefined_equality(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        let result =
            match (&lhs_type, &rhs_type) {
                (HirType::Optional(payload), HirType::Undefined) => Some(HirExpr::OptionalIsNone(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Undefined, HirType::Optional(payload)) => Some(HirExpr::OptionalIsNone(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Json, HirType::Null) => Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                    vec![lhs],
                )),
                (HirType::Null, HirType::Json) => Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                    vec![rhs],
                )),
                (HirType::Json, HirType::Undefined) => Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                    vec![lhs],
                )),
                (HirType::Undefined, HirType::Json) => Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                    vec![rhs],
                )),
                (HirType::Nullable(payload), HirType::Null) => Some(HirExpr::NullableIsNone(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullable(payload)) => Some(HirExpr::NullableIsNone(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Nullish(payload), HirType::Null) => Some(HirExpr::NullishIsNull(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullish(payload)) => Some(HirExpr::NullishIsNull(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Nullish(payload), HirType::Undefined) => Some(
                    HirExpr::NullishIsUndefined(Box::new(lhs), payload.as_ref().clone()),
                ),
                (HirType::Undefined, HirType::Nullish(payload)) => Some(
                    HirExpr::NullishIsUndefined(Box::new(rhs), payload.as_ref().clone()),
                ),
                (HirType::Union(left), HirType::Union(right)) if left == right => Some(
                    HirExpr::UnionIsEqual(Box::new(lhs), Box::new(rhs), left.clone()),
                ),
                (HirType::Union(left), HirType::Union(right))
                    if equivalent_union_members(left, right) =>
                {
                    let rhs = self.coerce_to_declared(&HirType::Union(left.clone()), rhs)?;
                    Some(HirExpr::UnionIsEqual(
                        Box::new(lhs),
                        Box::new(rhs),
                        left.clone(),
                    ))
                }
                (HirType::Union(elements), member) => elements
                    .iter()
                    .position(|element| element == member)
                    .map(|index| {
                        HirExpr::UnionMemberIsEqual(
                            Box::new(lhs),
                            Box::new(rhs),
                            index,
                            elements.clone(),
                        )
                    }),
                (member, HirType::Union(elements)) => elements
                    .iter()
                    .position(|element| element == member)
                    .map(|index| {
                        HirExpr::UnionMemberIsEqual(
                            Box::new(rhs),
                            Box::new(lhs),
                            index,
                            elements.clone(),
                        )
                    }),
                (HirType::Null, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Null, _) | (_, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(false))),
                (HirType::Undefined, HirType::Undefined) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Undefined, _) | (_, HirType::Undefined) => {
                    Some(HirExpr::Lit(HirLit::Bool(false)))
                }
                _ => None,
            };
        Ok(result)
    }

}
