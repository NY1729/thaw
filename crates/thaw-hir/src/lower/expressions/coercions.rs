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
        // A dynamic call (`hljs.getLanguage(lang)`, `Json`-typed) as the
        // *right* operand of `&&`/`||` -- `lang && hljs.getLanguage(lang)`,
        // real highlight.js. The result is dynamic either way (the left
        // operand when it's falsy, the dynamic value otherwise), so promote
        // the left to `Json` and let the existing `lhs_type == Json` path
        // below handle the rest. Only a `Json` *left* operand was handled
        // before, which is why this mirrored shape failed with "logical
        // operands have incompatible types Str and Json".
        if rhs_type == HirType::Json && lhs_type != HirType::Json {
            lhs = self.wrap_native_value_as_json(lhs, lhs_type.clone())?;
            lhs_type = HirType::Json;
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
            // `String(value)` performs JavaScript ToPrimitive and therefore
            // honors an opaque object's own `toString`/`valueOf`. Reading it
            // back through JSON first loses that identity (real trigger:
            // crypto-js WordArray implicitly renders as hex in `"x" + word`).
            HirType::JsValue => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_js_handle_to_string".to_string())),
                vec![value],
            )),
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
                // `lower/module/classes.rs`) is a real object with
                // inherited `message`/`name: Str` fields, not the tagged
                // string `new Error(...)` produces directly -- but
                // `throw`ing one (or any other string coercion:
                // `String(value)`, `+`, template literals) needs to
                // recover that same tagged form
                // (`\u{1}<Name>[$<AncestorName>...]\u{1}<message>\u{4}
                // <runtime name>`) so every existing exception reader
                // still understands it, instead of falling into the
                // generic `[object Object]` conversion. The identity
                // chain (`<Name>[$<AncestorName>...]`) is a *static*,
                // compile-time-derived string, used only for
                // `instanceof` matching against an intermediate
                // ancestor (`thaw_error_is_instance`); `.name`/`.stack`/
                // `.toString()` instead read the *runtime* value of the
                // object's own `name` field (a `\u{4}`-tagged override
                // segment, see `thaw_error_name` in thaw-runtime) so a
                // constructor's `this.name = "MyError"` -- or the
                // real-JS-matching default `lower/invocations/calls.rs`'s
                // `super(...)` handling already assigned -- is what
                // actually gets reported, not the (possibly compiler-
                // mangled) class identity string.
                let error_chain = object_type_is_error_family(&HirType::Object(fields.clone()))
                    .then(|| {
                        let (marker, _) = &fields[0];
                        marker
                            .strip_prefix("__thaw_class_identity_")
                            .expect("checked by object_type_is_error_family")
                            .to_string()
                    });
                match error_chain {
                    Some(chain)
                        if fields
                            .iter()
                            .any(|(field, ty)| field == "message" && *ty == HirType::Str)
                            && fields
                                .iter()
                                .any(|(field, ty)| field == "name" && *ty == HirType::Str) =>
                    {
                        let object_type = HirType::Object(fields);
                        let binding = format!("__thaw_error_tag_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(binding.clone(), object_type.clone());
                        let bound = HirExpr::Var(binding.clone());
                        let message = HirExpr::PropAccess(
                            Box::new(bound.clone()),
                            object_type.clone(),
                            "message".to_string(),
                        );
                        let name = HirExpr::PropAccess(
                            Box::new(bound),
                            object_type.clone(),
                            "name".to_string(),
                        );
                        let tag = HirExpr::Lit(HirLit::Str(format!("\u{1}{chain}\u{1}")));
                        let tagged_message = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![tag, message],
                        );
                        let name_marker = HirExpr::Lit(HirLit::Str("\u{4}".to_string()));
                        let tagged_name = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![name_marker, name],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![tagged_message, tagged_name],
                        );
                        self.wrap_call_argument_bindings(
                            result,
                            &[(binding, object_type, value)],
                        )
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

    /// `===`/`!==` between a live `JsValue` and a native scalar
    /// (`Str`/`F64`/`Bool`) -- a very common shape once a value comes from
    /// a dynamic getter (real example: koa's own `ctx.path === "/json"` or
    /// `ctx.method === "GET"`, where `ctx.path` reads as a `JsValue`).
    /// Coerces the `JsValue` side to the scalar's own native type via
    /// `coerce_to_declared` (which already maps `JsValue` -> scalar through
    /// `readDynamicValue` + `JsonAsString`/`JsonAsNumber`/`JsonAsBool`), so
    /// the ordinary strict-equality emit applies unchanged. Without this,
    /// the strict-equality type check rejected the mismatched pair outright.
    fn coerce_strict_equality_operands(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<(HirExpr, HirExpr), String> {
        let scalar = |ty: &HirType| matches!(ty, HirType::Str | HirType::F64 | HirType::Bool);
        // A dynamic operand -- live `JsValue` handle, or a `Json` value
        // (a Fallback/QuickJS method call's own result, e.g. cheerio's
        // `$("p").text()`) -- against a concrete scalar is decoded to that
        // scalar so the comparison is well-typed. Without the `Json` half,
        // `$("p").text() === "W"` failed outright with "strict equality
        // compares incompatible types Json and Str", even though the bound
        // form (`const t = ...; t === "W"`) worked once annotated.
        let dynamic = |ty: &HirType| matches!(ty, HirType::Json | HirType::JsValue);
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if dynamic(&lhs_type) && scalar(&rhs_type) {
            let lhs = self.coerce_to_declared(&rhs_type, lhs)?;
            return Ok((lhs, rhs));
        }
        if dynamic(&rhs_type) && scalar(&lhs_type) {
            let rhs = self.coerce_to_declared(&lhs_type, rhs)?;
            return Ok((lhs, rhs));
        }
        Ok((lhs, rhs))
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
                // `x == undefined`/`x == null` (either operand order) for
                // a plain `Json`/`JsValue` value -- real JS's loose-
                // equality-against-`null`-or-`undefined` rule: true iff
                // the value itself is nullish, no other coercion applies.
                // Before this arm, both fell through to this function's
                // final fallback (`coerce_primitive_to_number` on both
                // operands), which has no case for a bare `HirType::
                // Undefined`/`Null` literal operand and errors outright
                // ("numeric conversion is not defined for native type
                // Undefined") -- a real, empirically-confirmed build-time
                // crash for e.g. `JSON.parse('{}').missingKey ==
                // undefined`, not just a wrong runtime answer.
                (HirType::Json, HirType::Null) | (HirType::Json, HirType::Undefined) => {
                    Some(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                        vec![lhs.clone()],
                    ))
                }
                (HirType::Null, HirType::Json) | (HirType::Undefined, HirType::Json) => {
                    Some(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                        vec![rhs.clone()],
                    ))
                }
                (HirType::JsValue, HirType::Null) | (HirType::JsValue, HirType::Undefined) => {
                    Some(self.dynamic_value_is_nullish(lhs.clone()))
                }
                (HirType::Null, HirType::JsValue) | (HirType::Undefined, HirType::JsValue) => {
                    Some(self.dynamic_value_is_nullish(rhs.clone()))
                }
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
            // Real JS's own `ToNumber`: `Number(null) === 0`,
            // `Number(undefined) === NaN` -- these differ from each
            // other, so a shared fallback can't collapse them. Without
            // these two arms, any context that reaches this function
            // with a bare `Null`/`Undefined`-typed operand (relational
            // comparison, `new Date(...)`'s argument coercion, ...)
            // crashed outright at build time instead of just answering
            // per real JS semantics -- confirmed empirically for `5 <
            // undefined`. `value` itself is still evaluated (via
            // `wrap_call_argument_bindings`) rather than discarded
            // outright, preserving any side effect a statically
            // `Null`/`Undefined`-typed expression might still have
            // (e.g. a function call whose declared return type is
            // `null`).
            HirType::Null => {
                let name = format!("__thaw_number_coerce_null_{}", self.next_binding);
                self.next_binding += 1;
                self.wrap_call_argument_bindings(
                    HirExpr::Lit(HirLit::F64(0.0)),
                    &[(name, HirType::Null, value)],
                )
            }
            HirType::Undefined => {
                let name = format!("__thaw_number_coerce_undefined_{}", self.next_binding);
                self.next_binding += 1;
                self.wrap_call_argument_bindings(
                    HirExpr::Lit(HirLit::F64(f64::NAN)),
                    &[(name, HirType::Undefined, value)],
                )
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
                // A live `JsValue` (a real QuickJS handle, e.g. `const x:
                // JsValue = someDynamicCall()`) has no equivalent of the
                // `Json` case above's `$__thaw_napi_undefined$` sentinel
                // tag to check locally -- there's no local representation
                // of "undefined" to compare against at all, only the live
                // handle itself. Ask the engine directly instead, the same
                // way `typeof` on a `JsValue` already does
                // (`__thaw_typeof_dynamic_value`, just above in this same
                // file's `UnaryOp::TypeOf` handling): round-trip through a
                // tiny bootstrap-registered JS function
                // (`__thaw_is_undefined_dynamic_value`) via
                // `callDynamicValueWithValue`, decoding its boolean result.
                // Without this arm, the call fell through to the catch-all
                // `(_, HirType::Undefined) => false` below -- unconditionally
                // wrong for a live value that genuinely is `undefined` (real
                // trigger: joi's `schema.validate(...)` result, whose
                // `.error` field on a valid input is a real absent/
                // `undefined` property read off a live handle).
                (HirType::JsValue, HirType::Undefined) => {
                    Some(self.dynamic_value_is_undefined(lhs))
                }
                (HirType::Undefined, HirType::JsValue) => {
                    Some(self.dynamic_value_is_undefined(rhs))
                }
                // Same story as the `Undefined` case just above, for
                // `null` instead: a live value that's genuinely `null`
                // otherwise fell through to the blanket `(HirType::Null,
                // _) | (_, HirType::Null) => false` catch-all further
                // down, unconditionally wrong.
                (HirType::JsValue, HirType::Null) => Some(self.dynamic_value_is_null(lhs)),
                (HirType::Null, HirType::JsValue) => Some(self.dynamic_value_is_null(rhs)),
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

    /// Asks the live QuickJS engine a yes/no question about a `JsValue`
    /// by calling one of the small bootstrap-registered globals in
    /// `crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js`
    /// (`__thaw_typeof_dynamic_value`'s siblings), the same way `typeof`
    /// on a `JsValue` already does. Shared by `dynamic_value_is_undefined`/
    /// `dynamic_value_is_null`/`dynamic_value_is_nullish` below -- see
    /// their call sites (`lower_optional_undefined_equality`/
    /// `lower_loose_equality`) for why each is needed.
    fn dynamic_value_check(&mut self, global: &str, value: HirExpr) -> HirExpr {
        let callable = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(global.into()))],
        );
        HirExpr::JsonAsBool(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
            vec![callable, value],
        )))
    }

    /// `value instanceof target` for two live `JsValue` handles -- a real
    /// runtime `instanceof` between a dynamic result and a class's own
    /// decorator "class token". Used when the left operand is a `JsValue`
    /// and the right names a decorated local class: the compile-time
    /// `class_type_has_identity` check has no native layout to compare
    /// (the value is an opaque live object), so this asks the live engine
    /// instead, exactly as `dynamic_value_check`'s one-argument calls do.
    /// `callDynamicValueMixed` is the existing two-handle invocation
    /// (`json_args` carries no positional JSON here, so an empty array);
    /// its JSON result is a real boolean.
    fn dynamic_value_instanceof(
        &mut self,
        value: HirExpr,
        target: HirExpr,
    ) -> Result<HirExpr, String> {
        let callable = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(
                "__thaw_instanceof_dynamic_value".into(),
            ))],
        );
        let empty_arguments = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(Vec::new()),
            HirType::Array(Box::new(HirType::Json)),
        )?;
        Ok(HirExpr::JsonAsBool(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixed".into())),
            vec![
                callable,
                empty_arguments,
                HirExpr::ArrayLit(vec![value, target]),
            ],
        ))))
    }

    /// The live "class token" `JsValue` symbol a decorated class's bare
    /// reference resolves to (see `DECORATOR_CLASS_TOKENS` /
    /// `lower_class_decorator_tokens`), if this class has one. The token is
    /// keyed by the class layout's own identity marker (its first field).
    fn decorator_class_token(&self, class_name: &str) -> Option<Symbol> {
        let HirType::Object(fields) = self.interfaces.get(class_name)? else {
            return None;
        };
        let marker = fields.first()?.0.as_str();
        DECORATOR_CLASS_TOKENS.with(|tokens| tokens.borrow().get(marker).cloned())
    }

    /// `value === undefined`/`undefined === value` for a live `JsValue`.
    /// See the call site's comment (`lower_optional_undefined_equality`).
    fn dynamic_value_is_undefined(&mut self, value: HirExpr) -> HirExpr {
        self.dynamic_value_check("__thaw_is_undefined_dynamic_value", value)
    }

    /// `value === null`/`null === value` for a live `JsValue`. Sibling
    /// gap to `dynamic_value_is_undefined`, found while fixing the
    /// `Json`-side missing-key-vs-null distinction: `cb423ffb` added a
    /// `(JsValue, Undefined)` arm but no `(JsValue, Null)` one, so a
    /// live value that's genuinely `null` still fell through to the
    /// blanket "an operand's type is `Null` and nothing more specific
    /// matched -> `false`" catch-all.
    fn dynamic_value_is_null(&mut self, value: HirExpr) -> HirExpr {
        self.dynamic_value_check("__thaw_is_null_dynamic_value", value)
    }

    /// `value == undefined`/`value == null` (either order) for a live
    /// `JsValue` -- real JS's own loose-equality-against-`null`-or-
    /// `undefined` rule ("nullish", full stop, no other coercion
    /// applies). See `lower_loose_equality`'s call site.
    fn dynamic_value_is_nullish(&mut self, value: HirExpr) -> HirExpr {
        self.dynamic_value_check("__thaw_is_nullish_dynamic_value", value)
    }
}
