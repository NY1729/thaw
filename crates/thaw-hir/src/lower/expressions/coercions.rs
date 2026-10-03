/// Which tagged-absence wrapper `lower_absence_equality` is comparing.
#[derive(Clone, Copy)]
enum AbsenceKind {
    /// `T | undefined` (`Optional`).
    Optional,
    /// `T | null` (`Nullable`).
    Nullable,
    /// `T | null | undefined` (`Nullish`).
    Nullish,
}

impl<'a> FnLowerer<'a> {
    // Function and CallableFunction have a raw-pointer ABI. An absent
    // function value is a typed null pointer, distinct from JS `null`.
    pub(crate) fn function_pointer_is_undefined(value: HirExpr, ty: &HirType) -> HirExpr {
        HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(value),
            Box::new(HirExpr::OptionalValue(
                Box::new(HirExpr::OptionalNone(ty.clone())),
                ty.clone(),
            )),
        )
    }

    fn contains_function_value(ty: &HirType) -> bool {
        match ty {
            HirType::Function(_, _) | HirType::CallableFunction(..) => true,
            HirType::Optional(value) | HirType::Nullable(value) | HirType::Nullish(value) =>
                Self::contains_function_value(value),
            HirType::Union(values) => values.iter().any(Self::contains_function_value),
            _ => false,
        }
    }

    // 0 = present, 1 = undefined, 2 = null. Inspect both operands once an
    // equality pair contains a Function leaf: the *other* operand may itself
    // be an absent Optional<number>, Union, or dynamic value.
    pub(crate) fn function_observable_absence(&mut self, value: HirExpr, ty: &HirType) -> HirExpr {
        let number = |n| HirExpr::Lit(HirLit::F64(n));
        let choose = |test, yes, no| HirExpr::Conditional(
            Box::new(test), Box::new(yes), Box::new(no), HirType::F64,
        );
        match ty {
            HirType::Function(_, _) | HirType::CallableFunction(..) => choose(
                Self::function_pointer_is_undefined(value, ty), number(1.0), number(0.0)),
            HirType::Undefined | HirType::Void => number(1.0),
            HirType::Null => number(2.0),
            HirType::Optional(payload) => choose(
                HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                number(1.0),
                self.function_observable_absence(
                    HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone()), payload)),
            HirType::Nullable(payload) => choose(
                HirExpr::NullableIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                number(2.0),
                self.function_observable_absence(
                    HirExpr::NullableValue(Box::new(value), payload.as_ref().clone()), payload)),
            HirType::Nullish(payload) => choose(
                HirExpr::NullishIsUndefined(Box::new(value.clone()), payload.as_ref().clone()),
                number(1.0),
                choose(
                    HirExpr::NullishIsNull(Box::new(value.clone()), payload.as_ref().clone()),
                    number(2.0),
                    self.function_observable_absence(
                        HirExpr::NullishValue(Box::new(value), payload.as_ref().clone()), payload),
                )),
            HirType::Union(elements) => {
                let mut result = number(0.0);
                for (index, member) in elements.iter().enumerate().rev() {
                    result = choose(
                        HirExpr::BinOp(BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(value.clone()), elements.clone())),
                            Box::new(number(index as f64))),
                        self.function_observable_absence(
                            HirExpr::UnionValue(Box::new(value.clone()), index, elements.clone()), member),
                        result,
                    );
                }
                result
            }
            HirType::Json | HirType::Dictionary(_) => choose(
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                    vec![value.clone()]), number(1.0),
                choose(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                    vec![value]), number(2.0), number(0.0))),
            HirType::JsValue => choose(
                self.dynamic_value_is_undefined(value.clone()), number(1.0),
                choose(self.dynamic_value_is_null(value), number(2.0), number(0.0))),
            _ => number(0.0),
        }
    }

    fn function_observable_typeof(&mut self, value: HirExpr, ty: &HirType) -> Result<HirExpr, String> {
        let string = |name: &str| HirExpr::Lit(HirLit::Str(name.into()));
        let choose = |test, yes, no| HirExpr::Conditional(
            Box::new(test), Box::new(yes), Box::new(no), HirType::Str,
        );
        Ok(match ty {
            HirType::Function(_, _) | HirType::CallableFunction(..) => choose(
                Self::function_pointer_is_undefined(value, ty), string("undefined"), string("function")),
            HirType::Void => string("undefined"),
            HirType::Optional(payload) => choose(
                HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                string("undefined"), self.function_observable_typeof(
                    HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone()), payload)?),
            HirType::Nullable(payload) => choose(
                HirExpr::NullableIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                string("object"), self.function_observable_typeof(
                    HirExpr::NullableValue(Box::new(value), payload.as_ref().clone()), payload)?),
            HirType::Nullish(payload) => choose(
                HirExpr::NullishIsUndefined(Box::new(value.clone()), payload.as_ref().clone()),
                string("undefined"), choose(
                    HirExpr::NullishIsNull(Box::new(value.clone()), payload.as_ref().clone()),
                    string("object"), self.function_observable_typeof(
                        HirExpr::NullishValue(Box::new(value), payload.as_ref().clone()), payload)?)),
            HirType::Union(elements) => {
                let mut result = string("undefined");
                for (index, member) in elements.iter().enumerate().rev() {
                    result = choose(
                        HirExpr::BinOp(BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(value.clone()), elements.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64)))),
                        self.function_observable_typeof(
                            HirExpr::UnionValue(Box::new(value.clone()), index, elements.clone()), member)?,
                        result,
                    );
                }
                result
            }
            HirType::Json | HirType::Dictionary(_) => HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_typeof".into())), vec![value]),
            HirType::JsValue => {
                let callable = HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![string("__thaw_typeof_dynamic_value")]);
                HirExpr::JsonAsString(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicValueWithValue".into())), vec![callable, value])))
            }
            _ => string(native_typeof_name(ty).ok_or_else(||
                format!("`typeof` union member has no runtime category: {ty:?}"))?),
        })
    }

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
            | HirType::Promise(_) => Ok(HirExpr::Lit(HirLit::Bool(true))),
            HirType::Function(_, _) | HirType::CallableFunction(..) => Ok(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(Self::function_pointer_is_undefined(value, ty)),
                Box::new(false_lit()),
            )),
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
        // Nullish left operands need their own result conversion, because
        // the falsy branch must return the original tagged value.
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

    /// `&&` returns its original left operand whenever it is falsy,
    /// including a present 0, false, empty string or NaN.
    fn lower_nullish_and(
        &mut self,
        lhs: HirExpr,
        lhs_type: HirType,
        rhs: HirExpr,
        rhs_type: HirType,
    ) -> Result<HirExpr, String> {
        let payload = match &lhs_type {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) =>
                payload.as_ref().clone(),
            _ => unreachable!(),
        };
        let name = format!("__thaw_nullish_and_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let condition = self.truthiness_expr(left.clone(), &lhs_type)?;
        let payload_always_truthy = matches!(&payload,
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
            | HirType::Dictionary(_) | HirType::Promise(_));

        fn add_member(ty: HirType, members: &mut Vec<HirType>) {
            if !members.contains(&ty) { members.push(ty); }
        }
        fn add_parts(ty: &HirType, members: &mut Vec<HirType>) {
            match ty {
                HirType::Union(parts) => for part in parts { add_parts(part, members); },
                HirType::Optional(payload) => {
                    add_parts(payload, members);
                    add_member(HirType::Undefined, members);
                }
                HirType::Nullable(payload) => {
                    add_parts(payload, members);
                    add_member(HirType::Null, members);
                }
                HirType::Nullish(payload) => {
                    add_parts(payload, members);
                    add_member(HirType::Null, members);
                    add_member(HirType::Undefined, members);
                }
                leaf => add_member(leaf.clone(), members),
            }
        }
        let mut members = Vec::new();
        if !payload_always_truthy { add_parts(&payload, &mut members); }
        match &lhs_type {
            HirType::Optional(_) => add_member(HirType::Undefined, &mut members),
            HirType::Nullable(_) => add_member(HirType::Null, &mut members),
            HirType::Nullish(_) => {
                add_member(HirType::Null, &mut members);
                add_member(HirType::Undefined, &mut members);
            }
            _ => unreachable!(),
        }
        add_parts(&rhs_type, &mut members);
        let result_type = if members.len() == 2 && members.contains(&HirType::Undefined) {
            HirType::Optional(Box::new(members.iter().find(|ty| **ty != HirType::Undefined).unwrap().clone()))
        } else if members.len() == 2 && members.contains(&HirType::Null) {
            HirType::Nullable(Box::new(members.iter().find(|ty| **ty != HirType::Null).unwrap().clone()))
        } else if members.len() == 3 && members.contains(&HirType::Null)
            && members.contains(&HirType::Undefined) {
            HirType::Nullish(Box::new(members.iter()
                .find(|ty| !matches!(ty, HirType::Null | HirType::Undefined)).unwrap().clone()))
        } else if members.len() == 1 {
            members[0].clone()
        } else {
            HirType::Union(members)
        };
        let present = self.coerce_to_declared(&result_type, rhs)?;
        let falsy = if payload_always_truthy {
            // Only the absent tags can take this branch. Preserve which
            // absent tag a Nullish left operand actually carried.
            let mut absent = |kind| self.coerce_to_declared(&result_type, HirExpr::Lit(kind));
            match &lhs_type {
                HirType::Optional(_) => absent(HirLit::Undefined)?,
                HirType::Nullable(_) => absent(HirLit::Null)?,
                HirType::Nullish(_) => HirExpr::Conditional(
                    Box::new(HirExpr::NullishIsUndefined(Box::new(left), payload)),
                    Box::new(absent(HirLit::Undefined)?),
                    Box::new(absent(HirLit::Null)?),
                    result_type.clone(),
                ),
                _ => unreachable!(),
            }
        } else {
            self.coerce_to_declared(&result_type, left)?
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(present))],
            vec![HirStmt::Return(Some(falsy))],
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
                // A strict type check here rejected a plain-typed default
                // (e.g. a `1` literal) for a `Json`-payload position --
                // real trigger: `const [a = 1] = anyArr;` where `anyArr:
                // any[]` (`array_read_type` wraps a `Json` array element
                // as `Optional<Json>` the same way it does for any other
                // element type). Coercing, not just checking, matches
                // every other "declared-type slot gets a plain value"
                // site fixed this session.
                let rhs = self.coerce_to_declared(payload.as_ref(), rhs)?;
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
                let rhs = self.coerce_to_declared(payload.as_ref(), rhs)?;
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
        // `x ?? y` where `x` is a dynamic (`any`-typed) value -- unlike
        // `Optional`/`Nullable`/`Nullish` below, whose absent/-present
        // state is a compile-time-known tag, a `Json` value's own
        // "is this null/undefined" state is only known at runtime
        // (`__thaw_json_is_nullish`, the same check `==`/`===` against a
        // bare `null`/`undefined` literal already reuses just above).
        // Previously fell through to this function's final fallback
        // (`_ => return Ok(lhs)`), silently keeping a nullish `any`
        // value instead of falling back to `y` (real trigger: `const m:
        // any = null; m ?? "default"` returning `null`, not
        // `"default"`).
        if lhs_type == HirType::Json {
            let rhs = self.coerce_to_declared(&HirType::Json, rhs)?;
            let name = format!("__thaw_nullish_json_left_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), HirType::Json);
            let left = HirExpr::Var(name.clone());
            let is_nullish = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_nullish".to_string())),
                vec![left.clone()],
            );
            let result = HirExpr::Conditional(
                Box::new(is_nullish),
                Box::new(rhs),
                Box::new(left),
                HirType::Json,
            );
            return self.wrap_call_argument_bindings(result, &[(name, HirType::Json, lhs)]);
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
            HirType::StrLiteral(_) => Ok(HirExpr::TypedClosure(HirType::Str, Box::new(value))),
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
            HirType::Json | HirType::Dynamic | HirType::Dictionary(_) => {
                Ok(HirExpr::JsonAsString(Box::new(value)))
            }
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
                if let Some(result) =
                    self.invoke_object_to_primitive(value.clone(), &fields, "string")?
                {
                    return Ok(HirExpr::JsonAsString(Box::new(result)));
                }
                if let Some(result) = self.invoke_class_to_primitive(
                    value.clone(),
                    &HirType::Object(fields.clone()),
                    "string",
                )? {
                    return Ok(HirExpr::JsonAsString(Box::new(result)));
                }
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
                        let tagged_message = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_frame".to_string())),
                            vec![HirExpr::Lit(HirLit::Str(chain)), message],
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
                    // A plain object/class instance with no `toPrimitive`
                    // falls to real JS's `OrdinaryToPrimitive`, which tries
                    // `toString()` -- inherited from `Object.prototype`
                    // absent an override, whose own algorithm reports
                    // `"[object " + tag + "]"`, consulting the receiver's
                    // own `[Symbol.toStringTag]` getter if it has one
                    // (round32's own class-getter lookup, reused here)
                    // before falling back to the constant `"[object
                    // Object]"` (`__thaw_object_to_string`) every other
                    // class/object literal already gets.
                    _ => {
                        let tag_property = well_known_symbol_key("toStringTag");
                        if let Some(owner) = self.class_instance_accessor_owner(
                            &HirType::Object(fields.clone()), &tag_property,
                        ) {
                            let symbol = class_getter_symbol(&owner, &tag_property, false);
                            if self.signatures.get(&symbol).is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str())) {
                                let tag = HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![
                                    self.assert_class_accessor_receiver(value, &owner),
                                ]);
                                return Ok(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                    vec![
                                        HirExpr::Lit(HirLit::Str("[object ".to_string())),
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_string_concat".to_string(),
                                            )),
                                            vec![tag, HirExpr::Lit(HirLit::Str("]".to_string()))],
                                        ),
                                    ],
                                ));
                            }
                        }
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_to_string".to_string())),
                            vec![value],
                        ))
                    }
                }
            }
            HirType::Array(element) => {
                let builtin = match element.as_ref() {
                    HirType::F64 => "__thaw_number_array_to_string",
                    HirType::Str => "__thaw_string_array_to_string",
                    HirType::Bool => "__thaw_bool_array_to_string",
                    HirType::Object(_) => "__thaw_object_array_to_string",
                    HirType::Optional(_)
                    | HirType::Nullable(_)
                    | HirType::Nullish(_)
                    | HirType::Undefined
                    | HirType::Null
                    | HirType::Union(_) => {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_tagged_array_join".into())),
                            vec![value, HirExpr::Lit(HirLit::Str(",".into()))],
                        ));
                    }
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
                let (tuple, binding) = if matches!(value, HirExpr::Var(_))
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
                    let part = self.coerce_join_slot_to_string(HirExpr::TypedIndex(
                        Box::new(tuple.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element.clone(),
                    ), element)?;
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

    /// Join slots use the empty string for absent elements, while
    /// ordinary `String(value)` keeps "null" and "undefined".
    fn coerce_join_slot_to_string(
        &mut self,
        value: HirExpr,
        ty: &HirType,
    ) -> Result<HirExpr, String> {
        let empty = || HirExpr::Lit(HirLit::Str(String::new()));
        match ty {
            HirType::Null | HirType::Undefined => Ok(empty()),
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                let name = format!("__thaw_join_slot_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let bound = HirExpr::Var(name.clone());
                let kind = match ty {
                    HirType::Optional(_) => AbsenceKind::Optional,
                    HirType::Nullable(_) => AbsenceKind::Nullable,
                    HirType::Nullish(_) => AbsenceKind::Nullish,
                    _ => unreachable!(),
                };
                let is_none = self.absence_is_none(kind, bound.clone(), payload.as_ref().clone());
                let present = self.absence_value(kind, bound, payload.as_ref().clone());
                let text = self.coerce_join_slot_to_string(present, payload)?;
                let result = HirExpr::Conditional(
                    Box::new(is_none),
                    Box::new(empty()),
                    Box::new(text),
                    HirType::Str,
                );
                self.wrap_call_argument_bindings(result, &[(name, ty.clone(), value)])
            }
            HirType::Union(members) => {
                let name = format!("__thaw_join_union_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let mut statements = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    let part = self.coerce_join_slot_to_string(
                        HirExpr::UnionValue(
                            Box::new(HirExpr::Var(name.clone())),
                            index,
                            members.clone(),
                        ),
                        member,
                    )?;
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
                    &[(name, ty.clone(), value)],
                )
            }
            HirType::Json | HirType::JsValue => {
                let name = format!("__thaw_join_dynamic_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let bound = HirExpr::Var(name.clone());
                let is_nullish = if *ty == HirType::Json {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_is_nullish".to_string())),
                        vec![bound.clone()],
                    )
                } else {
                    self.dynamic_value_is_nullish(bound.clone())
                };
                let present = self.coerce_primitive_to_string(bound)?;
                let result = HirExpr::Conditional(
                    Box::new(is_nullish),
                    Box::new(empty()),
                    Box::new(present),
                    HirType::Str,
                );
                self.wrap_call_argument_bindings(result, &[(name, ty.clone(), value)])
            }
            _ => self.coerce_primitive_to_string(value),
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
            let part = self.coerce_join_slot_to_string(HirExpr::TypedIndex(
                Box::new(tuple.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                element.clone(),
            ), &element)?;
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

    /// Shared by ordinary strict operators and switch case tests. Both source
    /// operands are bound before any kind dispatch, including a static false.
    fn lower_strict_equality(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        // The sparse-array index check needs the original TypedIndex shape.
        // Its helper binds the index expression first and then the undefined
        // operand, so it remains a separate source-order-preserving path.
        let sparse_index = |value: &HirExpr, other: &HirType, this: &Self| -> Result<bool, String> {
            let HirExpr::TypedIndex(array, _, _) = value else { return Ok(false); };
            if *other != HirType::Undefined { return Ok(false); }
            let HirType::Array(element) = this.infer_expr_type(array)? else { return Ok(false); };
            Ok(!matches!(element.as_ref(), HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined)
                && !matches!(element.as_ref(), HirType::Union(members) if members.contains(&HirType::Undefined)))
        };
        if sparse_index(&lhs, &rhs_type, self)? {
            if let Some(result) = self.lower_optional_undefined_equality(lhs.clone(), rhs.clone())? {
                return Ok(result);
            }
        }
        if sparse_index(&rhs, &lhs_type, self)? {
            let result = self.lower_optional_undefined_equality(
                rhs.clone(), HirExpr::Lit(HirLit::Undefined))?
                .ok_or("sparse index equality did not lower")?;
            let name = format!("__thaw_strict_undefined_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), lhs_type.clone());
            return self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)]);
        }
        let lhs_name = format!("__thaw_strict_left_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_name = format!("__thaw_strict_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_name.clone(), lhs_type.clone());
        self.scope.insert(rhs_name.clone(), rhs_type.clone());
        let left = HirExpr::Var(lhs_name.clone());
        let right = HirExpr::Var(rhs_name.clone());
        let tagged_function_pair = Self::contains_function_value(&lhs_type)
            || Self::contains_function_value(&rhs_type);
        let base = if tagged_function_pair {
            self.function_present_pair(left.clone(), &lhs_type, right.clone(), &rhs_type, true)?
        } else if let Some(result) =
            self.lower_optional_undefined_equality(left.clone(), right.clone())? {
            result
        } else if lhs_type == HirType::Json && rhs_type == HirType::Json {
            self.strict_json_pair(left, right)?
        } else if lhs_type == HirType::JsValue && rhs_type == HirType::JsValue {
            self.call_dynamic_strict_comparator("__thaw_strict_equal_dynamic", Vec::new(), vec![left, right])?
        } else if lhs_type == HirType::JsValue {
            self.strict_handle_vs_native(left, right, &rhs_type)?
        } else if rhs_type == HirType::JsValue {
            self.strict_handle_vs_native(right, left, &lhs_type)?
        } else if lhs_type == HirType::Json {
            self.strict_json_vs_native(left, right, &rhs_type)?
        } else if rhs_type == HirType::Json {
            self.strict_json_vs_native(right, left, &lhs_type)?
        } else if matches!((&lhs_type, &rhs_type),
            (HirType::StrLiteral(_), HirType::Str)
            | (HirType::Str, HirType::StrLiteral(_))
            | (HirType::StrLiteral(_), HirType::StrLiteral(_))) {
            HirExpr::BinOp(BinOp::EqEqEq,
                Box::new(HirExpr::TypedClosure(HirType::Str, Box::new(left))),
                Box::new(HirExpr::TypedClosure(HirType::Str, Box::new(right))))
        } else if matches!(&lhs_type, HirType::Function(_, _) | HirType::CallableFunction(..))
            && rhs_type == HirType::Undefined {
            Self::function_pointer_is_undefined(left, &lhs_type)
        } else if matches!(&rhs_type, HirType::Function(_, _) | HirType::CallableFunction(..))
            && lhs_type == HirType::Undefined {
            Self::function_pointer_is_undefined(right, &rhs_type)
        } else if lhs_type == rhs_type {
            HirExpr::BinOp(BinOp::EqEqEq, Box::new(left), Box::new(right))
        } else if Self::strict_reference_type(&lhs_type) && Self::strict_reference_type(&rhs_type) {
            HirExpr::BinOp(BinOp::EqEqEq, Box::new(left), Box::new(right))
        } else {
            HirExpr::Lit(HirLit::Bool(false))
        };
        let result = if Self::contains_function_value(&lhs_type)
            || Self::contains_function_value(&rhs_type) {
            let left_status_name = format!("__thaw_strict_left_status_{}", self.next_binding);
            self.next_binding += 1;
            let right_status_name = format!("__thaw_strict_right_status_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(left_status_name.clone(), HirType::F64);
            self.scope.insert(right_status_name.clone(), HirType::F64);
            let left_status = HirExpr::Var(left_status_name.clone());
            let right_status = HirExpr::Var(right_status_name.clone());
            let zero = || HirExpr::Lit(HirLit::F64(0.0));
            let both_present = HirExpr::Conditional(
                Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                    Box::new(left_status.clone()), Box::new(zero()))),
                Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                    Box::new(right_status.clone()), Box::new(zero()))),
                Box::new(HirExpr::Lit(HirLit::Bool(false))), HirType::Bool,
            );
            let left_status_expr = self.function_observable_absence(left.clone(), &lhs_type);
            let right_status_expr = self.function_observable_absence(right.clone(), &rhs_type);
            self.wrap_call_argument_bindings(
                HirExpr::Conditional(Box::new(both_present), Box::new(base),
                    Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                        Box::new(left_status), Box::new(right_status))), HirType::Bool),
                &[(left_status_name, HirType::F64, left_status_expr),
                  (right_status_name, HirType::F64, right_status_expr)],
            )?
        } else { base };
        self.wrap_call_argument_bindings(result, &[(lhs_name, lhs_type, lhs), (rhs_name, rhs_type, rhs)])
    }

    fn strict_if(condition: HirExpr, yes: HirExpr, no: HirExpr) -> HirExpr {
        HirExpr::Conditional(Box::new(condition), Box::new(yes), Box::new(no), HirType::Bool)
    }

    fn strict_json_handle(json: HirExpr) -> HirExpr {
        HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_borrowed_handle_id".into())), vec![json])
    }

    fn strict_has_handle(handle: HirExpr) -> HirExpr {
        Self::strict_if(
            HirExpr::BinOp(BinOp::EqEqEq, Box::new(handle), Box::new(HirExpr::Lit(HirLit::I64(0)))),
            HirExpr::Lit(HirLit::Bool(false)),
            HirExpr::Lit(HirLit::Bool(true)),
        )
    }

    fn strict_handle_value(handle: HirExpr) -> HirExpr {
        HirExpr::TypedClosure(HirType::JsValue, Box::new(handle))
    }

    fn strict_json_pair(&mut self, left: HirExpr, right: HirExpr) -> Result<HirExpr, String> {
        let left_handle = Self::strict_json_handle(left.clone());
        let right_handle = Self::strict_json_handle(right.clone());
        let both_handles = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_dynamic", Vec::new(),
            vec![Self::strict_handle_value(left_handle.clone()), Self::strict_handle_value(right_handle.clone())],
        )?;
        let left_to_right = self.strict_handle_vs_json(Self::strict_handle_value(left_handle.clone()), right.clone())?;
        let right_to_left = self.strict_handle_vs_json(Self::strict_handle_value(right_handle.clone()), left.clone())?;
        let native = HirExpr::BinOp(BinOp::EqEqEq, Box::new(left), Box::new(right));
        Ok(Self::strict_if(Self::strict_has_handle(left_handle),
            Self::strict_if(Self::strict_has_handle(right_handle.clone()), both_handles, left_to_right),
            Self::strict_if(Self::strict_has_handle(right_handle), right_to_left, native)))
    }

    fn strict_json_vs_native(&mut self, json: HirExpr, native: HirExpr, ty: &HirType) -> Result<HirExpr, String> {
        let handle = Self::strict_json_handle(json.clone());
        let live = self.strict_handle_vs_native(Self::strict_handle_value(handle.clone()), native.clone(), ty)?;
        let plain = match ty {
            HirType::Dictionary(_) => HirExpr::BinOp(
                BinOp::EqEqEq, Box::new(json),
                Box::new(HirExpr::TypedClosure(HirType::Json, Box::new(native))),
            ),
            HirType::I64 => HirExpr::BinOp(
                BinOp::EqEqEq, Box::new(json),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_receiver_bigint".into())),
                    vec![native],
                )),
            ),
            HirType::StrLiteral(_) => {
                let boxed = self.wrap_native_value_as_json(
                    HirExpr::TypedClosure(HirType::Str, Box::new(native)), HirType::Str)?;
                HirExpr::BinOp(BinOp::EqEqEq, Box::new(json), Box::new(boxed))
            }
            HirType::F64 | HirType::Str | HirType::Bool | HirType::Null | HirType::Undefined => {
                let boxed = self.wrap_native_value_as_json(native, ty.clone())?;
                HirExpr::BinOp(BinOp::EqEqEq, Box::new(json), Box::new(boxed))
            }
            _ => HirExpr::Lit(HirLit::Bool(false)),
        };
        Ok(Self::strict_if(Self::strict_has_handle(handle), live, plain))
    }

    fn strict_handle_vs_json(&mut self, handle: HirExpr, json: HirExpr) -> Result<HirExpr, String> {
        let borrowed = Self::strict_json_handle(json.clone());
        let both_handles = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_dynamic", Vec::new(),
            vec![handle.clone(), Self::strict_handle_value(borrowed.clone())],
        )?;
        let kind = HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_typeof".into())), vec![json.clone()]);
        let test_kind = |name: &str| HirExpr::BinOp(BinOp::EqEqEq,
            Box::new(kind.clone()), Box::new(HirExpr::Lit(HirLit::Str(name.into()))));
        let string = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_dynamic", vec![json.clone()], vec![handle.clone()])?;
        let number = self.strict_handle_vs_native(handle.clone(),
            HirExpr::JsonAsNumber(Box::new(json.clone())), &HirType::F64)?;
        let boolean = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_dynamic", vec![json.clone()], vec![handle.clone()])?;
        let bigint_digits = self.coerce_to_declared(
            &HirType::Json, HirExpr::JsonAsString(Box::new(json.clone())))?;
        let bigint = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_bigint_dynamic", vec![bigint_digits], vec![handle.clone()])?;
        let undefined = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_undefined_dynamic", Vec::new(), vec![handle.clone()])?;
        let null = self.call_dynamic_strict_comparator(
            "__thaw_strict_equal_null_dynamic", Vec::new(), vec![handle])?;
        let false_value = HirExpr::Lit(HirLit::Bool(false));
        let object = Self::strict_if(
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_is_null".into())), vec![json]),
            null, false_value);
        let native = Self::strict_if(test_kind("string"), string,
            Self::strict_if(test_kind("number"), number,
            Self::strict_if(test_kind("boolean"), boolean,
            Self::strict_if(test_kind("bigint"), bigint,
            Self::strict_if(test_kind("undefined"), undefined, object)))));
        Ok(Self::strict_if(Self::strict_has_handle(borrowed), both_handles, native))
    }

    fn strict_handle_vs_native(&mut self, handle: HirExpr, native: HirExpr, ty: &HirType) -> Result<HirExpr, String> {
        let (helper, json) = match ty {
            HirType::F64 => {
                let text = self.coerce_primitive_to_string(native)?;
                let json = self.coerce_to_declared(&HirType::Json, text)?;
                ("__thaw_strict_equal_number_dynamic", vec![json])
            }
            HirType::I64 => {
                let text = self.bigint_decimal_string(native, ty)?;
                let json = self.coerce_to_declared(&HirType::Json, text)?;
                ("__thaw_strict_equal_bigint_dynamic", vec![json])
            }
            HirType::Str => ("__thaw_strict_equal_dynamic", vec![self.coerce_to_declared(&HirType::Json, native)?]),
            HirType::StrLiteral(_) => ("__thaw_strict_equal_dynamic", vec![self.coerce_to_declared(&HirType::Json,
                HirExpr::TypedClosure(HirType::Str, Box::new(native)))?]),
            HirType::Bool => ("__thaw_strict_equal_dynamic", vec![self.coerce_to_declared(&HirType::Json, native)?]),
            HirType::Null => ("__thaw_strict_equal_null_dynamic", Vec::new()),
            HirType::Undefined => ("__thaw_strict_equal_undefined_dynamic", Vec::new()),
            HirType::Json => return self.strict_handle_vs_json(handle, native),
            HirType::Dictionary(_) => return self.strict_handle_vs_json(handle,
                HirExpr::TypedClosure(HirType::Json, Box::new(native))),
            _ => return Ok(HirExpr::Lit(HirLit::Bool(false))),
        };
        self.call_dynamic_strict_comparator(helper, json, vec![handle])
    }

    fn strict_reference_type(ty: &HirType) -> bool {
        matches!(ty, HirType::Object(_) | HirType::Dictionary(_) | HirType::Array(_)
            | HirType::Tuple(_) | HirType::Bytes | HirType::Map(_, _)
            | HirType::WeakMap(_, _) | HirType::Set(_) | HirType::WeakSet(_)
            | HirType::Promise(_) | HirType::Function(_, _) | HirType::CallableFunction(..))
    }

    fn call_dynamic_strict_comparator(&mut self, name: &str,
        json_values: Vec<HirExpr>, handles: Vec<HirExpr>) -> Result<HirExpr, String> {
        let json_args = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(json_values), HirType::Array(Box::new(HirType::Json)))?;
        let callable = HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(name.into()))]);
        Ok(HirExpr::JsonAsBool(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixed".into())),
            vec![callable, json_args, HirExpr::ArrayLit(handles)]))))
    }

    // The two original operands have already been bound. Keep both as
    // positional graph arguments: the QuickJS bridge appends handle-array
    // arguments after JSON arguments, which would reverse ToPrimitive order.
    fn function_loose_dynamic_value(
        &mut self, value: HirExpr, ty: &HirType,
    ) -> Result<(HirExpr, f64), String> {
        let (value, kind) = match ty {
            HirType::Function(_, _) | HirType::CallableFunction(..) => (value, 3.0),
            HirType::I64 => (self.bigint_decimal_string(value, ty)?, 1.0),
            HirType::Symbol => (HirExpr::TypedClosure(HirType::Str, Box::new(value)), 2.0),
            HirType::StrLiteral(_) =>
                (HirExpr::TypedClosure(HirType::Str, Box::new(value)), 0.0),
            HirType::Dictionary(_) =>
                (HirExpr::TypedClosure(HirType::Json, Box::new(value)), 0.0),
            _ => (value, 0.0),
        };
        Ok((self.coerce_to_declared(&HirType::Json, value)?, kind))
    }

    fn function_dynamic_pair(
        &mut self, lhs: HirExpr, lhs_ty: &HirType,
        rhs: HirExpr, rhs_ty: &HirType, strict: bool,
    ) -> Result<HirExpr, String> {
        let (left, left_kind) = self.function_loose_dynamic_value(lhs, lhs_ty)?;
        let (right, right_kind) = self.function_loose_dynamic_value(rhs, rhs_ty)?;
        let left_flag = self.coerce_to_declared(&HirType::Json,
            HirExpr::Lit(HirLit::F64(left_kind)))?;
        let right_flag = self.coerce_to_declared(&HirType::Json,
            HirExpr::Lit(HirLit::F64(right_kind)))?;
        let arguments = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(vec![left, right, left_flag, right_flag]),
            HirType::Array(Box::new(HirType::Json)),
        )?;
        let callable = HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(if strict {
                "__thaw_strict_equal_dynamic"
            } else {
                "__thaw_native_loose_equal"
            }.into()))]);
        Ok(HirExpr::JsonAsBool(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixed".into())),
            vec![callable, arguments, HirExpr::ArrayLit(Vec::new())],
        ))))
    }

    fn function_present_pair(
        &mut self, lhs: HirExpr, lhs_ty: &HirType,
        rhs: HirExpr, rhs_ty: &HirType, strict: bool,
    ) -> Result<HirExpr, String> {
        let choose = |test, yes, no| HirExpr::Conditional(
            Box::new(test), Box::new(yes), Box::new(no), HirType::Bool);
        match lhs_ty {
            HirType::Optional(payload) => return self.function_present_pair(
                HirExpr::OptionalValue(Box::new(lhs), payload.as_ref().clone()), payload,
                rhs, rhs_ty, strict),
            HirType::Nullable(payload) => return self.function_present_pair(
                HirExpr::NullableValue(Box::new(lhs), payload.as_ref().clone()), payload,
                rhs, rhs_ty, strict),
            HirType::Nullish(payload) => return self.function_present_pair(
                HirExpr::NullishValue(Box::new(lhs), payload.as_ref().clone()), payload,
                rhs, rhs_ty, strict),
            HirType::Union(members) => {
                let mut result = HirExpr::Lit(HirLit::Bool(false));
                for (index, member) in members.iter().enumerate().rev() {
                    let active = HirExpr::BinOp(BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(lhs.clone()), members.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))));
                    result = choose(active, self.function_present_pair(
                        HirExpr::UnionValue(Box::new(lhs.clone()), index, members.clone()), member,
                        rhs.clone(), rhs_ty, strict)?, result);
                }
                return Ok(result);
            }
            _ => {}
        }
        match rhs_ty {
            HirType::Optional(payload) => return self.function_present_pair(
                lhs, lhs_ty, HirExpr::OptionalValue(Box::new(rhs), payload.as_ref().clone()), payload, strict),
            HirType::Nullable(payload) => return self.function_present_pair(
                lhs, lhs_ty, HirExpr::NullableValue(Box::new(rhs), payload.as_ref().clone()), payload, strict),
            HirType::Nullish(payload) => return self.function_present_pair(
                lhs, lhs_ty, HirExpr::NullishValue(Box::new(rhs), payload.as_ref().clone()), payload, strict),
            HirType::Union(members) => {
                let mut result = HirExpr::Lit(HirLit::Bool(false));
                for (index, member) in members.iter().enumerate().rev() {
                    let active = HirExpr::BinOp(BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(rhs.clone()), members.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))));
                    result = choose(active, self.function_present_pair(
                        lhs.clone(), lhs_ty,
                        HirExpr::UnionValue(Box::new(rhs.clone()), index, members.clone()), member, strict)?,
                        result);
                }
                return Ok(result);
            }
            _ => {}
        }
        let left_fn = matches!(lhs_ty, HirType::Function(_, _) | HirType::CallableFunction(..));
        let right_fn = matches!(rhs_ty, HirType::Function(_, _) | HirType::CallableFunction(..));
        if left_fn && right_fn {
            return Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)));
        }
        if left_fn || right_fn {
            let other = if left_fn { rhs_ty } else { lhs_ty };
            if strict {
                return if matches!(other, HirType::Json | HirType::JsValue | HirType::Dictionary(_)) {
                    self.function_dynamic_pair(lhs, lhs_ty, rhs, rhs_ty, true)
                } else {
                    Ok(HirExpr::Lit(HirLit::Bool(false)))
                };
            }
            // Both native reference values are distinct allocations. JS `==`
            // between references does not invoke ToPrimitive. Live host objects
            // remain JsValue/Json and take the graph bridge below.
            if matches!(other, HirType::Object(_)
                | HirType::Array(_) | HirType::Tuple(_) | HirType::Bytes
                | HirType::Map(_, _) | HirType::WeakMap(_, _)
                | HirType::Set(_) | HirType::WeakSet(_) | HirType::Promise(_)) {
                return Ok(HirExpr::Lit(HirLit::Bool(false)));
            }
            if matches!(other, HirType::Void | HirType::Undefined | HirType::Null) {
                return Ok(HirExpr::Lit(HirLit::Bool(false)));
            }
            return self.function_dynamic_pair(lhs, lhs_ty, rhs, rhs_ty, strict);
        }
        if strict {
            return self.lower_strict_equality(lhs, rhs);
        }
        // These leaves are reached only through an active tagged member.
        // Keep the established native coercion for source-supported pairs;
        // native Symbol/I64/JsValue/Json use the exact graph bridge instead.
        if matches!(lhs_ty, HirType::F64 | HirType::Str | HirType::Bool)
            && matches!(rhs_ty, HirType::F64 | HirType::Str | HirType::Bool) {
            return self.lower_loose_equality_base(lhs, rhs);
        }
        if matches!(lhs_ty, HirType::F64 | HirType::Str | HirType::StrLiteral(_)
            | HirType::Bool | HirType::I64 | HirType::Symbol | HirType::Json | HirType::JsValue
            | HirType::Dictionary(_))
            && matches!(rhs_ty, HirType::F64 | HirType::Str | HirType::StrLiteral(_)
            | HirType::Bool | HirType::I64 | HirType::Symbol | HirType::Json | HirType::JsValue
            | HirType::Dictionary(_)) {
            return self.function_dynamic_pair(lhs, lhs_ty, rhs, rhs_ty, strict);
        }
        self.lower_loose_equality_base(lhs, rhs)
    }

    fn lower_loose_equality(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if !Self::contains_function_value(&lhs_type)
            && !Self::contains_function_value(&rhs_type) {
            return self.lower_loose_equality_base(lhs, rhs);
        }
        let lhs_name = format!("__thaw_loose_left_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_name = format!("__thaw_loose_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_name.clone(), lhs_type.clone());
        self.scope.insert(rhs_name.clone(), rhs_type.clone());
        let left = HirExpr::Var(lhs_name.clone());
        let right = HirExpr::Var(rhs_name.clone());
        let base = self.function_present_pair(left.clone(), &lhs_type, right.clone(), &rhs_type, false)?;
        let lhs_status_name = format!("__thaw_loose_left_status_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_status_name = format!("__thaw_loose_right_status_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_status_name.clone(), HirType::F64);
        self.scope.insert(rhs_status_name.clone(), HirType::F64);
        let lhs_status = HirExpr::Var(lhs_status_name.clone());
        let rhs_status = HirExpr::Var(rhs_status_name.clone());
        let is_absent = |value: HirExpr| HirExpr::BinOp(BinOp::EqEqEq,
            Box::new(HirExpr::BinOp(BinOp::EqEqEq,
                Box::new(value), Box::new(HirExpr::Lit(HirLit::F64(0.0))))),
            Box::new(HirExpr::Lit(HirLit::Bool(false))));
        let left_absent = is_absent(lhs_status.clone());
        let right_absent = is_absent(rhs_status.clone());
        let result = HirExpr::Conditional(Box::new(left_absent.clone()),
            Box::new(right_absent.clone()),
            Box::new(HirExpr::Conditional(Box::new(right_absent),
                Box::new(HirExpr::Lit(HirLit::Bool(false))), Box::new(base), HirType::Bool)),
            HirType::Bool);
        let lhs_status_expr = self.function_observable_absence(left, &lhs_type);
        let rhs_status_expr = self.function_observable_absence(right, &rhs_type);
        let result = self.wrap_call_argument_bindings(result,
            &[(lhs_status_name, HirType::F64, lhs_status_expr),
              (rhs_status_name, HirType::F64, rhs_status_expr)])?;
        self.wrap_call_argument_bindings(result,
            &[(lhs_name, lhs_type, lhs), (rhs_name, rhs_type, rhs)])
    }

    fn lower_loose_equality_base(
        &mut self,
        mut lhs: HirExpr,
        mut rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type == rhs_type {
            return Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)));
        }
        // A raw-null Function is JavaScript `undefined`, so it loosely
        // equals both `undefined` and `null`. Bind both operands first to
        // preserve their effects and order, even when one is statically nullish.
        let nullish_function = match (&lhs_type, &rhs_type) {
            (ty @ (HirType::Function(_, _) | HirType::CallableFunction(..)),
                HirType::Null | HirType::Undefined) => Some((true, (*ty).clone())),
            (HirType::Null | HirType::Undefined,
                ty @ (HirType::Function(_, _) | HirType::CallableFunction(..))) => {
                Some((false, (*ty).clone()))
            }
            _ => None,
        };
        if let Some((function_left, function_type)) = nullish_function {
            let left_name = format!("__thaw_loose_function_left_{}", self.next_binding);
            self.next_binding += 1;
            let right_name = format!("__thaw_loose_function_right_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(left_name.clone(), lhs_type.clone());
            self.scope.insert(right_name.clone(), rhs_type.clone());
            let function = HirExpr::Var(if function_left { left_name.clone() } else { right_name.clone() });
            return self.wrap_call_argument_bindings(
                Self::function_pointer_is_undefined(function, &function_type),
                &[(left_name, lhs_type, lhs), (right_name, rhs_type, rhs)],
            );
        }
        // A native `bigint` against a beyond-`i64` one (a live handle):
        // equal exactly when their decimal digits agree.
        if matches!(
            (&lhs_type, &rhs_type),
            (HirType::JsValue, HirType::I64) | (HirType::I64, HirType::JsValue)
        ) {
            let left = self.bigint_decimal_string(lhs, &lhs_type)?;
            let right = self.bigint_decimal_string(rhs, &rhs_type)?;
            let comparison = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bigint_decimal_cmp".into())),
                vec![left, right],
            );
            return Ok(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(comparison),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            ));
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
        let union = match (&lhs_type, &rhs_type) {
            (HirType::Union(members), HirType::Null | HirType::Undefined) => {
                Some((members.clone(), true))
            }
            (HirType::Null | HirType::Undefined, HirType::Union(members)) => {
                Some((members.clone(), false))
            }
            _ => None,
        };
        if let Some((members, union_left)) = union {
            let left_name = format!("__thaw_loose_union_left_{}", self.next_binding);
            self.next_binding += 1;
            let right_name = format!("__thaw_loose_union_right_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(left_name.clone(), lhs_type.clone());
            self.scope.insert(right_name.clone(), rhs_type.clone());
            let union_name = if union_left { &left_name } else { &right_name };
            let mut result = HirExpr::Lit(HirLit::Bool(false));
            for (index, member) in members.iter().enumerate() {
                if matches!(member, HirType::Null | HirType::Undefined) {
                    result = self.lower_logical_expr(
                        result,
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(
                                Box::new(HirExpr::Var(union_name.clone())),
                                members.clone(),
                            )),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        false,
                    )?;
                }
            }
            return self.wrap_call_argument_bindings(
                result,
                &[(left_name, lhs_type, lhs), (right_name, rhs_type, rhs)],
            );
        }
        if matches!(lhs_type, HirType::Null | HirType::Undefined)
            || matches!(rhs_type, HirType::Null | HirType::Undefined)
        {
            let left_name = format!("__thaw_loose_non_nullish_left_{}", self.next_binding);
            self.next_binding += 1;
            let right_name = format!("__thaw_loose_non_nullish_right_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(left_name.clone(), lhs_type.clone());
            self.scope.insert(right_name.clone(), rhs_type.clone());
            return self.wrap_call_argument_bindings(
                HirExpr::Lit(HirLit::Bool(false)),
                &[(left_name, lhs_type, lhs), (right_name, rhs_type, rhs)],
            );
        }
        lhs = self.coerce_primitive_to_number(lhs)?;
        rhs = self.coerce_primitive_to_number(rhs)?;
        Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)))
    }

    fn coerce_primitive_to_number(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        self.coerce_primitive_to_number_impl(value, false)
    }

    // Binary `+` needs to build its numeric branch even when a different
    // runtime branch concatenates a string. Keep BigInt/Symbol failures local
    // to that branch; ordinary numeric coercions retain their existing ABI.
    fn coerce_primitive_to_number_for_add(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        self.coerce_primitive_to_number_impl(value, true)
    }

    fn coerce_primitive_to_number_impl(
        &mut self, value: HirExpr, for_add: bool,
    ) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::I64 if for_add => Ok(HirExpr::ThrowValue(
                Box::new(HirExpr::Lit(HirLit::Str(
                    "\u{1}TypeError\u{1}Cannot mix BigInt and other types".into(),
                ))),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )),
            HirType::Symbol if for_add => Ok(HirExpr::ThrowValue(
                Box::new(HirExpr::Lit(HirLit::Str(
                    "\u{1}TypeError\u{1}Cannot convert a Symbol value to a number".into(),
                ))),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )),
            HirType::F64 => Ok(value),
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                vec![value],
            )),
            HirType::Str => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                vec![value],
            )),
            HirType::StrLiteral(_) => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                vec![HirExpr::TypedClosure(HirType::Str, Box::new(value))],
            )),
            HirType::Json => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::JsValue => Ok(HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                vec![value],
            )))),
            HirType::Object(fields) => {
                // A `Date` is `{ timestamp }`; `ToNumber(date)` is its
                // `valueOf()` (the timestamp), so `date1 < date2` and
                // `date1 - date2` order by time.
                if fields.len() == 1 && fields[0].0 == "timestamp" && fields[0].1 == HirType::F64 {
                    return Ok(HirExpr::PropAccess(
                        Box::new(value),
                        HirType::Object(fields),
                        "timestamp".to_string(),
                    ));
                }
                if let Some(result) =
                    self.invoke_object_to_primitive(value.clone(), &fields, "number")?
                {
                    return Ok(HirExpr::JsonAsNumber(Box::new(result)));
                }
                if let Some(result) = self.invoke_class_to_primitive(
                    value.clone(),
                    &HirType::Object(fields.clone()),
                    "number",
                )? {
                    return Ok(HirExpr::JsonAsNumber(Box::new(result)));
                }
                let string = self.coerce_primitive_to_string(value)?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                    vec![string],
                ))
            }
            HirType::Array(_) | HirType::Tuple(_) => {
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
            HirType::Optional(payload) => {
                let optional_type = HirType::Optional(payload.clone());
                let name = format!("__thaw_number_optional_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), optional_type.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_number_impl(HirExpr::OptionalValue(
                    Box::new(bound.clone()),
                    payload.as_ref().clone(),
                ), for_add)?;
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(f64::NAN))))],
                    vec![HirStmt::Return(Some(present))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, optional_type, value)])
            }
            HirType::Nullable(payload) => {
                let ty = HirType::Nullable(payload.clone());
                let name = format!("__thaw_number_nullable_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_number_impl(HirExpr::NullableValue(
                    Box::new(bound.clone()), payload.as_ref().clone(),
                ), for_add)?;
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(0.0))))],
                    vec![HirStmt::Return(Some(present))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, ty, value)])
            }
            HirType::Nullish(payload) => {
                let ty = HirType::Nullish(payload.clone());
                let name = format!("__thaw_number_nullish_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let bound = HirExpr::Var(name.clone());
                let present = self.coerce_primitive_to_number_impl(HirExpr::NullishValue(
                    Box::new(bound.clone()), payload.as_ref().clone(),
                ), for_add)?;
                let result = HirExpr::Block(vec![
                    HirStmt::If(
                        HirExpr::NullishIsNull(Box::new(bound.clone()), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(0.0))))],
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(Box::new(bound), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(f64::NAN))))],
                        vec![HirStmt::Return(Some(present))],
                    ),
                ]);
                self.wrap_call_argument_bindings(result, &[(name, ty, value)])
            }
            HirType::Union(members) => {
                let ty = HirType::Union(members.clone());
                let name = format!("__thaw_number_union_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let mut statements = Vec::with_capacity(members.len());
                for index in 0..members.len() {
                    let part = self.coerce_primitive_to_number_impl(HirExpr::UnionValue(
                        Box::new(HirExpr::Var(name.clone())), index, members.clone(),
                    ), for_add)?;
                    if index + 1 == members.len() {
                        statements.push(HirStmt::Return(Some(part)));
                    } else {
                        statements.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(HirExpr::Var(name.clone())), members.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Return(Some(part))],
                            Vec::new(),
                        ));
                    }
                }
                self.wrap_call_argument_bindings(
                    HirExpr::Block(statements), &[(name, ty, value)],
                )
            }
            other => Err(format!(
                "numeric conversion is not defined for native type {other:?}"
            )),
        }
    }

    // The graph encoder has no native I64 field. Reconstruct a BigInt
    // primitive from its exact decimal digits using the already-registered
    // host constructor, then let the host `+` operator add two BigInts.
    fn add_bigint_live_handle(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let decimal = self.coerce_primitive_to_string(value)?;
        let arguments = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(vec![decimal]), HirType::Array(Box::new(HirType::Str)),
        )?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
            vec![
                HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("BigInt".into()))],
                ),
                arguments,
            ],
        ))
    }

    fn add_bigint_pair_as_json(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_handle = format!("__thaw_add_bigint_lhs_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_handle = format!("__thaw_add_bigint_rhs_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_handle.clone(), HirType::JsValue);
        self.scope.insert(rhs_handle.clone(), HirType::JsValue);
        let empty_arguments = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(Vec::new()), HirType::Array(Box::new(HirType::Json)),
        )?;
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("readDynamicValue".into())),
            vec![HirExpr::Call(
                Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())),
                vec![
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str("__thaw_dynamic_add".into()))],
                    ),
                    empty_arguments,
                    HirExpr::ArrayLit(vec![
                        HirExpr::Var(lhs_handle.clone()), HirExpr::Var(rhs_handle.clone()),
                    ]),
                ],
            )],
        );
        let lhs_live = self.add_bigint_live_handle(lhs)?;
        let rhs_live = self.add_bigint_live_handle(rhs)?;
        self.wrap_call_argument_bindings(result, &[
            (lhs_handle, HirType::JsValue, lhs_live),
            (rhs_handle, HirType::JsValue, rhs_live),
        ])
    }

    /// `ECMA-262`'s own `ToPrimitive(value)` (no hint override -- the
    /// default the spec's `+` operator itself uses) for a single binary
    /// `+` operand: an `Object`-typed operand with a `[Symbol.toPrimitive]`
    /// method is called *once*, with hint `"default"`, producing a `Json`
    /// value; anything else is already primitive and passed through
    /// unchanged. Errors for an `Object`-typed operand with no such method
    /// -- `OrdinaryToPrimitive`'s `valueOf`/`toString` fallback chain isn't
    /// implemented, matching this compiler's existing, narrower scope
    /// (round28/30 only ever added the explicit `Symbol.toPrimitive` path).
    fn add_operand_to_primitive(&mut self, value: HirExpr) -> Result<(HirExpr, HirType), String> {
        let ty = self.infer_expr_type(&value)?;
        // `any` fields normalized from constructor-function statics are
        // Optional<Json>.  Convert the optional tag to the existing JSON
        // undefined sentinel before the shared runtime string/number test.
        if matches!(&ty, HirType::Optional(inner) | HirType::Nullable(inner)
            | HirType::Nullish(inner) if inner.as_ref() == &HirType::Json) {
            let json = self.coerce_to_declared(&HirType::Json, value)?;
            return Ok((json, HirType::Json));
        }
        let HirType::Object(fields) = &ty else {
            return Ok((value, ty));
        };
        if let Some(result) = self.invoke_object_to_primitive(value.clone(), fields, "default")? {
            return Ok((result, HirType::Json));
        }
        if let Some(result) = self.invoke_class_to_primitive(value.clone(), &ty, "default")? {
            return Ok((result, HirType::Json));
        }
        Err(format!(
            "binary `+` on an object requires a `[Symbol.toPrimitive]` method, got {ty:?}"
        ))
    }

    /// Binary `+` when at least one operand is a statically `Object`-typed
    /// value with its own `[Symbol.toPrimitive]` (a class instance or
    /// object literal) -- unlike unary `+`/string coercion (each with one
    /// *fixed*, compile-time-known hint), `+`'s own algorithm calls
    /// `ToPrimitive` with hint `"default"` *once* per operand, then decides
    /// string-concat vs. numeric-add from the *actual runtime type* of
    /// each result. A statically `Str` operand also selects concat when
    /// this helper is reached from a Json-backed compound assignment.
    /// A class can legitimately return a different value
    /// for `"default"` than for `"string"`/`"number"` (confirmed real
    /// trigger: a `Money` class's `toPrimitive` returns the formatted
    /// `` `Money(${amount})` `` string for `"default"` alone). Builds the
    /// real spec dispatch: if either resulting primitive is a string
    /// (including a present optional string or a literal-typed string),
    /// stringify-and-concat both; otherwise numeric-add both (with two
    /// BigInts delegated to the existing host operator). Both branches reuse the existing, generic
    /// `coerce_primitive_to_string`/`coerce_primitive_to_number` (already
    /// handling a `Json` result via a plain decode, no second `ToPrimitive`
    /// call).
    fn add_operand_may_be_string(ty: &HirType) -> bool {
        match ty {
            HirType::Str | HirType::StrLiteral(_) => true,
            HirType::Optional(payload) | HirType::Nullable(payload)
            | HirType::Nullish(payload) => Self::add_operand_may_be_string(payload),
            HirType::Union(members) => members.iter().any(Self::add_operand_may_be_string),
            _ => false,
        }
    }

    fn add_operand_may_be_bigint(ty: &HirType) -> bool {
        match ty {
            HirType::I64 | HirType::Json => true,
            HirType::Optional(payload) | HirType::Nullable(payload)
            | HirType::Nullish(payload) => Self::add_operand_may_be_bigint(payload),
            HirType::Union(members) => members.iter().any(Self::add_operand_may_be_bigint),
            _ => false,
        }
    }

    // The native `+` decision is made after both operand values have been
    // evaluated and each has undergone ToPrimitive in left-to-right order.
    // Tagged string wrappers are only strings when their payload is present.
    fn add_operand_has_primitive_type(
        &mut self, value: HirExpr, ty: &HirType, primitive: &str,
    ) -> HirExpr {
        let bool_lit = |value| HirExpr::Lit(HirLit::Bool(value));
        let choose = |test, yes, no| HirExpr::Conditional(
            Box::new(test), Box::new(yes), Box::new(no), HirType::Bool,
        );
        match ty {
            HirType::Str | HirType::StrLiteral(_) => bool_lit(primitive == "string"),
            HirType::Symbol => bool_lit(primitive == "symbol"),
            HirType::I64 => bool_lit(primitive == "bigint"),
            HirType::Json => HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_typeof".into())),
                    vec![value],
                )),
                Box::new(HirExpr::Lit(HirLit::Str(primitive.into()))),
            ),
            HirType::Optional(payload) => choose(
                HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                bool_lit(false),
                self.add_operand_has_primitive_type(
                    HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone()), payload, primitive,
                ),
            ),
            HirType::Nullable(payload) => choose(
                HirExpr::NullableIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                bool_lit(false),
                self.add_operand_has_primitive_type(
                    HirExpr::NullableValue(Box::new(value), payload.as_ref().clone()), payload, primitive,
                ),
            ),
            HirType::Nullish(payload) => choose(
                HirExpr::NullishIsUndefined(Box::new(value.clone()), payload.as_ref().clone()),
                bool_lit(false),
                choose(
                    HirExpr::NullishIsNull(Box::new(value.clone()), payload.as_ref().clone()),
                    bool_lit(false),
                    self.add_operand_has_primitive_type(
                        HirExpr::NullishValue(Box::new(value), payload.as_ref().clone()), payload, primitive,
                    ),
                ),
            ),
            HirType::Union(members) => {
                let mut result = bool_lit(false);
                for (index, member) in members.iter().enumerate().rev() {
                    result = choose(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(value.clone()), members.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        self.add_operand_has_primitive_type(
                            HirExpr::UnionValue(Box::new(value.clone()), index, members.clone()),
                            member, primitive,
                        ),
                        result,
                    );
                }
                result
            }
            _ => bool_lit(false),
        }
    }

    fn lower_add_with_to_primitive(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        // The value of each operand precedes *both* ToPrimitive calls. In
        // particular, a left Symbol.toPrimitive hook must not run before a
        // right-hand call expression has completed.
        let lhs_raw_ty = self.infer_expr_type(&lhs)?;
        let rhs_raw_ty = self.infer_expr_type(&rhs)?;
        let lhs_raw_name = format!("__thaw_add_raw_lhs_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_raw_name = format!("__thaw_add_raw_rhs_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_raw_name.clone(), lhs_raw_ty.clone());
        self.scope.insert(rhs_raw_name.clone(), rhs_raw_ty.clone());
        let (lhs_prim, lhs_ty) = self.add_operand_to_primitive(HirExpr::Var(lhs_raw_name.clone()))?;
        let (rhs_prim, rhs_ty) = self.add_operand_to_primitive(HirExpr::Var(rhs_raw_name.clone()))?;
        let lhs_name = format!("__thaw_add_to_primitive_lhs_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_name = format!("__thaw_add_to_primitive_rhs_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_name.clone(), lhs_ty.clone());
        self.scope.insert(rhs_name.clone(), rhs_ty.clone());
        let guaranteed_string = matches!(&lhs_raw_ty, HirType::Str | HirType::StrLiteral(_))
            || matches!(&rhs_raw_ty, HirType::Str | HirType::StrLiteral(_));
        let concat = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_string_concat".into())),
            vec![
                self.coerce_primitive_to_string(HirExpr::Var(lhs_name.clone()))?,
                self.coerce_primitive_to_string(HirExpr::Var(rhs_name.clone()))?,
            ],
        );
        let result = if guaranteed_string {
            // A guaranteed string cannot need numeric conversion (including
            // BigInt), but implicit `+` still throws on a Symbol operand.
            let lhs_symbol = self.add_operand_has_primitive_type(
                HirExpr::Var(lhs_name.clone()), &lhs_ty, "symbol",
            );
            let rhs_symbol = self.add_operand_has_primitive_type(
                HirExpr::Var(rhs_name.clone()), &rhs_ty, "symbol",
            );
            let symbol = self.lower_logical_expr(lhs_symbol, rhs_symbol, false)?;
            let invalid_symbol = HirExpr::ThrowValue(
                Box::new(HirExpr::Lit(HirLit::Str(
                    "\u{1}TypeError\u{1}Cannot convert a Symbol value to a string".into(),
                ))),
                Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
            );
            HirExpr::Conditional(
                Box::new(symbol), Box::new(invalid_symbol), Box::new(concat), HirType::Str,
            )
        } else {
            let lhs_is_string = self.add_operand_has_primitive_type(
                HirExpr::Var(lhs_name.clone()), &lhs_ty, "string",
            );
            let rhs_is_string = self.add_operand_has_primitive_type(
                HirExpr::Var(rhs_name.clone()), &rhs_ty, "string",
            );
            let either_is_string = self.lower_logical_expr(
                lhs_is_string, rhs_is_string, false,
            )?;
            let numeric = HirExpr::BinOp(
                BinOp::Add,
                Box::new(self.coerce_primitive_to_number_for_add(HirExpr::Var(lhs_name.clone()))?),
                Box::new(self.coerce_primitive_to_number_for_add(HirExpr::Var(rhs_name.clone()))?),
            );
            let concat = self.coerce_to_declared(&HirType::Json, concat)?;
            let numeric = self.coerce_to_declared(&HirType::Json, numeric)?;
            let lhs_bigint = self.add_operand_has_primitive_type(
                HirExpr::Var(lhs_name.clone()), &lhs_ty, "bigint",
            );
            let rhs_bigint = self.add_operand_has_primitive_type(
                HirExpr::Var(rhs_name.clone()), &rhs_ty, "bigint",
            );
            let either_bigint = self.lower_logical_expr(lhs_bigint, rhs_bigint, false)?;
            let numeric = HirExpr::Conditional(
                Box::new(either_bigint),
                Box::new(HirExpr::ThrowValue(
                    Box::new(HirExpr::Lit(HirLit::Str(
                        "\u{1}TypeError\u{1}Cannot mix BigInt and other types".into(),
                    ))),
                    Box::new(Self::unreachable_value(&HirType::Json)?),
                )),
                Box::new(numeric),
                HirType::Json,
            );
            // Building the host arm at all makes an otherwise native-only
            // module depend on QuickJS. Only two bigint-capable static types
            // can ever reach this path at runtime.
            let numeric = if Self::add_operand_may_be_bigint(&lhs_ty)
                && Self::add_operand_may_be_bigint(&rhs_ty)
            {
                let lhs_bigint = self.add_operand_has_primitive_type(
                    HirExpr::Var(lhs_name.clone()), &lhs_ty, "bigint",
                );
                let rhs_bigint = self.add_operand_has_primitive_type(
                    HirExpr::Var(rhs_name.clone()), &rhs_ty, "bigint",
                );
                let both_bigint = self.lower_logical_expr(lhs_bigint, rhs_bigint, true)?;
                let bigint_sum = self.add_bigint_pair_as_json(
                    HirExpr::Var(lhs_name.clone()), HirExpr::Var(rhs_name.clone()),
                )?;
                HirExpr::Conditional(
                    Box::new(both_bigint), Box::new(bigint_sum), Box::new(numeric),
                    HirType::Json,
                )
            } else {
                numeric
            };
            let selected = HirExpr::Conditional(
                Box::new(either_is_string), Box::new(concat), Box::new(numeric),
                HirType::Json,
            );
            // `String(Symbol)` is legal, but implicit `+` never uses that
            // conversion, including when its other operand is a string.
            let lhs_symbol = self.add_operand_has_primitive_type(
                HirExpr::Var(lhs_name.clone()), &lhs_ty, "symbol",
            );
            let rhs_symbol = self.add_operand_has_primitive_type(
                HirExpr::Var(rhs_name.clone()), &rhs_ty, "symbol",
            );
            let symbol = self.lower_logical_expr(lhs_symbol, rhs_symbol, false)?;
            HirExpr::Conditional(
                Box::new(symbol),
                Box::new(HirExpr::ThrowValue(
                    Box::new(HirExpr::Lit(HirLit::Str(
                        "\u{1}TypeError\u{1}Cannot convert a Symbol value to a string".into(),
                    ))),
                    Box::new(Self::unreachable_value(&HirType::Json)?),
                )),
                Box::new(selected),
                HirType::Json,
            )
        };
        let after_values = self.wrap_call_argument_bindings(
            result,
            &[(lhs_name, lhs_ty, lhs_prim), (rhs_name, rhs_ty, rhs_prim)],
        )?;
        self.wrap_call_argument_bindings(after_values, &[
            (lhs_raw_name, lhs_raw_ty, lhs),
            (rhs_raw_name, rhs_raw_ty, rhs),
        ])
    }

    /// A Json-backed value plus a live JS value must execute `+` in that
    /// JS realm. Both operands stay in their original graph positions: the
    /// mixed-call ABI appends separate handle arguments after JSON arguments,
    /// which would reverse ToPrimitive order for a live left operand.
    fn lower_add_with_live_json(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let lhs_ty = self.infer_expr_type(&lhs)?;
        let rhs_ty = self.infer_expr_type(&rhs)?;
        let lhs_name = format!("__thaw_add_live_lhs_{}", self.next_binding);
        self.next_binding += 1;
        let rhs_name = format!("__thaw_add_live_rhs_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(lhs_name.clone(), lhs_ty.clone());
        self.scope.insert(rhs_name.clone(), rhs_ty.clone());
        let left = if matches!(&lhs_ty, HirType::StrLiteral(_)) {
            HirExpr::TypedClosure(HirType::Str, Box::new(HirExpr::Var(lhs_name.clone())))
        } else {
            HirExpr::Var(lhs_name.clone())
        };
        let right = if matches!(&rhs_ty, HirType::StrLiteral(_)) {
            HirExpr::TypedClosure(HirType::Str, Box::new(HirExpr::Var(rhs_name.clone())))
        } else {
            HirExpr::Var(rhs_name.clone())
        };
        let left = self.coerce_to_declared(&HirType::Json, left)?;
        let right = self.coerce_to_declared(&HirType::Json, right)?;
        let arguments = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(vec![left, right]),
            HirType::Array(Box::new(HirType::Json)),
        )?;
        let callable = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str("__thaw_dynamic_add".into()))],
        );
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixed".into())),
            vec![callable, arguments, HirExpr::ArrayLit(Vec::new())],
        );
        self.wrap_call_argument_bindings(result, &[
            (lhs_name, lhs_ty, lhs),
            (rhs_name, rhs_ty, rhs),
        ])
    }

    /// The decimal digits of a `bigint`-like operand: a native `i64` via
    /// `thaw_i64_to_string`, or a live `JsValue` (a `bigint` beyond `i64`,
    /// kept as a handle) via its own `toString()`. Used to compare a
    /// fixed-width native `bigint` against one outside its range.
    fn bigint_decimal_string(
        &mut self,
        value: HirExpr,
        ty: &HirType,
    ) -> Result<HirExpr, String> {
        match ty {
            HirType::I64 => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_i64_to_string".into())),
                vec![value],
            )),
            HirType::JsValue => {
                let no_args =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(Vec::new()))?;
                let call = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethod".into())),
                    vec![
                        value,
                        HirExpr::Lit(HirLit::Str("toString".into())),
                        no_args,
                    ],
                );
                Ok(HirExpr::JsonAsString(Box::new(call)))
            }
            other => Err(format!(
                "a bigint comparison needs a native `bigint` or a dynamic bigint, got {other:?}"
            )),
        }
    }

    /// `+`/`-`/`*`/`/`/`%` between a native `bigint` and a beyond-`i64` one
    /// (a live handle), which the ordinary numeric path rejects. Both
    /// sides become decimal strings, the operation runs in the runtime
    /// (arbitrary precision), and the digits are wrapped back into a real
    /// BigInt handle so chained arithmetic keeps working.
    fn lower_bigint_arithmetic(
        &mut self,
        lhs: &HirExpr,
        rhs: &HirExpr,
        op: BinaryOp,
    ) -> Result<Option<HirExpr>, String> {
        let intrinsic = match op {
            BinaryOp::Add => "__thaw_bigint_decimal_add",
            BinaryOp::Sub => "__thaw_bigint_decimal_sub",
            BinaryOp::Mul => "__thaw_bigint_decimal_mul",
            BinaryOp::Div => "__thaw_bigint_decimal_div",
            BinaryOp::Mod => "__thaw_bigint_decimal_mod",
            BinaryOp::BitAnd => "__thaw_bigint_decimal_and",
            BinaryOp::BitOr => "__thaw_bigint_decimal_or",
            BinaryOp::BitXor => "__thaw_bigint_decimal_xor",
            BinaryOp::LShift => "__thaw_bigint_decimal_shl",
            BinaryOp::RShift => "__thaw_bigint_decimal_shr",
            _ => return Ok(None),
        };
        let lhs_type = self.infer_expr_type(lhs)?;
        let rhs_type = self.infer_expr_type(rhs)?;
        let mixed = matches!(
            (&lhs_type, &rhs_type),
            (HirType::JsValue, HirType::I64) | (HirType::I64, HirType::JsValue)
        );
        // Two beyond-`i64` handles (e.g. two large literals) only for the
        // operations the ordinary path rejects outright; `+` between two
        // dynamic values keeps its existing string-concatenation behavior
        // (thaw can't tell a dynamic bigint from a dynamic string).
        let both_dynamic = lhs_type == HirType::JsValue
            && rhs_type == HirType::JsValue
            && op != BinaryOp::Add;
        if !mixed && !both_dynamic {
            return Ok(None);
        }
        let left = self.bigint_decimal_string(lhs.clone(), &lhs_type)?;
        let right = self.bigint_decimal_string(rhs.clone(), &rhs_type)?;
        let digits = HirExpr::Call(
            Box::new(HirExpr::Var(intrinsic.into())),
            vec![left, right],
        );
        let constructor = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str("BigInt".into()))],
        );
        let arguments = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(vec![digits]),
        )?;
        Ok(Some(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
            vec![constructor, arguments],
        )))
    }

    /// Strict equality between a native `bigint` and a beyond-`i64` one
    /// (a live handle), which `coerce_strict_equality_operands` would
    /// reject as incompatible: equal exactly when their decimal digits
    /// agree.
    fn lower_mixed_bigint_equality(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if !matches!(
            (&lhs_type, &rhs_type),
            (HirType::JsValue, HirType::I64) | (HirType::I64, HirType::JsValue)
        ) {
            return Ok(None);
        }
        let left = self.bigint_decimal_string(lhs, &lhs_type)?;
        let right = self.bigint_decimal_string(rhs, &rhs_type)?;
        let comparison = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bigint_decimal_cmp".into())),
            vec![left, right],
        );
        Ok(Some(HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(comparison),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        )))
    }

    fn lower_relational(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        op: BinOp,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type == HirType::I64 && rhs_type == HirType::I64 {
            return Ok(HirExpr::BinOp(op, Box::new(lhs), Box::new(rhs)));
        }
        // A native `bigint` against a beyond-`i64` one (a live handle):
        // compare their decimal digits, which orders integers exactly.
        if matches!(
            (&lhs_type, &rhs_type),
            (HirType::JsValue, HirType::I64) | (HirType::I64, HirType::JsValue)
        ) {
            let left = self.bigint_decimal_string(lhs, &lhs_type)?;
            let right = self.bigint_decimal_string(rhs, &rhs_type)?;
            let comparison = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bigint_decimal_cmp".into())),
                vec![left, right],
            );
            return Ok(HirExpr::BinOp(
                op,
                Box::new(comparison),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            ));
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
        if rhs_type == HirType::Undefined {
            if let HirExpr::TypedIndex(array, index, _) = &lhs {
                if let HirType::Array(element) = self.infer_expr_type(array)? {
                    if !matches!(element.as_ref(), HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined)
                        && !matches!(element.as_ref(), HirType::Union(members) if members.contains(&HirType::Undefined))
                    {
                    let is_json_element = *element == HirType::Json;
                    let has_function_element = Self::contains_function_value(&element);
                    let element_type = element.as_ref().clone();
                    let array_expr = array.as_ref().clone();
                    let index_expr = index.as_ref().clone();
                    let array_name = format!("__thaw_index_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_index_value_{}", self.next_binding);
                    self.next_binding += 1;
                    let array_type = HirType::Array(element);
                    self.scope.insert(array_name.clone(), array_type.clone());
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let array = HirExpr::Var(array_name.clone());
                    let index = HirExpr::Var(index_name.clone());
                    let in_bounds = self.lower_logical_expr(
                        HirExpr::BinOp(
                            BinOp::GtEq,
                            Box::new(index.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        ),
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(index.clone()),
                            Box::new(HirExpr::ArrayLen(Box::new(array.clone()))),
                        ),
                        true,
                    )?;
                    let integer_index = self.lower_logical_expr(
                        in_bounds,
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(index.clone()),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                                vec![index.clone()],
                            )),
                        ),
                        true,
                    )?;
                    let present = self.lower_logical_expr(
                        integer_index,
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                                vec![array.clone(), index.clone()],
                            )),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        ),
                        true,
                    )?;
                    let missing = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(present),
                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                    );
                    // A present (state `1`, not the `2` a literal/
                    // `OptionalNone(Json)` write marks) slot can still
                    // *hold* the real `Json` "undefined" sentinel
                    // (`thaw_json_undefined`) -- e.g. `.map()`'s generic
                    // `IndexAssign` write path (`lower_array_map`'s
                    // "callback type already matches element type" fast
                    // path) marks state `1` unconditionally, with no
                    // concept of "the value I'm storing happens to be
                    // undefined". The presence-state check above can't
                    // see that; also check the loaded value itself for a
                    // `Json` element, so `mapped[i] === undefined` agrees
                    // with `typeof mapped[i] === "undefined"` regardless
                    // of which write path produced the slot.
                    let missing = if is_json_element || has_function_element {
                        let loaded = HirExpr::TypedIndex(
                            Box::new(array), Box::new(index), element_type.clone(),
                        );
                        let value_is_undefined = if is_json_element {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                                vec![loaded],
                            )
                        } else {
                            HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(self.function_observable_absence(loaded, &element_type)),
                                Box::new(HirExpr::Lit(HirLit::F64(1.0))))
                        };
                        self.lower_logical_expr(missing, value_is_undefined, false)?
                    } else {
                        missing
                    };
                    let undefined_name = format!("__thaw_index_undefined_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(undefined_name.clone(), HirType::Undefined);
                    let snapshot_name = format!("__thaw_index_missing_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(snapshot_name.clone(), HirType::Bool);
                    return self
                        .wrap_call_argument_bindings(
                            HirExpr::Var(snapshot_name.clone()),
                            &[
                                (array_name, array_type, array_expr),
                                (index_name, HirType::F64, index_expr),
                                (snapshot_name, HirType::Bool, missing),
                                (undefined_name, HirType::Undefined, rhs),
                            ],
                        )
                        .map(Some);
                    }
                }
            }
        }
        if lhs_type == HirType::Undefined && matches!(rhs, HirExpr::TypedIndex(_, _, _)) {
            return self.lower_optional_undefined_equality(rhs, lhs);
        }
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
                (HirType::Optional(payload), other) if payload.as_ref() == other => {
                    let left_name = format!("__thaw_optional_equality_left_{}", self.next_binding);
                    self.next_binding += 1;
                    let right_name = format!("__thaw_optional_equality_right_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(left_name.clone(), lhs_type.clone());
                    self.scope.insert(right_name.clone(), rhs_type.clone());
                    let left = HirExpr::Var(left_name.clone());
                    let result = HirExpr::Block(vec![HirStmt::If(
                        HirExpr::OptionalIsNone(Box::new(left.clone()), other.clone()),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                        vec![HirStmt::Return(Some(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::OptionalValue(Box::new(left), other.clone())),
                            Box::new(HirExpr::Var(right_name.clone())),
                        )))],
                    )]);
                    Some(self.wrap_call_argument_bindings(
                        result,
                        &[(left_name, lhs_type.clone(), lhs), (right_name, rhs_type.clone(), rhs)],
                    )?)
                }
                (other, HirType::Optional(payload)) if payload.as_ref() == other => {
                    return self.lower_optional_undefined_equality(rhs, lhs);
                }
                (HirType::Optional(left), HirType::Optional(right)) if left == right => {
                    Some(self.lower_absence_equality(
                        lhs,
                        rhs,
                        lhs_type.clone(),
                        rhs_type.clone(),
                        left.as_ref().clone(),
                        AbsenceKind::Optional,
                    )?)
                }
                (HirType::Nullable(left), HirType::Nullable(right)) if left == right => {
                    Some(self.lower_absence_equality(
                        lhs,
                        rhs,
                        lhs_type.clone(),
                        rhs_type.clone(),
                        left.as_ref().clone(),
                        AbsenceKind::Nullable,
                    )?)
                }
                (HirType::Nullish(left), HirType::Nullish(right)) if left == right => {
                    Some(self.lower_absence_equality(
                        lhs,
                        rhs,
                        lhs_type.clone(),
                        rhs_type.clone(),
                        left.as_ref().clone(),
                        AbsenceKind::Nullish,
                    )?)
                }
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

    /// `===` between two values of the same tagged-absence wrapper type
    /// (`Optional`/`Nullable`/`Nullish`). Both absent compare equal, except
    /// for a `Nullish` only when the absence *kind* also matches (`null !==
    /// undefined`); a value present on one side is never equal to an absent
    /// one; two present values compare by their payload's own `===`.
    fn lower_absence_equality(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        lhs_type: HirType,
        rhs_type: HirType,
        payload: HirType,
        kind: AbsenceKind,
    ) -> Result<HirExpr, String> {
        let left_name = format!("__thaw_absence_equality_left_{}", self.next_binding);
        self.next_binding += 1;
        let right_name = format!("__thaw_absence_equality_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(left_name.clone(), lhs_type.clone());
        self.scope.insert(right_name.clone(), rhs_type.clone());
        let left = HirExpr::Var(left_name.clone());
        let right = HirExpr::Var(right_name.clone());

        let none_left = self.absence_is_none(kind, left.clone(), payload.clone());
        let none_right = self.absence_is_none(kind, right.clone(), payload.clone());
        let some_equality = self.lower_absence_payload_equality(
            self.absence_value(kind, left.clone(), payload.clone()),
            self.absence_value(kind, right.clone(), payload.clone()),
        )?;

        let mut stmts = Vec::new();
        match kind {
            AbsenceKind::Nullish => stmts.push(HirStmt::If(
                none_left,
                vec![HirStmt::If(
                    HirExpr::NullishIsNull(Box::new(left.clone()), payload.clone()),
                    vec![HirStmt::Return(Some(HirExpr::NullishIsNull(
                        Box::new(right.clone()),
                        payload.clone(),
                    )))],
                    vec![HirStmt::Return(Some(HirExpr::NullishIsUndefined(
                        Box::new(right.clone()),
                        payload.clone(),
                    )))],
                )],
                Vec::new(),
            )),
            _ => stmts.push(HirStmt::If(
                none_left,
                vec![HirStmt::Return(Some(none_right.clone()))],
                Vec::new(),
            )),
        }
        stmts.push(HirStmt::If(
            none_right,
            vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
            Vec::new(),
        ));
        stmts.push(HirStmt::Return(Some(some_equality)));

        self.wrap_call_argument_bindings(
            HirExpr::Block(stmts),
            &[(left_name, lhs_type, lhs), (right_name, rhs_type, rhs)],
        )
    }

    fn absence_is_none(&self, kind: AbsenceKind, value: HirExpr, payload: HirType) -> HirExpr {
        match kind {
            AbsenceKind::Optional => HirExpr::OptionalIsNone(Box::new(value), payload),
            AbsenceKind::Nullable => HirExpr::NullableIsNone(Box::new(value), payload),
            AbsenceKind::Nullish => HirExpr::NullishIsNone(Box::new(value), payload),
        }
    }

    fn absence_value(&self, kind: AbsenceKind, value: HirExpr, payload: HirType) -> HirExpr {
        match kind {
            AbsenceKind::Optional => HirExpr::OptionalValue(Box::new(value), payload),
            AbsenceKind::Nullable => HirExpr::NullableValue(Box::new(value), payload),
            AbsenceKind::Nullish => HirExpr::NullishValue(Box::new(value), payload),
        }
    }

    fn lower_absence_payload_equality(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        Ok(
            match self.lower_optional_undefined_equality(lhs.clone(), rhs.clone())? {
                Some(expr) => expr,
                None => HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)),
            },
        )
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

    /// `value instanceof <arbitrary named global>` for a live `JsValue`
    /// receiver against a class name this compiler has no dedicated
    /// native representation or hardcoded dynamic-value global for
    /// (`ArrayBuffer`, `DataView`, `Headers`, `ReadableStream`, ... --
    /// anything not in `native_builtin_instanceof_dynamic_global`'s own
    /// fixed table). Generalizes that whole mechanism: rather than
    /// adding one more hardcoded `value => value instanceof X` global
    /// per class name, this passes the name itself across the boundary
    /// and looks it up as `globalThis[name]` on the JS side
    /// (`__thaw_instanceof_dynamic_value_by_name`, `platform_globals/
    /// runtime.js`) -- safe for any string (a non-existent or non-
    /// callable name just answers `false`, not a thrown error), so it
    /// covers every real JS/Node global uniformly with no further table
    /// maintenance. Uses `callDynamicValueMixed` (like `dynamic_value_
    /// instanceof` above) rather than `dynamic_value_check`'s simpler
    /// one-handle call, since this needs *two* arguments of different
    /// kinds: `class_name` (positional JSON) and `value` (a live
    /// handle) -- `invoke_mixed` (thaw-quickjs's `api.rs`) appends
    /// JSON-parsed positional arguments before handle arguments, so the
    /// JS side's own parameter order is `(name, value)`.
    fn dynamic_value_check_by_name(
        &mut self,
        class_name: &str,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let callable = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(
                "__thaw_instanceof_dynamic_value_by_name".into(),
            ))],
        );
        let json_args = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::Str(class_name.to_string()))]),
            HirType::Array(Box::new(HirType::Str)),
        )?;
        Ok(HirExpr::JsonAsBool(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixed".into())),
            vec![callable, json_args, HirExpr::ArrayLit(vec![value])],
        ))))
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
