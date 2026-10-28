impl<'a> FnLowerer<'a> {
    fn lower_date_single_timestamp(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        match ty.clone() {
            HirType::Str | HirType::StrLiteral(_) => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_date_parse".into())),
                vec![if matches!(ty, HirType::StrLiteral(_)) {
                    HirExpr::TypedClosure(HirType::Str, Box::new(value))
                } else {
                    value
                }],
            )),
            HirType::Union(members) => {
                if members.is_empty() {
                    return Err("Date constructor has an empty union argument".into());
                }
                let name = format!("__thaw_date_union_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let mut result = None;
                for (index, _) in members.iter().enumerate().rev() {
                    let part = self.lower_date_single_timestamp(HirExpr::UnionValue(
                        Box::new(HirExpr::Var(name.clone())),
                        index,
                        members.clone(),
                    ))?;
                    result = Some(match result {
                        None => part,
                        Some(rest) => HirExpr::Conditional(
                            Box::new(HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(HirExpr::Var(name.clone())),
                                    members.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            )),
                            Box::new(part),
                            Box::new(rest),
                            HirType::F64,
                        ),
                    });
                }
                self.wrap_call_argument_bindings(result.unwrap(), &[(name, ty, value)])
            }
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                let name = format!("__thaw_date_absent_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let bound = HirExpr::Var(name.clone());
                let element = payload.as_ref().clone();
                let present = match &ty {
                    HirType::Optional(_) => {
                        HirExpr::OptionalValue(Box::new(bound.clone()), element.clone())
                    }
                    HirType::Nullable(_) => {
                        HirExpr::NullableValue(Box::new(bound.clone()), element.clone())
                    }
                    HirType::Nullish(_) => {
                        HirExpr::NullishValue(Box::new(bound.clone()), element.clone())
                    }
                    _ => unreachable!(),
                };
                let present = self.lower_date_single_timestamp(present)?;
                let result = match &ty {
                    HirType::Optional(_) => HirExpr::Conditional(
                        Box::new(HirExpr::OptionalIsNone(Box::new(bound), element)),
                        Box::new(HirExpr::Lit(HirLit::F64(f64::NAN))),
                        Box::new(present),
                        HirType::F64,
                    ),
                    HirType::Nullable(_) => HirExpr::Conditional(
                        Box::new(HirExpr::NullableIsNone(Box::new(bound), element)),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        Box::new(present),
                        HirType::F64,
                    ),
                    HirType::Nullish(_) => HirExpr::Conditional(
                        Box::new(HirExpr::NullishIsNull(
                            Box::new(bound.clone()),
                            element.clone(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        Box::new(HirExpr::Conditional(
                            Box::new(HirExpr::NullishIsUndefined(Box::new(bound), element)),
                            Box::new(HirExpr::Lit(HirLit::F64(f64::NAN))),
                            Box::new(present),
                            HirType::F64,
                        )),
                        HirType::F64,
                    ),
                    _ => unreachable!(),
                };
                self.wrap_call_argument_bindings(result, &[(name, ty, value)])
            }
            HirType::JsValue => Ok(HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
                vec![
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str(
                            "__thaw_date_from_live_value".into(),
                        ))],
                    ),
                    value,
                ],
            )))),
            HirType::Json => {
                let name = format!("__thaw_date_json_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), HirType::Json);
                let bound = HirExpr::Var(name.clone());
                let is_string = HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_typeof".into())),
                        vec![bound.clone()],
                    )),
                    Box::new(HirExpr::Lit(HirLit::Str("string".into()))),
                );
                let parsed = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_date_parse".into())),
                    vec![HirExpr::JsonAsString(Box::new(bound.clone()))],
                );
                let clipped = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_date_time_clip".into())),
                    vec![HirExpr::JsonAsNumber(Box::new(bound.clone()))],
                );
                let plain = HirExpr::Conditional(
                    Box::new(is_string),
                    Box::new(parsed),
                    Box::new(clipped),
                    HirType::F64,
                );
                let handle_name = format!("__thaw_date_live_handle_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(handle_name.clone(), HirType::I64);
                let handle = HirExpr::Var(handle_name.clone());
                let live = HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
                    vec![
                        HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str(
                                "__thaw_date_from_live_value".into(),
                            ))],
                        ),
                        HirExpr::TypedClosure(HirType::JsValue, Box::new(handle.clone())),
                    ],
                )));
                let non_live = HirExpr::Conditional(
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_is_date_shape".into())),
                        vec![bound.clone()],
                    )),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_date_timestamp".into())),
                        vec![bound.clone()],
                    )),
                    Box::new(plain),
                    HirType::F64,
                );
                let timestamp = self.wrap_call_argument_bindings(
                    HirExpr::Conditional(
                        Box::new(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(handle),
                            Box::new(HirExpr::Lit(HirLit::I64(0))),
                        )),
                        Box::new(non_live),
                        Box::new(live),
                        HirType::F64,
                    ),
                    &[(
                        handle_name,
                        HirType::I64,
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_borrowed_handle_id".into())),
                            vec![bound],
                    ),
                    )],
                )?;
                self.wrap_call_argument_bindings(timestamp, &[(name, HirType::Json, value)])
            }
            HirType::I64 | HirType::Symbol => {
                let message = if ty == HirType::I64 {
                    "Cannot convert a BigInt value to a number"
                } else {
                    "Cannot convert a Symbol value to a number"
                };
                let name = format!("__thaw_date_invalid_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                // The argument is evaluated before this body. Use the ordinary tagged
                // throw statement so catch/async rejection keeps Error provenance.
                let body = HirExpr::Block(vec![
                    HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                        "\u{1}TypeError\u{1}{message}"
                    )))),
                    HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(f64::NAN)))),
                ]);
                self.wrap_call_argument_bindings(body, &[(name, ty, value)])
            }
            _ => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_date_time_clip".into())),
                vec![self.coerce_primitive_to_number(value)?],
            )),
        }
    }

    fn collect_generator_for_array_spread(
        &mut self,
        generator: HirExpr,
        generator_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Function(params, result) = generator_type else {
            return Ok(None);
        };
        let [
            HirType::I64,
            HirType::Json,
            input,
            completion,
            request,
            forced,
        ] = params.as_slice()
        else {
            return Ok(None);
        };
        let HirType::Array(element) = result.as_ref() else {
            return Ok(None);
        };
        let element = element.as_ref().clone();
        let array_type = result.as_ref().clone();
        let producer = format!("__thaw_spread_generator_{}", self.next_binding);
        self.next_binding += 1;
        let output = format!("__thaw_spread_output_{}", self.next_binding);
        self.next_binding += 1;
        let chunk = format!("__thaw_spread_chunk_{}", self.next_binding);
        self.next_binding += 1;
        let empty_channel = |ty: &HirType| {
            HirExpr::TypedClosure(ty.clone(), Box::new(HirExpr::ArrayLit(Vec::new())))
        };
        let resume = HirExpr::Call(
            Box::new(HirExpr::Var(producer.clone())),
            vec![
                HirExpr::Lit(HirLit::I64(0)),
                HirExpr::Lit(HirLit::Str(String::new())),
                generator_placeholder(input).ok_or_else(|| {
                    format!("generator input type {input:?} has no default value")
                })?,
                empty_channel(completion),
                empty_channel(request),
                empty_channel(forced),
            ],
        );
        let value = format!("__thaw_spread_value_{}", self.next_binding);
        self.next_binding += 1;
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                output.clone(),
                array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::Let(
                chunk.clone(),
                array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::While(
                HirExpr::Lit(HirLit::Bool(true)),
                vec![
                    HirStmt::Expr(HirExpr::Assign(chunk.clone(), Box::new(resume))),
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(chunk.clone())))),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        ),
                        vec![HirStmt::Break],
                        Vec::new(),
                    ),
                    // The producer's own resumption check (`lower_generator_
                    // state_machine`, thaw-hir's `declarations.rs`) treats a
                    // *non-empty* `values` capture as "a value is still
                    // waiting to be collected" and short-circuits straight
                    // back to it on the *next* call, without ever advancing
                    // past it -- it never re-dispatches on `state`. A
                    // resume() call's returned array chunk *is* that same
                    // captured `values` array (returned by reference, not a
                    // copy), so the caller is required to *drain* it (not
                    // just read it) before calling again, exactly like
                    // `for...of`'s `resume_generator`/`.next()`'s own
                    // dispatch already do via `__thaw_array_shift`. Reading
                    // via `ArrayConcat` alone (the original, buggy version
                    // of this fix) never drained it, leaving `values`
                    // permanently non-empty after the first yield and
                    // hanging forever, re-yielding the same first value on
                    // every subsequent call (a real, confirmed infinite-
                    // loop/OOM bug -- see round20 in memory).
                    HirStmt::Let(
                        value.clone(),
                        element.clone(),
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_shift".to_string())),
                            vec![HirExpr::Var(chunk.clone())],
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        output.clone(),
                        Box::new(HirExpr::ArrayConcat(
                            vec![
                                HirExpr::Var(output.clone()),
                                HirExpr::ArrayLit(vec![HirExpr::Var(value.clone())]),
                            ],
                            element.clone(),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(output))),
        ]);
        Ok(Some((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: producer,
                        ty: generator_type.clone(),
                    }],
                    array_type,
                    Box::new(body),
                )),
                vec![generator],
            ),
            element,
        )))
    }

    /// The `async function*` sibling of `collect_generator_for_array_
    /// spread` above, for `Array.fromAsync(asyncGen())`. Structurally
    /// identical -- same drain-via-`__thaw_array_shift` loop, same
    /// producer-call shape -- except an async generator's producer
    /// returns `Promise<Array<T>>` per resume (`generator_function_
    /// type`'s `is_async` branch, `declarations.rs`), not a bare
    /// `Array<T>`, so each resume call needs an `AwaitPromise` before
    /// its chunk can be drained, and the collecting closure itself must
    /// be `Promise`-returning (thaw's async-lambda codegen,
    /// `compile_lambda`, detects this from the declared return type
    /// plus a genuine `await` reachable in the body -- both true here
    /// -- and compiles a real coroutine automatically, no other change
    /// needed).
    fn collect_async_generator_for_array_spread(
        &mut self,
        generator: HirExpr,
        generator_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Function(params, result) = generator_type else {
            return Ok(None);
        };
        let [
            HirType::I64,
            HirType::Json,
            input,
            completion,
            request,
            forced,
        ] = params.as_slice()
        else {
            return Ok(None);
        };
        let HirType::Promise(inner) = result.as_ref() else {
            return Ok(None);
        };
        let HirType::Array(element) = inner.as_ref() else {
            return Ok(None);
        };
        let element = element.as_ref().clone();
        let array_type = inner.as_ref().clone();
        let producer = format!("__thaw_async_spread_generator_{}", self.next_binding);
        self.next_binding += 1;
        let output = format!("__thaw_async_spread_output_{}", self.next_binding);
        self.next_binding += 1;
        let chunk = format!("__thaw_async_spread_chunk_{}", self.next_binding);
        self.next_binding += 1;
        let empty_channel = |ty: &HirType| {
            HirExpr::TypedClosure(ty.clone(), Box::new(HirExpr::ArrayLit(Vec::new())))
        };
        let resume = HirExpr::AwaitPromise(
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var(producer.clone())),
                vec![
                    HirExpr::Lit(HirLit::I64(0)),
                    HirExpr::Lit(HirLit::Str(String::new())),
                    generator_placeholder(input).ok_or_else(|| {
                        format!("generator input type {input:?} has no default value")
                    })?,
                    empty_channel(completion),
                    empty_channel(request),
                    empty_channel(forced),
                ],
            )),
            array_type.clone(),
        );
        let value = format!("__thaw_async_spread_value_{}", self.next_binding);
        self.next_binding += 1;
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                output.clone(),
                array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::Let(
                chunk.clone(),
                array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::While(
                HirExpr::Lit(HirLit::Bool(true)),
                vec![
                    HirStmt::Expr(HirExpr::Assign(chunk.clone(), Box::new(resume))),
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(chunk.clone())))),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        ),
                        vec![HirStmt::Break],
                        Vec::new(),
                    ),
                    HirStmt::Let(
                        value.clone(),
                        element.clone(),
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_shift".to_string())),
                            vec![HirExpr::Var(chunk.clone())],
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        output.clone(),
                        Box::new(HirExpr::ArrayConcat(
                            vec![
                                HirExpr::Var(output.clone()),
                                HirExpr::ArrayLit(vec![HirExpr::Var(value.clone())]),
                            ],
                            element.clone(),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(output))),
        ]);
        Ok(Some((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: producer,
                        ty: generator_type.clone(),
                    }],
                    HirType::Promise(Box::new(array_type)),
                    Box::new(body),
                )),
                vec![generator],
            ),
            element,
        )))
    }

    /// `Array.fromAsync`'s own sibling for a *literal* array argument
    /// mixing `Promise<T>` and already-resolved `T` elements (real
    /// trigger: `Array.fromAsync([Promise.resolve(1), Promise.resolve(2),
    /// 3])`) -- a fixed-length array literal like this infers as a
    /// `HirType::Tuple` with each position individually, statically typed
    /// (`Promise(F64)`, `Promise(F64)`, `F64` here), never a homogeneous
    /// `Array<Union<Promise<T>, T>>`. Since each `Promise`-typed position
    /// is *already*, individually, a genuine, statically-known `Promise
    /// <T>` (not a union at all), this needs none of the new "maybe a
    /// Promise" machinery -- each one becomes a real `HirExpr::
    /// AwaitPromise`, exactly like an ordinary `await somePromise`, which
    /// the existing frame-splitting planner (`extract_first_frame_await`'s
    /// own `ArrayLit` arm, unmodified) already knows how to hoist one at a
    /// time into a real, sequential coroutine suspend per element --
    /// matching real `Array.fromAsync`'s own spec'd sequential (not
    /// concurrent) per-element `await`. Requires every member to resolve
    /// to the *same* `T` (mixed final element types aren't attempted).
    fn collect_promise_tuple_array_for_from_async(
        &mut self,
        array: HirExpr,
        array_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Tuple(members) = array_type else {
            return Ok(None);
        };
        if members.is_empty()
            || !members
                .iter()
                .any(|member| matches!(member, HirType::Promise(_)))
        {
            return Ok(None);
        }
        let mut resolved_type: Option<HirType> = None;
        for member in members {
            let member_resolved = match member {
                HirType::Promise(inner) => inner.as_ref().clone(),
                other => other.clone(),
            };
            match &resolved_type {
                None => resolved_type = Some(member_resolved),
                Some(existing) if existing == &member_resolved => {}
                Some(_) => return Ok(None),
            }
        }
        let Some(resolved) = resolved_type else {
            return Ok(None);
        };
        let source_name = format!("__thaw_from_async_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), array_type.clone());
        let elements = members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                let access = HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(source_name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    member.clone(),
                );
                match member {
                    HirType::Promise(inner) => {
                        HirExpr::AwaitPromise(Box::new(access), inner.as_ref().clone())
                    }
                    _ => access,
                }
            })
            .collect::<Vec<_>>();
        let output_array_type = HirType::Array(Box::new(resolved));
        let result = HirExpr::ArrayLit(elements);
        Ok(Some((
            self.wrap_call_argument_bindings(result, &[(source_name, array_type.clone(), array)])?,
            output_array_type,
        )))
    }

    /// `Array.fromAsync`'s own sibling for a *homogeneous* array whose
    /// element type is uniformly `Promise<T>` -- no `Union`/`Tuple`
    /// involved at all (real trigger: `Array.fromAsync([delayed(1),
    /// delayed(2), delayed(3)])`, each call to the *same* async function,
    /// so every element shares the exact same `Promise<F64>` type and
    /// collapses to a plain `Array<Promise<F64>>` rather than a `Tuple` or
    /// a `Union` member). Each element is *already*, individually, a
    /// genuine, statically-known `Promise<T>`, so this is a plain
    /// sequential await-and-collect loop (`AwaitPromise` directly, no
    /// runtime discriminant branch needed at all) -- structurally the
    /// `Promise`-only special case of `collect_promise_union_array_for_
    /// from_async` below, kept separate since there's no union tag to
    /// read here.
    fn collect_promise_array_for_from_async(
        &mut self,
        array: HirExpr,
        array_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Array(element) = array_type else {
            return Ok(None);
        };
        let HirType::Promise(resolved) = element.as_ref() else {
            return Ok(None);
        };
        let resolved = resolved.as_ref().clone();
        let source_name = format!("__thaw_from_async_source_{}", self.next_binding);
        self.next_binding += 1;
        let output = format!("__thaw_from_async_output_{}", self.next_binding);
        self.next_binding += 1;
        let index = format!("__thaw_from_async_index_{}", self.next_binding);
        self.next_binding += 1;
        let value = format!("__thaw_from_async_value_{}", self.next_binding);
        self.next_binding += 1;
        let output_array_type = HirType::Array(Box::new(resolved.clone()));
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                output.clone(),
                output_array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::Let(index.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index.clone())),
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                        source_name.clone(),
                    )))),
                ),
                vec![
                    HirStmt::Let(
                        value.clone(),
                        resolved.clone(),
                        HirExpr::AwaitPromise(
                            Box::new(HirExpr::TypedIndex(
                                Box::new(HirExpr::Var(source_name.clone())),
                                Box::new(HirExpr::Var(index.clone())),
                                element.as_ref().clone(),
                            )),
                            resolved.clone(),
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        output.clone(),
                        Box::new(HirExpr::ArrayConcat(
                            vec![
                                HirExpr::Var(output.clone()),
                                HirExpr::ArrayLit(vec![HirExpr::Var(value.clone())]),
                            ],
                            resolved.clone(),
                        )),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(output))),
        ]);
        Ok(Some((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: source_name,
                        ty: array_type.clone(),
                    }],
                    HirType::Promise(Box::new(output_array_type.clone())),
                    Box::new(body),
                )),
                vec![array],
            ),
            output_array_type,
        )))
    }

    /// `Array.fromAsync`'s own sibling for a plain array literal/variable
    /// mixing `Promise<T>` and already-resolved `T` elements (real trigger:
    /// `Array.fromAsync([Promise.resolve(1), Promise.resolve(2), 3])`) --
    /// the general delegation this function's own caller otherwise falls
    /// back to hands the whole array off to QuickJS's native `Array.
    /// fromAsync` via a JSON-encoded argument, and JSON has no
    /// representation for a live `Promise` object at all ("cannot
    /// serialize collection element Promise(T) to JSON"). A native,
    /// sequential per-element loop sidesteps that boundary entirely --
    /// `await element` on each `Union<Promise<T>, T>`-typed element now
    /// resolves correctly (the `await`-on-a-maybe-`Promise`-`Union` fix,
    /// `compile_await_promise_union`/`infer_expr_type`'s matching `Union`
    /// arm) -- matching real `Array.fromAsync`'s own spec'd behavior of
    /// awaiting each source element in turn, not concurrently.
    fn collect_promise_union_array_for_from_async(
        &mut self,
        array: HirExpr,
        array_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Array(element) = array_type else {
            return Ok(None);
        };
        let HirType::Union(members) = element.as_ref() else {
            return Ok(None);
        };
        let promise_member = members.iter().enumerate().find_map(|(index, member)| {
            if let HirType::Promise(resolved) = member {
                Some((index, resolved.as_ref().clone()))
            } else {
                None
            }
        });
        let Some((promise_index, resolved)) = promise_member else {
            return Ok(None);
        };
        let other_members = members
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != promise_index)
            .map(|(_, member)| member)
            .collect::<Vec<_>>();
        if other_members.len() != 1 || other_members[0] != &resolved {
            return Ok(None);
        }
        let element_type = element.as_ref().clone();
        let source_name = format!("__thaw_from_async_source_{}", self.next_binding);
        self.next_binding += 1;
        let output = format!("__thaw_from_async_output_{}", self.next_binding);
        self.next_binding += 1;
        let index = format!("__thaw_from_async_index_{}", self.next_binding);
        self.next_binding += 1;
        let value = format!("__thaw_from_async_value_{}", self.next_binding);
        self.next_binding += 1;
        let output_array_type = HirType::Array(Box::new(resolved.clone()));
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                output.clone(),
                output_array_type.clone(),
                HirExpr::ArrayLit(Vec::new()),
            ),
            HirStmt::Let(index.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index.clone())),
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                        source_name.clone(),
                    )))),
                ),
                vec![
                    HirStmt::Let(
                        value.clone(),
                        resolved.clone(),
                        HirExpr::Await(Box::new(HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(source_name.clone())),
                            Box::new(HirExpr::Var(index.clone())),
                            element_type.clone(),
                        ))),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        output.clone(),
                        Box::new(HirExpr::ArrayConcat(
                            vec![
                                HirExpr::Var(output.clone()),
                                HirExpr::ArrayLit(vec![HirExpr::Var(value.clone())]),
                            ],
                            resolved.clone(),
                        )),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(output))),
        ]);
        Ok(Some((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: source_name,
                        ty: array_type.clone(),
                    }],
                    HirType::Promise(Box::new(output_array_type.clone())),
                    Box::new(body),
                )),
                vec![array],
            ),
            output_array_type,
        )))
    }

    /// A literal (`await 5`, `await null`, `await undefined`, `await -1`): nothing to join.
    fn is_plain_await_operand(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Lit(_) => true,
            Expr::Unary(unary) => matches!(unary.arg.as_ref(), Expr::Lit(_)),
            Expr::Ident(ident) => ident.sym == *"undefined" && !self.scope.contains_key("undefined"),
            _ => false,
        }
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        if has_dynamic_storage(expr.span()) {
            match expr {
                Expr::Object(object) => {
                    return self.lower_dynamic_accessor_object_lit(object, true);
                }
                Expr::New(new)
                    if matches!(new.callee.as_ref(), Expr::Ident(name)
                    if matches!(name.sym.as_ref(), "Array" | "Map" | "Set" | "WeakMap" | "WeakSet")) =>
                {
                    return self.lower_dynamic_storage_constructor(new);
                }
                _ => {}
            }
        }
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::BigInt(n)) => {
                let text = n.value.to_string();
                match text.parse::<i64>() {
                    Ok(value) => Ok(HirExpr::Lit(HirLit::I64(value))),
                    // A `bigint` beyond Thaw's fixed-width `i64` is kept as
                    // the real QuickJS BigInt, so `.toString()`, radix
                    // conversion, and comparisons stay exact. Arithmetic on
                    // it stays dynamic (Thaw's native `bigint` operators
                    // only cover the `i64` range).
                    Err(_) => {
                        let constructor = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("BigInt".into()))],
                        );
                        let arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::Str(text))]),
                        )?;
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
                            vec![constructor, arguments],
                        ))
                    }
                }
            }
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(hir_string_literal_from_wtf8(
                s.value.as_wtf8().as_bytes(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Lit(Lit::Null(_)) => Ok(HirExpr::Lit(HirLit::Null)),
            Expr::Lit(Lit::Regex(regex)) => Ok(HirExpr::ObjectLit(vec![
                (
                    "source".to_string(),
                    HirExpr::Lit(HirLit::Str(regex.exp.to_string())),
                ),
                (
                    "flags".to_string(),
                    HirExpr::Lit(HirLit::Str(regex.flags.to_string())),
                ),
                ("lastIndex".to_string(), HirExpr::Lit(HirLit::F64(0.0))),
            ])),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                if self.deferred_generic_binds.contains_key(&name) {
                    return Err(format!(
                        "generic bound function `{name}` needs a call-site specialization before it can be used as a value"
                    ));
                }
                if let Some(HirType::NonArrowReceiver(receiver)) = self.scope.get(&name) {
                    return Ok(HirExpr::NonArrowThis {
                        payload: Box::new(HirExpr::Var(name)),
                        declared: receiver.as_ref().clone(),
                        access: NonArrowThisAccess::Value,
                    });
                }
                if matches!(
                    self.generic_callable_template(&name),
                    Some(GenericCallableTemplate::Named { .. })
                ) {
                    return Err(format!(
                        "generic function template `{name}` needs a call-site specialization before it can be used as a value"
                    ));
                }
                // A read of a `new Proxy(target, handler)`'s own `target`
                // identifier, redirected to re-read the live QuickJS
                // handle a `set` trap's mutation actually reaches (see
                // `proxy_target_live_handles`'s own doc comment). Checked
                // first, ahead of every other narrowing below, since none
                // of those apply to this variable (it's never re-declared
                // after becoming a Proxy target in a way that would also
                // need narrowing).
                if let Some((handle_name, declared_ty)) =
                    self.proxy_target_live_handles.get(&name).cloned()
                {
                    let json = HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![HirExpr::Var(handle_name)],
                    );
                    return self.coerce_to_declared(&declared_ty, json);
                }
                if !self.scope.contains_key(&name) {
                    if let Some(signature) = self.signatures.get(&name) {
                        if !signature.generic_type_params.is_empty() {
                            return Err(format!(
                                "function value `{name}` needs a monomorphic native implementation"
                            ));
                        }
                        if signature.is_extern {
                            // A monomorphic extern has no function body to point at, so a
                            // value use is a closure forwarding to the call.
                            let dynamic = dynamic_symbol(&name);
                            let physical_signature = quickjs_function_source_type(
                                signature.source_range.0,
                                HirType::Function(
                                    signature.params.clone(),
                                    Box::new(signature.ret.clone()),
                                ),
                            );
                            let (physical_params, call_ret) = match physical_signature {
                                HirType::Function(params, ret) => (params, *ret),
                                other => {
                                    return Err(format!(
                                        "internal QuickJS function-value ABI is not callable: {other:?}"
                                    ));
                                }
                            };
                            let ret = if signature.is_async {
                                HirType::Promise(Box::new(call_ret.clone()))
                            } else {
                                call_ret.clone()
                            };
                            let params = physical_params
                                .iter()
                                .enumerate()
                                .map(|(index, ty)| HirParam {
                                    name: format!("__thaw_extern_arg_{}", self.next_binding + index),
                                    ty: ty.clone(),
                                })
                                .collect::<Vec<_>>();
                            self.next_binding += params.len();
                            let arguments = params
                                .iter()
                                .map(|param| HirExpr::Var(param.name.clone()))
                                .collect();
                            let call = match dynamic {
                                Some((backend, symbol)) => HirExpr::DynamicCall(
                                    DynamicSignature {
                                        backend,
                                        symbol,
                                        params: physical_params,
                                        ret: call_ret,
                                    },
                                    arguments,
                                ),
                                None => HirExpr::Call(Box::new(HirExpr::Var(name)), arguments),
                            };
                            return Ok(HirExpr::Lambda(
                                Vec::new(),
                                params,
                                ret.clone(),
                                Box::new(HirExpr::Block(vec![HirStmt::Return(Some(call))])),
                            ));
                        }
                        let ret = if signature.is_async {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        };
                        if !signature.public_overloads.is_empty() {
                            let physical = if let Some(rest) = &signature.native_rest {
                                HirType::CallableFunction(
                                    signature.params[..signature.params.len() - 1].to_vec(),
                                    signature.source_omittable.clone(),
                                    Some(Box::new(rest.clone())),
                                    Box::new(ret.clone()),
                                )
                            } else {
                                HirType::Function(signature.params.clone(), Box::new(ret.clone()))
                            };
                            let normalized = self.normalize_function_ref_value(HirExpr::FunctionRef(
                                name,
                                signature.params.clone(),
                                ret,
                            ))?;
                            return Ok(HirExpr::TypedClosure(
                                HirType::OverloadedFunction(signature.public_overloads.clone(), Box::new(physical)),
                                Box::new(normalized),
                            ));
                        }
                        // ponytail: a `this`-using function stays a plain FunctionRef;
                        // FunctionRefThis needs `.bind` lowering and llvm support first.
                        return self.normalize_function_ref_value(HirExpr::FunctionRef(
                            name,
                            signature.params.clone(),
                            ret,
                        ));
                    }
                    // A bare class-name reference (not immediately
                    // `new`'d) -- real trigger: `class-transformer`'s
                    // own `plainToInstance(User, plain)`, a common
                    // factory-function idiom (pass a class constructor
                    // as a plain argument; the real JS implementation
                    // does `new cls()` internally). A class never
                    // registers anything under its own bare name here
                    // (everything lives under the mangled `class_
                    // constructor_symbol`), so this fell through to the
                    // generic `HirExpr::Var(name)` default below,
                    // producing "unknown variable" the moment anything
                    // tried to infer its type. Mirrors the identical
                    // pattern already used for a static method's own
                    // `this` just below (`class_constructor_symbol` +
                    // `self.interfaces` lookup) -- a class's constructor
                    // is already an ordinary, referenceable native
                    // function (`new User(...)` itself desugars to a
                    // plain call to this exact symbol), so this just
                    // makes the bare identifier resolve to it the same
                    // way an ordinary plain function value already does
                    // just above. Every downstream consumer (`Function`-
                    // to-`JsValue` coercion via `registerNativeCallback`,
                    // dynamic-call argument marshaling, and real JS
                    // `new`-on-a-returning-constructor semantics) already
                    // exists and needs no further change.
                    let constructor = class_constructor_symbol(&name);
                    if let Some(signature) = self.signatures.get(&constructor) {
                        let result = self
                            .interfaces
                            .get(&name)
                            .cloned()
                            .ok_or_else(|| format!("class `{name}` has no layout"))?;
                        // A *decorated* class already has one real, live
                        // "class token" `JsValue` (see `DECORATOR_CLASS_
                        // TOKENS`/`lower_class_decorator_tokens`) -- the
                        // exact object its own decorators received as
                        // `target`, and the one metadata registrars like
                        // class-transformer key their registrations by. A
                        // bare reference must resolve to *that* token, not
                        // a fresh native-callback wrapper: otherwise real
                        // `plainToInstance(User, plain)` can't find the
                        // `@Expose`/`@Type`/`@Transform` metadata it
                        // recorded against the token (real symptom: every
                        // transform silently skipped). Falls back to the
                        // native constructor reference for an undecorated
                        // class, which has no token.
                        let token = match &result {
                            HirType::Object(fields) => fields.first().and_then(|(marker, _)| {
                                DECORATOR_CLASS_TOKENS
                                    .with(|tokens| tokens.borrow().get(marker).cloned())
                            }),
                            _ => None,
                        };
                        if let Some(token) = token {
                            return Ok(HirExpr::Var(token));
                        }
                        return Ok(HirExpr::FunctionRef(
                            constructor,
                            signature.params.clone(),
                            result,
                        ));
                    }
                }
                if !self.scope.contains_key(&name) && !self.signatures.contains_key(&name) {
                    match ident.sym.as_ref() {
                        "NaN" => return Ok(HirExpr::Lit(HirLit::F64(f64::NAN))),
                        "Infinity" => return Ok(HirExpr::Lit(HirLit::F64(f64::INFINITY))),
                        "undefined" => return Ok(HirExpr::Lit(HirLit::Undefined)),
                        // `String`/`Number`/`Boolean` referenced bare
                        // (not called) are real first-class function
                        // values in JS (`typeof String === 'function'`,
                        // `schema.name === String`) -- the idiom real
                        // `mongoose` schemas use (`{ name: String, age:
                        // Number }`). This never interferes with the
                        // existing `String(x)`/`Number(x)`/`Boolean(x)`
                        // call-position coercion intrinsics: those are
                        // intercepted by `lower_call` reading the callee
                        // name directly, which never routes through this
                        // bare-identifier expression lowering at all.
                        global @ ("Atomics" | "crypto" | "process" | "AbortSignal" | "String"
                        | "Number" | "Boolean" | "globalThis"
                        // Standard namespace objects used as values (`typeof
                        // JSON !== "undefined"`, `const ns = Math`): the realm
                        // has the real global. A known member call
                        // (`JSON.stringify`, `Math.floor`) is intercepted
                        // earlier by name.
                        | "JSON" | "Math" | "Reflect" | "Intl"
                        // Web / WHATWG globals (thaw's QuickJS platform
                        // globals provide them). Naming one bare --
                        // `typeof Headers`, `const Ctor = URL` -- yields the
                        // real global as an opaque dynamic value; the
                        // `new`/call positions are intercepted earlier.
                        | "structuredClone"
                        | "queueMicrotask"
                        | "URL"
                        | "URLSearchParams"
                        | "Headers"
                        | "Request"
                        | "Response"
                        | "Blob"
                        | "File"
                        | "FormData"
                        | "Event"
                        | "EventTarget"
                        | "MessageEvent"
                        | "MessageChannel"
                        | "MessagePort"
                        | "BroadcastChannel"
                        | "DOMException"
                        | "ReadableStream"
                        | "WritableStream"
                        | "TransformStream"
                        | "TextEncoder"
                        | "TextDecoder"
                        | "AbortController") => {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(global.to_string()))],
                            ))
                        }
                        // A bare standard-library *constructor* used as a
                        // value (`assert.throws(TypeError, ...)`,
                        // `Object.getPrototypeOf(x) === Array.prototype`):
                        // the realm has the real global. A `.prototype`/member
                        // read is intercepted earlier; this covers a value
                        // position.
                        global if is_builtin_prototype_owner(global) => {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(global.to_string()))],
                            ))
                        }
                        _ => {}
                    }
                }
                if let Some(target) = self.exception_object_narrowings.get(&name).cloned() {
                    let value = if self.catch_bindings.contains(&name) {
                        HirExpr::Var(format!("{name}__thaw_exception_object"))
                    } else {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_pending_exception_object".to_string())),
                            Vec::new(),
                        )
                    };
                    return Ok(HirExpr::Conditional(
                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                        Box::new(value.clone()),
                        Box::new(value),
                        target,
                    ));
                }
                if let Some((allowed, elements)) = self.union_narrowings.get(&name) {
                    if let [index] = allowed.as_slice() {
                        let value = match self.scope.get(&name) {
                            Some(HirType::Optional(payload))
                                if matches!(payload.as_ref(), HirType::Union(_)) =>
                            {
                                HirExpr::OptionalValue(
                                    Box::new(HirExpr::Var(name)),
                                    payload.as_ref().clone(),
                                )
                            }
                            _ => HirExpr::Var(name),
                        };
                        return Ok(HirExpr::UnionValue(
                            Box::new(value),
                            *index,
                            elements.clone(),
                        ));
                    }
                    // Dropping the trailing `undefined` member preserves every
                    // surviving tag and the union's native two-word layout.
                    if elements.last() == Some(&HirType::Undefined)
                        && allowed.len() + 1 == elements.len()
                        && allowed.iter().copied().eq(0..allowed.len())
                    {
                        return Ok(HirExpr::TypedClosure(
                            HirType::Union(elements[..allowed.len()].to_vec()),
                            Box::new(HirExpr::Var(name)),
                        ));
                    }
                }
                if let Some(narrowed) = self.json_narrowings.get(&name) {
                    let value = Box::new(HirExpr::Var(name));
                    return Ok(match narrowed {
                        HirType::Str => HirExpr::JsonAsString(value),
                        HirType::F64 => HirExpr::JsonAsNumber(value),
                        HirType::Bool => HirExpr::JsonAsBool(value),
                        _ => unreachable!("validated typeof narrowing target"),
                    });
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
                    }) {
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
                    None => {
                        if let Some(payload) = self.partial_nullable_narrowings.get(&name) {
                            let value = HirExpr::Var(name);
                            Ok(HirExpr::Block(vec![
                                HirStmt::If(
                                    HirExpr::NullishIsNull(
                                        Box::new(value.clone()),
                                        payload.clone(),
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::NullableNone(
                                        payload.clone(),
                                    )))],
                                    Vec::new(),
                                ),
                                HirStmt::Return(Some(HirExpr::NullableSome(
                                    Box::new(HirExpr::NullishValue(
                                        Box::new(value),
                                        payload.clone(),
                                    )),
                                    payload.clone(),
                                ))),
                            ]))
                        } else {
                            Ok(HirExpr::Var(name))
                        }
                    }
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
                    let result =
                        self.interfaces.get(class).cloned().ok_or_else(|| {
                            format!("static `this` class `{class}` has no layout")
                        })?;
                    return Ok(HirExpr::FunctionRef(
                        constructor,
                        signature.params.clone(),
                        result,
                    ));
                }
                if let Some((name, declared)) = &self.active_non_arrow_this {
                    return Ok(HirExpr::NonArrowThis {
                        payload: Box::new(HirExpr::Var(name.clone())),
                        declared: declared.clone(),
                        access: NonArrowThisAccess::Value,
                    });
                }
                let name = self.resolve_binding("this");
                if self.scope.contains_key(&name) {
                    Ok(HirExpr::Var(name))
                } else {
                    Err(
                        "`this` is only available inside a native class constructor or method"
                            .into(),
                    )
                }
            }
            Expr::Paren(paren) => self.lower_expr(&paren.expr),
            Expr::TsAs(assertion) => {
                // `(e as T)` on a caught exception (`e`, always
                // `HirType::Str` -- see `statements/lowering.rs`) reads the
                // parallel object channel an object `throw`
                // populated alongside the tagged string (see
                // `docs/design/exceptions.md` section 3 and
                // `HirStmt::Throw`'s lowering just above `Stmt::Throw`),
                // instead of the ordinary (no-op) type-assertion passthrough
                // every other `as` cast uses. This is the only way to reach
                // fields beyond `.message`/`.name` at a catch site; nothing
                // checks that an `instanceof` guard actually preceded it,
                // matching real TypeScript's own unchecked `as` -- unlike
                // real TypeScript, though, misusing it here (on a caught
                // value that was never an object, e.g. a plain `throw "x"`)
                // raises a normal Thaw exception instead of exposing the
                // parallel channel's null pointer.
                if let (Expr::Ident(ident), TsType::TsTypeRef(reference)) =
                    (assertion.expr.as_ref(), assertion.type_ann.as_ref())
                {
                    if let swc_ecma_ast::TsEntityName::Ident(class) = &reference.type_name {
                        let resolved = self.resolve_binding(ident.sym.as_ref());
                        let target =
                            self.interfaces
                                .get(class.sym.as_ref())
                                .cloned()
                                .or_else(|| {
                                    lower_ts_type(
                                        &assertion.type_ann,
                                        self.interfaces,
                                        self.generic_interfaces,
                                    )
                                    .ok()
                                })
                                .map(|target| {
                                    if matches!(target, HirType::Promise(_)) {
                                        storage_type(assertion.span, target)
                                    } else {
                                        target
                                    }
                                });
                        if self.scope.get(&resolved)
                            == Some(&crate::caught_exception_carrier_type())
                        {
                            if let Some(expected_tag) = crate::native_exception_tag(
                                &target.clone().unwrap_or(HirType::Void),
                            ) {
                                let target = target.clone().unwrap();
                                let layout = crate::native_exception_layout_token(&target).unwrap();
                                let HirType::Union(members) =
                                    crate::caught_exception_carrier_type()
                                else {
                                    unreachable!()
                                };
                                let carrier = HirExpr::Var(resolved.clone());
                                let descriptor =
                                    HirExpr::UnionValue(Box::new(carrier.clone()), 9, members);
                                let owner = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_native_exception_owner".into())),
                                    vec![
                                        descriptor,
                                        HirExpr::Lit(HirLit::I64(expected_tag as i64)),
                                        HirExpr::Lit(HirLit::Str(layout)),
                                    ],
                                );
                                let present = HirExpr::Call(
                                    Box::new(HirExpr::Var(
                                        "__thaw_native_exception_owner_present".into(),
                                    )),
                                    vec![owner.clone()],
                                );
                                return Ok(HirExpr::Conditional(
                                    Box::new(present),
                                    Box::new(HirExpr::TypedClosure(target.clone(), Box::new(owner))),
                                    Box::new(HirExpr::ThrowValue(
                                        Box::new(HirExpr::Lit(HirLit::Str(
                                            "caught native value has an incompatible category or layout".into(),
                                        ))),
                                        Box::new(Self::unreachable_value(&target)?),
                                    )),
                                    target,
                                ));
                            }
                            if matches!(&target, Some(HirType::Object(fields))
                                if has_dynamic_storage(assertion.span)
                                    && !fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_")))
                            {
                                return self.caught_carrier_as_json(HirExpr::Var(resolved));
                            }
                            if let Some(target @ HirType::Object(_)) = target.clone() {
                                let HirType::Object(target_fields) = &target else {
                                    unreachable!()
                                };
                                let HirType::Union(members) =
                                    crate::caught_exception_carrier_type()
                                else {
                                    unreachable!()
                                };
                                let carrier = HirExpr::Var(resolved);
                                let native = HirExpr::TypedClosure(
                                    target.clone(),
                                    Box::new(HirExpr::UnionValue(
                                        Box::new(carrier.clone()),
                                        6,
                                        members.clone(),
                                    )),
                                );
                                let checked = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_assert_class_identity".into())),
                                    vec![
                                        native,
                                        HirExpr::Lit(HirLit::Str(
                                            // An interface/alias target has no nominal identity.
                                            target_fields
                                                .first()
                                                .and_then(|(name, _)| {
                                                    name.strip_prefix(
                                                        "__thaw_class_identity_\u{1e}",
                                                    )
                                                })
                                                .filter(|ancestry| {
                                                    ancestry
                                                        .split('\u{1f}')
                                                        .any(|name| name == class.sym.as_ref())
                                                })
                                                .map(|_| class.sym.to_string())
                                                .unwrap_or_default(),
                                        )),
                                        HirExpr::Lit(HirLit::Str(
                                            crate::native_object_layout_token(target_fields),
                                        )),
                                    ],
                                );
                                return Ok(HirExpr::Conditional(
                                    Box::new(HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::UnionTag(Box::new(carrier), members)),
                                        Box::new(HirExpr::Lit(HirLit::F64(6.0))),
                                    )),
                                    Box::new(checked),
                                    Box::new(HirExpr::ThrowValue(
                                        Box::new(HirExpr::Lit(HirLit::Str(
                                            "caught value is not an object".into(),
                                        ))),
                                        Box::new(Self::unreachable_value(&target)?),
                                    )),
                                    target,
                                ));
                            }
                        }
                        if self.scope.get(&resolved) == Some(&HirType::Str) {
                            if let Some(target @ HirType::Object(_)) = target {
                                let structural = matches!(&target, HirType::Object(fields)
                                    if !fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_")));
                                let promise_catch = self.promise_catch_bindings.contains(&resolved)
                                    || self.promise_catch_parameter.as_deref().is_some_and(
                                        |name| self.resolve_binding(name) == resolved,
                                    );
                                if structural && (has_dynamic_storage(assertion.span) || promise_catch) {
                                    let carrier = self.promise_rejection_carrier(&resolved);
                                    return self.caught_carrier_as_json(carrier);
                                }
                                let value = if self.catch_bindings.contains(&resolved) {
                                    let object_name = format!("{resolved}__thaw_exception_object");
                                    self.scope.insert(object_name.clone(), target.clone());
                                    HirExpr::Var(object_name)
                                } else {
                                    // Promise rejection adapters restore the
                                    // typed side channel immediately before
                                    // invoking any callback. This also covers
                                    // callbacks stored in local function
                                    // values; ordinary
                                    // string calls see null and take the safe
                                    // ThrowValue branch below.
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_pending_exception_object".to_string(),
                                        )),
                                        Vec::new(),
                                    )
                                };
                                let typed_value = HirExpr::Conditional(
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    Box::new(value.clone()),
                                    Box::new(value.clone()),
                                    target.clone(),
                                );
                                return Ok(HirExpr::Conditional(
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_exception_object_present".to_string(),
                                        )),
                                        vec![value],
                                    )),
                                    Box::new(typed_value),
                                    Box::new(HirExpr::ThrowValue(
                                        Box::new(HirExpr::Lit(HirLit::Str(
                                            "caught value is not an object".to_string(),
                                        ))),
                                        Box::new(Self::unreachable_value(&target)?),
                                    )),
                                    target,
                                ));
                            }
                        }
                    }
                }
                if let Expr::Ident(ident) = assertion.expr.as_ref() {
                    let resolved = self.resolve_binding(ident.sym.as_ref());
                    let promise_catch = self.promise_catch_bindings.contains(&resolved)
                        || self
                            .promise_catch_parameter
                            .as_deref()
                            .is_some_and(|name| self.resolve_binding(name) == resolved);
                    if self.scope.get(&resolved) == Some(&crate::caught_exception_carrier_type()) {
                        let target = lower_ts_type(
                            &assertion.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )?;
                        let target = if matches!(target, HirType::Promise(_)) {
                            storage_type(assertion.span, target)
                        } else {
                            target
                        };
                        let projection = match &target {
                            HirType::F64 => Some((0, "number")),
                            HirType::I64 => Some((1, "bigint")),
                            HirType::Bool => Some((2, "boolean")),
                            _ => None,
                        };
                        if let Some((native_index, json_type)) = projection {
                            let carrier_type = crate::caught_exception_carrier_type();
                            let HirType::Union(members) = carrier_type else {
                                unreachable!()
                            };
                            let carrier = HirExpr::Var(resolved.clone());
                            let tag = HirExpr::UnionTag(Box::new(carrier.clone()), members.clone());
                            let fail = HirExpr::ThrowValue(
                                Box::new(HirExpr::Lit(HirLit::Str(format!(
                                    "caught value is not a {json_type}"
                                )))),
                                Box::new(Self::unreachable_value(&target)?),
                            );
                            let native_value = HirExpr::UnionValue(
                                Box::new(carrier.clone()),
                                native_index,
                                members.clone(),
                            );
                            let json =
                                HirExpr::UnionValue(Box::new(carrier.clone()), 7, members.clone());
                            let json_typeof = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_typeof".into())),
                                vec![json.clone()],
                            );
                            let json_matches = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(json_typeof),
                                Box::new(HirExpr::Lit(HirLit::Str(json_type.into()))),
                            );
                            // Keep every Json dereference nested under the tag-7
                            // branch. Union payload bits are otherwise opaque and
                            // must never be passed to a Json FFI as a pointer.
                            let projected_json = match &target {
                                HirType::F64 => HirExpr::JsonAsNumber(Box::new(json.clone())),
                                HirType::I64 => HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_as_bigint_i64".into())),
                                    vec![json.clone()],
                                ),
                                HirType::Bool => HirExpr::JsonAsBool(Box::new(json.clone())),
                                _ => unreachable!("only scalar targets reach this branch"),
                            };
                            let json_value = HirExpr::Conditional(
                                Box::new(json_matches),
                                Box::new(projected_json),
                                Box::new(fail.clone()),
                                target.clone(),
                            );
                            let is_json = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(tag.clone()),
                                Box::new(HirExpr::Lit(HirLit::F64(7.0))),
                            );
                            let is_native = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(tag),
                                Box::new(HirExpr::Lit(HirLit::F64(native_index as f64))),
                            );
                            let json_or_error = HirExpr::Conditional(
                                Box::new(is_json),
                                Box::new(json_value),
                                Box::new(fail),
                                target.clone(),
                            );
                            return Ok(HirExpr::Conditional(
                                Box::new(is_native),
                                Box::new(native_value),
                                Box::new(json_or_error),
                                target,
                            ));
                        }
                    }
                    if self.catch_bindings.contains(&resolved) || promise_catch {
                        let target = lower_ts_type(
                            &assertion.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )?;
                        let target = if matches!(target, HirType::Promise(_)) {
                            storage_type(assertion.span, target)
                        } else {
                            target
                        };
                        let suffix = match target {
                            HirType::F64 => Some("f64"),
                            HirType::I64 => Some("i64"),
                            HirType::Bool => Some("bool"),
                            _ => None,
                        };
                        if let Some(suffix) = suffix {
                            if promise_catch {
                                return Ok(HirExpr::Call(
                                    Box::new(HirExpr::Var(format!(
                                        "__thaw_pending_exception_{suffix}"
                                    ))),
                                    Vec::new(),
                                ));
                            }
                            let name = format!("{resolved}__thaw_exception_{suffix}");
                            self.scope.insert(name.clone(), target);
                            return Ok(HirExpr::Var(name));
                        }
                    }
                }
                if let Some(literal) =
                    self.lower_any_asserted_literal(&assertion.expr, &assertion.type_ann, false)?
                {
                    return Ok(literal);
                }
                self.lower_expr(&assertion.expr)
            }
            Expr::TsTypeAssertion(assertion) => {
                if let Some(literal) =
                    self.lower_any_asserted_literal(&assertion.expr, &assertion.type_ann, false)?
                {
                    return Ok(literal);
                }
                self.lower_expr(&assertion.expr)
            }
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
                let mut statements = values.into_iter().map(HirStmt::Expr).collect::<Vec<_>>();
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
                    let literal = match quasi.cooked.as_ref() {
                        Some(cooked) => hir_string_literal_from_wtf8(cooked.as_wtf8().as_bytes()),
                        None => HirLit::Str(quasi.raw.to_string()),
                    };
                    if !matches!(&literal, HirLit::Str(text) if text.is_empty()) {
                        parts.push(HirExpr::Lit(literal));
                    }
                    if let Some(expression) = template.exprs.get(index) {
                        let mut value = self.lower_expr(expression)?;
                        value = self.coerce_operand_to_string(value)?;
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
                            let mut parts =
                                Vec::with_capacity(template.quasis.len() + template.exprs.len());
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
                // The source position identifies the site; synthesized code has none.
                let site = if tagged.span.lo.0 == 0 {
                    self.next_binding += 1;
                    f64::from(0x8000_0000u32) + self.next_binding as f64
                } else {
                    f64::from(tagged.span.lo.0)
                };
                let mut cooked_strings = Vec::with_capacity(template.quasis.len());
                let mut raw_strings = Vec::with_capacity(template.quasis.len());
                for quasi in &template.quasis {
                    // An invalid escape makes the cooked entry `undefined`; the
                    // array keeps `Array<Str>`'s one-pointer-per-element layout.
                    cooked_strings.push(HirExpr::Lit(match quasi.cooked.as_ref() {
                        Some(cooked) => hir_string_literal_from_wtf8(cooked.as_wtf8().as_bytes()),
                        None => HirLit::Undefined,
                    }));
                    raw_strings.push(HirExpr::Lit(HirLit::Str(quasi.raw.to_string())));
                }
                // The tag function receives `cooked` -- `strings.raw` is a
                // real Web-platform requirement of that same array, not a
                // separate argument, so its raw sibling has to be reachable
                // *from* the cooked array's own identity: register it in a
                // runtime side table keyed by the cooked array's buffer
                // address (`thaw_template_strings_register`, thaw-runtime's
                // `template_strings.rs`), the same "metadata keyed by a
                // stable buffer identity" pattern `RegexMatchMeta` uses for
                // `.index`/`.input`/`.groups` (`regex.rs`) -- before calling
                // the tag, so a `.raw` read inside the tag function's own
                // body (or after it returns) can recover it.
                let cooked_name = format!("__thaw_template_cooked_{}", self.next_binding);
                self.next_binding += 1;
                let array_type = HirType::Array(Box::new(HirType::Str));
                self.scope.insert(cooked_name.clone(), array_type.clone());
                let mut args = vec![HirExpr::Var(cooked_name.clone())];
                for expression in &template.exprs {
                    args.push(self.lower_expr(expression)?);
                }
                let tag = self.lower_expr(&tagged.tag)?;
                // A tag that is a dynamic (`any`) value, e.g. a member of one (`o.t`..``), is
                // called like any dynamic method: `tag(strings, ...values)` with the template
                // object handed over as a live view (so `.raw` and identity survive).
                if matches!(
                    self.infer_expr_type(&tag)?,
                    HirType::Json | HirType::JsValue | HirType::Dynamic
                ) {
                    let strings_name = format!("__thaw_template_strings_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(strings_name.clone(), HirType::Json);
                    let site_array = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_template_site".to_string())),
                        vec![
                            HirExpr::Lit(HirLit::F64(site)),
                            HirExpr::ArrayLit(cooked_strings),
                            HirExpr::ArrayLit(raw_strings),
                        ],
                    );
                    let strings = self.coerce_to_declared_aliasing(&HirType::Json, site_array)?;
                    let mut call_args = vec![swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                            strings_name.clone().into(),
                            tagged.span,
                        ))),
                    }];
                    call_args.extend(template.exprs.iter().map(|expression| {
                        swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: expression.clone(),
                        }
                    }));
                    let call = Expr::Call(swc_ecma_ast::CallExpr {
                        span: tagged.span,
                        ctxt: Default::default(),
                        callee: swc_ecma_ast::Callee::Expr(tagged.tag.clone()),
                        args: call_args,
                        type_args: None,
                    });
                    let lowered = self.lower_expr(&call)?;
                    return self.wrap_call_argument_bindings(
                        lowered,
                        &[(strings_name, HirType::Json, strings)],
                    );
                }
                // A tagged-template call is built directly here rather than
                // through `lower_call` (`invocations/calls.rs`, which a
                // tagged template has no `CallExpr` AST node to feed), so
                // it never went through that function's own rest-parameter
                // packing -- the overwhelmingly common tag signature,
                // `(strings, ...values: any[])`, hit an LLVM "incorrect
                // number of arguments" verification failure for more than
                // one interpolation (a pre-existing bug, found while
                // testing `.raw` against a realistic signature, not caused
                // by it). Packs the trailing interpolated values into one
                // native array here too, mirroring `lower_call`'s own
                // `native_rest_array` use, whenever the tag resolves to a
                // named function whose signature declares a rest
                // parameter.
                let tag_name = match &tag {
                    HirExpr::Var(name)
                    | HirExpr::FunctionRef(name, _, _)
                    | HirExpr::FunctionRefThis(name, _, _, _) => Some(name.as_str()),
                    _ => None,
                };
                // The inferred callable type describes the value actually called
                // (including default/rest adapters); FnSignature is only needed
                // for plain physical Function types, which cannot carry rest data.
                let callable = match self.infer_expr_type(&tag)? {
                    HirType::Function(params, _) => tag_name
                        .and_then(|name| self.signatures.get(name))
                        .and_then(|signature| signature.native_rest.as_ref().map(|rest| {
                            (params[..params.len().saturating_sub(1)].to_vec(),
                             HirOptionalMask::default(), Some(rest.clone()))
                        }))
                        .or(Some((params, HirOptionalMask::default(), None))),
                    HirType::CallableFunction(params, optional, rest, _) => {
                        Some((params, optional, rest.map(|element| *element)))
                    }
                    HirType::FunctionWithThis(_, visible) => match *visible {
                        HirType::Function(params, _) => tag_name
                            .and_then(|name| self.signatures.get(name))
                            .and_then(|signature| {
                                signature.native_rest.as_ref().map(|rest| {
                                    (
                                        params[..params.len().saturating_sub(1)].to_vec(),
                                        HirOptionalMask::default(),
                                        Some(rest.clone()),
                                    )
                                })
                            })
                            .or(Some((params, HirOptionalMask::default(), None))),
                        HirType::CallableFunction(params, optional, rest, _) => {
                            Some((params, optional, rest.map(|element| *element)))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let mut extra_statements = Vec::new();
                let args = if let Some((params, optional, rest)) = callable {
                    let fixed_count = params.len();
                    let mut args = args;
                    if args.len() < fixed_count {
                        for (index, parameter) in params.iter().enumerate().skip(args.len()) {
                            if !is_optional_parameter(&optional, index) {
                                return Err(format!(
                                    "tagged template omits required parameter {}",
                                    index + 1
                                ));
                            }
                            args.push(omitted_parameter_value(parameter)?);
                        }
                    }
                    if rest.is_none() && args.len() > fixed_count {
                        let values = args.split_off(1);
                        for (index, value) in values.into_iter().enumerate() {
                            let ty = self.infer_expr_type(&value)?;
                            let name = format!("__thaw_template_arg_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            extra_statements.push(HirStmt::Let(name.clone(), ty, value));
                            if index + 1 < fixed_count {
                                args.push(HirExpr::Var(name));
                            }
                        }
                    }
                    if rest.is_none() {
                        args.truncate(fixed_count);
                    }
                    let trailing = if rest.is_some() && args.len() > fixed_count {
                        args.split_off(fixed_count)
                    } else {
                        Vec::new()
                    };
                    let mut args = args
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| {
                            if index == 0 && index < fixed_count {
                                self.coerce_to_declared_aliasing(&params[index], value)
                            } else if index < fixed_count {
                                self.coerce_to_declared(&params[index], value)
                            } else {
                                Ok(value)
                            }
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    if let Some(element) = rest {
                        let trailing = trailing
                            .into_iter()
                            .map(|value| self.coerce_to_declared(&element, value))
                            .collect::<Result<Vec<_>, _>>()?;
                        args.push(native_rest_array(trailing, &element));
                    }
                    args
                } else {
                    args
                };
                // A tag function may declare fewer parameters than the template has
                // substitutions: JavaScript still evaluates every substitution, in
                // order, and the callee ignores the extras.
                let tag = if extra_statements.is_empty() {
                    tag
                } else {
                    let ty = self.infer_expr_type(&tag)?;
                    let name = format!("__thaw_template_tag_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    extra_statements.insert(0, HirStmt::Let(name.clone(), ty, tag));
                    HirExpr::Var(name)
                };
                let call = HirExpr::Call(Box::new(tag), args);
                // `infer_expr_type` (not the simpler `_inner` variant
                // `wrap_call_argument_bindings` uses for its own upfront
                // type check) is what special-cases a bare `Var` callee to
                // route through the named-function/rest-parameter-aware
                // dispatch (`inference/types.rs`'s `HirExpr::Call` arm) --
                // computing `call_type` here and building the wrapping
                // lambda directly (matching Stage A/B's own `.get()`/
                // `.set()`-style pattern) avoids ever running `call`
                // through that simpler inference, which doesn't know how
                // to arity-check a rest-parameter tag function and
                // rejected a valid call outright.
                let call_type = self.infer_expr_type(&call)?;
                let mut body_statements = Vec::new();
                body_statements.extend(extra_statements);
                if call_type == HirType::Void {
                    body_statements.push(HirStmt::Expr(call));
                    body_statements.push(HirStmt::Return(None));
                } else {
                    body_statements.push(HirStmt::Return(Some(call)));
                }
                let body = HirExpr::Block(body_statements);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter(|name| name != &cooked_name)
                    .filter_map(|captured| {
                        self.scope
                            .get(&captured)
                            .cloned()
                            .map(|ty| HirParam { name: captured, ty })
                    })
                    .collect();
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        vec![HirParam {
                            name: cooked_name,
                            ty: array_type,
                        }],
                        call_type,
                        Box::new(body),
                    )),
                    // One frozen template object per site, created on first use.
                    vec![HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_template_site".to_string())),
                        vec![
                            HirExpr::Lit(HirLit::F64(site)),
                            HirExpr::ArrayLit(cooked_strings),
                            HirExpr::ArrayLit(raw_strings),
                        ],
                    )],
                ))
            }

            Expr::Bin(bin) => {
                // `instance.constructor === Class`: the instance's exact class.
                if matches!(
                    bin.op,
                    BinaryOp::EqEq | BinaryOp::EqEqEq | BinaryOp::NotEq | BinaryOp::NotEqEq
                ) {
                    for (constructor, class) in [(&bin.left, &bin.right), (&bin.right, &bin.left)] {
                        let (Expr::Member(inner), Expr::Ident(class)) =
                            (constructor.as_ref(), class.as_ref())
                        else {
                            continue;
                        };
                        if !self.signatures.contains_key(&class_constructor_symbol(class.sym.as_ref())) {
                            continue;
                        }
                        if let Some(instance) = self.class_instance_of_constructor(inner)? {
                            let same = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_object_class_name".into())),
                                    vec![instance],
                                )),
                                Box::new(HirExpr::Lit(HirLit::Str(
                                    display_class_name(class.sym.as_ref()).to_string(),
                                ))),
                            );
                            return Ok(if matches!(bin.op, BinaryOp::NotEq | BinaryOp::NotEqEq) {
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(same),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                )
                            } else {
                                same
                            });
                        }
                    }
                }
                if bin.op == BinaryOp::InstanceOf {
                    // A variable holding a constructor value (`const k: any = C`) is a
                    // dynamic right operand, not a class name.
                    let dynamic_class_variable = matches!(bin.right.as_ref(), Expr::Ident(name)
                        if matches!(self.scope.get(name.sym.as_ref()), Some(HirType::JsValue | HirType::Dynamic))
                            && !self.interfaces.contains_key(name.sym.as_ref()));
                    if dynamic_class_variable || !matches!(bin.right.as_ref(), Expr::Ident(_)) {
                        // A property-valued constructor needs the realm's exact
                        // instanceof semantics, including custom @@hasInstance.
                        let left = self.lower_expr(&bin.left)?;
                        let left_type = self.infer_expr_type(&left)?;
                        let right = self.lower_expr(&bin.right)?;
                        let right_type = self.infer_expr_type(&right)?;
                        return self.lower_host_instanceof(left, left_type, right, right_type);
                    }
                    let Expr::Ident(class) = bin.right.as_ref() else {
                        unreachable!("non-identifier instanceof RHS handled above");
                    };
                    // True for the built-ins themselves and for any user
                    // class transitively `extends`ing one of them (its
                    // identity chain, synthesized in
                    // `lower/module/classes.rs`, then includes an
                    // Error-family name even though it has no entry of its
                    // own in `self.signatures`/`self.interfaces` under that
                    // name).
                    let extends_error_family = is_error_family_name(class.sym.as_ref())
                        || self
                            .interfaces
                            .get(class.sym.as_ref())
                            .is_some_and(object_type_is_error_family);
                    // `Date` has no entry in `self.signatures` at all --
                    // `new Date()` compiles to a bespoke `HirType::F64`
                    // timestamp, not a real registered class -- so a
                    // *dynamic* value from a real npm package (real
                    // trigger: a Fallback function whose return type
                    // couldn't be classified -- see `[[project_npm_
                    // interop_gaps_19]]`) needs a faithful runtime check
                    // instead of the compile-time class-identity check
                    // every other native class gets below. Checked via
                    // `peek_type_without_lowering` (not the ordinary
                    // `lower_expr` + `infer_expr_type` below) so this can
                    // dispatch before the value is lowered the ordinary
                    // way.
                    //
                    // Two cases, two different checks:
                    // - `JsValue` (an opaque live handle): dispatch into
                    //   QuickJS via `dynamic_value_check`, same as
                    //   `typeof`/`== null` already do.
                    // - `Json` (e.g. js-yaml's `load(): unknown`,
                    //   classified `Json` by the established "a bare
                    //   `any`/`unknown` return is real JSON data"
                    //   heuristic, see `dynamic_declarations.rs`): the
                    //   value has already been JSON-decoded by this
                    //   point, but a real `Date` survives that decode as
                    //   a structurally-recognizable `{"timestamp": N}`
                    //   shape (`Date.prototype.toJSON` is overridden
                    //   globally, `platform_globals/dates.js`) --
                    //   `__thaw_json_is_date_shape` recognizes it
                    //   directly, no live handle needed.
                    // Originally Date-only, gated on a fixed table of
                    // class names (the comment below was written for
                    // that version) -- the `JsValue` case is now fully
                    // general (`dynamic_value_check_by_name`, below):
                    // a live QuickJS handle checked against *any* named
                    // global constructor is safe and unambiguous to ask
                    // the engine about directly (`globalThis[name]`
                    // itself, on the JS side), so this no longer needs
                    // gating on a fixed set of known native shapes at
                    // all -- fixes `instanceof` for e.g. `ArrayBuffer`/
                    // `DataView`/`Headers`/every other exotic global
                    // this compiler has no dedicated native
                    // representation for, not just the ones with their
                    // own table entry below. The `Json`-typed branch
                    // still needs per-class shape checks (`Date`/
                    // `Array`/`Uint8Array`/the sentinel-tagged trio), so
                    // still dispatches through the class-name tables.
                    match self.peek_type_without_lowering(&bin.left) {
                            Some(HirType::JsValue) => {
                                let value = self.lower_expr_with_expected_type(
                                    &bin.left,
                                    Some(&HirType::JsValue),
                                )?;
                            return self.lower_live_handle_instanceof(value, class.sym.as_ref());
                            }
                            // `bin.left` is only ever lowered *inside* a
                            // branch that immediately returns -- `WeakMap`/
                            // `WeakSet` (no `Json` representation to check
                            // at all) fall all the way through to the
                            // ordinary path below instead, which lowers
                            // `bin.left` itself exactly once; lowering it
                            // here unconditionally and *then* falling
                            // through would lower it a second time there.
                            Some(HirType::Json) => {
                                if class.sym == *"Date" {
                                    let value = self.lower_expr(&bin.left)?;
                                    return Ok(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_is_date_shape".to_string())),
                                        vec![value],
                                    ));
                                }
                                // `Array.isArray` already has exactly this
                                // check (`static_builtins.rs`) -- a `Json`
                                // value is either genuinely a JSON array
                                // or it isn't, no sentinel tagging needed
                                // (unlike `RegExp`/`Map`/`Set`, an array
                                // has no ambiguity to disambiguate).
                                if class.sym == *"Array" {
                                    let value = self.lower_expr(&bin.left)?;
                                    return Ok(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_is_array".to_string())),
                                        vec![value],
                                    ));
                                }
                                // A `Buffer`/`Uint8Array` crossing into
                                // `any` is wrapped as `{"type":"Buffer",
                                // "data":[...]}`, matching real Node's
                                // own `Buffer.prototype.toJSON` -- a
                                // third bespoke shape check, alongside
                                // `Date`'s and `Array`'s just above,
                                // rather than a `native_builtin_
                                // instanceof_json_sentinel` entry, since
                                // that helper's single-key check doesn't
                                // fit this two-field shape.
                                if class.sym == *"Uint8Array" {
                                    // A real Buffer/Uint8Array crossing
                                    // into `any` via a *statically*-typed
                                    // (`HirType::Bytes`) source is caught
                                    // by the `__thaw_json_is_buffer_shape`
                                    // check alone -- but a live, JsValue-
                                    // constructed one (`new Uint8Array(...)`
                                    // with no static annotation, e.g. round12's
                                    // `f(new Uint8Array([1,2,3]))`) crosses
                                    // into `any` as the opaque `{"__thaw_
                                    // js_handle_id__": id}` placeholder
                                    // instead (see `coerce_to_declared`'s
                                    // `Json`-target/`JsValue`-actual branch,
                                    // `inference/coercions.rs` -- kept
                                    // unconditionally identity-preserving
                                    // there rather than eagerly resolved,
                                    // since a naive fix at *that* shared
                                    // choke point was tried and reverted:
                                    // it broke `Atomics.waitAsync` on a
                                    // live `SharedArrayBuffer`-backed
                                    // TypedArray, which needs the value to
                                    // stay genuinely live, not a snapshot
                                    // copy). Narrower fix, scoped to this
                                    // one `instanceof` check only: detect
                                    // the handle-placeholder shape (the
                                    // same `__thaw_json_has_wrapper_key`
                                    // structural check the sentinel-tagged
                                    // table below already uses) and, only
                                    // then, resolve the live handle back
                                    // (`coerce_to_declared(JsValue, ...)`,
                                    // the existing, already-used-elsewhere
                                    // "recover a JsValue from its Json
                                    // placeholder" path) and ask the live
                                    // engine directly via `dynamic_value_
                                    // check_by_name`, exactly as the
                                    // `Some(HirType::JsValue)` arm above
                                    // already does for a receiver that's
                                    // statically `JsValue`. Every other
                                    // shape (a genuine Buffer-shape object,
                                    // `null`, a plain array/object, ...)
                                    // still falls through to the original,
                                    // unconditional `__thaw_json_is_buffer_
                                    // shape` check, unaffected.
                                    let value = self.lower_expr(&bin.left)?;
                                    let temp = format!(
                                        "__thaw_instanceof_uint8array_source_{}",
                                        self.next_binding
                                    );
                                    self.next_binding += 1;
                                    self.scope.insert(temp.clone(), HirType::Json);
                                    let is_handle = HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_has_wrapper_key".to_string(),
                                        )),
                                        vec![
                                            HirExpr::Var(temp.clone()),
                                            HirExpr::Lit(HirLit::Str(
                                                "__thaw_js_handle_id__".to_string(),
                                            )),
                                        ],
                                    );
                                    let resolved = self.coerce_to_declared(
                                        &HirType::JsValue,
                                        HirExpr::Var(temp.clone()),
                                    )?;
                                    let live_check =
                                        self.dynamic_value_check_by_name("Uint8Array", resolved)?;
                                    let buffer_shape_check = HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_is_buffer_shape".to_string(),
                                        )),
                                        vec![HirExpr::Var(temp.clone())],
                                    );
                                    let body = HirExpr::Conditional(
                                        Box::new(is_handle),
                                        Box::new(live_check),
                                        Box::new(buffer_shape_check),
                                        HirType::Bool,
                                    );
                                    return self.wrap_call_argument_bindings(
                                        body,
                                        &[(temp, HirType::Json, value)],
                                    );
                                }
                                if let Some(key) =
                                    native_builtin_instanceof_json_sentinel(class.sym.as_ref())
                                {
                                    let value = self.lower_expr(&bin.left)?;
                                    return Ok(HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_has_wrapper_key".to_string(),
                                        )),
                                        vec![value, HirExpr::Lit(HirLit::Str(key.to_string()))],
                                    ));
                                }
                            }
                            _ => {}
                        }
                    let value = self.lower_expr(&bin.left)?;
                    let value_type = self.infer_expr_type(&value)?;
                    // `x instanceof C` on an optional / nullable value: absent is never an instance;
                    // a present one is tested through its payload.
                    if let HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) = &value_type {
                        if matches!(inner.as_ref(), HirType::Object(_) | HirType::Map(..) | HirType::Set(_) | HirType::Array(_)) {
                            let inner = inner.as_ref().clone();
                            let id = self.next_binding;
                            self.next_binding += 1;
                            let holder = format!("__thaw_instanceof_optional_{id}");
                            let payload = format!("__thaw_instanceof_payload_{id}");
                            self.scope.insert(holder.clone(), value_type.clone());
                            self.scope.insert(payload.clone(), inner.clone());
                            let held = Box::new(HirExpr::Var(holder.clone()));
                            let (absent, unwrapped) = match &value_type {
                                HirType::Optional(_) => (
                                    HirExpr::OptionalIsNone(held.clone(), inner.clone()),
                                    HirExpr::OptionalValue(held, inner.clone()),
                                ),
                                HirType::Nullable(_) => (
                                    HirExpr::NullableIsNone(held.clone(), inner.clone()),
                                    HirExpr::NullableValue(held, inner.clone()),
                                ),
                                _ => (
                                    HirExpr::NullishIsNone(held.clone(), inner.clone()),
                                    HirExpr::NullishValue(held, inner.clone()),
                                ),
                            };
                            let mut payload_test = bin.clone();
                            *payload_test.left = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(payload.as_str().into(), bin.span));
                            let test = self.lower_expr(&Expr::Bin(payload_test))?;
                            let present = HirExpr::Conditional(
                                Box::new(absent),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                Box::new(test),
                                HirType::Bool,
                            );
                            return self.wrap_call_argument_bindings(
                                present,
                                &[(holder, value_type, value), (payload, inner, unwrapped)],
                            );
                        }
                    }
                    // Every native object, array and collection is an `Object`; primitives are not.
                    if class.sym == *"Object" && self.is_unshadowed_class_identifier("Object") {
                        if let Some(result) = native_object_instanceof(&value_type) {
                            let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), value_type.clone());
                            return self.wrap_call_argument_bindings(
                                HirExpr::Lit(HirLit::Bool(result)),
                                &[(name, value_type, value)],
                            );
                        }
                    }
                    // `instanceof Uint8Array`/`instanceof Array` both need
                    // a *third* HirType -- `HirType::Bytes` (a real
                    // Buffer/Uint8Array) and `HirType::Array(F64)` (a
                    // plain `number[]`) share one native layout, so
                    // `infer_expr_type` (used for `value_type` above, and
                    // by every other check in this whole function)
                    // deliberately normalizes the former to the latter
                    // for every consumer *except* a dispatch that must
                    // actually tell them apart -- exactly these two (see
                    // `infer_expr_type`'s own doc comment). `infer_expr_
                    // type_inner` still sees the real, un-normalized tag.
                    // Without this, a genuine, statically-typed
                    // `Uint8Array` value wrongly matched `instanceof
                    // Array` too (`native_builtin_instanceof_static_
                    // match`'s own `"Array"` arm sees only the
                    // normalized `Array(F64)`) -- caught by testing this
                    // exact combination directly against Node, not by
                    // reasoning alone. Other typed-array constructors
                    // (`Int8Array`, `Float64Array`, ...) have no native
                    // `HirType` of their own at all -- confirmed via a
                    // direct probe that even naming one as a static
                    // parameter type fails outright ("generics are not
                    // supported yet") -- so `new Int8Array(...)` etc.
                    // can only ever be a live `JsValue` handle
                    // (`Expr::New`'s own generic `constructDynamicValue`
                    // path, above), handled by the ordinary dynamic-
                    // value dispatch below; only `Uint8Array`/`Array`
                    // need this dedicated native-shape check.
                    if matches!(class.sym.as_ref(), "Uint8Array" | "Array")
                        && self.infer_expr_type_inner(&value)? == HirType::Bytes
                    {
                        let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(class.sym == *"Uint8Array")),
                            &[(name, value_type.clone(), value)],
                        );
                    }
                    // A live `JsValue` left operand against a *decorated*
                    // local class (real trigger: class-transformer's
                    // `plainToInstance(User, plain)` result tested with
                    // `instanceof User`): the value is an opaque live
                    // object, so the compile-time layout comparison below
                    // can't decide it. The class has a real "class token"
                    // `JsValue` (the same object its decorators received),
                    // so ask the live engine instead.
                    if value_type == HirType::JsValue {
                        if let Some(token) = self.decorator_class_token(class.sym.as_ref()) {
                            return self.dynamic_value_instanceof(value, HirExpr::Var(token));
                        }
                    }
                    // An `any` holding a native class instance projected as a live
                    // host: ask the engine against the class's real counterpart.
                    if value_type == HirType::Json {
                        let dynamic_constructor = self
                            .decorator_class_token(class.sym.as_ref())
                            .map(HirExpr::Var)
                            .or_else(|| {
                                is_error_family_name(class.sym.as_ref()).then(|| {
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".into())),
                                        vec![HirExpr::Lit(HirLit::Str(class.sym.to_string()))],
                                    )
                                })
                            });
                        if let Some(constructor) = dynamic_constructor {
                            if is_error_family_name(class.sym.as_ref()) {
                                // A Json copy of a native Error answers from the Error itself.
                                let temp = format!("__thaw_instanceof_error_json_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temp.clone(), HirType::Json);
                                let native = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_is_error_instance".to_string())),
                                    vec![
                                        HirExpr::Var(temp.clone()),
                                        HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                                    ],
                                );
                                let host = self.lower_host_instanceof(
                                    HirExpr::Var(temp.clone()),
                                    HirType::Json,
                                    constructor,
                                    HirType::JsValue,
                                )?;
                                let body = HirExpr::Conditional(
                                    Box::new(native),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    Box::new(host),
                                    HirType::Bool,
                                );
                                return self.wrap_call_argument_bindings(
                                    body,
                                    &[(temp, HirType::Json, value)],
                                );
                            }
                            return self.lower_host_instanceof(
                                value,
                                value_type,
                                constructor,
                                HirType::JsValue,
                            );
                        }
                    }
                    let class_names = self.instanceof_class_names(class.sym.as_ref());
                    if let HirType::Union(elements) = &value_type {
                        let matching = elements
                            .iter()
                            .enumerate()
                            .filter_map(|(index, element)| {
                                let native_member = matches!(class.sym.as_ref(), "Array" | "Map" | "Set")
                                    && native_builtin_instanceof_static_match(class.sym.as_ref(), element)
                                        == Some(true);
                                (if class.sym == *"Array" || native_member {
                                    native_member
                                } else {
                                    // The caught-exception carrier's `Object([])` member
                                    // has an unknown layout; its identity is a runtime query.
                                    class_names
                                        .iter()
                                        .any(|name| class_type_has_identity(element, name))
                                        || (extends_error_family
                                            && matches!(element, HirType::Object(fields) if fields.is_empty()))
                                })
                                .then_some(index)
                            })
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        let all_arrays = class.sym == *"Array" && matching.len() == elements.len();
                        let mut result = HirExpr::Lit(HirLit::Bool(all_arrays));
                        if !all_arrays {
                            for index in matching {
                                let check = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(name.clone())),
                                        elements.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                );
                                // A marker-shaped union member may also be an ordinary
                                // object with the same physical fields. Test its actual
                                // ancestry only after the tag matches.
                                let native_member = matches!(class.sym.as_ref(), "Array" | "Map" | "Set")
                                    && native_builtin_instanceof_static_match(class.sym.as_ref(), &elements[index])
                                        == Some(true);
                                let check = if class.sym == *"Array" || native_member {
                                    check
                                } else {
                                    let projected = HirExpr::UnionValue(
                                        Box::new(HirExpr::Var(name.clone())),
                                        index,
                                        elements.clone(),
                                    );
                                    let identity = HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_object_has_class_identity".into(),
                                        )),
                                        vec![
                                            projected,
                                            HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                                        ],
                                    );
                                    self.lower_logical_expr(check, identity, true)?
                                };
                                result = self.lower_logical_expr(result, check, false)?;
                            }
                        }
                        // Caught QuickJS exceptions can retain their original
                        // object in the Json arm. Check the union tag before
                        // projecting it, then hand that authenticated value to
                        // the engine's real `instanceof` path. This preserves
                        // prototype identity and keeps ordinary Json/name
                        // lookalikes from matching.
                        if extends_error_family && !all_arrays {
                            if let Some(index) = elements
                                .iter()
                                .position(|element| *element == HirType::Json)
                            {
                                let check = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(name.clone())),
                                        elements.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                );
                                let projected = HirExpr::UnionValue(
                                    Box::new(HirExpr::Var(name.clone())),
                                    index,
                                    elements.clone(),
                                );
                                let class_name = self.coerce_to_declared(
                                    &HirType::Json,
                                    HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                                )?;
                                let arguments = self.wrap_native_value_as_json(
                                    HirExpr::ArrayLit(vec![class_name, projected]),
                                    HirType::Array(Box::new(HirType::Json)),
                                )?;
                                let identity = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("callDynamic".into())),
                                    vec![
                                        HirExpr::Lit(HirLit::Str(
                                            "__thaw_instanceof_dynamic_value_by_name".into(),
                                        )),
                                        arguments,
                                    ],
                                )));
                                let check = self.lower_logical_expr(check, identity, true)?;
                                result = self.lower_logical_expr(result, check, false)?;
                            }
                        }
                        // A thrown `Error` travels as a framed string; its
                        // ancestry is decoded from the frame (an unframed
                        // string is never an instance).
                        if extends_error_family && !all_arrays {
                            for (index, element) in elements.iter().enumerate() {
                                if *element != HirType::Str {
                                    continue;
                                }
                                let check = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(name.clone())),
                                        elements.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                );
                                let framed = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_error_is_instance".into())),
                                    vec![
                                        HirExpr::UnionValue(
                                            Box::new(HirExpr::Var(name.clone())),
                                            index,
                                            elements.clone(),
                                        ),
                                        HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                                    ],
                                );
                                let check = self.lower_logical_expr(check, framed, true)?;
                                result = self.lower_logical_expr(result, check, false)?;
                            }
                        }
                        return self
                            .wrap_call_argument_bindings(result, &[(name, value_type, value)]);
                    }
                    // Built-in Error objects and subclasses share a physical
                    // prefix, but subtype ancestry lives in the allocation
                    // identity table. A tagged-looking string is not proof.
                    if extends_error_family && matches!(value_type, HirType::Object(_)) {
                        let name = format!("__thaw_instanceof_error_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        let query = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_has_class_identity".into())),
                            vec![
                                HirExpr::Var(name.clone()),
                                HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                            ],
                        );
                        return self
                            .wrap_call_argument_bindings(query, &[(name, value_type, value)]);
                    }
                    if extends_error_family && value_type == HirType::Str {
                        let name = format!("__thaw_instanceof_text_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        // `new Error(..)`/`Error(..)` are framed strings; an unframed string is not an instance.
                        let query = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_is_instance".into())),
                            vec![
                                HirExpr::Var(name.clone()),
                                HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                            ],
                        );
                        return self.wrap_call_argument_bindings(query, &[(name, value_type, value)]);
                    }
                    // Fallback for the same dynamic-value case the early
                    // `peek_type_without_lowering` check above already
                    // handles for the common case (a property-access
                    // chain) -- this catches a `JsValue` receiver that
                    // peek couldn't determine without lowering (e.g.
                    // itself a call expression), now that `value` has
                    // already been lowered normally above. A `Json`-typed
                    // `value` here can't be retroactively coerced into a
                    // real handle (a JSON snapshot has already lost
                    // whatever live identity it had), so only `JsValue`
                    // is handled -- a `Json` value peek couldn't catch
                    // falls through to the static-match check below, same
                    // as any other unrecognized class. Generic over any
                    // class name, same as the early check above.
                    if value_type == HirType::JsValue {
                        return self.lower_live_handle_instanceof(value, class.sym.as_ref());
                    }
                    // `Date`/`RegExp`/`Map`/`Set`/`WeakMap`/`WeakSet`
                    // aren't registered in `self.signatures` (see
                    // `native_builtin_instanceof_static_match`'s own doc
                    // comment) -- recognized here as an alternative
                    // "known class" source, bypassing the gate below, so
                    // e.g. `re instanceof RegExp` for an ordinary,
                    // statically-typed `RegExp` doesn't hit "not a known
                    // class" the way it did for every one of these eight
                    // names before this fix (confirmed via a direct
                    // probe -- this affected the plain static case, not
                    // just the `any`/`JsValue` ones the checks above
                    // handle).
                    if class.sym == *"Date" && value_type == date_object_type() {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_has_native_identity".into())),
                            vec![value],
                        ));
                    }
                    let native_builtin_match =
                        native_builtin_instanceof_static_match(class.sym.as_ref(), &value_type);
                    let native_collection_subclass = native_base_type(&value_type).is_some()
                        && matches!(class.sym.as_ref(), "Map" | "Set" | "Array");
                    if !extends_error_family
                        && native_builtin_match.is_none()
                        && !native_collection_subclass
                        && !class_names.iter().any(|name| {
                            self.signatures.contains_key(&class_constructor_symbol(name))
                        })
                    {
                        return Err(format!(
                            "native `instanceof` right operand `{}` is not a known class",
                            class.sym
                        ));
                    }
                    let result = native_builtin_match.unwrap_or_else(|| {
                        class_names
                            .iter()
                            .any(|name| class_type_has_identity(&value_type, name))
                    });
                    // The static type may name only the base (a subclass instance
                    // is passed as its base), so a class-identity value always asks
                    // the allocation identity table, which includes inherited classes.
                    let class_identity_value = matches!(&value_type, HirType::Object(fields)
                        if fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")));
                    if native_builtin_match.is_none() && (result || class_identity_value) {
                        // A physical marker slot is not proof of nominal class
                        // ancestry. The native table is keyed by this object's
                        // allocation identity and includes inherited classes.
                        let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        // A generic class is one monomorphised class per instantiation.
                        let mut query = None;
                        for class_name in &class_names {
                            let check = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_has_class_identity".into())),
                                vec![
                                    HirExpr::Var(name.clone()),
                                    HirExpr::Lit(HirLit::Str(class_name.clone())),
                                ],
                            );
                            query = Some(match query {
                                None => check,
                                Some(previous) => self.lower_logical_expr(previous, check, false)?,
                            });
                        }
                        let query = query.expect("an instanceof target names at least itself");
                        return self
                            .wrap_call_argument_bindings(query, &[(name, value_type, value)]);
                    }
                    let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), value_type.clone());
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(result)),
                        &[(name, value_type, value)],
                    );
                }
                let left_versions = self.narrowing_write_versions.clone();
                let observes_undefined = bin.op == BinaryOp::NullishCoalescing
                    || (matches!(
                        bin.op,
                        BinaryOp::EqEq | BinaryOp::EqEqEq | BinaryOp::NotEq | BinaryOp::NotEqEq
                    ) && matches!(bin.right.as_ref(), Expr::Ident(id) if id.sym == *"undefined"));
                let string_index = if observes_undefined {
                    self.lower_string_index_optional(&bin.left)?
                } else {
                    None
                };
                let mut lhs = match string_index {
                    Some(read) => read,
                    None => self.lower_expr(&bin.left)?,
                };
                let rhs_narrowing = self
                    .optional_undefined_narrowing(&bin.left)
                    .filter(|(name, _, _, _)| {
                        left_versions.get(name) == self.narrowing_write_versions.get(name)
                    })
                    .filter(|(_, _, present, _)| {
                        (bin.op == BinaryOp::LogicalAnd && *present)
                            || (bin.op == BinaryOp::LogicalOr && !*present)
                    })
                    .map(|(name, payload, _, nullable)| (name, payload, nullable));
                let rhs_union_narrowing =
                    self.union_narrowing(&bin.left)
                        .and_then(|(targets, equal, complement)| {
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
                        });
                let mut rhs = self.lower_expr_with_union_narrowing(
                        &bin.right,
                        rhs_union_narrowing.as_deref(),
                        rhs_narrowing.as_ref(),
                    )?;
                if matches!(
                    bin.op,
                    BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::Exp
                        | BinaryOp::Lt
                        | BinaryOp::Gt
                        | BinaryOp::LtEq
                        | BinaryOp::GtEq
                        | BinaryOp::EqEqEq
                        | BinaryOp::NotEqEq
                        | BinaryOp::EqEq
                        | BinaryOp::NotEq
                ) {
                    lhs = self.lower_primitive_array_operand(lhs)?;
                    rhs = self.lower_primitive_array_operand(rhs)?;
                } else if bin.op == BinaryOp::NullishCoalescing {
                    // `arr[i] ?? d`: an out-of-range or null slot is absent.
                    lhs = self.lower_primitive_array_operand(lhs)?;
                }
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
                if let Some(result) = self.lower_bigint_arithmetic(&lhs, &rhs, bin.op)? {
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                let value = match bin.op {
                    BinaryOp::In => {
                        let mut right_type = self.infer_expr_type(&rhs)?;
                        if let HirType::Union(members) = &right_type {
                            if members
                                .iter()
                                .all(|member| matches!(member, HirType::Array(_)))
                            {
                                (rhs, right_type) =
                                    self.lower_union_array_sequence(rhs, members)?;
                            }
                        }
                        if matches!(right_type, HirType::JsValue | HirType::Dynamic) {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("hasDynamicProperty".into())),
                                vec![rhs, lhs],
                            )
                        } else if matches!(right_type, HirType::Json | HirType::Dictionary(_)) {
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            let left_name = format!("__thaw_in_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(left_name.clone(), HirType::Str);
                            let right_name = format!("__thaw_in_object_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(right_name.clone(), right_type.clone());
                            let has = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_has".into())),
                                vec![
                                    HirExpr::Var(right_name.clone()),
                                    HirExpr::Var(left_name.clone()),
                                ],
                            );
                            // Node throws a TypeError for `in` on a primitive operand (number,
                            // string, boolean, null, undefined), so only objects reach the lookup.
                            let is_primitive = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_primitive_operand".into())),
                                vec![HirExpr::Var(right_name.clone())],
                            );
                            let concat = |left, right| HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".into())), vec![left, right]);
                            let message = concat(
                                concat(
                                    HirExpr::Lit(HirLit::Str(
                                        "\u{1}TypeError\u{1}Cannot use 'in' operator to search for '".into(),
                                    )),
                                    HirExpr::Var(left_name.clone()),
                                ),
                                concat(
                                    HirExpr::Lit(HirLit::Str("' in ".into())),
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_console_inspect_text".into())),
                                        vec![HirExpr::Var(right_name.clone())],
                                    ),
                                ),
                            );
                            let guarded = HirExpr::Conditional(
                                Box::new(is_primitive),
                                Box::new(HirExpr::ThrowValue(
                                    Box::new(message),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                )),
                                Box::new(has),
                                HirType::Bool,
                            );
                            self.wrap_call_argument_bindings(
                                guarded,
                                &[
                                    (left_name, HirType::Str, lhs),
                                    (right_name, right_type, rhs),
                                ],
                            )?
                        } else if right_type == HirType::Str {
                            // `"code" in err`: own properties of an Error value (runtime side table).
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_error_has_prop".into())),
                                vec![rhs, lhs],
                            )
                        } else if matches!(&right_type, HirType::Array(_)) {
                            let mut key_type = self.infer_expr_type(&lhs)?;
                            if matches!(key_type, HirType::Bool | HirType::I64) {
                                lhs = self.coerce_primitive_to_string(lhs)?;
                                key_type = HirType::Str;
                            }
                            if matches!(key_type, HirType::Null | HirType::Undefined) {
                                let key_name = format!("__thaw_in_array_key_{}", self.next_binding);
                                self.next_binding += 1;
                                let array_name = format!("__thaw_in_array_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(key_name.clone(), key_type.clone());
                                self.scope.insert(array_name.clone(), right_type.clone());
                                return self.wrap_call_argument_bindings(
                                    HirExpr::Lit(HirLit::Bool(false)),
                                    &[(key_name, key_type, lhs), (array_name, right_type, rhs)],
                                );
                            }
                            if !matches!(key_type, HirType::F64 | HirType::Str | HirType::Symbol) {
                                return Err("`in` on a native array requires a numeric, string, or symbol key".into());
                            }
                            let key_name = format!("__thaw_in_array_key_{}", self.next_binding);
                            self.next_binding += 1;
                            let array_name = format!("__thaw_in_array_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(key_name.clone(), key_type.clone());
                            self.scope.insert(array_name.clone(), right_type.clone());
                            let key = HirExpr::Var(key_name.clone());
                            let array = HirExpr::Var(array_name.clone());
                            let result = if matches!(key_type, HirType::Str | HirType::Symbol) {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_has_property".into())),
                                    vec![array, key],
                                )
                            } else {
                                let valid = self.lower_logical_expr(
                                    HirExpr::BinOp(
                                        BinOp::GtEq,
                                        Box::new(key.clone()),
                                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                    ),
                                    HirExpr::BinOp(
                                        BinOp::Lt,
                                        Box::new(key.clone()),
                                        Box::new(HirExpr::ArrayLen(Box::new(array.clone()))),
                                    ),
                                    true,
                                )?;
                                let valid = self.lower_logical_expr(
                                    valid,
                                    HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(key.clone()),
                                        Box::new(HirExpr::Call(
                                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                                            vec![key.clone()],
                                        )),
                                    ),
                                    true,
                                )?;
                                self.lower_logical_expr(
                                    valid,
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_array_has_index".into())),
                                        vec![array, key],
                                    ),
                                    true,
                                )?
                            };
                            self.wrap_call_argument_bindings(
                                result,
                                &[(key_name, key_type, lhs), (array_name, right_type, rhs)],
                            )?
                        } else if right_type == crate::caught_exception_carrier_type() {
                            // Evaluate both operands before ToPropertyKey, and keep framed Errors
                            // on their original side-table text so deletes remain visible.
                            let HirType::Union(members) = &right_type else { unreachable!() };
                            let key_type = self.infer_expr_type(&lhs)?;
                            let key_value_name = format!("__thaw_in_carrier_key_value_{}", self.next_binding);
                            self.next_binding += 1;
                            let carrier_name = format!("__thaw_in_carrier_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(key_value_name.clone(), key_type.clone());
                            self.scope.insert(carrier_name.clone(), right_type.clone());
                            let carrier = HirExpr::Var(carrier_name.clone());
                            let text = HirExpr::UnionValue(
                                Box::new(carrier.clone()),
                                3,
                                members.clone(),
                            );
                            let is_text = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(carrier.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(3.0))),
                            );
                            let is_error_text = HirExpr::Conditional(
                                Box::new(is_text),
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_error_is_error".into())),
                                    vec![text.clone()],
                                )),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                HirType::Bool,
                            );
                            let error_key = self.coerce_primitive_to_string(HirExpr::Var(key_value_name.clone()))?;
                            let error_key_name = format!("__thaw_in_error_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(error_key_name.clone(), HirType::Str);
                            let error_has = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_error_has_prop".into())),
                                vec![text, HirExpr::Var(error_key_name.clone())],
                            );
                            let inherited = [
                                "name",
                                "constructor",
                                "__defineGetter__",
                                "__defineSetter__",
                                "hasOwnProperty",
                                "__lookupGetter__",
                                "__lookupSetter__",
                                "isPrototypeOf",
                                "propertyIsEnumerable",
                                "toLocaleString",
                                "toString",
                                "valueOf",
                                "__proto__",
                            ]
                            .into_iter()
                            .fold(HirExpr::Lit(HirLit::Bool(false)), |has, name| {
                                let matches = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Var(error_key_name.clone())),
                                    Box::new(HirExpr::Lit(HirLit::Str(name.into()))),
                                );
                                HirExpr::Conditional(
                                    Box::new(has),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    Box::new(matches),
                                    HirType::Bool,
                                )
                            });
                            let error_has = HirExpr::Conditional(
                                Box::new(inherited),
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                Box::new(error_has),
                                HirType::Bool,
                            );
                            let error_has = self.wrap_call_argument_bindings(
                                error_has,
                                &[(error_key_name, HirType::Str, error_key)],
                            )?;

                            let json_name = format!("__thaw_in_carrier_json_{}", self.next_binding);
                            self.next_binding += 1;
                            let json_key_name = format!("__thaw_in_json_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(json_name.clone(), HirType::Json);
                            self.scope.insert(json_key_name.clone(), HirType::Str);
                            let json = self.caught_carrier_as_json(carrier)?;
                            let is_primitive = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_primitive_operand".into())),
                                vec![HirExpr::Var(json_name.clone())],
                            );
                            let json_key = self.coerce_primitive_to_string(HirExpr::Var(key_value_name.clone()))?;
                            let json_has = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_has".into())),
                                vec![
                                    HirExpr::Var(json_name.clone()),
                                    HirExpr::Var(json_key_name.clone()),
                                ],
                            );
                            let json_has = self.wrap_call_argument_bindings(
                                json_has,
                                &[(json_key_name, HirType::Str, json_key)],
                            )?;
                            let type_error = HirExpr::ThrowValue(
                                Box::new(HirExpr::Lit(HirLit::Str(
                                    "\u{1}TypeError\u{1}Cannot use 'in' operator with a primitive value".into(),
                                ))),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                            );
                            let json_result = HirExpr::Conditional(
                                Box::new(is_primitive),
                                Box::new(type_error),
                                Box::new(json_has),
                                HirType::Bool,
                            );
                            let json_result = self.wrap_call_argument_bindings(
                                json_result,
                                &[(json_name, HirType::Json, json)],
                            )?;
                            let result = HirExpr::Conditional(
                                Box::new(is_error_text),
                                Box::new(error_has),
                                Box::new(json_result),
                                HirType::Bool,
                            );
                            self.wrap_call_argument_bindings(
                                result,
                                &[
                                    (key_value_name, key_type, lhs),
                                    (carrier_name, right_type, rhs),
                                ],
                            )?
                        } else if let HirType::Union(elements) = &right_type {
                            let HirExpr::Lit(HirLit::Str(property)) = lhs else {
                                return Err("`in` on a union requires a string literal key".into());
                            };
                            let matching = elements
                                .iter()
                                .enumerate()
                                .filter_map(|(index, element)| match element {
                                    HirType::Object(fields)
                                        if fields.iter().any(|(name, _)| name == &property) =>
                                    {
                                        Some(index)
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>();
                            let name = format!("__thaw_in_union_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), right_type.clone());
                            let mut result = HirExpr::Lit(HirLit::Bool(false));
                            for index in matching {
                                let check = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(name.clone())),
                                        elements.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                );
                                result = self.lower_logical_expr(result, check, false)?;
                            }
                            // Node throws a TypeError for `in` on a primitive right-hand side,
                            // so each primitive member's tag throws instead of answering false.
                            for (index, element) in elements.iter().enumerate().rev() {
                                if !matches!(
                                    element,
                                    HirType::F64 | HirType::I64 | HirType::Bool | HirType::Str
                                        | HirType::StrLiteral(_) | HirType::Symbol
                                        | HirType::Null | HirType::Undefined | HirType::Void
                                ) {
                                    continue;
                                }
                                let member = HirExpr::UnionValue(
                                    Box::new(HirExpr::Var(name.clone())), index, elements.clone(),
                                );
                                let text = self.rejection_display_without_user_coercion(member, element)?;
                                let message = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".into())),
                                    vec![
                                        HirExpr::Lit(HirLit::Str(format!(
                                            "\u{1}TypeError\u{1}Cannot use 'in' operator to search for '{property}' in "
                                        ))),
                                        text,
                                    ],
                                );
                                let throw = HirExpr::ThrowValue(
                                    Box::new(message),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                );
                                let condition = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(name.clone())),
                                        elements.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                );
                                result = HirExpr::Conditional(
                                    Box::new(condition), Box::new(throw), Box::new(result), HirType::Bool,
                                );
                            }
                            self.wrap_call_argument_bindings(result, &[(name, right_type, rhs)])?
                        } else {
                            let HirType::Object(fields) = right_type else {
                                return Err("`in` currently requires a fixed-shape object".into());
                            };
                            let lhs = self.coerce_primitive_to_string(lhs)?;
                            // Keys every ordinary object inherits from
                            // `Object.prototype`: `"toString" in {a: 1}` is true.
                            const OBJECT_PROTOTYPE_KEYS: [&str; 12] = [
                                "__proto__",
                                "constructor",
                                "hasOwnProperty",
                                "isPrototypeOf",
                                "propertyIsEnumerable",
                                "toLocaleString",
                                "toString",
                                "valueOf",
                                "__defineGetter__",
                                "__defineSetter__",
                                "__lookupGetter__",
                                "__lookupSetter__",
                            ];
                            let field_names = fields
                                .iter()
                                .map(|(field, _)| field.clone())
                                .chain(OBJECT_PROTOTYPE_KEYS.iter().map(|key| key.to_string()))
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
                        self.lower_logical_expr(lhs, rhs, bin.op == BinaryOp::LogicalAnd)?
                    }
                    BinaryOp::NullishCoalescing => self.lower_nullish_coalescing(lhs, rhs)?,
                    BinaryOp::Lt => self.lower_relational(lhs, rhs, BinOp::Lt)?,
                    BinaryOp::Gt => self.lower_relational(lhs, rhs, BinOp::Gt)?,
                    BinaryOp::LtEq => self.lower_relational(lhs, rhs, BinOp::LtEq)?,
                    BinaryOp::GtEq => self.lower_relational(lhs, rhs, BinOp::GtEq)?,
                    BinaryOp::EqEqEq => self.lower_strict_equality(lhs, rhs)?,
                    BinaryOp::NotEqEq => {
                        let equality = self.lower_strict_equality(lhs, rhs)?;
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
                        if matches!(
                            self.infer_expr_type(&lhs)?,
                            HirType::Str | HirType::StrLiteral(_)
                        ) || matches!(
                            self.infer_expr_type(&rhs)?,
                            HirType::Str | HirType::StrLiteral(_)
                        ) =>
                    {
                        // A guaranteed string result keeps its existing Str
                        // contract, but both raw operands must be evaluated
                        // before either ToPrimitive call. A live handle uses
                        // the JS realm's default-hint `+` operator.
                        let live = self.infer_expr_type(&lhs)? == HirType::JsValue
                            || self.infer_expr_type(&rhs)? == HirType::JsValue;
                        let json_side = Self::add_operand_is_json(&self.infer_expr_type(&lhs)?)
                            || Self::add_operand_is_json(&self.infer_expr_type(&rhs)?);
                        if live {
                            let added = self.lower_add_with_live_json(lhs, rhs)?;
                            self.coerce_to_declared(&HirType::Str, added)?
                        } else if json_side {
                            self.lower_add_json_guarded(lhs, rhs, true)?
                        } else {
                            self.lower_add_with_to_primitive(lhs, rhs)?
                        }
                    }
                    BinaryOp::Add
                        if Self::add_operand_may_be_string(&self.infer_expr_type(&lhs)?)
                            || Self::add_operand_may_be_string(&self.infer_expr_type(&rhs)?) =>
                    {
                        let live = self.infer_expr_type(&lhs)? == HirType::JsValue
                            || self.infer_expr_type(&rhs)? == HirType::JsValue;
                        if live {
                            self.lower_add_with_live_json(lhs, rhs)?
                        } else {
                            self.lower_add_with_to_primitive(lhs, rhs)?
                        }
                    }
                    BinaryOp::Add
                        if (self.infer_expr_type(&lhs)? == HirType::JsValue
                            && (self.infer_expr_type(&rhs)? == HirType::Json
                                || self.infer_expr_type(&rhs)?
                                    == HirType::Optional(Box::new(HirType::Json))))
                            || (self.infer_expr_type(&rhs)? == HirType::JsValue
                                && (self.infer_expr_type(&lhs)? == HirType::Json
                                    || self.infer_expr_type(&lhs)?
                                        == HirType::Optional(Box::new(HirType::Json)))) =>
                    {
                        self.lower_add_with_live_json(lhs, rhs)?
                    }
                    BinaryOp::Add
                        if self.infer_expr_type(&lhs)? != HirType::JsValue
                            && self.infer_expr_type(&rhs)? != HirType::JsValue
                            && (matches!(self.infer_expr_type(&lhs)?, HirType::Json)
                                || self.infer_expr_type(&lhs)?
                                    == HirType::Optional(Box::new(HirType::Json))
                                || matches!(self.infer_expr_type(&rhs)?, HirType::Json)
                                || self.infer_expr_type(&rhs)?
                                    == HirType::Optional(Box::new(HirType::Json))) =>
                    {
                        // A Json operand may be a live host handle: JS picks the
                        // default-hint ToPrimitive, not the string/number hint.
                        self.lower_add_json_guarded(lhs, rhs, false)?
                    }
                    other
                        if (matches!(
                            self.infer_expr_type(&lhs)?,
                            HirType::JsValue | HirType::Json
                        ) && self.infer_expr_type(&rhs)? == HirType::F64)
                            || (self.infer_expr_type(&lhs)? == HirType::F64
                                && matches!(
                                    self.infer_expr_type(&rhs)?,
                                    HirType::JsValue | HirType::Json
                                )) =>
                    {
                        HirExpr::BinOp(
                            lower_bin_op(other)?,
                            Box::new(self.coerce_primitive_to_number(lhs)?),
                            Box::new(self.coerce_primitive_to_number(rhs)?),
                        )
                    }
                    BinaryOp::Add
                        if self.infer_expr_type(&lhs)? == HirType::JsValue
                            && self.infer_expr_type(&rhs)? == HirType::JsValue =>
                    {
                        // Both operands are live handles: let JS itself pick
                        // between numeric addition, string concatenation, and
                        // BigInt addition (which a compiled `String(x) +
                        // String(y)` would get wrong for a BigInt).
                        let callable = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("__thaw_dynamic_add".into()))],
                        );
                        let empty_arguments = self.wrap_native_value_as_json(
                            HirExpr::ArrayLit(Vec::new()),
                            HirType::Array(Box::new(HirType::Json)),
                        )?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())),
                            vec![callable, empty_arguments, HirExpr::ArrayLit(vec![lhs, rhs])],
                        )
                    }
                    BinaryOp::Add
                        if matches!(
                            self.infer_expr_type(&lhs)?,
                            HirType::Json | HirType::JsValue
                        ) || matches!(
                            self.infer_expr_type(&rhs)?,
                            HirType::Json | HirType::JsValue
                        ) =>
                    {
                        // `+` on a dynamic value a static checker can't
                        // prove numeric (the numeric `Json/JsValue + F64`
                        // case is handled just above). JS would
                        // `ToPrimitive` then add-or-concat; for the
                        // packages this reaches -- terminal-colour helpers
                        // and the like, whose functions return strings --
                        // it is always a string join, so stringify both
                        // sides and concat, the same `String(x) +
                        // String(y)` fallback a `.d.ts` author would reach
                        // for. Previously this errored (`arithmetic
                        // requires F64 operands`).
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![
                                self.coerce_primitive_to_string(lhs)?,
                                self.coerce_primitive_to_string(rhs)?,
                            ],
                        )
                    }
                    other
                        if matches!(self.infer_expr_type(&lhs)?, HirType::Optional(_))
                            || matches!(self.infer_expr_type(&rhs)?, HirType::Optional(_))
                            || matches!(
                                self.infer_expr_type(&lhs)?,
                                HirType::Nullable(_) | HirType::Nullish(_)
                            )
                            || matches!(
                                self.infer_expr_type(&rhs)?,
                                HirType::Nullable(_) | HirType::Nullish(_)
                            ) =>
                    {
                        HirExpr::BinOp(
                            lower_bin_op(other)?,
                            Box::new(self.coerce_primitive_to_number(lhs)?),
                            Box::new(self.coerce_primitive_to_number(rhs)?),
                        )
                    }
                    BinaryOp::Add
                        if matches!(self.infer_expr_type(&lhs)?, HirType::Object(_))
                            || matches!(self.infer_expr_type(&rhs)?, HirType::Object(_)) =>
                    {
                        // Neither operand is statically `Str` (the first
                        // `Add` arm above already owns that), so real JS's
                        // `+` genuinely needs to call `ToPrimitive` on the
                        // `Object`-typed side(s) and dispatch between
                        // string-concat and numeric-add from its *runtime*
                        // result -- see `lower_add_with_to_primitive`.
                        self.lower_add_with_to_primitive(lhs, rhs)?
                    }
                    other => HirExpr::BinOp(lower_bin_op(other)?, Box::new(lhs), Box::new(rhs)),
                };
                self.infer_expr_type(&value)?;
                self.wrap_call_argument_bindings(value, &bindings)
            }

            Expr::Unary(unary) => {
                if unary.op == UnaryOp::TypeOf {
                    if let Expr::Ident(identifier) = unary.arg.as_ref() {
                        let name = self.resolve_binding(identifier.sym.as_ref());
                        if self.deferred_generic_binds.contains_key(&name) {
                            return Ok(HirExpr::Lit(HirLit::Str("function".into())));
                        }
                        if self.generic_callable_template(&name).is_some() {
                            return Ok(HirExpr::Lit(HirLit::Str("function".into())));
                        }
                    }
                }
                // Implicit-return normalization inserts a dummy-span `void 0`
                // to avoid resolving a user binding named `undefined`. Only
                // that synthetic no-effect form is an Undefined HIR value;
                // ordinary unary-void expressions retain their current ABI.
                if unary.op == UnaryOp::Void
                    && unary.span == swc_common::DUMMY_SP
                    && matches!(unary.arg.as_ref(), Expr::Lit(Lit::Num(number))
                        if number.span == swc_common::DUMMY_SP && number.value == 0.0)
                {
                    return Ok(HirExpr::Lit(HirLit::Undefined));
                }
                if unary.op == UnaryOp::Delete {
                    // `delete o?.a` / `delete o?.["k"]`: `true` without
                    // touching anything when the receiver is nullish.
                    if let Expr::OptChain(chain) = unary.arg.as_ref() {
                        if let OptChainBase::Member(member) = chain.base.as_ref() {
                            let receiver = self.lower_expr(&member.obj)?;
                            let receiver_type = self.infer_expr_type(&receiver)?;
                            let name = format!("__thaw_delete_receiver_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), receiver_type.clone());
                            let bound = Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                name.clone().into(),
                                member.span,
                            )));
                            let mut rebound = member.clone();
                            rebound.obj = bound.clone();
                            let mut deletion = unary.clone();
                            *deletion.arg = Expr::Member(rebound);
                            let deleted = self.lower_expr(&Expr::Unary(deletion))?;
                            let is_nullish = self.lower_expr(&Expr::Bin(swc_ecma_ast::BinExpr {
                                span: member.span,
                                op: BinaryOp::EqEq,
                                left: bound,
                                right: Box::new(Expr::Lit(Lit::Null(swc_ecma_ast::Null {
                                    span: member.span,
                                }))),
                            }))?;
                            let result = HirExpr::Conditional(
                                Box::new(is_nullish),
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                Box::new(deleted),
                                HirType::Bool,
                            );
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(name, receiver_type, receiver)],
                            );
                        }
                    }
                    let Expr::Member(member) = unary.arg.as_ref() else {
                        return Err("native `delete` requires a JSON or dictionary property".into());
                    };
                    let object = self.lower_expr(&member.obj)?;
                    let object_type = self.infer_expr_type(&object)?;
                    if matches!(object_type, HirType::JsValue | HirType::Dynamic) {
                        let key = match &member.prop {
                            MemberProp::Ident(property) => {
                                HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                            }
                            MemberProp::Computed(computed) => self.lower_expr(&computed.expr)?,
                            MemberProp::PrivateName(_) => {
                                return Err(
                                    "native `delete` does not support private properties".into()
                                );
                            }
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("deleteDynamicProperty".into())),
                            vec![object, key],
                        ));
                    }
                    let array_union = match &object_type {
                        HirType::Union(members)
                            if members
                                .iter()
                                .all(|member| matches!(member, HirType::Array(_))) =>
                        {
                            Some(members.clone())
                        }
                        _ => None,
                    };
                    let (object, object_type) = if let Some(members) = array_union {
                        self.lower_union_array_sequence(object, &members)?
                    } else {
                        (object, object_type)
                    };
                    if matches!(object_type, HirType::Array(_)) {
                        let key = match &member.prop {
                            MemberProp::Ident(property) => {
                                HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                            }
                            MemberProp::Computed(computed) => {
                                let key = self.lower_expr(&computed.expr)?;
                                self.coerce_primitive_to_string(key)?
                            }
                            MemberProp::PrivateName(_) => {
                                return Err(
                                    "native `delete` does not support private properties".into()
                                );
                            }
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_delete_strict".into())),
                            vec![object, key],
                        ));
                    }
                    if object_type == HirType::Str {
                        // `delete err.code`: own properties of an Error value (runtime side table).
                        let key = match &member.prop {
                            MemberProp::Ident(property) => {
                                HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                            }
                            MemberProp::Computed(computed) => {
                                let key = self.lower_expr(&computed.expr)?;
                                self.coerce_primitive_to_string(key)?
                            }
                            MemberProp::PrivateName(_) => {
                                return Err(
                                    "native `delete` does not support private properties".into()
                                );
                            }
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_delete_prop".into())),
                            vec![object, key],
                        ));
                    }
                    if let (HirType::Union(members), true) = (
                        &object_type,
                        object_type == crate::caught_exception_carrier_type(),
                    ) {
                        // `delete e.k` on a catch binding: a framed error deletes from its side
                        // table, any other thrown value from its Json view.
                        let key = match &member.prop {
                            MemberProp::Ident(property) => {
                                HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                            }
                            MemberProp::Computed(computed) => {
                                let key = self.lower_expr(&computed.expr)?;
                                self.coerce_primitive_to_string(key)?
                            }
                            MemberProp::PrivateName(_) => {
                                return Err(
                                    "native `delete` does not support private properties".into()
                                );
                            }
                        };
                        let name = format!("__thaw_delete_carrier_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), object_type.clone());
                        let carrier = HirExpr::Var(name.clone());
                        let is_text = HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(carrier.clone()), members.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(3.0))),
                        );
                        let from_text = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_delete_prop".into())),
                            vec![
                                HirExpr::UnionValue(Box::new(carrier.clone()), 3, members.clone()),
                                key.clone(),
                            ],
                        );
                        let json = self.caught_carrier_as_json(carrier)?;
                        let from_json = HirExpr::JsonDelete(Box::new(json), Box::new(key));
                        let result = HirExpr::Conditional(
                            Box::new(is_text),
                            Box::new(from_text),
                            Box::new(from_json),
                            HirType::Bool,
                        );
                        return self.wrap_call_argument_bindings(result, &[(name, object_type, object)]);
                    }
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
                            return Err(
                                "native `delete` does not support private properties".into()
                            );
                        }
                    };
                    return Ok(HirExpr::JsonDelete(Box::new(object), Box::new(key)));
                }
                let string_index = if unary.op == UnaryOp::TypeOf {
                    self.lower_string_index_optional(&unary.arg)?
                } else {
                    None
                };
                let value = match string_index {
                    Some(read) => read,
                    None => self.lower_expr(&unary.arg)?,
                };
                let value = if matches!(
                    unary.op,
                    UnaryOp::TypeOf | UnaryOp::Bang | UnaryOp::Plus | UnaryOp::Minus
                ) {
                    self.lower_primitive_array_operand(value)?
                } else {
                    value
                };
                let lowered = match unary.op {
                    UnaryOp::Minus if self.infer_expr_type(&value)? == HirType::I64 => {
                        // No dedicated native negation for `I64` -- `0n -
                        // x` reuses the existing `bigint_sub` codegen
                        // (two's-complement wraps exactly the way real
                        // JS `BigInt` negation of `i64::MIN` itself
                        // would overflow anyway, so this stays within
                        // Thaw's already-documented fixed-64-bit-range
                        // subset).
                        HirExpr::BinOp(
                            BinOp::Sub,
                            Box::new(HirExpr::Lit(HirLit::I64(0))),
                            Box::new(value),
                        )
                    }
                    UnaryOp::Minus if self.infer_expr_type(&value)? == HirType::JsValue => {
                        // A beyond-`i64` bigint handle: `0 - x` on decimal
                        // digits, wrapped back into a real BigInt.
                        let digits = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_bigint_decimal_sub".into())),
                            vec![
                                HirExpr::Lit(HirLit::Str("0".to_string())),
                                self.bigint_decimal_string(value, &HirType::JsValue)?,
                            ],
                        );
                        let constructor = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("BigInt".into()))],
                        );
                        let arguments = self
                            .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![digits]))?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
                            vec![constructor, arguments],
                        )
                    }
                    UnaryOp::Minus => {
                        let value = if matches!(
                            self.infer_expr_type(&value)?,
                            HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)
                        ) {
                            self.coerce_primitive_to_number(value)?
                        } else {
                            self.expect_type(&HirType::F64, &value, "unary minus")?;
                            value
                        };
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_neg".to_string())),
                            vec![value],
                        )
                    }
                    UnaryOp::Plus => {
                        let value_type = self.infer_expr_type(&value)?;
                        if matches!(
                            value_type,
                            HirType::Optional(_)
                                | HirType::Nullable(_)
                                | HirType::Nullish(_)
                                | HirType::Object(_)
                                | HirType::Str
                                | HirType::StrLiteral(_)
                                | HirType::Bool
                        ) {
                            self.coerce_primitive_to_number(value)?
                        } else if matches!(value_type, HirType::Json | HirType::JsValue) {
                            self.dynamic_value_to_number(value, "__thaw_unary_plus")?
                        } else {
                            self.expect_type(&HirType::F64, &value, "unary plus")?;
                            value
                        }
                    }
                    UnaryOp::Bang => {
                        // Same truthiness coercion `if`/`while`/`do`/`for`
                        // conditions and the ternary's own test now get
                        // (`lower_condition_expr`) -- `!x` is just as
                        // valid for any value in real JS as those are,
                        // not only a literal `boolean`.
                        let ty = self.infer_expr_type(&value)?;
                        let value = self.truthiness_expr(value, &ty)?;
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )
                    }
                    UnaryOp::Tilde => {
                        if self.infer_expr_type(&value)? == HirType::I64 {
                            // `~n` is `-n - 1n`, i.e. `n ^ -1n`.
                            return Ok(HirExpr::BinOp(
                                BinOp::BitXor,
                                Box::new(value),
                                Box::new(HirExpr::Lit(HirLit::I64(-1))),
                            ));
                        }
                        self.expect_type(&HirType::F64, &value, "bitwise not")?;
                        HirExpr::BinOp(
                            BinOp::BitXor,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )
                    }
                    UnaryOp::TypeOf => {
                        // `typeof e` for a `catch (e)` binding reports
                        // `"object"`: real JavaScript throws an `Error`
                        // object. Internally the binding is the tagged
                        // error *string* (so `.name`/`.message`/`.code`/
                        // `(e as X)` keep working unchanged), so the
                        // generic `Str -> "string"` inference below would
                        // otherwise be wrong here.
                        if let HirExpr::Var(name) = &value {
                            let promise_catch = self.promise_catch_bindings.contains(name)
                                || self.promise_catch_parameter.as_deref().is_some_and(
                                    |parameter| self.resolve_binding(parameter) == *name,
                                );
                            if (self.catch_bindings.contains(name)
                                && self.scope.get(name)
                                    != Some(&crate::caught_exception_carrier_type()))
                                || promise_catch
                            {
                                let tag = if promise_catch {
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_pending_exception_tag".to_string(),
                                        )),
                                        Vec::new(),
                                    )
                                } else {
                                    let tag = format!("{name}__thaw_exception_tag");
                                    self.scope.insert(tag.clone(), HirType::I64);
                                    HirExpr::Var(tag)
                                };
                                return Ok(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_exception_typeof".to_string())),
                                    vec![tag],
                                ));
                            }
                        }
                        // `generic_arrows`/`generic_named_templates` describe *this same*
                        // local binding (a local `const` holding a generic arrow, or one
                        // forwarding to a named generic template) -- they apply regardless
                        // of whether the name is also in `self.scope`, since `infer_expr_
                        // type`'s scope-based inference doesn't know the real, generic
                        // signature for such a binding (it only sees a `Dynamic`
                        // placeholder there). `self.signatures`, in contrast, is the
                        // *global* function table -- a bare reference to an unrelated
                        // top-level function -- which must NOT be consulted when `name` is
                        // actually a local variable/parameter of some other type: a local
                        // binding shadows a same-named top-level function. Checking
                        // signatures unconditionally here used to let a same-named extern/
                        // Fallback function silently steal a shadowing parameter's `typeof`
                        // check, folding it to a constant and running the wrong branch at
                        // runtime.
                        let template = if let HirExpr::Var(name) = &value {
                            self.generic_callable_template(name)
                        } else {
                            None
                        };
                        let operand_type = match &template {
                            Some(GenericCallableTemplate::Arrow { expression, .. }) => {
                                HirType::Function(
                                    vec![HirType::Dynamic; expression.params.len()],
                                    Box::new(HirType::Dynamic),
                                )
                            }
                            Some(GenericCallableTemplate::Named { symbol }) => {
                                let signature = self.signatures.get(symbol).ok_or_else(|| {
                                    format!("generic callable template `{symbol}` lost its signature")
                                })?;
                                let ret = if signature.is_async {
                                    HirType::Promise(Box::new(signature.ret.clone()))
                                } else {
                                    signature.ret.clone()
                                };
                                HirType::Function(signature.params.clone(), Box::new(ret))
                            }
                            None => self.infer_expr_type(&value)?,
                        };
                        // A generic arrow (or an alias forwarding to a generic template) has
                        // no runtime value to read a pointer from; it is always a function.
                        if template.is_some() {
                            return Ok(HirExpr::Lit(HirLit::Str("function".into())));
                        }
                        if operand_type == HirType::Str {
                            let parameter = format!("__thaw_typeof_string_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), HirType::Str);
                            let observed = self.function_observable_typeof(
                                HirExpr::Var(parameter.clone()),
                                &HirType::Str,
                            )?;
                            return self.wrap_call_argument_bindings(
                                observed,
                                &[(parameter, HirType::Str, value)],
                            );
                        }
                        if Self::contains_function_value(&operand_type) {
                            let name =
                                format!("__thaw_typeof_function_tagged_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), operand_type.clone());
                            let observed = self.function_observable_typeof(
                                HirExpr::Var(name.clone()),
                                &operand_type,
                            )?;
                            return self.wrap_call_argument_bindings(
                                observed,
                                &[(name, operand_type, value)],
                            );
                        }
                        if operand_type == HirType::Json {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_typeof".into())),
                                vec![value],
                            ));
                        }
                        if operand_type == HirType::JsValue {
                            let callable = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".into())),
                                vec![HirExpr::Lit(HirLit::Str(
                                    "__thaw_typeof_dynamic_value".into(),
                                ))],
                            );
                            return Ok(HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
                                vec![callable, value],
                            ))));
                        }
                        if let HirType::Union(elements) = &operand_type {
                            let parameter = format!("__thaw_typeof_union_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let member_typeof = |index: usize, member: &HirType| {
                                let projected = HirExpr::UnionValue(
                                    Box::new(bound.clone()),
                                    index,
                                    elements.clone(),
                                );
                                match member {
                                    HirType::Json => Ok(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_typeof".into())),
                                        vec![projected],
                                    )),
                                    HirType::JsValue => Ok(HirExpr::JsonAsString(Box::new(
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
                                            vec![
                                                HirExpr::Call(
                                                    Box::new(HirExpr::Var("getDynamicValue".into())),
                                                    vec![HirExpr::Lit(HirLit::Str(
                                                        "__thaw_typeof_dynamic_value".into(),
                                                    ))],
                                                ),
                                                projected,
                                            ],
                                        ),
                                    ))),
                                    HirType::NativeException => Ok(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_exception_typeof".into())),
                                        vec![HirExpr::Call(
                                            Box::new(HirExpr::Var("__thaw_native_exception_tag".into())),
                                            vec![projected],
                                        )],
                                    )),
                                    _ => native_typeof_name(member)
                                        .map(|name| HirExpr::Lit(HirLit::Str(name.into())))
                                        .ok_or_else(|| format!(
                                            "`typeof` union member has no runtime category: {member:?}"
                                        )),
                                }
                            };
                            let last_index = elements
                                .len()
                                .checked_sub(1)
                                .ok_or("`typeof` cannot inspect an empty union")?;
                            let mut branches = vec![HirStmt::Return(Some(member_typeof(
                                last_index,
                                &elements[last_index],
                            )?))];
                            for (index, member) in elements.iter().enumerate().rev().skip(1) {
                                branches = vec![HirStmt::If(
                                    HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::UnionTag(
                                            Box::new(bound.clone()),
                                            elements.clone(),
                                        )),
                                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                    ),
                                    vec![HirStmt::Return(Some(member_typeof(index, member)?))],
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
                            let parameter = format!("__thaw_typeof_nullish_{}", self.next_binding);
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
                            HirType::Nullable(payload) => Some((payload.as_ref(), "object", true)),
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
                            let present_typeof = if matches!(
                                payload,
                                HirType::Function(_, _)
                                    | HirType::CallableFunction(..)
                                    | HirType::FunctionWithThis(_, _)
                            ) {
                                let present = if nullable {
                                    HirExpr::NullableValue(
                                        Box::new(HirExpr::Var(parameter.clone())),
                                        payload.clone(),
                                    )
                                } else {
                                    HirExpr::OptionalValue(
                                        Box::new(HirExpr::Var(parameter.clone())),
                                        payload.clone(),
                                    )
                                };
                                HirExpr::Conditional(
                                    Box::new(Self::function_pointer_is_undefined(present, payload)),
                                    Box::new(HirExpr::Lit(HirLit::Str("undefined".into()))),
                                    Box::new(HirExpr::Lit(HirLit::Str("function".into()))),
                                    HirType::Str,
                                )
                            } else {
                                HirExpr::Lit(HirLit::Str(type_name.into()))
                            };
                            let result = HirExpr::Block(vec![HirStmt::If(
                                is_none,
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    absent.into(),
                                ))))],
                                vec![HirStmt::Return(Some(present_typeof))],
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
                            if matches!(
                                &operand_type,
                                HirType::Function(_, _)
                                    | HirType::CallableFunction(..)
                                    | HirType::FunctionWithThis(_, _)
                            ) {
                            HirExpr::Conditional(
                                    Box::new(Self::function_pointer_is_undefined(
                                        value,
                                        &operand_type,
                                    )),
                                Box::new(HirExpr::Lit(HirLit::Str("undefined".into()))),
                                Box::new(HirExpr::Lit(HirLit::Str("function".into()))),
                                HirType::Str,
                            )
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
                        match (
                            test.left.as_ref(),
                            test.right.as_ref(),
                            conditional.alt.as_ref(),
                        ) {
                            (
                                Expr::Ident(value),
                                Expr::Ident(undefined),
                                Expr::Ident(alternate),
                            ) if undefined.sym == *"undefined" && value.sym == alternate.sym => {
                                Some(alternate)
                            }
                            (
                                Expr::Ident(undefined),
                                Expr::Ident(value),
                                Expr::Ident(alternate),
                            ) if undefined.sym == *"undefined" && value.sym == alternate.sym => {
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
                let optional_narrowing = self.optional_undefined_narrowing(&conditional.test);
                let union_narrowing = self.union_narrowing(&conditional.test);
                let branch_union = |truth: bool| {
                    union_narrowing
                        .as_ref()
                        .map(|(targets, equal, complement)| {
                        targets
                            .iter()
                            .map(|target| UnionNarrowingTarget {
                                name: target.name.clone(),
                                matching: if truth == *equal {
                                    target.matching.clone()
                                } else if *complement {
                                    target
                                        .allowed
                                        .iter()
                                        .filter(|index| !target.matching.contains(index))
                                        .copied()
                                        .collect()
                                } else {
                                    target.allowed.clone()
                                },
                                allowed: target.allowed.clone(),
                                elements: target.elements.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                };
                let branch_optional = |truth: bool| {
                    optional_narrowing.as_ref().and_then(
                        |(name, payload, present_when_true, absence_kind)| {
                            (truth == *present_when_true)
                                .then(|| (name.clone(), payload.clone(), *absence_kind))
                        },
                    )
                };
                let consequent_union = branch_union(true);
                let alternate_union = branch_union(false);
                let consequent_optional = branch_optional(true);
                let alternate_optional = branch_optional(false);
                let condition_versions = self.narrowing_write_versions.clone();
                let test = self.lower_condition_expr(&conditional.test)?;
                let still_valid = |name: &Symbol| {
                    condition_versions.get(name) == self.narrowing_write_versions.get(name)
                };
                let consequent_optional =
                    consequent_optional.filter(|(name, _, _)| still_valid(name));
                let alternate_optional =
                    alternate_optional.filter(|(name, _, _)| still_valid(name));
                let saved_narrowings = self.save_narrowings();
                let mut consequent = self.lower_expr_with_union_narrowing(
                    &conditional.cons,
                    consequent_union.as_deref(),
                    consequent_optional.as_ref(),
                )?;
                self.restore_narrowings_for_sibling(&saved_narrowings);
                let mut alternate = self.lower_expr_with_union_narrowing(
                    &conditional.alt,
                    alternate_union.as_deref(),
                    alternate_optional.as_ref(),
                )?;
                self.restore_narrowings(saved_narrowings);
                // `Bytes` survives only when both arms are the same buffer type; a mix keeps the
                // top-level erased types as before.
                let (consequent_type, alternate_type) = {
                    let (c, a) = (self.infer_expr_type_inner(&consequent)?, self.infer_expr_type_inner(&alternate)?);
                    if c == a {
                        (c, a)
                    } else {
                        let erase_top = |ty: HirType| if ty == HirType::Bytes { HirType::Array(Box::new(HirType::F64)) } else { ty };
                        (erase_top(c), erase_top(a))
                    }
                };
                let result_type = if consequent_type == alternate_type {
                    consequent_type
                } else if !matches!(consequent_type, HirType::Union(_))
                    && !matches!(alternate_type, HirType::Union(_))
                {
                    let result = match (&consequent_type, &alternate_type) {
                        // `null | undefined` (a nested `c ? null : undefined`)
                        // joined with a payload is that payload's `Nullish`.
                        (HirType::Optional(absent), payload)
                        | (payload, HirType::Optional(absent))
                            if **absent == HirType::Null
                                && !matches!(
                                    payload,
                                    HirType::Optional(_)
                                        | HirType::Nullable(_)
                                        | HirType::Nullish(_)
                                ) =>
                        {
                            HirType::Nullish(Box::new(payload.clone()))
                        }
                        (HirType::Undefined, HirType::Nullable(payload))
                        | (HirType::Nullable(payload), HirType::Undefined) => {
                            HirType::Nullish(payload.clone())
                        }
                        (HirType::Null, HirType::Optional(payload))
                        | (HirType::Optional(payload), HirType::Null) => {
                            HirType::Nullish(payload.clone())
                        }
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
                } else if matches!(&consequent_type, HirType::Union(members) if members.contains(&alternate_type))
                {
                    // `c ? (d ? A : B) : A`: the plain branch joins the union.
                    alternate = self.coerce_to_declared(&consequent_type, alternate)?;
                    consequent_type
                } else if matches!(&alternate_type, HirType::Union(members) if members.contains(&consequent_type))
                {
                    consequent = self.coerce_to_declared(&alternate_type, consequent)?;
                    alternate_type
                } else {
                    return Err(format!(
                        "conditional expression branches have incompatible types {consequent_type:?} and {alternate_type:?}"
                    ));
                };
                Ok(HirExpr::Conditional(
                    Box::new(test),
                    Box::new(consequent),
                    Box::new(alternate),
                    result_type,
                ))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::OptChain(chain) => {
                if let Some(guarded) = self.lower_json_optional_chain(expr)? {
                    return Ok(guarded);
                }
                match chain.base.as_ref() {
                    OptChainBase::Member(member) => self.lower_optional_member_read(member),
                    OptChainBase::Call(call) => self.lower_optional_call(call),
                }
            }

            Expr::Arrow(arrow) => self.lower_arrow(arrow),
            Expr::Fn(function) => self.lower_function_expression(function, None),

            Expr::Array(array_lit) => {
                if array_lit.elems.iter().all(|element| {
                    element
                        .as_ref()
                        .is_none_or(|element| element.spread.is_none())
                }) {
                    let values = array_lit
                        .elems
                        .iter()
                        .map(|element| match element {
                            None => Ok(HirExpr::Lit(HirLit::ArrayHole)),
                            Some(element) => self.lower_expr(&element.expr),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if has_dynamic_storage(expr.span()) {
                        let segments = values
                            .into_iter()
                            .map(|value| {
                                if matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                                    Ok(ArrayLitSegment::Hole)
                                } else {
                                    let ty = self.infer_expr_type(&value)?;
                                    Ok(ArrayLitSegment::Value(value, ty))
                                }
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        return self.lower_dynamic_storage_array(segments);
                    }
                    let preserve_order = values.iter().any(contains_await);
                    let mut bindings = Vec::new();
                    let values = if preserve_order {
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(position, value)| {
                                // A hole has no expression to evaluate. Keep
                                // its marker in the literal so the physical
                                // presence mask still records an absent slot.
                                if matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                                    return Ok(value);
                                }
                                let ty = match self.infer_expr_type_inner(&value) {
                                    Ok(HirType::Bytes) => HirType::Bytes,
                                    _ => self.infer_expr_type(&value)?,
                                };
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
                // Phase 1 -- collect: lower every element/spread exactly
                // once (same order as source, so side effects are
                // unaffected), deferring the "what's the shared element
                // type" decision instead of fixing it from whichever
                // segment is seen first. This is what lets `[0, ...anyArr]`
                // (a concrete element *before* an `any[]` spread) resolve
                // the same way `[...anyArr, 0]` already does.
                let mut segments = Vec::with_capacity(array_lit.elems.len());
                for element in &array_lit.elems {
                    let Some(element) = element else {
                        segments.push(ArrayLitSegment::Hole);
                        continue;
                    };
                    let mut value = self.lower_expr(&element.expr)?;
                    if element.spread.is_some() {
                        value = self.project_native_base_widening_bytes(value)?;
                        // A string/`Map`/`Set` spread source iterates the
                        // same way `for...of` already does for each --
                        // snapshot to an array up front via the exact same
                        // conversions that path uses, rather than requiring
                        // a typed array up front.
                        let mut spread_source_type = self.infer_expr_type(&value)?;
                        if let HirType::Union(members) = &spread_source_type {
                            if members
                                .iter()
                                .all(|member| matches!(member, HirType::Array(_)))
                            {
                                let (flattened, flattened_type) =
                                    self.lower_union_array_sequence(value, members)?;
                                value = flattened;
                                spread_source_type = flattened_type;
                            }
                        }
                        if let Some((collected, _)) = self.collect_generator_for_array_spread(
                            value.clone(),
                            &spread_source_type,
                        )? {
                            value = collected;
                        } else if let HirType::Tuple(elements) = &spread_source_type {
                            // A typed tuple can be a contextual prefix of a longer
                            // JavaScript array. Iterate its actual length through
                            // the same canonical reads as Array.from(tuple).
                            value = self.lower_tuple_as_json_array(value, elements.clone())?;
                        } else if spread_source_type == HirType::Json {
                            // An `any` array (plain, JS-owned or a live view of a native array)
                            // decodes element by element, like the `Array<Json>` copy methods use.
                            // A `Map`/`Set` (wrapper or host collection) yields its entries.
                            value = HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    "__thaw_json_map_or_set_entries".to_string(),
                                )),
                                vec![value],
                            );
                            value = self.coerce_to_declared(
                                &HirType::Array(Box::new(HirType::Json)),
                                value,
                            )?;
                        } else if spread_source_type == HirType::Str {
                            value = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_to_array".to_string())),
                                vec![value],
                            );
                        } else if let HirType::Map(key_type, value_type) = &spread_source_type {
                            let pair_type = HirType::Tuple(vec![
                                key_type.as_ref().clone(),
                                value_type.as_ref().clone(),
                            ]);
                            let array_type = HirType::Array(Box::new(pair_type));
                            value = HirExpr::TypedClosure(
                                array_type,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_map_snapshot_entries".into())),
                                    vec![value],
                                )),
                            );
                        } else if let HirType::Set(element_type) = &spread_source_type {
                            let array_type = HirType::Array(element_type.clone());
                            value = HirExpr::TypedClosure(
                                array_type,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                                    vec![value],
                                )),
                            );
                        } else if matches!(spread_source_type, HirType::Object(_)) {
                            // A custom iterable object
                            // (`{ [Symbol.iterator]() { ... } }`): call its
                            // `[Symbol.iterator]()` (normalized to the
                            // `__thaw_symbol_iterator` field), adapt the
                            // returned iterator object, and collect it into
                            // an array -- the same path `for...of` uses.
                            let iterator = if class_name_from_type(&spread_source_type).is_some() {
                                // A class instance keeps `[Symbol.iterator]` as a method, not a field.
                                let span = swc_common::DUMMY_SP;
                                self.lower_expr(&Expr::Call(CallExpr {
                                    span,
                                    ctxt: Default::default(),
                                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                        span,
                                        obj: element.expr.clone(),
                                        prop: MemberProp::Ident(IdentName::new(
                                            "__thaw_symbol_iterator".into(),
                                            span,
                                        )),
                                    }))),
                                    args: Vec::new(),
                                    type_args: None,
                                }))?
                            } else {
                                HirExpr::Call(
                                    Box::new(HirExpr::PropAccess(
                                        Box::new(value.clone()),
                                        spread_source_type.clone(),
                                        "__thaw_symbol_iterator".to_string(),
                                    )),
                                    Vec::new(),
                                )
                            };
                            let iterator_type = self.infer_expr_type(&iterator)?;
                            let name = format!("__thaw_iterator_{}", self.next_binding);
                            self.next_binding += 1;
                            if let Some((collected, _)) =
                                self.collect_generator_for_array_spread(iterator.clone(), &iterator_type)?
                            {
                                value = collected;
                            } else if let Some((producer, producer_type)) =
                                iterator_object_adapter(self, iterator, &iterator_type, name, false)
                            {
                                if let Some((collected, _)) = self
                                    .collect_generator_for_array_spread(producer, &producer_type)?
                                {
                                    value = collected;
                                }
                            }
                        }
                        let HirType::Array(spread_element) = self.infer_expr_type(&value)? else {
                            return Err("array spread source must be a typed array".into());
                        };
                        // Array spread iterates values: unlike concat, a source hole
                        // becomes an own `undefined` entry in the new array.
                        value = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_densify".into())),
                            vec![HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                vec![
                                    value,
                                    HirExpr::Lit(HirLit::F64(0.0)),
                                    HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                                ],
                            )],
                        );
                        segments.push(ArrayLitSegment::Spread(
                            value,
                            spread_element.as_ref().clone(),
                        ));
                    } else {
                        let actual = self.infer_expr_type(&value)?;
                        segments.push(ArrayLitSegment::Value(value, actual));
                    }
                }

                if has_dynamic_storage(expr.span()) {
                    return self.lower_dynamic_storage_array(segments);
                }

                // Phase 2 -- resolve: fold every segment's type into one
                // shared `element_type`, order-independently. This is a
                // symmetric generalization of the tolerance rules the old
                // single-pass loop already applied (Optional-unwrap,
                // Union-containment, Json-escape-hatch) -- anything that
                // type-checked left-to-right before still resolves to the
                // identical type (that resolution is a special case of this
                // reduce), so this only widens acceptance, never narrows it.
                fn widen_array_element_type(
                    existing: HirType,
                    next: HirType,
                ) -> Result<HirType, String> {
                    if existing == next {
                        return Ok(existing);
                    }
                    if existing == HirType::Optional(Box::new(next.clone())) {
                        return Ok(next);
                    }
                    if next == HirType::Optional(Box::new(existing.clone())) {
                        return Ok(existing);
                    }
                    if matches!(&existing, HirType::Union(members) if members.contains(&next)) {
                        return Ok(existing);
                    }
                    if matches!(&next, HirType::Union(members) if members.contains(&existing)) {
                        return Ok(next);
                    }
                    if existing == HirType::Json || next == HirType::Json {
                        return Ok(HirType::Json);
                    }
                    Err(format!(
                        "array element type {next:?} does not match {existing:?}"
                    ))
                }
                let mut element_type: Option<HirType> = None;
                for segment in &segments {
                    let next = match segment {
                        ArrayLitSegment::Hole => continue,
                        ArrayLitSegment::Value(_, ty) | ArrayLitSegment::Spread(_, ty) => {
                            ty.clone()
                        }
                    };
                    element_type = Some(match element_type {
                        None => next,
                        Some(existing) => widen_array_element_type(existing, next)?,
                    });
                }
                let element_type = element_type.unwrap_or(HirType::F64);

                // Phase 3 -- build: walk the already-lowered segments again
                // (no re-lowering, no re-evaluation) and reconstruct
                // `pending`/`parts` targeting the now-known `element_type`.
                let mut parts = Vec::new();
                let mut pending = Vec::new();
                for segment in segments {
                    match segment {
                        ArrayLitSegment::Hole => pending.push(HirExpr::Lit(HirLit::ArrayHole)),
                        ArrayLitSegment::Value(value, _) => {
                            pending.push(value);
                        }
                        ArrayLitSegment::Spread(mut value, spread_element) => {
                            if !pending.is_empty() {
                                let values = std::mem::take(&mut pending)
                                    .into_iter()
                                    .map(|item| {
                                        if matches!(item, HirExpr::Lit(HirLit::ArrayHole)) {
                                        Ok(item)
                                    } else {
                                        self.coerce_array_insert_value(item, &element_type)
                                        }
                                    })
                                    .collect::<Result<Vec<_>, String>>()?;
                                parts.push(
                                    self.lower_native_array_literal(values, element_type.clone())?,
                                );
                            }
                            if spread_element != element_type {
                                // Only reachable when a *later* segment
                                // forces a wider type than this spread's own
                                // source -- e.g. `[...floatArr, ...anyArr]`.
                                // Not reachable by any case the old
                                // left-to-right loop already accepted (it
                                // required exact equality at the point each
                                // spread was seen).
                                let parameter =
                                    format!("__thaw_array_lit_spread_widen_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(parameter.clone(), spread_element.clone());
                                let converted = self.coerce_to_declared(
                                    &element_type,
                                    HirExpr::Var(parameter.clone()),
                                )?;
                                let callback = HirExpr::Lambda(
                                    Vec::new(),
                                    vec![HirParam {
                                        name: parameter,
                                        ty: spread_element.clone(),
                                    }],
                                    element_type.clone(),
                                    Box::new(converted),
                                );
                                value = self.lower_array_map(
                                    value,
                                    HirType::Array(Box::new(spread_element.clone())),
                                    spread_element.clone(),
                                    spread_element,
                                    callback,
                                    None,
                                )?;
                            }
                            parts.push(value);
                        }
                    }
                }
                if !pending.is_empty() {
                    let values = pending
                        .into_iter()
                        .map(|item| {
                            if matches!(item, HirExpr::Lit(HirLit::ArrayHole)) {
                            Ok(item)
                        } else {
                            self.coerce_array_insert_value(item, &element_type)
                            }
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    parts.push(self.lower_native_array_literal(values, element_type.clone())?);
                }
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

            Expr::Object(obj_lit) => self.lower_object_lit(obj_lit, None),

            Expr::Member(member) => {
                let read = self.lower_member_read(member)?;
                self.bind_extracted_method(read)
            }

            Expr::SuperProp(member) => {
                let (_, base_type, base_name) = self
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
                let owner = if self.class_static_context {
                    base_name
                } else {
                    self.class_super_instance_accessor_owner(&base_type, &property)
                        .ok_or_else(|| {
                            format!("base class `{base_name}` has no accessor `{property}`")
                        })?
                };
                let symbol = class_getter_symbol(&owner, &property, self.class_static_context);
                if !self.signatures.get(&symbol).is_some_and(|signature| {
                    self.class_static_context
                        || signature.accessor_owner.as_deref() == Some(owner.as_str())
                }) {
                    return Err(format!("base class `{owner}` has no getter `{property}`"));
                }
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var(symbol)),
                    if self.class_static_context {
                        Vec::new()
                    } else {
                        vec![self.assert_class_accessor_receiver(
                            HirExpr::Var(self.resolve_binding("this")),
                            &owner,
                        )]
                    },
                ))
            }

            Expr::Assign(assign) => self.lower_assign(assign),

            Expr::Update(update) => self.lower_update(update),

            Expr::Await(await_expr) => {
                // `fetch` is the native body-text fetch; a `Response`-typed (opaque) result needs the realm's.
                let realm_fetch = (self.expected_return_hint == Some(HirType::JsValue))
                    .then(|| Self::realm_fetch_call(&await_expr.arg))
                    .flatten()
                    .filter(|_| !self.scope.contains_key("fetch"));
                let value = self.lower_expr(realm_fetch.as_ref().unwrap_or(&await_expr.arg))?;
                let value_type = self.infer_expr_type(&value)?;
                if value_type == HirType::JsValue {
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("resolveDynamicValue".into())),
                        vec![value],
                    ))
                } else if let HirType::Promise(resolved) = value_type {
                    if *resolved == HirType::Void {
                        Ok(HirExpr::Await(Box::new(value)))
                    } else if *resolved == HirType::Json {
                        // An async function's `any` result may itself be a thenable.
                        let awaited = HirExpr::AwaitPromise(Box::new(value), HirType::Json);
                        self.lower_await_json_thenable(awaited)
                    } else {
                        Ok(HirExpr::AwaitPromise(Box::new(value), *resolved))
                    }
                } else {
                    let resolves_at_dynamic_boundary = matches!(
                        &value,
                        HirExpr::Call(callee, _)
                            if matches!(callee.as_ref(), HirExpr::Var(name)
                                if matches!(name.as_str(),
                                    "callDynamic"
                                        | "callDynamicHandle"
                                        | "callDynamicMethod"
                                        | "callDynamicMethodHandle"
                                        | "callDynamicMethodHandleRaw"
                                        | "__thaw_call_selected_dynamic_method"
                                        | "__thaw_call_selected_dynamic_method_handle"
                                        | "__thaw_call_selected_dynamic_method_raw"
                                        // Invoking an arbitrary already-held
                                        // `JsValue` (e.g. a Hono middleware's
                                        // `next()` continuation) is the same
                                        // synchronous FFI round-trip as the
                                        // named-call forms above -- omitting
                                        // these left `await next()` wrapped
                                        // in a genuine `HirExpr::Await`,
                                        // which `thaw-llvm`'s closure codegen
                                        // can neither prove suspends (it
                                        // isn't a known frame-async source)
                                        // nor compile as a plain synchronous
                                        // statement, so a non-tail-position
                                        // `await next()` inside an async
                                        // closure failed to build at all
                                        // ("does not return a value on all
                                        // paths").
                                        | "callDynamicValue"
                                        | "callDynamicValueHandle"
                                        | "callDynamicValueMixed"
                                        | "callDynamicValueMixedExact"
                                        | "callDynamicValueWithValue"))
                    ) || matches!(
                        &value,
                        HirExpr::DynamicCall(signature, _)
                            if signature.backend == DynamicBackend::QuickJs
                    );
                    if resolves_at_dynamic_boundary {
                        Ok(value)
                    } else if value_type == HirType::Json {
                        self.lower_await_json_thenable(value)
                    } else if matches!(
                        value_type,
                        HirType::F64 | HirType::Bool | HirType::Str | HirType::StrLiteral(_)
                            | HirType::Null | HirType::Undefined
                    ) && self.is_plain_await_operand(&await_expr.arg)
                    // Only literals: a call such as `fetch(..)` is typed by its resolved value.
                    {
                        // Awaiting a plain value is `await Promise.resolve(value)`
                        // (one microtask turn), not a raw await of a non-promise.
                        let resolve = self.lower_call(&Self::promise_resolve_call(
                            await_expr.span,
                            &await_expr.arg,
                        ))?;
                        Ok(match self.infer_expr_type(&resolve)? {
                            HirType::Promise(resolved) if *resolved != HirType::Void => {
                                HirExpr::AwaitPromise(Box::new(resolve), *resolved)
                            }
                            _ => HirExpr::Await(Box::new(resolve)),
                        })
                    } else {
                        Ok(HirExpr::Await(Box::new(value)))
                    }
                }
            }

            Expr::New(new_expr) => {
                if let Expr::Ident(class) = new_expr.callee.as_ref() {
                    if class.sym == *"Array" {
                        return self.lower_array_constructor(
                            new_expr.args.as_deref().unwrap_or_default(),
                            new_expr.type_args.as_deref(),
                            None,
                        );
                    }
                    if class.sym == *"Function" {
                        return self.lower_function_constructor(new_expr);
                    }
                    if matches!(
                        class.sym.as_ref(),
                        "AbortController"
                            | "TextDecoder"
                            | "TextEncoder"
                            | "Int8Array"
                            | "Uint8Array"
                            | "Uint8ClampedArray"
                            | "Int16Array"
                            | "Uint16Array"
                            | "Int32Array"
                            | "Uint32Array"
                            | "Float16Array"
                            | "Float32Array"
                            | "Float64Array"
                            | "ArrayBuffer"
                            | "SharedArrayBuffer"
                            | "DataView"
                    ) {
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(format!(
                                "`new {}()` does not support spread arguments",
                                class.sym
                            ));
                        }
                        if self.is_buffer_view_constructor(class.sym.as_ref(), args)? {
                            return self.lower_buffer_view_constructor(args);
                        }
                        let values = args
                            .iter()
                            .map(|argument| {
                                let value = self.lower_expr_with_expected_type(
                                    &argument.expr,
                                    Some(&HirType::JsValue),
                                )?;
                                self.coerce_to_declared(&HirType::Json, value)
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        let values =
                            self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
                        let constructor = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str(class.sym.to_string()))],
                        );
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("constructDynamicValue".into())),
                            vec![constructor, values],
                        ));
                    }
                    if class.sym == *"RegExp" {
                        let args = new_expr.args.clone().unwrap_or_default();
                        if !(1..=2).contains(&args.len()) {
                            return Err("`new RegExp()` expects one or two arguments".into());
                        }
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("`new RegExp()` does not support spread arguments".into());
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
                        // Weak collections have the same native table layout,
                        // but distinct HIR types keep their non-enumerable API
                        // surface separate. Entries live until the request
                        // arena resets; there is no mid-request native GC.
                        let is_weak = matches!(class.sym.as_ref(), "WeakMap" | "WeakSet");
                        let key_validator: fn(&HirType) -> Result<&'static str, String> = if is_weak
                        {
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
                            let explicit_types = match params {
                                [key, value] => Some((
                                    lower_ts_type(key, self.interfaces, self.generic_interfaces)?,
                                    lower_ts_type(value, self.interfaces, self.generic_interfaces)?,
                                )),
                                [] if !is_weak && !args.is_empty() => None,
                                _ => {
                                    return Err(format!(
                                        "`new {}<K, V>()` requires explicit type arguments",
                                        class.sym
                                    ));
                                }
                            };
                            let Some(argument) = args.first() else {
                                let Some((key_type, value_type)) = explicit_types else {
                                    unreachable!("an untyped Map constructor has an argument")
                                };
                                key_validator(&key_type)?;
                                let map_type = if is_weak {
                                    HirType::WeakMap(Box::new(key_type), Box::new(value_type))
                                } else {
                                    HirType::Map(Box::new(key_type), Box::new(value_type))
                                };
                                return Ok(HirExpr::TypedClosure(
                                    map_type,
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                                        Vec::new(),
                                    )),
                                ));
                            };
                            let mut entries = self.lower_expr(&argument.expr)?;
                            entries = self.native_collection_entries(entries)?;
                            let actual_type = self.infer_expr_type(&entries)?;
                            if !is_weak {
                                if let HirType::Union(members) = &actual_type {
                                    if members
                                        .iter()
                                        .all(|member| matches!(member, HirType::Array(_)))
                                    {
                                        let source_name = format!(
                                            "__thaw_union_map_source_{}",
                                            self.next_binding
                                        );
                                        self.next_binding += 1;
                                        self.scope.insert(source_name.clone(), actual_type.clone());
                                        let mut branches = Vec::with_capacity(members.len());
                                        for (index, member) in members.iter().enumerate() {
                                            let HirType::Array(element) = member else {
                                                unreachable!()
                                            };
                                            let HirType::Tuple(pair) = element.as_ref() else {
                                                return Err(format!(
                                                    "Map constructor union member must contain [key, value] entries, got {element:?}"
                                                ));
                                            };
                                            let [inferred_key, inferred_value] = pair.as_slice()
                                            else {
                                                return Err(format!(
                                                    "Map constructor union member must contain [key, value] entries, got {element:?}"
                                                ));
                                            };
                                            let (key_type, value_type) =
                                                explicit_types.clone().unwrap_or_else(|| {
                                                (inferred_key.clone(), inferred_value.clone())
                                            });
                                            let map_type = HirType::Map(
                                                Box::new(key_type.clone()),
                                                Box::new(value_type.clone()),
                                            );
                                            branches.push(self.lower_map_or_set_from_iterable(
                                                HirExpr::UnionValue(
                                                    Box::new(HirExpr::Var(source_name.clone())),
                                                    index,
                                                    members.clone(),
                                                ),
                                                member.clone(),
                                                element.as_ref().clone(),
                                                map_type,
                                                key_validator(&key_type)?,
                                                key_type,
                                                Some((
                                                    inferred_key.clone(),
                                                    inferred_value.clone(),
                                                )),
                                                true,
                                            )?);
                                        }
                                        let result = self.merge_union_array_method_branches(
                                            &source_name,
                                            members,
                                            branches,
                                        )?;
                                        return self.wrap_call_argument_bindings(
                                            result,
                                            &[(source_name, actual_type, entries)],
                                        );
                                    }
                                }
                            }
                            let explicit_target = explicit_types.is_some();
                            let (key_type, value_type) = if let Some(types) = explicit_types.clone()
                            {
                                types
                            } else {
                                let HirType::Array(element) = &actual_type else {
                                    return Err(format!(
                                        "Map constructor entries must be an array of [key, value] pairs, got {actual_type:?}"
                                    ));
                                };
                                let HirType::Tuple(pair) = element.as_ref() else {
                                    return Err(format!(
                                        "Map constructor entries must be an array of [key, value] pairs, got {element:?}"
                                    ));
                                };
                                let [key, value] = pair.as_slice() else {
                                    return Err(format!(
                                        "Map constructor entries must be [key, value] pairs, got {element:?}"
                                    ));
                                };
                                (key.clone(), value.clone())
                            };
                            key_validator(&key_type)?;
                            let map_type = if is_weak {
                                HirType::WeakMap(
                                    Box::new(key_type.clone()),
                                    Box::new(value_type.clone()),
                                )
                            } else {
                                HirType::Map(
                                    Box::new(key_type.clone()),
                                    Box::new(value_type.clone()),
                                )
                            };
                            let target_pair_type =
                                HirType::Tuple(vec![key_type.clone(), value_type.clone()]);
                            let target_entries_type =
                                HirType::Array(Box::new(target_pair_type.clone()));
                            if is_weak {
                                if let Some((collected, _)) = self
                                    .collect_generator_for_array_spread(
                                        entries.clone(),
                                        &actual_type,
                                    )?
                                {
                                    entries = collected;
                                }
                                self.expect_type(
                                    &target_entries_type,
                                    &entries,
                                    &format!("{} constructor entries", class.sym),
                                )?;
                                return self.lower_map_or_set_from_iterable(
                                    entries,
                                    target_entries_type,
                                    target_pair_type.clone(),
                                    map_type,
                                    key_validator(&key_type)?,
                                    key_type.clone(),
                                    Some((key_type.clone(), value_type.clone())),
                                    false,
                                );
                            }

                            if explicit_target && matches!(&entries, HirExpr::ArrayLit(_)) {
                                let (entries, pair_type, source_key_type, source_value_type) = self
                                    .lower_explicit_map_literal_entries(
                                        entries,
                                        &key_type,
                                        &value_type,
                                    )?;
                                return self.lower_map_or_set_from_iterable(
                                    entries,
                                    target_entries_type,
                                    pair_type,
                                    map_type,
                                    key_validator(&key_type)?,
                                    key_type,
                                    Some((source_key_type, source_value_type)),
                                    false,
                                );
                            }

                            let mut source_array_type = actual_type.clone();
                            if let Some((collected, collected_type)) = self
                                .collect_generator_for_array_spread(entries.clone(), &actual_type)?
                            {
                                entries = collected;
                                // The collector reports the element type.
                                source_array_type = HirType::Array(Box::new(collected_type));
                            }
                            let (source_pair_type, source_key_type, source_value_type) =
                                if explicit_target
                                    && matches!(&entries, HirExpr::ArrayLit(values) if values.is_empty())
                                {
                                    (
                                        target_pair_type.clone(),
                                        key_type.clone(),
                                        value_type.clone(),
                                    )
                                } else {
                                    let HirType::Array(source_pair) = &source_array_type else {
                                        return Err(format!(
                                            "Map constructor entries must be an array of [key, value] pairs, got {source_array_type:?}"
                                        ));
                                    };
                                    let HirType::Tuple(source_slots) = source_pair.as_ref() else {
                                        return Err(format!(
                                            "Map constructor entries must be an array of [key, value] pairs, got {source_pair:?}"
                                        ));
                                    };
                                    let [source_key, source_value] = source_slots.as_slice() else {
                                        return Err(format!(
                                            "Map constructor entries must be [key, value] pairs, got {source_pair:?}"
                                        ));
                                    };
                                    (
                                        source_pair.as_ref().clone(),
                                        source_key.clone(),
                                        source_value.clone(),
                                    )
                                };
                            if explicit_target
                                && matches!(&entries, HirExpr::ArrayLit(values) if values.is_empty())
                            {
                                source_array_type = target_entries_type.clone();
                            }
                            return self.lower_map_or_set_from_iterable(
                                entries,
                                source_array_type,
                                source_pair_type,
                                map_type,
                                key_validator(&key_type)?,
                                key_type,
                                Some((source_key_type, source_value_type)),
                                true,
                            );
                        }
                        let explicit_type = match params {
                            [element] => Some(lower_ts_type(
                                element,
                                self.interfaces,
                                self.generic_interfaces,
                            )?),
                            [] if !is_weak && !args.is_empty() => None,
                            _ => {
                                return Err(format!(
                                    "`new {}<T>()` requires an explicit type argument",
                                    class.sym
                                ));
                            }
                        };
                        let Some(argument) = args.first() else {
                            let element_type =
                                explicit_type.expect("an untyped Set constructor has an argument");
                            key_validator(&element_type)?;
                            let set_type = if is_weak {
                                HirType::WeakSet(Box::new(element_type))
                            } else {
                                HirType::Set(Box::new(element_type))
                            };
                            return Ok(HirExpr::TypedClosure(
                                set_type,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                                    Vec::new(),
                                )),
                            ));
                        };
                        let mut iterable = self.lower_expr(&argument.expr)?;
                        iterable = self.native_collection_entries(iterable)?;
                        let actual_type = self.infer_expr_type(&iterable)?;
                        if !is_weak {
                            if let HirType::Union(members) = &actual_type {
                                if members
                                    .iter()
                                    .all(|member| matches!(member, HirType::Array(_)))
                                {
                                    let source_name =
                                        format!("__thaw_union_set_source_{}", self.next_binding);
                                    self.next_binding += 1;
                                    self.scope.insert(source_name.clone(), actual_type.clone());
                                    let mut branches = Vec::with_capacity(members.len());
                                    for (index, member) in members.iter().enumerate() {
                                        let HirType::Array(inferred_element) = member else {
                                            unreachable!()
                                        };
                                        let element_type = explicit_type
                                            .clone()
                                            .unwrap_or_else(|| inferred_element.as_ref().clone());
                                        let set_type = HirType::Set(Box::new(element_type.clone()));
                                        branches.push(self.lower_map_or_set_from_iterable(
                                            HirExpr::UnionValue(
                                                Box::new(HirExpr::Var(source_name.clone())),
                                                index,
                                                members.clone(),
                                            ),
                                            member.clone(),
                                            inferred_element.as_ref().clone(),
                                            set_type,
                                            key_validator(&element_type)?,
                                            element_type,
                                            None,
                                            true,
                                        )?);
                                    }
                                    let result = self.merge_union_array_method_branches(
                                        &source_name,
                                        members,
                                        branches,
                                    )?;
                                    return self.wrap_call_argument_bindings(
                                        result,
                                        &[(source_name, actual_type, iterable)],
                                    );
                                }
                            }
                        }
                        let explicit_target = explicit_type.is_some();
                        let element_type = if let Some(element_type) = explicit_type.clone() {
                            element_type
                        } else {
                            let HirType::Array(element) = &actual_type else {
                                return Err(format!(
                                    "Set constructor iterable must be an array, got {actual_type:?}"
                                ));
                            };
                            element.as_ref().clone()
                        };
                        key_validator(&element_type)?;
                        let set_type = if is_weak {
                            HirType::WeakSet(Box::new(element_type.clone()))
                        } else {
                            HirType::Set(Box::new(element_type.clone()))
                        };
                        let iterable_type = HirType::Array(Box::new(element_type.clone()));
                        if is_weak {
                            if let Some((collected, _)) = self.collect_generator_for_array_spread(
                                iterable.clone(),
                                &actual_type,
                            )? {
                                iterable = collected;
                            }
                            self.expect_type(
                                &iterable_type,
                                &iterable,
                                &format!("{} constructor iterable", class.sym),
                            )?;
                            return self.lower_map_or_set_from_iterable(
                                iterable,
                                iterable_type,
                                element_type.clone(),
                                set_type,
                                key_validator(&element_type)?,
                                element_type,
                                None,
                                false,
                            );
                        }

                        let mut source_array_type = actual_type.clone();
                        if let Some((collected, collected_type)) =
                            self.collect_generator_for_array_spread(iterable.clone(), &actual_type)?
                        {
                            iterable = collected;
                            // The collector reports the element type.
                            source_array_type = HirType::Array(Box::new(collected_type));
                        }
                        let source_element_type = if explicit_target
                                && matches!(&iterable, HirExpr::ArrayLit(values) if values.is_empty())
                            {
                                source_array_type = iterable_type.clone();
                                element_type.clone()
                            } else {
                                let HirType::Array(source_element) = &source_array_type else {
                                    return Err(format!(
                                        "Set constructor iterable must be an array, got {source_array_type:?}"
                                    ));
                                };
                                source_element.as_ref().clone()
                            };
                        return self.lower_map_or_set_from_iterable(
                            iterable,
                            source_array_type,
                            source_element_type,
                            set_type,
                            key_validator(&element_type)?,
                            element_type,
                            None,
                            true,
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
                            // seconds?, ms?)` uses host-local time.
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
                                Box::new(HirExpr::Var("__thaw_date_local".to_string())),
                                call_args,
                            )
                        } else if let Some(argument) = args.first() {
                            let value = self.lower_expr(&argument.expr)?;
                            self.lower_date_single_timestamp(value)?
                        } else {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_now".to_string())),
                                Vec::new(),
                            )
                        };
                        let date_type = date_object_type();
                        let date_name = format!("__thaw_native_date_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(date_name.clone(), date_type.clone());
                        return Ok(HirExpr::Block(vec![
                            HirStmt::Let(
                                date_name.clone(),
                                date_type.clone(),
                                HirExpr::ClassAlloc(date_type.clone()),
                            ),
                            HirStmt::Expr(HirExpr::PropAssign(
                                Box::new(HirExpr::Var(date_name.clone())),
                                date_type,
                                "timestamp".to_string(),
                                Box::new(timestamp),
                            )),
                            HirStmt::Return(Some(HirExpr::Var(date_name))),
                        ]));
                    }
                    if class.sym == *"SuppressedError" {
                        // `new SuppressedError(error, suppressed, message?)`.
                        // The runtime owns the length-prefixed representation
                        // so nested SuppressedErrors remain unambiguous.
                        let args = new_expr.args.clone().unwrap_or_default();
                        let (arguments, mut bindings) =
                            self.lower_error_constructor_arguments(&args, "SuppressedError")?;
                        let error = arguments
                            .first()
                            .cloned()
                            .unwrap_or(HirExpr::Lit(HirLit::Undefined));
                        let suppressed = arguments
                            .get(1)
                            .cloned()
                            .unwrap_or(HirExpr::Lit(HirLit::Undefined));
                        let (message_present, message) = match arguments.get(2) {
                            Some(message) => {
                                let missing = self.lower_strict_equality(
                                    message.clone(),
                                    HirExpr::Lit(HirLit::Undefined),
                                )?;
                                let text = self.coerce_primitive_to_string(message.clone())?;
                                (
                                    HirExpr::Conditional(
                                        Box::new(missing.clone()),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                        HirType::Bool,
                                    ),
                                    HirExpr::Conditional(
                                        Box::new(missing),
                                    Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
                                        Box::new(text),
                                        HirType::Str,
                                    ),
                                )
                            }
                            None => (
                                HirExpr::Lit(HirLit::Bool(false)),
                                HirExpr::Lit(HirLit::Str(String::new())),
                            ),
                        };
                        let message_name =
                            format!("__thaw_suppressed_message_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(message_name.clone(), HirType::Str);
                        bindings.push((message_name.clone(), HirType::Str, message));
                        let marker = "__thaw_class_identity_\u{1e}SuppressedError\u{1f}Error";
                        let fields = vec![
                            (marker.to_string(), HirExpr::Lit(HirLit::Bool(true))),
                            ("message".to_string(), HirExpr::Var(message_name)),
                            (
                                "name".to_string(),
                                HirExpr::Lit(HirLit::Str("SuppressedError".to_string())),
                            ),
                            ("error".to_string(), error),
                            ("suppressed".to_string(), suppressed),
                        ];
                        let result =
                            self.lower_trusted_error_fields(fields, message_present, None)?;
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if class.sym == *"AggregateError" {
                        // `new AggregateError(errors, message?, options?)`.
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(
                                "`new AggregateError()` does not support spread arguments".into()
                            );
                        }
                        if args.is_empty() {
                            return Err("`new AggregateError()` expects an errors iterable".into());
                        }
                        let errors = self.lower_expr(&args[0].expr)?;
                        let errors_type = self.infer_expr_type(&errors)?;
                        let errors_name = format!("__thaw_aggregate_errors_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(errors_name.clone(), errors_type.clone());
                        let mut bindings = vec![(errors_name.clone(), errors_type.clone(), errors)];

                        let message = if let Some(argument) = args.get(1) {
                            let message = self.lower_expr(&argument.expr)?;
                            let message_type = self.infer_expr_type(&message)?;
                            let message_name =
                                format!("__thaw_aggregate_message_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope
                                .insert(message_name.clone(), message_type.clone());
                            bindings.push((message_name.clone(), message_type, message));
                            Some(message_name)
                        } else {
                            None
                        };
                        let options = if let Some(argument) = args.get(2) {
                            let options = self.lower_expr(&argument.expr)?;
                            let options_type = self.infer_expr_type(&options)?;
                            let options_name =
                                format!("__thaw_aggregate_options_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope
                                .insert(options_name.clone(), options_type.clone());
                            bindings.push((options_name.clone(), options_type.clone(), options));
                            Some((options_name, options_type))
                        } else {
                            None
                        };
                        for argument in args.iter().skip(3) {
                            let value = self.lower_expr(&argument.expr)?;
                            let ty = self.infer_expr_type(&value)?;
                            let name = format!("__thaw_aggregate_extra_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            bindings.push((name, ty, value));
                        }
                        let (message_present, message) = match message {
                            Some(name) => {
                                let raw = HirExpr::Var(name);
                                let missing = self.lower_strict_equality(
                                    raw.clone(),
                                    HirExpr::Lit(HirLit::Undefined),
                                )?;
                                let text = self.coerce_primitive_to_string(raw)?;
                                (
                                    HirExpr::Conditional(
                                        Box::new(missing.clone()),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                        HirType::Bool,
                                    ),
                                    HirExpr::Conditional(
                                        Box::new(missing),
                                    Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
                                        Box::new(text),
                                        HirType::Str,
                                    ),
                                )
                            }
                            None => (
                                HirExpr::Lit(HirLit::Bool(false)),
                                HirExpr::Lit(HirLit::Str(String::new())),
                            ),
                        };
                        let message_name =
                            format!("__thaw_aggregate_message_text_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(message_name.clone(), HirType::Str);
                        bindings.push((message_name.clone(), HirType::Str, message));
                        let marker = "__thaw_class_identity_\u{1e}AggregateError\u{1f}Error";
                        let mut fields = vec![
                            (marker.to_string(), HirExpr::Lit(HirLit::Bool(true))),
                            ("message".to_string(), HirExpr::Var(message_name)),
                            (
                                "name".to_string(),
                                HirExpr::Lit(HirLit::Str("AggregateError".to_string())),
                            ),
                            (
                                "errors".to_string(),
                                self.native_aggregate_errors_as_json(
                                    HirExpr::Var(errors_name),
                                    &errors_type,
                                )?,
                            ),
                        ];
                        let mut cause_present = None;
                        if let Some((name, options_type)) = options {
                            let source = HirExpr::Var(name);
                            let (source, options_type) = if let Some(live) =
                                self.native_error_options_live_json(source.clone(), &options_type)?
                            {
                                // Bind the live Host after message conversion;
                                // HasProperty and Get(cause) share this value.
                                let live_name =
                                    format!("__thaw_aggregate_live_options_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(live_name.clone(), HirType::Json);
                                bindings.push((live_name.clone(), HirType::Json, live));
                                (HirExpr::Var(live_name), HirType::Json)
                            } else {
                                (source, options_type)
                            };
                            let key = HirExpr::Lit(HirLit::Str("cause".into()));
                            let (present, cause) = match &options_type {
                                HirType::Object(option_fields) => {
                                    if option_fields.iter().any(|(field, _)| field == "cause") {
                                        (
                                            self.lower_has_own_value(source.clone(), key.clone())?,
                                            Some(HirExpr::PropAccess(
                                                Box::new(source.clone()),
                                                options_type.clone(),
                                                "cause".into(),
                                            )),
                                        )
                                    } else if let Some(owner) =
                                        self.class_instance_accessor_owner(&options_type, "cause")
                                    {
                                        let symbol = class_getter_symbol(&owner, "cause", false);
                                        let getter = if self.signatures.get(&symbol).is_some_and(
                                            |signature| {
                                                signature.accessor_owner.as_deref()
                                                    == Some(owner.as_str())
                                            },
                                        ) {
                                            HirExpr::Call(
                                                Box::new(HirExpr::Var(symbol)),
                                                vec![self.assert_class_accessor_receiver(
                                                    source.clone(),
                                                    &owner,
                                                )],
                                            )
                                        } else {
                                            HirExpr::Lit(HirLit::Undefined)
                                        };
                                        (HirExpr::Lit(HirLit::Bool(true)), Some(getter))
                                    } else {
                                        let (present, value) = self
                                            .native_error_options_cause(
                                                source.clone(),
                                                &options_type,
                                            )?
                                            .ok_or(
                                                "AggregateError options object lacks cause lookup",
                                            )?;
                                        (present, Some(value))
                                    }
                                }
                                HirType::Json | HirType::Dictionary(_) => {
                                    let object_like = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_is_object_like".into())),
                                        vec![source.clone()],
                                    );
                                    let has = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_has".into())),
                                        vec![source.clone(), key.clone()],
                                    );
                                    (
                                        HirExpr::Conditional(
                                            Box::new(object_like),
                                            Box::new(has),
                                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                            HirType::Bool,
                                        ),
                                        Some(HirExpr::JsonKey(
                                            Box::new(source.clone()),
                                            Box::new(key.clone()),
                                        )),
                                    )
                                }
                                HirType::JsValue | HirType::Dynamic => {
                                    let json_name = format!(
                                        "__thaw_aggregate_options_json_{}",
                                        self.next_binding
                                    );
                                    self.next_binding += 1;
                                    self.scope.insert(json_name.clone(), HirType::Json);
                                    let json_source = if matches!(options_type, HirType::JsValue) {
                                        // Keep the original JS options object and its
                                        // prototype/getter authority; ordinary conversion can
                                        // assimilate a Promise or snapshot the object.
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_exception_json_from_handle".into(),
                                            )),
                                            vec![source.clone()],
                                        )
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, source.clone())?
                                    };
                                    bindings.push((json_name.clone(), HirType::Json, json_source));
                                    let json_source = HirExpr::Var(json_name);
                                    let object_like = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_is_object_like".into())),
                                        vec![json_source.clone()],
                                    );
                                    let has = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_has".into())),
                                        vec![json_source.clone(), key.clone()],
                                    );
                                    (
                                        HirExpr::Conditional(
                                            Box::new(object_like),
                                            Box::new(has),
                                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                            HirType::Bool,
                                        ),
                                        Some(HirExpr::JsonKey(
                                            Box::new(json_source),
                                            Box::new(key.clone()),
                                        )),
                                    )
                                }
                                _ => (HirExpr::Lit(HirLit::Bool(false)), None),
                            };
                            if let Some(cause) = cause {
                                let present_name =
                                    format!("__thaw_aggregate_cause_present_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(present_name.clone(), HirType::Bool);
                                bindings.push((present_name.clone(), HirType::Bool, present));
                                let visible = self.coerce_to_declared(&HirType::Json, cause)?;
                                fields.insert(
                                    3,
                                    (
                                        "cause".into(),
                                        HirExpr::Conditional(
                                    Box::new(HirExpr::Var(present_name.clone())),
                                    Box::new(visible),
                                            Box::new(HirExpr::Call(
                                                Box::new(HirExpr::Var(
                                                    "__thaw_json_undefined".into(),
                                                )),
                                                Vec::new(),
                                            )),
                                    HirType::Json,
                                        ),
                                    ),
                                );
                                cause_present = Some(HirExpr::Var(present_name));
                            }
                        }
                        let result = self.lower_trusted_error_fields(
                            fields,
                            message_present,
                            cause_present,
                        )?;
                        return self.wrap_call_argument_bindings(result, &bindings);
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
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        return self.lower_builtin_error_object(class.sym.as_ref(), args);
                    }
                    let constructor = class_constructor_symbol(class.sym.as_ref());
                    if let Some(signature) = self.signatures.get(&constructor) {
                        if signature.abstract_class_constructor {
                            return Err(format!("cannot construct abstract class `{}`", class.sym));
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
                // A namespaced global constructor (`new Intl.
                // DateTimeFormat(...)`, etc) -- unlike the bare-
                // identifier globals above, this callee is an
                // `Expr::Member`, not `Expr::Ident`, so it never
                // reaches the special-cased list above at all. Reuses
                // the exact same `getDynamicValue`/`constructDynamicValue`
                // mechanism, just with a dotted name (`"Intl.
                // DateTimeFormat"`) that `thaw_js_get_global`
                // (`crates/thaw-quickjs/src/quickjs/api.rs`) resolves by
                // walking nested object properties instead of a single
                // flat global lookup.
                if let Expr::Member(member) = new_expr.callee.as_ref() {
                    if let (Expr::Ident(namespace), MemberProp::Ident(class)) =
                        (member.obj.as_ref(), &member.prop)
                    {
                        let qualified = format!("{}.{}", namespace.sym, class.sym);
                        if matches!(
                            qualified.as_str(),
                            "Intl.DateTimeFormat"
                                | "Intl.NumberFormat"
                                | "Intl.ListFormat"
                                | "Intl.Locale"
                                | "Intl.PluralRules"
                                | "Intl.Collator"
                                | "Intl.Segmenter"
                                | "Intl.RelativeTimeFormat"
                                | "Intl.DisplayNames"
                                | "Intl.DurationFormat"
                        ) {
                            let args = new_expr.args.as_deref().unwrap_or_default();
                            if args.iter().any(|argument| argument.spread.is_some()) {
                                return Err(format!(
                                    "`new {qualified}()` does not support spread arguments"
                                ));
                            }
                            let values = args
                                .iter()
                                .map(|argument| {
                                    let value = self.lower_expr_with_expected_type(
                                        &argument.expr,
                                        Some(&HirType::JsValue),
                                    )?;
                                    self.coerce_to_declared(&HirType::Json, value)
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                            let values =
                                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
                            let constructor = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".into())),
                                vec![HirExpr::Lit(HirLit::Str(qualified))],
                            );
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("constructDynamicValue".into())),
                                vec![constructor, values],
                            ));
                        }
                    }
                }
                // A bare `new X(...)` whose callee is already a known,
                // ordinary (non-class) function signature -- real trigger:
                // an ambient `.d.ts` construct signature (`declare const
                // Database: { new (...): Database; (...): Database; ...}`,
                // real `better-sqlite3`) that the registry/bridge already
                // rewrote to its own generated typed-wrapper symbol
                // (`__thaw_typed_wrapper_js_<hex>`) *before* thaw-hir ever
                // sees this source, so none of the special-cased bare-
                // identifier forms above (real native classes, `Error`,
                // `Date`, `Map`/`Set`, the bare dynamic globals) can ever
                // match it by name. Lowered as a plain call to that same
                // symbol: the wrapper's own JSON-marshaling shim ultimately
                // reaches a real underlying JS/N-API value that already
                // knows how to be constructed, and the `.d.ts` shape this
                // targets always advertises an *identical* plain-call
                // signature for the exact same instance type, so dropping
                // `new` here doesn't change the JS semantics for this case.
                if let Expr::Ident(class) = new_expr.callee.as_ref() {
                    if class.sym == *"WeakRef" {
                        // A *strong* approximation: the target is kept
                        // alive for the enclosing arena's lifetime, so
                        // `.deref()` always yields it. This is
                        // indistinguishable from a real weak reference here
                        // -- thaw's arena never collects anything (the whole
                        // invocation's values live until it returns) and the
                        // only holder would be the `WeakRef` anyway, so a
                        // real `deref()` could not reliably observe a
                        // collection either. Modeled as a one-element array,
                        // so `.deref()` is a plain index-0 read.
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("`new WeakRef()` does not support spread arguments".into());
                        }
                        let [target] = args else {
                            return Err(
                                "`new WeakRef()` expects exactly one target argument".into()
                            );
                        };
                        let target = self.lower_expr(&target.expr)?;
                        return Ok(HirExpr::ArrayLit(vec![target]));
                    }
                    if matches!(
                        class.sym.as_ref(),
                        "FinalizationRegistry"
                            | "Proxy"
                            | "DisposableStack"
                            | "AsyncDisposableStack"
                            // Web / WHATWG constructors with no compiled
                            // layout: build the real QuickJS global (thaw's
                            // platform globals provide these) and keep it as
                            // an opaque dynamic handle, so `new URL(...)`,
                            // `new Headers(...)`, `new Response(...)`, ...
                            // work and their methods read back dynamically.
                            | "URL"
                            | "URLSearchParams"
                            | "Headers"
                            | "Request"
                            | "Response"
                            | "Blob"
                            | "File"
                            | "FormData"
                            | "Event"
                            | "EventTarget"
                            | "MessageEvent"
                            | "MessageChannel"
                            | "MessagePort"
                            | "BroadcastChannel"
                            | "DOMException"
                            | "ReadableStream"
                            | "WritableStream"
                            | "TransformStream"
                    ) {
                        // No compiled object model exists for these, so
                        // construct the real QuickJS global and keep it as
                        // a dynamic handle instead of approximating it:
                        // `FinalizationRegistry` retains its callback (it
                        // only fires on collection, which never happens
                        // observably within an arena), and `Proxy`
                        // delegates to a genuine JS proxy -- the
                        // target/handler must be JSON-representable (a
                        // compiled object is passed by value), matching
                        // every other dynamic-constructor argument.
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(format!(
                                "`new {}()` does not support spread arguments",
                                class.sym
                            ));
                        }
                        // `new Proxy(target, handler)`'s own `target`
                        // argument, when it's a simple identifier already
                        // declared `Object`/`Dictionary`-shaped in this
                        // function: `lower_var_decl` (statements/
                        // declarations.rs) already detected this exact
                        // shape ahead of time (before this expression is
                        // lowered at all) and emitted a `let` binding for
                        // an *independent*, live QuickJS handle retaining
                        // `target`'s current value, registering it in
                        // `proxy_target_live_handles`. Reused here as the
                        // constructor argument instead of a fresh one-off
                        // JSON snapshot, so this and every later read of
                        // the `target` identifier itself (see the
                        // `Expr::Ident` arm's own check of the same table)
                        // observe the same handle -- a `set` trap's
                        // mutation, which lands on this handle's own
                        // QuickJS object, becomes observable from `target`
                        // directly, not just through the proxy.
                        let mut values = Vec::with_capacity(args.len());
                        for (index, argument) in args.iter().enumerate() {
                            if class.sym == *"Proxy" && index == 0 {
                                if let Expr::Ident(target_ident) = argument.expr.as_ref() {
                                    let target_name =
                                        self.resolve_binding(target_ident.sym.as_ref());
                                    if let Some((handle_name, _)) =
                                        self.proxy_target_live_handles.get(&target_name).cloned()
                                    {
                                        values.push(self.coerce_to_declared(
                                            &HirType::Json,
                                            HirExpr::Var(handle_name),
                                        )?);
                                        continue;
                                    }
                                }
                            }
                            let value = self.lower_expr_with_expected_type(
                                &argument.expr,
                                Some(&HirType::JsValue),
                            )?;
                            values.push(self.coerce_to_declared(&HirType::Json, value)?);
                        }
                        let values =
                            self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
                        let constructor = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str(class.sym.to_string()))],
                        );
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("constructDynamicValue".into())),
                            vec![constructor, values],
                        ));
                    }
                }
                if let Expr::Ident(callee) = new_expr.callee.as_ref() {
                    let name = self.resolve_binding(callee.sym.as_ref());
                    if self.signatures.contains_key(&name) {
                        return self.lower_call(&CallExpr {
                            span: new_expr.span,
                            ctxt: new_expr.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(callee.clone()))),
                            args: new_expr.args.clone().unwrap_or_default(),
                            type_args: new_expr.type_args.clone(),
                        });
                    }
                }
                // A `new` whose callee is itself a dynamic value (`new (A as
                // any)()`, a class value or npm constructor held in an `any`
                // binding) -- construct through the realm's own `new`.
                let dynamic_callee = match new_expr.callee.as_ref() {
                    // A constructor held in an `any`/dynamic binding
                    // (`const C: any = SomeCtor; new C()`): construct through
                    // the realm. An ordinary typed callee (a function/class
                    // signature) was already handled above.
                    Expr::Ident(ident) => {
                        let name = self.resolve_binding(ident.sym.as_ref());
                        // The primitive wrapper constructors (`new Number(1)`)
                        // are realm constructors with no typed lowering.
                        let realm_wrapper = matches!(ident.sym.as_ref(), "Number" | "String" | "Boolean")
                            && !self.scope.contains_key(&name)
                            && !self.signatures.contains_key(&name);
                        realm_wrapper
                            || self.scope.get(&name).is_some_and(|ty| {
                                matches!(ty, HirType::JsValue | HirType::Dynamic | HirType::Json)
                            })
                    }
                    _ => true,
                };
                if dynamic_callee {
                    let callee = self.lower_expr_with_expected_type(
                        new_expr.callee.as_ref(),
                        Some(&HirType::JsValue),
                    )?;
                    let callee_type = self.infer_expr_type(&callee)?;
                    if matches!(callee_type, HirType::JsValue | HirType::Dynamic | HirType::Json) {
                        // The runtime construct call takes a live handle; a
                        // `Json`-held callable (`const p: any = fn`) must be
                        // turned back into one first.
                        let callee = if callee_type == HirType::Json {
                            self.coerce_to_declared(&HirType::JsValue, callee)?
                        } else {
                            callee
                        };
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        let values = self.dynamic_call_args_json(args, None)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("constructDynamicValue".into())),
                            vec![callee, values],
                        ));
                    }
                }
                self.lower_promise_new(new_expr)
            }

            Expr::MetaProp(meta) if meta.kind == swc_ecma_ast::MetaPropKind::NewTarget => Err(
                "SyntaxError: `new.target` is only supported inside a top-level, non-async, non-generator \
                 function declaration or class constructor (only as `new.target === C`, `new.target.name` or \
                 `Object.setPrototypeOf(this, new.target.prototype)` there); it is not supported at \
                 the top level, in methods, nested functions or exported functions"
                    .into(),
            ),

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
    /// pair (`source_element_type` and `value_info` carry its original
    /// physical slot types); for a `Set`, each element IS the key, with the value
    /// half fixed at `0.0` (a dummy word, exactly like `.add()`).
    fn constructor_slot_can_widen_to_json(source_type: &HirType) -> bool {
        match source_type {
            HirType::F64
            | HirType::I64
            | HirType::Str
            | HirType::StrLiteral(_)
            | HirType::Bool
            | HirType::Null
            | HirType::Undefined
            | HirType::Json
            | HirType::JsValue => true,
            HirType::Object(_) => {
                source_type != &date_object_type()
                    && Self::fixed_object_supports_live_projection(source_type)
            }
            _ => false,
        }
    }

    fn coerce_constructor_slot(
        &mut self,
        source_type: &HirType,
        target_type: &HirType,
        value: HirExpr,
        context: &str,
        allow_json_widening: bool,
        is_key: bool,
    ) -> Result<HirExpr, String> {
        if source_type == target_type {
            return Ok(value);
        }
        if allow_json_widening
            && *target_type == HirType::Json
            && Self::constructor_slot_can_widen_to_json(source_type)
        {
            let converted = if is_key {
                self.coerce_map_key(target_type, value)?
            } else {
                self.coerce_to_declared(target_type, value)?
            };
            self.expect_type(target_type, &converted, context)?;
            return Ok(converted);
        }
        self.expect_type(target_type, &value, context)?;
        Ok(value)
    }

    fn lower_explicit_map_literal_entries(
        &mut self,
        entries: HirExpr,
        key_type: &HirType,
        value_type: &HirType,
    ) -> Result<(HirExpr, HirType, HirType, HirType), String> {
        let HirExpr::ArrayLit(pairs) = entries else {
            unreachable!("literal constructor helper receives an ArrayLit")
        };
        let pair_type = HirType::Tuple(vec![key_type.clone(), value_type.clone()]);
        let entries_type = HirType::Array(Box::new(pair_type.clone()));
        let mut bindings = Vec::new();
        let mut normalized_pairs = Vec::with_capacity(pairs.len());

        for (pair_index, pair) in pairs.into_iter().enumerate() {
            let (source_key, source_value) = match pair {
                HirExpr::ArrayLit(slots) => {
                    let [source_key, source_value] = slots.as_slice() else {
                        return Err(format!(
                            "Map constructor entries must be [key, value] pairs, got {} slots at index {pair_index}",
                            slots.len()
                        ));
                    };
                    (source_key.clone(), source_value.clone())
                }
                entry => {
                    let entry_type = self.infer_expr_type(&entry)?;
                    let HirType::Tuple(source_slots) = &entry_type else {
                        return Err(format!(
                            "Map constructor entries must be a two-slot native Tuple, got {entry_type:?} at index {pair_index}"
                        ));
                    };
                    let [source_key_type, source_value_type] = source_slots.as_slice() else {
                        return Err(format!(
                            "Map constructor entries must be [key, value] pairs, got {} Tuple slots at index {pair_index}",
                            source_slots.len()
                        ));
                    };
                    let source_key_type = source_key_type.clone();
                    let source_value_type = source_value_type.clone();
                    let entry_name =
                        format!("__thaw_map_literal_pair_{pair_index}_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(entry_name.clone(), entry_type.clone());
                    bindings.push((entry_name.clone(), entry_type, entry));
                    (
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(entry_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                            source_key_type,
                        ),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(entry_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                            source_value_type,
                        ),
                    )
                }
            };

            let mut normalized_slots = Vec::with_capacity(2);
            for (slot_index, (source, target, role)) in [
                (&source_key, key_type, "key"),
                (&source_value, value_type, "value"),
            ]
            .into_iter()
            .enumerate()
            {
                let source_type = self.infer_expr_type(source)?;
                let name = format!(
                    "__thaw_map_literal_{role}_{pair_index}_{slot_index}_{}",
                    self.next_binding
                );
                self.next_binding += 1;
                self.scope.insert(name.clone(), source_type.clone());
                bindings.push((name.clone(), source_type.clone(), source.clone()));
                normalized_slots.push(self.coerce_constructor_slot(
                    &source_type,
                    target,
                    HirExpr::Var(name),
                    &format!("Map constructor {role}"),
                    true,
                    slot_index == 0,
                )?);
            }
            normalized_pairs.push(HirExpr::TypedClosure(
                pair_type.clone(),
                Box::new(HirExpr::ArrayLit(normalized_slots)),
            ));
        }

        let normalized = HirExpr::TypedClosure(
            entries_type.clone(),
            Box::new(HirExpr::ArrayLit(normalized_pairs)),
        );
        Ok((
            self.wrap_call_argument_bindings(normalized, &bindings)?,
            pair_type,
            key_type.clone(),
            value_type.clone(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_map_or_set_from_iterable(
        &mut self,
        source: HirExpr,
        source_type: HirType,
        source_element_type: HirType,
        result_type: HirType,
        key_suffix: &'static str,
        key_type: HirType,
        value_info: Option<(HirType, HirType)>,
        allow_json_widening: bool,
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
            if let Some((source_key_type, source_value_type)) = value_info {
                let pair_type = source_element_type.clone();
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
                    source_key_type.clone(),
                );
                let value_expr = HirExpr::TypedIndex(
                    Box::new(var(&pair_name)),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    source_value_type.clone(),
                );
                let converted_key = self.coerce_constructor_slot(
                    &source_key_type,
                    &key_type,
                    key_expr,
                    "Map constructor key",
                    allow_json_widening,
                    true,
                )?;
                let target_value_type = match &result_type {
                    HirType::Map(_, value) | HirType::WeakMap(_, value) => value.as_ref(),
                    _ => return Err("Map constructor builder received a non-Map result".into()),
                };
                let converted_value = self.coerce_constructor_slot(
                    &source_value_type,
                    target_value_type,
                    value_expr,
                    "Map constructor value",
                    allow_json_widening,
                    false,
                )?;
                (converted_key, converted_value, vec![pair_let])
            } else {
                let element_expr = HirExpr::TypedIndex(
                    Box::new(var(&source_name)),
                    Box::new(var(&index_name)),
                    source_element_type.clone(),
                );
                let converted_element = self.coerce_constructor_slot(
                    &source_element_type,
                    &key_type,
                    element_expr,
                    "Set constructor element",
                    allow_json_widening,
                    true,
                )?;
                (
                    converted_element,
                    HirExpr::Lit(HirLit::F64(0.0)),
                    Vec::new(),
                )
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
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                    Vec::new(),
                ),
            ),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&source_name))),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
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

    /// `fetch(...)` rewritten to `(globalThis as any).fetch(...)`, the realm's Response-returning fetch.
    fn realm_fetch_call(arg: &Expr) -> Option<Expr> {
        let Expr::Call(call) = arg else { return None };
        let swc_ecma_ast::Callee::Expr(callee) = &call.callee else { return None };
        if !matches!(callee.as_ref(), Expr::Ident(name) if &*name.sym == "fetch") {
            return None;
        }
        let global = Expr::Paren(swc_ecma_ast::ParenExpr {
            span: swc_common::DUMMY_SP,
            expr: Box::new(Expr::TsAs(swc_ecma_ast::TsAsExpr {
                span: swc_common::DUMMY_SP,
                expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt("globalThis".into(), swc_common::DUMMY_SP))),
                type_ann: Box::new(swc_ecma_ast::TsType::TsKeywordType(swc_ecma_ast::TsKeywordType {
                    span: swc_common::DUMMY_SP,
                    kind: swc_ecma_ast::TsKeywordTypeKind::TsAnyKeyword,
                })),
            })),
        });
        let mut rewritten = call.clone();
        rewritten.callee = swc_ecma_ast::Callee::Expr(Box::new(Expr::Member(swc_ecma_ast::MemberExpr {
            span: swc_common::DUMMY_SP,
            obj: Box::new(global),
            prop: MemberProp::Ident(swc_ecma_ast::IdentName::new("fetch".into(), swc_common::DUMMY_SP)),
        })));
        Some(Expr::Call(rewritten))
    }

    /// Lowers `expr` exactly like `lower_expr`, except that if `expr` is
    /// directly a call expression, `expected` (when given) is offered to
    /// that call's own generic-type-parameter inference as a fallback for
    /// any type parameter its arguments alone don't determine -- needed
    /// for a function like `nanoid<Type extends string>(size?: number):
    /// Type`, whose only type parameter appears solely in the return
    /// position, so `const id: string = nanoid()` is the only place
    /// `Type`'s value (`string`, from this very annotation) is ever
    /// written down.
    ///
    /// Deliberately not threaded generally through `lower_expr` itself --
    /// this compiler's types are checked by lowering an expression first
    /// and comparing the result against an expected type after
    /// (`coerce_to_declared`), and changing that everywhere would be a
    /// much larger, riskier change than this one narrow addition needs.
    /// Instead this is called only from the specific "sink" points that
    /// already know an expected type ahead of lowering (currently: a
    /// `let`/`const` declaration's own type annotation), via a one-shot
    /// hint (`expected_return_hint`) that `lower_call` takes (not just
    /// reads) as its very first action, before lowering this call's own
    /// arguments -- so it can never leak into a nested call's inference,
    /// and never lingers if this expression turns out not to be a call
    /// `lower_call` actually consumes it for.
    fn lower_expr_with_expected_type(
        &mut self,
        expr: &Expr,
        expected: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let expected_object_fields = expected.and_then(expected_object_fields);
        if let (Some((args, type_args)), Some(HirType::Array(element))) =
            (Self::array_constructor_parts(expr), expected)
        {
            return self.lower_array_constructor(args, type_args, Some(element.as_ref().clone()));
        }
        if has_dynamic_storage(expr.span()) {
            self.validate_storage_call_source(expr)?;
            if let Some(expected) = expected
                .filter(|ty| !matches!(ty, HirType::Json | HirType::JsValue | HirType::Dynamic))
            {
                self.validate_storage_source(expr, expected)?;
            }
            if let Expr::Object(object) = expr {
                if let Some(fields) = expected_object_fields {
                    return self.lower_dynamic_accessor_object_lit_with_expected_fields(
                        object,
                        true,
                        Some(fields),
                    );
                }
                return self.lower_expr(expr);
            }
            if matches!(expr, Expr::Array(_)) {
                return self.lower_expr(expr);
            }
        }
        if let Some(expected) = expected {
            let supplied = match expr {
                Expr::Arrow(arrow) => Some(arrow.params.len()),
                Expr::Fn(function) => Some(function.function.params.len()),
                Expr::Ident(ident) => {
                    let name = self.resolve_binding(ident.sym.as_ref());
                    (self.scope.get(&name).is_some_and(|ty| {
                        matches!(
                            ty,
                            HirType::Function(_, _)
                                | HirType::CallableFunction(..)
                                | HirType::FunctionWithThis(_, _)
                        )
                    }) || self.generic_arrows.contains_key(&name)
                        || self.generic_named_templates.contains_key(&name)
                        || self.signatures.contains_key(&name))
                    .then_some(usize::MAX)
                }
                _ => None,
            };
            if let Some((params, ret)) =
                supplied.and_then(|supplied| callback_signature(expected, supplied))
            {
                return self.lower_promise_callback(expr, &params, Some(&ret));
            }
        }
        if let (Expr::Object(object), Some(HirType::Json)) = (expr, expected) {
            // A method/accessor-bearing `any` literal needs one live storage
            // identity for its receiver, property writes and aliases. Reuse
            // the dynamic object builder's actual JS `this` representation.
            // A `Symbol.iterator`-style key must also stay a real symbol there:
            // the typed object form would store it under a string sentinel.
            if self.object_literal_needs_live_object(object) {
                return self.lower_dynamic_accessor_object_lit(object, true);
            }
        }
        if let (Expr::Array(array), Some(HirType::Json)) = (expr, expected) {
            if self.literal_holds_live_object(expr) {
                if let Some(lowered) = self.lower_any_live_array(array)? {
                    return self.coerce_to_declared(&HirType::Json, lowered);
                }
            }
        }
        if let (Expr::Array(array), Some(HirType::Json)) = (expr, expected) {
            if let Some(literal) = self.lower_any_array_with_collections(array)? {
                return Ok(literal);
            }
        }
        if let (Expr::Object(_), Some(HirType::Json)) = (expr, expected) {
            self.any_object_literal = true;
            let lowered = self.lower_expr(expr);
            self.any_object_literal = false;
            return lowered;
        }
        if let (Expr::Object(object), Some(fields)) = (expr, expected_object_fields) {
            return self.lower_object_lit(object, Some(fields));
        }
        // `Expr::Await` is included alongside `Expr::Call` -- an awaited
        // expression's own expected type is really about the *resolved*
        // value the await produces, which for a dynamic method call
        // (`await app.request(...)`, real hono) is exactly the hint its
        // own inner `Expr::Call` needs (`lower_call`'s own `.take()` of
        // this same field, unchanged, still finds it: `Expr::Await`'s
        // handler in `lower_expr`'s own big match calls plain `lower_
        // expr` on its argument, not this function, so the hint set here
        // stays live all the way through since nothing clears it until
        // this whole call returns). Real trigger: `const res: JsValue =
        // await app.request('/')` used to fail ("value has type Json,
        // expected JsValue") since the inner method call had no hint at
        // all and defaulted to the untyped, JSON-decoding dispatch.
        if matches!(expr, Expr::Call(_) | Expr::Await(_)) {
            self.expected_return_hint = expected.cloned();
        }
        // An arrow function passed directly as a dynamic-call argument
        // (a callback handed to hono's `app.get`, zod's `.refine`, etc.)
        // gets the same one-shot treatment, but aimed at its *body*'s
        // return statements rather than the arrow value itself -- see
        // `expected_arrow_return_hint`'s own doc comment. Only meaningful
        // when the hint is `JsValue`, since that's the only expected type
        // this call site ever passes for a callback argument.
        if matches!(expr, Expr::Arrow(_)) && expected == Some(&HirType::JsValue) {
            self.expected_arrow_return_hint = expected.cloned();
        }
        let result = self.lower_expr(expr);
        self.expected_return_hint = None;
        self.expected_arrow_return_hint = None;
        result
    }

    /// `[..] as any[]` / `[..] as any` / `<any[]>[..]` / `{..} as any`: the assertion hands the
    /// fresh literal to an `any` slot, so it gets the same dynamic element semantics as
    /// `const x: any[] | any = <literal>` (each element is an `any` value, `push` takes any
    /// type). `as_field` leaves the literal a native `any[]` for the caller to turn into a real JS
    /// array (a field of an `any` object literal). `None` when this is not such an assertion.
    pub(in crate::lower) fn lower_any_asserted_literal(
        &mut self,
        inner: &Expr,
        type_ann: &TsType,
        as_field: bool,
    ) -> Result<Option<HirExpr>, String> {
        let Some(array_target) = any_like_assertion_target(type_ann) else {
            return Ok(None);
        };
        let inner = peel_parens(inner);
        match inner {
            // Only an object that holds an array literal changes meaning under the assertion
            // (its element type); any other `{..} as any` keeps its typed lowering.
            Expr::Object(object) if !array_target && object_literal_holds_array(object) => {
                let lowered = self.lower_expr_with_expected_type(inner, Some(&HirType::Json))?;
                self.coerce_to_declared_aliasing(&HirType::Json, lowered)
                    .map(Some)
            }
            Expr::Array(array)
                if array.elems.iter().all(|element| {
                    element
                        .as_ref()
                        .is_some_and(|element| element.spread.is_none())
                }) =>
            {
                let mut elements = Vec::with_capacity(array.elems.len());
                for element in array.elems.iter().flatten() {
                    let value = self.lower_expr(&element.expr)?;
                    if contains_await(&value) {
                        return Ok(None);
                    }
                    elements.push(self.coerce_to_declared(&HirType::Json, value)?);
                }
                let array = HirExpr::ArrayLit(elements);
                self.infer_expr_type(&array)?;
                if array_target || as_field {
                    Ok(Some(array))
                } else {
                    self.coerce_to_declared(&HirType::Json, array).map(Some)
                }
            }
            _ => Ok(None),
        }
    }
}

/// `Some(false)` for an `any`/`unknown` assertion target, `Some(true)` for `any[]`/`unknown[]`/
/// `Array<any>`; `None` for every other type.
fn any_like_assertion_target(ty: &TsType) -> Option<bool> {
    fn any_keyword(ty: &TsType) -> bool {
        matches!(ty, TsType::TsKeywordType(keyword) if matches!(keyword.kind,
            TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword))
    }
    match ty {
        _ if any_keyword(ty) => Some(false),
        TsType::TsArrayType(array) if any_keyword(&array.elem_type) => Some(true),
        TsType::TsTypeRef(reference)
            if matches!(&reference.type_name,
                swc_ecma_ast::TsEntityName::Ident(name) if name.sym.as_ref() == "Array")
                && reference.type_params.as_ref().is_some_and(|params| {
                    params.params.len() == 1 && any_keyword(&params.params[0])
                }) =>
        {
            Some(true)
        }
        TsType::TsParenthesizedType(paren) => any_like_assertion_target(&paren.type_ann),
        _ => None,
    }
}

/// Whether an object literal has an array literal among its property values, at any depth.
fn object_literal_holds_array(object: &swc_ecma_ast::ObjectLit) -> bool {
    object.props.iter().any(|property| match property {
        swc_ecma_ast::PropOrSpread::Prop(property) => match property.as_ref() {
            swc_ecma_ast::Prop::KeyValue(swc_ecma_ast::KeyValueProp { value, .. }) => {
                let value = split_type_assertion(value).map_or(value.as_ref(), |(inner, _)| inner);
                match peel_parens(value) {
                    Expr::Array(_) => true,
                    Expr::Object(nested) => object_literal_holds_array(nested),
                    _ => false,
                }
            }
            _ => false,
        },
        _ => false,
    })
}

pub(in crate::lower) fn peel_parens(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(paren) = expr {
        expr = &paren.expr;
    }
    expr
}

/// The operand and asserted type of `x as T` / `<T>x`, looking through parentheses.
pub(in crate::lower) fn split_type_assertion(expr: &Expr) -> Option<(&Expr, &TsType)> {
    match expr {
        Expr::Paren(paren) => split_type_assertion(&paren.expr),
        Expr::TsAs(assertion) => Some((&assertion.expr, &assertion.type_ann)),
        Expr::TsTypeAssertion(assertion) => Some((&assertion.expr, &assertion.type_ann)),
        _ => None,
    }
}

/// `Date`/`RegExp`/`Map`/`Set`/`WeakMap`/`WeakSet` are all native,
/// non-generic-class shapes -- none has an entry in `self.signatures`
/// (each has its own bespoke method-dispatch file, not a real
/// `self.interfaces`/constructor-signature registration the way a real
/// user or npm-declared class does), so `class_type_has_identity`'s
/// `__thaw_class_identity_`-marker-field check can never match any of
/// them: `RegExp`'s native shape is `{source, flags, lastIndex}` (no
/// marker field at all), and `Map`/`Set`/`WeakMap`/`WeakSet` aren't even
/// `HirType::Object` to begin with. Before this, `instanceof` against
/// any of these eight names always hit the "not a known class" error --
/// confirmed via a direct probe that this affected even a plainly,
/// statically-typed `RegExp`/`Map` value, not just the dynamic-value
/// case `Date`'s own pre-existing special handling (elsewhere in this
/// file) already covered.
///
/// `None` for a name that isn't one of these eight (the caller falls back
/// to the ordinary `self.signatures`/`class_type_has_identity` path).
/// `Some(bool)` is a compile-time-constant answer -- `ty` already fully
/// determines it, since none of the eight is ever ambiguous at its own
/// native, non-`Json`/`JsValue` type.
/// Whether a value of static type `ty` is an `Object` (every native object, array and
/// collection) or a primitive; `None` when the static type cannot tell.
fn native_object_instanceof(ty: &HirType) -> Option<bool> {
    match ty {
        HirType::Array(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Dictionary(_)
        | HirType::Map(..)
        | HirType::Set(_)
        | HirType::WeakMap(..)
        | HirType::WeakSet(_)
        | HirType::Bytes
        | HirType::Promise(_) => Some(true),
        HirType::Bool => Some(false),
        _ => None,
    }
}

fn native_builtin_instanceof_static_match(class: &str, ty: &HirType) -> Option<bool> {
    // A `Map`/`Set` subclass instance is an object: its identity chain decides.
    if native_base_type(ty).is_some() && matches!(class, "Map" | "Set" | "Array") {
        return None;
    }
    Some(match class {
        "Date" => *ty == date_object_type(),
        "RegExp" => *ty == regex_object_type(),
        "Map" => matches!(ty, HirType::Map(_, _)),
        "Set" => matches!(ty, HirType::Set(_)),
        "WeakMap" => matches!(ty, HirType::WeakMap(_, _)),
        "WeakSet" => matches!(ty, HirType::WeakSet(_)),
        // A TS tuple (`[number, string]`) is a real `Array` at runtime
        // too -- matches `Array.isArray`'s own identical match arm
        // (`static_builtins.rs`).
        "Array" => matches!(ty, HirType::Array(_) | HirType::Tuple(_)),
        "Promise" => matches!(ty, HirType::Promise(_)),
        // A genuine match (`ty` here is `value_type`, the *normalized*
        // type -- `Bytes` always reads as `Array(F64)` by this point)
        // was already handled by a dedicated early check using
        // `infer_expr_type_inner` instead, right after `value_type` is
        // first computed -- this residual entry only exists so
        // `"anything else" instanceof Uint8Array` (a plain `number[]`,
        // a string, ...) still reads as the correct `false` instead of
        // hitting "not a known class", now that `Uint8Array` needs to
        // be recognized as a known class at all.
        "Uint8Array" => false,
        _ => return None,
    })
}

/// The sentinel key `coerce_to_declared`'s `Json`-target branch
/// (`inference/coercions.rs`) tags a `RegExp`/`Map`/`Set` value with
/// when it crosses into `any` -- used to recognize it again for
/// `instanceof` against a `Json`-typed receiver, the same wrapper
/// `regexp_wrapper_property`/`map_or_set_wrapper_size` (thaw-std's
/// `json.rs`) already unwrap for property reads. `None` for `Date`
/// (which uses its own, non-sentinel `__thaw_json_is_date_shape` check
/// instead -- its `{"timestamp": N}` wire shape isn't a tagged wrapper)
/// and for `WeakMap`/`WeakSet` (native table entries live only until
/// the request arena resets and never get a `Json`-crossing
/// representation at all, so a `WeakMap`/`WeakSet` can't actually reach
/// this call with a `Json`-typed receiver in practice).
fn native_builtin_instanceof_json_sentinel(class: &str) -> Option<&'static str> {
    Some(match class {
        "RegExp" => "__thaw_regexp__",
        "Map" => "__thaw_map_entries__",
        "Set" => "__thaw_set_values__",
        _ => return None,
    })
}
