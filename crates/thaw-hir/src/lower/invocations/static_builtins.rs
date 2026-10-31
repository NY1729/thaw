/// Types the native-to-`Json` conversion in `compile_json_object_set_native`
/// (thaw-llvm's `json_bridge.rs`, reached below through `HirExpr::JsonSet`)
/// already knows how to encode as an object field, and so can also encode
/// as a `JSON.stringify` argument via `wrap_native_value_as_json`. Anything
/// else (`Function`, `Promise`, `Map`/`Set`, ...) has no such encoding and
/// is rejected exactly as before. `Union` *is* one of these (`compile_
/// json_object_set_union` recursively dispatches back into this same
/// per-member encoder for whichever member is actually tagged active) --
/// real trigger: `const x: any = cond ? 1 : "str";`, a ternary/ if-else
/// naturally inferring a union of its branch types, assigned to an
/// `any`-typed slot.
fn json_convertible_native_type(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::F64
            | HirType::Str
            | HirType::Bool
            | HirType::Null
            | HirType::Undefined
            | HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Optional(_)
            | HirType::Nullable(_)
            | HirType::Nullish(_)
            | HirType::Union(_)
    )
}

fn contains_js_value(ty: &HirType) -> bool {
    match ty {
        HirType::JsValue => true,
        HirType::Array(element)
        | HirType::Optional(element)
        | HirType::Nullable(element)
        | HirType::Nullish(element)
        | HirType::Dictionary(element) => contains_js_value(element),
        HirType::Tuple(elements) => elements.iter().any(contains_js_value),
        HirType::Object(fields) => fields.iter().any(|(_, ty)| contains_js_value(ty)),
        _ => false,
    }
}

fn json_has_borrowed_handle(value: HirExpr) -> HirExpr {
    HirExpr::Conditional(
        Box::new(HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_borrowed_handle_id".into())),
                vec![value],
            )),
            Box::new(HirExpr::Lit(HirLit::I64(0))),
        )),
        Box::new(HirExpr::Lit(HirLit::Bool(false))),
        Box::new(HirExpr::Lit(HirLit::Bool(true))),
        HirType::Bool,
    )
}

impl<'a> FnLowerer<'a> {
    /// Builds the `__thawNativeIterator` protocol object (next/return/throw
    /// lambdas over the generator producer `source`).  `dynamic_any` makes the
    /// object a plain `any` view of the generator: `next(v)` takes the resume
    /// value and `throw(e)` takes any thrown value.
    pub(in crate::lower) fn generator_protocol_object(
        &mut self,
        source: &HirExpr,
        source_type: &HirType,
        span: swc_common::Span,
        ctxt: swc_common::SyntaxContext,
        bindings: &mut Vec<LoweredBinding>,
        dynamic_any: bool,
    ) -> Result<HirExpr, String> {
                            let producer = format!("__thaw_iterator_producer_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(producer.clone(), source_type.clone());
                            bindings.push((producer.clone(), source_type.clone(), source.clone()));
                let return_parameter = format!("__thaw_iterator_return_{}", self.next_binding);
                            self.next_binding += 1;
                let throw_parameter = format!("__thaw_iterator_throw_{}", self.next_binding);
                            self.next_binding += 1;
                let next_parameter = format!("__thaw_iterator_next_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut lower_protocol_method =
                                |method: &str, parameter: Option<(String, HirType)>| {
                                let protocol_call = CallExpr {
                                    span,
                                    ctxt,
                                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                        span,
                                obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                                producer.clone().into(),
                                                span,
                                ))),
                                prop: MemberProp::Ident(IdentName::new(method.into(), span)),
                                    }))),
                                    args: parameter
                                        .as_ref()
                                        .map(|(name, _)| {
                                            vec![swc_ecma_ast::ExprOrSpread {
                                                spread: None,
                                                expr: Box::new(Expr::Ident(
                                                    swc_ecma_ast::Ident::new_no_ctxt(
                                                        name.clone().into(),
                                                        span,
                                                    ),
                                                )),
                                            }]
                                        })
                                        .unwrap_or_default(),
                                    type_args: None,
                                };
                        let previous = parameter
                            .as_ref()
                            .and_then(|(name, ty)| self.scope.insert(name.clone(), ty.clone()));
                                let body = self.lower_call(&protocol_call);
                                let result = match &body {
                                    Ok(body) => self.infer_expr_type(body),
                                    Err(error) => Err(error.clone()),
                                };
                                if let Some((name, _)) = &parameter {
                                    if let Some(previous) = previous {
                                        self.scope.insert(name.clone(), previous);
                                    } else {
                                        self.scope.remove(name);
                                    }
                                }
                                let body = body?;
                                let result = result?;
                                let captures = vec![HirParam {
                                    name: producer.clone(),
                                    ty: source_type.clone(),
                                }];
                                if let Some((name, ty)) = parameter {
                                    let converted = match &ty {
                                HirType::F64 => {
                                    HirExpr::JsonAsNumber(Box::new(HirExpr::Var(name.clone())))
                                }
                                HirType::Json => HirExpr::Var(name.clone()),
                                HirType::Str => {
                                    HirExpr::JsonAsString(Box::new(HirExpr::Var(name.clone())))
                                }
                                HirType::Bool => {
                                    HirExpr::JsonAsBool(Box::new(HirExpr::Var(name.clone())))
                                        }
                                HirType::Undefined => HirExpr::Lit(HirLit::Undefined),
                                        _ => HirExpr::JsonAsNative(
                                            Box::new(HirExpr::Var(name.clone())),
                                            ty.clone(),
                                        ),
                                    };
                                    let native = HirExpr::Lambda(
                                        captures.clone(),
                                        vec![HirParam {
                                            name: name.clone(),
                                            ty,
                                        }],
                                        result.clone(),
                                        Box::new(body),
                                    );
                                    Ok::<_, String>(HirExpr::Lambda(
                                        captures,
                                        vec![HirParam {
                                            name,
                                            ty: HirType::Json,
                                        }],
                                        result,
                                        Box::new(HirExpr::Call(Box::new(native), vec![converted])),
                                    ))
                                } else {
                                    Ok::<_, String>(HirExpr::Lambda(
                                        captures,
                                        Vec::new(),
                                        result,
                                        Box::new(body),
                                    ))
                                }
                            };
                            let HirType::Function(params, _) = &source_type else {
                                unreachable!("native iterator producer was matched as a function")
                            };
                            let HirType::Array(return_values) = &params[3] else {
                                unreachable!("native iterator return channel was matched as an array")
                            };
                            let mut object = HirExpr::ObjectLit(vec![
                                (
                                    "__thawNativeIterator".into(),
                                    HirExpr::Lit(HirLit::Bool(true)),
                                ),
                                ("next".into(), if dynamic_any {
                                    lower_protocol_method("next", Some((next_parameter.clone(), params[2].clone())))?
                                } else {
                                    lower_protocol_method("next", None)?
                                }),
                                (
                                    "__thawNext".into(),
                        lower_protocol_method("next", Some((next_parameter, params[2].clone())))?,
                                ),
                                ("return".into(), if dynamic_any {
                                    lower_protocol_method("return", Some((format!("{return_parameter}_any"), return_values.as_ref().clone())))?
                                } else {
                                    lower_protocol_method("return", None)?
                                }),
                                (
                                    "__thawReturn".into(),
                                    lower_protocol_method(
                                        "return",
                            Some((return_parameter, return_values.as_ref().clone())),
                                    )?,
                                ),
                                (
                                    "__thawThrow".into(),
                        lower_protocol_method("throw", Some((throw_parameter, if dynamic_any { HirType::Json } else { HirType::Str })))?,
                                ),
                            ]);
        if let (true, HirExpr::ObjectLit(fields)) = (dynamic_any, &mut object) {
            let throw = fields.last().map(|(_, method)| method.clone());
            fields.extend(throw.map(|method| ("throw".into(), method)));
        }
        Ok(object)
    }

    /// Visible parameters of an `Array.from` mapper; a `this`-taking closure
    /// exposes the parameters of the signature it wraps.
    fn array_mapper_params(ty: &HirType) -> Option<Vec<HirType>> {
        match ty {
            HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                Some(params.clone())
            }
            HirType::FunctionWithThis(_, inner) => Self::array_mapper_params(inner),
            _ => None,
        }
    }

    fn lower_array_from_mapper_call(
        &mut self,
        callback_name: &str,
        params: &[HirType],
        returned: &HirType,
        available: &[HirExpr],
        this_name: Option<&str>,
    ) -> Result<HirExpr, String> {
        let args = available[..params.len()]
            .iter()
            .cloned()
            .zip(params)
            .map(|(value, declared)| self.coerce_primitive_array_argument(value, declared))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if let Some(this_name) = this_name {
            HirExpr::FunctionCallWithThis(
                Box::new(HirExpr::Var(callback_name.into())),
                Box::new(HirExpr::Var(this_name.into())),
                args,
                params.to_vec(),
                returned.clone(),
            )
        } else {
            HirExpr::Call(Box::new(HirExpr::Var(callback_name.into())), args)
        })
    }

    /// Iterate a tuple view to its actual runtime length. A contextual
    /// annotation describes only a typed prefix, so using the native tuple
    /// buffer here would silently discard a reflected array's tail.
    fn lower_tuple_as_json_array(
        &mut self,
        source: HirExpr,
        elements: Vec<HirType>,
    ) -> Result<HirExpr, String> {
        let tuple_type = HirType::Tuple(elements);
        let array_type = HirType::Array(Box::new(HirType::Json));
        let source_name = format!("__thaw_tuple_iterator_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), tuple_type.clone());
        let output_name = format!("__thaw_tuple_iterator_output_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(output_name.clone(), array_type.clone());
        let index_name = format!("__thaw_tuple_iterator_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(index_name.clone(), HirType::F64);
        let var = |name: &str| HirExpr::Var(name.into());
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                output_name.clone(),
                array_type,
                HirExpr::ArrayLit(Vec::new()),
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
                    Box::new(HirExpr::ArrayLen(Box::new(var(&source_name)))),
                ),
                vec![
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(var("__thaw_array_push")),
                vec![
                            var(&output_name),
                            HirExpr::TypedIndex(
                                Box::new(var(&source_name)),
                                Box::new(var(&index_name)),
                                HirType::Json,
                            ),
                        ],
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(var(&output_name))),
        ]);
        self.wrap_call_argument_bindings(body, &[(source_name, tuple_type, source)])
    }

    /// True when every leaf of `ty` has a canonical Json encoding, so a tuple
    /// of such members can be read through the Json array path.
    fn spread_json_encodable_type(ty: &HirType) -> bool {
        match ty {
            HirType::F64
            | HirType::Str
            | HirType::Bool
            | HirType::Null
            | HirType::Undefined
            | HirType::Json => true,
            HirType::Array(element)
            | HirType::Optional(element)
            | HirType::Nullable(element)
            | HirType::Nullish(element) => Self::spread_json_encodable_type(element),
            HirType::Tuple(members) | HirType::Union(members) => {
                members.iter().all(Self::spread_json_encodable_type)
            }
            HirType::Object(fields) => fields
                .iter()
                .all(|(_, ty)| Self::spread_json_encodable_type(ty)),
            // Opaque/native categories have no bridge to Json here.
            _ => false,
        }
    }

    // A fixed layout can contain a hidden native marker or an ordinary user
    // field with exactly the same name. Query the bound receiver before each
    // read, then append only visible fields to a runtime-length array.
    /// For a catch carrier holding a thrown Error-family class instance (member 6) whose props were
    /// registered at the throw: `from_props(<registered JSON text>)`; otherwise `fallback`.
    fn thrown_error_owner_branch(
        &mut self,
        carrier: &HirExpr,
        members: &[HirType],
        hidden: bool,
        fallback: HirExpr,
        result_type: HirType,
        from_props: impl FnOnce(HirExpr) -> HirExpr,
    ) -> HirExpr {
        let props_name = format!("__thaw_owner_props_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(props_name.clone(), HirType::Str);
        let props = HirExpr::Var(props_name.clone());
        let owner = HirExpr::UnionValue(Box::new(carrier.clone()), 6, members.to_vec());
        let registered = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_error_owner_props".to_string())),
            vec![owner, HirExpr::Lit(HirLit::Bool(hidden))],
        );
        let is_owner = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::UnionTag(Box::new(carrier.clone()), members.to_vec())),
            Box::new(HirExpr::Lit(HirLit::F64(6.0))),
        );
        let none_registered = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(props.clone()),
            Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
        );
        let owner_arm = HirExpr::Block(vec![
            HirStmt::Let(props_name, HirType::Str, registered),
            HirStmt::Return(Some(HirExpr::Conditional(
                Box::new(none_registered),
                Box::new(fallback.clone()),
                Box::new(from_props(props)),
                result_type.clone(),
            ))),
        ]);
        HirExpr::Conditional(Box::new(is_owner), Box::new(owner_arm), Box::new(fallback), result_type)
    }

    /// For an Error-family record: the `name` a class inherits when its constructor never assigns
    /// one. `message` is never an own enumerable key and `name` is one only when it differs.
    pub(crate) fn error_family_default_name(fields: &[(Symbol, HirType)]) -> Option<String> {
        let chain = fields
            .first()
            .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))?;
        let own = chain.split('\u{1f}').next().unwrap_or_default();
        // A built-in record (AggregateError, SuppressedError) names itself.
        if matches!(own, "AggregateError" | "SuppressedError") {
            return Some(own.to_string());
        }
        (!is_error_family_name(own) && chain.split('\u{1f}').any(is_error_family_name)).then(|| {
            chain
                .split('\u{1f}')
                .skip(1)
                .find(|part| is_error_family_name(part))
                .unwrap_or("Error")
                .to_string()
        })
    }

    /// Fields of a built-in Error record that are own but non-enumerable (besides `message`).
    pub(crate) fn error_record_hidden_fields(fields: &[(Symbol, HirType)]) -> &'static [&'static str] {
        let chain = fields
            .first()
            .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
            .unwrap_or_default();
        match chain.split('\u{1f}').next().unwrap_or_default() {
            "AggregateError" => &["cause", "errors"],
            "SuppressedError" => &["error", "suppressed"],
            _ => &[],
        }
    }

    /// `if (name !== default) { push }` for an Error-family record's `name` key.
    fn error_family_name_statement(
        &mut self,
        receiver: &HirExpr,
        fields: &[(Symbol, HirType)],
        default_name: String,
        append: HirExpr,
    ) -> Result<HirStmt, String> {
        let name = self.lower_fixed_object_property_read(receiver.clone(), fields, "name")?;
        Ok(HirStmt::If(
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(name),
                Box::new(HirExpr::Lit(HirLit::Str(default_name))),
            ),
            Vec::new(),
            vec![HirStmt::Expr(append)],
        ))
    }

    /// Own visible (non-index) property names of an `extends Array` instance type: `Some` when
    /// `ty` wraps an array and declares own fields.
    fn array_subclass_own_fields(ty: &HirType) -> Option<Vec<String>> {
        let HirType::Object(fields) = ty else {
            return None;
        };
        if !matches!(native_base_type(ty), Some(HirType::Array(_))) {
            return None;
        }
        let own = ecmascript_field_order(fields)
            .into_iter()
            .map(|index| fields[index].0.to_string())
            .filter(|name| !name.starts_with("__thaw_class_identity_\u{1e}"))
            .collect::<Vec<_>>();
        (!own.is_empty()).then_some(own)
    }

    /// `Object.keys` / `getOwnPropertyNames` / `for...in` keys of an `extends Array` instance
    /// with own fields: the index keys (plus `length` when `all`) of the wrapped array, then
    /// the own property names. `object` is a variable of type `ty`.
    pub(in crate::lower) fn lower_array_subclass_keys(
        &mut self,
        object: HirExpr,
        ty: &HirType,
        all: bool,
    ) -> Result<HirExpr, String> {
        let own = Self::array_subclass_own_fields(ty).unwrap_or_default();
        let keys_type = HirType::Array(Box::new(HirType::Str));
        let name = format!("__thaw_subclass_keys_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), keys_type.clone());
        let base = HirExpr::PropAccess(Box::new(object), ty.clone(), NATIVE_BASE_FIELD.to_string());
        let mut statements = vec![HirStmt::Let(
            name.clone(),
            keys_type,
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_keys".into())),
                vec![base, HirExpr::Lit(HirLit::Bool(all))],
            ),
        )];
        for field in own {
            statements.push(HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(name.clone()), HirExpr::Lit(HirLit::Str(field))],
            )));
        }
        statements.push(HirStmt::Return(Some(HirExpr::Var(name))));
        Ok(HirExpr::Block(statements))
    }

    /// `Object.values` / `Object.entries` of an `extends Array` instance with own fields: the
    /// elements, then the own property values: typed when the element and all own field types
    /// agree (`number[]`, `[string, number][]`), else an `any[]`.
    fn lower_array_subclass_sequence(
        &mut self,
        object: HirExpr,
        ty: &HirType,
        entries: bool,
    ) -> Result<HirExpr, String> {
        let (Some(own), Some(base_type), HirType::Object(fields)) =
            (Self::array_subclass_own_fields(ty), native_base_type(ty).cloned(), ty)
        else {
            return Err("Object.values/entries expects an Array subclass with own fields".into());
        };
        let HirType::Array(element) = &base_type else {
            return Err("Object.values/entries expects an Array subclass with own fields".into());
        };
        let element = element.as_ref().clone();
        let mut value_types = vec![element.clone()];
        for field in &own {
            let field_type = Self::fixed_object_property_read_type(fields, field)?;
            if !value_types.contains(&field_type) {
                value_types.push(field_type);
            }
        }
        // Mixed element / field types cannot share one native element type: those stay `any[]`.
        let value_type = match value_types.as_slice() {
            [only] => only.clone(),
            _ => HirType::Json,
        };
        let item_type = if entries && value_type != HirType::Json {
            HirType::Tuple(vec![HirType::Str, value_type.clone()])
        } else {
            value_type.clone()
        };
        let json_array = HirType::Array(Box::new(item_type.clone()));
        let id = self.next_binding;
        self.next_binding += 1;
        let (object_name, result_name, base_name, index_name, item_name) = (
            format!("__thaw_subclass_object_{id}"),
            format!("__thaw_subclass_items_{id}"),
            format!("__thaw_subclass_base_{id}"),
            format!("__thaw_subclass_index_{id}"),
            format!("__thaw_subclass_item_{id}"),
        );
        self.scope.insert(object_name.clone(), ty.clone());
        self.scope.insert(result_name.clone(), json_array.clone());
        self.scope.insert(base_name.clone(), base_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(item_name.clone(), element.clone());
        let push = |item: HirExpr| {
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(result_name.clone()), item],
            ))
        };
        let wrap = |this: &mut Self, key: HirExpr, value: HirExpr| -> Result<HirExpr, String> {
            let value = this.coerce_to_declared(&value_type, value)?;
            if !entries {
                return Ok(value);
            }
            let key = if item_type == HirType::Json { this.coerce_to_declared(&HirType::Json, key)? } else { key };
            this.coerce_to_declared(&item_type, HirExpr::ArrayLit(vec![key, value]))
        };
        let index_key = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_number_to_string".into())),
            vec![HirExpr::Var(index_name.clone())],
        );
        let element_item = wrap(self, index_key, HirExpr::Var(item_name.clone()))?;
        let mut statements = vec![
            HirStmt::Let(object_name.clone(), ty.clone(), object),
            HirStmt::Let(result_name.clone(), json_array, HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(
                base_name.clone(),
                base_type.clone(),
                HirExpr::PropAccess(
                    Box::new(HirExpr::Var(object_name.clone())),
                    ty.clone(),
                    NATIVE_BASE_FIELD.to_string(),
                ),
            ),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(base_name.clone())))),
                ),
                vec![
                    HirStmt::Let(
                        item_name.clone(),
                        element.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(base_name)),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element,
                        ),
                    ),
                    push(element_item),
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
        ];
        for field in own {
            let read =
                self.lower_fixed_object_property_read(HirExpr::Var(object_name.clone()), fields, &field)?;
            let item = wrap(self, HirExpr::Lit(HirLit::Str(field)), read)?;
            statements.push(push(item));
        }
        statements.push(HirStmt::Return(Some(HirExpr::Var(result_name))));
        Ok(HirExpr::Block(statements))
    }

    fn lower_visible_fixed_object_sequence(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        entries: bool,
    ) -> Result<HirExpr, String> {
        let default_name = Self::error_family_default_name(fields);
        let mut ordered = ecmascript_field_order(fields)
            .into_iter()
            .map(|index| {
                let name = fields[index].0.clone();
                let ty = Self::fixed_object_property_read_type(fields, &name)?;
                Ok((index, name, ty))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // An Error-family record hides `message` and shows `name` only when assigned (last).
        let hidden_extra = Self::error_record_hidden_fields(fields);
        let name_entry = default_name.as_ref().and_then(|_| {
            ordered.retain(|(_, name, _)| name != "message" && !hidden_extra.contains(&name.as_str()));
            ordered.iter().position(|(_, name, _)| name == "name").map(|at| ordered.remove(at))
        });
        let mut types = Vec::new();
        for (_, _, ty) in ordered.iter().chain(name_entry.iter()) {
            if !types.contains(ty) {
                types.push(ty.clone());
            }
        }
        let value_type = match types.as_slice() {
            [] => HirType::Json,
            [only] => only.clone(),
            _ => HirType::Union(types),
        };
        let element_type = if entries {
            HirType::Tuple(vec![HirType::Str, value_type.clone()])
        } else {
            value_type.clone()
        };
        let array_type = HirType::Array(Box::new(element_type.clone()));
        let object_type = HirType::Object(fields.to_vec());
        let receiver_name = format!("__thaw_visible_object_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), object_type.clone());
        let result_name = format!("__thaw_visible_fields_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), array_type.clone());
        let receiver = HirExpr::Var(receiver_name.clone());
        let mut statements = vec![HirStmt::Let(
            result_name.clone(),
            array_type,
            HirExpr::ArrayLit(Vec::new()),
        )];
        let mut actions = Vec::new();
        let mut name_statement = None;
        for (index, field, _) in ordered.into_iter().chain(name_entry) {
            let is_name = default_name.is_some() && field == "name";
            let read = self.lower_fixed_object_property_read(receiver.clone(), fields, &field)?;
            let read = self.coerce_to_declared(&value_type, read)?;
            let item = if entries {
                self.coerce_to_declared(
                    &element_type,
                    HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::Str(field)), read]),
                )?
            } else {
                read
            };
            let append = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(result_name.clone()), item],
            );
            if is_name {
                name_statement = Some(self.error_family_name_statement(
                    &receiver, fields, default_name.clone().unwrap(), append,
                )?);
            } else {
                actions.push((index, vec![HirStmt::Expr(append)]));
            }
        }
        statements.extend(self.lower_ordered_fixed_field_statements(receiver, fields, actions)?);
        statements.extend(name_statement);
        statements.push(HirStmt::Return(Some(HirExpr::Var(result_name))));
        self.wrap_call_argument_bindings(
            HirExpr::Block(statements),
            &[(receiver_name, object_type, value)],
        )
    }

    /// `Math.min(...values)`/`Math.max(...values)` for a runtime-length
    /// `number[]` spread source: folds pairwise through
    /// `__thaw_math_min`/`__thaw_math_max` in a runtime loop, seeded with
    /// `Infinity`/`-Infinity` so an empty array produces the same result
    /// the fixed-arity zero-argument form already does.
    fn lower_math_extreme_of_array(
        &mut self,
        source: HirExpr,
        is_min: bool,
    ) -> Result<HirExpr, String> {
        let array_type = HirType::Array(Box::new(HirType::F64));
        let source_name = format!("__thaw_math_extreme_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), array_type.clone());
        let acc_name = format!("__thaw_math_extreme_acc_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(acc_name.clone(), HirType::F64);
        let index_name = format!("__thaw_math_extreme_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(index_name.clone(), HirType::F64);
        let var = |name: &str| HirExpr::Var(name.into());
        let intrinsic = if is_min {
            "__thaw_math_min"
        } else {
            "__thaw_math_max"
        };
        let seed = if is_min {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                acc_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(seed)),
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
                    Box::new(HirExpr::ArrayLen(Box::new(var(&source_name)))),
                ),
                vec![
                    HirStmt::Expr(HirExpr::Assign(
                        acc_name.clone(),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![
                                var(&acc_name),
                                // Spread reads holes and explicit undefined as
                                // undefined; number conversion contributes NaN.
                                // State 1 alone carries a numeric payload.
                                HirExpr::Conditional(
                                    Box::new(HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_array_index_state".into(),
                                            )),
                                            vec![var(&source_name), var(&index_name)],
                                        )),
                                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                                    )),
                                    Box::new(HirExpr::TypedIndex(
                                        Box::new(var(&source_name)),
                                        Box::new(var(&index_name)),
                                        HirType::F64,
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(f64::NAN))),
                                    HirType::F64,
                                ),
                            ],
                        )),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(var(&acc_name))),
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
                HirType::F64,
                Box::new(body),
            )),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(result, &[(source_name, array_type, source)])
    }

    /// Converts a statically-typed native value (anything
    /// `json_convertible_native_type` accepts) into a `Json` value, so
    /// `JSON.stringify` can serialize a plain object/array/tuple literal or
    /// variable, not just an already-dynamic `Json`/`Dictionary` value.
    ///
    /// Rather than walking `value_type` itself and re-deriving thaw-llvm's
    /// own native-to-`Json` codegen (`compile_native_object_to_json` and
    /// friends, which already recurse through nested arrays/objects/tuples
    /// correctly), this reuses that existing codegen indirectly: it sets
    /// `value` as the one field of a fresh `Json` object via `JsonSet`
    /// (whose codegen already dispatches on the embedded field type for
    /// every case `json_convertible_native_type` allows, including nested
    /// ones), then reads that field straight back out with `JsonGet`. The
    /// round trip costs one throwaway object-field write per call, but adds
    /// no new codegen and cannot drift from what `console.log`'s own
    /// structured-value printing already does for the same types.
    fn wrap_native_value_as_json(
        &mut self,
        value: HirExpr,
        value_type: HirType,
    ) -> Result<HirExpr, String> {
        // The caller's type is usually the `Bytes`-erased one; the un-erased type
        // keeps a `Buffer` encoding as `{ type: "Buffer", data }` and widens it.
        let value_type = match self.infer_expr_type_inner(&value) {
            Ok(inner) if inner != value_type && bytes_layout_eq(&inner, &value_type) => inner,
            _ => value_type,
        };
        let value_name = format!("__thaw_json_wrap_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(value_name.clone(), value_type.clone());
        let obj_name = format!("__thaw_json_wrap_obj_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(obj_name.clone(), HirType::Json);
        let var = |name: &str| HirExpr::Var(name.into());
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                obj_name.clone(),
                HirType::Json,
                HirExpr::JsonObjectLit(Vec::new(), HirType::Json),
            ),
            HirStmt::Expr(HirExpr::JsonSet(
                Box::new(var(&obj_name)),
                Box::new(HirExpr::Lit(HirLit::Str("value".to_string()))),
                Box::new(var(&value_name)),
                value_type.clone(),
                // This is a *standalone* value being round-tripped
                // through a temporary object's one field and read
                // straight back out, not a real object literal whose
                // field can legitimately omit itself -- omitting it
                // here would make the read-back below see a plain
                // *missing* key (`thaw_json_get` reports that as JSON
                // `null`), silently turning a real `undefined` (an
                // absent `Optional`/`Nullable`/`Nullish` value) into
                // `null` instead. Irrelevant for any other `value_type`
                // (nothing to preserve without an absent/undefined tag
                // to begin with), so always `true` here is safe.
                true,
            )),
            HirStmt::Return(Some(HirExpr::JsonGet(
                Box::new(var(&obj_name)),
                "value".to_string(),
            ))),
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
                HirType::Json,
                Box::new(body),
            )),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(result, &[(value_name, value_type, value)])
    }

    fn is_static_builtin_call(object: &str, property: &str) -> bool {
        matches!(
            (object, property),
            ("Array", "of" | "from" | "isArray" | "fromAsync")
                | (
                    "Buffer",
                    "from" | "alloc" | "allocUnsafe" | "allocUnsafeSlow" | "concat" | "byteLength" | "isBuffer"
                )
                | ("Uint8Array", "fromHex" | "fromBase64" | "from")
                | ("BigInt", "asIntN" | "asUintN")
                | ("Error", "isError" | "captureStackTrace")
                | ("Proxy", "revocable")
                | ("Intl", "getCanonicalLocales" | "supportedValuesOf")
                | ("Map", "groupBy")
                | (
                    "Object",
                    "groupBy"
                        | "keys"
                        | "getOwnPropertyNames"
                        | "values"
                        | "entries"
                        | "fromEntries"
                        | "assign"
                        | "hasOwn"
                        | "is"
                        | "freeze"
                        | "seal"
                        | "preventExtensions"
                        | "isFrozen"
                        | "isSealed"
                        | "isExtensible"
                        | "getOwnPropertyDescriptor"
                        | "getOwnPropertyDescriptors"
                        | "defineProperty"
                        | "defineProperties"
                        | "create"
                        | "getPrototypeOf"
                        | "setPrototypeOf"
                        | "getOwnPropertySymbols"
                )
                | ("JSON", "stringify" | "parse" | "rawJSON" | "isRawJSON")
                | ("Iterator", "from" | "concat" | "zip" | "zipKeyed")
                | ("RegExp", "escape")
                | (
                    "Reflect",
                    "ownKeys"
                        | "has"
                        | "get"
                        | "set"
                        | "deleteProperty"
                        | "apply"
                        | "construct"
                        | "defineProperty"
                        | "getOwnPropertyDescriptor"
                        | "getPrototypeOf"
                        | "setPrototypeOf"
                        | "isExtensible"
                        | "preventExtensions"
                )
                | ("Symbol", "for" | "keyFor")
                | (
                    "Number",
                    "parseFloat"
                        | "parseInt"
                        | "isNaN"
                        | "isFinite"
                        | "isInteger"
                        | "isSafeInteger"
                )
                | ("String", "fromCharCode" | "fromCodePoint" | "raw")
                | (
                    "Math",
                    "sumPrecise"
                        | "random"
                        | "abs"
                        | "floor"
                        | "ceil"
                        | "trunc"
                        | "sqrt"
                        | "exp"
                        | "log"
                        | "log2"
                        | "log10"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "asin"
                        | "acos"
                        | "atan"
                        | "sinh"
                        | "cosh"
                        | "tanh"
                        | "cbrt"
                        | "acosh"
                        | "asinh"
                        | "atanh"
                        | "expm1"
                        | "log1p"
                        | "f16round"
                        | "fround"
                        | "clz32"
                        | "pow"
                        | "min"
                        | "max"
                        | "sign"
                        | "round"
                        | "atan2"
                        | "hypot"
                        | "imul"
                )
                | ("Date", "now" | "UTC" | "parse")
                | ("performance", "now")
        )
    }

    /// A callable field, directly or inside a nested object: `JSON.stringify` must then
    /// see the live object (a nested `toJSON` method is called there).
    fn type_holds_callable_field(ty: &HirType) -> bool {
        match ty {
            HirType::Function(_, _) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..) => true,
            HirType::Object(fields) => fields
                .iter()
                .any(|(name, field)| !is_hidden_accessor_field(name) && Self::type_holds_callable_field(field)),
            // (An array of callables keeps its null-element serialisation.)
            HirType::Optional(element) | HirType::Nullable(element) | HirType::Nullish(element)
                if matches!(element.as_ref(), HirType::Object(_)) =>
            {
                Self::type_holds_callable_field(element)
            }
            _ => false,
        }
    }

    fn lower_static_builtin_call(
        &mut self,
        object: &swc_ecma_ast::Ident,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                    if object.sym == *"Symbol" {
                        let label = format!("Symbol.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        // `Symbol.for` takes a string description;
                        // `Symbol.keyFor` takes the symbol itself.
                        let value = if property.sym == *"for" {
                            self.coerce_primitive_to_string(value.clone())?
                        } else {
                            value.clone()
                        };
                        let intrinsic = if property.sym == *"for" {
                            "__thaw_symbol_for"
                        } else {
                            "__thaw_symbol_key_for"
                        };
            let result = HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), vec![value]);
                        if property.sym == *"keyFor" {
                            let raw_name = format!("__thaw_symbol_key_raw_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(raw_name.clone(), HirType::Str);
                            let body = HirExpr::Block(vec![
                                HirStmt::Let(raw_name.clone(), HirType::Str, result),
                                HirStmt::If(
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                        vec![HirExpr::Var(raw_name.clone())],
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::OptionalNone(HirType::Str)))],
                                    Vec::new(),
                                ),
                                HirStmt::Return(Some(HirExpr::OptionalSome(
                                    Box::new(HirExpr::Var(raw_name)),
                                    HirType::Str,
                                ))),
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
                                    HirType::Optional(Box::new(HirType::Str)),
                                    Box::new(body),
                                )),
                                Vec::new(),
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object"
                        && matches!(
                            property.sym.as_ref(),
                            "freeze" | "seal" | "preventExtensions"
                        )
                    {
                        let label = format!("Object.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let value = value.clone();
                        let value_type = self.infer_expr_type(&value)?;
                        if value_type == HirType::JsValue {
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let operation = match property.sym.as_ref() {
                            "preventExtensions" => 1.0,
                            "seal" => 2.0,
                            _ => 3.0,
                        };
                        if value_type == HirType::Json {
                            let call_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![value.clone()]),
                            )?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let dynamic = HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            );
                            let native = HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    "__thaw_object_set_state_and_return".to_string(),
                                )),
                                vec![value.clone(), HirExpr::Lit(HirLit::F64(operation))],
                            );
                            let result = HirExpr::Conditional(
                                Box::new(json_has_borrowed_handle(value)),
                                Box::new(dynamic),
                                Box::new(native),
                                HirType::Json,
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(
                                "__thaw_object_set_state_and_return".to_string(),
                            )),
                            vec![value, HirExpr::Lit(HirLit::F64(operation))],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object"
                        && matches!(
                            property.sym.as_ref(),
                            "isFrozen" | "isSealed" | "isExtensible"
                        )
                    {
                        let label = format!("Object.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let value_type = self.infer_expr_type(value)?;
                        if value_type == HirType::JsValue {
                let call_arguments = self
                    .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value.clone()]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if value_type == HirType::Json {
                            let call_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![value.clone()]),
                            )?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let dynamic = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            )));
                            let query = match property.sym.as_ref() {
                                "isExtensible" => 0.0,
                                "isSealed" => 1.0,
                                _ => 2.0,
                            };
                            let native = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                                vec![value.clone(), HirExpr::Lit(HirLit::F64(query))],
                            );
                            let result = HirExpr::Conditional(
                                Box::new(json_has_borrowed_handle(value.clone())),
                                Box::new(dynamic),
                                Box::new(native),
                                HirType::Bool,
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let primitive = matches!(
                            value_type,
                            HirType::F64
                                | HirType::I64 | HirType::BigInt
                                | HirType::Str
                                | HirType::StrLiteral(_)
                                | HirType::Symbol
                                | HirType::Bool
                                | HirType::Null
                                | HirType::Undefined
                        );
                        if primitive {
                            let result = property.sym != *"isExtensible";
                return self
                    .wrap_call_argument_bindings(HirExpr::Lit(HirLit::Bool(result)), &bindings);
                        }
                        let query = match property.sym.as_ref() {
                            "isExtensible" => 0.0,
                            "isSealed" => 1.0,
                            _ => 2.0,
                        };
            let state = |value: &HirExpr, query: f64| {
                HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                            vec![value.clone(), HirExpr::Lit(HirLit::F64(query))],
                )
            };
                        // A non-extensible object with no own properties is vacuously
                        // sealed and frozen, whatever operation made it so.
                        if query != 0.0
                            && matches!(&value_type, HirType::Object(fields) if ecmascript_field_order(fields).is_empty())
                        {
                            let name = format!("__thaw_integrity_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), value_type.clone());
                            let bound = HirExpr::Var(name.clone());
                            let result = HirExpr::Conditional(
                                Box::new(state(&bound, 0.0)),
                                Box::new(state(&bound, query)),
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                HirType::Bool,
                            );
                            let mut bindings = bindings;
                            bindings.push((name, value_type, value.clone()));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let result = state(value, query);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"getOwnPropertyDescriptor"
                    {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "Object.getOwnPropertyDescriptor")?;
                        let [target, key] = arguments.as_slice() else {
                            return Err(
                    "`Object.getOwnPropertyDescriptor` expects exactly two arguments".into(),
                            );
                        };
                        // A `Json`/`any` receiver may hold a live native-owner wrapper;
                        // route it through the same dynamic call so the wrapper's own
                        // descriptor traps (owner attribute side tables) answer.
            if matches!(
                self.infer_expr_type(target)?,
                HirType::JsValue | HirType::Json
            ) {
                            let key = if self.infer_expr_type(key)? == HirType::JsValue {
                                key.clone()
                            } else {
                                self.coerce_to_declared(&HirType::Json, key.clone())?
                            };
                            let arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![target.clone(), key]),
                            )?;
                            let holder = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(object.sym.to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    holder,
                        HirExpr::Lit(HirLit::Str("getOwnPropertyDescriptor".to_string())),
                                    arguments,
                                ],
                            );
                            // The descriptor comes from a snapshot, so the recorded
                            // seal/freeze state of the target is applied to it.
                            if self.infer_expr_type(target)? == HirType::Json {
                                let result = self.coerce_to_declared(&HirType::Json, result)?;
                                let descriptor = format!("__thaw_descriptor_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(descriptor.clone(), HirType::Json);
                                let descriptor_var = HirExpr::Var(descriptor.clone());
                    let state = |query: f64| {
                        HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                                    vec![target.clone(), HirExpr::Lit(HirLit::F64(query))],
                        )
                    };
                                let set_false = |this: &mut Self, name: &str| -> Result<HirStmt, String> {
                        let value = this.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::Lit(HirLit::Bool(false)),
                        )?;
                                    Ok(HirStmt::Expr(HirExpr::JsonSet(
                                        Box::new(descriptor_var.clone()),
                                        Box::new(HirExpr::Lit(HirLit::Str(name.to_string()))),
                                        Box::new(value),
                                        HirType::Json,
                                        false,
                                    )))
                                };
                                let configurable = set_false(self, "configurable")?;
                                let writable = set_false(self, "writable")?;
                                let is_data = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_has_own".to_string())),
                        vec![
                            descriptor_var.clone(),
                            HirExpr::Lit(HirLit::Str("value".to_string())),
                        ],
                                );
                                let adjust = HirStmt::If(
                                    state(2.0),
                                    vec![
                                        configurable.clone(),
                                        HirStmt::If(is_data, vec![writable], Vec::new()),
                                    ],
                                    vec![HirStmt::If(state(1.0), vec![configurable], Vec::new())],
                                );
                                let present = HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_is_undefined".to_string())),
                                        vec![descriptor_var.clone()],
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                );
                                let block = HirExpr::Block(vec![
                                    HirStmt::Let(descriptor.clone(), HirType::Json, result),
                                    HirStmt::If(present, vec![adjust], Vec::new()),
                                    HirStmt::Return(Some(descriptor_var)),
                                ]);
                                return self.wrap_call_argument_bindings(block, &bindings);
                            }
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let key = if let HirExpr::Lit(HirLit::Str(key)) = key {
                            key.clone()
                        } else if call.args.len() == 2 && call.args[1].spread.is_none() {
                            // The shared argument binder turns even a literal into a
                            // Var; inspect source syntax while still evaluating its
                            // bound argument in the usual left-to-right order.
                            let Expr::Lit(Lit::Str(key)) = call.args[1].expr.as_ref() else {
                    return Err(
                        "`Object.getOwnPropertyDescriptor` currently requires a string-literal key"
                            .into(),
                    );
                            };
                            key.value.to_string_lossy().into_owned()
                        } else {
                return Err(
                    "`Object.getOwnPropertyDescriptor` currently requires a string-literal key"
                        .into(),
                );
                        };
                        let target_type = self.infer_expr_type(target)?;
                        let HirType::Object(fields) = &target_type else {
                            return Err(format!(
                                "`Object.getOwnPropertyDescriptor` currently requires a fixed \
                                 object, got {target_type:?}"
                            ));
                        };
                        if !fields.iter().any(|(name, _)| name == &key) {
                return self
                    .wrap_call_argument_bindings(HirExpr::Lit(HirLit::Undefined), &bindings);
                        }
                        if !key.starts_with("__thaw_class_identity_\u{1e}") {
                            let descriptor = self.fixed_object_property_descriptor(
                    target.clone(),
                    &target_type,
                    fields,
                    &key,
                            )?;
                            return self.wrap_call_argument_bindings(descriptor, &bindings);
                        }
                        let mut bindings = bindings;
                        let target_name = format!("__thaw_descriptor_target_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(target_name.clone(), target_type.clone());
                        let bound = HirExpr::Var(target_name.clone());
                        let hidden = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                            vec![bound.clone(), HirExpr::Lit(HirLit::Str(key.clone()))],
                        );
            let descriptor =
                self.fixed_object_property_descriptor(bound, &target_type, fields, &key)?;
                        let descriptor_type = self.infer_expr_type(&descriptor)?;
                        let result = HirExpr::Conditional(
                            Box::new(hidden),
                            Box::new(HirExpr::OptionalNone(descriptor_type.clone())),
                Box::new(HirExpr::OptionalSome(
                    Box::new(descriptor),
                    descriptor_type.clone(),
                )),
                            HirType::Optional(Box::new(descriptor_type)),
                        );
                        bindings.push((target_name, target_type, target.clone()));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"create" {
                        // Keep the actual JS object and its prototype chain
                        // behind a Json host projection so dynamic operations
                        // and `for...in` can observe normal JavaScript behavior.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.create")?;
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err("`Object.create` expects one or two arguments".into());
                        }
                        let mut json_values = Vec::with_capacity(arguments.len());
                        for (index, argument) in arguments.into_iter().enumerate() {
                            let value = if self.infer_expr_type(&argument)? == HirType::JsValue {
                                argument
                            } else if index == 0 {
                                self.coerce_to_declared_aliasing(&HirType::Json, argument)?
                            } else {
                                self.coerce_to_declared(&HirType::Json, argument)?
                            };
                            json_values.push(value);
                        }
                        let arguments = self
                            .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_values))?;
                        let object = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                            vec![
                                object,
                                HirExpr::Lit(HirLit::Str("create".to_string())),
                                arguments,
                            ],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(
                                "__thaw_json_from_consumed_dynamic".to_string(),
                            )),
                            vec![result],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"getPrototypeOf"
                    {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.getPrototypeOf")?;
                        let [_value] = arguments.as_slice() else {
                return Err("`Object.getPrototypeOf` expects exactly one argument".into());
                        };
                        if self.infer_expr_type(&arguments[0])? == HirType::JsValue {
                            let json_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![arguments[0].clone()]),
                            )?;
                            let holder = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(object.sym.to_string()))],
                            );
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    holder,
                                    HirExpr::Lit(HirLit::Str("getPrototypeOf".to_string())),
                                    json_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
            if self.infer_expr_type(&arguments[0])? == HirType::Json {
                let target = arguments[0].clone();
                let native = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_get_prototype".to_string())),
                    vec![target.clone()],
                );
                let json_arguments = self
                    .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![target.clone()]))?;
                let holder = HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".to_string())),
                    vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                );
                let dynamic = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                    vec![
                        holder,
                        HirExpr::Lit(HirLit::Str("getPrototypeOf".to_string())),
                        json_arguments,
                    ],
                );
                let result = HirExpr::Conditional(
                    Box::new(json_has_borrowed_handle(target.clone())),
                    Box::new(dynamic),
                    Box::new(native),
                    HirType::Json,
                );
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            if matches!(self.infer_expr_type(&arguments[0])?, HirType::Dictionary(_)) {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_get_prototype".to_string())),
                                vec![arguments[0].clone()],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        // A native class instance's real prototype is its
                        // class token's `.prototype` (the same stable
                        // `JsValue` a bare `C.prototype` resolves to, see
                        // `class_decorator_token_init`), so
                        // `Object.getPrototypeOf(new C()) === C.prototype`
                        // holds. Any other fixed-layout native object still
                        // has no modeled prototype, so report `null`.
                        let argument_type = self.infer_expr_type(&arguments[0])?;
                        if let Some(class_name) = class_name_from_type(&argument_type) {
                            let token = class_decorator_token_symbol(class_name);
                            if self.scope.contains_key(&token) {
                                let prototype = self.lower_dynamic_value_property_read(
                                    HirExpr::Var(token),
                                    HirExpr::Lit(HirLit::Str("prototype".to_string())),
                                )?;
                                return self.wrap_call_argument_bindings(prototype, &bindings);
                            }
                        }
                        // Approx: thaw models no prototype chain for any
                        // other fixed-layout native object, so report `null`.
            return self.wrap_call_argument_bindings(HirExpr::Lit(HirLit::Null), &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"setPrototypeOf" {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "Object.setPrototypeOf")?;
                        let [target, prototype] = arguments.as_slice() else {
                return Err("`Object.setPrototypeOf` expects exactly two arguments".into());
                        };
                        if self.infer_expr_type(target)? == HirType::JsValue {
                            let prototype = if self.infer_expr_type(prototype)? == HirType::JsValue {
                                prototype.clone()
                            } else {
                                self.coerce_to_declared(&HirType::Json, prototype.clone())?
                            };
                            let json_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![target.clone(), prototype]),
                            )?;
                            let holder = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    holder,
                                    HirExpr::Lit(HirLit::Str("setPrototypeOf".to_string())),
                                    json_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if self.infer_expr_type(target)? == HirType::Json {
                let prototype = self.coerce_to_declared(&HirType::Json, prototype.clone())?;
                let native = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_set_prototype".to_string())),
                    vec![target.clone(), prototype.clone()],
                );
                let json_arguments = self.coerce_to_declared(
                    &HirType::Json,
                    HirExpr::ArrayLit(vec![target.clone(), prototype]),
                )?;
                let holder = HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".to_string())),
                    vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                );
                let dynamic = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                    vec![
                        holder,
                        HirExpr::Lit(HirLit::Str("setPrototypeOf".to_string())),
                        json_arguments,
                    ],
                );
                let result = HirExpr::Conditional(
                    Box::new(json_has_borrowed_handle(target.clone())),
                    Box::new(dynamic),
                    Box::new(native),
                    HirType::Json,
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        return Err(
                "`Object.setPrototypeOf` is not supported for fixed-layout native objects".into(),
                        );
                    }
                    if object.sym == *"Object" && property.sym == *"getOwnPropertySymbols" {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "Object.getOwnPropertySymbols")?;
                        let [target] = arguments.as_slice() else {
                return Err("`Object.getOwnPropertySymbols` expects exactly one argument".into());
                        };
                        if matches!(
                            self.infer_expr_type(target)?,
                            HirType::JsValue | HirType::Json
                        ) {
                let call_arguments = self
                    .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![target.clone()]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                        HirExpr::Lit(HirLit::Str("getOwnPropertySymbols".to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let symbols = match self.infer_expr_type(target)? {
                            HirType::Object(fields) => fields
                                .into_iter()
                                .filter_map(|(name, _)| {
                        name.strip_prefix("\u{1f}@@")
                            .map(|symbol| HirExpr::Lit(HirLit::Str(format!("\u{1f}@@{symbol}"))))
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
            return self.wrap_call_argument_bindings(HirExpr::ArrayLit(symbols), &bindings);
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"defineProperty"
                    {
                        // Fixed objects can update data fields or existing
                        // accessor slots, but cannot change their layout.
                        let [target, key, descriptor] = call.args.as_slice() else {
                return Err("`Object.defineProperty` expects exactly three arguments".into());
                        };
                        if self.peek_type_without_lowering(&target.expr) == Some(HirType::JsValue) {
                            let label = format!("{}.defineProperty", object.sym);
                let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                            let json_arguments = arguments
                                .iter()
                                .map(|argument| {
                                    if self.infer_expr_type(argument)? == HirType::JsValue {
                                        Ok(argument.clone())
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, argument.clone())
                                    }
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                let json_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_arguments))?;
                            let holder = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(object.sym.to_string()))],
                            );
                            let call = HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    if object.sym == *"Reflect" {
                                        "callDynamicMethod"
                                    } else {
                                        "callDynamicMethodHandle"
                                    }
                                    .to_string(),
                                )),
                                vec![
                                    holder,
                                    HirExpr::Lit(HirLit::Str("defineProperty".to_string())),
                                    json_arguments,
                                ],
                            );
                            let result = if object.sym == *"Reflect" {
                                HirExpr::JsonAsBool(Box::new(call))
                            } else {
                                call
                            };
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        // A `Json`/`any` target with a *dynamic* key
                        // (`Object.defineProperty(o, k, { value: ... })`):
                        // write the descriptor's `value` through the dynamic
                        // object path (only `value` is modelled; accessor
                        // descriptors are ignored).
                        let dynamic_target = self.lower_expr(target.expr.as_ref())?;
                        let dynamic_target_type = self.infer_expr_type(&dynamic_target)?;
                        // The target is evaluated once: a call or other effectful
                        // expression is bound to a temporary shared by the write
                        // and the result.
                        let mut target_bindings = Vec::new();
                        let dynamic_target = if matches!(dynamic_target, HirExpr::Var(_)) {
                            dynamic_target
            } else if matches!(dynamic_target_type, HirType::Json | HirType::Dictionary(_)) {
                            let name = format!("__thaw_define_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), dynamic_target_type.clone());
                            target_bindings.push((name.clone(), dynamic_target_type.clone(), dynamic_target));
                            HirExpr::Var(name)
                        } else {
                            dynamic_target
                        };
                        let dynamic_target_var = dynamic_target.clone();
            if matches!(dynamic_target_type, HirType::Json | HirType::Dictionary(_)) {
                            if let Expr::Object(descriptor_object) = descriptor.expr.as_ref() {
                                // Every descriptor member expression runs once, in source
                                // order; only an explicit `value` is written, so a
                                // descriptor without one keeps the existing value.
                                let mut descriptor_steps: Vec<(bool, Box<Expr>)> = Vec::new();
                                for prop in &descriptor_object.props {
                                    if let PropOrSpread::Prop(prop) = prop {
                                        if let Prop::KeyValue(pair) = prop.as_ref() {
                                            let is_value = matches!(&pair.key, PropName::Ident(key) if key.sym == *"value");
                                            descriptor_steps.push((is_value, pair.value.clone()));
                                        }
                                    }
                                }
                                let key_value = self.lower_expr(key.expr.as_ref())?;
                                let key_value = self.coerce_primitive_to_string(key_value)?;
                                let mut body_statements = Vec::new();
                                let mut value = None;
                                for (is_value, expression) in descriptor_steps {
                                    let lowered = self.lower_expr(&expression)?;
                                    if is_value {
                                        let lowered = self.coerce_to_declared(&HirType::Json, lowered)?;
                                        let name = format!("__thaw_define_value_{}", self.next_binding);
                                        self.next_binding += 1;
                                        self.scope.insert(name.clone(), HirType::Json);
                                        body_statements.push(HirStmt::Let(
                                            name.clone(),
                                            HirType::Json,
                                            lowered,
                                        ));
                                        value = Some(HirExpr::Var(name));
                                    } else {
                                        body_statements.push(HirStmt::Expr(lowered));
                                    }
                                }
                                let set = value.map(|value| {
                                    HirExpr::JsonSet(
                                        Box::new(dynamic_target.clone()),
                                        Box::new(key_value),
                                        Box::new(value),
                                        HirType::Json,
                                        false,
                                    )
                                });
                                let (result, return_type) = if object.sym == *"Reflect" {
                                    (HirExpr::Lit(HirLit::Bool(true)), HirType::Bool)
                                } else {
                                    (dynamic_target, dynamic_target_type.clone())
                                };
                                if let Some(set) = set {
                                    body_statements.push(HirStmt::Expr(set));
                                }
                                body_statements.push(HirStmt::Return(Some(result)));
                                let body = HirExpr::Block(body_statements);
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
                                let existing = HirExpr::Call(
                                    Box::new(HirExpr::Lambda(
                                        captures,
                                        Vec::new(),
                                        return_type.clone(),
                                        Box::new(body),
                                    )),
                                    Vec::new(),
                                );
                    // A live host object carried by `Json` must receive the
                    // descriptor itself (attributes live on the owner), not a
                    // detached `value` write. Plain native JSON still uses the
                    // local property-write fallback below.
                                if dynamic_target_type == HirType::Json
                        && matches!(&dynamic_target_var, HirExpr::Var(_))
                    {
                                    let label = format!("{}.defineProperty", object.sym);
                                    let HirExpr::Var(target_name) = &dynamic_target_var else {
                                        unreachable!("host-backed target was bound above")
                                    };
                                    let mut host_call_args = call.args.clone();
                                    *host_call_args[0].expr = Expr::Ident(
                                        swc_ecma_ast::Ident::new_no_ctxt(
                                            target_name.clone().into(),
                                            call.span,
                                        ),
                                    );
                                    let (arguments, bindings) =
                                        self.lower_native_spread_values(&host_call_args, &label)?;
                                    let json_arguments = arguments
                                        .iter()
                                        .map(|argument| {
                                            if self.infer_expr_type(argument)? == HirType::JsValue {
                                                Ok(argument.clone())
                                            } else {
                                                self.coerce_to_declared(&HirType::Json, argument.clone())
                                            }
                                        })
                                        .collect::<Result<Vec<_>, String>>()?;
                                    let json_arguments = self.coerce_to_declared(
                                        &HirType::Json,
                                        HirExpr::ArrayLit(json_arguments),
                                    )?;
                                    let holder = HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                        vec![HirExpr::Lit(HirLit::Str(object.sym.to_string()))],
                                    );
                                    let dynamic = HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            if object.sym == *"Reflect" {
                                                "callDynamicMethod"
                                            } else {
                                                "callDynamicMethodHandle"
                                            }
                                            .to_string(),
                                        )),
                                        vec![
                                            holder,
                                            HirExpr::Lit(HirLit::Str("defineProperty".to_string())),
                                            json_arguments,
                                        ],
                                    );
                                    let dynamic = if object.sym == *"Reflect" {
                                        HirExpr::JsonAsBool(Box::new(dynamic))
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, dynamic)?
                                    };
                                    let dynamic = self.wrap_call_argument_bindings(dynamic, &bindings)?;
                                    return self.wrap_call_argument_bindings(
                                        HirExpr::Conditional(
                                Box::new(HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                            Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var(
                                            "__thaw_json_borrowed_handle_id".into(),
                                        )),
                                                vec![dynamic_target_var],
                                            )),
                                    Box::new(HirExpr::Lit(HirLit::I64(0))),
                                )),
                                            Box::new(existing),
                                Box::new(dynamic),
                                            return_type,
                                        ),
                                        &target_bindings,
                                    );
                                }
                                return self.wrap_call_argument_bindings(existing, &target_bindings);
                            }
                        }
                        // A string literal, or a `const` bound to one.
                        let Some(key) = self.static_property_name(&key.expr) else {
                            return Err(
                    "`Object.defineProperty` currently requires a string-literal key".into(),
                            );
                        };
                        let Expr::Object(descriptor) = descriptor.expr.as_ref() else {
                return Err("`Object.defineProperty` requires an object-literal descriptor".into());
                        };
                        let target_value = self.lower_expr(target.expr.as_ref())?;
                        let target_type = self.infer_expr_type(&target_value)?;
                        // One evaluation of the target serves the write and the result.
                        let mut fixed_target_bindings = Vec::new();
                        let target_value = if matches!(target_value, HirExpr::Var(_)) {
                            target_value
                        } else {
                            let name = format!("__thaw_define_fixed_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), target_type.clone());
                            fixed_target_bindings.push((name.clone(), target_type.clone(), target_value));
                            HirExpr::Var(name)
                        };
                        let label = format!("{}.defineProperty", object.sym);
                        let assignments = self.lower_native_property_descriptor(
                            target_value.clone(),
                            &target_type,
                            &key,
                            descriptor,
                            &label,
                        )?;
                        if object.sym == *"Reflect" {
                            if let [HirExpr::PropAssign(_, _, field, value)] = assignments.as_slice() {
                                if field == &key && matches!(&target_type, HirType::Object(_)) {
                                    // Bind target before descriptor value, including on failure.
                        let target_name =
                            format!("__thaw_reflect_define_target_{}", self.next_binding);
                                    self.next_binding += 1;
                        let value_name =
                            format!("__thaw_reflect_define_value_{}", self.next_binding);
                                    self.next_binding += 1;
                                    let value_type = self.infer_expr_type(value)?;
                                    self.scope.insert(target_name.clone(), target_type.clone());
                                    self.scope.insert(value_name.clone(), value_type.clone());
                                    let bound_target = HirExpr::Var(target_name.clone());
                                    let mut body = Vec::new();
                                    if key.starts_with("__thaw_class_identity_\u{1e}") {
                                        let hidden = HirExpr::Call(
                                            Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                                            vec![bound_target.clone(), HirExpr::Lit(HirLit::Str(key.clone()))],
                                        );
                                        let extensible = HirExpr::Call(
                                            Box::new(HirExpr::Var("__thaw_object_state".into())),
                                            vec![bound_target.clone(), HirExpr::Lit(HirLit::F64(0.0))],
                                        );
                            body.push(HirStmt::If(
                                hidden,
                                vec![HirStmt::If(
                                    extensible,
                                    Vec::new(),
                                            vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                                )],
                                Vec::new(),
                            ));
                                    }
                                    let frozen = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_object_state".into())),
                                        vec![bound_target.clone(), HirExpr::Lit(HirLit::F64(2.0))],
                                    );
                        body.push(HirStmt::If(
                            frozen,
                                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                            Vec::new(),
                        ));
                                    body.push(HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(bound_target),
                            target_type.clone(),
                            key.clone(),
                                        Box::new(HirExpr::Var(value_name.clone())),
                                    )));
                                    body.push(HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))));
                                    let mut reflect_bindings = fixed_target_bindings.clone();
                                    reflect_bindings.push((target_name, target_type, target_value));
                        reflect_bindings.push((value_name, value_type, value.as_ref().clone()));
                        return self
                            .wrap_call_argument_bindings(HirExpr::Block(body), &reflect_bindings);
                                }
                            }
                        }
                        // `Object.defineProperty` returns the object;
                        // `Reflect.defineProperty` returns a boolean. Run
                        // the assignment for its side effect and hand back
                        // the appropriate result.
                        let (return_value, return_type) = if object.sym == *"Reflect" {
                            (HirExpr::Lit(HirLit::Bool(true)), HirType::Bool)
                        } else {
                            (target_value, target_type)
                        };
                        let mut body = assignments
                            .into_iter()
                            .map(HirStmt::Expr)
                            .collect::<Vec<_>>();
                        body.push(HirStmt::Return(Some(return_value)));
                        let body = HirExpr::Block(body);
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
                                return_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        return self.wrap_call_argument_bindings(result, &fixed_target_bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"getOwnPropertyDescriptors" {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "Object.getOwnPropertyDescriptors")?;
                        let [target] = arguments.as_slice() else {
                            return Err(
                    "`Object.getOwnPropertyDescriptors` expects exactly one argument".into(),
                            );
                        };
                        let target_type = self.infer_expr_type(target)?;
            if matches!(target_type, HirType::JsValue | HirType::Json) {
                let arguments = self
                    .coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![target.clone()]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                        HirExpr::Lit(HirLit::Str("getOwnPropertyDescriptors".to_string())),
                                    arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &target_type else {
                            return Err(format!(
                                "`Object.getOwnPropertyDescriptors` currently requires a fixed \
                                 object, got {target_type:?}"
                            ));
                        };
                        let source_name = format!("__thaw_descriptor_map_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(source_name.clone(), target_type.clone());
                        let source = HirExpr::Var(source_name.clone());
                        let mut descriptors = Vec::new();
                        let mut map_fields = Vec::new();
                        for index in ecmascript_field_order(fields) {
                            let name = fields[index].0.clone();
                            let descriptor = self.fixed_object_property_descriptor(
                    source.clone(),
                    &target_type,
                    fields,
                    &name,
                            )?;
                            let descriptor_type = self.infer_expr_type(&descriptor)?;
                            let marker = name.starts_with("__thaw_class_identity_\u{1e}");
                            let field_type = if marker {
                                HirType::Optional(Box::new(descriptor_type.clone()))
                            } else {
                                descriptor_type.clone()
                            };
                            map_fields.push((name.clone(), field_type));
                            descriptors.push((index, name, descriptor, descriptor_type, marker));
                        }
                        let map_type = HirType::Object(map_fields);
                        let map_name = format!("__thaw_descriptor_map_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(map_name.clone(), map_type.clone());
                        let map = HirExpr::Var(map_name.clone());
                        let mut statements = vec![HirStmt::Let(
                map_name,
                map_type.clone(),
                HirExpr::ObjectAlloc(map_type.clone()),
                        )];
                        statements.push(HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_order_begin".into())),
                            vec![map.clone()],
                        )));
                        for (_, name, _, _, marker) in &descriptors {
                            if *marker {
                                statements.push(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_object_hide_marker".into())),
                                    vec![map.clone(), HirExpr::Lit(HirLit::Str(name.clone()))],
                                )));
                            }
                        }
                        let mut actions = Vec::new();
                        for (index, name, descriptor, descriptor_type, marker) in descriptors {
                            let value = if marker {
                                HirExpr::OptionalSome(Box::new(descriptor), descriptor_type)
                            } else {
                                descriptor
                            };
                            let write = HirStmt::Expr(HirExpr::PropAssign(
                    Box::new(map.clone()),
                    map_type.clone(),
                    name.clone(),
                    Box::new(value),
                            ));
                            let seed = HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                                vec![map.clone(), HirExpr::Lit(HirLit::Str(name))],
                            ));
                            actions.push((index, vec![write, seed]));
                        }
                        statements.extend(self.lower_ordered_fixed_field_statements(
                source.clone(),
                fields,
                actions,
                        )?);
                        statements.push(HirStmt::Return(Some(map)));
                        let result = self.wrap_call_argument_bindings(
                HirExpr::Block(statements),
                &[(source_name, target_type, target.clone())],
                        )?;
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"defineProperties" {
                        // Desugars to descriptor assignments over literal
                        // keys with the same fixed-layout limits as
                        // `Object.defineProperty`.
                        let [target, descriptors] = call.args.as_slice() else {
                return Err("`Object.defineProperties` expects exactly two arguments".into());
                        };
                        if self.peek_type_without_lowering(&target.expr) == Some(HirType::JsValue) {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Object.defineProperties")?;
                            let arguments = arguments
                                .iter()
                                .map(|argument| {
                                    if self.infer_expr_type(argument)? == HirType::JsValue {
                                        Ok(argument.clone())
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, argument.clone())
                                    }
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                let arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(arguments))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("defineProperties".to_string())),
                                    arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let Expr::Object(descriptors) = descriptors.expr.as_ref() else {
                            return Err(
                    "`Object.defineProperties` requires an object-literal descriptor".into(),
                            );
                        };
                        let target_value = self.lower_expr(target.expr.as_ref())?;
                        let target_type = self.infer_expr_type(&target_value)?;
                        // One evaluation of the target serves every descriptor and the result.
                        let mut target_bindings = Vec::new();
                        let target_value = if matches!(target_value, HirExpr::Var(_)) {
                            target_value
                        } else {
                            let name = format!("__thaw_define_properties_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), target_type.clone());
                            target_bindings.push((name.clone(), target_type.clone(), target_value));
                            HirExpr::Var(name)
                        };
                        let mut body = Vec::new();
                        for property in &descriptors.props {
                            let swc_ecma_ast::PropOrSpread::Prop(property) = property else {
                    return Err("`Object.defineProperties` descriptors must not spread".into());
                            };
                            let swc_ecma_ast::Prop::KeyValue(property) = property.as_ref() else {
                                return Err(
                                    "`Object.defineProperties` descriptors must use `key: value` entries"
                                        .into(),
                                );
                            };
                            let key = match &property.key {
                                swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                                swc_ecma_ast::PropName::Str(value) => {
                                    value.value.to_string_lossy().into_owned()
                                }
                    _ => return Err("`Object.defineProperties` requires literal keys".into()),
                            };
                            let Expr::Object(descriptor) = property.value.as_ref() else {
                                return Err(format!(
                                    "`Object.defineProperties` descriptor for `{key}` must be an object literal"
                                ));
                            };
                            body.extend(
                                self.lower_native_property_descriptor(
                                    target_value.clone(),
                                    &target_type,
                                    &key,
                                    descriptor,
                                    "Object.defineProperties",
                                )?
                                .into_iter()
                                .map(HirStmt::Expr),
                            );
                        }
                        body.push(HirStmt::Return(Some(target_value)));
                        let body = HirExpr::Block(body);
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
                        let lambda = HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                target_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        return self.wrap_call_argument_bindings(lambda, &target_bindings);
                    }
                    if object.sym == *"Reflect"
                        && matches!(
                            property.sym.as_ref(),
                            "setPrototypeOf" | "isExtensible" | "preventExtensions"
                        )
                    {
                        let label = format!("Reflect.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
            let expected = if property.sym == *"setPrototypeOf" {
                2
            } else {
                1
            };
                        if arguments.len() != expected {
                            return Err(format!("`{label}` expects {expected} argument(s)"));
                        }
                        let target = arguments[0].clone();
                        if property.sym != *"setPrototypeOf"
                            && self.infer_expr_type(&target)? == HirType::JsValue
                        {
                let json_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![target]))?;
                            let reflect = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Reflect".to_string()))],
                            );
                            let result = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    reflect,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    json_arguments,
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if property.sym == *"setPrototypeOf"
                            && self.infer_expr_type(&target)? == HirType::JsValue
                        {
                let prototype = if self.infer_expr_type(&arguments[1])? == HirType::JsValue {
                                arguments[1].clone()
                            } else {
                                self.coerce_to_declared(&HirType::Json, arguments[1].clone())?
                            };
                            let json_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![target, prototype]),
                            )?;
                            let reflect = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Reflect".to_string()))],
                            );
                            let result = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    reflect,
                                    HirExpr::Lit(HirLit::Str("setPrototypeOf".to_string())),
                                    json_arguments,
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
            let dynamic_json_set = if self.infer_expr_type(&target)? == HirType::Json {
                let mut json_values = vec![target.clone()];
                if property.sym == *"setPrototypeOf" {
                    let prototype = if self.infer_expr_type(&arguments[1])? == HirType::JsValue {
                        arguments[1].clone()
                    } else {
                        self.coerce_to_declared_aliasing(
                            &HirType::Json,
                            arguments[1].clone(),
                        )?
                    };
                    json_values.push(prototype);
                }
                let json_arguments = self.coerce_to_declared(
                    &HirType::Json,
                    HirExpr::ArrayLit(json_values),
                )?;
                let reflect = HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".to_string())),
                    vec![HirExpr::Lit(HirLit::Str("Reflect".to_string()))],
                );
                let dynamic = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                    vec![
                        reflect,
                        HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                        json_arguments,
                    ],
                )));
                Some((json_has_borrowed_handle(target.clone()), dynamic))
            } else {
                None
            };
                        let result = if property.sym == *"preventExtensions" {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_set_state".to_string())),
                                vec![target, HirExpr::Lit(HirLit::F64(1.0))],
                            )
                        } else if property.sym == *"isExtensible" {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                                vec![target, HirExpr::Lit(HirLit::F64(0.0))],
                            )
                        } else if property.sym == *"setPrototypeOf"
                            && self.infer_expr_type(&target)? == HirType::Json
                        {
                            // A dynamic object takes the new prototype only while it is
                            // extensible; the call reports whether it did.
                            let prototype = self.coerce_to_declared(&HirType::Json, arguments[1].clone())?;
                            HirExpr::Conditional(
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                                    vec![target.clone(), HirExpr::Lit(HirLit::F64(0.0))],
                                )),
                                Box::new(HirExpr::EvalThen(
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_json_set_prototype".to_string())),
                                        vec![target, prototype],
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                )),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                HirType::Bool,
                            )
                        } else if matches!(arguments.get(1), Some(HirExpr::Lit(HirLit::Null))) {
                            HirExpr::Lit(HirLit::Bool(true))
                        } else {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                                vec![target, HirExpr::Lit(HirLit::F64(0.0))],
                            )
                        };
            let result = if let Some((has_handle, dynamic)) = dynamic_json_set {
                HirExpr::Conditional(
                    Box::new(has_handle),
                    Box::new(dynamic),
                    Box::new(result),
                    HirType::Bool,
                )
            } else {
                result
            };
            return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Reflect" && property.sym == *"apply" {
                        // `Reflect.apply(target, thisArg, argsList)` is
                        // exactly `target.apply(thisArg, argsList)`, so
                        // reuse the existing function-value `.apply`
                        // lowering instead of duplicating it. (Its own
                        // static-args-length requirement applies, e.g. a
                        // literal array or tuple.)
                        let [target, this_argument, arguments] = call.args.as_slice() else {
                            return Err("`Reflect.apply` expects exactly three arguments".into());
                        };
                        let forwarded = CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                span: call.span,
                                obj: target.expr.clone(),
                                prop: MemberProp::Ident(IdentName::new("apply".into(), call.span)),
                            }))),
                            args: vec![this_argument.clone(), arguments.clone()],
                            type_args: None,
                        };
                        return self.lower_call(&forwarded);
                    }
                    if object.sym == *"Reflect" && property.sym == *"construct" {
                        // `Reflect.construct(Ctor, argsList)` == `new
                        // Ctor(...argsList)`. Thaw's `new` needs a statically
                        // spliced argument list, so only an array *literal*
                        // is accepted here (a spread element would also need
                        // a statically known length).
                        let [target, arguments] = call.args.as_slice() else {
                return Err("`Reflect.construct` expects exactly two arguments".into());
                        };
                        let Expr::Array(array) = arguments.expr.as_ref() else {
                return Err("`Reflect.construct` currently requires an array literal".into());
                        };
                        let arguments = array
                            .elems
                            .iter()
                            .cloned()
                            .collect::<Option<Vec<_>>>()
                            .ok_or("`Reflect.construct` array literal cannot contain holes")?;
                        return self.lower_expr(&Expr::New(swc_ecma_ast::NewExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: target.expr.clone(),
                            args: Some(arguments),
                            type_args: None,
                        }));
                    }
                    if object.sym == *"Reflect" && property.sym == *"set" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Reflect.set")?;
                        if matches!(arguments.len(), 3 | 4)
                            && self.infer_expr_type(&arguments[0])? == HirType::JsValue
                        {
                            let arguments = arguments
                                .iter()
                                .map(|argument| {
                                    if self.infer_expr_type(argument)? == HirType::JsValue {
                                        Ok(argument.clone())
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, argument.clone())
                                    }
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(arguments))?;
                            let reflect = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Reflect".to_string()))],
                            );
                            let result = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    reflect,
                                    HirExpr::Lit(HirLit::Str("set".to_string())),
                                    call_arguments,
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let [target, key, value] = arguments.as_slice() else {
                            return Err("`Reflect.set` expects exactly three arguments".into());
                        };
                        let key = if let HirExpr::Lit(HirLit::Str(key)) = key {
                            key.clone()
                        } else if call.args.len() == 3 && call.args[1].spread.is_none() {
                            // The argument binder turns source literals into Vars.
                            // Retain that binding for left-to-right evaluation.
                            let Expr::Lit(Lit::Str(key)) = call.args[1].expr.as_ref() else {
                                return Err("`Reflect.set` currently requires a string-literal key".into());
                            };
                            key.value.to_string_lossy().into_owned()
                        } else {
                            return Err("`Reflect.set` currently requires a string-literal key".into());
                        };
                        let target_type = self.infer_expr_type(target)?;
                        let value_type = self.infer_expr_type(value)?;
                        let assign = match &target_type {
                            HirType::Object(fields) => {
                                if !fields.iter().any(|(name, _)| name == &key) {
                                    return Err(format!(
                                        "`Reflect.set` cannot add the new field `{key}` to a fixed object"
                                    ));
                                }
                                HirExpr::PropAssign(
                                    Box::new(target.clone()),
                                    target_type.clone(),
                                    key.clone(),
                                    Box::new(value.clone()),
                                )
                            }
                            HirType::Json => HirExpr::JsonSet(
                                Box::new(target.clone()),
                                Box::new(HirExpr::Lit(HirLit::Str(key.clone()))),
                                Box::new(value.clone()),
                                value_type,
                                true,
                            ),
                            other => {
                                return Err(format!(
                                    "`Reflect.set` currently requires a fixed object or JSON \
                                     receiver, got {other:?}"
                    ));
                            }
                        };
                        // `Reflect.set` returns a boolean (true on success),
                        // so run the assignment for its side effect and
                        // return `true` from a zero-argument closure.
                        let frozen = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_state".to_string())),
                            vec![target.clone(), HirExpr::Lit(HirLit::F64(2.0))],
                        );
                        let mut body = Vec::new();
                        if matches!(&target_type, HirType::Object(_))
                            && key.starts_with("__thaw_class_identity_\u{1e}")
                        {
                            let hidden = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                                vec![target.clone(), HirExpr::Lit(HirLit::Str(key.clone()))],
                            );
                            let extensible = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_state".into())),
                                vec![target.clone(), HirExpr::Lit(HirLit::F64(0.0))],
                            );
                body.push(HirStmt::If(
                    hidden,
                    vec![HirStmt::If(
                        extensible,
                        Vec::new(),
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                    )],
                    Vec::new(),
                ));
                        }
                        body.push(HirStmt::If(
                            frozen,
                            vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                            vec![
                                HirStmt::Expr(assign),
                                HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                            ],
                        ));
                        let body = HirExpr::Block(body);
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
                                HirType::Bool,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Reflect"
                        && matches!(property.sym.as_ref(), "has" | "get" | "deleteProperty")
                    {
                        let label = format!("Reflect.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
            let expected = if property.sym == *"get" {
                "two or three"
            } else {
                "two"
            };
                        if (property.sym == *"get" && !matches!(arguments.len(), 2 | 3))
                            || (property.sym != *"get" && arguments.len() != 2)
                        {
                            return Err(format!("`{label}` expects {expected} arguments"));
                        }
                        let target = &arguments[0];
                        let key = &arguments[1];
                        if self.infer_expr_type(target)? == HirType::JsValue {
                            let arguments = arguments
                                .iter()
                                .map(|argument| {
                                    if self.infer_expr_type(argument)? == HirType::JsValue {
                                        Ok(argument.clone())
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, argument.clone())
                                    }
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(arguments))?;
                            let reflect = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Reflect".to_string()))],
                            );
                            let call = HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    if property.sym == *"get" {
                                        "callDynamicMethodHandle"
                                    } else {
                                        "callDynamicMethod"
                                    }
                                    .to_string(),
                                )),
                                vec![
                                    reflect,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            );
                            let result = if property.sym == *"get" {
                                call
                            } else {
                                HirExpr::JsonAsBool(Box::new(call))
                            };
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if property.sym == *"has" {
                            let mut target = target.clone();
                            let mut target_type = self.infer_expr_type(&target)?;
                            if let HirType::Union(members) = &target_type {
                                if members
                                    .iter()
                                    .all(|member| matches!(member, HirType::Array(_)))
                                {
                        (target, target_type) = self.lower_union_array_sequence(target, members)?;
                                }
                            }
                            if matches!(target_type, HirType::Array(_)) {
                                let key = self.coerce_primitive_to_string(key.clone())?;
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_has_property".into())),
                                    vec![target, key],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            // `Reflect.has` is the `in` operator: a dynamic object also
                            // answers for keys inherited through its prototype chain.
                            if matches!(target_type, HirType::Json | HirType::Dictionary(_)) {
                                let key = self.coerce_primitive_to_string(key.clone())?;
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_has".into())),
                                    vec![target, key],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            let result = self.lower_has_own_value(target, key.clone())?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if property.sym == *"deleteProperty" {
                            let mut array_target = target.clone();
                            let mut array_type = self.infer_expr_type(&array_target)?;
                            if let HirType::Union(members) = &array_type {
                    if members
                        .iter()
                        .all(|member| matches!(member, HirType::Array(_)))
                    {
                                    (array_target, array_type) =
                                        self.lower_union_array_sequence(array_target, members)?;
                                }
                            }
                            if matches!(array_type, HirType::Array(_)) {
                                let key = self.coerce_primitive_to_string(key.clone())?;
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_delete_reflect".into())),
                                    vec![array_target, key],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                        }
                        // A literal key resolves against a fixed object's
                        // own fields at lowering time; a `Json` receiver
                        // defers to the runtime.
                        // Arguments arrive bound to `__thaw_native_arg_N`
                        // temporaries, so look a literal up through them.
                        let key_literal = match key {
                            HirExpr::Lit(HirLit::Str(value)) => Some(value.clone()),
                            HirExpr::Var(name) => bindings.iter().find_map(|(bound, _, value)| match value {
                                HirExpr::Lit(HirLit::Str(text)) if bound == name => Some(text.clone()),
                                _ => None,
                            }),
                            _ => None,
                        };
                        let target_type = self.infer_expr_type(target)?;
                        // A primitive receiver (`1 as any`) must reach the runtime
                        // object check, which throws the TypeError.
                        if property.sym == *"deleteProperty"
                            && matches!(target_type, HirType::F64 | HirType::Bool | HirType::Str)
                        {
                            let target = self.coerce_to_declared(&HirType::Json, target.clone())?;
                            let key = self.coerce_primitive_to_string(key.clone())?;
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_object_delete_reflect".into())),
                                vec![target, key],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        match (property.sym.as_ref(), &target_type, key_literal) {
                            ("get", HirType::Object(fields), Some(key)) => {
                                let result = if fields.iter().any(|(name, _)| name == &key) {
                                    HirExpr::PropAccess(Box::new(target.clone()), target_type, key)
                                } else {
                                    HirExpr::Lit(HirLit::Undefined)
                                };
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            ("get", HirType::Json, Some(key)) => {
                                let result = HirExpr::JsonGet(Box::new(target.clone()), key);
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            ("deleteProperty", HirType::Json, Some(key)) => {
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_object_delete_reflect".into())),
                                    vec![target.clone(), HirExpr::Lit(HirLit::Str(key))],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            ("deleteProperty", HirType::Json, None) => {
                                let key = self.coerce_primitive_to_string(key.clone())?;
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_object_delete_reflect".into())),
                                    vec![target.clone(), key],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            _ => {
                                return Err(format!(
                                    "`{label}` currently requires a fixed object or JSON \
                                     receiver and a string-literal key, got {target_type:?}"
                    ));
                            }
                        }
                    }
                    if object.sym == *"Buffer" {
                        if property.sym == *"isBuffer" {
                            let (arguments, mut bindings) =
                                self.lower_native_spread_values(&call.args, "Buffer.isBuffer")?;
                            let [value] = arguments.as_slice() else {
                                return Err("`Buffer.isBuffer` expects exactly one argument".into());
                            };
                            let value = value.clone();
                            // `infer_expr_type_inner`, not `infer_expr_
                            // type`: the outer helper always erases
                            // `HirType::Bytes` to `Array(F64)` for
                            // generic Array-shaped consumers (indexing,
                            // `.length`, iteration, ...) -- every other
                            // method *dispatch* that must actually tell a
                            // real Buffer apart from a plain number array
                            // (`bytes_methods.rs`, `conversion_methods.
                            // rs`, ...) already reads the receiver's real
                            // type this same way, not through the erased
                            // one.
                            let ty = self.infer_expr_type_inner(&value)?;
                            if ty == HirType::Json {
                                let result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_is_buffer".to_string())),
                                    vec![value],
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            // A `JsValue` -- a retained QuickJS handle, real
                            // example: a real npm package's own class method
                            // declared to return `Buffer` (not one of
                            // thaw's own hand-authored ambient builtins, so
                            // `Buffer` never classified as `HirType::Bytes`
                            // for it -- see `allow_native_bytes_type`'s own
                            // doc comment). Its *runtime* value can still
                            // genuinely be a real `Buffer`, so ask the live
                            // engine the same way `instanceof Date` already
                            // does for a `JsValue` receiver
                            // (`dynamic_value_check`), instead of the
                            // static, always-`false` literal below.
                            if ty == HirType::JsValue {
                    let result = self.dynamic_value_check("__thaw_is_buffer_dynamic_value", value);
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            let name = format!("__thaw_is_buffer_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            bindings.push((name, ty.clone(), value));
                            return self.wrap_call_argument_bindings(
                                HirExpr::Lit(HirLit::Bool(ty == HirType::Bytes)),
                                &bindings,
                            );
                        }
                        if property.sym == *"byteLength" {
                            // `Buffer.byteLength(string, encoding?)` -- how
                            // many bytes the string occupies once encoded
                            // (`utf8` default). Handy for a `Content-Length`.
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Buffer.byteLength")?;
                            let text = arguments
                                .first()
                                .ok_or("`Buffer.byteLength` expects a string")?
                                .clone();
                            let encoding = match arguments.get(1) {
                                Some(argument) => argument.clone(),
                                None => HirExpr::Lit(HirLit::Str("utf8".to_string())),
                            };
                            return self.wrap_call_argument_bindings(
                                HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_bytes_byte_length".to_string())),
                                    vec![text, encoding],
                                ),
                                &bindings,
                            );
                        }
                        if property.sym == *"concat" {
                            // `Buffer.concat(list, totalLength?)` -- flatten
                            // an array of byte buffers into one. A given
                            // `totalLength` truncates / zero-pads the
                            // result; omitted, it's the sum of the parts
                            // (passed to the runtime as `-1`).
                            let (arguments, bindings) =
                                self.lower_native_spread_values(&call.args, "Buffer.concat")?;
                            let (list, total) = match arguments.as_slice() {
                                [list] => (list, HirExpr::Lit(HirLit::F64(-1.0))),
                    [list, total] => (list, self.coerce_primitive_to_number(total.clone())?),
                                _ => {
                                    return Err(
                            "`Buffer.concat` expects a list and an optional totalLength".into()
                        );
                                }
                            };
                            // A list held in a dynamic-storage slot (captured by a closure
                            // handed to a JS callback) reads as `Json`: decode it back.
                            let list = &if self.infer_expr_type(list)? == HirType::Json {
                                self.coerce_to_declared(&HirType::Array(Box::new(HirType::Bytes)), list.clone())?
                            } else {
                                list.clone()
                            };
                            // `Buffer.concat([])`: nothing to join, so an empty buffer.
                            if matches!(call.args.first().map(|arg| arg.expr.as_ref()),
                                Some(Expr::Array(array)) if array.elems.is_empty())
                            {
                                return self.wrap_call_argument_bindings(
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_bytes_alloc".to_string())),
                                        vec![HirExpr::Lit(HirLit::F64(0.0))],
                                    ),
                                    &bindings,
                                );
                            }
                            // A list of dynamic buffers (`cipher.update()` results) decodes back.
                            let list = &if matches!(self.infer_expr_type(list)?,
                                HirType::Array(element) if *element == HirType::Json)
                            {
                                self.coerce_to_declared(&HirType::Array(Box::new(HirType::Bytes)), list.clone())?
                            } else {
                                list.clone()
                            };
                            let list_type = self.infer_expr_type(list)?;
                            if !matches!(&list_type, HirType::Array(element)
                                if matches!(element.as_ref(), HirType::Bytes)
                                    || matches!(element.as_ref(),
                                        HirType::Array(inner) if **inner == HirType::F64))
                            {
                                return Err(format!(
                                    "`Buffer.concat` expects an array of byte buffers, got {list_type:?}"
                                ));
                            }
                            return self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_concat".to_string())),
                                    vec![list.clone(), total],
                                ),
                                &bindings,
                            );
                        }
                        if property.sym == *"alloc" || property.sym == *"allocUnsafe" || property.sym == *"allocUnsafeSlow" {
                            let (arguments, bindings) =
                                self.lower_native_spread_values(&call.args, "Buffer.alloc")?;
                            // `Buffer.alloc(size, fill?, encoding?)`: a string fill is
                            // decoded per `encoding` (default utf8), a number fills
                            // with `n & 255`, a byte buffer repeats as given; the
                            // pattern is repeated and truncated to `size`.
                            let (size, fill, encoding) = match arguments.as_slice() {
                                [size] => (size, None, None),
                                [size, fill] => (size, Some(fill), None),
                                [size, fill, encoding] => (size, Some(fill), Some(encoding)),
                                _ => return Err("`Buffer.alloc` expects a size".into()),
                            };
                            let size = self.coerce_primitive_to_number(size.clone())?;
                            let Some(fill) = fill else {
                                return self.wrap_call_argument_bindings(
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_bytes_alloc".to_string())),
                                        vec![size],
                                    ),
                                    &bindings,
                                );
                            };
                            let fill_type = self.infer_expr_type(fill)?;
                            let pattern = match fill_type {
                                HirType::Str => HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_from_string".to_string())),
                                    vec![
                                        fill.clone(),
                            encoding
                                .cloned()
                                .unwrap_or_else(|| HirExpr::Lit(HirLit::Str("utf8".to_string()))),
                                    ],
                                ),
                                HirType::Bytes => fill.clone(),
                                HirType::Array(ref element) if **element == HirType::F64 => {
                                    self.as_bytes_value(fill.clone())?
                                }
                                _ => {
                                    let number = self.coerce_primitive_to_number(fill.clone())?;
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_bytes_from_array".to_string())),
                                        vec![HirExpr::ArrayLit(vec![number])],
                                    )
                                }
                            };
                            return self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_alloc_fill".to_string())),
                                    vec![size, pattern],
                                ),
                                &bindings,
                            );
                        }
                        // `Buffer.from(value, encoding?)`. A string decodes
                        // per `encoding` (default `utf8`); a `number[]` /
                        // byte buffer copies through `__thaw_bytes_from_array`,
                        // which clamps each element to a byte (`300` -> `44`,
                        // `-1` -> `255`) and detaches the copy from the
                        // source array -- both `Buffer.from(array)` semantics.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Buffer.from")?;
                        let source = arguments
                            .first()
                            .ok_or("`Buffer.from` expects a value")?
                            .clone();
                        let source_type = self.infer_expr_type(&source)?;
                        // A string / number list held in a dynamic-storage slot (captured by a
                        // closure handed to a JS callback) reads as `Json`: pick by its shape.
                        if source_type == HirType::Json {
                            let encoding = match arguments.get(1) {
                                Some(argument) => argument.clone(),
                                None => HirExpr::Lit(HirLit::Str("utf8".to_string())),
                            };
                            let name = format!("__thaw_buffer_from_json_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), HirType::Json);
                            let numbers = self.coerce_to_declared(
                                &HirType::Array(Box::new(HirType::F64)),
                                HirExpr::Var(name.clone()),
                            )?;
                            let text = self.coerce_to_declared(&HirType::Str, HirExpr::Var(name.clone()))?;
                            let call = |intrinsic: &str, args: Vec<HirExpr>| {
                                HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), args)
                            };
                            let body = HirExpr::Conditional(
                                Box::new(call("__thaw_json_is_array", vec![HirExpr::Var(name.clone())])),
                                Box::new(call("__thaw_bytes_from_array", vec![numbers])),
                                Box::new(call("__thaw_bytes_from_string", vec![text, encoding])),
                                HirType::Bytes,
                            );
                            let mut bindings = bindings;
                            bindings.push((name, HirType::Json, source));
                            return self.wrap_call_argument_bindings(body, &bindings);
                        }
                        if matches!(&source_type, HirType::Array(elem) if **elem == HirType::F64) {
                            if self.infer_expr_type_inner(&source)? == HirType::Bytes {
                                // `Buffer.from(arrayBuffer, byteOffset?, length?)` shares the
                                // `ArrayBuffer`'s memory; `Buffer.from(buffer)` copies.
                                let mut operands = vec![source];
                                for position in 1..=2 {
                                    operands.push(match arguments.get(position) {
                                        Some(value) => self.coerce_primitive_to_number(value.clone())?,
                                        None => HirExpr::Lit(HirLit::F64(f64::NAN)),
                                    });
                                }
                                operands.push(HirExpr::Lit(HirLit::F64(1.0)));
                                return self.wrap_call_argument_bindings(
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_bytes_from_buffer_like".to_string())),
                                        operands,
                                    ),
                                    &bindings,
                                );
                            }
                            return self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_from_array".to_string())),
                                    vec![source],
                                ),
                                &bindings,
                            );
                        }
                        let encoding = match arguments.get(1) {
                            Some(argument) => argument.clone(),
                            None => HirExpr::Lit(HirLit::Str("utf8".to_string())),
                        };
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_bytes_from_string".to_string())),
                                vec![source, encoding],
                            ),
                            &bindings,
                        );
                    }
        if object.sym == *"Uint8Array" && matches!(property.sym.as_ref(), "fromHex" | "fromBase64")
                    {
                        // `Uint8Array.fromHex(text)` / `fromBase64(text)` --
                        // the same decode `Buffer.from(text, enc)` already
                        // performs.
                        let label = format!("Uint8Array.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [source] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let source = self.coerce_primitive_to_string(source.clone())?;
                        let encoding = HirExpr::Lit(HirLit::Str(
                            if property.sym == *"fromHex" {
                                "hex"
                            } else {
                                "base64"
                            }
                            .to_string(),
                        ));
                        let decoded = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_bytes_from_string".to_string())),
                            vec![source, encoding],
                        );
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_as_u8".to_string())), vec![decoded]),
                            &bindings,
                        );
                    }
                    if object.sym == *"Uint8Array" && property.sym == *"from" {
                        // `Uint8Array.from(array)` -- the same byte-clamped
                        // copy `Buffer.from(number[])` performs. A mapper
                        // argument isn't supported (no `thisArg` machinery
                        // here); `Buffer.from` has the same limit.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Uint8Array.from")?;
                        let [source] = arguments.as_slice() else {
                return Err("`Uint8Array.from` expects exactly one array source".into());
                        };
                        let source_type = self.infer_expr_type(source)?;
                        if !matches!(source_type, HirType::Array(_) | HirType::Bytes) {
                            return Err(format!(
                                "`Uint8Array.from` expects an array source, got {source_type:?}"
                            ));
                        }
                        let intrinsic = if self.infer_expr_type_inner(source)? == HirType::Bytes {
                            "__thaw_bytes_clone"
                        } else {
                            "__thaw_bytes_from_array"
                        };
                        let copy = HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![source.clone()],
                        );
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_as_u8".to_string())), vec![copy]),
                            &bindings,
                        );
                    }
                    if object.sym == *"JSON" && property.sym == *"parse" {
                        if call.args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("`JSON.parse` does not support spread arguments".into());
                        }
                        if call.args.is_empty() || call.args.len() > 2 {
                            return Err("`JSON.parse` expects one or two arguments".into());
                        }
                        let text = self.lower_expr(&call.args[0].expr)?;
                        let text = self.coerce_primitive_to_string(text)?;
                        if call.args.len() == 1 {
                let parsed =
                    HirExpr::Call(Box::new(HirExpr::Var("JSON.parse".to_string())), vec![text]);
                if has_dynamic_storage(call.span()) {
                    let retained = HirExpr::Call(
                        Box::new(HirExpr::Var("retainDynamicJson".into())),
                        vec![parsed],
                    );
                            return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                        vec![retained],
                            ));
                        }
                return Ok(parsed);
            }
                        // A reviver callback can only run in the JS realm,
                        // so delegate the whole parse there (`JSON.parse`
                        // itself, with the compiled closure registered as a
                        // native callback). The reviver is lowered with a
                        // `Json` return hint so a mixed-typed body
                        // (`k === "b" ? 99 : v`) unifies.
                        let reviver_ast = call.args[1].expr.as_ref();
                        let reviver = match reviver_ast {
                            Expr::Arrow(arrow) => {
                                let params = match arrow.params.len() {
                                    2 => vec![HirType::Str, HirType::Json],
                                    3 => vec![HirType::Str, HirType::Json, HirType::Json],
                                    count => vec![HirType::JsValue; count],
                                };
                    self.lower_contextual_arrow(arrow, &params, Some(&HirType::Json))?
                            }
                            other => self.lower_expr(other)?,
                        };
                        let reviver = self.coerce_to_declared(&HirType::JsValue, reviver)?;
                        let json_handle = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("JSON".to_string()))],
                        );
                        let text_json = self.coerce_to_declared(&HirType::Json, text)?;
                        let text_handle = HirExpr::Call(
                            Box::new(HirExpr::Var("retainDynamicJson".to_string())),
                            vec![text_json],
                        );
                        let arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![text_handle, reviver]),
                        )?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                            vec![
                                json_handle,
                                HirExpr::Lit(HirLit::Str("parse".to_string())),
                                arguments,
                            ],
                        ));
                    }
        if object.sym == *"JSON" && matches!(property.sym.as_ref(), "rawJSON" | "isRawJSON") {
                        let label = format!("JSON.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [argument] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let argument = if property.sym == *"rawJSON" {
                            let text = self.coerce_primitive_to_string(argument.clone())?;
                            self.coerce_to_declared(&HirType::Json, text)?
                        } else if self.infer_expr_type(argument)? == HirType::JsValue {
                            argument.clone()
                        } else {
                            self.coerce_to_declared(&HirType::Json, argument.clone())?
                        };
            let json_arguments =
                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![argument]))?;
                        let json = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("JSON".to_string()))],
                        );
                        let call = HirExpr::Call(
                            Box::new(HirExpr::Var(
                                if property.sym == *"rawJSON" {
                                    "callDynamicMethodHandle"
                                } else {
                                    "callDynamicMethod"
                                }
                                .to_string(),
                            )),
                            vec![
                                json,
                                HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                json_arguments,
                            ],
                        );
                        let result = if property.sym == *"isRawJSON" {
                            HirExpr::JsonAsBool(Box::new(call))
                        } else {
                            call
                        };
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"JSON" && property.sym == *"stringify" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "JSON.stringify")?;
                        if !(1..=3).contains(&arguments.len()) {
                            return Err("`JSON.stringify` expects one to three arguments".into());
                        }
                        let value = arguments[0].clone();
                        let value_type = self.infer_expr_type(&value)?;
                        let (value, value_type) = self
                            .apply_class_to_json(value.clone(), &value_type)?
                            .unwrap_or((value, value_type));
                        // An `extends Array` instance serializes as its items.
                        let (value, value_type) = match native_base_type(&value_type).cloned() {
                            Some(base @ HirType::Array(_)) => (self.project_native_base(value)?, base),
                            _ => (value, value_type),
                        };
                        if value_type == HirType::Str && arguments.len() == 1 {
                            // An Error value serializes as its own enumerable properties (`{}` for
                            // a fresh one); any other string stays a quoted JSON string.
                            let name = format!("__thaw_json_stringify_text_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), HirType::Str);
                            let var = HirExpr::Var(name.clone());
                            let plain = self.coerce_to_declared(&HirType::Json, var.clone())?;
                            let conditional = HirExpr::Conditional(
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_error_is_error".into())),
                                    vec![var.clone()],
                                )),
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_error_json_stringify".into())),
                                    vec![var],
                                )),
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("JSON.stringify".into())),
                                    vec![plain],
                                )),
                                HirType::Str,
                            );
                            bindings.push((name, HirType::Str, value));
                            return self.wrap_call_argument_bindings(conditional, &bindings);
                        }
                        if value_type == HirType::JsValue && arguments.len() == 1 {
                            // The source handle belongs to its caller. The exact mixed
                            // call consumes only the independent reference acquired
                            // here, and returns an exact graph result or original
                            // thrown value from a getter/toJSON callback.
                            let borrowed = HirExpr::Call(
                    Box::new(HirExpr::Var("@@thaw_retain_borrowed_dynamic_handle".into())),
                                vec![value],
                            );
                let empty =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(Vec::new()))?;
                            let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicValueMixedExact".into())),
                                vec![
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".into())),
                                        vec![HirExpr::Lit(HirLit::Str(
                                            "__thaw_json_stringify_direct".into(),
                                        ))],
                                    ),
                                    empty,
                                    HirExpr::ArrayLit(vec![borrowed]),
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if value_type == HirType::JsValue {
                            let replacer = arguments.get(1).cloned();
                let replacer_type = replacer
                    .as_ref()
                                .map(|value| self.infer_expr_type(value))
                                .transpose()?;
                            let space = arguments.get(2).cloned();
                let space_type = space
                    .as_ref()
                                .map(|value| self.infer_expr_type(value))
                                .transpose()?;
                            let supported_replacer = match replacer_type.as_ref() {
                    Some(
                        HirType::Function(..)
                        | HirType::CallableFunction(..)
                        | HirType::FunctionWithThis(..)
                        | HirType::Null
                        | HirType::Undefined
                        | HirType::Bool
                        | HirType::F64
                        | HirType::Str
                        | HirType::JsValue,
                    ) => true,
                    Some(HirType::Array(element)) => element.as_ref() == &HirType::Str,
                                _ => false,
                            };
                let supported_space = space_type.as_ref().is_none_or(|ty| {
                    matches!(
                        ty,
                        HirType::F64
                            | HirType::Str
                            | HirType::Bool
                            | HirType::Null
                            | HirType::Undefined
                    )
                });
                            if supported_replacer && supported_space {
                                let borrowed = HirExpr::Call(
                        Box::new(HirExpr::Var("@@thaw_retain_borrowed_dynamic_handle".into())),
                                    vec![value],
                                );
                    let space_json = self.coerce_to_declared(
                        &HirType::Json,
                        space.unwrap_or(HirExpr::Lit(HirLit::Undefined)),
                    )?;
                    let (helper, json_arguments, direct_arguments) = if matches!(
                        replacer_type.as_ref(),
                        Some(
                            HirType::Function(..)
                                            | HirType::CallableFunction(..)
                                | HirType::FunctionWithThis(..)
                        )
                    ) {
                                        (
                                            "__thaw_json_stringify_live_replacer",
                            self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![space_json]),
                            )?,
                            HirExpr::ArrayLit(vec![
                                borrowed,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("registerNativeCallbackGraph".into())),
                                                vec![replacer.unwrap()],
                                ),
                            ]),
                                        )
                                    } else if replacer_type == Some(HirType::JsValue) {
                                        (
                                            "__thaw_json_stringify_live_dynamic_replacer",
                            self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![space_json]),
                            )?,
                            HirExpr::ArrayLit(vec![
                                borrowed,
                                HirExpr::Call(
                                                Box::new(HirExpr::Var(
                                                    "@@thaw_retain_borrowed_dynamic_handle".into(),
                                                )),
                                                vec![replacer.unwrap()],
                                ),
                            ]),
                                        )
                                    } else {
                        let replacer_json =
                            self.coerce_to_declared(&HirType::Json, replacer.unwrap())?;
                                        (
                                            "__thaw_json_stringify_native_accessors",
                            self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![replacer_json, space_json]),
                            )?,
                                            HirExpr::ArrayLit(vec![borrowed]),
                                        )
                                    };
                                let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("callDynamicValueMixedExact".into())),
                                    vec![
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var("getDynamicValue".into())),
                                            vec![HirExpr::Lit(HirLit::Str(helper.into()))],
                                        ),
                                        json_arguments,
                                        direct_arguments,
                                    ],
                                )));
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                        }
            if contains_js_value(&value_type) || value_type == HirType::Json {
                            let mut json_arguments = Vec::with_capacity(arguments.len());
                            for (index, argument) in arguments.iter().enumerate() {
                                let ty = self.infer_expr_type(argument)?;
                                let argument = if ty == HirType::JsValue
                                    || (index == 1
                            && matches!(
                                ty,
                                HirType::Function(_, _)
                                            | HirType::CallableFunction(..)
                                    | HirType::FunctionWithThis(..)
                            )) {
                                    self.coerce_to_declared(&HirType::JsValue, argument.clone())?
                                } else {
                                    self.coerce_to_declared(&HirType::Json, argument.clone())?
                                };
                                json_arguments.push(argument);
                            }
                let json_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_arguments))?;
                            // A lone value goes through the QuickJS `__thaw_json_stringify_direct`
                            // helper, which drops symbol-keyed wire properties. Passing it as the
                            // same Json argument keeps the conversion identical to the old
                            // `JSON.stringify` method call; any replacer/space keeps that call.
                            let result = if arguments.len() == 1 {
                                HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("callDynamicValueMixedExact".to_string())),
                                    vec![
                                        HirExpr::Call(
                                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                            vec![HirExpr::Lit(HirLit::Str(
                                                "__thaw_json_stringify_direct".to_string(),
                                            ))],
                                        ),
                                        json_arguments,
                                        HirExpr::ArrayLit(Vec::new()),
                                    ],
                                )))
                            } else {
                                let json = HirExpr::Call(
                                    Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                    vec![HirExpr::Lit(HirLit::Str("JSON".to_string()))],
                                );
                                HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                    vec![
                                        json,
                                        HirExpr::Lit(HirLit::Str("stringify".to_string())),
                                        json_arguments,
                                    ],
                                )))
                            };
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let mut replacer_array = None;
                        let mut replacer_function = None;
                        if let Some(replacer) = arguments.get(1) {
                            let replacer_type = self.infer_expr_type(replacer)?;
                            match &replacer_type {
                                HirType::Array(element) if element.as_ref() == &HirType::Str => {
                                    replacer_array = Some(replacer.clone());
                                }
                                HirType::Function(_, _)
                                | HirType::CallableFunction(_, _, _, _)
                                | HirType::FunctionWithThis(_, _) => {
                                    replacer_function = Some(replacer.clone());
                                }
                                HirType::Null | HirType::Undefined => {
                                    if !matches!(replacer, HirExpr::Lit(_)) {
                                        let name = format!(
                                            "__thaw_json_stringify_replacer_{}",
                                            self.next_binding
                                        );
                                        self.next_binding += 1;
                                        self.scope.insert(name.clone(), replacer_type.clone());
                                        bindings.push((name, replacer_type, replacer.clone()));
                                    }
                                }
                                _ => return Err(
                                    "`JSON.stringify` replacer must be a function, null, undefined, or string[]"
                                        .into(),
                                ),
                            }
                        }
                        let native_accessor_fields = match &value_type {
                HirType::Object(fields) if Self::fixed_object_contains_accessor(fields) => {
                                Some(fields.clone())
                            }
                            _ => None,
                        };
                        if native_accessor_fields.is_some()
                && !Self::fixed_object_supports_live_projection(&value_type)
            {
                            return Err("`JSON.stringify` needs a live native accessor projector for this object layout".into());
                        }
                        // Preserve the original owner for function fields:
                        // snapshotting it would resurrect deleted keys and
                        // change the receiver observed by toJSON.
                        let live_function_fields = match &value_type {
                            HirType::Object(fields)
                                if native_accessor_fields.is_none()
                                    && fields.iter().any(|(name, ty)| {
                                        !is_hidden_accessor_field(name)
                                            && Self::type_holds_callable_field(ty)
                                    })
                                    && Self::fixed_object_supports_live_projection(&value_type) =>
                            {
                                Some(fields.clone())
                            }
                            _ => None,
                        };
            let value = if native_accessor_fields.is_none() && live_function_fields.is_none() {
                if let (HirType::Object(fields), Some(HirExpr::ArrayLit(keys))) =
                    (&value_type, &replacer_array)
                            {
                                let mut property_list = Vec::new();
                                for key in keys {
                                    let HirExpr::Lit(HirLit::Str(key)) = key else {
                                        continue;
                                    };
                                    if !property_list.contains(key) {
                                        property_list.push(key.clone());
                                    }
                                }
                    self.materialize_fixed_object_for_json_replacer(value, fields, &property_list)?
                            } else {
                                value
                            }
                        } else {
                            value
                        };
                        let value_type = self.infer_expr_type(&value)?;
                        // A fixed object with function-valued fields
                        // (shorthand methods / arrow properties): Node omits
                        // functions when serializing, and calls `toJSON()`
                        // first when one is present. Those fields encode as
                        // opaque `JsValue`s the native JSON bridge rejects,
                        // so drop them (or serialize `toJSON()`'s result).
                        let (value, value_type) = match &value_type {
                            HirType::Object(fields)
                                if native_accessor_fields.is_none()
                                    && live_function_fields.is_none()
                                    && fields.iter().any(|(name, ty)| {
                                        !is_hidden_accessor_field(name)
                                            && matches!(
                                                ty,
                                                HirType::Function(_, _)
                                                    | HirType::CallableFunction(..)
                                                    | HirType::JsValue
                                            )
                                    }) =>
                            {
                                if let Some((_, HirType::Function(params, _))) =
                                    fields.iter().find(|(name, _)| name == "toJSON")
                                {
                                    if params.is_empty() {
                                        let call = HirExpr::Call(
                                            Box::new(HirExpr::PropAccess(
                                                Box::new(value),
                                                value_type.clone(),
                                                "toJSON".to_string(),
                                            )),
                                            Vec::new(),
                                        );
                                        let ty = self.infer_expr_type(&call)?;
                                        (call, ty)
                                    } else {
                                        (value, value_type.clone())
                                    }
                                } else {
                                    // Shallow copy of the non-function
                                    // fields into a fresh object (a nested
                                    // object with its own methods is left
                                    // as-is, so it still fails loudly
                                    // rather than silently losing fields).
                        let source = format!("__thaw_json_object_{}", self.next_binding);
                                    self.next_binding += 1;
                                    self.scope.insert(source.clone(), value_type.clone());
                                    let mut snapshot = Vec::new();
                                    for (name, ty) in fields {
                                        if is_hidden_accessor_field(name)
                                            || matches!(
                                                ty,
                                                HirType::Function(_, _)
                                                    | HirType::CallableFunction(..)
                                                    | HirType::JsValue
                                            )
                                        {
                                            continue;
                                        }
                                        let read = self.lower_fixed_object_property_read(
                                            HirExpr::Var(source.clone()),
                                            fields,
                                            name,
                                        )?;
                                        snapshot.push((name.clone(), read));
                                    }
                                    let materialized = self.wrap_call_argument_bindings(
                                        HirExpr::ObjectLit(snapshot),
                                        &[(source, value_type.clone(), value)],
                                    )?;
                                    let ty = self.infer_expr_type(&materialized)?;
                                    (materialized, ty)
                                }
                            }
                            _ => (value, value_type.clone()),
                        };
            let value = if native_accessor_fields.is_some() || live_function_fields.is_some() {
                            value
                        } else if value_type == HirType::JsValue {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("readDynamicValue".into())),
                                vec![value],
                            )
                        } else if matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            value
                        } else if value_type == crate::caught_exception_carrier_type() {
                            self.caught_carrier_as_json(value)?
                        } else if json_convertible_native_type(&value_type) {
                            let wrapped = self.wrap_native_value_as_json(value, value_type.clone())?;
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_stringify_scope".into())),
                                vec![wrapped],
                            )
                        } else if matches!(
                            value_type,
                            HirType::Function(..) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..)
                        ) {
                            // `JSON.stringify(fn)` is `undefined`: evaluate the operand, then serialize `undefined`.
                            let undefined = HirExpr::Block(vec![
                                HirStmt::Expr(value),
                                HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                            ]);
                            let wrapped = self.wrap_native_value_as_json(undefined, HirType::Undefined)?;
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_stringify_scope".into())),
                                vec![wrapped],
                            )
                        } else {
                            return Err(format!(
                                "`JSON.stringify` requires a JSON or dictionary value, got {value_type:?}"
                            ));
                        };
                        let space = match arguments.get(2) {
                            None => None,
                            Some(space) => match self.infer_expr_type(space)? {
                                HirType::Null | HirType::Undefined => {
                                    if !matches!(space, HirExpr::Lit(_)) {
                                        let ty = self.infer_expr_type(space)?;
                            let name = format!("__thaw_json_stringify_space_{}", self.next_binding);
                                        self.next_binding += 1;
                                        self.scope.insert(name.clone(), ty.clone());
                                        bindings.push((name, ty, space.clone()));
                                    }
                                    None
                                }
                                HirType::F64 => Some((space.clone(), false)),
                                HirType::Str => Some((space.clone(), true)),
                                other => {
                                    return Err(format!(
                                        "`JSON.stringify` space must be number, string, null or undefined, got {other:?}"
                        ));
                                }
                            },
                        };
                        if let Some(fields) = live_function_fields.or(native_accessor_fields) {
                            let live = self.lower_fixed_object_as_dynamic_accessor_object(
                                value, &fields, false, false, false,
                            )?;
                            let space = space
                                .map(|(value, _)| value)
                                .unwrap_or(HirExpr::Lit(HirLit::Null));
                            let space_json = self.coerce_to_declared(&HirType::Json, space)?;
                            let (helper, json_arguments, direct_arguments) =
                                if let Some(replacer) = replacer_function {
                                    (
                                        "__thaw_json_stringify_live_replacer",
                                        self.coerce_to_declared(
                                            &HirType::Json,
                                            HirExpr::ArrayLit(vec![space_json]),
                                        )?,
                                        HirExpr::ArrayLit(vec![
                                            live,
                                            HirExpr::Call(
                                    Box::new(HirExpr::Var("registerNativeCallbackGraph".into())),
                                                vec![replacer],
                                            ),
                                        ]),
                                    )
                                } else {
                        let replacer = replacer_array.unwrap_or(HirExpr::Lit(HirLit::Null));
                        let replacer_json = self.coerce_to_declared(&HirType::Json, replacer)?;
                                    (
                                        "__thaw_json_stringify_native_accessors",
                                        self.coerce_to_declared(
                                            &HirType::Json,
                                            HirExpr::ArrayLit(vec![replacer_json, space_json]),
                                        )?,
                                        HirExpr::ArrayLit(vec![live]),
                                    )
                                };
                            let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicValueMixedExact".into())),
                                vec![
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".into())),
                                        vec![HirExpr::Lit(HirLit::Str(helper.into()))],
                                    ),
                                    json_arguments,
                                    direct_arguments,
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let Some(replacer) = replacer_function {
                            let space = space
                                .map(|(value, _)| value)
                                .unwrap_or(HirExpr::Lit(HirLit::Null));
                            // A Json ArrayLit holds a shallow share of each
                            // element. The native graph (including back
                            // edges) stays live until the replacer runs.
                            let space = self.coerce_to_declared(&HirType::Json, space)?;
                let json_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![space, value]))?;
                            let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicValueMixedExact".into())),
                                vec![
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".into())),
                                        vec![HirExpr::Lit(HirLit::Str(
                                            "__thaw_json_stringify_live_replacer".into(),
                                        ))],
                                    ),
                                    json_arguments,
                                    HirExpr::ArrayLit(vec![HirExpr::Call(
                                        Box::new(HirExpr::Var("registerNativeCallbackGraph".into())),
                                        vec![replacer],
                                    )]),
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let result = match (replacer_array, space) {
                (None, None) => {
                    HirExpr::Call(Box::new(HirExpr::Var("JSON.stringify".into())), vec![value])
                }
                            (Some(replacer), None) => HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_stringify_keys".into())),
                                vec![value, replacer],
                            ),
                            (None, Some((space, string_space))) => {
                                let runtime = if string_space {
                                    "__thaw_json_stringify_string_space"
                                } else {
                                    "__thaw_json_stringify_number_space"
                                };
                    HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), vec![value, space])
                            }
                            (Some(replacer), Some((space, string_space))) => {
                                let runtime = if string_space {
                                    "__thaw_json_stringify_keys_string_space"
                                } else {
                                    "__thaw_json_stringify_keys_number_space"
                                };
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(runtime.into())),
                                    vec![value, replacer, space],
                                )
                            }
                        };
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"RegExp" && property.sym == *"escape" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "RegExp.escape")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`RegExp.escape` expects exactly one argument".into());
                        };
                        let value = self.coerce_primitive_to_string(value.clone())?;
                        let value = self.coerce_to_declared(&HirType::Json, value)?;
            let args = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                        let regexp = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("RegExp".into()))],
                        );
                        let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethod".into())),
                            vec![regexp, HirExpr::Lit(HirLit::Str("escape".into())), args],
                        )));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Iterator" && property.sym == *"from" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Iterator.from")?;
                        let [source] = arguments.as_slice() else {
                            return Err("`Iterator.from` expects exactly one argument".into());
                        };
                        let source_type = self.infer_expr_type(source)?;
                        if matches!(&source_type, HirType::Function(params, result)
                            if matches!(params.as_slice(), [HirType::I64, HirType::Json, _, HirType::Array(_), HirType::Array(_), HirType::Array(_)])
                                && matches!(result.as_ref(), HirType::Promise(generated) if matches!(generated.as_ref(), HirType::Array(_))))
                        {
                            return Err("`Iterator.from` does not accept an async generator".into());
                        }
                        let source = if matches!(&source_type, HirType::Function(params, result)
                            if matches!(params.as_slice(), [HirType::I64, HirType::Json, _, HirType::Array(_), HirType::Array(_), HirType::Array(_)])
                                && matches!(result.as_ref(), HirType::Array(_)))
                        {
                            self.generator_protocol_object(source, &source_type, call.span, call.ctxt, &mut bindings, false)?
                        } else {
                            source.clone()
                        };
                        let source = self.coerce_to_declared(&HirType::Json, source)?;
            let args = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![source]))?;
                        let iterator_from = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("__thaw_iterator_from".into()))],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
                            vec![iterator_from, args],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Array" && property.sym == *"of" {
                        let explicit_type = if let Some(type_args) = &call.type_args {
                            let [element] = type_args.params.as_slice() else {
                                return Err("`Array.of` expects zero or one type argument".into());
                            };
                            Some(lower_ts_type(
                                element,
                                self.interfaces,
                                self.generic_interfaces,
                            )?)
                        } else {
                            None
                        };
                        let mut element_type = explicit_type;
                        let mut parts = Vec::new();
                        let mut pending = Vec::new();
                        for argument in &call.args {
                            let mut value = self.lower_expr(&argument.expr)?;
                            if argument.spread.is_some() {
                                value = self.project_native_base(value)?;
                                value = self.bytes_to_f64_array(value)?;
                                // Same snapshot conversion array-literal spreads
                                // (`[...str]`/`[...map]`/`[...set]`) already use.
                                let mut spread_source_type = self.infer_expr_type(&value)?;
                                if let HirType::Union(members) = &spread_source_type {
                        if members
                            .iter()
                            .all(|member| matches!(member, HirType::Array(_)))
                        {
                                        (value, spread_source_type) =
                                            self.lower_union_array_sequence(value, members)?;
                                    }
                                }
                                if spread_source_type == HirType::Str {
                                    value = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_string_to_array".into())),
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
                                } else if let HirType::Set(set_element) = &spread_source_type {
                                    let array_type = HirType::Array(set_element.clone());
                                    value = HirExpr::TypedClosure(
                                        array_type,
                                        Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                                            vec![value],
                                        )),
                                    );
                                }
                                let ty = self.infer_expr_type(&value)?;
                                let HirType::Array(element) = ty else {
                                    return Err(
                            "`Array.of` spread requires a homogeneous array, string, Map, or Set"
                                .into(),
                                    );
                                };
                                if let Some(expected) = &element_type {
                                    if expected != element.as_ref() {
                                        if expected == &HirType::Optional(element.clone()) {
                                            element_type = Some(element.as_ref().clone());
                                        } else {
                                            return Err(format!(
                                                "`Array.of` spread element has type {element:?}, expected {expected:?}"
                                            ));
                                        }
                                    }
                                } else {
                                    element_type = Some(element.as_ref().clone());
                                }
                                if !pending.is_empty() {
                        let values = std::mem::take(&mut pending)
                            .into_iter()
                                        .map(|item| self.coerce_array_insert_value(item, element.as_ref()))
                                        .collect::<Result<Vec<_>, String>>()?;
                        parts.push(
                            self.lower_native_array_literal(values, element.as_ref().clone())?,
                        );
                                }
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
                                parts.push(value);
                            } else {
                                value = self.lower_array_index_operand(value)?;
                                let ty = self.infer_expr_type(&value)?;
                                if let Some(expected) = &element_type {
                                    if expected != &ty && ty != HirType::Optional(Box::new(expected.clone())) {
                                        return Err(format!(
                                            "`Array.of` element has type {ty:?}, expected {expected:?}"
                                        ));
                                    }
                                } else {
                                    element_type = Some(match ty {
                                        HirType::Optional(payload) => *payload,
                                        other => other,
                                    });
                                }
                                pending.push(value);
                            }
                        }
                        if !pending.is_empty() {
                let element = element_type
                    .as_ref()
                    .expect("nonempty Array.of has an element type");
                let values = pending
                    .into_iter()
                                .map(|item| self.coerce_array_insert_value(item, element))
                                .collect::<Result<Vec<_>, String>>()?;
                            parts.push(self.lower_native_array_literal(values, element.clone())?);
                        }
            let element_type = element_type
                .ok_or("empty `Array.of()` requires an explicit element type argument")?;
                        return Ok(match parts.len() {
                0 => HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element_type),
                            1 => parts.pop().unwrap(),
                            _ => HirExpr::ArrayConcat(parts, element_type),
                        });
                    }
                    if object.sym == *"Array" && property.sym == *"from" {
                        let has_spread = call.args.iter().any(|argument| argument.spread.is_some());
                        let (spread_arguments, spread_bindings) = if has_spread {
                            self.lower_native_spread_values(&call.args, "Array.from")?
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        let argument_count = if has_spread {
                            spread_arguments.len()
                        } else {
                            call.args.len()
                        };
                        if !(1..=3).contains(&argument_count) {
                            return Err(
                                "native `Array.from` expects a source, optional mapper and optional thisArg"
                                    .into(),
                            );
                        }
                        let sparse_source = self.array_from_source_may_be_sparse(&call.args);
                        let explicit_types = call
                            .type_args
                            .as_ref()
                            .map(|type_args| {
                                type_args
                                    .params
                                    .iter()
                        .map(|ty| lower_ts_type(ty, self.interfaces, self.generic_interfaces))
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        if explicit_types.len() > 2 {
                            return Err("`Array.from` expects at most two type arguments".into());
                        }
                        let mut source = if has_spread {
                            spread_arguments[0].clone()
                        } else {
                            self.lower_expr(&call.args[0].expr)?
                        };
                        source = self.bytes_to_f64_array(source)?;
                        let mut source_type = self.infer_expr_type(&source)?;
                        if let HirType::Union(members) = &source_type {
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_)))
                {
                    (source, source_type) = self.lower_union_array_sequence(source, members)?;
                            }
                        }
                        if !has_spread && class_name_from_type(&source_type).is_some() {
                            // A user iterable class collects like `[...source]`.
                            let spread = Expr::Array(swc_ecma_ast::ArrayLit {
                                span: swc_common::DUMMY_SP,
                                elems: vec![Some(swc_ecma_ast::ExprOrSpread {
                                    spread: Some(swc_common::DUMMY_SP),
                                    expr: call.args[0].expr.clone(),
                                })],
                            });
                            source = self.lower_expr(&spread)?;
                            source_type = self.infer_expr_type(&source)?;
                        }
                        if let HirType::Object(fields) = &source_type {
                            if let [(field_name, HirType::F64)] = fields.as_slice() {
                                if field_name == "length" {
                                    let length = HirExpr::PropAccess(
                                        Box::new(source),
                                        source_type.clone(),
                                        "length".to_string(),
                                    );
                                    let callback = if argument_count == 1 {
                                        None
                                    } else if has_spread {
                                        let callback = spread_arguments[1].clone();
                                        if self.infer_expr_type(&callback)? == HirType::Undefined {
                                            Some(callback)
                                        } else {
                                let params =
                                    Self::array_mapper_params(&self.infer_expr_type(&callback)?)
                                            .ok_or("Array.from mapper is not a function value")?;
                                        if params.len() > 2 {
                                            return Err(format!(
                                                "Array.from mapper accepts at most two parameters, got {}",
                                                params.len()
                                            ));
                                        }
                                        let available = [HirType::Undefined, HirType::F64];
                                        self.validate_promise_callback_value(
                                            &callback,
                                            &available[..params.len()],
                                            None,
                                        )?;
                                        Some(callback)
                                        }
                                    } else {
                                        let callback = match &*call.args[1].expr {
                                            Expr::Arrow(_) | Expr::Fn(_) => self.lower_array_from_callback(
                                    &call.args[1].expr,
                                    &HirType::Undefined,
                                            )?,
                                Expr::Ident(ident)
                                    if ident.sym == *"undefined"
                                                && !self.scope.contains_key(
                                                    &self.resolve_binding(ident.sym.as_ref()),
                                        ) =>
                                {
                                    self.lower_expr(&call.args[1].expr)?
                                }
                                Expr::Ident(ident)
                                    if self
                                        .scope
                                        .get(&self.resolve_binding(ident.sym.as_ref()))
                                        != Some(&HirType::Undefined) =>
                                {
                                    self.lower_array_from_callback(
                                        &call.args[1].expr,
                                        &HirType::Undefined,
                                    )?
                                }
                                            _ => self.lower_expr(&call.args[1].expr)?,
                                        };
                                        if self.infer_expr_type(&callback)? != HirType::Undefined {
                                let params =
                                    Self::array_mapper_params(&self.infer_expr_type(&callback)?)
                                                .ok_or("Array.from mapper is not a function value")?;
                                            if params.len() > 2 {
                                                return Err(format!(
                                                    "Array.from mapper accepts at most two parameters, got {}",
                                                    params.len(),
                                                ));
                                            }
                                            let available = [HirType::Undefined, HirType::F64];
                                            self.validate_promise_callback_value(
                                    &callback,
                                    &available[..params.len()],
                                    None,
                                            )?;
                                        }
                                        Some(callback)
                                    };
                                    let this_arg = if has_spread {
                                        spread_arguments.get(2).cloned()
                                    } else {
                                        call.args
                                            .get(2)
                                            .map(|argument| self.lower_expr(&argument.expr))
                                            .transpose()?
                                    };
                        let result = self.lower_array_from_length(length, callback, this_arg)?;
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                                }
                            }
                        }
                        let (source, source_type, element_type) = match source_type {
                            HirType::Array(element) => {
                                let element_type = element.as_ref().clone();
                                (source, HirType::Array(element), element_type)
                            }
                            // A dynamic `any` value (e.g. `arguments`) used as an array source.
                            HirType::Json => {
                                let array_type = HirType::Array(Box::new(HirType::Json));
                                let source = self.coerce_to_declared(&array_type, source)?;
                                (source, array_type, HirType::Json)
                            }
                            HirType::Str => (
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_to_array".into())),
                                    vec![source],
                                ),
                                HirType::Array(Box::new(HirType::Str)),
                                HirType::Str,
                            ),
                            // Same snapshot conversion array-literal spreads
                            // (`[...map]`/`[...set]`) already use, matching
                            // each container's default iterator shape.
                            HirType::Map(key_type, value_type) => {
                                let pair_type = HirType::Tuple(vec![
                                    key_type.as_ref().clone(),
                                    value_type.as_ref().clone(),
                                ]);
                                let array_type = HirType::Array(Box::new(pair_type.clone()));
                                let source = HirExpr::TypedClosure(
                                    array_type.clone(),
                                    Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_snapshot_entries".into())),
                                        vec![source],
                                    )),
                                );
                                (source, array_type, pair_type)
                            }
                            HirType::Set(element) => {
                                let element_type = element.as_ref().clone();
                                let array_type = HirType::Array(element);
                                let source = HirExpr::TypedClosure(
                                    array_type.clone(),
                                    Box::new(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                                        vec![source],
                                    )),
                                );
                                (source, array_type, element_type)
                            }
                            // A `JsValue` following the JS iterator
                            // protocol -- real trigger: `Array.from(cache.
                            // keys())`, `lru-cache`'s `LRUCache.keys()`
                            // returning a real `Generator<K>` (a real npm
                            // class's method return type documented this
                            // way falls back to the generic dynamic escape
                            // hatch, `HirType::JsValue`, since thaw-
                            // bridge's own `.d.ts` classifier -- unlike
                            // thaw-hir's `lower_ts_type` -- has no
                            // `Generator`/`IterableIterator` case at all).
                            // Unlike `for...of` (`Stmt::ForOf`, `lower/
                            // statements/lowering.rs`), which lazily steps
                            // thaw's own resumable generator-producer ABI
                            // via `dynamic_iterator_adapter`, `Array.from`
                            // wants the whole thing eagerly drained into
                            // one array up front -- simpler to build
                            // directly as a self-contained native `while`
                            // loop than to route through that heavier,
                            // resumable machinery.
                            HirType::JsValue => {
                    let iterator_name = format!("__thaw_array_from_iterator_{}", self.next_binding);
                                self.next_binding += 1;
                    let array_name = format!("__thaw_array_from_result_{}", self.next_binding);
                                self.next_binding += 1;
                    let result_name = format!("__thaw_array_from_next_{}", self.next_binding);
                                self.next_binding += 1;
                                let element_type = HirType::Json;
                                let array_type = HirType::Array(Box::new(element_type.clone()));
                                let empty_args =
                                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(Vec::new()))?;
                                let call_next = HirExpr::Call(
                                    Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                    vec![
                                        HirExpr::Var(iterator_name.clone()),
                                        HirExpr::Lit(HirLit::Str("next".to_string())),
                                        empty_args,
                                    ],
                                );
                                let loop_body = vec![
                                    HirStmt::Let(result_name.clone(), HirType::Json, call_next),
                                    HirStmt::If(
                                        HirExpr::JsonAsBool(Box::new(HirExpr::JsonGet(
                                            Box::new(HirExpr::Var(result_name.clone())),
                                            "done".into(),
                                        ))),
                                        vec![HirStmt::Break],
                                        Vec::new(),
                                    ),
                                    HirStmt::Expr(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_array_push".to_string())),
                                        vec![
                                            HirExpr::Var(array_name.clone()),
                                            HirExpr::JsonGet(
                                                Box::new(HirExpr::Var(result_name)),
                                                "value".into(),
                                            ),
                                        ],
                                    )),
                                ];
                                let block = HirExpr::Block(vec![
                                    HirStmt::Let(
                                        array_name.clone(),
                                        array_type.clone(),
                                        HirExpr::ArrayLit(Vec::new()),
                                    ),
                        HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), loop_body),
                                    HirStmt::Return(Some(HirExpr::Var(array_name))),
                                ]);
                                let iterator = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_to_iterator_exact".into())),
                                    vec![source],
                                );
                                let source = HirExpr::Call(
                                    Box::new(HirExpr::Lambda(
                                        Vec::new(),
                                        vec![HirParam {
                                            name: iterator_name,
                                            ty: HirType::JsValue,
                                        }],
                                        array_type.clone(),
                                        Box::new(block),
                                    )),
                                    vec![iterator],
                                );
                                (source, array_type, element_type)
                            }
                            // A real, statically-typed `Generator<T>`
                            // producer (as opposed to the `JsValue` case
                            // just above, a *dynamic* npm-class iterator).
                            // Reuses `collect_generator_for_array_spread`
                            // (`expressions/lowering.rs`, round20/21's own
                            // fixed `[...generator()]` collector) directly
                            // -- its own drain-to-completion loop already
                            // produces exactly the array `Array.from`
                            // wants, element type and all.
                            HirType::Function(..) if is_generator_producer_type(&source_type) => {
                    match self.collect_generator_for_array_spread(source.clone(), &source_type)? {
                                    Some((collected, element_type)) => {
                            let array_type = HirType::Array(Box::new(element_type.clone()));
                                        (collected, array_type, element_type)
                                    }
                                    None => {
                                        return Err(format!(
                                            "native `Array.from` requires a homogeneous array, string, Map, Set, or Generator, got {source_type:?}"
                            ));
                                    }
                                }
                            }
                            other => {
                                return Err(format!(
                                    "native `Array.from` requires a homogeneous array, string, Map, or Set, got {other:?}"
                    ));
                            }
                        };
                        if let Some(expected) = explicit_types.first() {
                            if expected != &element_type {
                                return Err(format!(
                                    "`Array.from` source element has type {element_type:?}, expected {expected:?}"
                                ));
                            }
                        }
                        let source = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_densify".into())),
                            vec![HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                vec![
                                    source,
                                    HirExpr::Lit(HirLit::F64(0.0)),
                                    HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                                ],
                            )],
                        );
                        if argument_count == 1 {
                            return self.wrap_call_argument_bindings(source, &spread_bindings);
                        }
                        let callback_element_type = if sparse_source {
                            Self::array_read_type(&element_type)
                        } else {
                            element_type.clone()
                        };
                        let callback = if has_spread {
                            let callback = spread_arguments[1].clone();
                            let params = Self::array_mapper_params(&self.infer_expr_type(&callback)?)
                                .ok_or("Array.from mapper is not a function value")?;
                            if params.len() > 2 {
                                return Err(format!(
                                    "Array.from mapper accepts at most two parameters, got {}",
                                    params.len()
                                ));
                            }
                            let available = [callback_element_type.clone(), HirType::F64];
                self.validate_promise_callback_value(&callback, &available[..params.len()], None)?;
                            callback
                        } else {
                            self.lower_array_from_callback(&call.args[1].expr, &callback_element_type)?
                        };
                        if let Some(expected) = explicit_types.get(1).or(explicit_types.first()) {
                let HirType::Function(_, output) = self.infer_expr_type(&callback)? else {
                                unreachable!("Array.from mapper is a function")
                            };
                            if output.as_ref() != expected {
                                return Err(format!(
                                    "`Array.from` mapper returns {output:?}, expected {expected:?}"
                                ));
                            }
                        }
                        let this_arg = if has_spread {
                            spread_arguments.get(2).cloned()
                        } else {
                            call.args
                                .get(2)
                                .map(|argument| self.lower_expr(&argument.expr))
                                .transpose()?
                        };
                        let result = self.lower_array_map(
                            source,
                            source_type,
                            element_type.clone(),
                            callback_element_type,
                            callback,
                            this_arg,
                        )?;
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                    }
        if matches!(object.sym.as_ref(), "Map" | "Object") && property.sym == *"groupBy" {
                        let label = format!("{}.groupBy", object.sym);
                        let object_result = object.sym == *"Object";
                        let has_spread = call.args.iter().any(|argument| argument.spread.is_some());
                        let (arguments, spread_bindings) = if has_spread {
                            self.lower_native_spread_values(&call.args, &label)?
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        let argument_count = if has_spread {
                            arguments.len()
                        } else {
                            call.args.len()
                        };
                        if argument_count != 2 {
                            return Err(format!("`{label}` expects exactly two arguments"));
                        }
                        let mut items = if has_spread {
                            arguments[0].clone()
                        } else {
                            self.lower_expr(&call.args[0].expr)?
                        };
                        let mut items_type = self.infer_expr_type(&items)?;
                        if let HirType::Union(members) = &items_type {
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_)))
                {
                    (items, items_type) = self.lower_union_array_sequence(items, members)?;
                            }
                        }
                        let HirType::Array(item_type) = &items_type else {
                            return Err(format!(
                                "`{label}` requires a homogeneous array, got {items_type:?}"
                            ));
                        };
                        let item_type = item_type.as_ref().clone();
                        let key_fn = if has_spread {
                            let key_fn = arguments[1].clone();
                            let params = match self.infer_expr_type(&key_fn)? {
                    HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                        params
                                }
                    _ => return Err(format!("{label} key function is not a function value")),
                            };
                            if params.len() > 2 {
                                return Err(format!(
                                    "{label} key function accepts at most two parameters, got {}",
                                    params.len()
                                ));
                            }
                            let available = [item_type.clone(), HirType::F64];
                self.validate_promise_callback_value(&key_fn, &available[..params.len()], None)?;
                            key_fn
                        } else {
                            // Reuses `Array.from`'s own contextual-typing helper: its
                            // "zero-to-two-argument (element, index)" shape is exactly
                            // `Map.groupBy`'s own `(item, index) => key` callback.
                            self.lower_array_from_callback(&call.args[1].expr, &item_type)?
                        };
            let result = self.lower_group_by(items, item_type, key_fn, object_result, true)?;
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                    }
                    if object.sym == *"Array" && property.sym == *"fromAsync" {
                        // Delegate to the JS realm's `Array.fromAsync`, which
                        // handles async iterables, per-element `await`,
                        // iterator closing, and `thisArg` exactly; the
                        // previous native approximation only covered an
                        // array-like input with a synchronous mapper.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Array.fromAsync")?;
                        if arguments.is_empty() || arguments.len() > 3 {
                            return Err("`Array.fromAsync` expects one to three arguments".into());
                        }
                        // A real, statically typed `async function*`
                        // producer (thaw's own internal generator ABI, see
                        // `is_generator_producer_type`) has no JS-visible
                        // shape the QuickJS-delegating path below could ever
                        // hand off to -- its first parameter is an internal
                        // `i64` resume point, not a real argument, so
                        // crossing it as a dynamic-call argument fails deep
                        // inside JSON/callback decoding instead of at the
                        // call site. Reuses `collect_async_generator_for_
                        // array_spread` (`expressions/lowering.rs`, the
                        // `async function*` sibling of round20/21/23's own
                        // `[...generator()]`/`Array.from(generator())`
                        // collector) directly when there's no mapper/
                        // `thisArg` argument to also apply -- falls through
                        // to the general delegation below for every other
                        // shape (a sync generator, a real async iterable
                        // object, or a generator with extra arguments).
                        if let [source] = arguments.as_slice() {
                            let source_type = self.infer_expr_type(source)?;
                            if is_generator_producer_type(&source_type) {
                    if let Some((collected, _)) =
                        self.collect_async_generator_for_array_spread(source.clone(), &source_type)?
                                {
                                    return self.wrap_call_argument_bindings(collected, &bindings);
                                }
                            }
                if let Some((collected, _)) =
                    self.collect_promise_tuple_array_for_from_async(source.clone(), &source_type)?
                            {
                                return self.wrap_call_argument_bindings(collected, &bindings);
                            }
                if let Some((collected, _)) =
                    self.collect_promise_array_for_from_async(source.clone(), &source_type)?
                            {
                                return self.wrap_call_argument_bindings(collected, &bindings);
                            }
                if let Some((collected, _)) =
                    self.collect_promise_union_array_for_from_async(source.clone(), &source_type)?
                            {
                                return self.wrap_call_argument_bindings(collected, &bindings);
                            }
                        }
                        let array = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("Array".to_string()))],
                        );
                        let mut json_arguments = Vec::with_capacity(arguments.len());
                        for argument in &arguments {
                            // A mapper callback must cross as a live
                            // callback, not a JSON value.
                            let value = if matches!(
                                self.infer_expr_type(argument)?,
                                HirType::Function(_, _) | HirType::CallableFunction(..)
                            ) {
                                self.coerce_to_declared(&HirType::JsValue, argument.clone())?
                            } else {
                                self.coerce_to_declared(&HirType::Json, argument.clone())?
                            };
                            json_arguments.push(value);
                        }
            let json_arguments =
                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_arguments))?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                            vec![
                                array,
                                HirExpr::Lit(HirLit::Str("fromAsync".to_string())),
                                json_arguments,
                            ],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Array" && property.sym == *"isArray" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Array.isArray")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Array.isArray` expects exactly one argument".into());
                        };
                        let value = self.lower_primitive_array_operand(value.clone())?;
                        // A `Buffer` / `Uint8Array` is not an Array.
                        if self.infer_expr_type_inner(&value)? == HirType::Bytes {
                            return self.wrap_call_argument_bindings(
                                HirExpr::EvalThen(
                                    Box::new(value),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                ),
                                &bindings,
                            );
                        }
                        let ty = self.infer_expr_type(&value)?;
                        if let HirType::Union(elements) = &ty {
                            let matching = elements
                                .iter()
                                .enumerate()
                                .filter_map(|(index, element)| {
                        (matches!(element, HirType::Array(_) | HirType::Tuple(_))
                            || matches!(native_base_type(element), Some(HirType::Array(_))))
                        .then_some(index)
                                })
                                .collect::<Vec<_>>();
                            let name = format!("__thaw_is_array_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                let mut result = HirExpr::Lit(HirLit::Bool(matching.len() == elements.len()));
                            if matching.len() != elements.len() {
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
                            bindings.push((name, ty, value));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        // A live `JsValue` (a dynamically-read field/member of
                        // an `any`-typed class instance) has no local array
                        // representation to inspect -- ask the realm's own
                        // `Array.isArray` (the same live-engine query
                        // `typeof`/`is_undefined` already use). Without this
                        // the operand fell to the final `else` below and
                        // answered a compile-time `false` for a genuine array.
                        if ty == HirType::JsValue {
                let result = self.dynamic_value_check("__thaw_is_array_dynamic_value", value);
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if ty == HirType::Json {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_array".to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let name = format!("__thaw_is_array_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name, ty.clone(), value));
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(
                                matches!(ty, HirType::Array(_) | HirType::Tuple(_))
                                    || matches!(native_base_type(&ty), Some(HirType::Array(_))),
                            )),
                            &bindings,
                        );
                    }
                    if (object.sym == *"Object"
                        && matches!(property.sym.as_ref(), "keys" | "getOwnPropertyNames"))
                        || (object.sym == *"Reflect" && property.sym == *"ownKeys")
                    {
                        let label = format!("{}.{}", object.sym, property.sym);
            let (arguments, mut bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let mut value = value.clone();
                        let mut ty = self.infer_expr_type(&value)?;
                        if Self::array_subclass_own_fields(&ty).is_some() {
                            let name = format!("__thaw_subclass_object_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            let all = property.sym != *"keys";
                            let result = self.lower_array_subclass_keys(HirExpr::Var(name.clone()), &ty, all)?;
                            bindings.push((name, ty, value));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let Some(base @ HirType::Array(_)) = native_base_type(&ty).cloned() {
                            value = self.project_native_base(value)?;
                            ty = base;
                        }
                        value = self.bytes_to_f64_array(value)?;
                        if let HirType::Union(members) = &ty {
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_)))
                {
                                (value, ty) = self.lower_union_array_sequence(value, members)?;
                            }
                        }
                        if ty == HirType::JsValue {
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                            let holder = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(object.sym.to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    holder,
                                    HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if matches!(ty, HirType::Json | HirType::Dictionary(_)) {
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var(
                        if property.sym == *"keys" {
                                    "__thaw_json_keys"
                                } else {
                                    "__thaw_json_own_keys"
                        }
                        .to_string(),
                    )),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if matches!(ty, HirType::Array(_) | HirType::Tuple(_)) {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_keys".to_string())),
                    vec![value, HirExpr::Lit(HirLit::Bool(property.sym != *"keys"))],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let (true, HirType::Union(members)) =
                            (ty == crate::caught_exception_carrier_type(), &ty)
                        {
                            // `catch (e: any)`: the active member decides between an Error text
                            // (side-table keys) and a live object snapshot.
                            let name = format!("__thaw_keys_carrier_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            let var = HirExpr::Var(name.clone());
                            let is_text = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(var.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(3.0))),
                            );
                            let text = HirExpr::UnionValue(Box::new(var.clone()), 3, members.clone());
                            let keys_runtime =
                                if property.sym == *"keys" { "__thaw_json_keys" } else { "__thaw_json_own_keys" };
                            let error_keys = HirExpr::Call(
                                Box::new(HirExpr::Var(keys_runtime.to_string())),
                                vec![HirExpr::Call(
                                    Box::new(HirExpr::Var("JSON.parse".to_string())),
                                    vec![HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_error_own_keys".to_string())),
                                        vec![text, HirExpr::Lit(HirLit::Bool(property.sym != *"keys"))],
                                    )],
                                )],
                            );
                            let live_keys = HirExpr::Call(
                                Box::new(HirExpr::Var(keys_runtime.to_string())),
                                vec![self.caught_carrier_as_json(var.clone())?],
                            );
                            let keys_type = HirType::Array(Box::new(if property.sym == *"keys" { HirType::Str } else { HirType::Json }));
                            let owner_keys = self.thrown_error_owner_branch(
                                &var, members, property.sym != *"keys", live_keys.clone(), keys_type.clone(),
                                |props| HirExpr::Call(
                                    Box::new(HirExpr::Var(keys_runtime.to_string())),
                                    vec![HirExpr::Call(Box::new(HirExpr::Var("JSON.parse".to_string())), vec![props])],
                                ),
                            );
                            let result = HirExpr::Conditional(
                                Box::new(is_text),
                                Box::new(error_keys),
                                Box::new(owner_keys),
                                keys_type,
                            );
                            bindings.push((name, ty.clone(), value));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if ty == HirType::Str {
                            // An Error value: its own keys come from the runtime side table (a
                            // plain string yields its index keys, like `Object.keys("ab")`).
                            let text = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_error_own_keys".to_string())),
                                vec![value, HirExpr::Lit(HirLit::Bool(property.sym != *"keys"))],
                            );
                            let parsed = HirExpr::Call(Box::new(HirExpr::Var("JSON.parse".to_string())), vec![text]);
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_keys".to_string())),
                                vec![parsed],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`{label}` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let mut names = ecmascript_field_order(fields);
                        let hidden_extra = Self::error_record_hidden_fields(fields);
                        let default_name = Self::error_family_default_name(fields);
                        let name_index = default_name.as_ref().and_then(|_| {
                            names.retain(|&index| {
                                fields[index].0 != "message"
                                    && (property.sym != *"keys"
                                        || !hidden_extra.contains(&fields[index].0.as_str()))
                            });
                            names.iter().position(|&index| fields[index].0 == "name").map(|at| names.remove(at))
                        });
                        let name = format!("__thaw_object_keys_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let result_name = format!("__thaw_object_key_result_{}", self.next_binding);
                        self.next_binding += 1;
                        let array_type = HirType::Array(Box::new(HirType::Str));
                        self.scope.insert(result_name.clone(), array_type.clone());
                        let mut body = vec![HirStmt::Let(
                result_name.clone(),
                array_type,
                HirExpr::ArrayLit(Vec::new()),
                        )];
                        if default_name.is_some() && property.sym != *"keys" {
                            for hidden in ["stack", "message"] {
                                body.push(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_push".into())),
                                    vec![
                                        HirExpr::Var(result_name.clone()),
                                        HirExpr::Lit(HirLit::Str(hidden.to_string())),
                                    ],
                                )));
                            }
                        }
            let actions = names
                .into_iter()
                .map(|index| {
                            let key = HirExpr::Lit(HirLit::Str(fields[index].0.clone()));
                            let push = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_push".into())),
                                vec![HirExpr::Var(result_name.clone()), key],
                            );
                            (index, vec![HirStmt::Expr(push)])
                })
                .collect();
                        body.extend(self.lower_ordered_fixed_field_statements(
                HirExpr::Var(name.clone()),
                fields,
                actions,
                        )?);
                        if default_name.is_none() && !fields.iter().any(|(field, _)| field == "stack") {
                            // A side-table `stack` (captured: hidden, assigned: enumerable) is an own key.
                            let probe = if property.sym == *"keys" { "__thaw_object_stack_visible" } else { "__thaw_object_stack_has" };
                            body.push(HirStmt::If(
                                HirExpr::Call(Box::new(HirExpr::Var(probe.into())), vec![HirExpr::Var(name.clone())]),
                                vec![HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_push".into())),
                                    vec![HirExpr::Var(result_name.clone()), HirExpr::Lit(HirLit::Str("stack".to_string()))],
                                ))],
                                Vec::new(),
                            ));
                        }
                        if let (Some(default_name), Some(_)) = (default_name, name_index) {
                            let push = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_push".into())),
                                vec![
                                    HirExpr::Var(result_name.clone()),
                                    HirExpr::Lit(HirLit::Str("name".to_string())),
                                ],
                            );
                            let statement = self.error_family_name_statement(
                                &HirExpr::Var(name.clone()), fields, default_name, push,
                            )?;
                            body.push(statement);
                        }
                        body.push(HirStmt::Return(Some(HirExpr::Var(result_name))));
                        let result = HirExpr::Block(body);
                        let native_object = HirExpr::Var(name.clone());
                        let result = HirExpr::Conditional(
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_has_full_layout".into())),
                                vec![native_object.clone()],
                            )),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_object_full_own_keys".into())),
                                vec![
                                    native_object,
                                    HirExpr::Lit(HirLit::Bool(property.sym != *"keys")),
                                ],
                            )),
                            Box::new(result),
                            HirType::Array(Box::new(HirType::Str)),
                        );
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"values" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Object.values")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Object.values` expects exactly one argument".into());
                        };
                        let mut value = value.clone();
                        let mut ty = self.infer_expr_type(&value)?;
                        if let HirType::Union(members) = &ty {
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_)))
                {
                                (value, ty) = self.lower_union_array_sequence(value, members)?;
                            }
                        }
                        if ty == HirType::JsValue {
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("values".to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if ty == HirType::Json {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_values".to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let HirType::Dictionary(element) = &ty {
                            let runtime = match element.as_ref() {
                                HirType::F64 => "__thaw_json_number_values",
                                HirType::Str => "__thaw_json_string_values",
                                HirType::Bool => "__thaw_json_bool_values",
                                HirType::Json => "__thaw_json_values",
                                other => {
                                    return Err(format!(
                                        "`Object.values` does not support dictionary value type {other:?}"
                        ));
                                }
                            };
                let result =
                    HirExpr::Call(Box::new(HirExpr::Var(runtime.to_string())), vec![value]);
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if Self::array_subclass_own_fields(&ty).is_some() {
                            let result = self.lower_array_subclass_sequence(value, &ty, false)?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let Some(base @ HirType::Array(_)) = native_base_type(&ty).cloned() {
                            value = self.project_native_base(value)?;
                            ty = base;
                        }
                        value = self.bytes_to_f64_array(value)?;
                        if let HirType::Array(element) = &ty {
                            let element = element.as_ref().clone();
                            let result = self.lower_array_filter(
                                value,
                                ty.clone(),
                                element.clone(),
                                element,
                                HirExpr::Lambda(
                        Vec::new(),
                        Vec::new(),
                        HirType::Bool,
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                ),
                                None,
                            )?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.values` currently requires a fixed object, got {ty:?}"
                            ));
                        };
            if fields
                .iter()
                .any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}"))
            {
                            let values = self.lower_visible_fixed_object_sequence(value, fields, false)?;
                            return self.wrap_call_argument_bindings(values, &bindings);
                        }
                        let field_names = ecmascript_field_order(fields)
                            .into_iter()
                            .map(|index| fields[index].0.clone())
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_object_values_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let values = HirExpr::ArrayLit(
                            field_names
                                .into_iter()
                                .map(|field| {
                                    self.lower_fixed_object_property_read(
                                        HirExpr::Var(name.clone()),
                                        fields,
                                        &field,
                                    )
                                    .expect("value field was taken from its source type")
                                })
                                .collect(),
                        );
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(values, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"entries" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Object.entries")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Object.entries` expects exactly one argument".into());
                        };
                        let mut value = value.clone();
                        let mut ty = self.infer_expr_type(&value)?;
                        if let HirType::Union(members) = &ty {
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_)))
                {
                                (value, ty) = self.lower_union_array_sequence(value, members)?;
                            }
                        }
                        if ty == HirType::JsValue {
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("entries".to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if ty == HirType::Json {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_entries".to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let error_entries = |text: HirExpr| {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_entries".to_string())),
                                vec![HirExpr::Call(
                                    Box::new(HirExpr::Var("JSON.parse".to_string())),
                                    vec![HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_error_entries_json".to_string())),
                                        vec![text],
                                    )],
                                )],
                            )
                        };
                        if ty == HirType::Str {
                            // An Error value: own enumerable props from the runtime side table.
                            let result = error_entries(value);
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let (true, HirType::Union(members)) = (ty == crate::caught_exception_carrier_type(), &ty) {
                            // `catch (e: any)`: an Error text uses the side table, other members the live snapshot.
                            let name = format!("__thaw_entries_carrier_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            let var = HirExpr::Var(name.clone());
                            let is_text = HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(var.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(3.0))),
                            );
                            let text = HirExpr::UnionValue(Box::new(var.clone()), 3, members.clone());
                            let live_entries = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_entries".to_string())),
                                vec![self.caught_carrier_as_json(var.clone())?],
                            );
                            let entries_type = HirType::Array(Box::new(HirType::Tuple(vec![HirType::Str, HirType::Json])));
                            let owner_entries = self.thrown_error_owner_branch(
                                &var, members, false, live_entries.clone(), entries_type.clone(),
                                |props| HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_entries".to_string())),
                                    vec![HirExpr::Call(Box::new(HirExpr::Var("JSON.parse".to_string())), vec![props])],
                                ),
                            );
                            let result = HirExpr::Conditional(
                                Box::new(is_text),
                                Box::new(error_entries(text)),
                                Box::new(owner_entries),
                                entries_type,
                            );
                            bindings.push((name, ty.clone(), value));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let HirType::Dictionary(element) = &ty {
                            let runtime = match element.as_ref() {
                                HirType::F64 => "__thaw_json_number_entries",
                                HirType::Str => "__thaw_json_string_entries",
                                HirType::Bool => "__thaw_json_bool_entries",
                                HirType::Json => "__thaw_json_entries",
                                other => {
                                    return Err(format!(
                                        "`Object.entries` does not support dictionary value type {other:?}"
                        ));
                                }
                            };
                let result =
                    HirExpr::Call(Box::new(HirExpr::Var(runtime.to_string())), vec![value]);
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if Self::array_subclass_own_fields(&ty).is_some() {
                            let result = self.lower_array_subclass_sequence(value, &ty, true)?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let Some(base @ HirType::Array(_)) = native_base_type(&ty).cloned() {
                            value = self.project_native_base(value)?;
                            ty = base;
                        }
                        value = self.bytes_to_f64_array(value)?;
                        if let HirType::Array(element) = &ty {
                            let receiver_name = format!("__thaw_array_entries_source_{}", self.next_binding);
                            self.next_binding += 1;
                            let key_name = format!("__thaw_array_entries_key_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(receiver_name.clone(), ty.clone());
                            self.scope.insert(key_name.clone(), HirType::Str);
                            let receiver = HirExpr::Var(receiver_name.clone());
                            let keys = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_keys".into())),
                                vec![receiver.clone(), HirExpr::Lit(HirLit::Bool(false))],
                            );
                            let key = HirExpr::Var(key_name.clone());
                            let index = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_to_number".into())),
                                vec![key.clone()],
                            );
                let entry_value =
                    self.lower_array_at(receiver, ty.clone(), element.as_ref().clone(), index)?;
                let entry_type =
                    HirType::Tuple(vec![HirType::Str, self.infer_expr_type(&entry_value)?]);
                            let callback = HirExpr::Lambda(
                    vec![HirParam {
                        name: receiver_name.clone(),
                        ty: ty.clone(),
                    }],
                    vec![HirParam {
                        name: key_name,
                        ty: HirType::Str,
                    }],
                                entry_type,
                                Box::new(HirExpr::ArrayLit(vec![key, entry_value])),
                            );
                            let entries = self.lower_array_map(
                                keys,
                                HirType::Array(Box::new(HirType::Str)),
                                HirType::Str,
                                HirType::Str,
                                callback,
                                None,
                            )?;
                            bindings.push((receiver_name, ty, value));
                            return self.wrap_call_argument_bindings(entries, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.entries` currently requires a fixed object, got {ty:?}"
                            ));
                        };
            if fields
                .iter()
                .any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}"))
            {
                            let entries = self.lower_visible_fixed_object_sequence(value, fields, true)?;
                            return self.wrap_call_argument_bindings(entries, &bindings);
                        }
                        let entry_fields = ecmascript_field_order(fields)
                            .into_iter()
                            .map(|index| {
                                let name = fields[index].0.clone();
                                let ty = Self::fixed_object_property_read_type(fields, &name)
                                    .expect("entry field was taken from its source type");
                                (name, ty)
                            })
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_object_entries_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let entries = HirExpr::ArrayLit(
                            entry_fields
                                .into_iter()
                                .map(|(field, field_type)| {
                                    let entry_type = HirType::Tuple(vec![HirType::Str, field_type]);
                                    HirExpr::Call(
                                        Box::new(HirExpr::Lambda(
                                            vec![HirParam {
                                                name: name.clone(),
                                                ty: ty.clone(),
                                            }],
                                            Vec::new(),
                                            entry_type,
                                            Box::new(HirExpr::ArrayLit(vec![
                                                HirExpr::Lit(HirLit::Str(field.clone())),
                                                self.lower_fixed_object_property_read(
                                                    HirExpr::Var(name.clone()),
                                                    fields,
                                                    &field,
                                                )
                                                .expect("entry field was taken from its source type"),
                                            ])),
                                        )),
                                        Vec::new(),
                                    )
                                })
                                .collect(),
                        );
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(entries, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"fromEntries" {
                        let contextual_args = contextual_from_entries_arguments(&call.args);
                        let (mut arguments, mut bindings) =
                            self.lower_native_spread_values(contextual_args.as_deref().unwrap_or(&call.args), "Object.fromEntries")?;
                        let [entries] = arguments.as_slice() else {
                            return Err("`Object.fromEntries` expects exactly one argument".into());
                        };
                        let mut entries = entries.clone();
                        let mut ty = self.infer_expr_type(&entries)?;
                        // A number-keyed map: keys become property-name strings, so re-lower
                        // `Array.from(map).map(e => [String(e[0]), e[1]])` (the first lowering is discarded).
                        if matches!(&ty, HirType::Map(key, _) if **key == HirType::F64) {
                            if let Some(rewritten) = number_map_entries_arguments(&call.args) {
                                let contextual = contextual_from_entries_arguments(&rewritten);
                                (arguments, bindings) = self.lower_native_spread_values(
                                    contextual.as_deref().unwrap_or(&rewritten),
                                    "Object.fromEntries",
                                )?;
                                entries = arguments[0].clone();
                                ty = self.infer_expr_type(&entries)?;
                            }
                        }
                        // `Object.fromEntries(map)`: a string-keyed map iterates as its entries.
                        let (entries, ty) = match &ty {
                            HirType::Map(key, _) if **key == HirType::Str => {
                                let pairs = self.native_collection_entries(entries)?;
                                let pairs_type = self.infer_expr_type(&pairs)?;
                                (pairs, pairs_type)
                            }
                            _ => (entries, ty),
                        };
                        if ty == HirType::JsValue {
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![entries]))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("fromEntries".to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let HirType::Union(members) = &ty {
                let source_name = format!("__thaw_from_entries_union_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(source_name.clone(), ty.clone());
                            let mut branches = Vec::with_capacity(members.len());
                            for (index, member) in members.iter().enumerate() {
                                let HirType::Array(entry) = member else {
                                    return Err(format!(
                                        "`Object.fromEntries` union member {index} is not an entry array: {member:?}"
                                    ));
                                };
                                let HirType::Tuple(elements) = entry.as_ref() else {
                                    return Err(format!(
                                        "`Object.fromEntries` union member {index} must contain [string, value] tuples, got {entry:?}"
                                    ));
                                };
                                let [HirType::Str, element] = elements.as_slice() else {
                                    return Err(format!(
                                        "`Object.fromEntries` union member {index} must contain [string, value] tuples, got {elements:?}"
                                    ));
                                };
                                let runtime = match element {
                                    HirType::F64 => "__thaw_json_object_from_number_entries",
                                    HirType::Str => "__thaw_json_object_from_string_entries",
                                    HirType::Bool => "__thaw_json_object_from_bool_entries",
                                    HirType::Json => "__thaw_json_object_from_json_entries",
                                    other => {
                                        return Err(format!(
                                            "`Object.fromEntries` union member {index} has unsupported value type {other:?}"
                            ));
                                    }
                                };
                                branches.push(HirExpr::Call(
                                    Box::new(HirExpr::Var(runtime.to_string())),
                                    vec![HirExpr::UnionValue(
                                        Box::new(HirExpr::Var(source_name.clone())),
                                        index,
                                        members.clone(),
                                    )],
                                ));
                            }
                let result =
                    self.merge_union_array_method_branches(&source_name, members, branches)?;
                let result =
                    self.wrap_call_argument_bindings(result, &[(source_name, ty, entries)])?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Array(entry) = &ty else {
                            return Err(format!(
                                "`Object.fromEntries` requires an entry array, got {ty:?}"
                            ));
                        };
                        let HirType::Tuple(elements) = entry.as_ref() else {
                            return Err(format!(
                                "`Object.fromEntries` requires [string, value] tuples, got {entry:?}"
                            ));
                        };
                        let [HirType::Str, element] = elements.as_slice() else {
                            return Err(format!(
                                "`Object.fromEntries` requires [string, value] tuples, got {elements:?}"
                            ));
                        };
                        let runtime = match element {
                            HirType::F64 => "__thaw_json_object_from_number_entries",
                            HirType::Str => "__thaw_json_object_from_string_entries",
                            HirType::Bool => "__thaw_json_object_from_bool_entries",
                            HirType::Json => "__thaw_json_object_from_json_entries",
                            other => {
                                return Err(format!(
                                    "`Object.fromEntries` does not support value type {other:?}"
                    ));
                            }
                        };
            let result = HirExpr::Call(Box::new(HirExpr::Var(runtime.to_string())), vec![entries]);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"assign" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.assign")?;
                        let Some((target, sources)) = arguments.split_first() else {
                            return Err("`Object.assign` expects at least one argument".into());
                        };
                        let target_type = self.infer_expr_type(target)?;
                        let source_types = sources
                            .iter()
                            .map(|source| self.infer_expr_type(source))
                            .collect::<Result<Vec<_>, _>>()?;
            if target_type == HirType::JsValue || source_types.contains(&HirType::JsValue) {
                            let arguments = arguments
                                .iter()
                                .map(|argument| {
                                    if self.infer_expr_type(argument)? == HirType::JsValue {
                                        Ok(argument.clone())
                                    } else {
                                        self.coerce_to_declared(&HirType::Json, argument.clone())
                                    }
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                let call_arguments =
                    self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(arguments))?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("assign".to_string())),
                                    call_arguments,
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        // An Error target (a framed string): copy own properties into its side table.
                        if target_type == HirType::Str {
                            let target_name = format!("__thaw_assign_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(target_name.clone(), HirType::Str);
                            let mut statements = Vec::new();
                            for (source, source_type) in sources.iter().zip(&source_types) {
                                match source_type {
                                    HirType::Object(fields) => {
                                        let source_name = format!("__thaw_assign_source_{}", self.next_binding);
                                        self.next_binding += 1;
                                        self.scope.insert(source_name.clone(), source_type.clone());
                                        statements.push(HirStmt::Let(source_name.clone(), source_type.clone(), source.clone()));
                                        for index in enumerable_field_order(fields) {
                                            let (name, field_type) = &fields[index];
                                            if !json_convertible_native_type(field_type) {
                                                return Err(format!("`Object.assign` onto an Error cannot copy field `{name}` of type {field_type:?}"));
                                            }
                                            let value = HirExpr::PropAccess(
                                                Box::new(HirExpr::Var(source_name.clone())),
                                                source_type.clone(),
                                                name.clone(),
                                            );
                                            let json = self.coerce_to_declared(&HirType::Json, value)?;
                                            statements.push(HirStmt::Expr(HirExpr::Call(
                                                Box::new(HirExpr::Var("__thaw_error_set_prop".to_string())),
                                                vec![
                                                    HirExpr::Var(target_name.clone()),
                                                    HirExpr::Lit(HirLit::Str(name.clone())),
                                                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_encode".to_string())), vec![json]),
                                                ],
                                            )));
                                        }
                                    }
                                    HirType::Json => statements.push(HirStmt::Expr(HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_error_assign_json".to_string())),
                                        vec![
                                            HirExpr::Var(target_name.clone()),
                                            HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_encode".to_string())), vec![source.clone()]),
                                        ],
                                    ))),
                                    other => {
                                        return Err(format!("`Object.assign` onto an Error requires object or JSON sources, got {other:?}"));
                                    }
                                }
                            }
                            statements.push(HirStmt::Return(Some(HirExpr::Var(target_name.clone()))));
                            let mut assign_bindings = bindings.clone();
                            assign_bindings.push((target_name, HirType::Str, target.clone()));
                            return self.wrap_call_argument_bindings(HirExpr::Block(statements), &assign_bindings);
                        }
                        // A native object target mutates in place: each source
                        // field is written to the same-named, same-typed target
                        // field in source order, and the target is the result.
                        if let HirType::Object(target_fields) = &target_type {
                            let fits = |source_type: &HirType| match source_type {
                    HirType::Object(fields) => {
                        enumerable_field_order(fields).into_iter().all(|index| {
                                        let (name, ty) = &fields[index];
                                        target_fields.iter().any(|(target_name, target_ty)| {
                                            target_name == name
                                                && (target_ty == ty
                                                    || matches!(target_ty, HirType::Optional(payload) if payload.as_ref() == ty))
                                        })
                        })
                    }
                                _ => false,
                            };
                            if !sources.is_empty() && source_types.iter().all(fits) {
                    let target_name = format!("__thaw_assign_target_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(target_name.clone(), target_type.clone());
                                let mut assign_bindings = bindings.clone();
                                assign_bindings.push((
                                    target_name.clone(),
                                    target_type.clone(),
                                    target.clone(),
                                ));
                                let mut statements = Vec::new();
                                for (source, source_type) in sources.iter().zip(&source_types) {
                        let HirType::Object(fields) = source_type else {
                            unreachable!()
                        };
                        let source_name = format!("__thaw_assign_source_{}", self.next_binding);
                                    self.next_binding += 1;
                                    self.scope.insert(source_name.clone(), source_type.clone());
                                    statements.push(HirStmt::Let(
                                        source_name.clone(),
                                        source_type.clone(),
                                        source.clone(),
                                    ));
                                    for index in enumerable_field_order(fields) {
                                        let (name, field_ty) = &fields[index];
                                        let mut value = HirExpr::PropAccess(
                                            Box::new(HirExpr::Var(source_name.clone())),
                                            source_type.clone(),
                                            name.clone(),
                                        );
                                        // A target field stored as `Optional(T)` (`x?: T`,
                                        // or possibly uninitialised) takes the value as `Some`.
                                        if target_fields.iter().any(|(target_name, target_ty)| {
                                            target_name == name
                                                && matches!(target_ty, HirType::Optional(payload) if payload.as_ref() == field_ty)
                                        }) {
                                            value = HirExpr::OptionalSome(Box::new(value), field_ty.clone());
                                        }
                                        statements.push(HirStmt::Expr(HirExpr::PropAssign(
                                            Box::new(HirExpr::Var(target_name.clone())),
                                            target_type.clone(),
                                            name.clone(),
                                            Box::new(value),
                                        )));
                                    }
                                }
                                statements.push(HirStmt::Return(Some(HirExpr::Var(target_name))));
                                return self
                                    .wrap_call_argument_bindings(HirExpr::Block(statements), &assign_bindings);
                            }
                        }
                        // When every operand is *already* the exact same
                        // `Json`/`Dictionary` type, keep the original
                        // behavior byte-for-byte: the result stays that
                        // same type (so e.g. assigning it to a declared
                        // `Record<string, number>` still coerces), instead
                        // of normalizing to plain `Json` and losing that.
                        // Otherwise (a plain object/interface literal target
                        // or source, or a type mismatch between operands),
                        // normalize everything to `Json` via
                        // `wrap_native_value_as_json` -- the same
                        // native-value support `JSON.stringify` already
                        // has -- and let `__thaw_json_object_assign`'s
                        // matching-operand check enforce the rest.
                        let uniform_existing = matches!(target_type, HirType::Json | HirType::Dictionary(_))
                && source_types
                    .iter()
                    .all(|source_type| *source_type == target_type);
                        let mut result = target.clone();
                        // `Object.assign(jsonTarget, nativeObjectWithGetters)` reads and
                        // writes one property at a time, so a getter observes the
                        // properties already copied and an exception leaves them behind.
                        let property_wise = target_type == HirType::Json
                            && !sources.is_empty()
                            && source_types.iter().all(|source_type| match source_type {
                                HirType::Object(fields) => {
                                    // Property-wise copying exists for getters; an accessor
                                    // pair with only a setter keeps the snapshot path.
                                    let accessor = |prefix: &str, name: &str| {
                            fields
                                .iter()
                                .any(|(field, _)| field.strip_prefix(prefix) == Some(name))
                                    };
                                    let has_getter = fields
                                        .iter()
                                        .any(|(field, _)| field.starts_with("__thaw_getter_"));
                                    let setter_only = fields.iter().any(|(field, _)| {
                            field
                                .strip_prefix("__thaw_setter_")
                                .is_some_and(|name| !accessor("__thaw_getter_", name))
                                    });
                                    has_getter
                                        && !setter_only
                                        && enumerable_field_order(fields).into_iter().all(|index| {
                                            let (name, ty) = &fields[index];
                                            !name.starts_with('\u{1f}') && json_convertible_native_type(ty)
                                        })
                                }
                                _ => false,
                            });
                        if property_wise {
                            let target_name = format!("__thaw_assign_target_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(target_name.clone(), HirType::Json);
                            let mut statements = vec![HirStmt::Let(
                                target_name.clone(),
                                HirType::Json,
                                target.clone(),
                            )];
                            for (source, source_type) in sources.iter().zip(&source_types) {
                    let HirType::Object(fields) = source_type else {
                        unreachable!()
                    };
                                let source_name = format!("__thaw_assign_source_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(source_name.clone(), source_type.clone());
                                statements.push(HirStmt::Let(
                                    source_name.clone(),
                                    source_type.clone(),
                                    source.clone(),
                                ));
                                for index in enumerable_field_order(fields) {
                                    let (name, field_type) = &fields[index];
                                    let value = self.coerce_to_declared(
                                        &HirType::Json,
                                        HirExpr::PropAccess(
                                            Box::new(HirExpr::Var(source_name.clone())),
                                            source_type.clone(),
                                            name.clone(),
                                        ),
                                    )?;
                                    let _ = field_type;
                                    statements.push(HirStmt::Expr(HirExpr::JsonSet(
                                        Box::new(HirExpr::Var(target_name.clone())),
                                        Box::new(HirExpr::Lit(HirLit::Str(name.clone()))),
                                        Box::new(value),
                                        HirType::Json,
                                        false,
                                    )));
                                }
                            }
                            statements.push(HirStmt::Return(Some(HirExpr::Var(target_name))));
                            return self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings);
                        }
                        if uniform_existing {
                            for source in sources {
                                result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                                    vec![result, source.clone()],
                                );
                            }
                        } else {
                            let normalize = |lowerer: &mut Self,
                                              value: HirExpr,
                                              value_type: HirType|
                             -> Result<HirExpr, String> {
                                if value_type == HirType::Json {
                                    Ok(value)
                                } else if json_convertible_native_type(&value_type) {
                                    lowerer.wrap_native_value_as_json(value, value_type)
                                } else {
                                    Err(format!(
                                        "`Object.assign` requires a JSON-convertible value, got {value_type:?}"
                                    ))
                                }
                            };
                            result = normalize(self, result, target_type)?;
                            for (source, source_type) in sources.iter().zip(source_types) {
                                let source = normalize(self, source.clone(), source_type)?;
                                result = HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                                    vec![result, source],
                                );
                            }
                        }
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"hasOwn" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.hasOwn")?;
                        let [object_value, key_value] = arguments.as_slice() else {
                            return Err("`Object.hasOwn` expects exactly two arguments".into());
                        };
                        let object_value = object_value.clone();
                        let object_type = self.infer_expr_type(&object_value)?;
                        if object_type == HirType::JsValue {
                            let key_value = if self.infer_expr_type(key_value)? == HirType::JsValue {
                                key_value.clone()
                            } else {
                                self.coerce_to_declared(&HirType::Json, key_value.clone())?
                            };
                            let call_arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![object_value, key_value]),
                            )?;
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str("Object".to_string()))],
                            );
                            let result = HirExpr::JsonAsBool(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicMethod".to_string())),
                                vec![
                                    object,
                                    HirExpr::Lit(HirLit::Str("hasOwn".to_string())),
                                    call_arguments,
                                ],
                            )));
                            // A write through a derived class (`Child.value = 2`) creates an own
                            // static property that only the native shadow flag records.
                            let shadowed = match (call.args.first().map(|a| a.expr.as_ref()),
                                call.args.get(1).map(|a| a.expr.as_ref())) {
                                (Some(Expr::Ident(class)), Some(Expr::Lit(Lit::Str(key)))) => {
                                    let accessor = format!("__thaw_inherited_static_has_own_{}_{}",
                                        class.sym, key.value.to_string_lossy());
                                    self.signatures.contains_key(&accessor).then_some(accessor)
                                }
                                _ => None,
                            };
                            let result = match shadowed {
                                Some(accessor) => HirExpr::Conditional(
                                    Box::new(HirExpr::Call(Box::new(HirExpr::Var(accessor)), Vec::new())),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    Box::new(result),
                                    HirType::Bool,
                                ),
                                None => result,
                            };
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let result = self.lower_has_own_value(object_value, key_value.clone())?;
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"is" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Object.is")?;
                        let [left_value, right_value] = arguments.as_slice() else {
                            return Err("`Object.is` expects exactly two arguments".into());
                        };
                        let left_value = left_value.clone();
                        let right_value = right_value.clone();
                        let left_type = self.infer_expr_type(&left_value)?;
                        let right_type = self.infer_expr_type(&right_value)?;
                        if left_type == HirType::Dynamic || right_type == HirType::Dynamic {
                            return Err("`Object.is` requires statically native operands".into());
                        }
                        let left_name = format!("__thaw_object_is_left_{}", self.next_binding);
                        self.next_binding += 1;
                        let right_name = format!("__thaw_object_is_right_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(left_name.clone(), left_type.clone());
                        self.scope.insert(right_name.clone(), right_type.clone());
                        let left = HirExpr::Var(left_name.clone());
                        let right = HirExpr::Var(right_name.clone());
                        let result = if left_type == HirType::Json || right_type == HirType::Json {
                            let (json, native, native_type) = if left_type == HirType::Json {
                                (left, right, &right_type)
                            } else {
                                (right, left, &left_type)
                            };
                            let runtime = match native_type {
                                HirType::Json => "__thaw_json_object_is",
                                HirType::F64 => "__thaw_json_object_is_number",
                                HirType::Str => "__thaw_json_object_is_string",
                                HirType::Bool => "__thaw_json_object_is_bool",
                                _ => "",
                            };
                            if matches!(native_type, HirType::Null | HirType::Undefined) {
                                // `null`/`undefined` operands compare by SameValue against the
                                // dynamic value's own null/undefined state.
                                let native = self.coerce_to_declared(&HirType::Json, native)?;
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_object_is".into())),
                                    vec![json, native],
                                )
                            } else if runtime.is_empty() {
                                HirExpr::Lit(HirLit::Bool(false))
                            } else {
                    HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), vec![json, native])
                            }
                        } else if left_type != right_type {
                            HirExpr::Lit(HirLit::Bool(false))
                        } else if left_type == HirType::F64 {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_number_object_is".into())),
                                vec![
                                    HirExpr::Var(left_name.clone()),
                                    HirExpr::Var(right_name.clone()),
                                ],
                            )
                        } else {
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(left_name.clone())),
                                Box::new(HirExpr::Var(right_name.clone())),
                            )
                        };
                        bindings.push((left_name, left_type, left_value));
                        bindings.push((right_name, right_type, right_value));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Iterator"
                        && matches!(property.sym.as_ref(), "concat" | "zip" | "zipKeyed")
                    {
                        // Lazy iterator composition lives in the JS realm
                        // (the bundled QuickJS provides
                        // `Iterator.concat`/`zip`/`zipKeyed`), so keep the
                        // result a live handle and let its own
                        // `.toArray()`/`.next()` run there too.
                        let label = format!("Iterator.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let json_arguments = arguments
                            .iter()
                .map(|argument| self.coerce_to_declared(&HirType::Json, argument.clone()))
                            .collect::<Result<Vec<_>, String>>()?;
            let json_arguments =
                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_arguments))?;
                        let holder = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("Iterator".to_string()))],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                            vec![
                                holder,
                                HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                                json_arguments,
                            ],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Proxy" && property.sym == *"revocable" {
                        // `Proxy.revocable(target, handler)` constructs a
                        // genuine JS proxy. The result holds a `revoke`
                        // *function*, which a JSON snapshot can't carry, so
                        // it stays a live handle.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Proxy.revocable")?;
                        let [target, handler] = arguments.as_slice() else {
                            return Err("`Proxy.revocable` expects two arguments".into());
                        };
                        let target = self.coerce_to_declared(&HirType::Json, target.clone())?;
                        let handler = self.coerce_to_declared(&HirType::Json, handler.clone())?;
            let json_arguments =
                self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![target, handler]))?;
                        let holder = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("Proxy".to_string()))],
                        );
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethodHandle".to_string())),
                            vec![
                                holder,
                                HirExpr::Lit(HirLit::Str("revocable".to_string())),
                                json_arguments,
                            ],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Intl"
                        && matches!(
                            property.sym.as_ref(),
                            "getCanonicalLocales" | "supportedValuesOf"
                        )
                    {
                        // The bundled QuickJS `Intl` polyfill doesn't expose
                        // these statics, so thaw approximates them: locale
                        // tags are canonicalized at compile time when they
                        // are literals, and `supportedValuesOf` returns a
                        // small representative list per key.
                        if call.args.len() != 1 || call.args[0].spread.is_some() {
                            return Err(format!(
                                "`Intl.{}` expects exactly one argument",
                                property.sym
                            ));
                        }
                        let argument = call.args[0].expr.as_ref();
                        if property.sym == *"supportedValuesOf" {
                            let Expr::Lit(Lit::Str(key)) = argument else {
                    return Err("`Intl.supportedValuesOf` requires a string-literal key".into());
                            };
                            let values = supported_values_of(&key.value.to_string_lossy());
                            return Ok(HirExpr::ArrayLit(
                                values
                                    .into_iter()
                                    .map(|value| HirExpr::Lit(HirLit::Str(value)))
                                    .collect(),
                            ));
                        }
            let canonical = |value: &str| HirExpr::Lit(HirLit::Str(canonicalize_locale(value)));
                        match argument {
                            Expr::Lit(Lit::Str(value)) => {
                                return Ok(HirExpr::ArrayLit(vec![canonical(
                                    &value.value.to_string_lossy(),
                                )]));
                            }
                            Expr::Array(array) => {
                                let mut seen: Vec<String> = Vec::new();
                                let mut locales = Vec::with_capacity(array.elems.len());
                                for element in &array.elems {
                                    let Some(element) = element else {
                            return Err("`Intl.getCanonicalLocales` does not support holes".into());
                                    };
                                    if element.spread.is_some() {
                            return Err("`Intl.getCanonicalLocales` does not support spread".into());
                                    }
                                    let Expr::Lit(Lit::Str(value)) = element.expr.as_ref() else {
                                        return Err(
                                "`Intl.getCanonicalLocales` requires string-literal locales".into(),
                                        );
                                    };
                                    let tag = canonicalize_locale(&value.value.to_string_lossy());
                                    if !seen.contains(&tag) {
                                        seen.push(tag.clone());
                                        locales.push(HirExpr::Lit(HirLit::Str(tag)));
                                    }
                                }
                                return Ok(HirExpr::ArrayLit(locales));
                            }
                            _ => {
                                return Err(
                                    "`Intl.getCanonicalLocales` requires a string or string-array literal"
                                        .into(),
                    );
                            }
                        }
                    }
                    if object.sym == *"Error" && property.sym == *"captureStackTrace" {
                        // `Error.captureStackTrace(target[, constructorOpt])`: there are no call
                        // frames to capture, so an Error value only fixes its `stack` header (read
                        // once, V8 serializes it lazily), a Json object gets a `stack` string, and
                        // any other target is left unchanged.
                        let (arguments, bindings) = if call.args.iter().any(|argument| argument.spread.is_some()) {
                            self.lower_native_spread_values(&call.args, "Error.captureStackTrace")?
                        } else {
                            // The constructor argument only trims frames; a member read like
                            // `this.constructor` that cannot be lowered has no effect to keep.
                            let mut lowered = Vec::new();
                            for (index, argument) in call.args.iter().enumerate() {
                                match self.lower_expr(&argument.expr) {
                                    Ok(value) => lowered.push(value),
                                    Err(_) if index > 0 && matches!(&*argument.expr, Expr::Member(_)) => {}
                                    Err(error) => return Err(error),
                                }
                            }
                            (lowered, Vec::new())
                        };
                        let Some(target) = arguments.first().cloned() else {
                            return Err("`Error.captureStackTrace` expects a target object".into());
                        };
                        let mut statements = Vec::new();
                        for extra in arguments.iter().skip(1) {
                            statements.push(HirStmt::Expr(extra.clone()));
                        }
                        let target_type = self.infer_expr_type(&target)?;
                        if let HirType::Object(fields) = &target_type {
                            let capture = self.lower_object_capture_stack(target, fields)?;
                            statements.insert(0, HirStmt::Expr(capture));
                        } else if target_type == HirType::Str {
                            statements.insert(0, HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_error_stack".to_string())),
                                vec![target],
                            )));
                        } else if target_type == HirType::Json {
                            // A plain object gets an own `stack` string built from its name/message.
                            statements.insert(0, HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_capture_stack_trace".to_string())),
                                vec![target],
                            )));
                        } else {
                            statements.insert(0, HirStmt::Expr(target));
                        }
                        statements.push(HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))));
                        return self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings);
                    }
                    if object.sym == *"Error" && property.sym == *"isError" {
                        // `Error.isError(value)`: true for a tagged error
                        // string (`new Error(...)`-family / a QuickJS-thrown
                        // error) or an instance of a user class extending one,
                        // false for any other value.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Error.isError")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Error.isError` expects exactly one argument".into());
                        };
                        let value = value.clone();
                        let value_type = self.infer_expr_type(&value)?;
                        let is_error_object = matches!(
                            &value_type,
                            HirType::Object(fields)
                                if fields.first().is_some_and(|(marker, ty)| {
                                    *ty == HirType::Bool
                                        && marker
                                            .strip_prefix("__thaw_class_identity_\u{1e}")
                                            .is_some_and(|chain| {
                                                chain.split('\u{1f}').any(|name| matches!(
                                                    name,
                                                    "Error"
                                                        | "TypeError"
                                                        | "RangeError"
                                                        | "SyntaxError"
                                                        | "ReferenceError"
                                                        | "EvalError"
                                                         | "URIError"
                                                         | "AggregateError"
                                                         | "SuppressedError"
                                                 ))
                                            })
                                })
                        );
                        let result = if is_error_object {
                            HirExpr::Lit(HirLit::Bool(true))
                        } else if value_type == HirType::Str {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_error_is_error".to_string())),
                                vec![value],
                            )
                        } else {
                            HirExpr::Lit(HirLit::Bool(false))
                        };
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
        if object.sym == *"BigInt" && matches!(property.sym.as_ref(), "asIntN" | "asUintN") {
                        // `BigInt.asIntN(bits, value)` / `asUintN(bits, value)`:
                        // thaw's BigInt is a fixed-width `i64`, so these are
                        // native bit-mask operations (see the runtime's
                        // `thaw_i64_as_int_n`/`thaw_i64_as_uint_n`), not the
                        // unbounded-precision wraparound real JS performs.
                        let label = format!("BigInt.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [bits, value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects two arguments"));
                        };
                        let bits = self.coerce_primitive_to_number(bits.clone())?;
                        let value = value.clone();
                        self.expect_type(&HirType::BigInt, &value, &format!("{label} value"))?;
                        let intrinsic = if property.sym == *"asIntN" {
                            "__thaw_i64_as_int_n"
                        } else {
                            "__thaw_i64_as_uint_n"
                        };
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(
                                Box::new(HirExpr::Var(intrinsic.to_string())),
                                vec![value, bits],
                            ),
                            &bindings,
                        );
                    }
        if object.sym == *"Number" && matches!(property.sym.as_ref(), "parseFloat" | "parseInt") {
                        return self.lower_parse_call(call, property.sym == *"parseInt");
                    }
                    if object.sym == *"Date" && property.sym == *"now" {
                        if !call.args.is_empty() {
                            return Err("`Date.now` expects no arguments".into());
                        }
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_now".to_string())),
                            Vec::new(),
                        ));
                    }
                    if object.sym == *"performance" && property.sym == *"now" {
                        if !call.args.is_empty() {
                            return Err("`performance.now` expects no arguments".into());
                        }
                        // Milliseconds since process start, not the Unix
                        // epoch `Date.now` reports -- a monotonic clock
                        // unaffected by wall-clock adjustments, matching the
                        // Web/Node `performance.now()` contract. Recognized
                        // as this exact call-expression pattern the same way
                        // `Math`/`Date`/`JSON` are, rather than as a real
                        // `performance` global value.
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_performance_now".to_string())),
                            Vec::new(),
                        ));
                    }
                    if object.sym == *"Date" && property.sym == *"UTC" {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Date.UTC")?;
                        if arguments.is_empty() || arguments.len() > 7 {
                            return Err("`Date.UTC` expects one to seven arguments".into());
                        }
                        // year has no meaningful default (it's always
                        // required), so its slot is unused below.
                        const DEFAULTS: [f64; 7] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
                        let mut bindings = spread_bindings;
                        let mut call_vars = Vec::new();
                        for (index, default) in DEFAULTS.iter().enumerate() {
                            let value = if let Some(argument) = arguments.get(index) {
                                self.coerce_primitive_to_number(argument.clone())?
                            } else {
                                HirExpr::Lit(HirLit::F64(*default))
                            };
                            let name = format!("__thaw_date_utc_arg_{index}_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), HirType::F64);
                            bindings.push((name.clone(), HirType::F64, value));
                            call_vars.push(HirExpr::Var(name));
                        }
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_utc".to_string())),
                            call_vars,
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Date" && property.sym == *"parse" {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Date.parse")?;
                        let [text] = arguments.as_slice() else {
                            return Err("`Date.parse` expects exactly one argument".into());
                        };
                        let text = self.coerce_primitive_to_string(text.clone())?;
                        let text_name = format!("__thaw_date_parse_text_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(text_name.clone(), HirType::Str);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_parse".to_string())),
                            vec![HirExpr::Var(text_name.clone())],
                        );
                        let mut bindings = spread_bindings;
                        bindings.push((text_name, HirType::Str, text));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Math" && property.sym == *"sumPrecise" {
                        // `Math.sumPrecise(numbers)`: the array's elements are
                        // summed natively with Neumaier compensation.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Math.sumPrecise")?;
                        let [array] = arguments.as_slice() else {
                            return Err("`Math.sumPrecise` expects exactly one argument".into());
                        };
                        self.expect_type(
                            &HirType::Array(Box::new(HirType::F64)),
                            array,
                            "Math.sumPrecise argument",
                        )?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_sum_precise".to_string())),
                            vec![array.clone()],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Math" && property.sym == *"random" {
                        if !call.args.is_empty() {
                            return Err("`Math.random` expects no arguments".into());
                        }
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_random".to_string())),
                            Vec::new(),
                        ));
                    }
                    if object.sym == *"Math" && property.sym == *"f16round" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Math.f16round")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Math.f16round` expects exactly one argument".into());
                        };
                        let value = self.coerce_primitive_to_number(value.clone())?;
                        let value = self.coerce_to_declared(&HirType::Json, value)?;
            let args = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                        let math = HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".into())),
                            vec![HirExpr::Lit(HirLit::Str("Math".into()))],
                        );
                        let result = HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("callDynamicMethod".into())),
                            vec![math, HirExpr::Lit(HirLit::Str("f16round".into())), args],
                        )));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Math"
                        && matches!(
                            property.sym.as_ref(),
                            "abs"
                                | "floor"
                                | "ceil"
                                | "trunc"
                                | "sqrt"
                                | "exp"
                                | "log"
                                | "log2"
                                | "log10"
                                | "sin"
                                | "cos"
                                | "tan"
                                | "asin"
                                | "acos"
                                | "atan"
                                | "sinh"
                                | "cosh"
                                | "tanh"
                                | "cbrt"
                                | "acosh"
                                | "asinh"
                                | "atanh"
                                | "expm1"
                                | "log1p"
                                | "fround"
                                | "clz32"
                        )
                    {
                        let label = format!("Math.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!(
                                "`Math.{}` expects exactly one argument",
                                property.sym
                            ));
                        };
                        let value = self.coerce_primitive_to_number(value.clone())?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_math_{}", property.sym))),
                            vec![value],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Math" && matches!(property.sym.as_ref(), "min" | "max") {
                        // `Math.max(...arr, 10)` / `Math.max(...set)`: a runtime-length spread
                        // (anything but an array literal) among other arguments, or a non-array
                        // iterable alone, collects into one array literal first.
                        let runtime_spread = |a: &swc_ecma_ast::ExprOrSpread| {
                            a.spread.is_some() && !matches!(a.expr.as_ref(), Expr::Array(_))
                        };
                        let mut is_set = false;
                        if let [a] = call.args.as_slice() {
                            if runtime_spread(a) {
                                let lowered = self.lower_expr(&a.expr)?;
                                is_set = matches!(self.infer_expr_type(&lowered)?, HirType::Set(_));
                            }
                        }
                        if (call.args.len() > 1 && call.args.iter().any(runtime_spread)) || is_set {
                            let spread = Expr::Array(swc_ecma_ast::ArrayLit {
                                span: swc_common::DUMMY_SP,
                                elems: call.args.iter().cloned().map(Some).collect(),
                            });
                            let mut rewritten = call.clone();
                            rewritten.args = vec![swc_ecma_ast::ExprOrSpread {
                                spread: Some(swc_common::DUMMY_SP),
                                expr: Box::new(spread),
                            }];
                            return self.lower_static_builtin_call(object, property, &rewritten);
                        }
                        if let [argument] = call.args.as_slice() {
                            if argument.spread.is_some() {
                                let source = self.lower_expr(&argument.expr)?;
                                let source = self.project_native_base(source)?;
                                let source = self.bytes_to_f64_array(source)?;
                                // A single spread of a runtime-length
                                // `number[]` (not an array literal or
                                // tuple, both already handled below by
                                // `lower_native_spread_values` unrolling
                                // them into individual arguments at
                                // compile time -- there's no fixed N of
                                // those to unroll here) folds pairwise
                                // through the same `__thaw_math_min`/`_max`
                                // intrinsic the fixed-arity form already
                                // calls, via a runtime loop instead of a
                                // compile-time-unrolled argument list.
                    // Elements that may be `undefined` (sink reads) arrive as `Json`: number-convert them.
                                let source = if matches!(self.infer_expr_type(&source)?, HirType::Json)
                                    || self.infer_expr_type(&source)? == HirType::Array(Box::new(HirType::Json))
                                {
                                    self.coerce_to_declared(&HirType::Array(Box::new(HirType::F64)), source)?
                                } else {
                                    source
                                };
                    if self.infer_expr_type(&source)? == HirType::Array(Box::new(HirType::F64))
                                    && !matches!(source, HirExpr::ArrayLit(_))
                                {
                        return self.lower_math_extreme_of_array(source, property.sym == *"min");
                                }
                            }
                        }
                    }
                    if object.sym == *"Math"
                        && matches!(
                            property.sym.as_ref(),
                            "pow" | "min" | "max" | "sign" | "round" | "atan2" | "hypot" | "imul"
                        )
                    {
                        let label = format!("Math.{}", property.sym);
            let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let expected = match property.sym.as_ref() {
                            "pow" | "atan2" | "imul" => Some(2),
                            "sign" | "round" => Some(1),
                            _ => None,
                        };
                        if expected.is_some_and(|expected| arguments.len() != expected) {
                            return Err(format!(
                                "`Math.{}` expects {} argument(s)",
                                property.sym,
                                expected.unwrap()
                            ));
                        }
                        let arguments = arguments
                            .into_iter()
                            .map(|value| self.coerce_primitive_to_number(value))
                            .collect::<Result<Vec<_>, _>>()?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_math_{}", property.sym))),
                            arguments,
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Number"
                        && matches!(
                            property.sym.as_ref(),
                            "isNaN" | "isFinite" | "isInteger" | "isSafeInteger"
                        )
                    {
                        let label = format!("Number.{}", property.sym);
            let (arguments, mut bindings) = self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!(
                                "`Number.{}` expects exactly one argument",
                                property.sym
                            ));
                        };
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
                        // `Json` (e.g. js-yaml's `.nan`/`.inf` tags,
                        // classified `Json` via `load(): unknown`, see
                        // `[[project_npm_interop_gaps_19]]`) decodes to a
                        // real `f64` through the same `JsonAsNumber`
                        // intrinsic `json_narrowings`' typeof-narrowing
                        // already uses -- `thaw_json_as_number` now
                        // recognizes the `$__thaw_non_finite$` sentinel
                        // `__thaw_json_safe_stringify` emits for a real
                        // `NaN`/`Infinity`, so this reaches the exact same
                        // native predicate the `F64` arm below already
                        // calls, rather than needing its own logic.
                        let value = if ty == HirType::Json {
                            HirExpr::JsonAsNumber(Box::new(value))
                        } else {
                            value
                        };
                        if ty == HirType::F64 || ty == HirType::Json {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    match property.sym.as_ref() {
                                        "isNaN" => "__thaw_number_is_nan",
                                        "isFinite" => "__thaw_number_is_finite",
                                        "isInteger" => "__thaw_number_is_integer",
                                        "isSafeInteger" => "__thaw_number_is_safe_integer",
                                        _ => unreachable!(),
                                    }
                                    .to_string(),
                                )),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if ty == HirType::Optional(Box::new(HirType::F64)) {
                            let name = format!("__thaw_number_predicate_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            let bound = HirExpr::Var(name.clone());
                            let result = HirExpr::Conditional(
                    Box::new(HirExpr::OptionalIsNone(
                        Box::new(bound.clone()),
                        HirType::F64,
                    )),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var(
                            match property.sym.as_ref() {
                                        "isNaN" => "__thaw_number_is_nan",
                                        "isFinite" => "__thaw_number_is_finite",
                                        "isInteger" => "__thaw_number_is_integer",
                                        "isSafeInteger" => "__thaw_number_is_safe_integer",
                                        _ => unreachable!(),
                            }
                            .into(),
                        )),
                                    vec![HirExpr::OptionalValue(Box::new(bound), HirType::F64)],
                                )),
                                HirType::Bool,
                            );
                            bindings.push((name, ty, value));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if let HirType::Nullable(inner) | HirType::Nullish(inner) = &ty {
                            if **inner == HirType::F64 {
                                // `number | null` / `number | null | undefined`: the number
                                // arm runs the predicate, every other arm is `false`.
                                let name = format!("__thaw_number_predicate_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(name.clone(), ty.clone());
                                let bound = HirExpr::Var(name.clone());
                                let (is_none, present) = if matches!(ty, HirType::Nullable(_)) {
                                    (
                                        HirExpr::NullableIsNone(Box::new(bound.clone()), HirType::F64),
                                        HirExpr::NullableValue(Box::new(bound), HirType::F64),
                                    )
                                } else {
                                    (
                                        HirExpr::NullishIsNone(Box::new(bound.clone()), HirType::F64),
                                        HirExpr::NullishValue(Box::new(bound), HirType::F64),
                                    )
                                };
                                let result = HirExpr::Conditional(
                                    Box::new(is_none),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                    Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(
                                match property.sym.as_ref() {
                                            "isNaN" => "__thaw_number_is_nan",
                                            "isFinite" => "__thaw_number_is_finite",
                                            "isInteger" => "__thaw_number_is_integer",
                                            "isSafeInteger" => "__thaw_number_is_safe_integer",
                                            _ => unreachable!(),
                                }
                                .into(),
                            )),
                                        vec![present],
                                    )),
                                    HirType::Bool,
                                );
                                bindings.push((name, ty, value));
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                        }
                        let name = format!("__thaw_number_predicate_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name, ty, value));
            return self.wrap_call_argument_bindings(HirExpr::Lit(HirLit::Bool(false)), &bindings);
                    }
                    if object.sym == *"String" && property.sym == *"raw" {
                        // `String.raw({ raw: [...] }, ...substitutions)`.
                        // Requires an object-literal `raw` array of
                        // string-literal segments (the direct-call form;
                        // the tagged-template form is lowered separately).
                        let [strings, substitutions @ ..] = call.args.as_slice() else {
                            return Err("`String.raw` expects a strings argument".into());
                        };
                        if strings.spread.is_some() {
                            return Err("`String.raw` does not support a spread strings argument".into());
                        }
                        let Expr::Object(object) = strings.expr.as_ref() else {
                            return Err(
                    "`String.raw` currently requires an object-literal strings argument".into(),
                            );
                        };
                        let mut raw = None;
                        for property in &object.props {
                            let swc_ecma_ast::PropOrSpread::Prop(property) = property else {
                                continue;
                            };
                            let swc_ecma_ast::Prop::KeyValue(property) = property.as_ref() else {
                                continue;
                            };
                            let name = match &property.key {
                                swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                                swc_ecma_ast::PropName::Str(value) => {
                                    value.value.to_string_lossy().into_owned()
                                }
                                _ => continue,
                            };
                            if name == "raw" {
                                raw = Some(property.value.as_ref());
                            }
                        }
                        let Some(Expr::Array(array)) = raw else {
                return Err("`String.raw` currently requires a `raw` array literal".into());
                        };
            if substitutions
                .iter()
                .any(|argument| argument.spread.is_some())
            {
                            return Err("`String.raw` does not support a spread substitution".into());
                        }
                        let (substitutions, bindings) =
                            self.lower_native_spread_values(substitutions, "String.raw")?;
                        let mut parts = Vec::with_capacity(array.elems.len() * 2);
                        for (index, element) in array.elems.iter().enumerate() {
                            let Some(element) = element else {
                                return Err("`String.raw` does not support holes".into());
                            };
                            if element.spread.is_some() {
                                return Err("`String.raw` does not support a spread segment".into());
                            }
                            let Expr::Lit(Lit::Str(text)) = element.expr.as_ref() else {
                                return Err(
                        "`String.raw` currently requires string-literal raw segments".into(),
                                );
                            };
                            let text = text.value.to_string_lossy().into_owned();
                            if !text.is_empty() {
                                parts.push(HirExpr::Lit(HirLit::Str(text)));
                            }
                            // A substitution is only inserted *between* raw
                            // segments, so the last segment has none.
                            if index + 1 < array.elems.len() {
                                if let Some(substitution) = substitutions.get(index) {
                                    parts.push(self.coerce_primitive_to_string(substitution.clone())?);
                                }
                            }
                        }
                        let mut parts = parts.into_iter();
                        let Some(mut result) = parts.next() else {
                            return self.wrap_call_argument_bindings(
                                HirExpr::Lit(HirLit::Str(String::new())),
                                &bindings,
                            );
                        };
                        for part in parts {
                            result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                vec![result, part],
                            );
                        }
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"String" && property.sym == *"fromCharCode" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "String.fromCharCode")?;
                        let mut units = Vec::with_capacity(arguments.len());
                        for argument in &arguments {
                            units.push(self.coerce_primitive_to_number(argument.clone())?);
                        }
                        let mut result = HirExpr::Lit(HirLit::Str(String::new()));
                        for unit in units {
                            let unit = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_from_char_code".into())),
                                vec![unit],
                            );
                            result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".into())),
                                vec![result, unit],
                            );
                        }
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"String" && property.sym == *"fromCodePoint" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "String.fromCodePoint")?;
                        let mut points = Vec::with_capacity(arguments.len());
                        for argument in &arguments {
                            points.push(self.coerce_primitive_to_number(argument.clone())?);
                        }
            let result_name = format!("__thaw_from_code_point_result_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(result_name.clone(), HirType::Str);
                        let var = |name: &str| HirExpr::Var(name.into());
                        let mut stmts = vec![HirStmt::Let(
                            result_name.clone(),
                            HirType::Str,
                            HirExpr::Lit(HirLit::Str(String::new())),
                        )];
                        for point in points {
                let raw_name = format!("__thaw_from_code_point_raw_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(raw_name.clone(), HirType::Str);
                            stmts.push(HirStmt::Let(
                                raw_name.clone(),
                                HirType::Str,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_from_code_point".into())),
                                    vec![point],
                                ),
                            ));
                            stmts.push(HirStmt::If(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                    vec![var(&raw_name)],
                                ),
                                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                    "\u{1}RangeError\u{1}Invalid code point".into(),
                                )))],
                                Vec::new(),
                            ));
                            stmts.push(HirStmt::Expr(HirExpr::Assign(
                                result_name.clone(),
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".into())),
                                    vec![var(&result_name), var(&raw_name)],
                                )),
                            )));
                        }
                        stmts.push(HirStmt::Return(Some(var(&result_name))));
                        let body = HirExpr::Block(stmts);
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
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
        unreachable!("static builtin dispatch was checked before lowering")
    }
}

/// Simplified BCP-47 canonicalization: the language subtag is lowercased,
/// a 4-letter script subtag is title-cased, and a 2/3-letter region subtag
/// is uppercased. `Intl.getCanonicalLocales`'s step beyond this (likely
/// subtags, grandfathered tags, extension ordering) is not modeled.
fn canonicalize_locale(value: &str) -> String {
    value
        .replace('_', "-")
        .split('-')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_ascii_lowercase()
            } else if part.len() == 4 {
                let mut characters = part.chars();
                match characters.next() {
                    Some(first) => {
                        first.to_ascii_uppercase().to_string()
                            + &characters.as_str().to_ascii_lowercase()
                    }
                    None => String::new(),
                }
            } else if part.len() == 2 || part.len() == 3 {
                part.to_ascii_uppercase()
            } else {
                part.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// A small representative list for `Intl.supportedValuesOf(key)` (the full
/// ICU data set is not bundled). An unknown key yields an empty array.
fn supported_values_of(key: &str) -> Vec<String> {
    match key {
        "calendar" => vec!["gregory", "iso8601"],
        "collation" => vec!["emoji", "eor"],
        "currency" => vec!["USD", "EUR", "JPY", "GBP"],
        "numberingSystem" => vec!["latn", "arab"],
        "timeZone" => vec!["UTC", "America/New_York", "Europe/London"],
        "unit" => vec!["meter", "second", "kilogram", "celsius"],
        _ => Vec::new(),
    }
    .into_iter()
    .map(str::to_string)
    .collect()
}


/// `Object.fromEntries(map)` -> `Object.fromEntries(Array.from(map).map((e) => [String(e[0]), e[1]]))`
/// arguments, for maps whose keys are not already strings.
fn number_map_entries_arguments(args: &[swc_ecma_ast::ExprOrSpread]) -> Option<Vec<swc_ecma_ast::ExprOrSpread>> {
    use swc_common::DUMMY_SP;
    use swc_ecma_ast::*;
    let [argument] = args else { return None };
    if argument.spread.is_some() {
        return None;
    }
    let ident = |name: &str| Box::new(Expr::Ident(Ident::new_no_ctxt(name.into(), DUMMY_SP)));
    let member = |obj: Box<Expr>, prop: &str| {
        Box::new(Expr::Member(MemberExpr {
            span: DUMMY_SP,
            obj,
            prop: MemberProp::Ident(IdentName::new(prop.into(), DUMMY_SP)),
        }))
    };
    let call = |callee: Box<Expr>, args: Vec<Box<Expr>>| {
        Box::new(Expr::Call(CallExpr {
            span: DUMMY_SP,
            ctxt: Default::default(),
            callee: Callee::Expr(callee),
            args: args.into_iter().map(|expr| ExprOrSpread { spread: None, expr }).collect(),
            type_args: None,
        }))
    };
    let index = |position: f64| {
        Box::new(Expr::Member(MemberExpr {
            span: DUMMY_SP,
            obj: ident("e"),
            prop: MemberProp::Computed(ComputedPropName {
                span: DUMMY_SP,
                expr: Box::new(Expr::Lit(Lit::Num(Number { span: DUMMY_SP, value: position, raw: None }))),
            }),
        }))
    };
    let entry = Box::new(Expr::Array(ArrayLit {
        span: DUMMY_SP,
        elems: vec![
            Some(ExprOrSpread { spread: None, expr: call(ident("String"), vec![index(0.0)]) }),
            Some(ExprOrSpread { spread: None, expr: index(1.0) }),
        ],
    }));
    let arrow = Box::new(Expr::Arrow(ArrowExpr {
        span: DUMMY_SP,
        ctxt: Default::default(),
        params: vec![Pat::Ident(BindingIdent { id: Ident::new_no_ctxt("e".into(), DUMMY_SP), type_ann: None })],
        body: Box::new(ArrowFunctionBody::Expr(entry)),
        is_async: false,
        is_generator: false,
        type_params: None,
        return_type: None,
    }));
    let array = call(member(ident("Array"), "from"), vec![argument.expr.clone()]);
    Some(vec![ExprOrSpread { spread: None, expr: call(member(array, "map"), vec![arrow]) }])
}

/// `Object.fromEntries(xs.map((x) => [k, v]))`: the callback's pair literal is an entry, not a
/// heterogeneous array, so give it the `[string, any]` entry type (`k` keys are property names).
fn contextual_from_entries_arguments(args: &[swc_ecma_ast::ExprOrSpread]) -> Option<Vec<swc_ecma_ast::ExprOrSpread>> {
    use swc_common::DUMMY_SP;
    use swc_ecma_ast::*;
    let [argument] = args else { return None };
    if argument.spread.is_some() {
        return None;
    }
    let Expr::Call(call) = argument.expr.as_ref() else { return None };
    let Callee::Expr(callee) = &call.callee else { return None };
    let Expr::Member(member) = callee.as_ref() else { return None };
    if !matches!(&member.prop, MemberProp::Ident(name) if name.sym == *"map") {
        return None;
    }
    let [callback] = call.args.as_slice() else { return None };
    let Expr::Arrow(arrow) = callback.expr.as_ref() else { return None };
    let ArrowFunctionBody::Expr(body) = arrow.body.as_ref() else { return None };
    let mut inner = body.as_ref();
    while let Expr::Paren(paren) = inner {
        inner = paren.expr.as_ref();
    }
    let Expr::Array(pair) = inner else { return None };
    let [Some(key), Some(value)] = pair.elems.as_slice() else { return None };
    if key.spread.is_some() || value.spread.is_some() {
        return None;
    }
    let keyword = |kind| Box::new(TsType::TsKeywordType(TsKeywordType { span: DUMMY_SP, kind }));
    let tuple_element = |kind| TsTupleElement { span: DUMMY_SP, label: None, ty: keyword(kind) };
    let entry_type = TsType::TsTupleType(TsTupleType {
        span: DUMMY_SP,
        elem_types: vec![
            tuple_element(TsKeywordTypeKind::TsStringKeyword),
            tuple_element(TsKeywordTypeKind::TsAnyKeyword),
        ],
    });
    let string_key = Expr::Call(CallExpr {
        span: DUMMY_SP,
        ctxt: Default::default(),
        callee: Callee::Expr(Box::new(Expr::Ident(Ident::new_no_ctxt("String".into(), DUMMY_SP)))),
        args: vec![ExprOrSpread { spread: None, expr: key.expr.clone() }],
        type_args: None,
    });
    let entry = Expr::Array(ArrayLit {
        span: DUMMY_SP,
        elems: vec![
            Some(ExprOrSpread { spread: None, expr: Box::new(string_key) }),
            Some(value.clone()),
        ],
    });
    let mut rewritten_arrow = arrow.clone();
    *rewritten_arrow.body = ArrowFunctionBody::Expr(Box::new(entry));
    rewritten_arrow.return_type = Some(Box::new(TsTypeAnn { span: DUMMY_SP, type_ann: Box::new(entry_type) }));
    let mut rewritten_call = call.clone();
    rewritten_call.args = vec![ExprOrSpread { spread: None, expr: Box::new(Expr::Arrow(rewritten_arrow)) }];
    Some(vec![ExprOrSpread { spread: None, expr: Box::new(Expr::Call(rewritten_call)) }])
}
