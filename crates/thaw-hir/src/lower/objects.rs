fn callback_signature(ty: &HirType, supplied: usize) -> Option<(Vec<HirType>, HirType)> {
    match ty {
        HirType::Function(params, ret) if supplied == usize::MAX || supplied <= params.len() => {
            Some((params.clone(), ret.as_ref().clone()))
        }
        HirType::CallableFunction(params, optional, rest, ret) => {
            let mut params = params.clone();
            if let Some(rest) = rest {
                params.push(HirType::Array(rest.clone()));
            }
            let required = params.len().saturating_sub(optional.count() as usize);
            if supplied >= required && supplied <= params.len() {
                params.truncate(supplied);
            }
            Some((params, ret.as_ref().clone()))
        }
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            callback_signature(payload, supplied)
        }
        HirType::Union(elements) => elements
            .iter()
            .find_map(|element| callback_signature(element, supplied)),
        _ => None,
    }
}

/// Only a direct static field access can use a prefix view safely. A whole
/// `this` value could enumerate the synthetic marker's physical offset, and
/// a nested closure or member call can let that receiver escape indirectly.
fn method_this_stays_in_prefix(
    function: &swc_ecma_ast::Function,
    fields: &[(Symbol, HirExpr)],
) -> bool {
    struct Inspector<'a> { fields: &'a [(Symbol, HirExpr)], safe: bool }
    impl Visit for Inspector<'_> {
        fn visit_expr(&mut self, expression: &Expr) {
            if matches!(expression, Expr::OptChain(_) | Expr::TaggedTpl(_) | Expr::New(_) | Expr::SuperProp(_)) {
                self.safe = false;
            }
            expression.visit_children_with(self);
        }
        fn visit_member_expr(&mut self, member: &swc_ecma_ast::MemberExpr) {
            if matches!(member.obj.as_ref(), Expr::This(_)) {
                self.safe &= matches!(&member.prop, MemberProp::Ident(name)
                    if self.fields.iter().any(|(field, _)| field.as_str() == name.sym.as_ref()));
                return;
            }
            member.visit_children_with(self);
        }
        fn visit_call_expr(&mut self, call: &swc_ecma_ast::CallExpr) {
            if matches!(&call.callee, swc_ecma_ast::Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Member(member)
                    if matches!(member.obj.as_ref(), Expr::This(_)))) {
                self.safe = false;
            }
            call.visit_children_with(self);
        }
        fn visit_this_expr(&mut self, _: &swc_ecma_ast::ThisExpr) { self.safe = false; }
        fn visit_function(&mut self, _: &swc_ecma_ast::Function) { self.safe = false; }
        fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) { self.safe = false; }
    }
    let mut inspector = Inspector { fields, safe: true };
    for parameter in &function.params { parameter.visit_with(&mut inspector); }
    for decorator in &function.decorators { decorator.visit_with(&mut inspector); }
    if let Some(body) = &function.body { body.visit_with(&mut inspector); }
    inspector.safe
}

impl<'a> FnLowerer<'a> {
    /// Visit the fixed fields present when enumeration began in effective
    /// own-key order. Actions remain lazy so an earlier getter can change a
    /// later field's live descriptor before it is read.
    fn lower_ordered_fixed_field_statements(
        &mut self,
        receiver: HirExpr,
        fields: &[(Symbol, HirType)],
        actions: Vec<(usize, Vec<HirStmt>)>,
    ) -> Result<Vec<HirStmt>, String> {
        if !matches!(receiver, HirExpr::Var(_)) {
            return Err("fixed own-key dispatch requires a bound receiver".into());
        }
        let ordered = ecmascript_field_order(fields);
        if !fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")) {
            // Only compiler markers can change visibility or insertion rank.
            // Even after rest removes a marker, other keys keep static relative
            // order, so ordinary fixed layouts stay on the linear path.
            let mut static_actions = actions.into_iter().map(|(index, body)| {
                let rank = ordered.iter().position(|&candidate| candidate == index)
                    .ok_or_else(|| format!("field {} is not an own enumerable field", fields[index].0))?;
                Ok((rank, body))
            }).collect::<Result<Vec<_>, String>>()?;
            static_actions.sort_by_key(|(rank, _)| *rank);
            return Ok(static_actions.into_iter().flat_map(|(_, body)| body).collect());
        }
        // ponytail: this finite marker-layout path emits quadratic rank
        // comparisons; use a runtime sorted dispatch only if real layouts
        // grow enough for generated-code size to matter.
        let mut ranked = Vec::with_capacity(actions.len());
        let mut statements = Vec::new();
        for (index, body) in actions {
            let static_rank = ordered.iter().position(|&candidate| candidate == index)
                .ok_or_else(|| format!("field {} is not an own enumerable field", fields[index].0))?;
            let static_rank = i64::try_from(static_rank)
                .map_err(|_| "fixed object has too many fields for own-key ordering".to_string())?;
            let field = fields[index].0.clone();
            let rank_name = format!("__thaw_field_rank_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(rank_name.clone(), HirType::I64);
            let key = HirExpr::Lit(HirLit::Str(field.clone()));
            let hidden = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                vec![receiver.clone(), key.clone()],
            );
            let rank = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_order_rank".into())),
                vec![receiver.clone(), key, HirExpr::Lit(HirLit::I64(static_rank))],
            );
            statements.push(HirStmt::Let(rank_name.clone(), HirType::I64,
                HirExpr::Conditional(Box::new(hidden),
                    Box::new(HirExpr::Lit(HirLit::I64(i64::MAX))),
                    Box::new(rank), HirType::I64)));
            ranked.push((rank_name, field, body));
        }
        for _ in 0..ranked.len() {
            let min_name = format!("__thaw_field_min_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(min_name.clone(), HirType::I64);
            statements.push(HirStmt::Let(min_name.clone(), HirType::I64,
                HirExpr::Lit(HirLit::I64(i64::MAX))));
            for (rank_name, _, _) in &ranked {
                statements.push(HirStmt::If(
                    HirExpr::BinOp(BinOp::Lt,
                        Box::new(HirExpr::Var(rank_name.clone())),
                        Box::new(HirExpr::Var(min_name.clone()))),
                    vec![HirStmt::Expr(HirExpr::Assign(min_name.clone(),
                        Box::new(HirExpr::Var(rank_name.clone()))))],
                    Vec::new(),
                ));
            }
            let mut dispatch = Vec::new();
            for (rank_name, field, body) in &ranked {
                let chosen = HirExpr::BinOp(BinOp::EqEqEq,
                    Box::new(HirExpr::Var(rank_name.clone())),
                    Box::new(HirExpr::Var(min_name.clone())));
                let live_hidden = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                    vec![receiver.clone(), HirExpr::Lit(HirLit::Str(field.clone()))],
                );
                let mut on_chosen = vec![HirStmt::If(live_hidden, Vec::new(), body.clone())];
                on_chosen.push(HirStmt::Expr(HirExpr::Assign(rank_name.clone(),
                    Box::new(HirExpr::Lit(HirLit::I64(i64::MAX))))));
                dispatch.push(HirStmt::If(chosen, on_chosen, Vec::new()));
            }
            statements.push(HirStmt::If(
                HirExpr::BinOp(BinOp::EqEqEq,
                    Box::new(HirExpr::Var(min_name)),
                    Box::new(HirExpr::Lit(HirLit::I64(i64::MAX)))),
                Vec::new(), dispatch,
            ));
        }
        Ok(statements)
    }

    /// Copy selected own fields into a fresh fixed layout without treating
    /// every zeroed physical slot as a property. Used by all fixed rest paths.
    fn lower_fixed_object_rest_copy(
        &mut self,
        source: HirExpr,
        fields: &[(Symbol, HirType)],
        omitted: &[Symbol],
    ) -> Result<(HirExpr, HirType), String> {
        let included = ecmascript_field_order(fields).into_iter()
            .filter(|&index| !omitted.contains(&fields[index].0))
            .collect::<Vec<_>>();
        let result_fields = included.iter().map(|&index| {
            let name = fields[index].0.clone();
            Ok((name.clone(), Self::fixed_object_property_read_type(fields, &name)?))
        }).collect::<Result<Vec<_>, String>>()?;
        let result_type = HirType::Object(result_fields.clone());
        let source_type = HirType::Object(fields.to_vec());
        let source_name = format!("__thaw_rest_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), source_type.clone());
        let result_name = format!("__thaw_rest_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), result_type.clone());
        let receiver = HirExpr::Var(source_name.clone());
        let result = HirExpr::Var(result_name.clone());
        let mut body = vec![HirStmt::Let(result_name, result_type.clone(),
            HirExpr::ObjectAlloc(result_type.clone()))];
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_object_order_begin".into())),
            vec![result.clone()],
        )));
        for (name, _) in &result_fields {
            if name.starts_with("__thaw_class_identity_\u{1e}") {
                body.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_hide_marker".into())),
                    vec![result.clone(), HirExpr::Lit(HirLit::Str(name.clone()))],
                )));
            }
        }
        let mut actions = Vec::new();
        for index in included {
            let name = fields[index].0.clone();
            let read = self.lower_fixed_object_property_read(receiver.clone(), fields, &name)?;
            let field_type = result_fields.iter().find(|(field, _)| field == &name)
                .expect("included field belongs to the result layout").1.clone();
            let read = self.coerce_to_declared(&field_type, read)?;
            let write = HirStmt::Expr(HirExpr::PropAssign(
                Box::new(result.clone()), result_type.clone(), name.clone(), Box::new(read),
            ));
            let seed = HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                vec![result.clone(), HirExpr::Lit(HirLit::Str(name))],
            ));
            actions.push((index, vec![write, seed]));
        }
        body.extend(self.lower_ordered_fixed_field_statements(receiver, fields, actions)?);
        body.push(HirStmt::Return(Some(result)));
        let value = self.wrap_call_argument_bindings(
            HirExpr::Block(body), &[(source_name, source_type, source)],
        )?;
        Ok((value, result_type))
    }

    fn static_object_property_name(&self, property: &PropName) -> Option<String> {
        literal_property_name(property).or_else(|| match property {
            PropName::Computed(computed) => well_known_symbol_from_expr(&computed.expr)
                .map(well_known_symbol_key)
                .or_else(|| self.static_property_name(&computed.expr)),
            _ => None,
        })
    }

    fn lower_object_method(
        &mut self,
        method: &swc_ecma_ast::MethodProp,
        receiver: HirType,
        expected: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        self.lower_object_function(&method.function, receiver, expected, false)
    }

    // Accessors keep a visible typed placeholder plus hidden closures used by
    // ordinary property reads and writes.
    fn lower_object_callable_property(
        &mut self,
        property: &Prop,
        fields: &[(Symbol, HirExpr)],
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<Option<Vec<(Symbol, HirExpr)>>, String> {
        let receiver = |prefix_only: bool| -> Result<HirType, String> {
            if let Some(expected) = expected_fields {
                if !expected
                    .iter()
                    .any(|(name, _)| is_hidden_accessor_field(name))
                {
                    let mut receiver = expected.to_vec();
                    receiver.push(("__thaw_object_method_receiver".into(),
                        if prefix_only { HirType::Bool } else { HirType::Undefined }));
                    return Ok(HirType::Object(receiver));
                }
            }
            Ok(HirType::Object(
                fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.infer_expr_type(value)?)))
                    .chain(std::iter::once(Ok((
                        "__thaw_object_method_receiver".into(),
                        if prefix_only { HirType::Bool } else { HirType::Undefined },
                    ))))
                    .collect::<Result<Vec<_>, String>>()?,
            ))
        };
        Ok(match property {
            Prop::Method(method) => {
                let name = self
                    .static_object_property_name(&method.key)
                    .ok_or_else(|| "object method name must be static".to_string())?;
                let expected = expected_fields.and_then(|fields| {
                    fields
                        .iter()
                        .find_map(|(field, ty)| (field == &name).then_some(ty))
                });
                Some(vec![(
                    name,
                    self.lower_object_method(method,
                        receiver(method_this_stays_in_prefix(&method.function, fields))?, expected)?,
                )])
            }
            Prop::Getter(getter) => {
                let name = self
                    .static_object_property_name(&getter.key)
                    .ok_or_else(|| "object getter name must be static".to_string())?;
                let value =
                    self.lower_object_function(&getter.function, receiver(false)?, None, true)?;
                let return_type = match self.infer_expr_type(&value)? {
                    HirType::Function(_, ret) => *ret,
                    HirType::CallableFunction(_, _, _, ret) => *ret,
                    other => return Err(format!("object getter has non-function type {other:?}")),
                };
                Some(vec![
                    (name.clone(), Self::unreachable_value(&return_type)?),
                    (format!("__thaw_getter_{name}"), value),
                ])
            }
            Prop::Setter(setter) => {
                let name = self
                    .static_object_property_name(&setter.key)
                    .ok_or_else(|| "object setter name must be static".to_string())?;
                let value =
                    self.lower_object_function(&setter.function, receiver(false)?, None, true)?;
                let parameter_type = setter
                    .function
                    .params
                    .first()
                    .ok_or_else(|| "object setter requires exactly one parameter".to_string())
                    .and_then(|parameter| {
                        lower_param(
                            &parameter.pat,
                            self.interfaces,
                            self.generic_interfaces,
                            false,
                            &HashMap::new(),
                        )
                        .map(|parameter| parameter.ty)
                    })?;
                let mut additions = vec![(format!("__thaw_setter_{name}"), value)];
                if !fields.iter().any(|(field, _)| field == &name) {
                    additions.insert(0, (name, Self::unreachable_value(&parameter_type)?));
                }
                Some(additions)
            }
            _ => None,
        })
    }

    /// Lowers an object-literal method/getter/setter body into a closure
    /// value. When the body uses `this` (or `force_receiver`, for an
    /// accessor whose read/write site always passes the receiver), the
    /// receiver becomes the closure's leading parameter -- the same
    /// convention the ordinary member-call path already passes a method's
    /// receiver by, so accessors reuse it unchanged.
    fn lower_object_function(
        &mut self,
        function: &swc_ecma_ast::Function,
        receiver: HirType,
        expected: Option<&HirType>,
        force_receiver: bool,
    ) -> Result<HirExpr, String> {
        struct ReplaceThis<'a>(&'a str);
        impl VisitMut for ReplaceThis<'_> {
            fn visit_mut_expr(&mut self, expression: &mut Expr) {
                if matches!(expression, Expr::This(_)) {
                    *expression = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                        self.0.into(),
                        swc_common::DUMMY_SP,
                    ));
                } else {
                    expression.visit_mut_children_with(self);
                }
            }
        }

        let expression = swc_ecma_ast::FnExpr {
            ident: None,
            function: Box::new(function.clone()),
        };
        let mut arrow = function_expression_as_arrow(&expression)?;
        let uses_this = force_receiver || function_uses_this(function);
        let contextual = expected.and_then(|ty| callback_signature(ty, function.params.len()));
        let mut params = Vec::new();
        if uses_this {
            let receiver_name = format!("__thaw_object_this_{}", self.next_binding);
            self.next_binding += 1;
            arrow.body.visit_mut_with(&mut ReplaceThis(&receiver_name));
            arrow.params.insert(
                0,
                Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: swc_ecma_ast::Ident::new_no_ctxt(
                        receiver_name.into(),
                        swc_common::DUMMY_SP,
                    ),
                    type_ann: None,
                }),
            );
            params.push(receiver);
        }
        params.extend(function.params.iter().enumerate().map(|(index, parameter)| {
            lower_param(
                        &parameter.pat,
                        self.interfaces,
                        self.generic_interfaces,
                        false,
                        &HashMap::new(),
                    )
                    .map(|parameter| parameter.ty)
                    .or_else(|error| {
                        contextual
                            .as_ref()
                            .and_then(|(params, _)| params.get(index).cloned())
                            .ok_or(error)
                    })
        }).collect::<Result<Vec<_>, _>>()?);
        let expected_return = function
            .return_type
            .as_ref()
            .map(|annotation| {
                lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )
            })
            .transpose()?
            .or_else(|| contextual.as_ref().map(|(_, ret)| ret.clone()));
        let lowered = self.lower_contextual_arrow(&arrow, &params, expected_return.as_ref())?;
        Ok(rest_aware_closure(&arrow, lowered))
    }

    fn lower_computed_dictionary_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
    ) -> Result<HirExpr, String> {
        let mut properties = Vec::new();
        for (index, property) in obj_lit.props.iter().enumerate() {
            let PropOrSpread::Prop(property) = property else {
                return Err("computed object literals cannot contain spreads yet".into());
            };
            let (key, value) = match property.as_ref() {
                Prop::KeyValue(KeyValueProp { key, value }) => {
                    let key = match self.static_object_property_name(key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(key) = key else {
                                return Err("unsupported computed object literal key".into());
                            };
                            let key = self.lower_expr(&key.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    (key, self.lower_object_lit_field_value(value, None)?)
                }
                Prop::Shorthand(value) => (
                    HirExpr::Lit(HirLit::Str(value.sym.to_string())),
                    self.lower_expr(&Expr::Ident(value.clone()))?,
                ),
                _ => return Err("computed dictionaries support data properties only".into()),
            };
            let value_type = self.infer_expr_type(&value)?;
            properties.push((index, key, value, value_type));
        }
        let first_type = properties
            .first()
            .map(|(_, _, _, ty)| ty.clone())
            .ok_or("computed dictionary cannot be empty")?;
        let element = if properties.iter().all(|(_, _, _, ty)| ty == &first_type) {
            first_type
        } else {
            HirType::Json
        };
        let mut bindings = Vec::new();
        let mut entries = Vec::new();
        for (index, key, value, value_type) in properties {
            let key_name = format!("__thaw_computed_key_{index}_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(key_name.clone(), HirType::Str);
            bindings.push((key_name.clone(), HirType::Str, key));
            let value_name = format!("__thaw_computed_value_{index}_{}", self.next_binding);
            self.next_binding += 1;
            let value = if element == HirType::Json {
                self.coerce_to_declared(&HirType::Json, value)?
            } else {
                value
            };
            let value_type = if element == HirType::Json {
                HirType::Json
            } else {
                value_type
            };
            self.scope.insert(value_name.clone(), value_type.clone());
            bindings.push((value_name.clone(), value_type, value));
            entries.push((key_name, value_name));
        }
        let dictionary = HirType::Dictionary(Box::new(element.clone()));
        let object_name = format!("__thaw_computed_object_{}", self.next_binding);
        self.next_binding += 1;
        let mut body = entries
            .into_iter()
            .map(|(key, value)| {
                HirStmt::Expr(HirExpr::JsonSet(
                    Box::new(HirExpr::Var(object_name.clone())),
                    Box::new(HirExpr::Var(key)),
                    Box::new(HirExpr::Var(value)),
                    element.clone(),
                    false,
                ))
            })
            .collect::<Vec<_>>();
        body.push(HirStmt::Return(Some(HirExpr::Var(object_name.clone()))));
        let captures = bindings
            .iter()
            .map(|(name, ty, _)| HirParam {
                name: name.clone(),
                ty: ty.clone(),
            })
            .collect();
        let result = HirExpr::Call(
            Box::new(HirExpr::Lambda(
                captures,
                vec![HirParam {
                    name: object_name,
                    ty: dictionary.clone(),
                }],
                dictionary,
                Box::new(HirExpr::Block(body)),
            )),
            vec![HirExpr::JsonObjectLit(Vec::new(), element)],
        );
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// An object literal with at least one `...spread` whose source
    /// isn't a compile-time-known-shape `HirType::Object` (a bare `any`/
    /// `Json` value, or a `Dictionary`) -- `lower_object_lit`'s normal
    /// path requires every spread source to have a fixed field list it
    /// can expand at compile time, since it merges spreads directly into
    /// a static `HirExpr::ObjectLit`'s own field list. There's no field
    /// list to expand here, so this builds the object at runtime
    /// instead: start from an empty `Json` object, then walk the
    /// literal's properties *in source order*, `__thaw_json_object_assign`-
    /// merging a spread's keys in wholesale or `JsonSet`-ing a single
    /// key -- the same order real JS evaluates a literal in, so a later
    /// spread/key correctly overwrites an earlier one's value without
    /// moving its position (`JsonSet`/`__thaw_json_object_assign` on an
    /// existing key both update in place, matching `Object.assign`'s own
    /// semantics; only a genuinely new key gets appended).
    fn lower_dynamic_spread_object_lit(&mut self, obj_lit: &SwcObjectLit) -> Result<HirExpr, String> {
        let object_name = format!("__thaw_dynamic_object_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(object_name.clone(), HirType::Json);
        let mut body = Vec::new();
        let mut locals = BTreeSet::new();
        for property in &obj_lit.props {
            match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let source = self.coerce_to_declared(&HirType::Json, source).map_err(
                        |_| format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ),
                    )?;
                    let source_name =
                        format!("__thaw_dynamic_object_spread_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(source_name.clone(), HirType::Json);
                    locals.insert(source_name.clone());
                    body.push(HirStmt::Let(source_name.clone(), HirType::Json, source));
                    body.push(HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                        vec![
                            HirExpr::Var(object_name.clone()),
                            HirExpr::Var(source_name),
                        ],
                    )));
                }
                PropOrSpread::Prop(prop) => {
                    let (key, value) = match prop.as_ref() {
                        Prop::KeyValue(KeyValueProp { key, value }) => {
                            let key = match self.static_object_property_name(key) {
                                Some(key) => HirExpr::Lit(HirLit::Str(key)),
                                None => {
                                    let PropName::Computed(computed) = key else {
                                        return Err("unsupported object literal key".into());
                                    };
                                    let key = self.lower_expr(&computed.expr)?;
                                    self.coerce_primitive_to_string(key)?
                                }
                            };
                            (key, self.lower_object_lit_field_value(value, None)?)
                        }
                        Prop::Shorthand(ident) => (
                            HirExpr::Lit(HirLit::Str(ident.sym.to_string())),
                            self.lower_expr(&Expr::Ident(ident.clone()))?,
                        ),
                        _ => {
                            return Err(
                                "an object literal with a dynamic spread supports data properties only"
                                    .to_string(),
                            )
                        }
                    };
                    let key_name = format!("__thaw_dynamic_object_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(key_name.clone(), HirType::Str);
                    locals.insert(key_name.clone());
                    body.push(HirStmt::Let(key_name.clone(), HirType::Str, key));
                    let value = self.coerce_to_declared(&HirType::Json, value)?;
                    let value_name =
                        format!("__thaw_dynamic_object_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), HirType::Json);
                    locals.insert(value_name.clone());
                    body.push(HirStmt::Let(value_name.clone(), HirType::Json, value));
                    body.push(HirStmt::Expr(HirExpr::JsonSet(
                        Box::new(HirExpr::Var(object_name.clone())),
                        Box::new(HirExpr::Var(key_name)),
                        Box::new(HirExpr::Var(value_name)),
                        HirType::Json,
                        false,
                    )));
                }
            }
        }
        body.push(HirStmt::Return(Some(HirExpr::Var(object_name.clone()))));
        locals.insert(object_name.clone());
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&HirExpr::Block(body.clone()), &mut referenced);
        let captures = referenced
            .into_iter()
            .filter(|name| !locals.contains(name))
            .filter_map(|name| self.scope.get(&name).cloned().map(|ty| HirParam { name, ty }))
            .collect();
        let result = HirExpr::Call(
            Box::new(HirExpr::Lambda(
                captures,
                vec![HirParam {
                    name: object_name,
                    ty: HirType::Json,
                }],
                HirType::Json,
                Box::new(HirExpr::Block(body)),
            )),
            vec![HirExpr::JsonObjectLit(Vec::new(), HirType::Json)],
        );
        Ok(result)
    }

    fn lower_dynamic_accessor_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
        live_json: bool,
    ) -> Result<HirExpr, String> {
        let mut bindings = Vec::new();
        let mut kinds = Vec::new();
        let mut keys = Vec::new();
        let mut values = Vec::new();
        let mut callbacks = Vec::new();
        for property in &obj_lit.props {
            if let PropOrSpread::Spread(spread) = property {
                let value = self.lower_expr(&spread.expr)?;
                let value = self.coerce_to_declared(&HirType::Json, value)?;
                let name = format!("__thaw_dynamic_object_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), HirType::Json);
                bindings.push((name.clone(), HirType::Json, value));
                kinds.push(HirExpr::Lit(HirLit::Str("spread".into())));
                keys.push(HirExpr::Lit(HirLit::Str(String::new())));
                values.push(HirExpr::Var(name));
                continue;
            }
            let PropOrSpread::Prop(property) = property else {
                unreachable!()
            };
            let (kind, key, value, callback) = match property.as_ref() {
                Prop::KeyValue(KeyValueProp { key, value }) => {
                    let key = match self.static_object_property_name(key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = key else {
                                return Err("unsupported object literal key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    let value = self.lower_object_lit_field_value(value, None)?;
                    let value = self.coerce_to_declared(&HirType::Json, value)?;
                    ("data", key, Some(value), None)
                }
                Prop::Shorthand(ident) => {
                    let value = self.lower_expr(&Expr::Ident(ident.clone()))?;
                    let value = self.coerce_to_declared(&HirType::Json, value)?;
                    (
                        "data",
                        HirExpr::Lit(HirLit::Str(ident.sym.to_string())),
                        Some(value),
                        None,
                    )
                }
                Prop::Method(method) => {
                    let key = match self.static_object_property_name(&method.key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = &method.key else {
                                return Err("unsupported object method key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    let callback = self.lower_object_function(
                        &method.function,
                        HirType::JsValue,
                        None,
                        true,
                    )?;
                    ("method", key, None, Some(callback))
                }
                Prop::Getter(getter) => {
                    let key = match self.static_object_property_name(&getter.key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = &getter.key else {
                                return Err("unsupported object getter key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    let callback = self.lower_object_function(
                        &getter.function,
                        HirType::JsValue,
                        None,
                        true,
                    )?;
                    ("getter", key, None, Some(callback))
                }
                Prop::Setter(setter) => {
                    let key = match self.static_object_property_name(&setter.key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = &setter.key else {
                                return Err("unsupported object setter key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    let callback = self.lower_object_function(
                        &setter.function,
                        HirType::JsValue,
                        None,
                        true,
                    )?;
                    ("setter", key, None, Some(callback))
                }
                _ => return Err("unsupported dynamic object literal property".into()),
            };
            let key_name = format!("__thaw_dynamic_object_key_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(key_name.clone(), HirType::Str);
            bindings.push((key_name.clone(), HirType::Str, key));
            kinds.push(HirExpr::Lit(HirLit::Str(kind.into())));
            keys.push(HirExpr::Var(key_name));
            if let Some(value) = value {
                let value_name = format!("__thaw_dynamic_object_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(value_name.clone(), HirType::Json);
                bindings.push((value_name.clone(), HirType::Json, value));
                values.push(HirExpr::Var(value_name));
            } else {
                values.push(self.coerce_to_declared(
                    &HirType::Json,
                    HirExpr::Lit(HirLit::Null),
                )?);
            }
            if let Some(callback) = callback {
                callbacks.push(HirExpr::Call(
                    Box::new(HirExpr::Var(if live_json {
                        "registerNativeCallbackGraph".into()
                    } else {
                        "registerNativeCallback".into()
                    })),
                    vec![callback],
                ));
            }
        }
        let kinds = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(kinds))?;
        let keys = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(keys))?;
        let values = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
        let arguments = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(vec![kinds, keys, values]),
        )?;
        if live_json {
            // Bind the ordinary property values and encode the metadata before
            // acquiring native callback handles. The consuming call below owns
            // all handles once it starts, including its builder handle.
            let args_name = format!("__thaw_dynamic_object_args_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(args_name.clone(), HirType::Json);
            let builder_name = format!("__thaw_dynamic_object_builder_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(builder_name.clone(), HirType::JsValue);
            let callback_type = HirType::Array(Box::new(HirType::JsValue));
            let callbacks_name = format!("__thaw_dynamic_object_callbacks_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(callbacks_name.clone(), callback_type.clone());
            let mut body = vec![
                HirStmt::Let(args_name.clone(), HirType::Json, arguments),
                HirStmt::Let(
                    callbacks_name.clone(),
                    callback_type,
                    HirExpr::ArrayLit(Vec::new()),
                ),
                HirStmt::Let(
                    builder_name.clone(),
                    HirType::JsValue,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str(
                            "__thaw_object_from_operations".into(),
                        ))],
                    ),
                ),
            ];
            let acquisition = callbacks
                .into_iter()
                .map(|callback| {
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".into())),
                        vec![HirExpr::Var(callbacks_name.clone()), callback],
                    ))
                })
                .collect();
            let failure_name = format!("__thaw_dynamic_object_failure_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(failure_name.clone(), HirType::Str);
            body.push(HirStmt::Try(
                acquisition,
                failure_name.clone(),
                vec![
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_release_native_projection_callbacks".into())),
                        vec![HirExpr::Var(callbacks_name.clone())],
                    )),
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("releaseDynamicValue".into())),
                        vec![HirExpr::Var(builder_name.clone())],
                    )),
                    HirStmt::Throw(HirExpr::Var(failure_name)),
                ],
                None,
            ));
            let built = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_build_native_object_wrapper".into())),
                vec![
                    HirExpr::Var(builder_name),
                    HirExpr::Var(args_name),
                    HirExpr::Var(callbacks_name),
                ],
            );
            body.push(HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_host_from_dynamic".into())),
                vec![built],
            ))));
            self.wrap_call_argument_bindings(HirExpr::Block(body), &bindings)
        } else {
            let result = HirExpr::Call(
                Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())),
                vec![
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str(
                            "__thaw_object_from_operations".into(),
                        ))],
                    ),
                    arguments,
                    HirExpr::ArrayLit(callbacks),
                ],
            );
            self.wrap_call_argument_bindings(result, &bindings)
        }
    }

    /// Lowers an object literal's own `key: value` field value -- almost
    /// always just `lower_expr`, with one narrow exception: a method
    /// call whose *receiver* is already known to be `JsValue`-typed
    /// (`infer_member_receiver_type`) gets `Some(&HirType::JsValue)` as
    /// its own expected-type hint, the same one-shot mechanism a `let`/
    /// `const` declaration's explicit annotation already resolves
    /// elsewhere (see `lower_expr_with_expected_type`'s own doc
    /// comment) -- and the same heuristic this session's chained-
    /// method-call fix already established for a method call used as
    /// *another* method call's own receiver ("more of the same object"
    /// for a builder-style chain). Without this, the field defaults to
    /// the plain JSON-decoding behavior no hint gives it, discarding the
    /// real handle -- real example: zod's own `object({ nickname:
    /// string().optional() })`, where `.optional()`'s receiver
    /// (`string()`) is a `JsValue` schema instance and `.optional()`
    /// itself returns *another* one, not plain data -- an object-literal
    /// field is exactly as much a "sink" for this value as a further
    /// chained method call, just spelled differently. Silently
    /// swallowing the real handle here doesn't just misdeclare
    /// anything -- it hands whatever `object`'s own generic type
    /// inference decides on (a `Json` snapshot of a value with no
    /// serializable content, since a schema instance's own state lives
    /// behind methods) into real zod's own shape-validation logic on the
    /// far side, which throws on its own once it doesn't recognize the
    /// field as a real Zod schema.
    fn lower_object_lit_field_value(
        &mut self,
        value: &Expr,
        expected: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if let Some(expected) = expected {
            let supplied = match value {
                Expr::Arrow(arrow) => arrow.params.len(),
                Expr::Fn(function) => function.function.params.len(),
                _ => usize::MAX,
            };
            let contextual = callback_signature(expected, supplied);
            if let Some((params, ret)) = contextual {
                let needs_context = match value {
                    Expr::Arrow(arrow) => arrow.params.iter().any(
                        |param| matches!(param, Pat::Ident(binding) if binding.type_ann.is_none()),
                    ),
                    Expr::Fn(function) => function.function.params.iter().any(|param| {
                        matches!(&param.pat, Pat::Ident(binding) if binding.type_ann.is_none())
                    }),
                    _ => false,
                };
                if needs_context {
                    return self.lower_promise_callback(value, &params, Some(&ret));
                }
            }
            return self.lower_expr_with_expected_type(value, Some(expected));
        }
        let Expr::Call(call) = value else {
            return self.lower_expr(value);
        };
        let Callee::Expr(callee) = &call.callee else {
            return self.lower_expr(value);
        };
        let Expr::Member(member) = callee.as_ref() else {
            return self.lower_expr(value);
        };
        if self.infer_member_receiver_type(&member.obj) == Some(HirType::JsValue) {
            self.lower_expr_with_expected_type(value, Some(&HirType::JsValue))
        } else {
            self.lower_expr(value)
        }
    }

    /// Marker-capable fixed spreads must copy each source's own-key snapshot
    /// before evaluating the following literal property or spread operand.
    fn lower_marker_ordered_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        enum Step {
            Spread(Symbol, HirType, HirExpr),
            Field(Symbol, HirExpr),
        }
        let mut steps = Vec::new();
        let mut fields: Vec<(Symbol, HirExpr)> = Vec::new();
        let mut field_types: Vec<(Symbol, HirType)> = Vec::new();
        for property in &obj_lit.props {
            let additions = match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let HirType::Object(source_fields) = &source_type else {
                        return Err(format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ));
                    };
                    let source_name = format!("__thaw_ordered_spread_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(source_name.clone(), source_type.clone());
                    let additions = ecmascript_field_order(source_fields).into_iter()
                        .map(|index| {
                            let name = &source_fields[index].0;
                            Ok((name.clone(), self.lower_fixed_object_property_read(
                                HirExpr::Var(source_name.clone()), source_fields, name,
                            )?))
                        }).collect::<Result<Vec<_>, String>>()?;
                    steps.push(Step::Spread(source_name, source_type, source));
                    additions
                }
                PropOrSpread::Prop(prop) => {
                    let additions = match prop.as_ref() {
                        Prop::KeyValue(KeyValueProp { key, value }) => {
                            let name = self.static_object_property_name(key)
                                .ok_or_else(|| "unsupported object literal key".to_string())?;
                            let expected = expected_fields.and_then(|fields| fields.iter()
                                .find_map(|(field, ty)| (field == &name).then_some(ty)));
                            vec![(name, self.lower_object_lit_field_value(value, expected)?)]
                        }
                        Prop::Shorthand(ident) => vec![(
                            ident.sym.to_string(), self.lower_expr(&Expr::Ident(ident.clone()))?,
                        )],
                        property => self.lower_object_callable_property(
                            property, &fields, expected_fields,
                        )?.ok_or_else(|| "only data properties are supported in object literals".to_string())?,
                    };
                    if let Some(expected) = expected_fields.filter(|fields| !fields.is_empty()) {
                        if let Some((name, _)) = additions.iter().find(|(name, _)| {
                            !name.starts_with("__thaw_")
                                && !expected.iter().any(|(field, _)| field == name)
                        }) {
                            return Err(format!(
                                "object literal property `{name}` is not present in the declared type"
                            ));
                        }
                    }
                    for (name, value) in &additions {
                        steps.push(Step::Field(name.clone(), value.clone()));
                    }
                    additions
                }
            };
            for (name, value) in additions {
                let ty = self.infer_expr_type(&value)?;
                if !is_hidden_accessor_field(&name)
                    && (matches!(property, PropOrSpread::Spread(_))
                        || matches!(property, PropOrSpread::Prop(prop)
                            if matches!(prop.as_ref(), Prop::KeyValue(_) | Prop::Shorthand(_) | Prop::Method(_))))
                {
                    fields.retain(|(field, _)| {
                        field != &format!("__thaw_getter_{name}")
                            && field != &format!("__thaw_setter_{name}")
                    });
                    field_types.retain(|(field, _)| {
                        field != &format!("__thaw_getter_{name}")
                            && field != &format!("__thaw_setter_{name}")
                    });
                }
                if let Some((_, existing)) = fields.iter_mut().find(|(field, _)| field == &name) {
                    *existing = value.clone();
                } else {
                    fields.push((name.clone(), value.clone()));
                }
                if let Some((_, existing)) = field_types.iter_mut().find(|(field, _)| field == &name) {
                    if name.starts_with("__thaw_class_identity_\u{1e}") && *existing != ty {
                        let mut members = match existing {
                            HirType::Union(members) => members.clone(),
                            previous => vec![previous.clone()],
                        };
                        if !members.contains(&ty) { members.push(ty.clone()); }
                        *existing = HirType::Union(members);
                    } else if !name.starts_with("__thaw_class_identity_\u{1e}") {
                        *existing = ty.clone();
                    }
                } else {
                    field_types.push((name, ty));
                }
            }
        }
        let result_type = HirType::Object(field_types.clone());
        let result_name = format!("__thaw_ordered_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), result_type.clone());
        let result = HirExpr::Var(result_name.clone());
        let mut body = vec![HirStmt::Let(result_name, result_type.clone(),
            HirExpr::ObjectAlloc(result_type.clone()))];
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_object_order_begin".into())), vec![result.clone()],
        )));
        for (name, _) in &field_types {
            if name.starts_with("__thaw_class_identity_\u{1e}") {
                body.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_hide_marker".into())),
                    vec![result.clone(), HirExpr::Lit(HirLit::Str(name.clone()))],
                )));
            }
        }
        // An overwritten value still runs at its original position and
        // creates its key there, but need not fit the final physical slot's
        // type. Marker spreads are conditional, so their earlier values
        // remain live until a definite later writer replaces them.
        let mut last_definite = HashMap::new();
        let mut last_hidden = HashMap::new();
        for (position, step) in steps.iter().enumerate() {
            match step {
                Step::Spread(_, HirType::Object(fields), _) => {
                    for index in ecmascript_field_order(fields) {
                        let name = &fields[index].0;
                        if !name.starts_with("__thaw_class_identity_\u{1e}") {
                            last_definite.insert(name.clone(), position);
                        }
                    }
                }
                Step::Field(name, _) if is_hidden_accessor_field(name) => {
                    last_hidden.insert(name.clone(), position);
                }
                Step::Field(name, _)
                    if !name.starts_with("__thaw_class_identity_\u{1e}") =>
                {
                    last_definite.insert(name.clone(), position);
                }
                _ => {}
            }
        }
        let mut deferred_accessors = Vec::new();
        for (position, step) in steps.into_iter().enumerate() {
            match step {
                Step::Spread(name, ty, source) => {
                    let HirType::Object(source_fields) = &ty else { unreachable!() };
                    body.push(HirStmt::Let(name.clone(), ty.clone(), source));
                    let receiver = HirExpr::Var(name);
                    let mut actions = Vec::new();
                    for index in ecmascript_field_order(source_fields) {
                        let field = source_fields[index].0.clone();
                        let read = self.lower_fixed_object_property_read(
                            receiver.clone(), source_fields, &field,
                        )?;
                        let write = if last_definite.get(&field).is_some_and(|&last| last != position) {
                            HirStmt::Expr(read)
                        } else {
                            let target = field_types.iter().find(|(name, _)| name == &field)
                                .expect("spread field belongs to the result").1.clone();
                            let read = self.coerce_to_declared(&target, read)?;
                            HirStmt::Expr(HirExpr::PropAssign(
                                Box::new(result.clone()), result_type.clone(), field.clone(),
                                Box::new(read),
                            ))
                        };
                        let seed = HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                            vec![result.clone(), HirExpr::Lit(HirLit::Str(field))],
                        ));
                        actions.push((index, vec![write, seed]));
                    }
                    body.extend(self.lower_ordered_fixed_field_statements(
                        receiver, source_fields, actions,
                    )?);
                }
                Step::Field(name, value) => {
                    if is_hidden_accessor_field(&name) {
                        if last_hidden.get(&name) != Some(&position)
                            || !field_types.iter().any(|(field, _)| field == &name)
                        {
                            body.push(HirStmt::Expr(value));
                        } else {
                            let field_type = field_types.iter().find(|(field, _)| field == &name)
                                .expect("retained accessor field has a type").1.clone();
                            let temporary = format!("__thaw_deferred_accessor_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(temporary.clone(), field_type.clone());
                            body.push(HirStmt::Let(temporary.clone(), field_type, value));
                            deferred_accessors.push((name, temporary));
                        }
                        continue;
                    }
                    if last_definite.get(&name).is_some_and(|&last| last != position) {
                        body.push(HirStmt::Expr(value));
                        body.push(HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                            vec![result.clone(), HirExpr::Lit(HirLit::Str(name))],
                        )));
                        continue;
                    }
                    let target = field_types.iter().find(|(field, _)| field == &name)
                        .expect("literal field belongs to the result").1.clone();
                    let value = self.coerce_to_declared(&target, value)?;
                    body.push(HirStmt::Expr(HirExpr::PropAssign(
                        Box::new(result.clone()), result_type.clone(), name.clone(),
                        Box::new(value),
                    )));
                    if !is_hidden_accessor_field(&name) {
                        body.push(HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                            vec![result.clone(), HirExpr::Lit(HirLit::Str(name))],
                        )));
                    }
                }
            }
        }
        // Defining a literal accessor does not invoke it. Install the
        // already-created closures after data creation so an earlier setter
        // cannot intercept a later own data property definition.
        for (name, temporary) in deferred_accessors {
            body.push(HirStmt::Expr(HirExpr::PropAssign(
                Box::new(result.clone()), result_type.clone(), name,
                Box::new(HirExpr::Var(temporary)),
            )));
        }
        body.push(HirStmt::Return(Some(result)));
        let block = HirExpr::Block(body);
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&block, &mut referenced);
        let captures = referenced.into_iter()
            .filter_map(|name| self.scope.get(&name).cloned()
                .map(|ty| HirParam { name, ty }))
            .collect();
        let suspends = contains_await(&block);
        let lambda_return = if suspends {
            HirType::Promise(Box::new(result_type.clone()))
        } else {
            result_type.clone()
        };
        let call = HirExpr::Call(Box::new(HirExpr::Lambda(
            captures, Vec::new(), lambda_return, Box::new(block),
        )), Vec::new());
        Ok(if suspends {
            HirExpr::AwaitPromise(Box::new(call), result_type)
        } else {
            call
        })
    }

    fn lower_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        let has_callable = obj_lit.props.iter().any(|property| {
            matches!(property, PropOrSpread::Prop(property)
                if matches!(property.as_ref(), Prop::Method(_) | Prop::Getter(_) | Prop::Setter(_)))
        });
        let has_dynamic_computed_key = expected_fields.is_none()
            && obj_lit.props.iter().any(|property| {
                match property {
                    PropOrSpread::Prop(property) => match property.as_ref() {
                        Prop::KeyValue(KeyValueProp {
                            key: PropName::Computed(computed),
                            ..
                        }) => self.static_property_name(&computed.expr).is_none(),
                        Prop::Method(property) => {
                            self.static_object_property_name(&property.key).is_none()
                        }
                        Prop::Getter(property) => {
                            self.static_object_property_name(&property.key).is_none()
                        }
                        Prop::Setter(property) => {
                            self.static_object_property_name(&property.key).is_none()
                        }
                        _ => false,
                    },
                    _ => false,
                }
            });
        if has_dynamic_computed_key {
            if has_callable {
                return self.lower_dynamic_accessor_object_lit(obj_lit, false);
            }
            return if obj_lit
                .props
                .iter()
                .any(|property| matches!(property, PropOrSpread::Spread(_)))
            {
                self.lower_dynamic_spread_object_lit(obj_lit)
            } else {
                self.lower_computed_dictionary_lit(obj_lit)
            };
        }
        let mut marker_spread = false;
        if expected_fields.is_none() {
            let mut dynamic_spread = None;
            for property in &obj_lit.props {
                if let PropOrSpread::Spread(spread) = property {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    match source_type {
                        HirType::Object(fields) => {
                            marker_spread |= fields.iter().any(|(name, _)|
                                name.starts_with("__thaw_class_identity_\u{1e}"));
                        }
                        _ => { dynamic_spread = Some(()); break; }
                    }
                }
            }
            if dynamic_spread.is_some() {
                return if has_callable {
                    self.lower_dynamic_accessor_object_lit(obj_lit, false)
                } else {
                    self.lower_dynamic_spread_object_lit(obj_lit)
                };
            }
        } else {
            for property in &obj_lit.props {
                if let PropOrSpread::Spread(spread) = property {
                    let source = self.lower_expr(&spread.expr)?;
                    if let HirType::Object(fields) = self.infer_expr_type(&source)? {
                        marker_spread |= fields.iter().any(|(name, _)|
                            name.starts_with("__thaw_class_identity_\u{1e}"));
                    }
                }
            }
        }
        if marker_spread {
            return self.lower_marker_ordered_object_lit(obj_lit, expected_fields);
        }
        struct AwaitFinder(bool);
        impl Visit for AwaitFinder {
            fn visit_await_expr(&mut self, _: &AwaitExpr) {
                self.0 = true;
            }

            fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}

            fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) {}
        }
        let mut finder = AwaitFinder(false);
        obj_lit.visit_with(&mut finder);
        if finder.0 {
            return self.lower_ordered_await_object_lit(obj_lit, expected_fields);
        }
        let mut fields: Vec<(Symbol, HirExpr)> = Vec::new();
        let mut property_bindings: Vec<LoweredBinding> = Vec::new();
        for property in &obj_lit.props {
            let additions = match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let HirType::Object(source_fields) = &source_type else {
                        return Err(format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ));
                    };
                    if matches!(source, HirExpr::ObjectLit(_))
                        && !source_fields
                            .iter()
                            .any(|(name, _)| is_hidden_accessor_field(name))
                    {
                        let HirExpr::ObjectLit(source_values) = &source else {
                            unreachable!()
                        };
                        source_values.clone()
                    } else if matches!(spread.expr.as_ref(), Expr::Ident(_)) {
                        ecmascript_field_order(source_fields)
                            .into_iter()
                            .map(|index| {
                                let (name, _) = &source_fields[index];
                                (
                                    name.clone(),
                                    self.lower_fixed_object_property_read(
                                        source.clone(),
                                        source_fields,
                                        name,
                                    )
                                    .expect("spread field was taken from its source type"),
                                )
                            })
                            .collect::<Vec<_>>()
                    } else {
                        let temporary = format!("__thaw_object_spread_{}", self.next_binding);
                        self.next_binding += 1;
                        let additions = ecmascript_field_order(source_fields)
                            .into_iter()
                            .map(|index| {
                                let (name, _) = &source_fields[index];
                                (
                                    name.clone(),
                                    self.lower_fixed_object_property_read(
                                        HirExpr::Var(temporary.clone()),
                                        source_fields,
                                        name,
                                    )
                                    .expect("spread field was taken from its source type"),
                                )
                            })
                            .collect();
                        self.scope.insert(temporary.clone(), source_type.clone());
                        property_bindings.push((temporary, source_type, source));
                        additions
                    }
                }
                PropOrSpread::Prop(prop) => match prop.as_ref() {
                    Prop::KeyValue(KeyValueProp { key, value }) => {
                        let name = self
                            .static_object_property_name(key)
                            .ok_or_else(|| "unsupported object literal key".to_string())?;
                        let expected = expected_fields.and_then(|fields| {
                            fields
                                .iter()
                                .find_map(|(field, ty)| (field == &name).then_some(ty))
                        });
                        vec![(name, self.lower_object_lit_field_value(value, expected)?)]
                    }
                    Prop::Shorthand(ident) => vec![(
                        ident.sym.to_string(),
                        self.lower_expr(&Expr::Ident(ident.clone()))?,
                    )],
                    property => self
                        .lower_object_callable_property(property, &fields, expected_fields)?
                        .ok_or_else(|| {
                            "only data properties are supported in object literals".to_string()
                        })?,
                },
            };
            if matches!(property, PropOrSpread::Prop(_)) {
                // An empty `expected` field list isn't "exactly zero
                // fields allowed" -- it's "no specific shape to check
                // against". A literal `{}` type annotation means "any
                // non-null/undefined value" in real TS (not "an object
                // with no properties"), so an object literal assigned to
                // a `{}`-typed variable should accept any fields at all,
                // the same way TS itself doesn't flag excess properties
                // there. Skip the excess-property check entirely when
                // there's no real declared shape to check against.
                if let Some(expected) = expected_fields.filter(|fields| !fields.is_empty()) {
                    if let Some((name, _)) = additions.iter().find(|(name, _)| {
                        // Accessor closures are internal (`__thaw_getter_`/
                        // `__thaw_setter_`), never declared-type members.
                        !name.starts_with("__thaw_")
                            && !expected.iter().any(|(field, _)| field == name)
                    }) {
                        return Err(format!(
                            "object literal property `{name}` is not present in the declared type"
                        ));
                    }
                }
            }
            for (name, value) in additions {
                let ty = self.infer_expr_type(&value)?;
                let temporary = format!("__thaw_object_field_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(temporary.clone(), ty.clone());
                property_bindings.push((temporary.clone(), ty, value));
                let value = HirExpr::Var(temporary);
                if !is_hidden_accessor_field(&name)
                    && (matches!(property, PropOrSpread::Spread(_))
                        || matches!(property, PropOrSpread::Prop(prop) if matches!(prop.as_ref(), Prop::KeyValue(_) | Prop::Shorthand(_) | Prop::Method(_))))
                {
                    fields.retain(|(field, _)| {
                        field != &format!("__thaw_getter_{name}")
                            && field != &format!("__thaw_setter_{name}")
                    });
                }
                if let Some((_, existing)) =
                    fields.iter_mut().find(|(existing, _)| existing == &name)
                {
                    *existing = value;
                } else {
                    fields.push((name, value));
                }
            }
        }
        let result = HirExpr::ObjectLit(fields);
        self.wrap_call_argument_bindings(result, &property_bindings)
    }

    fn lower_ordered_await_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        let mut fields = Vec::new();
        let mut bindings = Vec::new();
        for (position, property) in obj_lit.props.iter().enumerate() {
            let additions = match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let HirType::Object(source_fields) = &source_type else {
                        return Err(format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ));
                    };
                    let name = format!("__thaw_object_source_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), source_type.clone());
                    bindings.push((name.clone(), source_type.clone(), source));
                    ecmascript_field_order(source_fields)
                        .into_iter()
                        .map(|index| {
                            let (field, _) = &source_fields[index];
                            (
                                field.clone(),
                                self.lower_fixed_object_property_read(
                                    HirExpr::Var(name.clone()),
                                    source_fields,
                                    field,
                                )
                                .expect("spread field was taken from its source type"),
                            )
                        })
                        .collect::<Vec<_>>()
                }
                PropOrSpread::Prop(prop) => match prop.as_ref() {
                    property @ (Prop::Method(_) | Prop::Getter(_) | Prop::Setter(_)) => self
                        .lower_object_callable_property(property, &fields, expected_fields)?
                        .expect("callable object property handled above"),
                    property => {
                        let (field, source) = match property {
                        Prop::KeyValue(KeyValueProp { key, value }) => {
                            let field = self
                                .static_object_property_name(key)
                                .ok_or_else(|| "unsupported object literal key".to_string())?;
                            let expected = expected_fields.and_then(|fields| {
                                fields
                                    .iter()
                                    .find_map(|(name, ty)| (name == &field).then_some(ty))
                            });
                            (
                                field,
                                self.lower_object_lit_field_value(value, expected)?,
                            )
                        }
                        Prop::Shorthand(ident) => (
                            ident.sym.to_string(),
                            self.lower_expr(&Expr::Ident(ident.clone()))?,
                        ),
                        _ => {
                            return Err(
                                "only data properties are supported in object literals".into()
                            )
                        }
                        };
                        let ty = self.infer_expr_type(&source)?;
                        let name =
                            format!("__thaw_object_value_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name.clone(), ty, source));
                        vec![(field, HirExpr::Var(name))]
                    }
                },
            };
            if matches!(property, PropOrSpread::Prop(_)) {
                // See the sibling check above (object literal lowering's
                // other arm) for why an empty `expected` list skips this
                // entirely instead of rejecting every field.
                if let Some(expected) = expected_fields.filter(|fields| !fields.is_empty()) {
                    if let Some((name, _)) = additions.iter().find(|(name, _)| {
                        !name.starts_with("__thaw_")
                            && !expected.iter().any(|(field, _)| field == name)
                    }) {
                        return Err(format!(
                            "object literal property `{name}` is not present in the declared type"
                        ));
                    }
                }
            }
            for (name, value) in additions {
                if !is_hidden_accessor_field(&name)
                    && (matches!(property, PropOrSpread::Spread(_))
                        || matches!(property, PropOrSpread::Prop(prop) if matches!(prop.as_ref(), Prop::KeyValue(_) | Prop::Shorthand(_) | Prop::Method(_))))
                {
                    fields.retain(|field| {
                        field.0 != format!("__thaw_getter_{name}")
                            && field.0 != format!("__thaw_setter_{name}")
                    });
                }
                if let Some((_, existing)) =
                    fields.iter_mut().find(|(existing, _)| existing == &name)
                {
                    *existing = value;
                } else {
                    fields.push((name, value));
                }
            }
        }
        self.wrap_call_argument_bindings(HirExpr::ObjectLit(fields), &bindings)
    }

    fn unreachable_value(ty: &HirType) -> Result<HirExpr, String> {
        match ty {
            HirType::F64 => Ok(HirExpr::Lit(HirLit::F64(0.0))),
            HirType::Bool => Ok(HirExpr::Lit(HirLit::Bool(false))),
            HirType::Str => Ok(HirExpr::Lit(HirLit::Str(String::new()))),
            HirType::Undefined | HirType::Void => Ok(HirExpr::Lit(HirLit::Undefined)),
            HirType::Null => Ok(HirExpr::Lit(HirLit::Null)),
            HirType::Object(_) => Ok(HirExpr::ObjectAlloc(ty.clone())),
            HirType::Array(element) => Ok(HirExpr::ArrayAlloc(
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                element.as_ref().clone(),
            )),
            // Never actually read (this whole branch only exists to
            // type-check a signature that's structurally unreachable at
            // runtime, e.g. an instance method's declared return type
            // when compiled for a context with no real `this` receiver)
            // -- an empty `Json`/`Dictionary` object is as good a
            // placeholder as `Object`'s own `ObjectAlloc` above.
            //
            // `HirExpr::JsonObjectLit(_, element)`'s own `infer_expr_type`
            // *always* wraps its second argument in another `Dictionary`
            // (`inference/types.rs`) -- so for `Dictionary(element)` this
            // must pass the *inner* `element`, not `ty` itself (which is
            // already `Dictionary(element)`), or the placeholder's own
            // inferred type comes out double-wrapped
            // (`Dictionary(Dictionary(element))`) instead of matching
            // `ty`. Caught by a real, if narrow, repro: a class with both
            // a `Record<string, number>`-typed field *and* a method
            // returning that same type -- unrelated code (a constructor
            // call's own `Record<string, number>` argument, coerced via
            // `coerce_to_declared`'s stricter `HirType::Dictionary`
            // target branch, which requires an exact type match or a raw
            // `ObjectLit`) started failing ("dictionary value must be an
            // object literal with F64 values") once the double-wrapped
            // type from *this* unrelated placeholder leaked into shared
            // class-member type inference. `Json` itself has no such
            // inner type to unwrap to and is unaffected in every case
            // this was tested against (`coerce_to_declared`'s `Json`
            // target branch, unlike `Dictionary`'s, accepts any
            // `json_convertible_native_type` source including
            // `Dictionary(_)` and wraps it) -- left as `HirType::Json`,
            // not chased further since nothing currently exercises the
            // stricter path `Dictionary` just hit.
            HirType::Json => Ok(HirExpr::JsonObjectLit(Vec::new(), HirType::Json)),
            HirType::Dictionary(element) => {
                Ok(HirExpr::JsonObjectLit(Vec::new(), element.as_ref().clone()))
            }
            HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
            HirType::Nullable(payload) => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
            HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
            // A tagged union's placeholder injects the first member's own
            // placeholder at tag 0. Without this, generating an instance
            // method's *unbound* variant (compiled only for a detached
            // `this`, but always lowered) failed for any method that read a
            // union-typed field from `this`.
            HirType::Union(elements) => {
                let first = elements
                    .first()
                    .ok_or("unreachable union has no members")?;
                Ok(HirExpr::UnionInject(
                    Box::new(Self::unreachable_value(first)?),
                    0,
                    elements.clone(),
                ))
            }
            HirType::Function(params, result) => Ok(HirExpr::Lambda(
                Vec::new(),
                params
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| HirParam {
                        name: format!("__thaw_unreachable_parameter_{index}"),
                        ty: ty.clone(),
                    })
                    .collect(),
                result.as_ref().clone(),
                Box::new(Self::unreachable_value(result)?),
            )),
            other => Err(format!(
                "unbound `this` cannot synthesize unreachable value of type {other:?}"
            )),
        }
    }

    fn unbound_this_member_type(&self, property: &str) -> Option<HirType> {
        let class = self.class_context.as_deref()?;
        if self.class_static_context {
            let field = class_static_field_symbol(class, property);
            if let Some(ty) = self.scope.get(&field) {
                return Some(ty.clone());
            }
            let getter = class_getter_symbol(class, property, true);
            if let Some(signature) = self.signatures.get(&getter) {
                return Some(signature.ret.clone());
            }
            let method = class_static_method_symbol(class, property);
            let signature = self.signatures.get(&method)?;
            let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
                HirType::Promise(Box::new(signature.ret.clone()))
            } else {
                signature.ret.clone()
            };
            return Some(HirType::Function(
                signature.params.clone(),
                Box::new(result),
            ));
        }
        let instance = self.interfaces.get(class)?;
        if let HirType::Object(fields) = instance {
            if let Some((_, ty)) = fields.iter().find(|(name, _)| name == property) {
                return Some(ty.clone());
            }
        }
        let getter = class_getter_symbol(class, property, false);
        if let Some(signature) = self.signatures.get(&getter) {
            return Some(signature.ret.clone());
        }
        let method = class_method_symbol(class, property);
        let signature = self.signatures.get(&method)?;
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        Some(HirType::Function(
            signature.params[1..].to_vec(),
            Box::new(result),
        ))
    }

    fn lower_unbound_this_error(&self, property: &str, ty: &HirType) -> Result<HirExpr, String> {
        Ok(HirExpr::ThrowValue(
            Box::new(HirExpr::Lit(HirLit::Str(format!(
                "Cannot read properties of undefined (reading '{property}')"
            )))),
            Box::new(Self::unreachable_value(ty)?),
        ))
    }

    fn unwrap_required_optional_member(
        &mut self,
        obj: HirExpr,
        obj_ty: HirType,
        property: &str,
    ) -> (HirExpr, HirType) {
        let payload = match &obj_ty {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                payload.as_ref().clone()
            }
            _ => return (obj, obj_ty),
        };
        let wrapper_type = obj_ty.clone();
        let name = format!("__thaw_required_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let bound = HirExpr::Var(name.clone());
        let undefined_throw = || {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                "\u{1}TypeError\u{1}Cannot read properties of undefined (reading '{property}')"
            ))))
        };
        let null_throw = || {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                "\u{1}TypeError\u{1}Cannot read properties of null (reading '{property}')"
            ))))
        };
        let (is_none, on_none, value) = match &obj_ty {
            HirType::Optional(_) => (
                HirExpr::OptionalIsNone(Box::new(bound.clone()), payload.clone()),
                vec![undefined_throw()],
                HirExpr::OptionalValue(Box::new(bound.clone()), payload.clone()),
            ),
            HirType::Nullable(_) => (
                HirExpr::NullableIsNone(Box::new(bound.clone()), payload.clone()),
                vec![null_throw()],
                HirExpr::NullableValue(Box::new(bound.clone()), payload.clone()),
            ),
            HirType::Nullish(_) => (
                HirExpr::NullishIsNone(Box::new(bound.clone()), payload.clone()),
                vec![HirStmt::If(
                    HirExpr::NullishIsNull(Box::new(bound.clone()), payload.clone()),
                    vec![null_throw()],
                    vec![undefined_throw()],
                )],
                HirExpr::NullishValue(Box::new(bound.clone()), payload.clone()),
            ),
            _ => unreachable!(),
        };
        (
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name,
                        ty: wrapper_type,
                    }],
                    payload.clone(),
                    Box::new(HirExpr::Block(vec![
                        HirStmt::If(is_none, on_none, Vec::new()),
                        HirStmt::Return(Some(value)),
                    ])),
                )),
                vec![obj],
            ),
            payload,
        )
    }

    fn lower_unbound_this_member(&self, property: &str) -> Result<HirExpr, String> {
        let ty = self.unbound_this_member_type(property).ok_or_else(|| {
            format!(
                "class `{}` has no native member `{property}`",
                self.class_context.as_deref().unwrap_or("<unknown>")
            )
        })?;
        self.lower_unbound_this_error(property, &ty)
    }

    fn dictionary_value_from_json(value: HirExpr, element: &HirType) -> Result<HirExpr, String> {
        match element {
            HirType::F64 => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::Str => Ok(HirExpr::JsonAsString(Box::new(value))),
            HirType::Bool => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::Json => Ok(value),
            HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_) => {
                Ok(HirExpr::JsonAsNative(Box::new(value), element.clone()))
            }
            other => Err(format!(
                "dictionary reads do not yet support value type {other:?}"
            )),
        }
    }

    pub(super) fn typed_dictionary_read(
        &mut self,
        object: HirExpr,
        key: HirExpr,
        element: &HirType,
    ) -> Result<HirExpr, String> {
        let object_name = format!("__thaw_dictionary_object_{}", self.next_binding);
        self.next_binding += 1;
        let key_name = format!("__thaw_dictionary_key_{}", self.next_binding);
        self.next_binding += 1;
        let object_type = HirType::Dictionary(Box::new(element.clone()));
        let raw = HirExpr::JsonKey(
            Box::new(HirExpr::Var(object_name.clone())),
            Box::new(HirExpr::Var(key_name.clone())),
        );
        let has_own = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_has_own".into())),
            vec![
                HirExpr::Var(object_name.clone()),
                HirExpr::Var(key_name.clone()),
            ],
        );
        let body = match element {
            HirType::Optional(payload) => HirExpr::Block(vec![HirStmt::If(
                has_own,
                vec![HirStmt::Return(Some(HirExpr::OptionalSome(
                    Box::new(Self::dictionary_value_from_json(raw, payload)?),
                    payload.as_ref().clone(),
                )))],
                vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                    payload.as_ref().clone(),
                )))],
            )]),
            HirType::Nullable(payload) => {
                Self::lower_nullable_dictionary_value(raw, payload, false)?
            }
            HirType::Nullish(payload) => {
                let present = Self::lower_nullable_dictionary_value(raw, payload, true)?;
                HirExpr::Block(vec![HirStmt::If(
                    has_own,
                    vec![HirStmt::Return(Some(present))],
                    vec![HirStmt::Return(Some(HirExpr::NullishUndefined(
                        payload.as_ref().clone(),
                    )))],
                )])
            }
            _ => HirExpr::Block(vec![HirStmt::Return(Some(
                Self::dictionary_value_from_json(raw, element)?,
            ))]),
        };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                vec![
                    HirParam {
                        name: object_name,
                        ty: object_type,
                    },
                    HirParam {
                        name: key_name,
                        ty: HirType::Str,
                    },
                ],
                element.clone(),
                Box::new(body),
            )),
            vec![object, key],
        ))
    }

    fn lower_nullable_dictionary_value(
        raw: HirExpr,
        payload: &HirType,
        nullish: bool,
    ) -> Result<HirExpr, String> {
        let raw_name = "__thaw_dictionary_raw_value".to_string();
        let is_null = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_is_null".into())),
            vec![HirExpr::Var(raw_name.clone())],
        );
        // For a `Nullable` dictionary value (no separate `undefined`
        // state -- unlike `Nullish` below), a genuinely-missing key
        // must also map to "none". `typed_dictionary_read`'s `Nullable`
        // arm calls this unconditionally, with no surrounding `has_own`
        // check the way the `Nullish` arm has -- so `raw` (a `JsonKey`
        // read) is now the `$__thaw_napi_undefined$` sentinel, not bare
        // `null`, whenever the key was never actually present at all
        // (`thaw_json_get`, thaw-std). A strict `is_null`-only check
        // would treat that sentinel as "present" and decode it as a real
        // payload value -- garbage (e.g. `0` for a numeric payload)
        // instead of the closest available state, "null". The `Nullish`
        // case, by contrast, is only ever reached after its own caller
        // already confirmed `has_own` -- so a stored sentinel there is a
        // rarer, pre-existing, genuinely-present marshaled-`undefined`
        // value, not a missing key; deliberately left as `is_null`-only,
        // unaffected either way by this fix.
        let is_none = if nullish {
            is_null
        } else {
            let is_undefined = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                vec![HirExpr::Var(raw_name.clone())],
            );
            HirExpr::Conditional(
                Box::new(is_null),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                Box::new(is_undefined),
                HirType::Bool,
            )
        };
        let value = Self::dictionary_value_from_json(HirExpr::Var(raw_name.clone()), payload)?;
        let (none, some) = if nullish {
            (
                HirExpr::NullishNull(payload.clone()),
                HirExpr::NullishSome(Box::new(value), payload.clone()),
            )
        } else {
            (
                HirExpr::NullableNone(payload.clone()),
                HirExpr::NullableSome(Box::new(value), payload.clone()),
            )
        };
        let result_type = if nullish {
            HirType::Nullish(Box::new(payload.clone()))
        } else {
            HirType::Nullable(Box::new(payload.clone()))
        };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                vec![HirParam {
                    name: raw_name,
                    ty: HirType::Json,
                }],
                result_type,
                Box::new(HirExpr::Block(vec![HirStmt::If(
                    is_none,
                    vec![HirStmt::Return(Some(none))],
                    vec![HirStmt::Return(Some(some))],
                )])),
            )),
            vec![raw],
        ))
    }

    /// Lowers the object of a member access, keeping a *dynamic* receiver a
    /// live `JsValue` handle rather than letting it decay to a `Json`
    /// snapshot.
    ///
    /// A bound dynamic call already gets this through
    /// `member_receiver_bindings` (`const r = hljs.highlightAuto(code);
    /// r.value.length`), but the inline form has no binding to mark:
    /// `hljs.highlightAuto(code).value.length` lowered the call with no
    /// expected type, so it defaulted to the JSON-decoding `callDynamicMethod`
    /// and `.value` became a `JsonGet` on the handle placeholder -- silently
    /// `undefined` instead of the string. `lower_expr_with_expected_type`
    /// only acts on a `Call`/`Await`, so every other receiver shape (a
    /// variable, `this`, `new C()`, a literal) is unaffected.
    fn lower_member_receiver(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        let value = if self.infer_member_receiver_type(expr) == Some(HirType::JsValue) {
            self.lower_expr_with_expected_type(expr, Some(&HirType::JsValue))?
        } else {
            self.lower_expr(expr)?
        };
        let (Expr::Member(member), HirExpr::TypedIndex(array, index, element)) = (expr, &value) else {
            return Ok(value);
        };
        if !matches!(member.prop, MemberProp::Computed(_))
            || !self.expression_may_be_sparse_array(&member.obj)
        {
            return Ok(value);
        }
        let array_type = self.infer_expr_type(array)?;
        if !matches!(array_type, HirType::Array(_)) {
            return Ok(value);
        }
        self.lower_array_index(array.as_ref().clone(), array_type, element.clone(), index.as_ref().clone())
    }

    fn lower_required_member_receiver(&mut self, expr: &Expr, property: &str) -> Result<HirExpr, String> {
        let receiver = self.lower_member_receiver(expr)?;
        let ty = self.infer_expr_type(&receiver)?;
        Ok(self.unwrap_required_optional_member(receiver, ty, property).0)
    }

    fn lower_union_array_index(
        &mut self,
        object: HirExpr,
        elements: &[HirType],
        index: HirExpr,
        sparse: bool,
        conservative: bool,
    ) -> Result<HirExpr, String> {
        let object_name = format!("__thaw_union_index_array_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_union_index_offset_{}", self.next_binding);
        self.next_binding += 1;
        let object_type = HirType::Union(elements.to_vec());
        self.scope.insert(object_name.clone(), object_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);

        let mut reads = Vec::with_capacity(elements.len());
        let mut read_types = Vec::new();
        for (member_index, member) in elements.iter().enumerate() {
            let HirType::Array(element) = member else {
                return Err(format!("cannot index non-array union member {member:?}"));
            };
            let array = HirExpr::UnionValue(
                Box::new(HirExpr::Var(object_name.clone())),
                member_index,
                elements.to_vec(),
            );
            let tagged = matches!(element.as_ref(),
                HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Undefined)
                || matches!(element.as_ref(), HirType::Union(members) if members.contains(&HirType::Undefined));
            let scalar = matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool);
            let read = if sparse && (tagged || (scalar && !conservative)) {
                self.lower_array_index(
                    array,
                    member.clone(),
                    element.as_ref().clone(),
                    HirExpr::Var(index_name.clone()),
                )?
            } else {
                HirExpr::TypedIndex(
                    Box::new(array),
                    Box::new(HirExpr::Var(index_name.clone())),
                    element.as_ref().clone(),
                )
            };
            let read_type = self.infer_expr_type(&read)?;
            if !read_types.contains(&read_type) {
                read_types.push(read_type.clone());
            }
            reads.push((read, read_type));
        }

        let result_type = match read_types.as_slice() {
            [] => return Err("cannot index an empty union".into()),
            [single] => single.clone(),
            types => {
                let mut members = Vec::new();
                for ty in types {
                    Self::flatten_property_union_members(ty, &mut members)?;
                }
                match members.as_slice() {
                    [single] => single.clone(),
                    _ => HirType::Union(members),
                }
            }
        };
        let mut statements = Vec::new();
        for (member_index, (read, read_type)) in reads.into_iter().enumerate() {
            let returns = if read_types.len() == 1 {
                vec![HirStmt::Return(Some(read))]
            } else {
                self.lower_flattened_property_return(read, &read_type, &result_type)?
            };
            if member_index + 1 == elements.len() {
                statements.extend(returns);
            } else {
                statements.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(object_name.clone())),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(member_index as f64))),
                    ),
                    returns,
                    Vec::new(),
                ));
            }
        }
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                vec![
                    HirParam { name: object_name, ty: object_type },
                    HirParam { name: index_name, ty: HirType::F64 },
                ],
                result_type,
                Box::new(HirExpr::Block(statements)),
            )),
            vec![object, index],
        ))
    }

    fn class_instance_accessor_owner(&self, ty: &HirType, property: &str) -> Option<Symbol> {
        self.class_instance_accessor_owner_inner(ty, property, true)
    }

    fn class_super_instance_accessor_owner(&self, ty: &HirType, property: &str) -> Option<Symbol> {
        self.class_instance_accessor_owner_inner(ty, property, false)
    }

    fn class_instance_accessor_owner_inner(
        &self,
        ty: &HirType,
        property: &str,
        respect_own_field: bool,
    ) -> Option<Symbol> {
        let HirType::Object(fields) = ty else { return None };
        // A user object may spell a marker key and still own this ordinary
        // data property. Class accessor layouts have no physical field for it.
        // `super.property` starts at the base prototype, so it ignores that
        // instance field even when it shadows the same accessor normally.
        if respect_own_field && fields.iter().any(|(name, _)| name == property) { return None; }
        let (marker, HirType::Bool) = fields.first()? else { return None };
        let ancestry = marker.strip_prefix("__thaw_class_identity_\u{1e}")?;
        ancestry.split('\u{1f}').find_map(|class| {
            let owns = |symbol: Symbol| self.signatures.get(&symbol)
                .is_some_and(|signature| signature.accessor_owner.as_deref() == Some(class));
            (owns(class_getter_symbol(class, property, false))
                || owns(class_setter_symbol(class, property, false)))
                .then(|| class.to_string())
        })
    }

    fn assert_class_accessor_receiver(&self, receiver: HirExpr, owner: &str) -> HirExpr {
        HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_assert_class_identity".into())),
            vec![receiver, HirExpr::Lit(HirLit::Str(owner.to_string()))],
        )
    }

    fn call_class_instance_setter(
        &mut self,
        owner: &str,
        symbol: Symbol,
        receiver: HirExpr,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_type = self.infer_expr_type(&receiver)?;
        let value_type = self.infer_expr_type(&value)?;
        let receiver_name = format!("__thaw_accessor_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let value_name = format!("__thaw_accessor_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), receiver_type.clone());
        self.scope.insert(value_name.clone(), value_type.clone());
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(symbol)),
            vec![
                self.assert_class_accessor_receiver(HirExpr::Var(receiver_name.clone()), owner),
                HirExpr::Var(value_name.clone()),
            ],
        );
        self.wrap_call_argument_bindings(call, &[
            (receiver_name, receiver_type, receiver),
            (value_name, value_type, value),
        ])
    }

    fn lower_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        if self.unbound_this_context && matches!(member.obj.as_ref(), Expr::This(_)) {
            let property = member_property_name(&member.prop)
                .ok_or("unbound `this` member access requires a statically known property")?;
            return self.lower_unbound_this_member(&property);
        }
        // `Symbol.<wellKnown>` as a value -- lowered to a distinctive
        // sentinel string a computed access/call can dispatch on. These are
        // not registered symbols (`Symbol.for`), so no runtime symbol is
        // created.
        if let Expr::Ident(receiver) = member.obj.as_ref() {
            if receiver.sym == *"Symbol" {
                if let MemberProp::Ident(property) = &member.prop {
                    if well_known_symbol_name(property.sym.as_ref()).is_some() {
                        return Ok(HirExpr::Lit(HirLit::Str(well_known_symbol_key(
                            property.sym.as_ref(),
                        ))));
                    }
                }
            }
        }
        // `obj[Symbol.toStringTag]` -- a compile-time tag per receiver type.
        if let MemberProp::Computed(computed) = &member.prop {
            if well_known_symbol_from_expr(&computed.expr) == Some("toStringTag") {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                // A user class's own `get [Symbol.toStringTag]()` (mangled
                // via `well_known_symbol_key`, `normalize_class` in
                // `classes/normalization.rs`) takes priority over the
                // built-in table below, matching real JS: a class instance
                // with its own tag getter reports *that*, not `undefined`.
                let tag_property = well_known_symbol_key("toStringTag");
                if let Some(owner) = self.class_instance_accessor_owner(&receiver_type, &tag_property) {
                    let symbol = class_getter_symbol(&owner, &tag_property, false);
                    if self.signatures.get(&symbol).is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str())) {
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![
                            self.assert_class_accessor_receiver(receiver, &owner),
                        ]));
                    }
                }
                // Only the built-ins that actually define a
                // `Symbol.toStringTag` getter report one; an Array/plain
                // object has none, so this reads `undefined` (its "Array"
                // tag comes from `Object.prototype.toString`, not here).
                if receiver_type == date_object_type() {
                    let checked = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_date_has_native_identity".into())),
                        vec![receiver],
                    );
                    return Ok(HirExpr::Conditional(
                        Box::new(checked),
                        Box::new(HirExpr::OptionalSome(
                            Box::new(HirExpr::Lit(HirLit::Str("Date".into()))),
                            HirType::Str,
                        )),
                        Box::new(HirExpr::OptionalNone(HirType::Str)),
                        HirType::Optional(Box::new(HirType::Str)),
                    ));
                }
                let tag = if receiver_type == regex_object_type() {
                    Some("RegExp")
                } else {
                    match &receiver_type {
                        HirType::F64 => Some("Number"),
                        HirType::Str => Some("String"),
                        HirType::Bool => Some("Boolean"),
                        HirType::Map(_, _) => Some("Map"),
                        HirType::WeakMap(_, _) => Some("WeakMap"),
                        HirType::Set(_) => Some("Set"),
                        HirType::WeakSet(_) => Some("WeakSet"),
                        HirType::Bytes => Some("Uint8Array"),
                        HirType::Symbol => Some("Symbol"),
                        HirType::Promise(_) => Some("Promise"),
                        HirType::I64 => Some("BigInt"),
                        _ => None,
                    }
                };
                return Ok(match tag {
                    Some(tag) => HirExpr::Lit(HirLit::Str(tag.to_string())),
                    None => HirExpr::Lit(HirLit::Undefined),
                });
            }
        }
        // Temporal property reads (`instant.epochMilliseconds`,
        // `date.year`, ...) computed from the value's timestamp.
        if let MemberProp::Ident(property) = &member.prop {
            if matches!(
                property.sym.as_ref(),
                "epochMilliseconds"
                    | "epochSeconds"
                    | "epochNanoseconds"
                    | "timeZoneId"
                    | "offset"
                    | "hoursInDay"
                    | "calendarId"
                    | "monthCode"
                    | "era"
                    | "eraYear"
                    | "dayOfYear"
                    | "daysInMonth"
                    | "daysInYear"
                    | "monthsInYear"
                    | "inLeapYear"
                    | "sign"
                    | "blank"
                    | "year"
                    | "month"
                    | "day"
                    | "hour"
                    | "minute"
                    | "second"
                    | "millisecond"
                    | "dayOfWeek"
                    | "days"
                    | "hours"
                    | "minutes"
                    | "seconds"
                    | "milliseconds"
                    | "years"
                    | "months"
                    | "weeks"
                    | "microseconds"
                    | "nanoseconds"
            ) {
                // A receiver that's itself a Temporal-returning call
                // (`Temporal.Now.zonedDateTimeISO(tz).timeZoneId`) can't be
                // typed by `peek_type_without_lowering`; lower it (pure) and
                // infer. `lower_temporal_property` returns `None` when the
                // receiver isn't Temporal, so the caller falls through.
                let kind = match self.peek_type_without_lowering(&member.obj) {
                    Some(ty) => Self::temporal_kind(&ty),
                    None => {
                        let receiver = self.lower_expr(&member.obj)?;
                        Self::temporal_kind(&self.infer_expr_type(&receiver)?)
                    }
                };
                if let Some(kind) = kind {
                    if let Some(result) = self.lower_temporal_property(member, property, kind)? {
                        return Ok(result);
                    }
                }
            }
        }
        if let Expr::Ident(enum_name) = member.obj.as_ref() {
            let member_name = match &member.prop {
                MemberProp::Ident(member) => Some(member.sym.to_string()),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(member)) => {
                        Some(member.value.to_string_lossy().into_owned())
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(member_name) = member_name {
                let key = (enum_name.sym.to_string(), member_name.clone());
                if let Some(value) = self.enum_values.get(&key) {
                    return Ok(HirExpr::Lit(value.clone()));
                }
                if self
                    .enum_values
                    .keys()
                    .any(|(candidate, _)| candidate == enum_name.sym.as_str())
                {
                    return Err(format!(
                        "enum `{}` has no member `{member_name}`",
                        enum_name.sym
                    ));
                }
            }
            if let MemberProp::Computed(computed) = &member.prop {
                if let Some(entries) = self.enum_reverse_values.get(enum_name.sym.as_str()) {
                    let index = self.lower_expr(&computed.expr)?;
                    self.expect_type(&HirType::F64, &index, "numeric enum reverse lookup")?;
                    return Ok(HirExpr::EnumReverseLookup(Box::new(index), entries.clone()));
                }
                if self
                    .enum_values
                    .keys()
                    .any(|(candidate, _)| candidate == enum_name.sym.as_str())
                {
                    return Err(format!(
                        "string enum `{}` does not support numeric reverse lookup",
                        enum_name.sym
                    ));
                }
            }
        }
        if let Some(property) = member_property_name(&member.prop) {
            if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                let class = self
                    .class_context
                    .as_deref()
                    .expect("static class lowering retains its class context");
                let field_symbol = class_static_field_symbol(class, &property);
                if self.scope.contains_key(&field_symbol) {
                    return Ok(HirExpr::Var(field_symbol));
                }
                let getter = class_getter_symbol(class, &property, true);
                if self.signatures.contains_key(&getter) {
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new()));
                }
            }
            if let Expr::Ident(receiver) = member.obj.as_ref() {
                let field_symbol = class_static_field_symbol(receiver.sym.as_ref(), &property);
                if self.scope.contains_key(&field_symbol) {
                    return Ok(HirExpr::Var(field_symbol));
                }
                let static_symbol = class_getter_symbol(receiver.sym.as_ref(), &property, true);
                if self.signatures.contains_key(&static_symbol) {
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(static_symbol)),
                        Vec::new(),
                    ));
                }
                let binding = self.resolve_binding(receiver.sym.as_ref());
                if let Some(receiver_type) = self.scope.get(&binding).cloned() {
                    if let Some(owner) = self.class_instance_accessor_owner(&receiver_type, &property) {
                        let symbol = class_getter_symbol(&owner, &property, false);
                        if self.signatures.get(&symbol).is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str())) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![self.assert_class_accessor_receiver(HirExpr::Var(binding), &owner)],
                            ));
                        }
                    }
                }
            }
            if matches!(member.obj.as_ref(), Expr::This(_)) {
                let binding = self.resolve_binding("this");
                if let Some(receiver_type) = self.scope.get(&binding).cloned() {
                    if let Some(owner) = self.class_instance_accessor_owner(&receiver_type, &property) {
                        let symbol = class_getter_symbol(&owner, &property, false);
                        if self.signatures.get(&symbol).is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str())) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![self.assert_class_accessor_receiver(HirExpr::Var(binding), &owner)],
                            ));
                        }
                    }
                }
            }
            if let Some(reference) = self.lower_native_method_reference(member, &property)? {
                return Ok(reference);
            }
        }
        // `process.env.NAME` -- checked before the general cases since it's
        // a fixed two-level member chain, not a general property access.
        if let MemberProp::Ident(name_prop) = &member.prop {
            if let Expr::Member(inner) = member.obj.as_ref() {
                if let (Expr::Ident(obj), MemberProp::Ident(env_prop)) =
                    (inner.obj.as_ref(), &inner.prop)
                {
                    if obj.sym == *"process" && env_prop.sym == *"env" {
                        return Ok(HirExpr::EnvVar(name_prop.sym.to_string()));
                    }
                }
            }
        }
        // A receiver that's itself a call/new (`new C().x`, `factory().x`)
        // can't be seen through by the class-getter dispatch above, which
        // only recognizes identifier/`this` receivers -- so
        // `new C().getter` used to fail with "object has no field".
        // Lower the receiver once into a local and re-dispatch on that
        // binding, so a class getter still fires and the receiver is
        // evaluated exactly once (never once for the getter check and
        // again for the ordinary read).
        if matches!(member.obj.as_ref(), Expr::New(_) | Expr::Call(_)) {
            if let MemberProp::Ident(_) = &member.prop {
                let receiver = self.lower_member_receiver(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                let name = format!("__thaw_member_receiver_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), receiver_type.clone());
                let mut rebound = member.clone();
                *rebound.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    name.clone().into(),
                    member.span,
                ));
                let value = self.lower_member_read(&rebound)?;
                return self
                    .wrap_call_argument_bindings(value, &[(name, receiver_type, receiver)]);
            }
        }

        match &member.prop {
            MemberProp::Computed(computed) => {
                let sparse_array = self.expression_may_be_sparse_array(&member.obj);
                let conservative = matches!(member.obj.as_ref(), Expr::Ident(identifier)
                    if self.conservative_sparse_arrays.contains(&self.resolve_binding(identifier.sym.as_ref())));
                let obj = self.lower_member_receiver(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                let (obj, obj_ty) =
                    self.unwrap_required_optional_member(obj, obj_ty, "computed property");
                match obj_ty {
                    HirType::Array(element) => {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
                        // ponytail: object/array slots and conservatively sparse parameters
                        // keep their declared index ABI; fully tagged T[] reads need wider callers.
                        let tagged = matches!(element.as_ref(),
                            HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Undefined)
                            || matches!(element.as_ref(), HirType::Union(members) if members.contains(&HirType::Undefined));
                        let scalar = matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool);
                        if sparse_array && (tagged || (scalar && !conservative)) {
                            let array_type = HirType::Array(element.clone());
                            return self.lower_array_index(obj, array_type, *element, index);
                        }
                        Ok(HirExpr::TypedIndex(
                            Box::new(obj),
                            Box::new(index),
                            *element,
                        ))
                    }
                    HirType::Tuple(elements) => {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
                        let HirExpr::Lit(HirLit::F64(position)) = index else {
                            return Err("tuple index must be a numeric literal".into());
                        };
                        let position = position as usize;
                        let element = elements
                            .get(position)
                            .cloned()
                            .ok_or_else(|| format!("tuple index {position} is out of bounds"))?;
                        Ok(HirExpr::TypedIndex(
                            Box::new(obj),
                            Box::new(HirExpr::Lit(HirLit::F64(position as f64))),
                            element,
                        ))
                    }
                    HirType::Union(elements)
                        if elements.iter().all(|element| matches!(element, HirType::Array(_))) =>
                    {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
                        self.lower_union_array_index(
                            obj,
                            &elements,
                            index,
                            sparse_array,
                            conservative,
                        )
                    }
                    HirType::Object(fields) => {
                        if let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() {
                            let key = key.value.to_string_lossy().into_owned();
                            if fields.iter().any(|(name, _)| name == &key) {
                                return self.lower_fixed_object_property_read(obj, &fields, &key);
                            }
                            return Err(format!("object has no field `{key}`"));
                        }
                        let key = self.lower_expr(&computed.expr)?;
                        if let HirType::StrLiteral(key_name) = self.infer_expr_type(&key)? {
                            if fields.iter().any(|(name, _)| name == &key_name) {
                                return self
                                    .lower_fixed_object_property_read(obj, &fields, &key_name);
                            }
                            return Err(format!("object has no field `{key_name}`"));
                        }
                        let Some((_, payload)) = fields.first() else {
                            return Err("cannot dynamically index an empty object".into());
                        };
                        let payload = payload.clone();
                        if fields.iter().any(|(_, ty)| ty != &payload) {
                            let mut members = Vec::new();
                            for (_, ty) in &fields {
                                let (payload, absences) = match ty {
                                    HirType::Optional(payload) => {
                                        (payload.as_ref(), &[HirType::Undefined][..])
                                    }
                                    HirType::Nullable(payload) => {
                                        (payload.as_ref(), &[HirType::Null][..])
                                    }
                                    HirType::Nullish(payload) => {
                                        (payload.as_ref(), &[HirType::Null, HirType::Undefined][..])
                                    }
                                    other => (other, &[][..]),
                                };
                                if !matches!(
                                    payload,
                                    HirType::F64
                                        | HirType::I64
                                        | HirType::Bool
                                        | HirType::Str
                                        | HirType::Json
                                        | HirType::JsValue
                                        | HirType::Array(_)
                                        | HirType::Tuple(_)
                                        | HirType::Object(_)
                                        | HirType::Function(_, _)
                                        | HirType::Null
                                        | HirType::Undefined
                                ) {
                                    return Err(
                                        "dynamic heterogeneous object index requires word-sized union-compatible field payloads"
                                            .into(),
                                    );
                                }
                                if !members.contains(payload) {
                                    members.push(payload.clone());
                                }
                                for absence in absences {
                                    if !members.contains(absence) {
                                        members.push(absence.clone());
                                    }
                                }
                            }
                            if !members.contains(&HirType::Undefined) {
                                members.push(HirType::Undefined);
                            }
                            self.expect_type(&HirType::Str, &key, "computed object key")?;
                            return Ok(HirExpr::DynamicPropAccess(
                                Box::new(obj),
                                Box::new(key),
                                fields,
                                HirType::Union(members),
                            ));
                        }
                        let result = match &payload {
                            HirType::Optional(inner) => HirType::Optional(inner.clone()),
                            HirType::Nullable(inner) | HirType::Nullish(inner) => {
                                HirType::Nullish(inner.clone())
                            }
                            other => HirType::Optional(Box::new(other.clone())),
                        };
                        self.expect_type(&HirType::Str, &key, "computed object key")?;
                        Ok(HirExpr::DynamicPropAccess(
                            Box::new(obj),
                            Box::new(key),
                            fields,
                            result,
                        ))
                    }
                    HirType::Dictionary(element) => {
                        let key = self.lower_expr(&computed.expr)?;
                        let key = self.coerce_primitive_to_string(key)?;
                        self.typed_dictionary_read(obj, key, element.as_ref())
                    }
                    HirType::Json => {
                        let key = self.lower_expr(&computed.expr)?;
                        match self.infer_expr_type(&key)? {
                            HirType::Str => {
                                Ok(HirExpr::JsonKey(Box::new(obj), Box::new(key)))
                            }
                            HirType::F64 => {
                                Ok(HirExpr::JsonIndex(Box::new(obj), Box::new(key)))
                            }
                            other => Err(format!(
                                "JSON index expression must be string or number, got {other:?}"
                            )),
                        }
                    }
                    HirType::JsValue | HirType::Dynamic => {
                        let key = self.lower_expr(&computed.expr)?;
                        let key = match self.infer_expr_type(&key)? {
                            HirType::Str | HirType::Symbol | HirType::Dynamic => key,
                            HirType::F64 => self.coerce_primitive_to_string(key)?,
                            other => {
                                return Err(format!(
                                    "dynamic value index expression must be string, number, or symbol, got {other:?}"
                                ))
                            }
                        };
                        self.lower_dynamic_value_property_read(obj, key)
                    }
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            MemberProp::Ident(prop) => {
                // `<Builtin>.prototype` for a standard-library constructor
                // thaw only models as a call/new target (`Object`, `Array`,
                // `Date`, `RegExp`, `Map`, `Set`, `ArrayBuffer`, typed
                // arrays, ...). The realm has the real constructor, so read
                // `.prototype` off it as a live value -- the shape most of
                // test262's prototype tests use
                // (`Object.prototype.hasOwnProperty.call`,
                // `Array.prototype.map.call`, ...).
                if prop.sym == *"prototype" {
                    if let Expr::Ident(owner) = member.obj.as_ref() {
                        let name = self.resolve_binding(owner.sym.as_ref());
                        if !self.scope.contains_key(&name)
                            && !self.interfaces.contains_key(&name)
                            && is_builtin_prototype_owner(owner.sym.as_ref())
                        {
                            let object = HirExpr::Call(
                                Box::new(HirExpr::Var("getDynamicValue".to_string())),
                                vec![HirExpr::Lit(HirLit::Str(owner.sym.to_string()))],
                            );
                            return self.lower_dynamic_value_property_read(
                                object,
                                HirExpr::Lit(HirLit::Str("prototype".to_string())),
                            );
                        }
                    }
                }
                if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Math") {
                    let value = match prop.sym.as_ref() {
                        "E" => std::f64::consts::E,
                        "PI" => std::f64::consts::PI,
                        "LN2" => std::f64::consts::LN_2,
                        "LN10" => std::f64::consts::LN_10,
                        "LOG2E" => std::f64::consts::LOG2_E,
                        "LOG10E" => std::f64::consts::LOG10_E,
                        "SQRT1_2" => std::f64::consts::FRAC_1_SQRT_2,
                        "SQRT2" => std::f64::consts::SQRT_2,
                        _ => {
                            return Err(format!("unsupported Math property `{}`", prop.sym));
                        }
                    };
                    return Ok(HirExpr::Lit(HirLit::F64(value)));
                }
                if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Number") {
                    let value = match prop.sym.as_ref() {
                        "NaN" => f64::NAN,
                        "POSITIVE_INFINITY" => f64::INFINITY,
                        "NEGATIVE_INFINITY" => f64::NEG_INFINITY,
                        "MAX_VALUE" => f64::MAX,
                        "MIN_VALUE" => f64::from_bits(1),
                        "MAX_SAFE_INTEGER" => 9_007_199_254_740_991.0,
                        "MIN_SAFE_INTEGER" => -9_007_199_254_740_991.0,
                        "EPSILON" => f64::EPSILON,
                        _ => {
                            return Err(format!("unsupported Number property `{}`", prop.sym));
                        }
                    };
                    return Ok(HirExpr::Lit(HirLit::F64(value)));
                }
                let obj = self.lower_member_receiver(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                let (obj, obj_ty) =
                    self.unwrap_required_optional_member(obj, obj_ty, prop.sym.as_ref());
                // A custom property read on a *catch-bound* error string
                // (`catch (e) { e.status }`, real trigger: koa's
                // `http-errors` error) has no declared field to read.
                // Route it through `thaw_error_property`, which pulls the
                // value out of the caught error's own `\u{5}<json>` bag.
                // Only for a `catch` binding: an ordinary string has no
                // such bag, and treating every `str.status` as an error
                // lookup would be wrong.
                if let HirExpr::Var(name) = &obj {
                    if self.catch_bindings.contains(name)
                        && !matches!(
                            prop.sym.as_ref(),
                            "length"
                                | "name"
                                | "message"
                                | "cause"
                                | "code"
                                | "stack"
                                | "error"
                                | "suppressed"
                        )
                    {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_error_property".to_string())),
                            vec![
                                obj,
                                HirExpr::Lit(HirLit::Str(prop.sym.to_string())),
                            ],
                        ));
                    }
                }
                // `.prototype` on a first-class *function* value -- a
                // function declaration/expression used as a constructor
                // (real trigger: the test262 harness's own `Test262Error`,
                // whose `.prototype.toString` is assigned right after its
                // declaration). Coerce the function to a live realm function
                // and read its real `.prototype` object.
                if prop.sym == *"prototype"
                    && matches!(
                        obj_ty,
                        HirType::Function(_, _)
                            | HirType::CallableFunction(_, _, _, _)
                            | HirType::JsValue
                    )
                {
                    let object = if obj_ty == HirType::JsValue {
                        obj
                    } else {
                        self.coerce_to_declared(&HirType::JsValue, obj)?
                    };
                    return self.lower_dynamic_value_property_read(
                        object,
                        HirExpr::Lit(HirLit::Str("prototype".to_string())),
                    );
                }
                match &obj_ty {
                    HirType::Array(_) | HirType::Tuple(_) if prop.sym == *"length" => {
                        Ok(HirExpr::ArrayLen(Box::new(obj)))
                    }
                    // `RegExp.prototype.exec`/`String.prototype.match` result
                    // arrays carry `.index`/`.input`/`.groups` as extra
                    // properties (they're a plain `string[]` otherwise). The
                    // runtime records them per result buffer; a plain string
                    // array has no metadata, so these read as `-1`/null/empty.
                    HirType::Array(element)
                        if element.as_ref() == &HirType::Optional(Box::new(HirType::Str))
                            && matches!(prop.sym.as_ref(), "groups" | "index" | "input") =>
                    {
                        Ok(match prop.sym.as_ref() {
                            "groups" => HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_exec_groups".into())),
                                vec![obj],
                            ),
                            "index" => HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_exec_index".into())),
                                vec![obj],
                            ),
                            _ => HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_exec_input".into())),
                                vec![obj],
                            ),
                        })
                    }
                    // A tagged template's cooked-strings array (also a
                    // plain `string[]`) carries `.raw`, its unescaped
                    // sibling, the same "metadata keyed by the array's own
                    // buffer identity" pattern as `.index`/`.input`/
                    // `.groups` just above -- see `thaw_template_strings_
                    // register`/`_raw`, thaw-runtime's `template_strings.rs`.
                    // An unrelated `string[]` degrades to an empty array.
                    HirType::Array(element)
                        if element.as_ref() == &HirType::Str && prop.sym == *"raw" =>
                    {
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_template_strings_raw".into())),
                            vec![obj],
                        ))
                    }
                    HirType::Str if prop.sym == *"length" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".to_string())),
                        vec![obj],
                    )),
                    // `Symbol("x").description` -- the description part of
                    // the symbol's own runtime representation.
                    HirType::Symbol if prop.sym == *"description" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_symbol_description".to_string())),
                        vec![obj],
                    )),
                    // Every string is a potential caught exception (there is
                    // no separate `Error` type -- see
                    // `lower/expressions/lowering.rs`'s `new Error(...)`
                    // handling), so `.message`/`.name` are available on any
                    // `HirType::Str` value, not only ones bound by `catch`.
                    // An untagged string (a plain `throw "..."`, or any
                    // other ordinary string) has no name of its own and
                    // defaults to `Error`, with itself as the message,
                    // matching JavaScript's own default
                    // `Error.prototype.name`.
                    HirType::Str if prop.sym == *"message" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_message".to_string())),
                        vec![obj],
                    )),
                    HirType::Str if prop.sym == *"name" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_name".to_string())),
                        vec![obj],
                    )),
                    HirType::Str if prop.sym == *"cause" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_cause".to_string())),
                        vec![obj],
                    )),
                    // `SuppressedError`'s two sub-errors, recovered from the
                    // exception tag's own trailing segments (empty on any
                    // other error, matching `.cause`'s convention).
                    HirType::Str if prop.sym == *"error" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_suppressed_error".to_string())),
                        vec![obj],
                    )),
                    HirType::Str if prop.sym == *"suppressed" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_suppressed".to_string())),
                        vec![obj],
                    )),
                    HirType::Str if prop.sym == *"code" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_code".to_string())),
                        vec![obj],
                    )),
                    // ponytail: no real call-stack frames -- see
                    // `thaw_error_stack`'s own doc comment (thaw-runtime)
                    // for why this can't just reuse `.toString()`'s
                    // intrinsic despite the similar rendering.
                    HirType::Str if prop.sym == *"stack" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_stack".to_string())),
                        vec![obj],
                    )),
                    HirType::Map(_, _) | HirType::Set(_) if prop.sym == *"size" => {
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_size".to_string())),
                            vec![obj],
                        ))
                    }
                    HirType::Object(_)
                        if obj_ty == regex_object_type()
                            && matches!(
                                prop.sym.as_ref(),
                                "global"
                                    | "ignoreCase"
                                    | "multiline"
                                    | "dotAll"
                                    | "sticky"
                                    | "unicode"
                                    | "unicodeSets"
                                    | "hasIndices"
                            ) =>
                    {
                        // These aren't stored fields -- `regex_object_type`
                        // only has `source`/`flags` -- they're each just
                        // "does `flags` contain this one character",
                        // matching the specification's own definition of
                        // each accessor.
                        let flag_char = match prop.sym.as_ref() {
                            "global" => "g",
                            "ignoreCase" => "i",
                            "multiline" => "m",
                            "dotAll" => "s",
                            "sticky" => "y",
                            "unicode" => "u",
                            "unicodeSets" => "v",
                            "hasIndices" => "d",
                            _ => unreachable!(),
                        };
                        let flags = HirExpr::PropAccess(
                            Box::new(obj),
                            obj_ty.clone(),
                            "flags".to_string(),
                        );
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_includes".to_string())),
                            vec![
                                flags,
                                HirExpr::Lit(HirLit::Str(flag_char.to_string())),
                                HirExpr::Lit(HirLit::F64(0.0)),
                            ],
                        ))
                    }
                    HirType::Object(fields) => {
                        // An object-literal accessor stores a hidden closure
                        // under `__thaw_getter_<name>`/`__thaw_setter_<name>`;
                        // a read dispatches to the getter (always passing the
                        // receiver as its leading argument), a lone setter
                        // reads `undefined`, and an ordinary data field reads
                        // directly.
                        self.lower_fixed_object_property_read(obj, fields, prop.sym.as_ref())
                    }
                    HirType::Union(elements) => {
                        self.lower_union_property_read(obj, elements, prop.sym.as_ref())
                    }
                    HirType::Json => {
                        // `obj.x` on a JSON `null`/`undefined` throws in JS;
                        // a bare `JsonGet` would otherwise read a missing key
                        // as `null`. Guard the receiver first, throwing the
                        // same tagged `TypeError` `catch` decodes (and
                        // `z?.x` already short-circuits before this).
                        let receiver = format!("__thaw_json_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver.clone(), HirType::Json);
                        let var = HirExpr::Var(receiver.clone());
                        let is_nullish = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                            vec![var.clone()],
                        );
                        let get = HirExpr::JsonGet(Box::new(var), prop.sym.to_string());
                        self.wrap_call_argument_bindings(
                            HirExpr::Block(vec![
                                HirStmt::If(
                                    is_nullish,
                                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                                        "\u{1}TypeError\u{1}Cannot read properties of null (reading '{}')",
                                        prop.sym
                                    ))))],
                                    Vec::new(),
                                ),
                                HirStmt::Return(Some(get)),
                            ]),
                            &[(receiver, HirType::Json, obj)],
                        )
                    }
                    HirType::Dictionary(element) => self.typed_dictionary_read(
                        obj,
                        HirExpr::Lit(HirLit::Str(prop.sym.to_string())),
                        element.as_ref(),
                    ),
                    HirType::JsValue | HirType::Dynamic => {
                        self.lower_dynamic_value_property_read(
                            obj,
                            HirExpr::Lit(HirLit::Str(prop.sym.to_string())),
                        )
                    }
                    other => Err(format!(
                        "unsupported property access `.{}` on a value of type {other:?}",
                        prop.sym
                    )),
                }
            }
            _ => Err("unsupported property access".into()),
        }
    }

    fn lower_optional_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        let object = self.lower_expr(&member.obj)?;
        let object_type = self.infer_expr_type(&object)?;
        // A dynamic `any` value (`Json`) short-circuits to `undefined` for
        // a nullish receiver -- read the key directly, without the
        // TypeError guard the non-optional `Json` read now applies.
        if object_type == HirType::Json {
            let field = match &member.prop {
                MemberProp::Ident(field) => field.sym.to_string(),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(field)) => field.value.to_string_lossy().into_owned(),
                    _ => {
                        return Err(
                            "optional computed object keys must be string literals".into()
                        )
                    }
                },
                _ => return Err("unsupported optional object member".into()),
            };
            return Ok(HirExpr::JsonGet(Box::new(object), field));
        }
        let (payload, absence_kind) = match object_type.clone() {
            HirType::Optional(payload) => (payload, 0),
            HirType::Nullable(payload) => (payload, 1),
            HirType::Nullish(payload) => (payload, 2),
            _ => return self.lower_member_read(member),
        };
        let name = format!("__thaw_optional_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), object_type.clone());
        let bound = HirExpr::Var(name.clone());
        let unwrapped = match absence_kind {
            0 => HirExpr::OptionalValue(Box::new(bound.clone()), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(bound.clone()), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(bound.clone()), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let (access, field_type) = match payload.as_ref() {
            HirType::Object(fields) => {
                let field = match &member.prop {
                    MemberProp::Ident(field) => field.sym.to_string(),
                    MemberProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(field)) => field.value.to_string_lossy().into_owned(),
                        _ => {
                            return Err(
                                "optional computed object keys must be string literals".into()
                            )
                        }
                    },
                    _ => return Err("unsupported optional object member".into()),
                };
                let field_type = fields
                    .iter()
                    .find(|(name, _)| name == &field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{field}`"))?;
                (
                    self.lower_fixed_object_property_read(unwrapped, fields, &field)?,
                    field_type,
                )
            }
            HirType::Array(element) => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => {
                    (HirExpr::ArrayLen(Box::new(unwrapped)), HirType::F64)
                }
                MemberProp::Ident(property)
                    if property.sym == *"groups" && element.as_ref() == &HirType::Optional(Box::new(HirType::Str)) =>
                {
                    let ty = HirType::Dictionary(Box::new(HirType::Optional(Box::new(
                        HirType::Str,
                    ))));
                    (
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_regex_exec_groups".into())),
                            vec![unwrapped],
                        ),
                        ty,
                    )
                }
                MemberProp::Ident(property)
                    if property.sym == *"index" && element.as_ref() == &HirType::Optional(Box::new(HirType::Str)) =>
                {
                    (
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_regex_exec_index".into())),
                            vec![unwrapped],
                        ),
                        HirType::F64,
                    )
                }
                MemberProp::Ident(property)
                    if property.sym == *"input" && element.as_ref() == &HirType::Optional(Box::new(HirType::Str)) =>
                {
                    (
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_regex_exec_input".into())),
                            vec![unwrapped],
                        ),
                        HirType::Str,
                    )
                }
                MemberProp::Computed(computed) => {
                    let index = self.lower_expr(&computed.expr)?;
                    self.expect_type(&HirType::F64, &index, "optional array index")?;
                    let element_type = element.as_ref().clone();
                    let access = self.lower_array_index(
                        unwrapped,
                        payload.as_ref().clone(),
                        element_type,
                        index,
                    )?;
                    let ty = self.infer_expr_type(&access)?;
                    (
                        access,
                        ty,
                    )
                }
                _ => return Err("unsupported optional array member".into()),
            },
            HirType::Tuple(elements) => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => {
                    (HirExpr::ArrayLen(Box::new(unwrapped)), HirType::F64)
                }
                MemberProp::Computed(computed) => {
                    let Expr::Lit(Lit::Num(index)) = computed.expr.as_ref() else {
                        return Err("optional tuple index must be a numeric literal".into());
                    };
                    let index = index.value as usize;
                    let element = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple index {index} is out of bounds"))?;
                    (
                        HirExpr::TypedIndex(
                            Box::new(unwrapped),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element.clone(),
                        ),
                        element,
                    )
                }
                _ => return Err("unsupported optional tuple member".into()),
            },
            HirType::Str => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => (
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".into())),
                        vec![unwrapped],
                    ),
                    HirType::F64,
                ),
                _ => return Err("unsupported optional string member".into()),
            },
            HirType::Dictionary(element) => {
                let key = match &member.prop {
                    MemberProp::Ident(property) => {
                        HirExpr::Lit(HirLit::Str(property.sym.to_string()))
                    }
                    MemberProp::Computed(computed) => {
                        let key = self.lower_expr(&computed.expr)?;
                        self.coerce_primitive_to_string(key)?
                    }
                    _ => return Err("unsupported optional dictionary member".into()),
                };
                (
                    self.typed_dictionary_read(unwrapped, key, element.as_ref())?,
                    element.as_ref().clone(),
                )
            }
            HirType::Json => match &member.prop {
                MemberProp::Ident(property) => (
                    HirExpr::JsonGet(Box::new(unwrapped), property.sym.to_string()),
                    HirType::Json,
                ),
                MemberProp::Computed(computed) => {
                    let key = self.lower_expr(&computed.expr)?;
                    match self.infer_expr_type(&key)? {
                        HirType::Str => (
                            HirExpr::JsonKey(Box::new(unwrapped), Box::new(key)),
                            HirType::Json,
                        ),
                        HirType::F64 => (
                            HirExpr::JsonIndex(Box::new(unwrapped), Box::new(key)),
                            HirType::Json,
                        ),
                        other => {
                            return Err(format!(
                                "optional JSON key must be string or number, got {other:?}"
                            ))
                        }
                    }
                }
                _ => return Err("unsupported optional JSON member".into()),
            },
            other => {
                return Err(format!(
                    "optional member access is not yet supported on {other:?}"
                ))
            }
        };
        let (missing_value, present_value) = match &field_type {
            HirType::Optional(inner) => (
                HirExpr::OptionalNone(inner.as_ref().clone()),
                access,
            ),
            HirType::Nullish(inner) => (
                HirExpr::NullishUndefined(inner.as_ref().clone()),
                access,
            ),
            HirType::Union(members) => {
                let mut members = members.clone();
                if !members.contains(&HirType::Undefined) {
                    members.push(HirType::Undefined);
                }
                let index = members.iter().position(|member| member == &HirType::Undefined).unwrap();
                (
                    HirExpr::UnionInject(
                        Box::new(HirExpr::Lit(HirLit::Undefined)), index, members,
                    ),
                    access,
                )
            }
            other => (
                HirExpr::OptionalNone(other.clone()),
                HirExpr::OptionalSome(Box::new(access), other.clone()),
            ),
        };
        let is_none = match absence_kind {
            0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
            1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
            2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            is_none,
            vec![HirStmt::Return(Some(missing_value))],
            vec![HirStmt::Return(Some(present_value))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, object_type, object)])
    }

    fn lower_fixed_object_property_read(
        &mut self,
        object: HirExpr,
        fields: &[(Symbol, HirType)],
        property: &str,
    ) -> Result<HirExpr, String> {
        if !fields.iter().any(|(name, _)| name == property) {
            return Err(format!("object has no field `{property}`"));
        }
        let object_type = HirType::Object(fields.to_vec());
        let getter = format!("__thaw_getter_{property}");
        if fields.iter().any(|(name, _)| name == &getter) {
            if !matches!(object, HirExpr::Var(_)) {
                let receiver = format!("__thaw_accessor_receiver_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(receiver.clone(), object_type.clone());
                let bound = HirExpr::Var(receiver.clone());
                let getter = HirExpr::PropAccess(
                    Box::new(bound.clone()),
                    object_type.clone(),
                    getter,
                );
                let read = HirExpr::Call(Box::new(getter), vec![bound]);
                return self.wrap_call_argument_bindings(
                    read,
                    &[(receiver, object_type, object)],
                );
            }
            let getter = HirExpr::PropAccess(Box::new(object.clone()), object_type, getter);
            return Ok(HirExpr::Call(Box::new(getter), vec![object]));
        }
        if fields
            .iter()
            .any(|(name, _)| name == &format!("__thaw_setter_{property}"))
        {
            if !matches!(object, HirExpr::Var(_)) {
                let receiver = format!("__thaw_accessor_receiver_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(receiver.clone(), object_type.clone());
                return self.wrap_call_argument_bindings(
                    HirExpr::Lit(HirLit::Undefined),
                    &[(receiver, object_type, object)],
                );
            }
            return Ok(HirExpr::Lit(HirLit::Undefined));
        }
        Ok(HirExpr::PropAccess(
            Box::new(object),
            object_type,
            property.to_string(),
        ))
    }

    fn fixed_object_property_read_type(
        fields: &[(Symbol, HirType)],
        property: &str,
    ) -> Result<HirType, String> {
        let getter = format!("__thaw_getter_{property}");
        if let Some((_, getter_type)) = fields.iter().find(|(name, _)| name == &getter) {
            return match getter_type {
                HirType::Function(_, ret) | HirType::CallableFunction(_, _, _, ret) => {
                    Ok(ret.as_ref().clone())
                }
                other => Err(format!("object getter `{property}` has type {other:?}")),
            };
        }
        if fields
            .iter()
            .any(|(name, _)| name == &format!("__thaw_setter_{property}"))
        {
            return Ok(HirType::Undefined);
        }
        fields
            .iter()
            .find(|(name, _)| name == property)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| format!("object has no field `{property}`"))
    }

    fn materialize_fixed_object_for_json_replacer(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        keys: &[Symbol],
    ) -> Result<HirExpr, String> {
        let source_type = HirType::Object(fields.to_vec());
        let source_name = format!("__thaw_json_replacer_object_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), source_type.clone());
        let source = HirExpr::Var(source_name.clone());
        let mut selected = Vec::new();
        for key in keys {
            if !fields.iter().any(|(name, _)| name == key) {
                continue;
            }
            let read = self.lower_fixed_object_property_read(
                source.clone(), fields, key,
            )?;
            let read_type = Self::fixed_object_property_read_type(fields, key)?;
            let read = if let HirType::Object(nested) = &read_type {
                self.materialize_fixed_object_for_json_replacer(read, nested, keys)?
            } else {
                read
            };
            let read_type = self.infer_expr_type(&read)?;
            selected.push((key.clone(), read_type, read));
        }
        let result_type = HirType::Object(selected.iter()
            .map(|(name, ty, _)| (name.clone(), ty.clone())).collect());
        let result_name = format!("__thaw_json_replacer_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), result_type.clone());
        let result = HirExpr::Var(result_name.clone());
        let mut body = vec![HirStmt::Let(result_name, result_type.clone(),
            HirExpr::ObjectAlloc(result_type.clone()))];
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_object_order_begin".into())), vec![result.clone()],
        )));
        for (name, _, _) in &selected {
            if name.starts_with("__thaw_class_identity_\u{1e}") {
                body.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_hide_marker".into())),
                    vec![result.clone(), HirExpr::Lit(HirLit::Str(name.clone()))],
                )));
            }
        }
        for (name, _, read) in selected {
            let key = HirExpr::Lit(HirLit::Str(name.clone()));
            let hidden = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_marker_hidden".into())),
                vec![source.clone(), key.clone()],
            );
            body.push(HirStmt::If(hidden, Vec::new(), vec![
                HirStmt::Expr(HirExpr::PropAssign(
                    Box::new(result.clone()), result_type.clone(), name,
                    Box::new(read),
                )),
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                    vec![result.clone(), key],
                )),
            ]));
        }
        body.push(HirStmt::Return(Some(result)));
        self.wrap_call_argument_bindings(
            HirExpr::Block(body),
            &[(source_name, source_type, value)],
        )
    }

    fn fixed_object_property_descriptor(
        &mut self,
        target: HirExpr,
        target_type: &HirType,
        fields: &[(Symbol, HirType)],
        property: &str,
    ) -> Result<HirExpr, String> {
        if !fields.iter().any(|(name, _)| {
            name == &format!("__thaw_getter_{property}")
                || name == &format!("__thaw_setter_{property}")
        }) {
            return self.runtime_fixed_object_property_descriptor(
                target,
                target_type,
                fields,
                property,
            );
        }
        let source_name = format!("__thaw_descriptor_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), target_type.clone());
        let source = HirExpr::Var(source_name.clone());
        let flags_name = format!("__thaw_descriptor_static_flags_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(flags_name.clone(), HirType::F64);
        let attribute = |bit: u8| HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::BinOp(
                BinOp::BitAnd,
                Box::new(HirExpr::Var(flags_name.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(bit as f64))),
            )),
            Box::new(HirExpr::Lit(HirLit::F64(bit as f64))),
        );
        let getter = format!("__thaw_getter_{property}");
        let setter = format!("__thaw_setter_{property}");
        let mut accessor = |hidden: String| -> Result<Option<HirExpr>, String> {
            let Some((_, ty)) = fields.iter().find(|(name, _)| name == &hidden) else {
                return Ok(None);
            };
            let (params, ret) = match ty {
                HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
                    (params, ret.as_ref().clone())
                }
                other => return Err(format!("object accessor `{property}` has type {other:?}")),
            };
            let Some((_, forwarded)) = params.split_first() else {
                return Err(format!("object accessor `{property}` has no receiver"));
            };
            let mut adapter_params = Vec::with_capacity(forwarded.len());
            for (index, ty) in forwarded.iter().enumerate() {
                let name = format!("__thaw_descriptor_argument_{}_{}", index, self.next_binding);
                self.scope.insert(name.clone(), ty.clone());
                adapter_params.push(HirParam { name, ty: ty.clone() });
            }
            self.next_binding += 1;
            let mut arguments = vec![source.clone()];
            arguments.extend(
                adapter_params
                    .iter()
                    .map(|parameter| HirExpr::Var(parameter.name.clone())),
            );
            let call = HirExpr::Call(
                Box::new(HirExpr::PropAccess(
                    Box::new(source.clone()),
                    target_type.clone(),
                    hidden,
                )),
                arguments,
            );
            Ok(Some(HirExpr::Lambda(
                vec![HirParam {
                    name: source_name.clone(),
                    ty: target_type.clone(),
                }],
                adapter_params,
                ret,
                Box::new(call),
            )))
        };
        let getter = accessor(getter)?;
        let setter = accessor(setter)?;
        if getter.is_some() || setter.is_some() {
            let descriptor = HirExpr::ObjectLit(vec![
                (
                    "get".into(),
                    getter.unwrap_or(HirExpr::Lit(HirLit::Undefined)),
                ),
                (
                    "set".into(),
                    setter.unwrap_or(HirExpr::Lit(HirLit::Undefined)),
                ),
                ("enumerable".into(), attribute(2)),
                ("configurable".into(), attribute(4)),
            ]);
            return self.wrap_call_argument_bindings(
                descriptor,
                &[
                    (source_name, target_type.clone(), target),
                    (flags_name, HirType::F64, HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_property_flags".into())),
                        vec![source.clone(), HirExpr::Lit(HirLit::Str(property.into()))],
                    )),
                ],
            );
        }
        let descriptor = HirExpr::ObjectLit(vec![
            (
                "value".into(),
                HirExpr::PropAccess(
                    Box::new(source.clone()),
                    target_type.clone(),
                    property.into(),
                ),
            ),
            ("writable".into(), attribute(1)),
            ("enumerable".into(), attribute(2)),
            ("configurable".into(), attribute(4)),
        ]);
        self.wrap_call_argument_bindings(
            descriptor,
            &[
                (source_name, target_type.clone(), target),
                (flags_name, HirType::F64, HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_property_flags".into())),
                    vec![source, HirExpr::Lit(HirLit::Str(property.into()))],
                )),
            ],
        )
    }

    fn runtime_fixed_object_property_descriptor(
        &mut self,
        target: HirExpr,
        target_type: &HirType,
        fields: &[(Symbol, HirType)],
        property: &str,
    ) -> Result<HirExpr, String> {
        let field_type = fields
            .iter()
            .find(|(name, _)| name == property)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| format!("object has no field `{property}`"))?;
        let source_name = format!("__thaw_descriptor_source_{}", self.next_binding);
        self.next_binding += 1;
        let getter_name = format!("__thaw_descriptor_has_getter_{}", self.next_binding);
        self.next_binding += 1;
        let setter_name = format!("__thaw_descriptor_has_setter_{}", self.next_binding);
        self.next_binding += 1;
        let flags_name = format!("__thaw_descriptor_flags_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), target_type.clone());
        self.scope.insert(getter_name.clone(), HirType::Bool);
        self.scope.insert(setter_name.clone(), HirType::Bool);
        self.scope.insert(flags_name.clone(), HirType::F64);
        let source = HirExpr::Var(source_name.clone());
        let key = HirExpr::Lit(HirLit::Str(property.into()));
        let has_accessor = |setter| {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_has_accessor".into())),
                vec![
                    source.clone(),
                    key.clone(),
                    HirExpr::Lit(HirLit::Bool(setter)),
                ],
            )
        };
        let getter = HirExpr::Lambda(
            vec![HirParam {
                name: source_name.clone(),
                ty: target_type.clone(),
            }],
            Vec::new(),
            field_type.clone(),
            Box::new(HirExpr::PropAccess(
                Box::new(source.clone()),
                target_type.clone(),
                property.into(),
            )),
        );
        let value_name = format!("__thaw_descriptor_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(value_name.clone(), field_type.clone());
        let setter = HirExpr::Lambda(
            vec![HirParam {
                name: source_name.clone(),
                ty: target_type.clone(),
            }],
            vec![HirParam {
                name: value_name.clone(),
                ty: field_type.clone(),
            }],
            HirType::Void,
            Box::new(HirExpr::Block(vec![
                HirStmt::Expr(HirExpr::PropAssign(
                    Box::new(source.clone()),
                    target_type.clone(),
                    property.into(),
                    Box::new(HirExpr::Var(value_name)),
                )),
                HirStmt::Return(None),
            ])),
        );
        let flags = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(vec![
                HirExpr::Var(getter_name.clone()),
                HirExpr::Var(setter_name.clone()),
                HirExpr::Var(flags_name.clone()),
            ]),
        )?;
        let accessor_descriptor = HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())),
            vec![
                HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str(
                        "__thaw_accessor_descriptor".into(),
                    ))],
                ),
                flags,
                HirExpr::ArrayLit(vec![
                    HirExpr::Call(
                        Box::new(HirExpr::Var("registerNativeCallback".into())),
                        vec![getter],
                    ),
                    HirExpr::Call(
                        Box::new(HirExpr::Var("registerNativeCallback".into())),
                        vec![setter],
                    ),
                ]),
            ],
        );
        let value = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::PropAccess(
                Box::new(source.clone()),
                target_type.clone(),
                property.into(),
            ),
        )?;
        let arguments = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![
            value, HirExpr::Var(flags_name.clone()),
        ]))?;
        let data_descriptor = HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
            vec![
                HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("__thaw_data_descriptor".into()))],
                ),
                arguments,
            ],
        );
        let descriptor = HirExpr::Conditional(
            Box::new(HirExpr::Var(getter_name.clone())),
            Box::new(accessor_descriptor.clone()),
            Box::new(HirExpr::Conditional(
                Box::new(HirExpr::Var(setter_name.clone())),
                Box::new(accessor_descriptor),
                Box::new(data_descriptor),
                HirType::JsValue,
            )),
            HirType::JsValue,
        );
        self.wrap_call_argument_bindings(
            descriptor,
            &[
                (source_name, target_type.clone(), target),
                (getter_name, HirType::Bool, has_accessor(false)),
                (setter_name, HirType::Bool, has_accessor(true)),
                (flags_name, HirType::F64, HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_property_flags".into())),
                    vec![source, key],
                )),
            ],
        )
    }

    fn fixed_object_contains_accessor(fields: &[(Symbol, HirType)]) -> bool {
        fields.iter().any(|(name, ty)| {
            is_hidden_accessor_field(name)
                || matches!(ty, HirType::Object(nested) if Self::fixed_object_contains_accessor(nested))
        })
    }

    /// An unsafe prefix receiver may enumerate or return its synthetic
    /// marker. Keep that layout on the established snapshot path until a
    /// full concrete receiver shape can be recovered at allocation time.
    fn fixed_object_supports_live_projection(ty: &HirType) -> bool {
        match ty {
            HirType::Object(fields) => !fields.iter().any(|(name, _)|
                    name.starts_with("__thaw_class_identity_\u{1e}"))
                && fields.iter()
                    .filter(|(name, _)| !is_hidden_accessor_field(name))
                    .all(|(_, field)| Self::fixed_object_supports_live_projection(field)),
            HirType::Function(params, _)
            | HirType::CallableFunction(params, _, _, _) =>
                !matches!(params.first(), Some(HirType::Object(receiver))
                    if receiver.last().is_some_and(|(name, marker)|
                        name == "__thaw_object_method_receiver"
                            && *marker != HirType::Bool)),
            HirType::Optional(inner) | HirType::Nullable(inner)
            | HirType::Nullish(inner) | HirType::Array(inner) =>
                Self::fixed_object_supports_live_projection(inner),
            HirType::Union(members) | HirType::Tuple(members) =>
                members.iter().all(Self::fixed_object_supports_live_projection),
            _ => true,
        }
    }

    fn lower_fixed_object_as_dynamic_accessor_object(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        factory_body: bool,
    ) -> Result<HirExpr, String> {
        let source_type = HirType::Object(fields.to_vec());
        let source_name = format!("__thaw_dynamic_accessor_object_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), source_type.clone());
        let keys_name = format!("__thaw_dynamic_keys_{}", self.next_binding);
        self.next_binding += 1;
        let readable_name = format!("__thaw_dynamic_readable_{}", self.next_binding);
        self.next_binding += 1;
        let writable_name = format!("__thaw_dynamic_writable_{}", self.next_binding);
        self.next_binding += 1;
        let accessor_name = format!("__thaw_dynamic_accessor_{}", self.next_binding);
        self.next_binding += 1;
        let callbacks_name = format!("__thaw_dynamic_callbacks_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(keys_name.clone(), HirType::Array(Box::new(HirType::Str)));
        self.scope.insert(readable_name.clone(), HirType::Array(Box::new(HirType::Bool)));
        self.scope.insert(writable_name.clone(), HirType::Array(Box::new(HirType::Bool)));
        self.scope.insert(accessor_name.clone(), HirType::Array(Box::new(HirType::Bool)));
        self.scope.insert(callbacks_name.clone(), HirType::Array(Box::new(HirType::JsValue)));
        let mut metadata_actions = Vec::new();
        let mut callback_actions = Vec::new();
        let mut define_actions = Vec::new();
        for index in ecmascript_field_order(fields) {
            let name = &fields[index].0;
            let has_getter = fields
                .iter()
                .any(|(field, _)| field == &format!("__thaw_getter_{name}"));
            let has_setter = fields
                .iter()
                .any(|(field, _)| field == &format!("__thaw_setter_{name}"));
            let setter_only = !has_getter && has_setter;
            let writable = !has_getter || has_setter;
            let absent_callback = || HirExpr::Call(
                Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
            );
            let getter = if setter_only {
                absent_callback()
            } else {
                let mut read = self.lower_fixed_object_property_read(
                    HirExpr::Var("__thaw_this".into()), fields, name,
                )?;
                let mut read_type = Self::fixed_object_property_read_type(fields, name)?;
                if let HirType::Object(nested) = &read_type {
                    if Self::fixed_object_supports_live_projection(&read_type) {
                        // Preserve nested native storage identity when its
                        // receiver layout is safe to expose as a live host.
                        read = self.lower_fixed_object_as_dynamic_accessor_object(read, nested, false)?;
                        read_type = HirType::JsValue;
                    }
                }
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: "__thaw_this".into(), ty: source_type.clone(),
                    }],
                    read_type, Box::new(read),
                );
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_register_native_method_callback_graph".into())),
                    vec![callback],
                )
            };
            let setter = if writable {
                // PropAssign's LLVM path uses the visible field slot type,
                // then honors the registered setter and integrity guards.
                let field_type = fields.iter().find(|(field, _)| field == name)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{name}`"))?;
                let value_name = format!("__thaw_dynamic_write_{}", self.next_binding);
                self.next_binding += 1;
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![
                        HirParam { name: "__thaw_this".into(), ty: source_type.clone() },
                        HirParam { name: value_name.clone(), ty: field_type },
                    ],
                    HirType::Void,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(HirExpr::Var("__thaw_this".into())),
                            source_type.clone(), name.clone(),
                            Box::new(HirExpr::Var(value_name)),
                        )),
                        HirStmt::Return(None),
                    ])),
                );
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_register_native_method_callback_graph".into())),
                    vec![callback],
                )
            } else { absent_callback() };
            let append = |array: &str, value| HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(array.to_string()), value],
            ));
            metadata_actions.push((index, vec![
                append(&keys_name, HirExpr::Lit(HirLit::Str(name.clone()))),
                append(&readable_name, HirExpr::Lit(HirLit::Bool(!setter_only))),
                append(&writable_name, HirExpr::Lit(HirLit::Bool(writable))),
                append(&accessor_name, HirExpr::Lit(HirLit::Bool(has_getter || has_setter))),
            ]));
            callback_actions.push((index, vec![
                append(&callbacks_name, getter),
                append(&callbacks_name, setter),
            ]));
            // DefineOwnProperty writes an existing data slot even when it is
            // non-writable but still configurable. Its private callback
            // stores through the same typed offset without using PropAssign's
            // ordinary-assignment readonly guard.
            let define_value = if has_getter || has_setter {
                absent_callback()
            } else {
                let field_type = fields[index].1.clone();
                let value_name = format!("__thaw_dynamic_define_{}", self.next_binding);
                self.next_binding += 1;
                let flags_name = format!("__thaw_dynamic_define_flags_{}", self.next_binding);
                self.next_binding += 1;
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![
                        HirParam { name: "__thaw_this".into(), ty: source_type.clone() },
                        HirParam { name: value_name.clone(), ty: field_type },
                        HirParam { name: flags_name.clone(), ty: HirType::F64 },
                    ],
                    HirType::Bool,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_define_data_value".into())),
                            vec![
                                HirExpr::Var("__thaw_this".into()),
                                HirExpr::Lit(HirLit::Str(name.clone())),
                                HirExpr::Var(value_name),
                                HirExpr::Var(flags_name),
                            ],
                        ))),
                    ])),
                );
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_register_native_method_callback_graph".into())),
                    vec![callback],
                )
            };
            define_actions.push((index, vec![append(&callbacks_name, define_value)]));
        }
        let cached_name = format!("__thaw_cached_native_object_{}", self.next_binding);
        self.next_binding += 1;
        let cached_type = HirType::Optional(Box::new(HirType::JsValue));
        self.scope.insert(cached_name.clone(), cached_type.clone());
        let layout = crate::native_object_layout_token(fields);
        let mut body = Vec::new();
        if !factory_body {
            body.extend([
            HirStmt::Let(cached_name.clone(), cached_type,
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_lookup_native_object".into())),
                    vec![HirExpr::Var(source_name.clone()),
                        HirExpr::Lit(HirLit::Str(layout.clone()))])),
            HirStmt::If(
                HirExpr::OptionalIsNone(Box::new(HirExpr::Var(cached_name.clone())), HirType::JsValue),
                Vec::new(),
                vec![HirStmt::Return(Some(HirExpr::OptionalValue(
                    Box::new(HirExpr::Var(cached_name)), HirType::JsValue)))],
            ),
            ]);
            let projector_name = format!("__thaw_full_native_projector_{}", self.next_binding);
            self.next_binding += 1;
            let projector_type = HirType::Function(Vec::new(), Box::new(HirType::JsValue));
            self.scope.insert(projector_name.clone(), HirType::Optional(Box::new(projector_type.clone())));
            body.extend([
                HirStmt::Let(projector_name.clone(), HirType::Optional(Box::new(projector_type.clone())),
                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_lookup_native_projector".into())),
                        vec![HirExpr::Var(source_name.clone()),
                            HirExpr::Lit(HirLit::Str(layout.clone()))])),
                HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(HirExpr::Var(projector_name.clone())), projector_type.clone()),
                    Vec::new(),
                    vec![HirStmt::Return(Some(HirExpr::Call(
                        Box::new(HirExpr::OptionalValue(Box::new(HirExpr::Var(projector_name)), projector_type)),
                        Vec::new(),
                    )))],
                ),
            ]);
        }
        body.extend([
            HirStmt::Let(keys_name.clone(), HirType::Array(Box::new(HirType::Str)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(readable_name.clone(), HirType::Array(Box::new(HirType::Bool)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(writable_name.clone(), HirType::Array(Box::new(HirType::Bool)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(accessor_name.clone(), HirType::Array(Box::new(HirType::Bool)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(callbacks_name.clone(), HirType::Array(Box::new(HirType::JsValue)),
                HirExpr::ArrayLit(Vec::new())),
        ]);
        body.extend(self.lower_ordered_fixed_field_statements(
            HirExpr::Var(source_name.clone()), fields, metadata_actions,
        )?);
        let keys = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::Var(keys_name),
        )?;
        let readable = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::Var(readable_name),
        )?;
        let writable = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::Var(writable_name),
        )?;
        let accessors = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::Var(accessor_name),
        )?;
        let arguments = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(vec![keys, readable, writable, accessors]),
        )?;
        let args_name = format!("__thaw_native_projection_args_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(args_name.clone(), HirType::Json);
        body.push(HirStmt::Let(args_name.clone(), HirType::Json, arguments));
        let builder_name = format!("__thaw_native_projection_builder_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(builder_name.clone(), HirType::JsValue);
        body.push(HirStmt::Let(builder_name.clone(), HirType::JsValue,
            HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str(
                    "__thaw_object_with_native_getters".into(),
                ))])));
        // A borrowed foreign FFI object has no arena owner to retain. Fail
        // before registering the first callback that could capture its pointer.
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_require_native_owner".into())),
            vec![HirExpr::Var(source_name.clone())],
        )));
        // Resolve the builder and encode plain metadata before acquiring any
        // temporary native callback handles. From this point to the builder
        // ABI, a later callback registration can still fail after earlier
        // handles were acquired. Guard that acquisition interval separately;
        // the builder ABI consumes the complete array on both outcomes.
        let callback_start = body.len();
        body.extend(self.lower_ordered_fixed_field_statements(
            HirExpr::Var(source_name.clone()), fields, callback_actions,
        )?);
        // These callbacks read the same pointer-keyed integrity state used by
        // direct native Object/Reflect operations. The JS wrapper must never
        // infer native extensibility from its own shadow target alone.
        for query in 0..3 {
            let state = HirExpr::Lambda(
                vec![HirParam { name: source_name.clone(), ty: source_type.clone() }],
                Vec::new(), HirType::Bool,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_state".into())),
                    vec![HirExpr::Var(source_name.clone()), HirExpr::Lit(HirLit::F64(query as f64))],
                )),
            );
            body.push(HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(callbacks_name.clone()), HirExpr::Call(
                    Box::new(HirExpr::Var("registerNativeCallbackGraph".into())), vec![state],
                )],
            )));
        }
        let operation_name = format!("__thaw_native_integrity_op_{}", self.next_binding);
        self.next_binding += 1;
        let state_update = HirExpr::Lambda(
            vec![HirParam { name: source_name.clone(), ty: source_type.clone() }],
            vec![HirParam { name: operation_name.clone(), ty: HirType::F64 }],
            HirType::Bool,
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_set_state".into())),
                vec![HirExpr::Var(source_name.clone()), HirExpr::Var(operation_name)],
            )),
        );
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![HirExpr::Var(callbacks_name.clone()), HirExpr::Call(
                Box::new(HirExpr::Var("registerNativeCallbackGraph".into())), vec![state_update],
            )],
        )));
        let key_name = format!("__thaw_native_descriptor_key_{}", self.next_binding);
        self.next_binding += 1;
        let flags_name = format!("__thaw_native_descriptor_flags_{}", self.next_binding);
        self.next_binding += 1;
        let descriptor_callback = |operation: usize| {
            let mut params = vec![HirParam { name: key_name.clone(), ty: HirType::Str }];
            let (name, ret) = if operation != 0 {
                params.push(HirParam { name: flags_name.clone(), ty: HirType::F64 });
                (if operation == 1 {
                    "__thaw_object_set_property_flags"
                } else {
                    "__thaw_object_can_set_property_flags"
                }, HirType::Bool)
            } else { ("__thaw_object_property_flags", HirType::F64) };
            let mut arguments = vec![HirExpr::Var(source_name.clone()), HirExpr::Var(key_name.clone())];
            if operation != 0 { arguments.push(HirExpr::Var(flags_name.clone())); }
            HirExpr::Lambda(
                vec![HirParam { name: source_name.clone(), ty: source_type.clone() }],
                params, ret,
                Box::new(HirExpr::Call(Box::new(HirExpr::Var(name.into())), arguments)),
            )
        };
        for operation in 0..3 {
            body.push(HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_push".into())),
                vec![HirExpr::Var(callbacks_name.clone()), HirExpr::Call(
                    Box::new(HirExpr::Var("registerNativeCallbackGraph".into())),
                    vec![descriptor_callback(operation)],
                )],
            )));
        }
        body.extend(self.lower_ordered_fixed_field_statements(
            HirExpr::Var(source_name.clone()), fields, define_actions,
        )?);
        let callback_body = body.split_off(callback_start);
        let failure_name = format!("__thaw_native_projection_failure_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(failure_name.clone(), HirType::Str);
        body.push(HirStmt::Try(callback_body, failure_name.clone(), vec![
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_release_native_projection_callbacks".into())),
                vec![HirExpr::Var(callbacks_name.clone())],
            )),
            HirStmt::Throw(HirExpr::Var(failure_name)),
        ], None));
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_build_native_object_wrapper".into())),
            vec![
                HirExpr::Var(builder_name),
                HirExpr::Var(args_name),
                HirExpr::Var(callbacks_name),
            ],
        );
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_intern_native_object".into())),
            vec![
                HirExpr::Var(source_name.clone()),
                HirExpr::Lit(HirLit::Str(layout)),
                result,
            ],
        );
        body.push(HirStmt::Return(Some(result)));
        self.wrap_call_argument_bindings(
            HirExpr::Block(body), &[(source_name, source_type, value)],
        )
    }

    fn lower_native_accessor_definition(
        &mut self,
        property: &swc_ecma_ast::Prop,
        accessor_type: &HirType,
        label: &str,
    ) -> Result<HirExpr, String> {
        let (accessor_params, accessor_ret) = match accessor_type {
            HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
                (params, ret.as_ref().clone())
            }
            _ => return Err(format!("`{label}` accessor slot is not callable")),
        };
        let Some((receiver, forwarded)) = accessor_params.split_first() else {
            return Err(format!("`{label}` accessor slot has no receiver"));
        };
        if let swc_ecma_ast::Prop::Method(method) = property {
            return self.lower_object_function(
                &method.function,
                receiver.clone(),
                Some(accessor_type),
                true,
            );
        }
        let swc_ecma_ast::Prop::KeyValue(property) = property else {
            return Err(format!("`{label}` accessor must be a function value or method"));
        };
        if let Expr::Fn(function) = property.value.as_ref() {
            return self.lower_object_function(
                &function.function,
                receiver.clone(),
                Some(accessor_type),
                true,
            );
        }
        let callback = self.lower_expr(&property.value)?;
        let callback_type = self.infer_expr_type(&callback)?;
        let (callback_params, callback_ret) = match &callback_type {
            HirType::Function(params, ret) => (params.clone(), ret.as_ref().clone()),
            HirType::CallableFunction(params, _, None, ret) => {
                (params.clone(), ret.as_ref().clone())
            }
            _ => return Err(format!("`{label}` accessor must be callable")),
        };
        if callback_params.len() != forwarded.len() {
            return Err(format!(
                "`{label}` accessor expects {} argument(s), got {}",
                forwarded.len(),
                callback_params.len()
            ));
        }
        let callback_name = format!("__thaw_descriptor_accessor_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let mut params = Vec::with_capacity(accessor_params.len());
        for (index, ty) in accessor_params.iter().enumerate() {
            let name = format!("__thaw_descriptor_parameter_{}_{}", index, self.next_binding);
            self.scope.insert(name.clone(), ty.clone());
            params.push(HirParam { name, ty: ty.clone() });
        }
        self.next_binding += 1;
        let arguments = params[1..]
            .iter()
            .zip(&callback_params)
            .map(|(parameter, expected)| {
                self.coerce_to_declared(expected, HirExpr::Var(parameter.name.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let call = HirExpr::FunctionCallWithThis(
            Box::new(HirExpr::Var(callback_name.clone())),
            Box::new(HirExpr::Var(params[0].name.clone())),
            arguments,
            callback_params,
            callback_ret.clone(),
        );
        let body = if forwarded.is_empty() {
            self.coerce_to_declared(&accessor_ret, call)?
        } else {
            HirExpr::Block(vec![HirStmt::Expr(call), HirStmt::Return(None)])
        };
        let adapter = HirExpr::Lambda(
            vec![HirParam {
                name: callback_name.clone(),
                ty: callback_type.clone(),
            }],
            params,
            accessor_ret,
            Box::new(body),
        );
        self.wrap_call_argument_bindings(adapter, &[(callback_name, callback_type, callback)])
    }

    fn lower_native_property_descriptor(
        &mut self,
        target: HirExpr,
        target_type: &HirType,
        key: &str,
        descriptor: &swc_ecma_ast::ObjectLit,
        label: &str,
    ) -> Result<Vec<HirExpr>, String> {
        let mut value = None;
        let mut getter = None;
        let mut setter = None;
        for entry in &descriptor.props {
            let swc_ecma_ast::PropOrSpread::Prop(entry) = entry else {
                return Err(format!("`{label}` descriptor must not spread"));
            };
            let name = match entry.as_ref() {
                swc_ecma_ast::Prop::KeyValue(property) => match &property.key {
                    swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                    swc_ecma_ast::PropName::Str(value) => {
                        value.value.to_string_lossy().into_owned()
                    }
                    _ => continue,
                },
                swc_ecma_ast::Prop::Method(method) => match &method.key {
                    swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                    swc_ecma_ast::PropName::Str(value) => {
                        value.value.to_string_lossy().into_owned()
                    }
                    _ => continue,
                },
                _ => {
                    return Err(format!(
                        "`{label}` descriptor must use `key: value` entries or methods"
                    ))
                }
            };
            match name.as_str() {
                "value" => {
                    let swc_ecma_ast::Prop::KeyValue(property) = entry.as_ref() else {
                        return Err(format!("`{label}` descriptor value must be an expression"));
                    };
                    value = Some(property.value.as_ref());
                }
                "get" => getter = Some(entry.as_ref()),
                "set" => setter = Some(entry.as_ref()),
                "writable" | "enumerable" | "configurable" => {}
                _ => {}
            }
        }
        if value.is_some() && (getter.is_some() || setter.is_some()) {
            return Err(format!(
                "`{label}` descriptor cannot mix value and accessor entries"
            ));
        }
        match target_type {
            HirType::Object(fields) => {
                let Some((_, field_type)) = fields.iter().find(|(name, _)| name == key) else {
                    return Err(format!(
                        "`{label}` cannot add the new field `{key}` to a fixed object"
                    ));
                };
                let getter_name = format!("__thaw_getter_{key}");
                let setter_name = format!("__thaw_setter_{key}");
                let existing_getter = fields.iter().find(|(name, _)| name == &getter_name);
                let existing_setter = fields.iter().find(|(name, _)| name == &setter_name);
                if let Some(value) = value {
                    if existing_getter.is_some() || existing_setter.is_some() {
                        return Err(format!(
                            "`{label}` cannot convert accessor `{key}` to a data property on a fixed object"
                        ));
                    }
                    let value = self.lower_expr(value)?;
                    let value = self.coerce_to_declared(field_type, value)?;
                    return Ok(vec![HirExpr::PropAssign(
                        Box::new(target),
                        target_type.clone(),
                        key.into(),
                        Box::new(value),
                    )]);
                }
                let mut assignments = Vec::new();
                for (entry, hidden, existing) in [
                    (getter, getter_name, existing_getter),
                    (setter, setter_name, existing_setter),
                ] {
                    let Some(entry) = entry else { continue };
                    let Some((_, accessor_type)) = existing else {
                        return Err(format!(
                            "`{label}` cannot add an accessor slot to fixed field `{key}`"
                        ));
                    };
                    let value = self.lower_native_accessor_definition(
                        entry,
                        accessor_type,
                        label,
                    )?;
                    assignments.push(HirExpr::PropAssign(
                        Box::new(target.clone()),
                        target_type.clone(),
                        hidden,
                        Box::new(value),
                    ));
                }
                Ok(assignments)
            }
            HirType::Json => {
                if getter.is_some() || setter.is_some() {
                    return Err(format!(
                        "`{label}` accessors require a live dynamic object"
                    ));
                }
                let Some(value) = value else { return Ok(Vec::new()) };
                let value = self.lower_expr(value)?;
                let value_type = self.infer_expr_type(&value)?;
                Ok(vec![HirExpr::JsonSet(
                    Box::new(target),
                    Box::new(HirExpr::Lit(HirLit::Str(key.into()))),
                    Box::new(value),
                    value_type,
                    true,
                )])
            }
            other => Err(format!(
                "`{label}` currently requires a fixed object or JSON receiver, got {other:?}"
            )),
        }
    }

}

/// The ECMAScript well-known symbols (`Symbol.<name>`) that thaw recognizes
/// as computed member keys.
const WELL_KNOWN_SYMBOLS: &[&str] = &[
    "asyncDispose",
    "asyncIterator",
    "dispose",
    "hasInstance",
    "isConcatSpreadable",
    "iterator",
    "match",
    "matchAll",
    "replace",
    "search",
    "species",
    "split",
    "toPrimitive",
    "toStringTag",
    "unscopables",
];

fn well_known_symbol_name(name: &str) -> Option<&'static str> {
    WELL_KNOWN_SYMBOLS.iter().copied().find(|known| *known == name)
}

/// The distinctive sentinel a `Symbol.<name>` value lowers to. The
/// `\u{1f}` prefix cannot appear as an ordinary property-name literal in
/// source, so it can't collide with a real string key.
fn well_known_symbol_key(name: &str) -> String {
    format!("\u{1f}@@{name}")
}

/// Extracts a well-known symbol name from a computed member key expression
/// (`Symbol.iterator`, etc.).
fn well_known_symbol_from_expr(expr: &Expr) -> Option<&'static str> {
    match expr {
        Expr::Member(member) => {
            let Expr::Ident(object) = member.obj.as_ref() else {
                return None;
            };
            if object.sym != *"Symbol" {
                return None;
            }
            let MemberProp::Ident(property) = &member.prop else {
                return None;
            };
            well_known_symbol_name(property.sym.as_ref())
        }
        // `Symbol.iterator` in a computed key is normalized by thaw's
        // iterator-protocol support into this internal string before
        // lowering.
        Expr::Lit(swc_ecma_ast::Lit::Str(value)) => {
            well_known_symbol_name(value.value.to_string_lossy().strip_prefix("__thaw_symbol_")?)
        }
        _ => None,
    }
}

impl<'a> FnLowerer<'a> {
    /// A method's synthetic marker is type metadata, not a physical field
    /// used by its body. When all readable/writable receiver fields are an
    /// exact prefix of the real allocation, alias the original pointer under
    /// the method's narrow receiver type. The body then reads and writes the
    /// original slots. Other layouts retain the existing materialization
    /// path until they have an explicit projection.
    fn native_object_method_receiver(
        &mut self,
        source: HirExpr,
        actual: &HirType,
        fields: &[(Symbol, HirType)],
    ) -> (HirExpr, Option<(Symbol, HirType, HirExpr)>) {
        let marker = "__thaw_object_method_receiver";
        let prefix = fields.iter().take_while(|(name, _)| name.as_str() != marker)
            .collect::<Vec<_>>();
        let compatible = matches!(actual, HirType::Object(actual_fields)
            if fields.last().is_some_and(|(name, ty)| name == marker && *ty == HirType::Bool)
                && fields[..fields.len() - 1].iter().all(|(name, _)| name.as_str() != marker)
                && actual_fields.iter().all(|(name, _)| name.as_str() != marker)
                // Accessor callbacks can return the whole aliased receiver.
                && !actual_fields.iter().any(|(name, _)| is_hidden_accessor_field(name))
                && prefix.len() <= actual_fields.len()
                && prefix.iter().copied().eq(actual_fields.iter().take(prefix.len())));
        if compatible {
            let view_type = HirType::Object(fields.to_vec());
            let name = format!("__thaw_live_method_receiver_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), view_type.clone());
            return (HirExpr::Var(name.clone()), Some((name, view_type, source)));
        }
        let copied = fields.iter().map(|(name, _)| {
            if name == marker {
                (name.clone(), HirExpr::Lit(if fields.last().is_some_and(|(_, ty)| *ty == HirType::Bool) {
                    HirLit::Bool(false)
                } else {
                    HirLit::Undefined
                }))
            } else {
                (name.clone(), HirExpr::PropAccess(
                    Box::new(source.clone()), actual.clone(), name.clone()))
            }
        }).collect();
        (HirExpr::ObjectLit(copied), None)
    }

    /// If an object literal's compiled field table (`fields`) includes a
    /// `[Symbol.toPrimitive]` method (stored under `well_known_symbol_
    /// key`'s sentinel), builds a call to it with the given ECMAScript
    /// hint (`"number"`/`"string"`/`"default"`) and returns the call's
    /// raw `Json` result -- `Ok(None)` if the object has no such method,
    /// so a caller (a numeric/string coercion site) can fall back to its
    /// own default behavior. Reuses the exact "receiver" object-literal
    /// convention `invocations/calls.rs`'s own method-call dispatch
    /// already builds (a `__thaw_object_method_receiver`-marked first
    /// parameter, populated from the object's own current field values).
    fn invoke_object_to_primitive(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        hint: &str,
    ) -> Result<Option<HirExpr>, String> {
        let key = well_known_symbol_key("toPrimitive");
        let Some((_, method_ty)) = fields.iter().find(|(name, _)| *name == key) else {
            return Ok(None);
        };
        let HirType::Function(params, _) = method_ty.clone() else {
            return Ok(None);
        };
        let Some(HirType::Object(receiver_fields)) = params.into_iter().next() else {
            return Ok(None);
        };
        let object_type = HirType::Object(fields.to_vec());
        let name = format!("__thaw_to_primitive_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), object_type.clone());
        let bound = HirExpr::Var(name.clone());
        let method = HirExpr::PropAccess(Box::new(bound.clone()), object_type.clone(), key);
        let (this_arg, view_binding) = self.native_object_method_receiver(
            bound.clone(), &object_type, &receiver_fields,
        );
        let call = HirExpr::Call(
            Box::new(method),
            vec![this_arg, HirExpr::Lit(HirLit::Str(hint.to_string()))],
        );
        let mut bindings = vec![(name, object_type, value)];
        if let Some(binding) = view_binding { bindings.push(binding); }
        let result = self.wrap_call_argument_bindings(call, &bindings)?;
        Ok(Some(result))
    }

    /// The class-instance sibling of `invoke_object_to_primitive` above.
    /// A class instance's own `[Symbol.toPrimitive](hint)` method (mangled
    /// via `well_known_symbol_key`, same as an object literal's, by
    /// `static_class_member_name`'s module-wide computed-key rewrite --
    /// `classes/normalization.rs`) lives in `self.signatures` under
    /// `class_method_symbol`, not as an object field the way an object
    /// literal's does -- a class instance's own field table only holds its
    /// declared instance properties plus its identity marker
    /// (`class_name_from_type`). Calling it needs no receiver-copy object
    /// (unlike the object-literal case): a native class method's own
    /// calling convention (`invocations/calls.rs`'s `obj.method(args)`
    /// rewrite) passes the receiver value directly as the first argument.
    fn invoke_class_to_primitive(
        &mut self,
        value: HirExpr,
        object_type: &HirType,
        hint: &str,
    ) -> Result<Option<HirExpr>, String> {
        let Some(class_name) = class_name_from_type(object_type) else {
            return Ok(None);
        };
        let symbol = class_method_symbol(class_name, &well_known_symbol_key("toPrimitive"));
        let Some(signature) = self.signatures.get(&symbol).cloned() else {
            return Ok(None);
        };
        let call = HirExpr::Call(
            Box::new(HirExpr::FunctionRef(
                symbol,
                signature.params.clone(),
                signature.ret.clone(),
            )),
            vec![value, HirExpr::Lit(HirLit::Str(hint.to_string()))],
        );
        Ok(Some(call))
    }
}
