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
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Promise(_)
            | HirType::Function(_, _)
            | HirType::CallableFunction(..) => Ok(HirExpr::Lit(HirLit::Bool(true))),
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
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type != rhs_type {
            return Err(format!(
                "logical operands have incompatible types {lhs_type:?} and {rhs_type:?}"
            ));
        }
        let name = format!("__thaw_logical_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let condition = self.truthiness_expr(left.clone(), &lhs_type)?;
        let (then_value, else_value) = if is_and { (rhs, left) } else { (left, rhs) };
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(then_value))],
            vec![HirStmt::Return(Some(else_value))],
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
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                vec![value],
            )),
            HirType::F64 => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                vec![value],
            )),
            HirType::Object(_) => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_to_string".to_string())),
                vec![value],
            )),
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
        &self,
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

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
                s.value.to_string_lossy().into_owned(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Lit(Lit::Null(_)) => Ok(HirExpr::Lit(HirLit::Null)),
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
                        if right_type == HirType::Json {
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            let left_name = format!("__thaw_in_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(left_name.clone(), HirType::Str);
                            let right_name = format!("__thaw_in_object_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(right_name.clone(), HirType::Json);
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
                                    (right_name, HirType::Json, rhs),
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

    fn lower_satisfies(
        &mut self,
        satisfies: &swc_ecma_ast::TsSatisfiesExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&satisfies.expr)?;
        let expected = lower_ts_type(
            &satisfies.type_ann,
            self.interfaces,
            self.generic_interfaces,
        )?;
        self.coerce_to_declared(&expected, value.clone())?;
        Ok(value)
    }

    fn lower_non_null_assertion(
        &mut self,
        assertion: &swc_ecma_ast::TsNonNullExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&assertion.expr)?;
        match self.infer_expr_type(&value)? {
            HirType::Optional(payload) => Ok(HirExpr::OptionalValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullable(payload) => Ok(HirExpr::NullableValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullish(payload) => Ok(HirExpr::NullishValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Null | HirType::Undefined => Err(
                "non-null assertion cannot produce a native value from a statically null or undefined expression"
                    .into(),
            ),
            _ => Ok(value),
        }
    }

    fn lower_recursive_function_expression(
        &mut self,
        outer_name: &str,
        expression: &swc_ecma_ast::FnExpr,
        annotated: Option<&HirType>,
    ) -> Result<Option<(Symbol, HirType, HirExpr)>, String> {
        if !named_function_is_recursive(expression) {
            return Ok(None);
        }
        if expression.function.type_params.is_some() {
            return Ok(None);
        }
        let internal = expression
            .ident
            .as_ref()
            .expect("recursive named function expression")
            .sym
            .to_string();
        let arrow = function_expression_as_arrow(expression)?;
        let ty = if let Some(annotated) = annotated {
            annotated.clone()
        } else {
            let params = arrow
                .params
                .iter()
                .map(|parameter| {
                    lower_param(
                        parameter,
                        self.interfaces,
                        self.generic_interfaces,
                        false,
                        &HashMap::new(),
                    )
                    .map(|parameter| parameter.ty)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let result = arrow
                .return_type
                .as_ref()
                .ok_or_else(|| "recursive local function values need a return annotation or an outer function type".to_string())
                .and_then(|annotation| {
                    lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )
                })?;
            HirType::Function(params, Box::new(result))
        };
        let HirType::Function(params, ret) = &ty else {
            return Err("recursive named function expression needs a function type".into());
        };
        let hir_name = self.bind_local(outer_name, ty.clone());
        let saved_internal = (internal != outer_name)
            .then(|| self.bindings.get(&internal).cloned())
            .flatten();
        if internal != outer_name {
            self.bindings
                .entry(internal.clone())
                .or_default()
                .push(hir_name.clone());
        }
        let lowered = self.lower_contextual_arrow(&arrow, params, Some(ret));
        if internal != outer_name {
            if let Some(saved) = saved_internal {
                self.bindings.insert(internal, saved);
            } else {
                self.bindings.remove(&internal);
            }
        }
        let closure = self.coerce_to_declared(&ty, lowered?)?;
        Ok(Some((
            hir_name.clone(),
            ty.clone(),
            HirExpr::RecursiveClosure(hir_name, ty, Box::new(closure)),
        )))
    }

    fn lower_arrow(&mut self, arrow: &swc_ecma_ast::ArrowExpr) -> Result<HirExpr, String> {
        if arrow.is_generator || arrow.type_params.is_some() {
            return Err("generator and generic arrow functions are not supported yet".into());
        }
        let source_params = arrow
            .params
            .iter()
            .map(|param| {
                lower_param(
                    param,
                    self.interfaces,
                    self.generic_interfaces,
                    false,
                    &HashMap::new(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let declared_return = arrow
            .return_type
            .as_ref()
            .map(|ann| lower_ts_type(&ann.type_ann, self.interfaces, self.generic_interfaces))
            .transpose()?;

        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(source_params.len());
            let mut destructuring = Vec::new();
            for (pattern, param) in arrow.params.iter().zip(source_params) {
                let name = self.bind_local(&param.name, param.ty.clone());
                if !matches!(pattern, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pattern, name.clone(), param.ty.clone()));
                }
                params.push(HirParam { name, ty: param.ty });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            let declared_async_result = if arrow.is_async {
                match &declared_return {
                    Some(HirType::Promise(result)) => Some(result.as_ref().clone()),
                    Some(other) => {
                        return Err(format!(
                            "async arrow return annotation must be Promise<T>, got {other:?}"
                        ))
                    }
                    None => None,
                }
            } else {
                None
            };
            self.ret_type = declared_async_result
                .clone()
                .or_else(|| declared_return.clone())
                .unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let mut expression = self.lower_expr(expr)?;
                    let mut inferred = self.infer_expr_type(&expression)?;
                    if arrow.is_async {
                        if let HirExpr::AwaitPromise(promise, resolved) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else if let HirExpr::Await(promise) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(inferred));
                        }
                        let resolved = declared_async_result.clone().unwrap_or_else(|| {
                            if let HirType::Promise(inner) = &inferred {
                                inner.as_ref().clone()
                            } else {
                                inferred.clone()
                            }
                        });
                        if contains_await(&expression) {
                            expression = self.coerce_to_declared(&resolved, expression)?;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else {
                            let assimilates = matches!(
                                &inferred,
                                HirType::Promise(inner) if inner.as_ref() == &resolved
                            );
                            if !assimilates {
                                expression = self.coerce_to_declared(&resolved, expression)?;
                            }
                            let resolve_type = HirType::Function(
                                vec![if assimilates {
                                    HirType::Promise(Box::new(resolved.clone()))
                                } else {
                                    resolved.clone()
                                }],
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&expression, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name.clone(),
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name)),
                                    vec![expression],
                                )),
                            );
                            expression = HirExpr::PromiseNew(
                                Box::new(executor),
                                resolved.clone(),
                                assimilates,
                            );
                            inferred = HirType::Promise(Box::new(resolved));
                        }
                    } else if let Some(expected) = &declared_return {
                        expression = self.coerce_to_declared(expected, expression)?;
                        inferred = self.infer_expr_type(&expression)?;
                    }
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    if arrow.is_async {
                        let has_await = stmts.iter().any(stmt_contains_await);
                        let only_tail_awaits =
                            has_await && async_arrow_has_only_tail_await_returns(&stmts);
                        let resolved = declared_async_result.clone().unwrap_or(inferred);
                        if has_await && !only_tail_awaits {
                            (HirExpr::Block(stmts), HirType::Promise(Box::new(resolved)))
                        } else {
                            let assimilates = if only_tail_awaits {
                                stmts = strip_async_arrow_tail_awaits(stmts);
                                true
                            } else {
                                false
                            };
                            let resolve_type = HirType::Function(
                                if assimilates {
                                    vec![HirType::Promise(Box::new(resolved.clone()))]
                                } else if resolved == HirType::Void {
                                    Vec::new()
                                } else {
                                    vec![resolved.clone()]
                                },
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut executor_body =
                                rewrite_async_arrow_returns(stmts, &resolve_name, &resolved)?;
                            if resolved == HirType::Void && !assimilates {
                                executor_body.push(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name.clone())),
                                    Vec::new(),
                                )));
                            }
                            let executor_body = HirExpr::Block(executor_body);
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&executor_body, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter(|name| name != &resolve_name)
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name,
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(executor_body),
                            );
                            (
                                HirExpr::PromiseNew(
                                    Box::new(executor),
                                    resolved.clone(),
                                    assimilates,
                                ),
                                HirType::Promise(Box::new(resolved)),
                            )
                        }
                    } else {
                        (HirExpr::Block(stmts), inferred)
                    }
                }
            };
            let return_type = declared_return.clone().unwrap_or(inferred_return);
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    saved_scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();
            Ok(HirExpr::Lambda(
                captures,
                params,
                return_type,
                Box::new(body),
            ))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }

    fn lower_generic_instantiation_expression(
        &mut self,
        instantiation: &swc_ecma_ast::TsInstantiation,
    ) -> Result<HirExpr, String> {
        let Expr::Ident(identifier) = instantiation.expr.as_ref() else {
            return Err(
                "generic instantiation expressions require a named top-level function".into(),
            );
        };
        let name = identifier.sym.to_string();
        let signature = self
            .signatures
            .get(&name)
            .ok_or_else(|| format!("unknown function `{name}` in instantiation expression"))?;
        if signature.generic_type_params.is_empty() {
            return Err(format!(
                "non-generic function `{name}` cannot be used in an instantiation expression"
            ));
        }
        let types = resolve_explicit_generic_type_tuple(
            signature,
            &instantiation.type_args.params,
            &[],
            self.interfaces,
            self.generic_interfaces,
        )?;
        for ty in &types {
            if !supports_generic_native_layout(ty) {
                return Err(format!(
                    "generic function `{name}` cannot specialize for native layout {ty:?}"
                ));
            }
        }
        let substitution = signature
            .generic_type_params
            .iter()
            .cloned()
            .zip(types.iter().cloned())
            .collect::<HashMap<_, _>>();
        let params = signature
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ret = resolve_ts_type_with_substitution(
            signature
                .generic_return_type
                .as_ref()
                .expect("generic instantiation return type"),
            &substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        if signature.is_async && !matches!(ret, HirType::Promise(_)) {
            ret = HirType::Promise(Box::new(ret));
        }
        if let Some(constraints) = self.call_constraints {
            constraints
                .borrow_mut()
                .push(CallConstraint::Generic(name.clone(), types.clone()));
        }
        let specialized = specialized_generic_function_name(&name, &params, signature, &types);
        Ok(HirExpr::FunctionRef(specialized, params, ret))
    }

    fn lower_stored_generic_arrow(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let Some(internal) = self.generic_arrow_self_names.get(name).cloned() else {
            return self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        };
        let signature = self.generic_arrow_signature(arrow)?;
        let concrete_types = infer_generic_type_tuple(
            &signature,
            parameter_types,
            self.interfaces,
            self.generic_interfaces,
        )?;
        let return_type =
            if let Some(expected) = expected_return.filter(|ty| **ty != HirType::Dynamic) {
                expected.clone()
            } else {
                let declared = signature.generic_return_type.as_ref().ok_or_else(|| {
                    format!("recursive generic local function `{name}` needs a return annotation")
                })?;
                let substitution = signature
                    .generic_type_params
                    .iter()
                    .cloned()
                    .zip(concrete_types)
                    .collect::<HashMap<_, _>>();
                resolve_ts_type_with_substitution(
                    declared,
                    &substitution,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )?
            };
        let self_type = HirType::Function(parameter_types.to_vec(), Box::new(return_type));
        let self_name = format!("__thaw_recursive_generic_{}", self.next_binding);
        self.next_binding += 1;
        let saved_binding = self.bindings.get(&internal).cloned();
        self.scope.insert(self_name.clone(), self_type.clone());
        self.bindings
            .entry(internal.clone())
            .or_default()
            .push(self_name.clone());
        let lowered = self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        self.scope.remove(&self_name);
        if let Some(saved) = saved_binding {
            self.bindings.insert(internal.clone(), saved);
        } else {
            self.bindings.remove(&internal);
        }
        lowered.map(|closure| HirExpr::RecursiveClosure(self_name, self_type, Box::new(closure)))
    }

    fn lower_contextual_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if arrow.params.len() != parameter_types.len() {
            return Err(format!(
                "Promise callback expects {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
        }
        if arrow.is_async {
            if arrow.is_generator {
                return Err("async generator arrow functions are not supported".into());
            }
            let mut contextual = arrow.clone();
            for (parameter, ty) in contextual.params.iter_mut().zip(parameter_types) {
                let annotation = match parameter {
                    Pat::Ident(binding) => &mut binding.type_ann,
                    Pat::Rest(rest) => &mut rest.type_ann,
                    _ => return Err(
                        "contextual async arrows currently require identifier or rest parameters"
                            .into(),
                    ),
                };
                if annotation.is_none() {
                    *annotation = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    }));
                }
            }
            if contextual.return_type.is_none() {
                if let Some(expected) = expected_return {
                    contextual.return_type = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(expected)?),
                    }));
                }
            }
            return self.lower_arrow(&contextual);
        }
        if arrow.is_generator {
            return Err("generator Promise callbacks are not supported".into());
        }
        let generic_return = if let Some(type_params) = &arrow.type_params {
            validate_trailing_type_parameter_defaults(
                "generic arrow function",
                "<anonymous>",
                type_params,
            )?;
            let generic_type_params = type_params
                .params
                .iter()
                .map(|parameter| parameter.name.sym.to_string())
                .collect::<Vec<_>>();
            let substitutions = generic_type_params
                .iter()
                .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                .collect::<HashMap<_, _>>();
            let generic_param_patterns = arrow
                .params
                .iter()
                .map(|parameter| {
                    let Pat::Ident(binding) = parameter else {
                        return Err(
                            "generic contextual arrows require identifier parameters".into()
                        );
                    };
                    let annotation = binding
                        .type_ann
                        .as_ref()
                        .ok_or("generic contextual arrow parameters need type annotations")?;
                    generic_type_pattern(
                        &annotation.type_ann,
                        &substitutions,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            let signature = FnSignature {
                params: Vec::new(),
                variadic: None,
                native_rest: None,
                abstract_class_constructor: false,
                ret: HirType::Dynamic,
                is_async: false,
                uses_this: false,
                is_extern: false,
                source_range: (arrow.span.lo.0, arrow.span.hi.0),
                generic_type_params,
                generic_type_constraints: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.constraint.clone())
                    .collect(),
                generic_type_defaults: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.default.clone())
                    .collect(),
                generic_param_patterns,
                generic_param_optional: arrow
                    .params
                    .iter()
                    .map(
                        |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.optional),
                    )
                    .collect(),
                generic_return_type: arrow
                    .return_type
                    .as_ref()
                    .map(|annotation| annotation.type_ann.clone()),
            };
            let types = infer_generic_type_tuple(
                &signature,
                parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )?;
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(types)
                .collect::<HashMap<_, _>>();
            arrow
                .return_type
                .as_ref()
                .map(|annotation| {
                    resolve_ts_type_with_substitution(
                        &annotation.type_ann,
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        if let (Some(declared), Some(contextual)) = (&generic_return, expected_return) {
            if contextual != &HirType::Dynamic && declared != contextual {
                return Err(format!(
                    "generic callback declares return type {declared:?}, expected {contextual:?}"
                ));
            }
        }
        let expected_return = generic_return.as_ref().or(expected_return);
        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(parameter_types.len());
            let mut destructuring = Vec::new();
            for (pat, ty) in arrow.params.iter().zip(parameter_types) {
                let source_name = match pat {
                    Pat::Ident(binding) => binding.id.sym.to_string(),
                    Pat::Assign(assignment) => match assignment.left.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "default callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Rest(rest) => match rest.arg.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "rest callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Object(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    Pat::Array(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    _ => return Err("unsupported Promise callback parameter pattern".into()),
                };
                let name = self.bind_local(&source_name, ty.clone());
                if !matches!(pat, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pat, name.clone(), ty.clone()));
                }
                params.push(HirParam {
                    name,
                    ty: ty.clone(),
                });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            self.ret_type = expected_return.cloned().unwrap_or(HirType::Dynamic);
            let (body, inferred) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let expression = self.lower_expr(expr)?;
                    let inferred = self.infer_expr_type(&expression)?;
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    (HirExpr::Block(stmts), inferred)
                }
            };
            if let Some(expected) = expected_return {
                if inferred != *expected && inferred != HirType::Dynamic {
                    return Err(format!(
                        "Promise callback returns {inferred:?}, expected {expected:?}"
                    ));
                }
            }
            let ret = expected_return.cloned().unwrap_or(inferred);
            if let Some(expected) = expected_return {
                debug_assert_eq!(&ret, expected);
            }
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    saved_scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();
            Ok(HirExpr::Lambda(captures, params, ret, Box::new(body)))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }
}
