enum ArrayIteratorKind {
    Keys,
    Values(HirType),
    Entries(HirType),
}

impl<'a> FnLowerer<'a> {
    fn lower_inferred_array_literal(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let HirExpr::ArrayLit(elements) = value else {
            return Ok(value);
        };
        if !elements.iter().any(|item| matches!(item, HirExpr::TypedIndex(_, _, _))) {
            return Ok(HirExpr::ArrayLit(elements));
        }
        let converted = elements.iter().cloned()
            .map(|item| self.lower_array_index_operand(item))
            .collect::<Result<Vec<_>, _>>()?;
        let types = converted.iter()
            .filter(|item| !matches!(item, HirExpr::Lit(HirLit::ArrayHole)))
            .map(|item| self.infer_expr_type(item))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(HirType::Optional(payload)) = types.first() else {
            return Ok(HirExpr::ArrayLit(elements));
        };
        let element = payload.as_ref().clone();
        if !types.iter().all(|ty| ty == &element || ty == &HirType::Optional(Box::new(element.clone()))) {
            return Ok(HirExpr::ArrayLit(elements));
        }
        let values = converted.into_iter()
            .map(|item| if matches!(item, HirExpr::Lit(HirLit::ArrayHole)) {
                Ok(item)
            } else {
                self.coerce_array_insert_value(item, &element)
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.lower_native_array_literal(values, element)
    }

    fn lower_native_array_literal(
        &mut self,
        values: Vec<HirExpr>,
        element: HirType,
    ) -> Result<HirExpr, String> {
        let optional = HirType::Optional(Box::new(element.clone()));
        let has_optional = values.iter()
            .filter(|value| !matches!(value, HirExpr::Lit(HirLit::ArrayHole)))
            .map(|value| self.infer_expr_type(value))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|ty| ty == optional);
        if !has_optional {
            return Ok(HirExpr::ArrayLit(values));
        }
        let length = values.len() as f64;
        let length_name = format!("__thaw_literal_length_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        let result_name = format!("__thaw_literal_result_{}", self.next_binding);
        self.next_binding += 1;
        let array_type = HirType::Array(Box::new(element.clone()));
        self.scope.insert(result_name.clone(), array_type);
        let result = || HirExpr::Var(result_name.clone());
        let mut body = vec![HirStmt::Let(
            result_name.clone(),
            HirType::Array(Box::new(element.clone())),
            HirExpr::ArrayAlloc(Box::new(HirExpr::Var(length_name.clone())), element.clone()),
        )];
        for (index, value) in values.into_iter().enumerate() {
            let offset = HirExpr::Lit(HirLit::F64(index as f64));
            if matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                body.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_set_hole".into())),
                    vec![result(), offset],
                )));
            } else if self.infer_expr_type(&value)? == HirType::Optional(Box::new(element.clone())) {
                body.push(HirStmt::Expr(self.lower_optional_index_assignment(
                    result(), offset, value, element.clone(),
                )?));
            } else {
                body.push(HirStmt::Expr(HirExpr::IndexAssign(
                    Box::new(result()), Box::new(offset), Box::new(value),
                )));
            }
        }
        body.push(HirStmt::Return(Some(result())));
        self.wrap_call_argument_bindings(
            HirExpr::Block(body),
            &[(length_name, HirType::F64, HirExpr::Lit(HirLit::F64(length)))],
        )
    }

    fn coerce_array_insert_value(
        &mut self,
        value: HirExpr,
        element: &HirType,
    ) -> Result<HirExpr, String> {
        let value = self.lower_array_index_operand(value)?;
        let actual = self.infer_expr_type(&value)?;
        if actual == HirType::Optional(Box::new(element.clone())) {
            Ok(value)
        } else if actual == HirType::Undefined
            && !matches!(element, HirType::Undefined | HirType::Optional(_) | HirType::Nullish(_))
            && !matches!(element, HirType::Union(members) if members.contains(&HirType::Undefined))
        {
            Ok(HirExpr::OptionalNone(element.clone()))
        } else {
            self.coerce_to_declared(element, value)
        }
    }

    fn lower_primitive_array_operand(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let HirExpr::TypedIndex(array, index, element) = value.clone() else {
            return Ok(value);
        };
        if !matches!(element, HirType::F64 | HirType::Str | HirType::Bool) {
            return Ok(value);
        }
        let ty = self.infer_expr_type(&array)?;
        if !matches!(ty, HirType::Array(_)) {
            return Ok(value);
        }
        self.lower_array_index(*array, ty, element, *index)
    }

    fn coerce_primitive_array_argument(
        &mut self,
        value: HirExpr,
        declared: &HirType,
    ) -> Result<HirExpr, String> {
        let value = if matches!(declared, HirType::F64 | HirType::Str) {
            self.lower_primitive_array_operand(value)?
        } else {
            value
        };
        let actual = self.infer_expr_type(&value)?;
        // ponytail: native number/string parameters cannot carry undefined;
        // widening their ABI is needed if `typeof` inside the callee must see it.
        match (declared, &actual) {
            (HirType::F64, HirType::Optional(payload)) if **payload == HirType::F64 => {
                self.coerce_primitive_to_number(value)
            }
            (HirType::Str, HirType::Optional(payload)) if **payload == HirType::Str => {
                self.coerce_primitive_to_string(value)
            }
            _ => self.coerce_to_declared(declared, value),
        }
    }

    fn array_read_type(element: &HirType) -> HirType {
        match element {
            HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined => element.clone(),
            HirType::Nullable(payload) => HirType::Nullish(payload.clone()),
            HirType::Union(members) => {
                let mut members = members.clone();
                if !members.contains(&HirType::Undefined) {
                    members.push(HirType::Undefined);
                }
                HirType::Union(members)
            }
            _ => HirType::Optional(Box::new(element.clone())),
        }
    }

    fn array_optional_read(element: &HirType, value: HirExpr) -> (HirType, HirExpr, HirExpr) {
        let ty = Self::array_read_type(element);
        let (present, absent) = match element {
            HirType::Optional(payload) => (value, HirExpr::OptionalNone(payload.as_ref().clone())),
            HirType::Nullish(payload) => (value, HirExpr::NullishUndefined(payload.as_ref().clone())),
            HirType::Nullable(payload) => {
                let payload = payload.as_ref().clone();
                (
                    HirExpr::Conditional(
                        Box::new(HirExpr::NullableIsNone(Box::new(value.clone()), payload.clone())),
                        Box::new(HirExpr::NullishNull(payload.clone())),
                        Box::new(HirExpr::NullishSome(
                            Box::new(HirExpr::NullableValue(Box::new(value), payload.clone())),
                            payload.clone(),
                        )),
                        ty.clone(),
                    ),
                    HirExpr::NullishUndefined(payload),
                )
            }
            HirType::Undefined => (HirExpr::Lit(HirLit::Undefined), HirExpr::Lit(HirLit::Undefined)),
            HirType::Union(_) => {
                let HirType::Union(members) = &ty else { unreachable!() };
                let index = members.iter().position(|member| member == &HirType::Undefined).unwrap();
                (HirExpr::TypedClosure(ty.clone(), Box::new(value)), HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::Undefined)), index, members.clone(),
                ))
            }
            _ => (
                HirExpr::OptionalSome(Box::new(value), element.clone()),
                HirExpr::OptionalNone(element.clone()),
            ),
        };
        (ty, present, absent)
    }

    /// `Array.from({ length }, mapfn?)` -- the array-like-object overload,
    /// as opposed to the real-array/string overload `lower_array_map`
    /// handles. There is no underlying element storage to read (a plain
    /// `{ length }` object has no indexed properties in this compiler's
    /// fixed-layout object model), so every per-index value the spec would
    /// read from the source is always exactly `undefined`, matching
    /// `Array.from({length: 3})` producing `[undefined, undefined,
    /// undefined]` for real JavaScript too. `mapfn` may still ignore that
    /// value and use only the index, which is the overload's common use
    /// (`Array.from({length: n}, (_, i) => ...)` to build a range).
    fn lower_array_from_length(
        &mut self,
        length: HirExpr,
        callback: Option<HirExpr>,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let length_name = format!("__thaw_array_from_length_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        let mut bindings = vec![(length_name.clone(), HirType::F64, length)];
        let Some(callback) = callback else {
            let result = HirExpr::ArrayAlloc(
                Box::new(HirExpr::Var(length_name)),
                HirType::Undefined,
            );
            return self.wrap_call_argument_bindings(result, &bindings);
        };
        let callback_name = format!("__thaw_array_from_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let HirType::Function(params, output_type) = &callback_type else {
            unreachable!("Array.from mapper was validated as a function")
        };
        let returns_void = **output_type == HirType::Void;
        let output_type = if returns_void { HirType::Undefined } else { output_type.as_ref().clone() };
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let result_name = format!("__thaw_array_from_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_array_from_index_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Array(Box::new(output_type.clone()));
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        let available = [
            HirExpr::Lit(HirLit::Undefined),
            HirExpr::Var(index_name.clone()),
        ];
        let callback_call =
            self.lower_array_callback_call(&callback_name, params, &available)?;
        let callback_call = if returns_void {
            self.array_void_to_undefined(callback_call)?
        } else {
            callback_call
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(HirExpr::Var(length_name.clone())), output_type),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(HirExpr::Var(result_name.clone())),
                        Box::new(HirExpr::Var(index_name.clone())),
                        Box::new(callback_call),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(result_name))),
        ]);
        bindings.push((callback_name, callback_type, callback));
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_array_from_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_map(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback_element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_map_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_map_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let HirType::Function(params, output_type) = &callback_type else {
            unreachable!("array mapper was validated as a function")
        };
        let returns_void = **output_type == HirType::Void;
        let output_type = if returns_void { HirType::Undefined } else { output_type.as_ref().clone() };
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_map_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_map_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_map_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_map_element_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Array(Box::new(output_type.clone()));
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), callback_element_type.clone());
        let raw_element = HirExpr::TypedIndex(
            Box::new(HirExpr::Var(receiver_name.clone())),
            Box::new(HirExpr::Var(index_name.clone())),
            element_type.clone(),
        );
        let element = if callback_element_type == element_type
            && !matches!(element_type, HirType::Optional(_) | HirType::Nullish(_))
            && !matches!(&element_type, HirType::Union(members) if members.contains(&HirType::Undefined))
        {
            raw_element
        } else {
            let (ty, present, absent) = Self::array_optional_read(&element_type, raw_element);
            HirExpr::Conditional(
                Box::new(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                        vec![HirExpr::Var(receiver_name.clone()), HirExpr::Var(index_name.clone())],
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
                Box::new(present),
                Box::new(absent),
                ty,
            )
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call =
            self.lower_array_callback_call(&callback_name, params, &available)?;
        let callback_call = if returns_void {
            self.array_void_to_undefined(callback_call)?
        } else {
            callback_call
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(HirExpr::Var(length_name.clone())), output_type),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_has_index".into())),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(index_name.clone()),
                            ],
                        ),
                        vec![
                            HirStmt::Let(
                                element_name,
                                callback_element_type,
                                element,
                            ),
                            HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(HirExpr::Var(result_name.clone())),
                                Box::new(HirExpr::Var(index_name.clone())),
                                Box::new(callback_call),
                            )),
                        ],
                        Vec::new(),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_map_presence".into())),
                vec![
                    HirExpr::Var(result_name),
                    HirExpr::Var(receiver_name.clone()),
                ],
            ))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_map_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_filter(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback_element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_filter_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_filter_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_filter_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_filter_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_filter_index_{}", self.next_binding);
        self.next_binding += 1;
        let output_index_name = format!("__thaw_filter_output_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_filter_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(output_index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), callback_element_type.clone());
        let raw_element = HirExpr::TypedIndex(
            Box::new(HirExpr::Var(receiver_name.clone())),
            Box::new(HirExpr::Var(index_name.clone())),
            element_type.clone(),
        );
        let element = if callback_element_type == element_type
            && !matches!(element_type, HirType::Optional(_) | HirType::Nullish(_))
            && !matches!(&element_type, HirType::Union(members) if members.contains(&HirType::Undefined))
        {
            raw_element
        } else {
            let (ty, present, absent) = Self::array_optional_read(&element_type, raw_element);
            HirExpr::Conditional(
                Box::new(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                        vec![HirExpr::Var(receiver_name.clone()), HirExpr::Var(index_name.clone())],
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
                Box::new(present),
                Box::new(absent),
                ty,
            )
        };
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array filter was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call =
            self.lower_array_callback_call(&callback_name, params, &available)?;
        let callback_truthy = self.array_callback_truthy(callback_call)?;
        let increment = |name: &str| {
            HirStmt::Expr(HirExpr::Assign(
                name.into(),
                Box::new(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(HirExpr::Var(name.into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
            ))
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(
                    Box::new(HirExpr::Var(length_name.clone())),
                    element_type.clone(),
                ),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::Let(
                output_index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_has_index".into())),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(index_name.clone()),
                            ],
                        ),
                        vec![
                            HirStmt::Let(
                                element_name.clone(),
                                callback_element_type,
                                element,
                            ),
                            HirStmt::If(
                                callback_truthy,
                                vec![
                                    HirStmt::Expr(HirExpr::IndexAssign(
                                        Box::new(HirExpr::Var(result_name.clone())),
                                        Box::new(HirExpr::Var(output_index_name.clone())),
                                        Box::new(HirExpr::TypedIndex(
                                            Box::new(HirExpr::Var(receiver_name.clone())),
                                            Box::new(HirExpr::Var(index_name.clone())),
                                            element_type.clone(),
                                        )),
                                    )),
                                    HirStmt::Expr(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_array_copy_index_state".into())),
                                        vec![
                                            HirExpr::Var(result_name.clone()),
                                            HirExpr::Var(output_index_name.clone()),
                                            HirExpr::Var(receiver_name.clone()),
                                            HirExpr::Var(index_name.clone()),
                                        ],
                                    )),
                                    increment(&output_index_name),
                                ],
                                Vec::new(),
                            ),
                        ],
                        Vec::new(),
                    ),
                    increment(&index_name),
                ],
            ),
            HirStmt::Return(Some(HirExpr::ArraySetLen(
                Box::new(HirExpr::Var(result_name)),
                Box::new(HirExpr::Var(output_index_name)),
                element_type,
            ))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_filter_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_flat_map_result(&mut self, mapped: HirExpr) -> Result<HirExpr, String> {
        let mapped_type = self.infer_expr_type(&mapped)?;
        let HirType::Array(element) = mapped_type.clone() else {
            unreachable!("array map always returns an array")
        };
        match *element {
            HirType::Array(inner) => {
                self.lower_array_flat_one(mapped, mapped_type, *inner)
            }
            // Same runtime-only "is this element itself an array" gap
            // `.flat()` has for a `Json`-typed element -- the callback's
            // return value could be a JS array or not, only knowable once
            // it actually runs. `lower_array_flat_one` only handles a
            // statically nested `T[][]` shape.
            HirType::Json => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_any_array_flat".to_string())),
                vec![mapped, HirExpr::Lit(HirLit::F64(1.0))],
            )),
            // A ternary/mixed-return callback (`(v) => Array.isArray(v) ?
            // v : [v]`) infers as `Union([Json, Array(Json), ...])`, not
            // plain `Json` -- normalize every element to `Json` first
            // (via a nested `.map()`, reusing `coerce_to_declared`'s
            // existing Union-to-Json support), then reuse the runtime
            // flattener above.
            HirType::Union(members) if members.contains(&HirType::Json) => {
                let union_type = HirType::Union(members.clone());
                let parameter = format!("__thaw_flat_map_any_normalize_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(parameter.clone(), union_type.clone());
                let coerced =
                    self.coerce_to_declared(&HirType::Json, HirExpr::Var(parameter.clone()))?;
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam { name: parameter, ty: union_type.clone() }],
                    HirType::Json,
                    Box::new(coerced),
                );
                // `callback_element_type` (the 4th argument) must match
                // this callback's own *parameter* type (`union_type`),
                // not its return type (`Json`) -- passing `Json` here
                // once made `lower_array_map` size its internal "element
                // read from the source array" scratch variable/alloca
                // for an 8-byte pointer while actually storing a 16-byte
                // `{tag, payload}` union struct into it: a stack buffer
                // overflow that silently corrupted the tag/payload bits
                // read back, observed as `thaw_json_object_set_json`
                // segfaulting on a garbage "pointer" that was actually
                // leftover float bits. Found via `gdb` on a locally
                // unstripped build (`-Wl,--strip-all` removed from
                // `thaw-cli`'s linker invocation) plus a scratch
                // `#[test]` dumping `compiler.print_to_string()`'s LLVM
                // IR for the crashing case -- tracing the IR by hand
                // located the undersized alloca precisely.
                let normalized = self.lower_array_map(
                    mapped, mapped_type, union_type.clone(), union_type, callback, None,
                )?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_any_array_flat".to_string())),
                    vec![normalized, HirExpr::Lit(HirLit::F64(1.0))],
                ))
            }
            HirType::Union(members) if members.len() == 2 => {
                let Some((array_index, inner)) = members.iter().enumerate().find_map(|(index, member)| {
                    let HirType::Array(inner) = member else { return None };
                    Some((index, inner.as_ref().clone()))
                }) else {
                    return self.lower_array_flat_map_scalar(mapped, mapped_type, HirType::Union(members));
                };
                let Some(scalar_index) = members.iter().position(|member| member == &inner) else {
                    return self.lower_array_flat_map_scalar(mapped, mapped_type, HirType::Union(members));
                };
                let parameter = format!("__thaw_flat_map_mixed_{}", self.next_binding);
                self.next_binding += 1;
                let union_type = HirType::Union(members.clone());
                self.scope.insert(parameter.clone(), union_type.clone());
                let array_type = HirType::Array(Box::new(inner.clone()));
                let value = || HirExpr::Var(parameter.clone());
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam { name: parameter.clone(), ty: union_type.clone() }],
                    array_type.clone(),
                    Box::new(HirExpr::Conditional(
                        Box::new(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(value()), members.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(array_index as f64))),
                        )),
                        Box::new(HirExpr::UnionValue(Box::new(value()), array_index, members.clone())),
                        Box::new(HirExpr::ArrayLit(vec![HirExpr::UnionValue(
                            Box::new(value()), scalar_index, members.clone(),
                        )])),
                        array_type,
                    )),
                );
                let normalized = self.lower_array_map(
                    mapped, mapped_type, union_type.clone(), union_type, callback, None,
                )?;
                let normalized_type = self.infer_expr_type(&normalized)?;
                self.lower_array_flat_one(normalized, normalized_type, inner)
            }
            scalar => self.lower_array_flat_map_scalar(mapped, mapped_type, scalar),
        }
    }

    fn lower_array_flat_map_scalar(
        &mut self,
        mapped: HirExpr,
        mapped_type: HirType,
        scalar: HirType,
    ) -> Result<HirExpr, String> {
        self.lower_array_filter(
            mapped,
            mapped_type,
            scalar.clone(),
            scalar,
            HirExpr::Lambda(
                Vec::new(),
                Vec::new(),
                HirType::Bool,
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ),
            None,
        )
    }

    fn lower_array_flat_one(
        &mut self,
        receiver: HirExpr,
        nested_array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_flat_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), nested_array_type.clone());
        let outer_length_name = format!("__thaw_flat_outer_length_{}", self.next_binding);
        self.next_binding += 1;
        let total_length_name = format!("__thaw_flat_total_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_index_name = format!("__thaw_flat_outer_index_{}", self.next_binding);
        self.next_binding += 1;
        let inner_array_name = format!("__thaw_flat_inner_array_{}", self.next_binding);
        self.next_binding += 1;
        let inner_length_name = format!("__thaw_flat_inner_length_{}", self.next_binding);
        self.next_binding += 1;
        let inner_index_name = format!("__thaw_flat_inner_index_{}", self.next_binding);
        self.next_binding += 1;
        let destination_name = format!("__thaw_flat_destination_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_flat_result_{}", self.next_binding);
        self.next_binding += 1;
        for name in [
            &outer_length_name,
            &total_length_name,
            &outer_index_name,
            &inner_length_name,
            &inner_index_name,
            &destination_name,
        ] {
            self.scope.insert(name.clone(), HirType::F64);
        }
        let inner_array_type = HirType::Array(Box::new(element_type.clone()));
        let result_type = inner_array_type.clone();
        self.scope
            .insert(inner_array_name.clone(), inner_array_type.clone());
        self.scope.insert(result_name.clone(), result_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let add = |left, right| HirExpr::BinOp(BinOp::Add, Box::new(left), Box::new(right));
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let increment = |name: &str| assign(name, add(var(name), number(1.0)));
        let load_inner = || {
            HirExpr::TypedIndex(
                Box::new(var(&receiver_name)),
                Box::new(var(&outer_index_name)),
                inner_array_type.clone(),
            )
        };
        let outer_state = || HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_index_state".into())),
            vec![var(&receiver_name), var(&outer_index_name)],
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                outer_length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(total_length_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(outer_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(outer_state()),
                            Box::new(number(1.0)),
                        ),
                        vec![
                            HirStmt::Let(
                                inner_array_name.clone(),
                                inner_array_type.clone(),
                                load_inner(),
                            ),
                            assign(
                                &total_length_name,
                                add(
                                    var(&total_length_name),
                                    HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                                ),
                            ),
                        ],
                        vec![HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(outer_state()),
                                Box::new(number(2.0)),
                            ),
                            vec![increment(&total_length_name)],
                            Vec::new(),
                        )],
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(var(&total_length_name)), element_type.clone()),
            ),
            assign(&outer_index_name, number(0.0)),
            HirStmt::Let(destination_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(outer_state()),
                            Box::new(number(1.0)),
                        ),
                        vec![
                            HirStmt::Let(
                                inner_array_name.clone(),
                                inner_array_type.clone(),
                                load_inner(),
                            ),
                            HirStmt::Let(
                                inner_length_name.clone(),
                                HirType::F64,
                                HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                            ),
                            HirStmt::Let(inner_index_name.clone(), HirType::F64, number(0.0)),
                            HirStmt::While(
                                HirExpr::BinOp(
                                    BinOp::Lt,
                                    Box::new(var(&inner_index_name)),
                                    Box::new(var(&inner_length_name)),
                                ),
                                vec![
                                    HirStmt::If(
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_array_has_index".into(),
                                            )),
                                            vec![
                                                var(&inner_array_name),
                                                var(&inner_index_name),
                                            ],
                                        ),
                                        vec![
                                            HirStmt::Expr(HirExpr::IndexAssign(
                                                Box::new(var(&result_name)),
                                                Box::new(var(&destination_name)),
                                                Box::new(HirExpr::TypedIndex(
                                                    Box::new(var(&inner_array_name)),
                                                    Box::new(var(&inner_index_name)),
                                                    element_type.clone(),
                                                )),
                                            )),
                                            HirStmt::Expr(HirExpr::Call(
                                                Box::new(HirExpr::Var("__thaw_array_copy_index_state".into())),
                                                vec![
                                                    var(&result_name),
                                                    var(&destination_name),
                                                    var(&inner_array_name),
                                                    var(&inner_index_name),
                                                ],
                                            )),
                                            increment(&destination_name),
                                        ],
                                        Vec::new(),
                                    ),
                                    increment(&inner_index_name),
                                ],
                            ),
                        ],
                        vec![HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(outer_state()),
                                Box::new(number(2.0)),
                            ),
                            vec![
                                HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_set_undefined".into())),
                                    vec![var(&result_name), var(&destination_name)],
                                )),
                                increment(&destination_name),
                            ],
                            Vec::new(),
                        )],
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Return(Some(HirExpr::ArraySetLen(
                Box::new(var(&result_name)),
                Box::new(var(&destination_name)),
                element_type,
            ))),
        ]);
        self.wrap_call_argument_bindings(body, &[(receiver_name, nested_array_type, receiver)])
    }

    fn lower_array_with(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_with_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_with_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        let value_name = format!("__thaw_with_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        let value_type = self.infer_expr_type(&value)?;
        self.scope.insert(value_name.clone(), value_type.clone());
        let length_name = format!("__thaw_with_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_with_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        let copy_index_name = format!("__thaw_with_copy_index_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_with_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        self.scope.insert(copy_index_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let range_error = || {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                "\u{1}RangeError\u{1}Invalid index for Array.prototype.with".into(),
            )))
        };
        let replacement = if value_type == HirType::Optional(Box::new(element_type.clone())) {
            vec![HirStmt::If(
                HirExpr::OptionalIsNone(Box::new(var(&value_name)), element_type.clone()),
                vec![HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_set_undefined".into())),
                    vec![var(&result_name), var(&copy_index_name)],
                ))],
                vec![HirStmt::Expr(HirExpr::IndexAssign(
                    Box::new(var(&result_name)), Box::new(var(&copy_index_name)),
                    Box::new(HirExpr::OptionalValue(Box::new(var(&value_name)), element_type.clone())),
                ))],
            )]
        } else {
            vec![HirStmt::Expr(HirExpr::IndexAssign(
                Box::new(var(&result_name)), Box::new(var(&copy_index_name)),
                Box::new(var(&value_name)),
            ))]
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            // ToIntegerOrInfinity maps NaN to +0; finite fractional indices
            // are truncated by the typed element-address conversion.
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(&actual_index_name, number(0.0))],
            ),
            assign(
                &actual_index_name,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                    vec![var(&actual_index_name)],
                ),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(
                    &actual_index_name,
                    HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(var(&length_name)),
                        Box::new(var(&actual_index_name)),
                    ),
                )],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&length_name)), element_type.clone()),
            ),
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_copy_presence".into())),
                vec![var(&result_name), var(&receiver_name)],
            )),
            HirStmt::Let(copy_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&copy_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(var(&copy_index_name)),
                            Box::new(var(&actual_index_name)),
                        ),
                         replacement,
                        vec![HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                                    vec![var(&receiver_name), var(&copy_index_name)],
                                )),
                                Box::new(number(1.0)),
                            ),
                            vec![HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(var(&result_name)),
                                Box::new(var(&copy_index_name)),
                                Box::new(HirExpr::TypedIndex(
                                    Box::new(var(&receiver_name)),
                                    Box::new(var(&copy_index_name)),
                                    element_type.clone(),
                                )),
                            ))],
                            Vec::new(),
                        )],
                    ),
                    assign(
                        &copy_index_name,
                        HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&copy_index_name)),
                            Box::new(number(1.0)),
                        ),
                    ),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_densify".into())),
                vec![var(&result_name)],
            ))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
                (value_name, value_type, value),
            ],
        )
    }

    fn lower_array_index(
        &mut self,
        source: HirExpr,
        array_type: HirType,
        element_type: HirType,
        offset: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_index_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_index_offset_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        let receiver = HirExpr::Var(receiver_name.clone());
        let index = HirExpr::Var(index_name.clone());
        let (optional_type, present, absent) = Self::array_optional_read(
            &element_type,
            HirExpr::TypedIndex(
                Box::new(receiver.clone()), Box::new(index.clone()), element_type.clone(),
            ),
        );
        let value = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                    vec![receiver.clone(), index.clone()],
                )),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
            )),
            Box::new(present),
            Box::new(absent.clone()),
            optional_type.clone(),
        );
        let value = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(
                BinOp::Lt,
                Box::new(index.clone()),
                Box::new(HirExpr::ArrayLen(Box::new(receiver.clone()))),
            )),
            Box::new(value),
            Box::new(absent.clone()),
            optional_type.clone(),
        );
        let value = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(
                BinOp::GtEq,
                Box::new(index.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )),
            Box::new(value),
            Box::new(absent.clone()),
            optional_type.clone(),
        );
        let value = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(index.clone()),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                    vec![index],
                )),
            )),
            Box::new(value),
            Box::new(absent),
            optional_type,
        );
        self.wrap_call_argument_bindings(
            value,
            &[
                (receiver_name, array_type, source),
                (index_name, HirType::F64, offset),
            ],
        )
    }

    fn lower_array_at(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_at_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_at_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        let length_name = format!("__thaw_at_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_at_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |value| HirStmt::Expr(HirExpr::Assign(actual_index_name.clone(), Box::new(value)));
        let (_, present, absent) = Self::array_optional_read(
            &element_type,
            HirExpr::TypedIndex(
                Box::new(var(&receiver_name)),
                Box::new(var(&actual_index_name)),
                element_type.clone(),
            ),
        );
        let none = || HirStmt::Return(Some(absent.clone()));
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(number(0.0))],
            ),
            assign(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![var(&actual_index_name)],
            )),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(var(&length_name)),
                    Box::new(var(&actual_index_name)),
                ))],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                        vec![var(&receiver_name), var(&actual_index_name)],
                    )),
                    Box::new(number(1.0)),
                ),
                Vec::new(),
                vec![none()],
            ),
            HirStmt::Return(Some(present)),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
            ],
        )
    }

    fn lower_array_keys(&mut self, receiver: HirExpr, array_type: HirType) -> Result<HirExpr, String> {
        self.lower_array_iterator(receiver, array_type, ArrayIteratorKind::Keys)
    }

    fn lower_array_values(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        self.lower_array_iterator(receiver, array_type, ArrayIteratorKind::Values(element_type))
    }

    fn lower_array_entries(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        self.lower_array_iterator(receiver, array_type, ArrayIteratorKind::Entries(element_type))
    }

    fn lower_array_iterator(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        kind: ArrayIteratorKind,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_array_iterator_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_array_iterator_index_{}", self.next_binding);
        self.next_binding += 1;
        let current_name = format!("__thaw_array_iterator_current_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(current_name.clone(), HirType::F64);
        let var = |name: &str| HirExpr::Var(name.into());
        let (yielded_type, yielded) = match kind {
            ArrayIteratorKind::Keys => (HirType::F64, var(&current_name)),
            ArrayIteratorKind::Values(element_type) => {
                let value = self.lower_array_at(
                    var(&receiver_name),
                    array_type.clone(),
                    element_type,
                    var(&current_name),
                )?;
                let value_type = self.infer_expr_type(&value)?;
                (value_type, value)
            }
            ArrayIteratorKind::Entries(element_type) => {
                let value = self.lower_array_at(
                    var(&receiver_name),
                    array_type.clone(),
                    element_type,
                    var(&current_name),
                )?;
                let value_type = self.infer_expr_type(&value)?;
                (
                    HirType::Tuple(vec![HirType::F64, value_type]),
                    HirExpr::ArrayLit(vec![var(&current_name), value]),
                )
            }
        };
        let generated_type = HirType::Array(Box::new(yielded_type.clone()));
        let producer_type = generator_function_type(
            false,
            yielded_type,
            HirType::Undefined,
            HirType::Undefined,
        );
        let params = [HirType::I64, HirType::Str, HirType::Undefined]
            .into_iter()
            .chain(std::iter::repeat_n(
                HirType::Array(Box::new(HirType::Undefined)),
                3,
            ))
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_array_iterator_arg_{index}_{}", self.next_binding),
                ty,
            })
            .collect::<Vec<_>>();
        self.next_binding += 1;
        let control_name = params[0].name.clone();
        let producer = HirExpr::Lambda(
            vec![
                HirParam {
                    name: receiver_name.clone(),
                    ty: array_type.clone(),
                },
                HirParam {
                    name: index_name.clone(),
                    ty: HirType::F64,
                },
            ],
            params,
            generated_type.clone(),
            Box::new(HirExpr::Block(vec![
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(var(&control_name)),
                        Box::new(HirExpr::Lit(HirLit::I64(0))),
                    ),
                    Vec::new(),
                    vec![HirStmt::Return(Some(HirExpr::ArrayLit(Vec::new())))],
                ),
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::Lt,
                        Box::new(var(&index_name)),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![HirStmt::Return(Some(HirExpr::ArrayLit(Vec::new())))],
                    Vec::new(),
                ),
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::GtEq,
                        Box::new(var(&index_name)),
                        Box::new(HirExpr::ArrayLen(Box::new(var(&receiver_name)))),
                    ),
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            index_name.clone(),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )),
                        HirStmt::Return(Some(HirExpr::ArrayLit(Vec::new()))),
                    ],
                    Vec::new(),
                ),
                HirStmt::Let(current_name, HirType::F64, var(&index_name)),
                HirStmt::Expr(HirExpr::Assign(
                    index_name.clone(),
                    Box::new(HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(var(&index_name)),
                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    )),
                )),
                HirStmt::Return(Some(HirExpr::ArrayLit(vec![yielded]))),
            ])),
        );
        let iterator = HirExpr::Call(
            Box::new(HirExpr::Lambda(
                vec![HirParam {
                    name: receiver_name.clone(),
                    ty: array_type.clone(),
                }],
                Vec::new(),
                producer_type,
                Box::new(HirExpr::Block(vec![
                    HirStmt::Let(index_name, HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
                    HirStmt::Return(Some(producer)),
                ])),
            )),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(iterator, &[(receiver_name, array_type, receiver)])
    }

}
