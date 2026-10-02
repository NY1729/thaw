impl<'a> FnLowerer<'a> {
    fn collect_generator_for_array_spread(
        &mut self,
        generator: HirExpr,
        generator_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Function(params, result) = generator_type else {
            return Ok(None);
        };
        let [HirType::I64, HirType::Str, input, completion, request, forced] =
            params.as_slice()
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
            HirStmt::Let(output.clone(), array_type.clone(), HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(chunk.clone(), array_type.clone(), HirExpr::ArrayLit(Vec::new())),
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
        let [HirType::I64, HirType::Str, input, completion, request, forced] =
            params.as_slice()
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
            HirStmt::Let(output.clone(), array_type.clone(), HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(chunk.clone(), array_type.clone(), HirExpr::ArrayLit(Vec::new())),
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
            || !members.iter().any(|member| matches!(member, HirType::Promise(_)))
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
            self.wrap_call_argument_bindings(
                result,
                &[(source_name, array_type.clone(), array)],
            )?,
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
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(source_name.clone())))),
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
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(source_name.clone())))),
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

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
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
                ("source".to_string(), HirExpr::Lit(HirLit::Str(regex.exp.to_string()))),
                (
                    "flags".to_string(),
                    HirExpr::Lit(HirLit::Str(regex.flags.to_string())),
                ),
                ("lastIndex".to_string(), HirExpr::Lit(HirLit::F64(0.0))),
            ])),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
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
                        let result = self.interfaces.get(&name).cloned().ok_or_else(|| {
                            format!("class `{name}` has no layout")
                        })?;
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
                            Box::new(HirExpr::Var(
                                "__thaw_pending_exception_object".to_string(),
                            )),
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
                        let target = self.interfaces.get(class.sym.as_ref()).cloned();
                        if self.scope.get(&resolved) == Some(&HirType::Str) {
                            if let Some(target @ HirType::Object(_)) = target {
                                let value = if self.catch_bindings.contains(&resolved) {
                                    let object_name =
                                        format!("{resolved}__thaw_exception_object");
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
                    if self.catch_bindings.contains(&resolved)
                        || promise_catch
                    {
                        let target = lower_ts_type(
                            &assertion.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )?;
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
                self.lower_expr(&assertion.expr)
            }
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
                    let literal = match quasi.cooked.as_ref() {
                        Some(cooked) => {
                            hir_string_literal_from_wtf8(cooked.as_wtf8().as_bytes())
                        }
                        None => HirLit::Str(quasi.raw.to_string()),
                    };
                    if !matches!(&literal, HirLit::Str(text) if text.is_empty()) {
                        parts.push(HirExpr::Lit(literal));
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
                let mut cooked_strings = Vec::with_capacity(template.quasis.len());
                let mut raw_strings = Vec::with_capacity(template.quasis.len());
                for quasi in &template.quasis {
                    let cooked = match quasi.cooked.as_ref() {
                        Some(cooked) => {
                            hir_string_literal_from_wtf8(cooked.as_wtf8().as_bytes())
                        }
                        None => HirLit::Str(quasi.raw.to_string()),
                    };
                    cooked_strings.push(HirExpr::Lit(cooked));
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
                    HirExpr::Var(name) | HirExpr::FunctionRef(name, _, _) => Some(name.as_str()),
                    _ => None,
                };
                let args = if let Some(name) = tag_name {
                    match self.signatures.get(name).map(|signature| {
                        (signature.params.len(), signature.native_rest.clone())
                    }) {
                        // `signature.params` already counts the rest
                        // parameter's own packed-array slot as its last
                        // entry (matching `lower_call`'s own
                        // `param_types = signature.params.clone()` after
                        // packing) -- the number of *positional* arguments
                        // before that slot is one less.
                        Some((total_params, Some(element)))
                            if total_params > 0 && args.len() >= total_params - 1 =>
                        {
                            let positional = total_params - 1;
                            let mut args = args;
                            let trailing = args.split_off(positional);
                            let trailing = trailing
                                .into_iter()
                                .map(|value| self.coerce_to_declared(&element, value))
                                .collect::<Result<Vec<_>, _>>()?;
                            args.push(native_rest_array(trailing, &element));
                            args
                        }
                        _ => args,
                    }
                } else {
                    args
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
                let register = HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_template_strings_register".to_string())),
                    vec![HirExpr::Var(cooked_name.clone()), HirExpr::ArrayLit(raw_strings)],
                ));
                let body = HirExpr::Block(vec![register, HirStmt::Return(Some(call))]);
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
                        vec![HirParam { name: cooked_name, ty: array_type }],
                        call_type,
                        Box::new(body),
                    )),
                    vec![HirExpr::ArrayLit(cooked_strings)],
                ))
            }

            Expr::Bin(bin) => {
                if bin.op == BinaryOp::InstanceOf {
                    let Expr::Ident(class) = bin.right.as_ref() else {
                        return Err(
                            "native `instanceof` requires a class identifier on the right"
                                .into(),
                        );
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
                                return self.dynamic_value_check_by_name(
                                    class.sym.as_ref(),
                                    value,
                                );
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
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_is_date_shape".to_string(),
                                        )),
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
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_is_array".to_string(),
                                        )),
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
                            return self.dynamic_value_instanceof(
                                value,
                                HirExpr::Var(token),
                            );
                        }
                    }
                    if let HirType::Union(elements) = &value_type {
                        let matching = elements
                            .iter()
                            .enumerate()
                            .filter_map(|(index, element)| {
                                (if class.sym == *"Array" {
                                    native_builtin_instanceof_static_match("Array", element)
                                        .unwrap_or(false)
                                } else {
                                    class_type_has_identity(element, class.sym.as_ref())
                                })
                                .then_some(index)
                            })
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        let all_arrays = class.sym == *"Array"
                            && matching.len() == elements.len();
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
                                result = self.lower_logical_expr(result, check, false)?;
                            }
                        }
                        return self.wrap_call_argument_bindings(
                            result,
                            &[(name, value_type, value)],
                        );
                    }
                    // A caught exception has no real `Error` object or class
                    // hierarchy behind it once thrown -- just a string,
                    // optionally tagged with a class identity chain ahead
                    // of the message (see `new Error(...)`/
                    // `coerce_primitive_to_string` above) -- so testing one
                    // against an Error-family class checks the tagged (or
                    // defaulted) chain at runtime. A *not-yet-thrown* value
                    // of a real Error-derived class (still `HirType::Object`,
                    // e.g. right after `new MyError(...)`) instead falls
                    // through to the same compile-time class-identity check
                    // used for every other native class below.
                    if extends_error_family && value_type == HirType::Str {
                        let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), value_type.clone());
                        let call = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_is_instance".to_string())),
                            vec![
                                HirExpr::Var(name.clone()),
                                HirExpr::Lit(HirLit::Str(class.sym.to_string())),
                            ],
                        );
                        return self.wrap_call_argument_bindings(
                            call,
                            &[(name, value_type, value)],
                        );
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
                        return self.dynamic_value_check_by_name(class.sym.as_ref(), value);
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
                    let native_builtin_match =
                        native_builtin_instanceof_static_match(class.sym.as_ref(), &value_type);
                    if !extends_error_family
                        && native_builtin_match.is_none()
                        && !self
                            .signatures
                            .contains_key(&class_constructor_symbol(class.sym.as_ref()))
                    {
                        return Err(format!(
                            "native `instanceof` right operand `{}` is not a known class",
                            class.sym
                        ));
                    }
                    let result = native_builtin_match
                        .unwrap_or_else(|| class_type_has_identity(&value_type, class.sym.as_ref()));
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
                if matches!(bin.op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                    | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Exp
                    | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq
                    | BinaryOp::EqEqEq | BinaryOp::NotEqEq | BinaryOp::EqEq | BinaryOp::NotEq)
                {
                    lhs = self.lower_primitive_array_operand(lhs)?;
                    rhs = self.lower_primitive_array_operand(rhs)?;
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
                            if members.iter().all(|member| matches!(member, HirType::Array(_))) {
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
                            self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_has".into())),
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
                                    HirExpr::BinOp(BinOp::GtEq, Box::new(key.clone()), Box::new(HirExpr::Lit(HirLit::F64(0.0)))),
                                    HirExpr::BinOp(BinOp::Lt, Box::new(key.clone()), Box::new(HirExpr::ArrayLen(Box::new(array.clone())))),
                                    true,
                                )?;
                                let valid = self.lower_logical_expr(
                                    valid,
                                    HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(key.clone()),
                                        Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_math_trunc".into())), vec![key.clone()])),
                                    ),
                                    true,
                                )?;
                                self.lower_logical_expr(
                                    valid,
                                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_array_has_index".into())), vec![array, key]),
                                    true,
                                )?
                            };
                            self.wrap_call_argument_bindings(result, &[
                                (key_name, key_type, lhs),
                                (array_name, right_type, rhs),
                            ])?
                        } else if let HirType::Union(elements) = &right_type {
                            let HirExpr::Lit(HirLit::Str(property)) = lhs else {
                                return Err(
                                    "`in` on a union requires a string literal key".into()
                                );
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
                            self.wrap_call_argument_bindings(
                                result,
                                &[(name, right_type, rhs)],
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
                    BinaryOp::EqEqEq => {
                        if let Some(result) =
                            self.lower_mixed_bigint_equality(lhs.clone(), rhs.clone())?
                        {
                            result
                        } else {
                            let (lhs, rhs) = self.coerce_strict_equality_operands(lhs, rhs)?;
                            self.lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                                .unwrap_or(HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(lhs),
                                    Box::new(rhs),
                                ))
                        }
                    }
                    BinaryOp::NotEqEq => {
                        let equality = if let Some(result) =
                            self.lower_mixed_bigint_equality(lhs.clone(), rhs.clone())?
                        {
                            result
                        } else {
                            let (lhs, rhs) = self.coerce_strict_equality_operands(lhs, rhs)?;
                            self.lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                                .unwrap_or(HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(lhs),
                                    Box::new(rhs),
                                ))
                        };
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
                        if matches!(self.infer_expr_type(&lhs)?, HirType::Str)
                            || matches!(
                                self.infer_expr_type(&lhs)?,
                                HirType::Optional(payload) if payload.as_ref() == &HirType::Str
                            )
                            || matches!(self.infer_expr_type(&rhs)?, HirType::Str)
                            || matches!(
                                self.infer_expr_type(&rhs)?,
                                HirType::Optional(payload) if payload.as_ref() == &HirType::Str
                            ) =>
                    {
                        // Real JS's `+` always calls `ToPrimitive(operand)`
                        // with hint `"default"` for *both* operands, then
                        // decides string-concat purely from whichever
                        // result(s) are already strings -- it never passes
                        // hint `"string"` just because the *other* operand
                        // happens to be one (confirmed: a class returning a
                        // different value for `"string"` than `"default"`
                        // diverged from Node here before this fix).
                        // `add_operand_to_primitive` is a no-op for an
                        // already-`Str`/other-primitive operand, so this
                        // only changes behavior for an `Object`-typed side.
                        let (lhs, _) = self.add_operand_to_primitive(lhs)?;
                        let (rhs, _) = self.add_operand_to_primitive(rhs)?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![
                                self.coerce_primitive_to_string(lhs)?,
                                self.coerce_primitive_to_string(rhs)?,
                            ],
                        )
                    }
                    other
                        if (matches!(
                            self.infer_expr_type(&lhs)?,
                            HirType::JsValue | HirType::Json
                        )
                            && self.infer_expr_type(&rhs)? == HirType::F64)
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
                            vec![
                                callable,
                                empty_arguments,
                                HirExpr::ArrayLit(vec![lhs, rhs]),
                            ],
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
                            || matches!(self.infer_expr_type(&lhs)?, HirType::Nullable(_) | HirType::Nullish(_))
                            || matches!(self.infer_expr_type(&rhs)?, HirType::Nullable(_) | HirType::Nullish(_)) =>
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
                    if matches!(object_type, HirType::JsValue | HirType::Dynamic) {
                        let key = match &member.prop {
                            MemberProp::Ident(property) => {
                                HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                            }
                            MemberProp::Computed(computed) => self.lower_expr(&computed.expr)?,
                            MemberProp::PrivateName(_) => {
                                return Err("native `delete` does not support private properties".into())
                            }
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("deleteDynamicProperty".into())),
                            vec![object, key],
                        ));
                    }
                    let array_union = match &object_type {
                        HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))) => Some(members.clone()),
                        _ => None,
                    };
                    let (object, object_type) = if let Some(members) = array_union {
                        self.lower_union_array_sequence(object, &members)?
                    } else {
                        (object, object_type)
                    };
                    if matches!(object_type, HirType::Array(_)) {
                        let key = match &member.prop {
                            MemberProp::Ident(property) => HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                            MemberProp::Computed(computed) => {
                                let key = self.lower_expr(&computed.expr)?;
                                self.coerce_primitive_to_string(key)?
                            }
                            MemberProp::PrivateName(_) => {
                                return Err("native `delete` does not support private properties".into())
                            }
                        };
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_delete_strict".into())),
                            vec![object, key],
                        ));
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
                            return Err("native `delete` does not support private properties".into())
                        }
                    };
                    return Ok(HirExpr::JsonDelete(Box::new(object), Box::new(key)));
                }
                let value = self.lower_expr(&unary.arg)?;
                let value = if matches!(unary.op, UnaryOp::TypeOf | UnaryOp::Bang | UnaryOp::Plus | UnaryOp::Minus) {
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
                        let arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![digits]),
                        )?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
                            vec![constructor, arguments],
                        )
                    }
                    UnaryOp::Minus => {
                        let value = if matches!(self.infer_expr_type(&value)?, HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)) {
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
                        if matches!(self.infer_expr_type(&value)?, HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Object(_)) {
                            self.coerce_primitive_to_number(value)?
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
                                || self
                                    .promise_catch_parameter
                                    .as_deref()
                                    .is_some_and(|parameter| {
                                        self.resolve_binding(parameter) == *name
                                    });
                            if self.catch_bindings.contains(name)
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
                                    Box::new(HirExpr::Var(
                                        "__thaw_exception_typeof".to_string(),
                                    )),
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
                                    if self.scope.contains_key(name) {
                                        return None;
                                    }
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
                let optional_narrowing = self.optional_undefined_narrowing(&conditional.test);
                let union_narrowing = self.union_narrowing(&conditional.test);
                let branch_union = |truth: bool| {
                    union_narrowing.as_ref().map(|(targets, equal, complement)| {
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
                            (truth == *present_when_true).then(|| {
                                (name.clone(), payload.clone(), *absence_kind)
                            })
                        },
                    )
                };
                let consequent_union = branch_union(true);
                let alternate_union = branch_union(false);
                let consequent_optional = branch_optional(true);
                let alternate_optional = branch_optional(false);
                let test = self.lower_condition_expr(&conditional.test)?;
                let mut consequent = self.lower_expr_with_union_narrowing(
                    &conditional.cons,
                    consequent_union.as_deref(),
                    consequent_optional.as_ref(),
                )?;
                let mut alternate = self.lower_expr_with_union_narrowing(
                    &conditional.alt,
                    alternate_union.as_deref(),
                    alternate_optional.as_ref(),
                )?;
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
                Ok(HirExpr::Conditional(
                    Box::new(test),
                    Box::new(consequent),
                    Box::new(alternate),
                    result_type,
                ))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => self.lower_optional_member_read(member),
                OptChainBase::Call(call) => self.lower_optional_call(call),
            },

            Expr::Arrow(arrow) => self.lower_arrow(arrow),
            Expr::Fn(function) => self.lower_function_expression(function, None),

            Expr::Array(array_lit) => {
                if array_lit
                    .elems
                    .iter()
                    .all(|element| element.as_ref().is_none_or(|element| element.spread.is_none()))
                {
                    let values = array_lit
                        .elems
                        .iter()
                        .map(|element| match element {
                            None => Ok(HirExpr::Lit(HirLit::ArrayHole)),
                            Some(element) => self.lower_expr(&element.expr),
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
                // Phase 1 -- collect: lower every element/spread exactly
                // once (same order as source, so side effects are
                // unaffected), deferring the "what's the shared element
                // type" decision instead of fixing it from whichever
                // segment is seen first. This is what lets `[0, ...anyArr]`
                // (a concrete element *before* an `any[]` spread) resolve
                // the same way `[...anyArr, 0]` already does.
                enum ArrayLitSegment {
                    Hole,
                    Value(HirExpr, HirType),
                    Spread(HirExpr, HirType),
                }
                let mut segments = Vec::with_capacity(array_lit.elems.len());
                for element in &array_lit.elems {
                    let Some(element) = element else {
                        segments.push(ArrayLitSegment::Hole);
                        continue;
                    };
                    let mut value = self.lower_expr(&element.expr)?;
                    if element.spread.is_some() {
                        // A string/`Map`/`Set` spread source iterates the
                        // same way `for...of` already does for each --
                        // snapshot to an array up front via the exact same
                        // conversions that path uses, rather than requiring
                        // a typed array up front.
                        let mut spread_source_type = self.infer_expr_type(&value)?;
                        if let HirType::Union(members) = &spread_source_type {
                            if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                                let (flattened, flattened_type) =
                                    self.lower_union_array_sequence(value, members)?;
                                value = flattened;
                                spread_source_type = flattened_type;
                            }
                        }
                        if let Some((collected, _)) = self
                            .collect_generator_for_array_spread(value.clone(), &spread_source_type)?
                        {
                            value = collected;
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
                            let iterator = HirExpr::Call(
                                Box::new(HirExpr::PropAccess(
                                    Box::new(value.clone()),
                                    spread_source_type.clone(),
                                    "__thaw_symbol_iterator".to_string(),
                                )),
                                Vec::new(),
                            );
                            let iterator_type = self.infer_expr_type(&iterator)?;
                            let name = format!("__thaw_iterator_{}", self.next_binding);
                            self.next_binding += 1;
                            if let Some((producer, producer_type)) =
                                iterator_object_adapter(self, iterator, &iterator_type, name)
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
                        segments.push(ArrayLitSegment::Spread(value, spread_element.as_ref().clone()));
                    } else {
                        let actual = self.infer_expr_type(&value)?;
                        segments.push(ArrayLitSegment::Value(value, actual));
                    }
                }

                // Phase 2 -- resolve: fold every segment's type into one
                // shared `element_type`, order-independently. This is a
                // symmetric generalization of the tolerance rules the old
                // single-pass loop already applied (Optional-unwrap,
                // Union-containment, Json-escape-hatch) -- anything that
                // type-checked left-to-right before still resolves to the
                // identical type (that resolution is a special case of this
                // reduce), so this only widens acceptance, never narrows it.
                fn widen_array_element_type(existing: HirType, next: HirType) -> Result<HirType, String> {
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
                    Err(format!("array element type {next:?} does not match {existing:?}"))
                }
                let mut element_type: Option<HirType> = None;
                for segment in &segments {
                    let next = match segment {
                        ArrayLitSegment::Hole => continue,
                        ArrayLitSegment::Value(_, ty) | ArrayLitSegment::Spread(_, ty) => ty.clone(),
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
                                let values = std::mem::take(&mut pending).into_iter()
                                    .map(|item| if matches!(item, HirExpr::Lit(HirLit::ArrayHole)) {
                                        Ok(item)
                                    } else {
                                        self.coerce_array_insert_value(item, &element_type)
                                    })
                                    .collect::<Result<Vec<_>, String>>()?;
                                parts.push(self.lower_native_array_literal(values, element_type.clone())?);
                            }
                            if spread_element != element_type {
                                // Only reachable when a *later* segment
                                // forces a wider type than this spread's own
                                // source -- e.g. `[...floatArr, ...anyArr]`.
                                // Not reachable by any case the old
                                // left-to-right loop already accepted (it
                                // required exact equality at the point each
                                // spread was seen).
                                let parameter = format!(
                                    "__thaw_array_lit_spread_widen_{}",
                                    self.next_binding
                                );
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
                    let values = pending.into_iter()
                        .map(|item| if matches!(item, HirExpr::Lit(HirLit::ArrayHole)) {
                            Ok(item)
                        } else {
                            self.coerce_array_insert_value(item, &element_type)
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
                let value_type = self.infer_expr_type(&value)?;
                if value_type == HirType::JsValue {
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("resolveDynamicValue".into())),
                        vec![value],
                    ))
                } else if let HirType::Promise(resolved) = value_type {
                    if *resolved == HirType::Void {
                        Ok(HirExpr::Await(Box::new(value)))
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
                                        | "callDynamicValueWithValue"))
                    ) || matches!(
                        &value,
                        HirExpr::DynamicCall(signature, _)
                            if signature.backend == DynamicBackend::QuickJs
                    );
                    if resolves_at_dynamic_boundary {
                        Ok(value)
                    } else {
                        Ok(HirExpr::Await(Box::new(value)))
                    }
                }
            }

            Expr::New(new_expr) => {
                if let Expr::Ident(class) = new_expr.callee.as_ref() {
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
                        let values = self
                            .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
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
                        // Weak collections have the same native table layout,
                        // but distinct HIR types keep their non-enumerable API
                        // surface separate. Entries live until the request
                        // arena resets; there is no mid-request native GC.
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
                                    ))
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
                            let actual_type = self.infer_expr_type(&entries)?;
                            if !is_weak {
                                if let HirType::Union(members) = &actual_type {
                                    if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                                        let source_name = format!("__thaw_union_map_source_{}", self.next_binding);
                                        self.next_binding += 1;
                                        self.scope.insert(source_name.clone(), actual_type.clone());
                                        let mut branches = Vec::with_capacity(members.len());
                                        for (index, member) in members.iter().enumerate() {
                                            let HirType::Array(element) = member else { unreachable!() };
                                            let HirType::Tuple(pair) = element.as_ref() else {
                                                return Err(format!(
                                                    "Map constructor union member must contain [key, value] entries, got {element:?}"
                                                ));
                                            };
                                            let [inferred_key, inferred_value] = pair.as_slice() else {
                                                return Err(format!(
                                                    "Map constructor union member must contain [key, value] entries, got {element:?}"
                                                ));
                                            };
                                            let (key_type, value_type) = explicit_types.clone().unwrap_or_else(|| {
                                                (inferred_key.clone(), inferred_value.clone())
                                            });
                                            let pair_type = HirType::Tuple(vec![key_type.clone(), value_type.clone()]);
                                            if element.as_ref() != &pair_type {
                                                return Err(format!(
                                                    "Map constructor union member has entry type {element:?}, expected {pair_type:?}"
                                                ));
                                            }
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
                                                map_type,
                                                key_validator(&key_type)?,
                                                key_type,
                                                Some((value_type, pair_type)),
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
                            let (key_type, value_type) = if let Some(types) = explicit_types {
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
                            let pair_type =
                                HirType::Tuple(vec![key_type.clone(), value_type.clone()]);
                            let entries_type = HirType::Array(Box::new(pair_type.clone()));
                            if let Some((collected, _)) =
                                self.collect_generator_for_array_spread(entries.clone(), &actual_type)?
                            {
                                entries = collected;
                            }
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
                                ))
                            }
                        };
                        let Some(argument) = args.first() else {
                            let element_type = explicit_type
                                .expect("an untyped Set constructor has an argument");
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
                        let actual_type = self.infer_expr_type(&iterable)?;
                        if !is_weak {
                            if let HirType::Union(members) = &actual_type {
                                if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                                    let source_name = format!("__thaw_union_set_source_{}", self.next_binding);
                                    self.next_binding += 1;
                                    self.scope.insert(source_name.clone(), actual_type.clone());
                                    let mut branches = Vec::with_capacity(members.len());
                                    for (index, member) in members.iter().enumerate() {
                                        let HirType::Array(inferred_element) = member else { unreachable!() };
                                        let element_type = explicit_type
                                            .clone()
                                            .unwrap_or_else(|| inferred_element.as_ref().clone());
                                        if inferred_element.as_ref() != &element_type {
                                            return Err(format!(
                                                "Set constructor union member has element type {inferred_element:?}, expected {element_type:?}"
                                            ));
                                        }
                                        let set_type = HirType::Set(Box::new(element_type.clone()));
                                        branches.push(self.lower_map_or_set_from_iterable(
                                            HirExpr::UnionValue(
                                                Box::new(HirExpr::Var(source_name.clone())),
                                                index,
                                                members.clone(),
                                            ),
                                            member.clone(),
                                            set_type,
                                            key_validator(&element_type)?,
                                            element_type,
                                            None,
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
                        let element_type = if let Some(element_type) = explicit_type {
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
                        if let Some((collected, _)) = self
                            .collect_generator_for_array_spread(iterable.clone(), &actual_type)?
                        {
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
                            if self.infer_expr_type(&value)? == HirType::Str {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_date_parse".to_string())),
                                    vec![value],
                                )
                            } else {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_date_time_clip".to_string())),
                                    vec![self.coerce_primitive_to_number(value)?],
                                )
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
                    if class.sym == *"SuppressedError" {
                        // `new SuppressedError(error, suppressed, message?)`.
                        // The runtime owns the length-prefixed representation
                        // so nested SuppressedErrors remain unambiguous.
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(
                                "`new SuppressedError()` does not support spread arguments".into()
                            );
                        }
                        if args.len() < 2 || args.len() > 3 {
                            return Err(
                                "`new SuppressedError()` expects an error, a suppressed error, and an optional message"
                                    .into(),
                            );
                        }
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&args, "SuppressedError")?;
                        let error = arguments[0].clone();
                        let suppressed = arguments[1].clone();
                        let message = match arguments.get(2) {
                            Some(message) => self.coerce_primitive_to_string(message.clone())?,
                            None => HirExpr::Lit(HirLit::Str(String::new())),
                        };
                        let result = HirExpr::ObjectLit(vec![
                            (
                                "__thaw_class_identity_\u{1e}SuppressedError\u{1f}Error".to_string(),
                                HirExpr::Lit(HirLit::Bool(true)),
                            ),
                            ("message".to_string(), message),
                            (
                                "name".to_string(),
                                HirExpr::Lit(HirLit::Str("SuppressedError".to_string())),
                            ),
                            ("error".to_string(), error),
                            ("suppressed".to_string(), suppressed),
                        ]);
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
                        if args.is_empty() || args.len() > 3 {
                            return Err(
                                "`new AggregateError()` expects an errors argument and at most a message and options".into()
                            );
                        }
                        let errors = self.lower_expr(&args[0].expr)?;
                        let errors_type = self.infer_expr_type(&errors)?;
                        if !matches!(errors_type, HirType::Array(_) | HirType::Tuple(_)) {
                            return Err(
                                "`AggregateError` errors must be a statically typed array or tuple"
                                    .into(),
                            );
                        }
                        let errors_name = format!("__thaw_aggregate_errors_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(errors_name.clone(), errors_type.clone());
                        let mut bindings = vec![(errors_name.clone(), errors_type, errors)];

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
                            if !matches!(options_type, HirType::Object(_)) {
                                return Err("`AggregateError` options must be an object".into());
                            }
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
                        let message = match message {
                            Some(name) => {
                                self.coerce_primitive_to_string(HirExpr::Var(name))?
                            }
                            None => HirExpr::Lit(HirLit::Str(String::new())),
                        };
                        let mut fields = vec![
                            (
                                "__thaw_class_identity_\u{1e}AggregateError\u{1f}Error".to_string(),
                                HirExpr::Lit(HirLit::Bool(true)),
                            ),
                            ("message".to_string(), message),
                            (
                                "name".to_string(),
                                HirExpr::Lit(HirLit::Str("AggregateError".to_string())),
                            ),
                            ("errors".to_string(), HirExpr::Var(errors_name)),
                        ];
                        if let Some((name, HirType::Object(option_fields))) = options {
                            if option_fields.iter().any(|(field, _)| field == "cause") {
                                let options_type = HirType::Object(option_fields);
                                fields.push((
                                    "cause".to_string(),
                                    HirExpr::PropAccess(
                                        Box::new(HirExpr::Var(name)),
                                        options_type,
                                        "cause".to_string(),
                                    ),
                                ));
                            }
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::ObjectLit(fields),
                            &bindings,
                        );
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
                        // `throw` unwinds a single tagged string (`catch`
                        // binds it as `HirType::Str`, see
                        // `lower/statements/lowering.rs`) -- there is no
                        // `Error` object, stack trace, or support for a class
                        // extending one of these, but `new Error(message)`/
                        // `new TypeError(...)`/etc. tag that string with
                        // their class name ahead of a `\u{1}` marker and the
                        // message (see `thaw_runtime`'s `split_error_tag`),
                        // so `.message`/`.name`/`instanceof` can recover it
                        // at a catch site. A bare `throw "x"` (no `new`)
                        // still throws exactly that string, untagged, and
                        // every reader treats an untagged string as a
                        // default-named `Error` whose message is the whole
                        // string.
                        let args = new_expr.args.clone().unwrap_or_default();
                        if args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(format!(
                                "`new {}()` does not support spread arguments",
                                class.sym
                            ));
                        }
                        if args.len() > 2 {
                            return Err(format!(
                                "`new {}()` expects at most a message and options argument",
                                class.sym
                            ));
                        }
                        let message = match args.first() {
                            Some(argument) => {
                                let message = self.lower_expr(&argument.expr)?;
                                self.coerce_primitive_to_string(message)?
                            }
                            None => HirExpr::Lit(HirLit::Str(String::new())),
                        };
                        let tag = HirExpr::Lit(HirLit::Str(format!("\u{1}{}\u{1}", class.sym)));
                        let tagged = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![tag, message],
                        );
                        let Some(options) = args.get(1) else {
                            return Ok(tagged);
                        };
                        let options = self.lower_expr(&options.expr)?;
                        let options_type = self.infer_expr_type(&options)?;
                        let HirType::Object(fields) = &options_type else {
                            return Err("Error options must be an object with a `cause` field".into());
                        };
                        let cause_type = fields
                            .iter()
                            .find(|(name, _)| name == "cause")
                            .map(|(_, ty)| ty)
                            .ok_or("Error options must have a `cause` field")?;
                        let cause = HirExpr::PropAccess(
                            Box::new(options),
                            options_type.clone(),
                            "cause".to_string(),
                        );
                        let cause = match cause_type {
                            HirType::Str => cause,
                            _ => self.coerce_primitive_to_string(cause)?,
                        };
                        let with_marker = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![tagged, HirExpr::Lit(HirLit::Str("\u{2}".to_string()))],
                        );
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![with_marker, cause],
                        ));
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
                            let values = self
                                .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
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
                            return Err("`new WeakRef()` expects exactly one target argument".into());
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
                        self.scope.get(&name).is_some_and(|ty| {
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
                    if matches!(
                        self.infer_expr_type(&callee)?,
                        HirType::JsValue | HirType::Dynamic | HirType::Json
                    ) {
                        let args = new_expr.args.as_deref().unwrap_or_default();
                        let values = self.dynamic_call_args_json(args)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("constructDynamicValue".into())),
                            vec![callee, values],
                        ));
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
        if let Some(expected) = expected {
            let supplied = match expr {
                Expr::Arrow(arrow) => Some(arrow.params.len()),
                Expr::Fn(function) => Some(function.function.params.len()),
                Expr::Ident(ident) => {
                    let name = self.resolve_binding(ident.sym.as_ref());
                    (self
                        .scope
                        .get(&name)
                        .is_some_and(|ty| {
                            matches!(ty, HirType::Function(_, _) | HirType::CallableFunction(..))
                        })
                        || self.generic_arrows.contains_key(&name)
                        || self.generic_named_templates.contains_key(&name)
                        || self.signatures.contains_key(&name))
                    .then_some(usize::MAX)
                }
                _ => None,
            };
            if let Some((params, ret)) = supplied
                .and_then(|supplied| callback_signature(expected, supplied))
            {
                return self.lower_promise_callback(expr, &params, Some(&ret));
            }
        }
        if let (Expr::Object(object), Some(expected)) = (expr, expected) {
            let fields = match expected {
                HirType::Object(fields) => Some(fields.as_slice()),
                HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload) => match payload.as_ref() {
                    HirType::Object(fields) => Some(fields.as_slice()),
                    _ => None,
                },
                // A `T | T[]` (or `T | undefined`) parameter -- real:
                // hapi's `route(route: ServerRoute | ServerRoute[])` --
                // contextually types an object-literal argument against
                // its single object member, without which the nested
                // `handler` callback's parameters get no type and lowering
                // fails outright. Restricted to exactly one object member:
                // a discriminated union of several object shapes
                // (`{ text: string } | { count: number }`) must keep
                // matching each literal against its own member instead.
                HirType::Union(elements) => {
                    let mut objects = elements.iter().filter_map(|element| match element {
                        HirType::Object(fields) => Some(fields.as_slice()),
                        _ => None,
                    });
                    match (objects.next(), objects.next()) {
                        (Some(fields), None) => Some(fields),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(fields) = fields {
                return self.lower_object_lit(object, Some(fields));
            }
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
fn native_builtin_instanceof_static_match(class: &str, ty: &HirType) -> Option<bool> {
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
