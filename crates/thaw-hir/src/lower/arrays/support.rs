impl<'a> FnLowerer<'a> {
    /// `Math.f` used as a function value: `(x: number) => Math.f(x)` (variadic
    /// `max`/`min`/`hypot` take `...args: number[]`).
    fn math_function_value(name: &str) -> Option<Expr> {
        use swc_ecma_ast::{
            ArrowExpr, ArrowFunctionBody, BindingIdent, ExprOrSpread, Ident, RestPat, TsArrayType,
            TsKeywordType, TsKeywordTypeKind, TsType, TsTypeAnn,
        };
        let sp = swc_common::DUMMY_SP;
        let number = || {
            Box::new(TsType::TsKeywordType(TsKeywordType {
                span: sp,
                kind: TsKeywordTypeKind::TsNumberKeyword,
            }))
        };
        let annotate = |ty: Box<TsType>| Some(Box::new(TsTypeAnn { span: sp, type_ann: ty }));
        let param = |id: &str, ty: Box<TsType>| {
            Pat::Ident(BindingIdent { id: Ident::new_no_ctxt(id.into(), sp), type_ann: annotate(ty) })
        };
        let arg = |id: &str, spread: bool| ExprOrSpread {
            spread: spread.then_some(sp),
            expr: Box::new(Expr::Ident(Ident::new_no_ctxt(id.into(), sp))),
        };
        let (params, args) = match name {
            "max" | "min" | "hypot" => (
                vec![Pat::Rest(RestPat {
                    span: sp,
                    dot3_token: sp,
                    arg: Box::new(Pat::Ident(BindingIdent {
                        id: Ident::new_no_ctxt("a".into(), sp),
                        type_ann: None,
                    })),
                    type_ann: annotate(Box::new(TsType::TsArrayType(TsArrayType {
                        span: sp,
                        elem_type: number(),
                    }))),
                })],
                vec![arg("a", true)],
            ),
            "pow" | "atan2" | "imul" => (
                vec![param("a", number()), param("b", number())],
                vec![arg("a", false), arg("b", false)],
            ),
            "abs" | "ceil" | "floor" | "round" | "trunc" | "sqrt" | "cbrt" | "sin" | "cos" | "tan"
            | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "expm1" | "log" | "log2"
            | "log10" | "log1p" | "sign" | "fround" | "clz32" => {
                (vec![param("a", number())], vec![arg("a", false)])
            }
            _ => return None,
        };
        let body = Expr::Call(CallExpr {
            span: sp,
            ctxt: Default::default(),
            callee: Callee::Expr(Box::new(Expr::Member(swc_ecma_ast::MemberExpr {
                span: sp,
                obj: Box::new(Expr::Ident(Ident::new_no_ctxt("Math".into(), sp))),
                prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(name.into(), sp)),
            }))),
            args,
            type_args: None,
        });
        Some(Expr::Arrow(ArrowExpr {
            span: sp,
            ctxt: Default::default(),
            params,
            body: Box::new(ArrowFunctionBody::Expr(Box::new(body))),
            is_async: false,
            is_generator: false,
            type_params: None,
            return_type: None,
        }))
    }

    /// `new Array(...)` / `Array(...)` (no other callee): its arguments and type arguments.
    fn array_constructor_parts(
        expr: &Expr,
    ) -> Option<(&[swc_ecma_ast::ExprOrSpread], Option<&swc_ecma_ast::TsTypeParamInstantiation>)>
    {
        match expr {
            Expr::New(new_expr) => match new_expr.callee.as_ref() {
                Expr::Ident(class) if class.sym == *"Array" => Some((
                    new_expr.args.as_deref().unwrap_or_default(),
                    new_expr.type_args.as_deref(),
                )),
                _ => None,
            },
            Expr::Call(call) => match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(class) if class.sym == *"Array" => {
                        Some((call.args.as_slice(), call.type_args.as_deref()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// `new Array(n)` is an array of `n` holes; `new Array(a, b)` is `[a, b]`. The
    /// element type is the explicit type argument, else `element_hint`, else `any`.
    fn lower_array_constructor(
        &mut self,
        args: &[swc_ecma_ast::ExprOrSpread],
        type_args: Option<&swc_ecma_ast::TsTypeParamInstantiation>,
        element_hint: Option<HirType>,
    ) -> Result<HirExpr, String> {
        if args.iter().any(|argument| argument.spread.is_some()) {
            return Err("`Array(...)` does not support spread arguments".into());
        }
        let element = match type_args.map(|type_args| type_args.params.as_slice()) {
            Some([element]) => lower_ts_type(element, self.interfaces, self.generic_interfaces)?,
            Some(_) => return Err("`Array` expects one type argument".into()),
            None => element_hint.unwrap_or(HirType::Json),
        };
        let length = match args {
            [single] => {
                let length = self.lower_expr(&single.expr)?;
                if self.infer_expr_type(&length)? == HirType::F64 {
                    Some(length)
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(length) = length else {
            // A single non-number argument or several arguments are the elements.
            let literal = Expr::Array(swc_ecma_ast::ArrayLit {
                span: swc_common::DUMMY_SP,
                elems: args.iter().cloned().map(Some).collect(),
            });
            return self.lower_expr(&literal);
        };
        let array_type = HirType::Array(Box::new(element.clone()));
        let name = format!("__thaw_array_ctor_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), array_type.clone());
        let length_name = format!("__thaw_array_ctor_len_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        let len = || HirExpr::Var(length_name.clone());
        let invalid = || HirStmt::Throw(HirExpr::Lit(HirLit::Str(
            "\u{1}RangeError\u{1}Invalid array length".into(),
        )));
        let body = HirExpr::Block(vec![
            HirStmt::If(
                HirExpr::BinOp(BinOp::Lt, Box::new(len()), Box::new(HirExpr::Lit(HirLit::F64(0.0)))),
                vec![invalid()], Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(BinOp::GtEq, Box::new(len()), Box::new(HirExpr::Lit(HirLit::F64(4294967296.0)))),
                vec![invalid()], Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(BinOp::EqEqEq, Box::new(len()), Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())), vec![len()],
                ))),
                Vec::new(), vec![invalid()],
            ),
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_resize".into())),
                vec![HirExpr::Var(name.clone()), len()],
            )),
            HirStmt::Return(Some(HirExpr::Var(name.clone()))),
        ]);
        let empty = HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element);
        self.wrap_call_argument_bindings(body, &[(name, array_type, empty), (length_name, HirType::F64, length)])
    }

    /// `new Array(n).fill(v)` with no type argument: the element type is `v`'s.
    fn array_constructor_fill_receiver(&mut self, call: &CallExpr) -> Option<Expr> {
        let Callee::Expr(callee) = &call.callee else { return None };
        let Expr::Member(member) = callee.as_ref() else { return None };
        if !matches!(&member.prop, MemberProp::Ident(p) if p.sym == *"fill") {
            return None;
        }
        let (_, type_args) = Self::array_constructor_parts(&member.obj)?;
        let value = call.args.first()?;
        if type_args.is_some() || value.spread.is_some() {
            return None;
        }
        let kind = match self.lower_expr(&value.expr).ok().and_then(|v| self.infer_expr_type(&v).ok())? {
            HirType::F64 => swc_ecma_ast::TsKeywordTypeKind::TsNumberKeyword,
            HirType::Str | HirType::StrLiteral(_) => swc_ecma_ast::TsKeywordTypeKind::TsStringKeyword,
            HirType::Bool => swc_ecma_ast::TsKeywordTypeKind::TsBooleanKeyword,
            _ => return None,
        };
        let instantiation = Box::new(swc_ecma_ast::TsTypeParamInstantiation {
            span: swc_common::DUMMY_SP,
            params: vec![Box::new(swc_ecma_ast::TsType::TsKeywordType(
                swc_ecma_ast::TsKeywordType { span: swc_common::DUMMY_SP, kind },
            ))],
        });
        let mut receiver = member.obj.as_ref().clone();
        match &mut receiver {
            Expr::New(new_expr) => new_expr.type_args = Some(instantiation),
            Expr::Call(inner) => inner.type_args = Some(instantiation),
            _ => return None,
        }
        Some(receiver)
    }

    fn validate_array_callback_value(
        &self,
        callback: HirExpr,
        available: &[HirType],
        expected_return: Option<&HirType>,
        label: &str,
    ) -> Result<HirExpr, String> {
        let (params, ret) = match self.infer_expr_type(&callback)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(params, _, None, ret) => (params, ret),
            _ => return Err(format!("{label} callback is not a function value")),
        };
        if params.len() > available.len() || params != available[..params.len()] {
            return Err(format!(
                "{label} callback has parameters {params:?}, expected a prefix of {available:?}"
            ));
        }
        if expected_return.is_some_and(|expected| ret.as_ref() != expected) {
            return Err(format!(
                "{label} callback returns {ret:?}, expected {:?}",
                expected_return.unwrap()
            ));
        }
        Ok(callback)
    }

    fn lower_spread_promise_combinator(
        &mut self,
        value: HirExpr,
        combinator: &str,
    ) -> Result<HirExpr, String> {
        let value = self.project_native_base(value)?;
        match self.infer_expr_type(&value)? {
            HirType::Array(element) => {
                let HirType::Promise(resolved) = *element else {
                    return Err(format!(
                        "`Promise.{combinator}` expects an array of promises"
                    ));
                };
                if *resolved == HirType::Void && combinator != "all" {
                    return Err(format!(
                        "`Promise.{combinator}` elements must not resolve to void"
                    ));
                }
                Ok(match combinator {
                    "all" => HirExpr::PromiseAllArray(Box::new(value), *resolved),
                    "allSettled" => {
                        HirExpr::PromiseAllSettledArray(Box::new(value), *resolved)
                    }
                    "race" => HirExpr::PromiseRaceArray(Box::new(value), *resolved),
                    "any" => HirExpr::PromiseAnyArray(Box::new(value), *resolved),
                    _ => unreachable!(),
                })
            }
            HirType::Union(members) => {
                let mut resolved = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    let HirType::Array(element) = member else {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} is not an array: {member:?}"
                        ));
                    };
                    let HirType::Promise(payload) = element.as_ref() else {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} is not an array of promises: {member:?}"
                        ));
                    };
                    if payload.as_ref() == &HirType::Void {
                        return Err(format!(
                            "`Promise.{combinator}` union member {index} resolves to void"
                        ));
                    }
                    resolved.push(payload.as_ref().clone());
                }

                let branch_results = resolved
                    .iter()
                    .map(|payload| match combinator {
                        "all" => HirType::Array(Box::new(payload.clone())),
                        "allSettled" => HirType::Array(Box::new(
                            promise_settled_result_type(payload.clone()),
                        )),
                        "race" | "any" => payload.clone(),
                        _ => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                let mut result_members = Vec::new();
                for result in &branch_results {
                    let member = match result {
                        HirType::Array(element) => element.as_ref(),
                        other => other,
                    };
                    Self::flatten_property_union_members(member, &mut result_members)?;
                }
                let common_result = match result_members.as_slice() {
                    [single] => single.clone(),
                    _ => HirType::Union(result_members),
                };
                let result_type = match combinator {
                    "all" | "allSettled" => HirType::Array(Box::new(common_result)),
                    "race" | "any" => common_result,
                    _ => unreachable!(),
                };
                let source_type = HirType::Union(members.clone());
                let source_name = format!("__thaw_promise_union_array_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), source_type.clone());

                let mut branches = Vec::with_capacity(members.len());
                for (index, (payload, branch_result)) in resolved
                    .into_iter()
                    .zip(branch_results)
                    .enumerate()
                {
                    let array = HirExpr::UnionValue(
                        Box::new(HirExpr::Var(source_name.clone())),
                        index,
                        members.clone(),
                    );
                    let promise = match combinator {
                        "all" => HirExpr::PromiseAllArray(Box::new(array), payload),
                        "allSettled" => {
                            HirExpr::PromiseAllSettledArray(Box::new(array), payload)
                        }
                        "race" => HirExpr::PromiseRaceArray(Box::new(array), payload),
                        "any" => HirExpr::PromiseAnyArray(Box::new(array), payload),
                        _ => unreachable!(),
                    };
                    // Widen fulfilled values after the exact member combinator
                    // runs so its promise layout and rejection path stay intact.
                    if branch_result == result_type {
                        branches.push(promise);
                        continue;
                    }
                    let result_name = format!("__thaw_promise_union_result_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(result_name.clone(), branch_result.clone());
                    let widened = match (&branch_result, &result_type) {
                        (HirType::Array(source), HirType::Array(target)) => {
                            let element_name = format!(
                                "__thaw_promise_union_element_{}",
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope
                                .insert(element_name.clone(), source.as_ref().clone());
                            let converted = self.coerce_to_declared(
                                target,
                                HirExpr::Var(element_name.clone()),
                            )?;
                            self.lower_array_map(
                                HirExpr::Var(result_name.clone()),
                                branch_result.clone(),
                                source.as_ref().clone(),
                                source.as_ref().clone(),
                                HirExpr::Lambda(
                                    Vec::new(),
                                    vec![HirParam {
                                        name: element_name,
                                        ty: source.as_ref().clone(),
                                    }],
                                    target.as_ref().clone(),
                                    Box::new(converted),
                                ),
                                None,
                            )?
                        }
                        _ => self.coerce_to_declared(
                            &result_type,
                            HirExpr::Var(result_name.clone()),
                        )?,
                    };
                    branches.push(HirExpr::PromiseThen(
                        Box::new(promise),
                        Box::new(HirExpr::Lambda(
                            Vec::new(),
                            vec![HirParam {
                                name: result_name,
                                ty: branch_result.clone(),
                            }],
                            result_type.clone(),
                            Box::new(widened),
                        )),
                        branch_result,
                        result_type.clone(),
                        false,
                        false,
                    ));
                }
                let result = self.merge_union_array_method_branches(
                    &source_name,
                    &members,
                    branches,
                )?;
                self.wrap_call_argument_bindings(result, &[(source_name, source_type, value)])
            }
            HirType::Tuple(elements) => {
                let source_type = HirType::Tuple(elements.clone());
                let source_name = format!("__thaw_promise_tuple_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), source_type.clone());
                let mut resolved = Vec::with_capacity(elements.len());
                let mut promises = Vec::with_capacity(elements.len());
                for (index, element) in elements.into_iter().enumerate() {
                    let HirType::Promise(payload) = &element else {
                        return Err(format!(
                            "Promise.{combinator} element {index} must be a Promise, got {element:?}"
                        ));
                    };
                    if payload.as_ref() == &HirType::Void {
                        return Err(format!(
                            "Promise.{combinator} element {index} resolves to void"
                        ));
                    }
                    resolved.push(payload.as_ref().clone());
                    promises.push(HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(source_name.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element,
                    ));
                }
                let first = resolved.first().cloned().unwrap_or(HirType::F64);
                if combinator == "all" && resolved.iter().any(|element| element != &first) {
                    return self.wrap_call_argument_bindings(
                        HirExpr::PromiseAllTuple(promises, resolved),
                        &[(source_name, source_type, value)],
                    );
                }
                if let Some((index, actual)) = resolved
                    .iter()
                    .enumerate()
                    .find(|(_, element)| *element != &first)
                {
                    return Err(format!(
                        "Promise.{combinator} element {index} resolves to {actual:?}, expected {first:?}"
                    ));
                }
                let result = match combinator {
                    "all" => HirExpr::PromiseAll(promises, first),
                    "allSettled" => HirExpr::PromiseAllSettled(promises, first),
                    "race" => HirExpr::PromiseRace(promises, first),
                    "any" => HirExpr::PromiseAny(promises, first),
                    _ => unreachable!(),
                };
                self.wrap_call_argument_bindings(result, &[(source_name, source_type, value)])
            }
            other => Err(format!(
                "`Promise.{combinator}` expects an array of promises, got {other:?}"
            )),
        }
    }

    fn lower_parse_call(&mut self, call: &CallExpr, parse_int: bool) -> Result<HirExpr, String> {
        let label = if parse_int { "parseInt" } else { "parseFloat" };
        let expected = if parse_int { 1..=2 } else { 1..=1 };
        let (arguments, bindings) = self.lower_native_spread_values(&call.args, label)?;
        if !expected.contains(&arguments.len()) {
            return Err(format!(
                "`{label}` expects one{} argument",
                if parse_int { " or two" } else { "" }
            ));
        }
        let text = arguments[0].clone();
        let text = self.coerce_primitive_to_string(text)?;
        let mut lowered = vec![text];
        if parse_int {
            let radix = if let Some(argument) = arguments.get(1) {
                self.coerce_primitive_to_number(argument.clone())?
            } else {
                HirExpr::Lit(HirLit::F64(0.0))
            };
            lowered.push(radix);
        }
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(
                if parse_int {
                    "__thaw_parse_int"
                } else {
                    "__thaw_parse_float"
                }
                .to_string(),
            )),
            lowered,
        );
        self.wrap_call_argument_bindings(call, &bindings)
    }

    fn lower_array_sort_default(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        copy: bool,
    ) -> Result<HirExpr, String> {
        if matches!(element_type, HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Undefined | HirType::Null | HirType::Union(_) | HirType::Json) {
            return self.lower_array_sort_default_tagged(receiver, array_type, element_type, copy);
        }
        let prefix = match element_type {
            HirType::F64 => "number",
            HirType::BigInt => "bigint",
            HirType::Str => "string",
            HirType::Bool => "bool",
            HirType::Object(_) => "object",
            other => return Err(format!("default array sort does not support element type {other:?}")),
        };
        let suffix = if copy { "to_sorted" } else { "sort" };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
            vec![receiver],
        ))
    }

    // ponytail: reuses the existing O(n²) comparator sort; add a native tagged sorter if large arrays matter.
    fn lower_array_sort_default_tagged(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        copy: bool,
    ) -> Result<HirExpr, String> {
        let left = format!("__thaw_sort_default_left_{}", self.next_binding);
        self.next_binding += 1;
        let right = format!("__thaw_sort_default_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(left.clone(), element_type.clone());
        self.scope.insert(right.clone(), element_type.clone());
        let left_string = self.coerce_primitive_to_string(HirExpr::Var(left.clone()))?;
        let right_string = self.coerce_primitive_to_string(HirExpr::Var(right.clone()))?;
        // Spec default order: -1 / 0 / 1 over the String() forms (each converted once).
        let left_text = format!("__thaw_sort_default_left_text_{}", self.next_binding);
        self.next_binding += 1;
        let right_text = format!("__thaw_sort_default_right_text_{}", self.next_binding);
        self.next_binding += 1;
        let text_call = |name: &str, a: &str, b: &str| {
            HirExpr::Call(
                Box::new(HirExpr::Var(name.into())),
                vec![HirExpr::Var(a.to_string()), HirExpr::Var(b.to_string())],
            )
        };
        let number = |value: f64| HirExpr::Lit(HirLit::F64(value));
        let comparator = HirExpr::Lambda(
            Vec::new(),
            vec![
                HirParam { name: left, ty: element_type.clone() },
                HirParam { name: right, ty: element_type.clone() },
            ],
            HirType::F64,
            Box::new(HirExpr::Block(vec![
                HirStmt::Let(left_text.clone(), HirType::Str, left_string),
                HirStmt::Let(right_text.clone(), HirType::Str, right_string),
                HirStmt::Return(Some(HirExpr::Conditional(
                    Box::new(text_call("__thaw_string_gt", &left_text, &right_text)),
                    Box::new(number(1.0)),
                    Box::new(HirExpr::Conditional(
                        Box::new(text_call("__thaw_string_lt", &left_text, &right_text)),
                        Box::new(number(-1.0)),
                        Box::new(number(0.0)),
                        HirType::F64,
                    )),
                    HirType::F64,
                ))),
            ])),
        );
        self.lower_array_sort_comparator(
            receiver,
            array_type,
            element_type.clone(),
            element_type,
            comparator,
            copy,
            None,
        )
    }

    /// `(x, y) => x - y` is ascending, `(x, y) => y - x` descending; anything else is None.
    pub(super) fn numeric_comparator_direction(argument: &Expr) -> Option<bool> {
        let Expr::Arrow(arrow) = argument else { return None };
        let [left, right] = arrow.params.as_slice() else { return None };
        let (Pat::Ident(left), Pat::Ident(right)) = (&left, &right) else { return None };
        let (left, right) = (left.id.sym.as_ref(), right.id.sym.as_ref());
        let swc_ecma_ast::ArrowFunctionBody::Expr(body) = arrow.body.as_ref() else { return None };
        let Expr::Bin(swc_ecma_ast::BinExpr { op: swc_ecma_ast::BinaryOp::Sub, left: a, right: b, .. }) = body.as_ref() else {
            return None;
        };
        let (Expr::Ident(a), Expr::Ident(b)) = (a.as_ref(), b.as_ref()) else { return None };
        let (a, b) = (a.sym.as_ref(), b.sym.as_ref());
        if left == right {
            None
        } else if (a, b) == (left, right) {
            Some(false)
        } else if (a, b) == (right, left) {
            Some(true)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_array_sort_comparator(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        comparator_element_type: HirType,
        comparator: HirExpr,
        copy: bool,
        native_descending: Option<bool>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_sort_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_name = format!("__thaw_sort_comparator_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_type = HirType::Function(
            vec![
                comparator_element_type.clone(),
                comparator_element_type.clone(),
            ],
            Box::new(HirType::F64),
        );
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(comparator_name.clone(), comparator_type.clone());

        let array_name = format!("__thaw_sort_array_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_sort_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_name = format!("__thaw_sort_outer_{}", self.next_binding);
        self.next_binding += 1;
        let inner_name = format!("__thaw_sort_inner_{}", self.next_binding);
        self.next_binding += 1;
        let left_name = format!("__thaw_sort_left_{}", self.next_binding);
        self.next_binding += 1;
        let right_name = format!("__thaw_sort_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(outer_name.clone(), HirType::F64);
        self.scope.insert(inner_name.clone(), HirType::F64);
        self.scope.insert(left_name.clone(), element_type.clone());
        self.scope.insert(right_name.clone(), element_type.clone());

        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let variable = |name: &str| HirExpr::Var(name.to_string());
        let add_one =
            |value: HirExpr| HirExpr::BinOp(BinOp::Add, Box::new(value), Box::new(number(1.0)));
        let working_source = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_slice".into())),
            vec![variable(&receiver_name), number(0.0), number(f64::INFINITY)],
        );
        // Stable bottom-up merge sort (O(n log n) comparator calls) over the
        // compacted working copy: runs of `width` are merged into `tmp` and
        // copied back. The comparator sees (earlier, later) pairs and the
        // later element wins only when it returns > 0, as in V8.
        let compare = HirExpr::Call(
            Box::new(variable(&comparator_name)),
            vec![
                self.coerce_to_declared(&comparator_element_type, variable(&left_name))?,
                self.coerce_to_declared(&comparator_element_type, variable(&right_name))?,
            ],
        );
        let should_swap = HirExpr::BinOp(BinOp::Gt, Box::new(compare), Box::new(number(0.0)));
        let tmp_name = format!("__thaw_sort_tmp_{}", self.next_binding);
        self.next_binding += 1;
        let names: Vec<String> = ["width", "lo", "mid", "hi", "k", "take", "copy"]
            .iter()
            .map(|label| {
                self.next_binding += 1;
                format!("__thaw_sort_{label}_{}", self.next_binding)
            })
            .collect();
        let [width, lo, mid, hi, k, take, copy_index] = &names[..] else {
            unreachable!()
        };
        self.scope.insert(tmp_name.clone(), array_type.clone());
        for name in &names {
            self.scope.insert(name.clone(), HirType::F64);
        }
        // `inner_name` = i (left cursor), `outer_name` is unused by the merge.
        let i = &inner_name;
        let j = format!("__thaw_sort_j_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(j.clone(), HirType::F64);
        let elem = |array: &str, index: &str| {
            HirExpr::TypedIndex(
                Box::new(variable(array)),
                Box::new(variable(index)),
                element_type.clone(),
            )
        };
        let assign = |name: &str, value: HirExpr| HirStmt::Expr(HirExpr::Assign(name.to_string(), Box::new(value)));
        let lt = |l: HirExpr, r: HirExpr| HirExpr::BinOp(BinOp::Lt, Box::new(l), Box::new(r));
        let clamp_to_length = |name: &str| {
            HirStmt::If(
                HirExpr::BinOp(BinOp::Gt, Box::new(variable(name)), Box::new(variable(&length_name))),
                vec![assign(name, variable(&length_name))],
                Vec::new(),
            )
        };
        let merge_step = vec![
            assign(take, number(0.0)),
            HirStmt::If(
                lt(variable(i), variable(mid)),
                vec![HirStmt::If(
                    lt(variable(&j), variable(hi)),
                    vec![
                        HirStmt::Let(left_name.clone(), element_type.clone(), elem(&array_name, i)),
                        HirStmt::Let(right_name.clone(), element_type.clone(), elem(&array_name, &j)),
                        HirStmt::If(should_swap, Vec::new(), vec![assign(take, number(1.0))]),
                    ],
                    vec![assign(take, number(1.0))],
                )],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(BinOp::EqEqEq, Box::new(variable(take)), Box::new(number(1.0))),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(variable(&tmp_name)),
                        Box::new(variable(k)),
                        Box::new(elem(&array_name, i)),
                    )),
                    assign(i, add_one(variable(i))),
                ],
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(variable(&tmp_name)),
                        Box::new(variable(k)),
                        Box::new(elem(&array_name, &j)),
                    )),
                    assign(&j, add_one(variable(&j))),
                ],
            ),
            assign(k, add_one(variable(k))),
        ];
        let merge_run = vec![
            assign(mid, HirExpr::BinOp(BinOp::Add, Box::new(variable(lo)), Box::new(variable(width)))),
            clamp_to_length(mid),
            assign(
                hi,
                HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(variable(lo)),
                    Box::new(HirExpr::BinOp(BinOp::Mul, Box::new(variable(width)), Box::new(number(2.0)))),
                ),
            ),
            clamp_to_length(hi),
            assign(i, variable(lo)),
            assign(&j, variable(mid)),
            assign(k, variable(lo)),
            HirStmt::While(lt(variable(k), variable(hi)), merge_step),
            assign(
                lo,
                HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(variable(lo)),
                    Box::new(HirExpr::BinOp(BinOp::Mul, Box::new(variable(width)), Box::new(number(2.0)))),
                ),
            ),
        ];
        let copy_back = vec![
            assign(copy_index, number(0.0)),
            HirStmt::While(
                lt(variable(copy_index), variable(&length_name)),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(variable(&array_name)),
                        Box::new(variable(copy_index)),
                        Box::new(elem(&tmp_name, copy_index)),
                    )),
                    assign(copy_index, add_one(variable(copy_index))),
                ],
            ),
        ];
        // Arrays shorter than 64 are one V8 TimSort run: `CountAndMakeRun` (a descending prefix is
        // reversed) and a binary insertion sort of the rest, so an inconsistent comparator sees
        // and produces exactly V8's calls and order. Longer arrays use the merge sort below.
        let small_run = {
            let mut names = Vec::new();
            for label in ["run", "idx", "order", "stop", "lo2", "hi2", "start", "left", "right", "mid", "p", "desc"] {
                self.next_binding += 1;
                let name = format!("__thaw_ts_{label}_{}", self.next_binding);
                self.scope.insert(name.clone(), HirType::F64);
                names.push(name);
            }
            let [run, idx, order, stop, lo2, hi2, start, left, right, mid_i, p, desc] = &names[..] else {
                unreachable!()
            };
            let mut elements = Vec::new();
            for label in ["prev", "cur", "tmp", "pivot"] {
                self.next_binding += 1;
                let name = format!("__thaw_ts_{label}_{}", self.next_binding);
                self.scope.insert(name.clone(), element_type.clone());
                elements.push(name);
            }
            let [prev, cur, tmp, pivot] = &elements[..] else { unreachable!() };
            let call = |this: &mut Self, a: HirExpr, b: HirExpr| -> Result<HirExpr, String> {
                Ok(HirExpr::Call(
                    Box::new(variable(&comparator_name)),
                    vec![
                        this.coerce_to_declared(&comparator_element_type, a)?,
                        this.coerce_to_declared(&comparator_element_type, b)?,
                    ],
                ))
            };
            let sub_one = |value: HirExpr| HirExpr::BinOp(BinOp::Sub, Box::new(value), Box::new(number(1.0)));
            let gt = |l: HirExpr, r: HirExpr| HirExpr::BinOp(BinOp::Gt, Box::new(l), Box::new(r));
            let ge = |l: HirExpr, r: HirExpr| HirExpr::BinOp(BinOp::GtEq, Box::new(l), Box::new(r));
            let eq = |l: HirExpr, r: HirExpr| HirExpr::BinOp(BinOp::EqEqEq, Box::new(l), Box::new(r));
            let at = |index: HirExpr| HirExpr::TypedIndex(Box::new(variable(&array_name)), Box::new(index), element_type.clone());
            let store = |index: HirExpr, value: HirExpr| {
                HirStmt::Expr(HirExpr::IndexAssign(Box::new(variable(&array_name)), Box::new(index), Box::new(value)))
            };
            // CountAndMakeRun
            let first = call(self, at(number(1.0)), at(number(0.0)))?;
            let in_run = call(self, variable(cur), variable(prev))?;
            let count_run = vec![
                assign(run, number(2.0)),
                assign(order, first),
                assign(desc, HirExpr::Conditional(
                    Box::new(lt(variable(order), number(0.0))),
                    Box::new(number(1.0)), Box::new(number(0.0)), HirType::F64,
                )),
                HirStmt::Let(prev.clone(), element_type.clone(), at(number(1.0))),
                assign(idx, number(2.0)),
                HirStmt::While(lt(variable(idx), variable(&length_name)), vec![
                    HirStmt::Let(cur.clone(), element_type.clone(), at(variable(idx))),
                    assign(order, in_run),
                    assign(stop, HirExpr::Conditional(
                        Box::new(eq(variable(desc), number(1.0))),
                        // V8 maps a NaN comparator result to +0, so `order >= 0` is `!(order < 0)`.
                        Box::new(HirExpr::Conditional(
                            Box::new(lt(variable(order), number(0.0))),
                            Box::new(number(0.0)), Box::new(number(1.0)), HirType::F64,
                        )),
                        Box::new(HirExpr::Conditional(
                            Box::new(lt(variable(order), number(0.0))),
                            Box::new(number(1.0)), Box::new(number(0.0)), HirType::F64,
                        )),
                        HirType::F64,
                    )),
                    HirStmt::If(
                        eq(variable(stop), number(1.0)),
                        vec![assign(idx, variable(&length_name))],
                        vec![
                            assign(prev, variable(cur)),
                            assign(run, add_one(variable(run))),
                            assign(idx, add_one(variable(idx))),
                        ],
                    ),
                ]),
                HirStmt::If(
                    eq(variable(desc), number(1.0)),
                    vec![
                        assign(lo2, number(0.0)),
                        assign(hi2, sub_one(variable(run))),
                        HirStmt::While(lt(variable(lo2), variable(hi2)), vec![
                            HirStmt::Let(tmp.clone(), element_type.clone(), at(variable(lo2))),
                            store(variable(lo2), at(variable(hi2))),
                            store(variable(hi2), variable(tmp)),
                            assign(lo2, add_one(variable(lo2))),
                            assign(hi2, sub_one(variable(hi2))),
                        ]),
                    ],
                    Vec::new(),
                ),
            ];
            // BinaryInsertionSort(0, run, length)
            let probe = call(self, variable(pivot), at(variable(mid_i)))?;
            let insertion = vec![
                assign(start, variable(run)),
                HirStmt::While(lt(variable(start), variable(&length_name)), vec![
                    assign(left, number(0.0)),
                    assign(right, variable(start)),
                    HirStmt::Let(pivot.clone(), element_type.clone(), at(variable(right))),
                    HirStmt::While(lt(variable(left), variable(right)), vec![
                        assign(mid_i, HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(variable(left)),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_math_floor".into())),
                                vec![HirExpr::BinOp(
                                    BinOp::Div,
                                    Box::new(HirExpr::BinOp(BinOp::Sub, Box::new(variable(right)), Box::new(variable(left)))),
                                    Box::new(number(2.0)),
                                )],
                            )),
                        )),
                        assign(order, probe),
                        HirStmt::If(
                            lt(variable(order), number(0.0)),
                            vec![assign(right, variable(mid_i))],
                            vec![assign(left, add_one(variable(mid_i)))],
                        ),
                    ]),
                    assign(p, variable(start)),
                    HirStmt::While(gt(variable(p), variable(left)), vec![
                        store(variable(p), at(sub_one(variable(p)))),
                        assign(p, sub_one(variable(p))),
                    ]),
                    store(variable(left), variable(pivot)),
                    assign(start, add_one(variable(start))),
                ]),
            ];
            let mut body = count_run;
            body.push(HirStmt::If(
                lt(variable(run), variable(&length_name)),
                insertion,
                Vec::new(),
            ));
            let mut declared: Vec<HirStmt> = names
                .iter()
                .map(|name| HirStmt::Let(name.clone(), HirType::F64, number(0.0)))
                .collect();
            declared.push(HirStmt::If(ge(variable(&length_name), number(2.0)), body, Vec::new()));
            declared
        };
        let mut pass = vec![assign(lo, number(0.0)), HirStmt::While(lt(variable(lo), variable(&length_name)), merge_run)];
        pass.extend(copy_back);
        pass.push(assign(
            width,
            HirExpr::BinOp(BinOp::Mul, Box::new(variable(width)), Box::new(number(2.0))),
        ));
        let result = if copy {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_densify".into())),
                vec![variable(&array_name)],
            )
        } else {
            variable(&receiver_name)
        };
        let mut statements = vec![
            HirStmt::Let(array_name.clone(), array_type.clone(), working_source),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_compact_for_sort".into())),
                    vec![variable(&array_name)],
                ),
            ),
            HirStmt::Let(
                tmp_name.clone(),
                array_type.clone(),
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_slice".into())),
                    vec![variable(&array_name), number(0.0), number(f64::INFINITY)],
                ),
            ),
            HirStmt::Let(width.clone(), HirType::F64, number(1.0)),
            HirStmt::Let(lo.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(mid.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(hi.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(k.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(take.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(copy_index.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(inner_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(j.clone(), HirType::F64, number(0.0)),
        ];
        let generic_sort = HirStmt::If(
            lt(variable(&length_name), number(64.0)),
            small_run,
            vec![HirStmt::While(lt(variable(width), variable(&length_name)), pass)],
        );
        match native_descending {
            Some(descending) => {
                let sorted_name = format!("__thaw_sort_native_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(sorted_name.clone(), HirType::F64);
                statements.push(HirStmt::Let(
                    sorted_name.clone(),
                    HirType::F64,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_comparator_sort".into())),
                        vec![variable(&array_name), variable(&length_name), number(f64::from(descending))],
                    ),
                ));
                statements.push(HirStmt::If(
                    HirExpr::BinOp(BinOp::EqEqEq, Box::new(variable(&sorted_name)), Box::new(number(0.0))),
                    vec![generic_sort],
                    Vec::new(),
                ));
            }
            None => statements.push(generic_sort),
        }
        if !copy {
            let write_index = format!("__thaw_sort_write_{}", self.next_binding);
            self.next_binding += 1;
            let write_state = format!("__thaw_sort_state_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(write_index.clone(), HirType::F64);
            self.scope.insert(write_state.clone(), HirType::F64);
            let delete_key = self.coerce_primitive_to_string(variable(&write_index))?;
            statements.push(HirStmt::Let(write_index.clone(), HirType::F64, number(0.0)));
            statements.push(HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(variable(&write_index)),
                    Box::new(HirExpr::ArrayLen(Box::new(variable(&array_name)))),
                ),
                vec![
                    HirStmt::Let(write_state.clone(), HirType::F64, HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                        vec![variable(&array_name), variable(&write_index)],
                    )),
                    HirStmt::If(
                        HirExpr::BinOp(BinOp::EqEqEq,
                            Box::new(variable(&write_state)), Box::new(number(0.0))),
                        vec![HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_delete_strict".into())),
                            vec![variable(&receiver_name), delete_key],
                        ))],
                        vec![HirStmt::If(
                            HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(variable(&write_state)), Box::new(number(2.0))),
                            vec![HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_set_undefined".into())),
                                vec![variable(&receiver_name), variable(&write_index)],
                            ))],
                            vec![HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(variable(&receiver_name)),
                                Box::new(variable(&write_index)),
                                Box::new(HirExpr::TypedIndex(
                                    Box::new(variable(&array_name)),
                                    Box::new(variable(&write_index)), element_type.clone(),
                                )),
                            ))],
                        )],
                    ),
                    HirStmt::Expr(HirExpr::Assign(write_index.clone(),
                        Box::new(add_one(variable(&write_index))))),
                ],
            ));
        }
        statements.push(HirStmt::Return(Some(result)));
        let body = HirExpr::Block(statements);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (comparator_name, comparator_type, comparator),
            ],
        )
    }

    /// Builds a call to an array iteration callback, coercing each
    /// argument to the callback's own declared parameter type first.
    ///
    /// The callback's parameter types come from its own signature (e.g. an
    /// explicit `(x: number) =>` annotation), which can be narrower than
    /// the contextual type used to build `available` -- a sparse array's
    /// element read is `T | undefined` (`Optional`) even when an explicit
    /// annotation declares plain `T`, since the compiler respects explicit
    /// annotations over the contextual type (`lower_contextual_arrow`).
    /// Without this coercion, `.map()`/`.filter()`/predicate/`reduce`/
    /// `.forEach()` on a possibly-sparse array pass an `Optional` value's
    /// runtime layout to a callback compiled to expect the bare payload,
    /// which LLVM's module verifier rejects as a parameter type mismatch.
    /// `thisArg` is evaluated once into a binding. A receiver-taking callback
    /// (`function (this: T, ...)`) is invoked with it as `this`; any other
    /// callback ignores it, as in JavaScript.
    fn bind_array_callback_this(
        &mut self,
        label: &str,
        callback_type: &HirType,
        this_arg: &Option<HirExpr>,
    ) -> Result<Option<(String, HirType, HirExpr)>, String> {
        let Some(this_arg) = this_arg else { return Ok(None) };
        let declared_receiver = match callback_type {
            HirType::FunctionWithThis(receiver, _) => Some(receiver.as_ref()),
            _ => None,
        };
        let this_arg = &self.coerce_this_for_receiver(declared_receiver, this_arg.clone())?;
        let ty = self.infer_expr_type(this_arg)?;
        let name = format!("__thaw_{label}_this_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), ty.clone());
        Ok(Some((name, ty, this_arg.clone())))
    }

    /// The visible signature of an array callback and the `this` binding to
    /// call it with (only for a `FunctionWithThis` callback).
    fn array_callback_signature(
        callback_type: &HirType,
        this_binding: &Option<(String, HirType, HirExpr)>,
    ) -> (HirType, Option<String>) {
        match callback_type {
            HirType::FunctionWithThis(_, inner) => (
                inner.as_ref().clone(),
                this_binding.as_ref().map(|(name, _, _)| name.clone()),
            ),
            other => (other.clone(), None),
        }
    }

    fn lower_array_callback_call(
        &mut self,
        callback_name: &str,
        params: &[HirType],
        available: &[HirExpr],
    ) -> Result<HirExpr, String> {
        let args = available[..params.len()]
            .iter()
            .cloned()
            .zip(params)
            .map(|(value, declared)| self.coerce_primitive_array_argument(value, declared))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.to_string())),
            args,
        ))
    }

    /// `arr.filter(Boolean)` / `arr.map(Number)`: the callback `(x) => Boolean(x)`.
    fn builtin_conversion_callback(&self, expr: &Expr) -> Option<Expr> {
        match expr {
            Expr::Ident(ident)
                if matches!(ident.sym.as_ref(), "Boolean" | "Number" | "String")
                    && !self.scope.contains_key(ident.sym.as_ref())
                    && !self.signatures.contains_key(ident.sym.as_ref()) =>
            {
                let span = ident.span;
                let param = swc_ecma_ast::Ident::new_no_ctxt("x".into(), span);
                Some(Expr::Arrow(swc_ecma_ast::ArrowExpr {
                    span,
                    ctxt: Default::default(),
                    params: vec![Pat::Ident(swc_ecma_ast::BindingIdent {
                        id: param.clone(),
                        type_ann: None,
                    })],
                    body: Box::new(swc_ecma_ast::ArrowFunctionBody::Expr(Box::new(Expr::Call(
                        CallExpr {
                            span,
                            ctxt: Default::default(),
                            callee: Callee::Expr(Box::new(expr.clone())),
                            args: vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Ident(param)),
                            }],
                            type_args: None,
                        },
                    )))),
                    is_async: false,
                    is_generator: false,
                    type_params: None,
                    return_type: None,
                }))
            }
            _ => None,
        }
    }

    fn lower_array_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let conversion = self.builtin_conversion_callback(expr);
        let expr = conversion.as_ref().unwrap_or(expr);
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array predicate `{name}`"))?
            }
            _ => self.callback_value_arity(expr, "array predicate")?,
        };
        if arity > 3 {
            return Err(format!(
                "array predicate accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        self.lower_native_iteration_callback(expr, &available[..arity], expected_return)
    }

    fn array_callback_truthy(&mut self, callback: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&callback)?;
        if ty == HirType::Void {
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&callback, &mut referenced);
            let captures = referenced.into_iter().filter_map(|name| {
                self.scope.get(&name).cloned().map(|ty| HirParam { name, ty })
            }).collect();
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    Vec::new(),
                    HirType::Bool,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Expr(callback),
                        HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false)))),
                    ])),
                )),
                Vec::new(),
            ));
        }
        let name = format!("__thaw_array_predicate_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), ty.clone());
        let truthy = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
        self.wrap_call_argument_bindings(truthy, &[(name, ty, callback)])
    }

    fn array_void_to_undefined(&mut self, callback: HirExpr) -> Result<HirExpr, String> {
        Ok(HirExpr::Conditional(
            Box::new(self.array_callback_truthy(callback)?),
            Box::new(HirExpr::Lit(HirLit::Undefined)),
            Box::new(HirExpr::Lit(HirLit::Undefined)),
            HirType::Undefined,
        ))
    }

    /// Arity of a callback that is neither a literal nor a plain name (`o.method`, `f.bind(x)`):
    /// the type of the lowered value decides it.
    fn callback_value_arity(&mut self, expr: &Expr, label: &str) -> Result<usize, String> {
        let value = self.lower_expr(expr)?;
        let ty = match self.infer_expr_type(&value)? {
            HirType::FunctionWithThis(_, visible) => *visible,
            other => other,
        };
        match ty {
            HirType::Function(params, _) | HirType::CallableFunction(params, ..) => Ok(params.len()),
            _ => Err(format!("{label} must be an arrow or function value")),
        }
    }

    fn lower_array_reducer_callback(
        &mut self,
        expr: &Expr,
        accumulator_type: &HirType,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array reducer `{name}`"))?
            }
            _ => self.callback_value_arity(expr, "array reducer")?,
        };
        if arity > 4 {
            return Err(format!(
                "array reducer accepts at most four parameters, got {arity}"
            ));
        }
        let available = [
            accumulator_type.clone(),
            element_type.clone(),
            HirType::F64,
            array_type.clone(),
        ];
        self.lower_native_iteration_callback(expr, &available[..arity], Some(accumulator_type))
    }

    fn lower_array_mapping_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let conversion = self.builtin_conversion_callback(expr);
        let expr = conversion.as_ref().unwrap_or(expr);
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array mapper `{name}`"))?
            }
            _ => self.callback_value_arity(expr, "array mapper")?,
        };
        if arity > 3 {
            return Err(format!(
                "array mapper accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        // A fixed local callback with a declared numeric/string parameter
        // keeps its own ABI. The array loop already adapts sparse primitive
        // reads at the call site; normalizing the callback here instead
        // would route Optional<F64> through the general declared-value
        // coercion and reject a valid `number` parameter.
        let direct_fixed_primitive_callback = if let Expr::Ident(ident) = expr {
            let name = self.resolve_binding(ident.sym.as_ref());
            let is_template = self.generic_arrows.contains_key(&name)
                || self.generic_named_templates.contains_key(&name);
            let has_rest = self.signatures.get(&name)
                .is_some_and(|signature| signature.native_rest.is_some());
            !is_template && !has_rest && self.scope.get(&name).is_some_and(|ty| {
                let HirType::Function(params, _) = ty else { return false };
                if params.len() != arity || params.len() > available.len() {
                    return false;
                }
                let supported = params.iter().zip(&available).all(|(declared, supplied)| {
                    declared == supplied
                        || matches!((declared, supplied),
                            (HirType::F64, HirType::Optional(payload)) if **payload == HirType::F64)
                        || matches!((declared, supplied),
                            (HirType::Str, HirType::Optional(payload)) if **payload == HirType::Str)
                });
                let needs_primitive_adapter = params.iter().zip(&available).any(|(declared, supplied)| {
                    matches!((declared, supplied),
                        (HirType::F64, HirType::Optional(payload)) if **payload == HirType::F64)
                        || matches!((declared, supplied),
                        (HirType::Str, HirType::Optional(payload)) if **payload == HirType::Str)
                });
                supported && needs_primitive_adapter
            })
        } else {
            false
        };
        self.sparse_mapping_result = true;
        let result = if direct_fixed_primitive_callback {
            self.lower_expr(expr)
        } else {
            self.lower_native_iteration_callback(expr, &available[..arity], None)
        };
        self.sparse_mapping_result = false;
        result
    }

    fn lower_array_from_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown Array.from mapper `{name}`"))?
            }
            _ => self.callback_value_arity(expr, "Array.from mapper")?,
        };
        if arity > 2 {
            return Err(format!(
                "Array.from mapper accepts at most two parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64];
        self.sparse_mapping_result = true;
        let result = self.lower_native_iteration_callback(expr, &available[..arity], None);
        self.sparse_mapping_result = false;
        result
    }

}
