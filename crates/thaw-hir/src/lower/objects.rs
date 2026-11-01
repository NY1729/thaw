fn expected_object_fields(expected: &HirType) -> Option<&[(Symbol, HirType)]> {
    match expected {
        HirType::Object(fields) => Some(fields.as_slice()),
        HirType::Optional(payload)
        | HirType::Nullable(payload)
        | HirType::Nullish(payload) => match payload.as_ref() {
            HirType::Object(fields) => Some(fields.as_slice()),
            _ => None,
        },
        // A `T | T[]` parameter (for example hapi's route overload) gives a
        // literal one unambiguous contextual object shape. Keep ambiguous
        // unions of multiple object shapes on their normal inference path.
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
    }
}

impl<'a> FnLowerer<'a> {
    pub(super) fn object_method_receiver_fields(ty: &HirType) -> Option<&[(Symbol, HirType)]> {
        let HirType::NonArrowReceiver(inner) = ty else { return None };
        let HirType::Object(fields) = inner.as_ref() else { return None };
        fields.iter().any(|(name, _)| name == "__thaw_object_method_receiver")
            .then_some(fields.as_slice())
    }

    /// Whether an `any` object literal must be built as one live object: it has a method,
    /// accessor or well-known-symbol key, or a nested object literal that does.
    fn object_literal_needs_live_object(&self, object: &SwcObjectLit) -> bool {
        if has_dynamic_storage(object.span) { return true; }
        object.props.iter().any(|property| match property {
            PropOrSpread::Prop(property) => match property.as_ref() {
                Prop::Method(_) | Prop::Getter(_) | Prop::Setter(_) => true,
                Prop::KeyValue(KeyValueProp { key, value }) => {
                    self.is_compiler_symbol_property(key)
                        || matches!(value.as_ref(), Expr::Fn(_))
                        || matches!(value.as_ref(), Expr::Ident(name) if {
                            let name = self.resolve_binding(name.sym.as_ref());
                            self.scope.get(&name).is_some_and(|ty| matches!(
                                ty, HirType::Function(..) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..)))
                                || self.signatures.contains_key(&name)
                        })
                        || self.literal_holds_live_object(value)
                }
                _ => false,
            },
            _ => false,
        })
    }

    /// Whether an `any` literal (an object, or an array of them, at any depth) holds an object
    /// literal that must be built as a live object.
    pub(in crate::lower) fn literal_holds_live_object(&self, expr: &Expr) -> bool {
        match peel_parens(expr) {
            Expr::Object(object) => self.object_literal_needs_live_object(object),
            Expr::Array(array) => array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_none() && self.literal_holds_live_object(&element.expr)),
            _ => false,
        }
    }

    /// An `any` array literal that holds live objects: every element is an `any` value (a live
    /// object, a nested array of them, or a plain value coerced to `Json`), so the elements keep
    /// their `this` and identity. `None` when an element spreads or awaits (the typed path stays).
    pub(in crate::lower) fn lower_any_live_array(
        &mut self,
        array: &swc_ecma_ast::ArrayLit,
    ) -> Result<Option<HirExpr>, String> {
        let mut elements = Vec::with_capacity(array.elems.len());
        for element in &array.elems {
            let Some(element) = element else { return Ok(None) };
            if element.spread.is_some() {
                return Ok(None);
            }
            let value = match peel_parens(&element.expr) {
                Expr::Object(object) if self.object_literal_needs_live_object(object) => {
                    self.lower_dynamic_accessor_object_lit(object, true)?
                }
                Expr::Array(inner) if self.literal_holds_live_object(&element.expr) => {
                    match self.lower_any_live_array(inner)? {
                        Some(inner) => inner,
                        None => return Ok(None),
                    }
                }
                _ => self.lower_expr(&element.expr)?,
            };
            if contains_await(&value) {
                return Ok(None);
            }
            elements.push(self.coerce_to_declared(&HirType::Json, value)?);
        }
        let array = HirExpr::ArrayLit(elements);
        self.infer_expr_type(&array)?;
        Ok(Some(array))
    }
}

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
            PropName::Computed(computed) => {
                if well_known_symbol_from_expr(&computed.expr).is_some()
                    && self.builtin_symbol_member_name(&computed.expr).is_none()
                {
                    None
                } else {
                    self.builtin_symbol_member_name(&computed.expr)
                        .map(|name| {
                            if name == "iterator" {
                                "__thaw_symbol_iterator".to_string()
                            } else {
                                well_known_symbol_key(name)
                            }
                        })
                        .or_else(|| self.static_property_name(&computed.expr))
                }
            }
            _ => None,
        })
    }

    pub(super) fn builtin_symbol_member_name(&self, expression: &Expr) -> Option<&'static str> {
        let name = well_known_symbol_from_expr(expression)?;
        // `Symbol.iterator` is a compiler intrinsic only when the identifier resolves
        // to the ambient builtin. Check both lexical bindings and module declarations;
        // otherwise preserve ordinary user values named `Symbol`.
        let binding = self.resolve_binding("Symbol");
        if self.scope.contains_key(&binding)
            || self.signatures.contains_key(&binding)
            || self.interfaces.get(&binding).is_some_and(|ty| {
                matches!(ty, HirType::Object(fields)
                    if fields.first().is_some_and(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")))
            })
        {
            return None;
        }
        Some(name)
    }

    pub(super) fn is_compiler_symbol_property(&self, property: &PropName) -> bool {
        matches!(property, PropName::Computed(computed)
            if self.builtin_symbol_member_name(&computed.expr).is_some())
    }

    fn dynamic_operation_property_name(&self, property: &PropName) -> Option<String> {
        match property {
            PropName::Computed(computed) => self
                .builtin_symbol_member_name(&computed.expr)
                .map(well_known_symbol_key)
                .or_else(|| self.static_object_property_name(property)),
            _ => self.static_object_property_name(property),
        }
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
                let lowered = self.lower_object_method(method,
                    receiver(method_this_stays_in_prefix(&method.function, fields))?, expected)?;
                let lowered = name_function_value(
                    lowered, Some(name.to_string()), method.function.is_async, method.function.is_generator);
                Some(vec![(name, lowered)])
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
        let expression = swc_ecma_ast::FnExpr {
            ident: None,
            function: Box::new(function.clone()),
        };
        let arrow = function_expression_as_arrow(&expression)?;
        let uses_this = force_receiver || function_uses_this(function);
        let contextual = expected.and_then(|ty| callback_signature(ty, function.params.len()));
        let mut params = Vec::new();
        if uses_this {
            let receiver_name = format!("__thaw_object_this_{}", self.next_binding);
            self.next_binding += 1;
            self.non_arrow_receiver = Some((receiver, receiver_name));
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
                            .or_else(|| {
                                (self.implicit_any_callbacks && matches!(parameter.pat, Pat::Ident(_)))
                                    .then_some(HirType::JsValue)
                            })
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
        Ok(callable_aware_closure(&arrow.params, lowered))
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
        self.lower_dynamic_accessor_object_lit_with_expected_fields(obj_lit, live_json, None)
    }

    fn lower_dynamic_accessor_object_lit_with_expected_fields(
        &mut self,
        obj_lit: &SwcObjectLit,
        live_json: bool,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        let saved = std::mem::replace(&mut self.implicit_any_callbacks, true);
        let lowered = self.lower_dynamic_accessor_object_lit_inner(obj_lit, live_json, expected_fields);
        self.implicit_any_callbacks = saved;
        lowered
    }

    fn lower_dynamic_accessor_object_lit_inner(
        &mut self,
        obj_lit: &SwcObjectLit,
        live_json: bool,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        let proto_setters = obj_lit
            .props
            .iter()
            .filter(|property| matches!(property, PropOrSpread::Prop(prop)
                if matches!(prop.as_ref(), Prop::KeyValue(KeyValueProp { key, .. }) if is_proto_setter_key(key))))
            .count();
        if proto_setters > 1 {
            return Err(PROTO_SETTER_DUPLICATE.into());
        }
        let mut bindings = Vec::new();
        let mut kinds = Vec::new();
        let mut keys = Vec::new();
        let mut key_kinds = Vec::new();
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
                key_kinds.push(HirExpr::Lit(HirLit::Bool(false)));
                values.push(HirExpr::Var(name));
                continue;
            }
            let PropOrSpread::Prop(property) = property else {
                unreachable!()
            };
            let is_symbol_key = match property.as_ref() {
                Prop::KeyValue(KeyValueProp { key, .. }) => self.is_compiler_symbol_property(key),
                Prop::Method(property) => self.is_compiler_symbol_property(&property.key),
                Prop::Getter(property) => self.is_compiler_symbol_property(&property.key),
                Prop::Setter(property) => self.is_compiler_symbol_property(&property.key),
                _ => false,
            };
            let (kind, key, value, callback) = match property.as_ref() {
                Prop::KeyValue(KeyValueProp { key, value }) if is_proto_setter_key(key) => {
                    // Applied by the runtime at this position: `null` or an object installs
                    // the prototype, any other value is ignored (Node semantics).
                    let value = self.lower_object_lit_field_value(value, None)?;
                    let value = self.coerce_to_declared(&HirType::Json, value)?;
                    ("proto", HirExpr::Lit(HirLit::Str(String::new())), Some(value), None)
                }
                Prop::KeyValue(KeyValueProp { key, value }) => {
                    let expected_value = self
                        .static_object_property_name(key)
                        .and_then(|name| {
                            expected_fields.and_then(|fields| {
                                fields.iter().find_map(|(field, ty)| {
                                    (field == &name).then_some(ty)
                                })
                            })
                        });
                    let key = match self.dynamic_operation_property_name(key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = key else {
                                return Err("unsupported object literal key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                    };
                    // A function expression keeps its dynamic `this` exactly like
                    // method syntax, so it is built as a receiver-forwarding callback.
                    if let Expr::Fn(function) = value.as_ref() {
                        let callback = self.lower_object_function(
                            &function.function,
                            HirType::JsValue,
                            expected_value,
                            true,
                        )?;
                        ("method", key, None, Some(callback))
                    } else if let Expr::Object(inner) = value.as_ref() {
                        // A nested literal that itself has methods is a live object too.
                        let nested = if self.object_literal_needs_live_object(inner) {
                            self.lower_dynamic_accessor_object_lit_with_expected_fields(
                                inner,
                                true,
                                expected_value.and_then(expected_object_fields),
                            )?
                        } else {
                            self.lower_object_lit_field_value(value, expected_value)?
                        };
                        let nested = self.coerce_to_declared(&HirType::Json, nested)?;
                        ("data", key, Some(nested), None)
                    } else if let (Expr::Array(inner), true) = (peel_parens(value), self.literal_holds_live_object(value)) {
                        let array = match self.lower_any_live_array(inner)? {
                            Some(array) => array,
                            None => self.lower_object_lit_field_value(value, None)?,
                        };
                        let array = self.coerce_to_declared(&HirType::Json, array)?;
                        ("data", key, Some(array), None)
                    } else {
                        let value = self.lower_object_lit_field_value(value, expected_value)?;
                        let value = self.coerce_to_declared(&HirType::Json, value)?;
                        ("data", key, Some(value), None)
                    }
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
                    let expected_method = self
                        .static_object_property_name(&method.key)
                        .and_then(|name| {
                            expected_fields.and_then(|fields| {
                                fields.iter().find_map(|(field, ty)| {
                                    (field == &name).then_some(ty)
                                })
                            })
                        });
                    let key = match self.dynamic_operation_property_name(&method.key) {
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
                        expected_method,
                        true,
                    )?;
                    ("method", key, None, Some(callback))
                }
                Prop::Getter(getter) => {
                    let key = match self.dynamic_operation_property_name(&getter.key) {
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
                    let key = match self.dynamic_operation_property_name(&setter.key) {
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
            let mut adapted_callback = false;
            let callback = match callback {
                Some(callback) => {
                    let callback_type = self.infer_expr_type(&callback)?;
                    let (callback, adapted_type) =
                        self.adapt_callable_result_for_live_projection(callback, callback_type.clone())?;
                    adapted_callback = adapted_type != callback_type;
                    Some(callback)
                }
                None => None,
            };
            let key_name = format!("__thaw_dynamic_object_key_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(key_name.clone(), HirType::Str);
            bindings.push((key_name.clone(), HirType::Str, key));
            kinds.push(HirExpr::Lit(HirLit::Str(kind.into())));
            keys.push(HirExpr::Var(key_name));
            key_kinds.push(HirExpr::Lit(HirLit::Bool(is_symbol_key)));
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
                    Box::new(HirExpr::Var(if live_json || adapted_callback {
                        "registerNativeCallbackGraph".into()
                    } else {
                        "registerNativeCallback".into()
                    })),
                    vec![callback],
                ));
            }
        }
        let kinds = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(kinds))?;
        // Keep raw key strings out of the graph reviver: it upgrades
        // well-known-symbol sentinels in scalar strings before the separate
        // key-kind metadata can distinguish literal strings from Symbols.
        // JSON text starts with `[` and is parsed by the captured native
        // parser in the operation builder, so key contents stay ordinary
        // strings throughout this metadata hop.
        let keys = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(keys))?;
        let keys = HirExpr::Call(
            Box::new(HirExpr::Var("JSON.stringify".into())),
            vec![keys],
        );
        let keys = self.coerce_to_declared(&HirType::Json, keys)?;
        let key_kinds = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(key_kinds))?;
        let values = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(values))?;
        let arguments = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(vec![kinds, keys, key_kinds, values]),
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
        if expected.is_none() && self.implicit_any_callbacks {
            if let Some(lowered) = self.lower_implicit_any_callback(value) {
                return lowered;
            }
        }
        // `{ inc: o.inc }` borrows a receiver-aware method: the new object becomes its `this`.
        if let (None, Expr::Member(member)) = (expected, value) {
            return self.lower_member_read(member);
        }
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
            let lowered = self.lower_expr_with_expected_type(value, Some(expected))?;
            // A field typed through a back-edge is coerced right here, while the literal is
            // still an `ArrayLit`/`ObjectLit` that can adopt the named layout.
            return if expected.contains_recursive() {
                self.coerce_to_declared(expected, lowered)
            } else {
                Ok(lowered)
            };
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
                    let additions = enumerable_field_order(source_fields).into_iter()
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
                        let mut members = match &*existing {
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
                    for index in enumerable_field_order(fields) {
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
                    for index in enumerable_field_order(source_fields) {
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

    /// `new Map()`/`new Set()` elements of an `any` array literal cross as the Json collection
    /// wrapper (a typed `Array<Map>` has no Json encoding). `None` when no element is one.
    pub(in crate::lower) fn lower_any_array_with_collections(
        &mut self,
        array: &swc_ecma_ast::ArrayLit,
    ) -> Result<Option<HirExpr>, String> {
        let collection = |element: &Option<swc_ecma_ast::ExprOrSpread>| element.as_ref().is_some_and(|element| {
            element.spread.is_none() && match peel_parens(&element.expr) {
                Expr::New(new) => matches!(new.callee.as_ref(), Expr::Ident(name) if matches!(name.sym.as_ref(), "Map" | "Set")),
                // A variable holding a native Map/Set: it crosses as a live view.
                Expr::Ident(name) => {
                    let name = self.resolve_binding(name.sym.as_ref());
                    matches!(self.scope.get(&name), Some(HirType::Map(..) | HirType::Set(_)))
                }
                _ => false,
            }
        });
        if !array.elems.iter().any(collection)
            || !array.elems.iter().all(|element| element.as_ref().is_some_and(|element| element.spread.is_none()))
        {
            return Ok(None);
        }
        let mut values = Vec::new();
        for element in array.elems.iter().flatten() {
            let value = self.lower_expr(&element.expr)?;
            let nested = std::mem::replace(&mut self.alias_native_arrays, true);
            let converted = self.coerce_to_declared(&HirType::Json, value);
            self.alias_native_arrays = nested;
            values.push(converted?);
        }
        let literal = HirExpr::ArrayLit(values);
        self.infer_expr_type(&literal)?;
        Ok(Some(literal))
    }

    /// A native `Map`/`Set` stored in a field of an `any` literal crosses as the same Json snapshot
    /// wrapper `const m: any = new Map()` uses (the realm revives it as a JS collection).
    fn any_literal_collection_field(&mut self, value: HirExpr, any_literal: bool) -> Result<HirExpr, String> {
        if any_literal && matches!(self.infer_expr_type_inner(&value), Ok(HirType::Map(..) | HirType::Set(_))) {
            let nested = std::mem::replace(&mut self.alias_native_arrays, true);
            let converted = self.coerce_to_declared(&HirType::Json, value);
            self.alias_native_arrays = nested;
            return converted;
        }
        Ok(value)
    }

    fn lower_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
        expected_fields: Option<&[(Symbol, HirType)]>,
    ) -> Result<HirExpr, String> {
        let any_literal = std::mem::take(&mut self.any_object_literal);
        // `{__proto__: v}` sets the prototype (Node semantics). Only the live object builder
        // can apply it in source order, and its result reads inherited properties.
        if object_literal_has_proto_setter(obj_lit) {
            return self.lower_dynamic_accessor_object_lit_with_expected_fields(obj_lit, true, None);
        }
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
                        }) => self.static_object_property_name(&PropName::Computed(computed.clone())).is_none(),
                        Prop::Method(property) => {
                            self.is_compiler_symbol_property(&property.key)
                                || self.static_object_property_name(&property.key).is_none()
                        }
                        Prop::Getter(property) => {
                            self.is_compiler_symbol_property(&property.key)
                                || self.static_object_property_name(&property.key).is_none()
                        }
                        Prop::Setter(property) => {
                            self.is_compiler_symbol_property(&property.key)
                                || self.static_object_property_name(&property.key).is_none()
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
                        enumerable_field_order(source_fields)
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
                        let additions = enumerable_field_order(source_fields)
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
                        let lowered = match value.as_ref() {
                            Expr::Array(_) if any_literal => {
                                let array = match value.as_ref() {
                                    Expr::Array(literal) => self.lower_any_array_with_collections(literal)?,
                                    _ => None,
                                };
                                let array = match array {
                                    Some(array) => array,
                                    None => self.lower_object_lit_field_value(value, expected)?,
                                };
                                self.coerce_to_declared(&HirType::JsValue, array)?
                            }
                            nested => {
                                // `[..] as any[]` in an `any` literal is the same dynamic array.
                                let asserted = match split_type_assertion(nested) {
                                    Some((inner, ty)) if any_literal && matches!(peel_parens(inner), Expr::Array(_)) =>
                                        self.lower_any_asserted_literal(inner, ty, true)?,
                                    _ => None,
                                };
                                match asserted {
                                    Some(array) => self.coerce_to_declared(&HirType::JsValue, array)?,
                                    _ => {
                                        self.any_object_literal = any_literal && matches!(nested, Expr::Object(_));
                                        let lowered = self.lower_object_lit_field_value(value, expected);
                                        self.any_object_literal = false;
                                        self.any_literal_collection_field(lowered?, any_literal)?
                                    }
                                }
                            }
                        };
                        vec![(name, lowered)]
                    }
                    Prop::Shorthand(ident) => {
                        let lowered = self.lower_expr(&Expr::Ident(ident.clone()))?;
                        vec![(ident.sym.to_string(), self.any_literal_collection_field(lowered, any_literal)?)]
                    }
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
                // Keep `Bytes` (instead of the erased `number[]`) so `console.log({ b })` can
                // still tell a `Buffer` field apart (see `console_object_with_buffers`).
                let ty = match self.infer_expr_type_inner(&value) {
                    Ok(HirType::Bytes) => HirType::Bytes,
                    _ => self.infer_expr_type(&value)?,
                };
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
                    enumerable_field_order(source_fields)
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

    pub(in crate::lower) fn unreachable_value(ty: &HirType) -> Result<HirExpr, String> {
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
            HirType::I64 => Ok(HirExpr::Lit(HirLit::I64(0))),
            HirType::BigInt => Ok(HirExpr::Lit(HirLit::BigInt("0".into()))),
            HirType::Symbol => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_symbol_new".into())),
                vec![HirExpr::Lit(HirLit::Str(String::new()))],
            )),
            HirType::Bytes => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_alloc".into())),
                vec![HirExpr::Lit(HirLit::F64(0.0))],
            )),
            // `new Map()` / `new Set()` and the weak variants share the one allocation
            // intrinsic; the closure only types it (inference does not look inside).
            HirType::Map(..) | HirType::WeakMap(..) | HirType::Set(_) | HirType::WeakSet(_) => {
                Ok(HirExpr::TypedClosure(
                    ty.clone(),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_new".into())),
                        Vec::new(),
                    )),
                ))
            }
            // A promise that resolves with its own placeholder: the executor form every
            // other `Promise<T>` construction in this crate uses.
            HirType::Promise(resolved) => {
                let resolve = "__thaw_unreachable_resolve".to_string();
                let (resolve_params, resolve_args) = if matches!(resolved.as_ref(), HirType::Void) {
                    (Vec::new(), Vec::new())
                } else {
                    (vec![resolved.as_ref().clone()], vec![Self::unreachable_value(resolved)?])
                };
                let executor = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: resolve.clone(),
                        ty: HirType::Function(resolve_params, Box::new(HirType::Void)),
                    }],
                    HirType::Void,
                    Box::new(HirExpr::Call(Box::new(HirExpr::Var(resolve)), resolve_args)),
                );
                Ok(HirExpr::PromiseNew(Box::new(executor), resolved.as_ref().clone(), false, false))
            }
            // The literal's own inferred type would be an `Array` for equal member types,
            // so the closure states the tuple type (erased before codegen like `new Map()`).
            HirType::Tuple(members) => Ok(HirExpr::TypedClosure(
                ty.clone(),
                Box::new(HirExpr::ArrayLit(
                    members.iter().map(Self::unreachable_value).collect::<Result<Vec<_>, _>>()?,
                )),
            )),
            HirType::CallableFunction(params, _, None, result) => Ok(HirExpr::TypedClosure(
                ty.clone(),
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    params.iter().enumerate().map(|(index, param)| HirParam {
                        name: format!("__thaw_unreachable_parameter_{index}"),
                        ty: param.clone(),
                    }).collect(),
                    result.as_ref().clone(),
                    Box::new(Self::unreachable_value(result)?),
                )),
            )),
            HirType::FunctionWithThis(receiver, visible) => {
                let HirType::Function(params, result) = visible.as_ref() else {
                    return Err(format!(
                        "unreachable value of a `this`-taking function needs a plain visible signature, got {visible:?}"
                    ));
                };
                let mut lambda_params = vec![HirParam {
                    name: "__thaw_this".into(),
                    ty: receiver.as_ref().clone(),
                }];
                lambda_params.extend(params.iter().enumerate().map(|(index, param)| HirParam {
                    name: format!("__thaw_unreachable_parameter_{index}"),
                    ty: param.clone(),
                }));
                Ok(HirExpr::NonArrowFunction(Box::new(HirExpr::Lambda(
                    Vec::new(),
                    lambda_params,
                    result.as_ref().clone(),
                    Box::new(Self::unreachable_value(result)?),
                ))))
            }
            other => Err(format!(
                "cannot synthesize an unreachable placeholder value of type {other:?}"
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
                // A widened maybe-uninitialised field is read leniently.
                return Some(match ty {
                    HirType::Optional(payload) if is_uninit_class_field(fields, property) => {
                        payload.as_ref().clone()
                    }
                    HirType::Nullish(payload) if is_uninit_class_field(fields, property) => {
                        HirType::Nullable(payload.clone())
                    }
                    other => other.clone(),
                });
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
                "\u{1}TypeError\u{1}Cannot read properties of undefined (reading '{property}')"
            )))),
            Box::new(HirExpr::TypedClosure(ty.clone(), Box::new(Self::unreachable_value(ty)?))),
        ))
    }

    fn unwrap_required_optional_member(
        &mut self,
        obj: HirExpr,
        obj_ty: HirType,
        property: &str,
        reject_unnarrowed: bool,
    ) -> Result<(HirExpr, HirType), String> {
        let payload = match &obj_ty {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                payload.as_ref().clone()
            }
            _ => return Ok((obj, obj_ty)),
        };
        // Like `tsc` (strictNullChecks): a property read on a *named binding* whose type may be
        // `undefined`/`null` and that was not narrowed is a compile-time error. Receivers that are
        // not plain bindings (element reads, call results, ...) can only look optional because the
        // native types are more conservative than TypeScript's, so they keep a runtime
        // `TypeError` guard instead.
        if reject_unnarrowed {
            let absent = match &obj_ty {
                HirType::Optional(_) => "undefined",
                HirType::Nullable(_) => "null",
                _ => "null or undefined",
            };
            return Err(format!(
                "Object is possibly {absent}: cannot read property `{property}` of {obj_ty:?} before narrowing it"
            ));
        }
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
        Ok((
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
        ))
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

    /// String indexing has a possibly-undefined result, even when its value is
    /// saved before it is observed.
    pub(super) fn lower_string_index_optional(&mut self, expr: &Expr) -> Result<Option<HirExpr>, String> {
        let Expr::Member(member) = expr else { return Ok(None) };
        // `d[k] ?? x` / `d[k] === undefined` on a `Record<string, number>`: a missing key is
        // absent (`undefined`), not the element's zero value, so read it as an `Optional`.
        if matches!(member.obj.as_ref(), Expr::Ident(_)) {
            if let Some(HirType::Dictionary(element)) = self.peek_type_without_lowering(&member.obj) {
                if matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool) {
                    let key = match &member.prop {
                        MemberProp::Ident(property) => HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                        MemberProp::Computed(computed) => {
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                        _ => return Ok(None),
                    };
                    let obj = self.lower_member_receiver(&member.obj)?;
                    return self
                        .typed_dictionary_read(obj, key, &HirType::Optional(element))
                        .map(Some);
                }
            }
        }
        let MemberProp::Computed(computed) = &member.prop else { return Ok(None) };
        if !matches!(member.obj.as_ref(), Expr::Ident(_)) {
            return Ok(None);
        }
        if !matches!(self.peek_type_without_lowering(&member.obj), Some(HirType::Str | HirType::StrLiteral(_))) {
            return Ok(None);
        }
        let receiver = self.lower_member_receiver(&member.obj)?;
        let index = self.lower_expr(&computed.expr)?;
        if self.infer_expr_type(&index)? != HirType::F64 {
            return Ok(None);
        }
        self.lower_string_index_optional_from_values(member, receiver, index).map(Some)
    }

    fn lower_string_index_optional_from_values(
        &mut self,
        member: &MemberExpr,
        receiver: HirExpr,
        index: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver = self.coerce_to_declared(&HirType::Str, receiver)?;
        self.expect_type(&HirType::F64, &index, "string index")?;
        let receiver_name = format!("__thaw_str_index_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_str_index_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_str_index_length_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), HirType::Str);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(length_name.clone(), HirType::F64);
        let receiver_var = HirExpr::Var(receiver_name.clone());
        let index_var = HirExpr::Var(index_name.clone());
        let length_var = HirExpr::Var(length_name.clone());
        let at = self.lower_call(&CallExpr {
            span: member.span,
            ctxt: Default::default(),
            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                span: member.span,
                obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    receiver_name.clone().into(),
                    member.span,
                ))),
                prop: MemberProp::Ident(swc_ecma_ast::IdentName::new("charAt".into(), member.span)),
            }))),
            args: vec![swc_ecma_ast::ExprOrSpread {
                spread: None,
                expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    index_name.clone().into(),
                    member.span,
                ))),
            }],
            type_args: None,
        })?;
        let lower_bound = HirExpr::BinOp(
            BinOp::GtEq,
            Box::new(index_var.clone()),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        );
        let upper_bound = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(index_var.clone()),
            Box::new(length_var),
        );
        let integral = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(index_var.clone()),
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![index_var],
            )),
        );
        let in_range = self.lower_logical_expr(lower_bound, upper_bound, true)?;
        let test = self.lower_logical_expr(in_range, integral, true)?;
        let result = HirExpr::Conditional(
            Box::new(test),
            Box::new(HirExpr::OptionalSome(Box::new(at), HirType::Str)),
            Box::new(HirExpr::OptionalNone(HirType::Str)),
            HirType::Optional(Box::new(HirType::Str)),
        );
        let result = HirExpr::Block(vec![
            HirStmt::Let(
                length_name,
                HirType::F64,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_length".into())),
                    vec![receiver_var],
                ),
            ),
            HirStmt::Return(Some(result)),
        ]);
        self.wrap_call_argument_bindings(
            result,
            &[
                (receiver_name, HirType::Str, receiver),
                (index_name, HirType::F64, index),
            ],
        )
    }

    fn lower_string_key_read(
        &mut self,
        member: &MemberExpr,
        receiver: HirExpr,
        key: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver = self.coerce_to_declared(&HirType::Str, receiver)?;
        let key = self.coerce_to_declared(&HirType::Str, key)?;
        let receiver_name = format!("__thaw_str_key_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let key_name = format!("__thaw_str_key_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_str_key_index_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_str_key_length_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), HirType::Str);
        self.scope.insert(key_name.clone(), HirType::Str);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(length_name.clone(), HirType::F64);
        let receiver_var = HirExpr::Var(receiver_name.clone());
        let key_var = HirExpr::Var(key_name.clone());
        let index_var = HirExpr::Var(index_name.clone());
        let parsed_index = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_string_to_number".into())),
            vec![key_var.clone()],
        );
        let string_index = self.lower_string_index_optional_from_values(
            member,
            receiver_var.clone(),
            index_var.clone(),
        )?;
        let string_index = self.optional_read_as_json(string_index)?;
        let canonical_index = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(key_var.clone()),
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".into())),
                vec![index_var],
            )),
        );
        let undefined = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_undefined".into())),
            Vec::new(),
        );
        let string_property = HirExpr::Conditional(
            Box::new(canonical_index),
            Box::new(string_index),
            Box::new(undefined),
            HirType::Json,
        );
        let error_property = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_error_get_prop".into())),
            vec![receiver_var.clone(), key_var.clone()],
        );
        let result = HirExpr::Conditional(
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_is_error".into())),
                vec![receiver_var.clone()],
            )),
            Box::new(error_property),
            Box::new(HirExpr::Block(vec![
                HirStmt::Let(
                    length_name,
                    HirType::F64,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".into())),
                        vec![receiver_var],
                    ),
                ),
                HirStmt::Return(Some(string_property)),
            ])),
            HirType::Json,
        );
        self.wrap_call_argument_bindings(
            result,
            &[
                (receiver_name, HirType::Str, receiver),
                (key_name, HirType::Str, key),
                (index_name, HirType::F64, parsed_index),
            ],
        )
    }

    /// `console.log(record.missing)` must print `undefined` like Node, but a
    /// `Record<string, number|string|boolean>` read is statically the bare
    /// payload (a missing key would decode as NaN/""/false). For a console
    /// argument only, read through the `Optional` form so absence stays visible.
    pub(super) fn lower_console_dictionary_read(&mut self, expr: &Expr) -> Result<Option<HirExpr>, String> {
        let Expr::Member(member) = expr else { return Ok(None) };
        let Expr::Ident(object) = member.obj.as_ref() else { return Ok(None) };
        let Some(HirType::Dictionary(element)) = self.scope.get(&self.resolve_binding(object.sym.as_ref())).cloned() else {
            return Ok(None);
        };
        if !matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool) {
            return Ok(None);
        }
        let key = match &member.prop {
            MemberProp::Ident(prop) => HirExpr::Lit(HirLit::Str(prop.sym.to_string())),
            MemberProp::Computed(computed) => {
                let key = self.lower_expr(&computed.expr)?;
                if self.infer_expr_type(&key)? != HirType::Str {
                    return Ok(None);
                }
                key
            }
            _ => return Ok(None),
        };
        let object = self.lower_expr(&member.obj)?;
        let optional = HirType::Optional(element);
        self.typed_dictionary_read(object, key, &optional).map(Some)
    }

    /// A dictionary read whose value is later observed as `undefined` (`typeof v`, `!v`,
    /// `v === undefined`) keeps a missing key absent instead of decoding it to ""/NaN/false.
    fn dictionary_read_observing_absence(
        &mut self,
        span: swc_common::Span,
        object: HirExpr,
        key: HirExpr,
        element: &HirType,
    ) -> Result<HirExpr, String> {
        if is_observed_member(span) && matches!(element, HirType::F64 | HirType::Str | HirType::Bool) {
            return self.typed_dictionary_read(object, key, &HirType::Optional(Box::new(element.clone())));
        }
        self.typed_dictionary_read(object, key, element)
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
        // An `undefined` dictionary (a regex result's `groups` without named groups) has no keys.
        let has_own = HirExpr::Conditional(
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                vec![HirExpr::TypedClosure(HirType::Json, Box::new(HirExpr::Var(object_name.clone())))],
            )),
            Box::new(HirExpr::Lit(HirLit::Bool(false))),
            Box::new(has_own),
            HirType::Bool,
        );
        let body = match element {
            // A stored `undefined` (e.g. an unmatched named regex group) is the
            // absent state of an `Optional`, same as a missing key.
            HirType::Optional(payload) => {
                let none = || HirStmt::Return(Some(HirExpr::OptionalNone(payload.as_ref().clone())));
                HirExpr::Block(vec![HirStmt::If(
                    has_own,
                    vec![HirStmt::If(
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_undefined".into())),
                            vec![raw.clone()],
                        ),
                        vec![none()],
                        vec![HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(Self::dictionary_value_from_json(raw, payload)?),
                            payload.as_ref().clone(),
                        )))],
                    )],
                    vec![none()],
                )])
            }
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
        let mut receiver = self.lower_member_receiver(expr)?;
        // A builtin method reached on an `extends Map/Set/Array` instance (e.g. the result of a
        // chained `stack.reverse().join()`) runs on the wrapped native collection.
        if !matches!(property, "hasOwnProperty" | "isPrototypeOf" | "propertyIsEnumerable") {
            receiver = self.project_native_base(receiver)?;
        }
        let ty = self.infer_expr_type(&receiver)?;
        Ok(self.unwrap_required_optional_member(receiver, ty, property, self.receiver_requires_narrowing(expr))?.0)
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

    fn call_class_static_setter(
        &mut self,
        symbol: Symbol,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let value_type = self.infer_expr_type(&value)?;
        let value_name = format!("__thaw_static_setter_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(value_name.clone(), value_type.clone());
        let parameter_type = self.signatures.get(&symbol)
            .and_then(|signature| signature.params.first()).cloned()
            .ok_or_else(|| format!("static setter `{symbol}` has no value parameter"))?;
        let argument = self.coerce_to_declared(&parameter_type, HirExpr::Var(value_name.clone()))?;
        let result = HirExpr::Block(vec![
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var(symbol)),
                vec![argument],
            )),
            HirStmt::Return(Some(HirExpr::Var(value_name.clone()))),
        ]);
        self.wrap_call_argument_bindings(result, &[(value_name, value_type, value)])
    }

    fn call_class_instance_setter(
        &mut self,
        owner: &str,
        // `None` binds the owner's setter statically (`super.x = v`).
        property: Option<&str>,
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
        let parameter_type = self.signatures.get(&symbol)
            .and_then(|signature| signature.params.get(1)).cloned()
            .ok_or_else(|| format!("instance setter `{symbol}` has no value parameter"))?;
        let argument = self.coerce_to_declared(&parameter_type, HirExpr::Var(value_name.clone()))?;
        let mut call = HirExpr::Call(
            Box::new(HirExpr::Var(symbol)),
            vec![
                self.assert_class_accessor_receiver(HirExpr::Var(receiver_name.clone()), owner),
                argument.clone(),
            ],
        );
        for (derived, derived_type) in property
            .map(|property| self.virtual_accessor_overrides(owner, property, true))
            .unwrap_or_default()
        {
            let property = property.unwrap_or_default();
            let cast = HirExpr::TypedClosure(derived_type, Box::new(HirExpr::Var(receiver_name.clone())));
            call = HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_has_class_identity".into())),
                    vec![HirExpr::Var(receiver_name.clone()), HirExpr::Lit(HirLit::Str(derived.to_string()))],
                )),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var(class_setter_symbol(&derived, property, false))),
                    vec![self.assert_class_accessor_receiver(cast, &derived), argument.clone()],
                )),
                Box::new(call),
                HirType::Void,
            );
        }
        let result = HirExpr::Block(vec![
            HirStmt::Expr(call),
            HirStmt::Return(Some(HirExpr::Var(value_name.clone()))),
        ]);
        self.wrap_call_argument_bindings(result, &[
            (receiver_name, receiver_type, receiver),
            (value_name, value_type, value),
        ])
    }

    /// `member` is `<expr>.constructor` and `<expr>` is a class instance: the lowered instance.
    fn class_instance_of_constructor(&mut self, member: &MemberExpr) -> Result<Option<HirExpr>, String> {
        if !matches!(&member.prop, MemberProp::Ident(p) if p.sym == *"constructor") {
            return Ok(None);
        }
        let is_class_name = matches!(member.obj.as_ref(), Expr::Ident(i)
            if self.signatures.contains_key(&class_constructor_symbol(i.sym.as_ref())));
        if is_class_name {
            return Ok(None);
        }
        let instance = self.lower_expr(&member.obj)?;
        let ty = self.infer_expr_type(&instance)?;
        let (instance, ty) = match &ty {
            HirType::Optional(payload) if matches!(payload.as_ref(), HirType::Object(_)) => {
                let strict = self.receiver_requires_narrowing(&member.obj);
                self.unwrap_required_optional_member(instance, ty.clone(), "constructor", strict)?
            }
            _ => (instance, ty),
        };
        if class_name_from_type(&ty).is_some() {
            return Ok(Some(instance));
        }
        // An interface-typed value: the class instance behind it (a view remembers it).
        if matches!(&ty, HirType::Object(fields) if !fields.is_empty()) {
            return self.view_unwrap(instance, &ty).map(Some);
        }
        Ok(None)
    }

    fn lower_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        // `counter.size` on a class extending `Map`/`Set`: read the wrapped
        // collection unless the class declares that property itself.
        if let MemberProp::Ident(property) = &member.prop {
            if property.sym.as_ref() != NATIVE_BASE_FIELD {
                if let Some(receiver_type) = self.infer_member_receiver_type(&member.obj) {
                    if native_base_type(&receiver_type).is_some()
                        && !Self::class_receiver_has_own_field(&receiver_type, property.sym.as_ref())
                        && self
                            .class_instance_accessor_owner(&receiver_type, property.sym.as_ref())
                            .is_none()
                    {
                        let mut forwarded = member.clone();
                        *forwarded.obj = native_base_member((*member.obj).clone());
                        return self.lower_member_read(&forwarded);
                    }
                }
            }
        }
        // `stack[i]` on a class extending `Array`: index the wrapped array.
        if matches!(member.prop, MemberProp::Computed(_)) {
            if let Some(receiver_type) = self.infer_member_receiver_type(&member.obj) {
                if matches!(native_base_type(&receiver_type), Some(HirType::Array(_))) {
                    let mut forwarded = member.clone();
                    *forwarded.obj = native_base_member((*member.obj).clone());
                    return self.lower_member_read(&forwarded);
                }
            }
        }
        if self.unbound_this_context && matches!(member.obj.as_ref(), Expr::This(_)) {
            let property = member_property_name(&member.prop)
                .ok_or("unbound `this` member access requires a statically known property")?;
            return self.lower_unbound_this_member(&property);
        }
        // `instance.constructor.name` for a class instance: its most-derived class's name.
        if let (MemberProp::Ident(name), Expr::Member(inner)) = (&member.prop, member.obj.as_ref()) {
            if name.sym == *"name" {
                if let Some(instance) = self.class_instance_of_constructor(inner)? {
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_class_name".into())),
                        vec![instance],
                    ));
                }
            }
        }
        // `f.length` on a declared function identifier: a compile-time
        // constant from its source parameter list. Anything else (aliases,
        // arrows, class constructors) has no `source_length` and falls
        // through to the existing rejection.
        if let (MemberProp::Ident(property), Expr::Ident(ident)) = (&member.prop, member.obj.as_ref()) {
            let name = self.resolve_binding(ident.sym.as_ref());
            if property.sym.as_ref() == "length"
                && !self.scope.contains_key(&name)
            {
                if let Some(length) = self.signatures.get(&name).and_then(|signature| signature.source_length) {
                    return Ok(HirExpr::Lit(HirLit::F64(length as f64)));
                }
            }
        }
        // `Symbol.<wellKnown>` as a value -- lowered to a distinctive
        // sentinel string a computed access/call can dispatch on. These are
        // not registered symbols (`Symbol.for`), so no runtime symbol is
        // created.
        if let Some(symbol) = self
            .builtin_symbol_member_name(&Expr::Member(member.clone()))
        {
            return Ok(HirExpr::Lit(HirLit::Str(well_known_symbol_key(symbol))));
        }
        // `obj[Symbol.toStringTag]` -- a compile-time tag per receiver type.
        if let MemberProp::Computed(computed) = &member.prop {
            if self.builtin_symbol_member_name(&computed.expr) == Some("toStringTag") {
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
                        HirType::I64 | HirType::BigInt => Some("BigInt"),
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
                    let mut index = self.lower_expr(&computed.expr)?;
                    if matches!(self.infer_expr_type(&index)?, HirType::Json | HirType::JsValue) {
                        // An enum value that travelled through `any` is a dynamic number.
                        index = self.coerce_primitive_to_number(index)?;
                    }
                    if self.infer_expr_type(&index)? == HirType::Str {
                        let object = enum_dynamic_index(enum_name.sym.as_str(), &computed.expr, self.enum_values, self.enum_reverse_values);
                        return self.lower_expr(&object);
                    }
                    self.expect_type(&HirType::F64, &index, "numeric enum reverse lookup")?;
                    return Ok(HirExpr::EnumReverseLookup(Box::new(index), entries.clone()));
                }
                if self
                    .enum_values
                    .keys()
                    .any(|(candidate, _)| candidate == enum_name.sym.as_str())
                {
                    let index = self.lower_expr(&computed.expr)?;
                    if self.infer_expr_type(&index)? == HirType::Str {
                        let object = enum_dynamic_index(enum_name.sym.as_str(), &computed.expr, self.enum_values, self.enum_reverse_values);
                        return self.lower_expr(&object);
                    }
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
                            return Ok(self.call_class_instance_getter(&owner, &property, HirExpr::Var(binding)));
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
                            return Ok(self.call_class_instance_getter(&owner, &property, HirExpr::Var(binding)));
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
                    self.unwrap_required_optional_member(obj, obj_ty, "computed property", self.receiver_requires_narrowing(&member.obj))?;
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
                        // The read is observed as possibly `undefined`: check bounds and holes.
                        if !tagged
                            && is_observed_member(member.span)
                            && matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool | HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_))
                        {
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
                        // A constant numeric key (a literal or an enum member) names a property.
                        let constant_number = match computed.expr.as_ref() {
                            Expr::Lit(Lit::Num(number)) => Some(number.value),
                            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                                (Expr::Ident(owner), MemberProp::Ident(property)) => {
                                    match self.enum_values.get(&(self.resolve_binding(owner.sym.as_ref()), property.sym.to_string())) {
                                        Some(HirLit::F64(number)) => Some(*number),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(number) = constant_number {
                            let key = if number.fract() == 0.0 { format!("{}", number as i64) } else { number.to_string() };
                            if fields.iter().any(|(name, _)| name == &key) {
                                return self.lower_fixed_object_property_read(obj, &fields, &key);
                            }
                        }
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
                                        | HirType::I64 | HirType::BigInt
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
                            let key = if self.infer_expr_type(&key)? == HirType::F64 { self.coerce_primitive_to_string(key)? } else { key };
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
                        let key = if self.infer_expr_type(&key)? == HirType::F64 { self.coerce_primitive_to_string(key)? } else { key };
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
                        self.dictionary_read_observing_absence(member.span, obj, key, element.as_ref())
                    }
                    HirType::Json => {
                        let key = self.lower_expr(&computed.expr)?;
                        let key_type = self.infer_expr_type(&key)?;
                        if !matches!(obj, HirExpr::NonArrowThis { .. }) {
                            return match key_type {
                                HirType::Str | HirType::Symbol => {
                                    Ok(HirExpr::JsonKey(Box::new(obj), Box::new(key)))
                                }
                                HirType::F64 => {
                                    Ok(HirExpr::JsonIndex(Box::new(obj), Box::new(key)))
                                }
                                other => Err(format!(
                                    "JSON index expression must be string or number, got {other:?}"
                                )),
                            };
                        }
                        let suffix = self.next_binding;
                        self.next_binding += 1;
                        let receiver = format!("__thaw_computed_receiver_{suffix}");
                        let key_name = format!("__thaw_computed_key_{suffix}");
                        let read = match key_type {
                            // A symbol is stored under its private string token.
                            HirType::Str | HirType::Symbol => {
                                HirExpr::JsonKey(Box::new(HirExpr::Var(receiver.clone())), Box::new(HirExpr::Var(key_name.clone())))
                            }
                            HirType::F64 => {
                                HirExpr::JsonIndex(Box::new(HirExpr::Var(receiver.clone())), Box::new(HirExpr::Var(key_name.clone())))
                            }
                            other => return Err(format!(
                                "JSON index expression must be string or number, got {other:?}"
                            )),
                        };
                        let receiver_var = HirExpr::Var(receiver.clone());
                        let key_var = HirExpr::Var(key_name.clone());
                        let property = if key_type == HirType::Str {
                            key_var.clone()
                        } else {
                            self.coerce_primitive_to_string(key_var.clone())?
                        };
                        let error = |kind: &str| HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".into())),
                            vec![
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_concat".into())),
                                    vec![
                                        HirExpr::Lit(HirLit::Str(format!("\u{1}TypeError\u{1}Cannot read properties of {kind} (reading '"))),
                                        property.clone(),
                                    ],
                                ),
                                HirExpr::Lit(HirLit::Str("')".into())),
                            ],
                        );
                        let is_nullish = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                            vec![receiver_var.clone()],
                        );
                        let is_null = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                            vec![receiver_var.clone()],
                        );
                        let result = HirExpr::Block(vec![
                            HirStmt::If(is_nullish, vec![HirStmt::If(
                                is_null,
                                vec![HirStmt::Throw(error("null"))],
                                vec![HirStmt::Throw(error("undefined"))],
                            )], Vec::new()),
                            HirStmt::Return(Some(read)),
                        ]);
                        self.scope.insert(receiver.clone(), HirType::Json);
                        self.scope.insert(key_name.clone(), key_type.clone());
                        self.wrap_call_argument_bindings(
                            result,
                            &[(receiver, HirType::Json, obj), (key_name, key_type, key)],
                        )
                    }
                    HirType::JsValue | HirType::Dynamic => {
                        let key = self.lower_expr(&computed.expr)?;
                        let key = match self.infer_expr_type(&key)? {
                            HirType::Str | HirType::Symbol | HirType::Dynamic => key,
                            // A live `any` key (a Proxy trap's property parameter) is its string form.
                            HirType::F64 | HirType::JsValue => self.coerce_primitive_to_string(key)?,
                            other => {
                                return Err(format!(
                                    "dynamic value index expression must be string, number, or symbol, got {other:?}"
                                ))
                            }
                        };
                        self.lower_dynamic_value_property_read(obj, key)
                    }
                    HirType::Str | HirType::StrLiteral(_) => {
                        let key = self.lower_expr(&computed.expr)?;
                        match self.infer_expr_type(&key)? {
                            HirType::F64 => self.lower_string_index_optional_from_values(member, obj, key),
                            HirType::Str | HirType::StrLiteral(_) => {
                                self.lower_string_key_read(member, obj, key)
                            }
                            other => Err(format!("string index key must be a number or string, got {other:?}")),
                        }
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
                if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Error")
                    && prop.sym == *"stackTraceLimit"
                    && !self.scope.contains_key("Error")
                {
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_stack_trace_limit".to_string())),
                        Vec::new(),
                    ));
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
                        name => {
                            return match Self::math_function_value(name) {
                                Some(function) => self.lower_expr(&function),
                                None => Err(format!("unsupported Math property `{}`", prop.sym)),
                            };
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
                let obj = if matches!(&obj, HirExpr::NonArrowThis { .. }) {
                    obj.with_non_arrow_this_access(NonArrowThisAccess::Read(prop.sym.to_string()))
                } else {
                    obj
                };
                let obj_ty = match &obj {
                    HirExpr::NonArrowThis { declared, .. } => declared.clone(),
                    _ => self.infer_expr_type(&obj)?,
                };
                let (obj, obj_ty) =
                    self.unwrap_required_optional_member(obj, obj_ty, prop.sym.as_ref(), self.receiver_requires_narrowing(&member.obj))?;
                // An unwrapped optional buffer (`arr[i]` of a `Buffer[]`) reads its properties as an array.
                let obj_ty = match obj_ty {
                    HirType::Bytes => HirType::Array(Box::new(HirType::F64)),
                    other => other,
                };
                // Raw catch-bound strings can carry the legacy
                // `\u{5}<json>` property bag. Structured caught values must
                // continue to union-property lowering, where the active
                // member selects that bag or the live JavaScript object.
                if obj_ty != crate::caught_exception_carrier_type() {
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
                    HirType::Array(_)
                        if matches!(
                            prop.sym.as_ref(),
                            "byteLength" | "byteOffset" | "buffer" | "maxByteLength" | "resizable" | "detached"
                        ) && self.infer_expr_type_inner(&obj)? == HirType::Bytes =>
                    {
                        Ok(if prop.sym == *"byteLength" || prop.sym == *"maxByteLength" {
                            HirExpr::ArrayLen(Box::new(obj))
                        } else if prop.sym == *"resizable" {
                            HirExpr::EvalThen(Box::new(obj), Box::new(HirExpr::Lit(HirLit::Bool(false))))
                        } else if prop.sym == *"detached" {
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_kind".into())),
                                    vec![obj, HirExpr::Lit(HirLit::F64(2.0))],
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                            )
                        } else if prop.sym == *"buffer" {
                            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_buffer".into())), vec![obj])
                        } else {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_bytes_byte_offset".into())),
                                vec![obj],
                            )
                        })
                    }
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
                    HirType::Object(fields)
                        if prop.sym == *"stack"
                            && !fields.iter().any(|(name, _)| {
                                name == "stack" || name == "__thaw_getter_stack"
                            }) =>
                    {
                        self.lower_object_stack_read(obj, fields)
                    }
                    HirType::Str if prop.sym == *"length" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".to_string())),
                        vec![obj],
                    )),
                    // `{ 0: x } = "str"` reads the one-character string at that
                    // index (out of range reads "", like a plain `s[i]`).
                    HirType::Str if prop.sym.parse::<u32>().is_ok_and(|n| n.to_string() == *prop.sym) => {
                        let index = prop.sym.parse::<f64>().unwrap_or(0.0);
                        Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_slice".to_string())),
                            vec![
                                obj,
                                HirExpr::Lit(HirLit::F64(index)),
                                HirExpr::Lit(HirLit::F64(index + 1.0)),
                            ],
                        ))
                    }
                    // `Symbol("x").description` -- the description part of
                    // the symbol's own runtime representation.
                    HirType::Symbol if prop.sym == *"description" => {
                        let name = format!("__thaw_symbol_description_raw_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        let raw = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_symbol_description".into())), vec![obj]);
                        let result = HirExpr::Conditional(
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![HirExpr::Var(name.clone())])),
                            Box::new(HirExpr::OptionalNone(HirType::Str)),
                            Box::new(HirExpr::OptionalSome(
                                Box::new(HirExpr::Var(name.clone())), HirType::Str)),
                            HirType::Optional(Box::new(HirType::Str)),
                        );
                        self.wrap_call_argument_bindings(result, &[(name, HirType::Str, raw)])
                    },
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
                    HirType::Str if prop.sym == *"constructor" => Ok(Self::error_constructor_value(obj)),
                    HirType::Str if prop.sym == *"message" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_message".to_string())),
                        vec![obj],
                    )),
                    HirType::Str if prop.sym == *"name" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_name".to_string())),
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
                    HirType::Str if prop.sym == *"stack" && PROGRAM_DELETES_STACK.with(Cell::get) => {
                        // The program deletes `stack` somewhere: an Error without its own `stack`
                        // reads `undefined`, so the read is `string | undefined` in this program.
                        let name = format!("__thaw_stack_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        let receiver = || HirExpr::Var(name.clone());
                        let call = |function: &str, args: Vec<HirExpr>| {
                            HirExpr::Call(Box::new(HirExpr::Var(function.into())), args)
                        };
                        let deleted = HirExpr::Conditional(
                            Box::new(call("__thaw_error_is_error", vec![receiver()])),
                            Box::new(HirExpr::Conditional(
                                Box::new(call(
                                    "__thaw_error_has_prop",
                                    vec![receiver(), HirExpr::Lit(HirLit::Str("stack".into()))],
                                )),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                HirType::Bool,
                            )),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                            HirType::Bool,
                        );
                        let result = HirExpr::Conditional(
                            Box::new(deleted),
                            Box::new(HirExpr::OptionalNone(HirType::Str)),
                            Box::new(HirExpr::OptionalSome(
                                Box::new(call("__thaw_error_stack_with_frames", vec![receiver()])),
                                HirType::Str,
                            )),
                            HirType::Optional(Box::new(HirType::Str)),
                        );
                        self.wrap_call_argument_bindings(result, &[(name, HirType::Str, obj)])
                    }
                    HirType::Str if prop.sym == *"stack" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_stack_with_frames".to_string())),
                        vec![obj],
                    )),
                    // Any other own property of an Error value (`err.status = 404` earlier): the
                    // runtime side table, `undefined` when absent (also for a plain string).
                    HirType::Str => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_error_get_prop".to_string())),
                        vec![obj, HirExpr::Lit(HirLit::Str(prop.sym.to_string()))],
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
                        let read = if self.union_narrowings.is_empty() {
                            self.lower_fixed_object_property_read(obj, fields, prop.sym.as_ref())?
                        } else {
                            self.lower_narrowed_fixed_object_property_read(
                                obj,
                                member,
                                fields,
                                prop.sym.as_ref(),
                            )?
                        };
                        // An `any` payload already holds `undefined`: it never needs the raw `Optional`.
                        let json_payload = matches!(&read, HirExpr::PropAccess(_, HirType::Object(fields), field)
                            if fields.iter().any(|(name, ty)| name == field
                                && matches!(ty, HirType::Optional(payload) if payload.as_ref() == &HirType::Json)));
                        let read = if (is_observed_member(member.span) || is_dynamic_read(member.span))
                            && matches!(&read, HirExpr::PropAccess(_, HirType::Object(fields), field)
                                if is_unset_number_field(fields, field))
                        {
                            self.unset_aware_number_read(read)?
                        } else {
                            read
                        };
                        if is_dynamic_read(member.span) && !json_payload {
                            // A sink that keeps `undefined`: the field's raw value as `Json`.
                            return self.optional_read_as_json(read);
                        }
                        if is_observed_member(member.span) && !json_payload {
                            Ok(read)
                        } else {
                            self.lenient_uninit_read(read)
                        }
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
                        let is_null = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                            vec![var.clone()],
                        );
                        let get = HirExpr::JsonGet(Box::new(var), prop.sym.to_string());
                        self.wrap_call_argument_bindings(
                            HirExpr::Block(vec![
                                HirStmt::If(
                                    is_nullish,
                                    vec![HirStmt::If(
                                        is_null,
                                        vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                                            "\u{1}TypeError\u{1}Cannot read properties of null (reading '{}')",
                                            prop.sym
                                        ))))],
                                        vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                                            "\u{1}TypeError\u{1}Cannot read properties of undefined (reading '{}')",
                                            prop.sym
                                        ))))],
                                    )],
                                    Vec::new(),
                                ),
                                HirStmt::Return(Some(get)),
                            ]),
                            &[(receiver, HirType::Json, obj)],
                        )
                    }
                    HirType::Dictionary(element) => self.dictionary_read_observing_absence(
                        member.span,
                        obj,
                        HirExpr::Lit(HirLit::Str(prop.sym.to_string())),
                        element.as_ref(),
                    ),
                    HirType::JsValue | HirType::Dynamic => {
                        let obj = if matches!(member.obj.as_ref(), Expr::This(_)) {
                            obj.with_non_arrow_this_access(NonArrowThisAccess::Read(
                                prop.sym.to_string(),
                            ))
                        } else {
                            obj
                        };
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
        // `view?.length` on an optional `extends Array/Map/Set` instance reads the wrapped collection.
        let (object, object_type) = match &object_type {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload)
                if native_base_type(payload).is_some() =>
            {
                let base = native_base_type(payload).cloned().expect("checked above");
                let name = format!("__thaw_optional_view_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), object_type.clone());
                let held = Box::new(HirExpr::Var(name.clone()));
                let payload_type = payload.as_ref().clone();
                let (absent, value, some, none, wrapped) = match &object_type {
                    HirType::Optional(_) => (
                        HirExpr::OptionalIsNone(held.clone(), payload_type.clone()),
                        HirExpr::OptionalValue(held, payload_type.clone()),
                        HirExpr::OptionalSome as fn(Box<HirExpr>, HirType) -> HirExpr,
                        HirExpr::OptionalNone(base.clone()),
                        HirType::Optional(Box::new(base.clone())),
                    ),
                    HirType::Nullable(_) => (
                        HirExpr::NullableIsNone(held.clone(), payload_type.clone()),
                        HirExpr::NullableValue(held, payload_type.clone()),
                        HirExpr::NullableSome as fn(Box<HirExpr>, HirType) -> HirExpr,
                        HirExpr::NullableNone(base.clone()),
                        HirType::Nullable(Box::new(base.clone())),
                    ),
                    _ => (
                        HirExpr::NullishIsNone(held.clone(), payload_type.clone()),
                        HirExpr::NullishValue(held, payload_type.clone()),
                        HirExpr::NullishSome as fn(Box<HirExpr>, HirType) -> HirExpr,
                        HirExpr::NullishNull(base.clone()),
                        HirType::Nullish(Box::new(base.clone())),
                    ),
                };
                let projected = HirExpr::PropAccess(Box::new(value), payload_type, NATIVE_BASE_FIELD.to_string());
                let conditional = HirExpr::Conditional(
                    Box::new(absent),
                    Box::new(none),
                    Box::new(some(Box::new(projected), base)),
                    wrapped.clone(),
                );
                let object = self.wrap_call_argument_bindings(conditional, &[(name, object_type, object)])?;
                (object, wrapped)
            }
            _ => (object, object_type),
        };
        // A live JavaScript value short-circuits the same way: read it through its dynamic form.
        let (object, object_type) = if object_type == HirType::JsValue {
            (self.coerce_to_declared(&HirType::Json, object)?, HirType::Json)
        } else {
            (object, object_type)
        };
        // A dynamic `any` value (`Json`) short-circuits to `undefined` for
        // a nullish receiver -- read the key directly, without the
        // TypeError guard the non-optional `Json` read now applies.
        if object_type == HirType::Json {
            let field = match &member.prop {
                MemberProp::Ident(field) => field.sym.to_string(),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(field)) => field.value.to_string_lossy().into_owned(),
                    _ => {
                        // `any?.[expr]`: the key is only evaluated for a non-nullish receiver.
                        let name = format!("__thaw_optional_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Json);
                        let mut rebound = member.clone();
                        *rebound.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                            name.clone().into(),
                            member.span,
                        ));
                        let read = self.lower_member_read(&rebound)?;
                        let read = self.coerce_to_declared(&HirType::Json, read)?;
                        let undefined =
                            self.coerce_to_declared(&HirType::Json, HirExpr::Lit(HirLit::Undefined))?;
                        let is_nullish = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                            vec![HirExpr::Var(name.clone())],
                        );
                        let value = HirExpr::Conditional(
                            Box::new(is_nullish),
                            Box::new(undefined),
                            Box::new(read),
                            HirType::Json,
                        );
                        return self.wrap_call_argument_bindings(
                            value,
                            &[(name, HirType::Json, object)],
                        );
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
            HirType::Map(..) | HirType::Set(_)
                if matches!(&member.prop, MemberProp::Ident(property) if property.sym == *"size") =>
            {
                (
                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_map_size".to_string())), vec![unwrapped]),
                    HirType::F64,
                )
            }
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
        // An Error subclass instance has no `stack` slot: it reads as `name: message` plus the
        // generated frame line, like a native Error's.
        if property == "stack"
            && !fields.iter().any(|(name, _)| name == property)
            && object_type_is_error_family(&HirType::Object(fields.to_vec()))
        {
            let framed = self.coerce_primitive_to_string(object)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_stack_with_frames".to_string())),
                vec![framed],
            ));
        }
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
                    Box::new(bound.clone().with_non_arrow_this_access(
                        NonArrowThisAccess::Read(property.to_string()),
                    )),
                    object_type.clone(),
                    getter,
                );
                let receiver_type = match self.infer_expr_type(&getter)? {
                    HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => params.first().cloned(),
                    _ => None,
                }.and_then(|ty| match ty { HirType::NonArrowReceiver(inner) => Some(*inner), _ => None })
                    .ok_or("object getter has no private receiver parameter")?;
                let read = HirExpr::Call(Box::new(getter), vec![HirExpr::NonArrowReceiverPack {
                    value: Box::new(bound), declared: receiver_type,
                }]);
                return self.wrap_call_argument_bindings(
                    read,
                    &[(receiver, object_type, object)],
                );
            }
            let getter = HirExpr::PropAccess(
                Box::new(object.clone().with_non_arrow_this_access(
                    NonArrowThisAccess::Read(property.to_string()),
                )),
                object_type,
                getter,
            );
            let getter_type = self.infer_expr_type(&getter)?;
            let receiver_type = match getter_type {
                HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => params.first().cloned(),
                _ => None,
            }.and_then(|ty| match ty { HirType::NonArrowReceiver(inner) => Some(*inner), _ => None })
                .ok_or("object getter has no private receiver parameter")?;
            return Ok(HirExpr::Call(Box::new(getter), vec![HirExpr::NonArrowReceiverPack {
                value: Box::new(object), declared: receiver_type,
            }]));
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
            Box::new(object.with_non_arrow_this_access(
                NonArrowThisAccess::Read(property.to_string()),
            )),
            object_type,
            property.to_string(),
        ))
    }

    /// A read of a class field widened to `Optional` because it may be
    /// uninitialised (`is_uninit_class_field`), in a value context: an absent
    /// field reads as NaN (number) or its zero payload, like Node's
    /// `undefined` coerced to the declared type. Other reads pass through.
    /// A raw (possibly `Optional`) field read as `Json`: `undefined` while absent.
    fn optional_read_as_json(&mut self, read: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&read)?;
        if ty == HirType::Str {
            let name = format!("__thaw_sink_read_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), HirType::Str);
            let var = HirExpr::Var(name.clone());
            let result = HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                    vec![var.clone()],
                )),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_undefined".into())),
                    Vec::new(),
                )),
                Box::new(self.coerce_to_declared(&HirType::Json, var)?),
                HirType::Json,
            );
            return self.wrap_call_argument_bindings(result, &[(name, HirType::Str, read)]);
        }
        let HirType::Optional(payload) = &ty else {
            return self.coerce_to_declared(&HirType::Json, read);
        };
        let payload = payload.as_ref().clone();
        let name = format!("__thaw_sink_read_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), ty.clone());
        let var = HirExpr::Var(name.clone());
        let present = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::OptionalValue(Box::new(var.clone()), payload.clone()),
        )?;
        let result = HirExpr::Conditional(
            Box::new(HirExpr::OptionalIsNone(Box::new(var), payload)),
            Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_undefined".into())), Vec::new())),
            Box::new(present),
            HirType::Json,
        );
        self.wrap_call_argument_bindings(result, &[(name, ty, read)])
    }

    /// A `number` read that may be the marker NaN of an unset class field: `undefined` then.
    fn unset_aware_number_read(&mut self, read: HirExpr) -> Result<HirExpr, String> {
        let name = format!("__thaw_unset_number_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), HirType::F64);
        let var = HirExpr::Var(name.clone());
        let result = HirExpr::Conditional(
            Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_is_unset_number".into())), vec![var.clone()])),
            Box::new(HirExpr::OptionalNone(HirType::F64)),
            Box::new(HirExpr::OptionalSome(Box::new(var), HirType::F64)),
            HirType::Optional(Box::new(HirType::F64)),
        );
        self.wrap_call_argument_bindings(result, &[(name, HirType::F64, read)])
    }

    fn lenient_uninit_read(&mut self, read: HirExpr) -> Result<HirExpr, String> {
        self.lenient_uninit_read_as(read, f64::NAN)
    }

    /// `lenient_uninit_read` with `unset` as the number an absent `number` field reads as.
    fn lenient_uninit_read_as(&mut self, read: HirExpr, unset: f64) -> Result<HirExpr, String> {
        let HirExpr::PropAccess(_, HirType::Object(fields), field) = &read else {
            return Ok(read);
        };
        if !is_uninit_class_field(fields, field) {
            return Ok(read);
        }
        let Some((_, widened)) = fields.iter().find(|(name, _)| name == field) else {
            return Ok(read);
        };
        if let HirType::Nullish(payload) = widened {
            // Widened from `T | null`: the lenient view is the declared
            // `T | null` with `undefined` folded into `null`.
            let payload = payload.as_ref().clone();
            let name = format!("__thaw_uninit_read_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), widened.clone());
            let var = HirExpr::Var(name.clone());
            let nullable = HirType::Nullable(Box::new(payload.clone()));
            let result = HirExpr::Conditional(
                Box::new(HirExpr::NullishIsNone(Box::new(var.clone()), payload.clone())),
                Box::new(HirExpr::NullableNone(payload.clone())),
                Box::new(HirExpr::NullableSome(
                    Box::new(HirExpr::NullishValue(Box::new(var), payload.clone())),
                    payload,
                )),
                nullable,
            );
            return self.wrap_call_argument_bindings(result, &[(name, widened.clone(), read)]);
        }
        let HirType::Optional(payload) = widened else {
            return Ok(read);
        };
        let payload = payload.as_ref().clone();
        if payload == HirType::Json {
            // An absent `any` field reads `undefined`, not a null handle.
            let name = format!("__thaw_uninit_read_{}", self.next_binding);
            self.next_binding += 1;
            let optional = HirType::Optional(Box::new(HirType::Json));
            self.scope.insert(name.clone(), optional.clone());
            let var = HirExpr::Var(name.clone());
            let result = HirExpr::Conditional(
                Box::new(HirExpr::OptionalIsNone(Box::new(var.clone()), HirType::Json)),
                Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_undefined".into())), Vec::new())),
                Box::new(HirExpr::OptionalValue(Box::new(var), HirType::Json)),
                HirType::Json,
            );
            return self.wrap_call_argument_bindings(result, &[(name, optional, read)]);
        }
        if payload != HirType::F64 {
            return Ok(HirExpr::OptionalValue(Box::new(read), payload));
        }
        let name = format!("__thaw_uninit_read_{}", self.next_binding);
        self.next_binding += 1;
        let optional = HirType::Optional(Box::new(HirType::F64));
        self.scope.insert(name.clone(), optional.clone());
        let var = HirExpr::Var(name.clone());
        let result = HirExpr::Conditional(
            Box::new(HirExpr::OptionalIsNone(Box::new(var.clone()), HirType::F64)),
            Box::new(HirExpr::Lit(HirLit::F64(unset))),
            Box::new(HirExpr::OptionalValue(Box::new(var), HirType::F64)),
            HirType::F64,
        );
        self.wrap_call_argument_bindings(result, &[(name, optional, read)])
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
            let hidden_is_getter = hidden.starts_with("__thaw_getter_");
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
            let receiver_argument = match params.first() {
                Some(HirType::NonArrowReceiver(inner)) => HirExpr::NonArrowReceiverPack {
                    value: Box::new(source.clone()),
                    declared: inner.as_ref().clone(),
                },
                _ => source.clone(),
            };
            let mut arguments = vec![receiver_argument];
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
            let label = format!("{} {property}", if hidden_is_getter { "get" } else { "set" });
            Ok(Some(name_function_value(
                HirExpr::Lambda(
                    vec![HirParam {
                        name: source_name.clone(),
                        ty: target_type.clone(),
                    }],
                    adapter_params,
                    ret,
                    Box::new(call),
                ),
                Some(label),
                false,
                false,
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
    pub(super) fn fixed_object_supports_live_projection(ty: &HirType) -> bool {
        Self::live_projection_supported(ty, &|_| false)
    }

    /// Like `fixed_object_supports_live_projection`, but a class instance
    /// nested in the layout is acceptable when it can itself be exposed as a
    /// live view that carries its methods.
    pub(super) fn live_projection_supported_with_classes(&self, ty: &HirType) -> bool {
        Self::live_projection_supported(ty, &|class| self.class_instance_supports_live_methods(class))
    }

    fn live_projection_supported(ty: &HirType, class_ok: &dyn Fn(&HirType) -> bool) -> bool {
        match ty {
            HirType::Object(fields) => {
                if fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")) {
                    return class_ok(ty);
                }
                fields.iter()
                    .filter(|(name, _)| !is_hidden_accessor_field(name))
                    .all(|(_, field)| Self::live_projection_supported(field, class_ok))
            }
            HirType::Function(params, _)
            | HirType::CallableFunction(params, _, _, _) =>
                !params.first().and_then(Self::object_method_receiver_fields).is_some_and(|receiver|
                    receiver.last().is_some_and(|(name, marker)|
                        name == "__thaw_object_method_receiver"
                            && !matches!(marker, HirType::Bool | HirType::Undefined))),
            HirType::Optional(inner) | HirType::Nullable(inner)
            | HirType::Nullish(inner) | HirType::Array(inner) =>
                Self::live_projection_supported(inner, class_ok),
            HirType::Union(members) | HirType::Tuple(members) =>
                members.iter().all(|member| Self::live_projection_supported(member, class_ok)),
            _ => true,
        }
    }

    /// A class instance with prototype methods can cross into `any` as a live
    /// view of the native owner that still carries those methods. Instances
    /// whose fields cannot all be projected keep the snapshot path.
    pub(super) fn class_instance_supports_live_methods(&self, ty: &HirType) -> bool {
        let HirType::Object(fields) = ty else { return false };
        let decorated = fields.first().is_some_and(|(marker, _)| {
            marker.strip_prefix("__thaw_class_identity_\u{1e}").is_some_and(|ancestry|
                DECORATED_CLASS_NAMES.with(|names| {
                    let names = names.borrow();
                    ancestry.split('\u{1f}').any(|class| names.contains(class))
                }))
        });
        let carries_identity = class_name_from_type(ty).is_some() && *ty != date_object_type();
        // A decorated instance's `constructor` must stay the registered class token, which the
        // live view's prototype provides; only values handed to ambient/npm code keep the snapshot.
        (!decorated || self.alias_native_arrays)
            && carries_identity
            && fields.iter()
                .filter(|(name, _)| !name.starts_with("__thaw_class_identity_\u{1e}")
                    && !is_hidden_accessor_field(name))
                .all(|(_, field)| self.live_projection_supported_with_classes(field))
    }

    /// Whether a fixed object needs the live accessor bridge to preserve a
    /// getter at any depth reachable through ordinary object/function/promise
    /// fields. Arrays and union wrappers are intentionally left on the existing
    /// snapshot path until their projection semantics are explicit.
    pub(super) fn fixed_object_needs_live_projection(fields: &[(Symbol, HirType)]) -> bool {
        fn needs(ty: &HirType) -> bool {
            match ty {
                HirType::Object(fields) => fields.iter().any(|(name, field)| {
                    is_hidden_accessor_field(name)
                        || (!is_hidden_accessor_field(name) && needs(field))
                }),
                HirType::Function(_, result) => needs(result),
                HirType::Promise(result) => needs(result),
                _ => false,
            }
        }
        fields.iter().any(|(name, field)| {
            is_hidden_accessor_field(name)
                || (!is_hidden_accessor_field(name) && needs(field))
        })
    }

    /// Keep callable result objects live across the QuickJS callback boundary.
    /// The projected handle is consumed into Json; callers use graph callbacks
    /// so decode releases the temporary graph lease after rebuilding the value.
    pub(super) fn adapt_callable_result_for_live_projection(
        &mut self,
        callable: HirExpr,
        callable_type: HirType,
    ) -> Result<(HirExpr, HirType), String> {
        let HirType::Function(params, result_type) = callable_type.clone() else {
            return Ok((callable, callable_type));
        };
        if !Self::callback_result_needs_live_projection(result_type.as_ref()) {
            return Ok((callable, callable_type));
        }
        let callable_name = format!("__thaw_live_result_callback_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(callable_name.clone(), callable_type.clone());
        let mut arguments = Vec::with_capacity(params.len());
        let mut wrapper_params = Vec::with_capacity(params.len());
        for (index, ty) in params.iter().enumerate() {
            let name = format!("__thaw_live_result_arg_{}_{}", index, self.next_binding);
            self.scope.insert(name.clone(), ty.clone());
            arguments.push(HirExpr::Var(name.clone()));
            wrapper_params.push(HirParam { name, ty: ty.clone() });
        }
        let call = HirExpr::Call(Box::new(HirExpr::Var(callable_name.clone())), arguments);
        let Some((body, projected_result_type)) =
            self.project_live_callback_result(call, result_type.as_ref())?
        else {
            return Ok((callable, callable_type));
        };
        let wrapper = HirExpr::Lambda(
            vec![HirParam { name: callable_name.clone(), ty: callable_type.clone() }],
            wrapper_params,
            projected_result_type.clone(),
            Box::new(body),
        );
        let wrapper_type = HirType::Function(params, Box::new(projected_result_type));
        let aliased = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_closure_alias".into())),
            vec![wrapper, HirExpr::Var(callable_name.clone())],
        );
        let wrapped = self.wrap_call_argument_bindings(
            aliased,
            &[(callable_name, callable_type, callable)],
        )?;
        Ok((wrapped, wrapper_type))
    }

    fn callback_result_needs_live_projection(result_type: &HirType) -> bool {
        match result_type {
            HirType::Object(fields) => {
                Self::fixed_object_needs_live_projection(fields)
                    && Self::fixed_object_supports_live_projection(result_type)
            }
            HirType::Promise(inner) => Self::callback_result_needs_live_projection(inner),
            _ => false,
        }
    }

    /// Projects only callback results whose declared fixed-object shape can
    /// preserve a getter lazily using the existing native-object bridge.
    fn project_live_callback_result(
        &mut self,
        value: HirExpr,
        result_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        match result_type {
            HirType::Object(fields)
                if Self::fixed_object_needs_live_projection(fields)
                    && Self::fixed_object_supports_live_projection(result_type) =>
            {
                let projected = self.lower_fixed_object_as_dynamic_accessor_object(
                    value, fields, false, true, false,
                )?;
                let json = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                    vec![projected],
                );
                Ok(Some((json, HirType::Json)))
            }
            HirType::Promise(inner) => {
                let parameter = format!("__thaw_live_result_promise_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(parameter.clone(), inner.as_ref().clone());
                let Some((body, projected_type)) = self.project_live_callback_result(
                    HirExpr::Var(parameter.clone()), inner,
                )? else {
                    return Ok(None);
                };
                let callback = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam { name: parameter, ty: inner.as_ref().clone() }],
                    projected_type.clone(),
                    Box::new(body),
                );
                Ok(Some((
                    HirExpr::PromiseThen(
                        Box::new(value), Box::new(callback), inner.as_ref().clone(),
                        projected_type.clone(), false, false,
                    ),
                    HirType::Promise(Box::new(projected_type)),
                )))
            }
            _ => Ok(None),
        }
    }

    /// Registers the native callback a live projection calls to read one
    /// property: `read` is evaluated with `__thaw_this` bound to the owner.
    fn projected_read_callback(
        &mut self,
        mut read: HirExpr,
        mut read_type: HirType,
        source_type: &HirType,
        this_name: &str,
    ) -> Result<HirExpr, String> {
        if Self::native_collection_view_supported(&read_type) {
            read = self.lower_native_collection_as_dynamic_view(read, &read_type)?;
            read_type = HirType::JsValue;
        }
        let original_read_type = read_type.clone();
        let (adapted_read, adapted_type) = self.adapt_callable_result_for_live_projection(
            read, read_type.clone(),
        )?;
        if adapted_type != original_read_type {
            let callback = HirExpr::Call(
                Box::new(HirExpr::Var("registerNativeCallbackGraph".into())),
                vec![HirExpr::TypedClosure(adapted_type, Box::new(adapted_read))],
            );
            read = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                vec![callback],
            );
            read_type = HirType::Json;
        } else {
            read = adapted_read;
            read_type = adapted_type;
        }
        let callback = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam { name: this_name.into(), ty: source_type.clone() }],
            read_type, Box::new(read),
        );
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_method_callback_graph".into())),
            vec![callback],
        ))
    }

    /// The instance methods a class instance exposes through a live
    /// projection (own and inherited, which each class symbol-table already
    /// carries): `(property, symbol)`. Overload-mangled names are skipped.
    pub(super) fn class_instance_projected_methods(
        &self,
        ty: &HirType,
    ) -> Vec<(String, Symbol)> {
        let Some(class) = class_name_from_type(ty) else { return Vec::new() };
        let prefix = class_method_symbol(class, "");
        let mut methods = self.signatures.keys()
            .filter_map(|symbol| {
                let name = symbol.strip_prefix(&prefix)?;
                (!name.is_empty() && !name.contains("__thaw") && !name.contains('\u{1e}'))
                    .then(|| (name.to_string(), symbol.clone()))
            })
            .collect::<Vec<_>>();
        methods.sort();
        methods
    }

    fn class_instance_projected_accessors(
        &self,
        ty: &HirType,
    ) -> Vec<(String, Option<Symbol>, Option<Symbol>)> {
        let properties = self.signatures.iter().filter_map(|(symbol, signature)| {
            let owner = signature.accessor_owner.as_deref()?;
            symbol.strip_prefix(&class_getter_symbol(owner, "", false))
                .or_else(|| symbol.strip_prefix(&class_setter_symbol(owner, "", false)))
                .map(str::to_string)
        }).collect::<std::collections::BTreeSet<_>>();
        properties.into_iter().filter_map(|property| {
            let owner = self.class_instance_accessor_owner(ty, &property)?;
            let owned = |symbol: Symbol| self.signatures.get(&symbol)
                .is_some_and(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str()))
                .then_some(symbol);
            let getter = owned(class_getter_symbol(&owner, &property, false));
            let setter = owned(class_setter_symbol(&owner, &property, false));
            Some((property, getter, setter))
        }).collect()
    }

    fn lower_fixed_object_as_dynamic_accessor_object(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        factory_body: bool,
        live: bool,
        class_methods: bool,
    ) -> Result<HirExpr, String> {
        // A class whose methods return its own type projects itself without end; report it
        // instead of overflowing the compiler's stack.
        if self.accessor_projection_depth >= 24 {
            return Err("a recursive class type (a method returning its own class, or a self-referential field) cannot be projected as a dynamic object".into());
        }
        self.accessor_projection_depth += 1;
        let result = self.lower_fixed_object_as_dynamic_accessor_object_inner(
            value, fields, factory_body, live, class_methods,
        );
        self.accessor_projection_depth -= 1;
        result
    }

    fn lower_fixed_object_as_dynamic_accessor_object_inner(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        factory_body: bool,
        live: bool,
        class_methods: bool,
    ) -> Result<HirExpr, String> {
        let source_type = HirType::Object(fields.to_vec());
        // A class receiver is entered through the ordinary callback ABI, so its
        // parameter must not carry the non-arrow `__thaw_this` name.
        let this_name = if class_name_from_type(&source_type).is_some() {
            "__thaw_class_owner"
        } else {
            "__thaw_this"
        };
        let proto_methods = if class_methods {
            self.class_instance_projected_methods(&source_type)
        } else {
            Vec::new()
        };
        let proto_accessors = if class_methods {
            self.class_instance_projected_accessors(&source_type)
        } else { Vec::new() };
        // Private slots stay hidden, but `#x in value` still needs to see them.
        let private_slots = if class_methods {
            fields.iter()
                .filter(|(name, _)| name.starts_with(PRIVATE_SLOT_PREFIX))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
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
        let methods_name = format!("__thaw_dynamic_methods_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(methods_name.clone(), HirType::Array(Box::new(HirType::Str)));
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
            let mut is_method = false;
            let getter = if setter_only {
                absent_callback()
            } else {
                let mut read = self.lower_fixed_object_property_read(
                    HirExpr::Var(this_name.into()), fields, name,
                )?;
                let mut read_type = Self::fixed_object_property_read_type(fields, name)?;
                if let HirType::Array(element) = &read_type {
                    // Under an `any` slot a native array field stays a live view.
                    if self.alias_native_arrays {
                        if let Ok(view) = self.lower_native_array_as_dynamic_view(read.clone(), element) {
                            read = view;
                            read_type = HirType::JsValue;
                        }
                    }
                }
                if let HirType::Object(nested) = &read_type {
                    if self.live_projection_supported_with_classes(&read_type) {
                        // Preserve nested native storage identity when its
                        // receiver layout is safe to expose as a live host;
                        // a nested class instance also keeps its methods.
                        let nested_class = nested.iter().any(|(name, _)|
                            name.starts_with("__thaw_class_identity_\u{1e}"));
                        read = self.lower_fixed_object_as_dynamic_accessor_object(read, nested, false, live || nested_class, nested_class)?;
                        read_type = HirType::JsValue;
                    }
                }
                // A receiver-aware method field: expose it as a function with the
                // receiver already bound to this live owner, so a host call
                // `view.method(...)` mutates the original storage.
                if let Some((bound, bound_type)) =
                    self.bind_projected_method_receiver(&read, &read_type, &source_type)?
                {
                    read = bound;
                    read_type = bound_type;
                    is_method = true;
                }
                self.projected_read_callback(read, read_type, &source_type, this_name)?
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
                        HirParam { name: this_name.into(), ty: source_type.clone() },
                        HirParam { name: value_name.clone(), ty: field_type },
                    ],
                    HirType::Void,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(HirExpr::Var(this_name.into())),
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
                append(&methods_name, HirExpr::Lit(HirLit::Str(if is_method { "method" } else { "" }.into()))),
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
                        HirParam { name: this_name.into(), ty: source_type.clone() },
                        HirParam { name: value_name.clone(), ty: field_type },
                        HirParam { name: flags_name.clone(), ty: HirType::F64 },
                    ],
                    HirType::Bool,
                    Box::new(HirExpr::Block(vec![
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_define_data_value".into())),
                            vec![
                                HirExpr::Var(this_name.into()),
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
            HirStmt::Let(methods_name.clone(), HirType::Array(Box::new(HirType::Str)),
                HirExpr::ArrayLit(Vec::new())),
        ]);
        body.extend(self.lower_ordered_fixed_field_statements(
            HirExpr::Var(source_name.clone()), fields, metadata_actions,
        )?);
        let push = |array: &str, value| HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![HirExpr::Var(array.to_string()), value],
        ));
        for property in proto_methods.iter().map(|(property, _)| property) {
            body.extend([
                push(&keys_name, HirExpr::Lit(HirLit::Str(property.clone()))),
                push(&readable_name, HirExpr::Lit(HirLit::Bool(true))),
                push(&writable_name, HirExpr::Lit(HirLit::Bool(false))),
                push(&accessor_name, HirExpr::Lit(HirLit::Bool(false))),
                push(&methods_name, HirExpr::Lit(HirLit::Str("method".into()))),
            ]);
        }
        for (property, getter, setter) in &proto_accessors {
            body.extend([
                push(&keys_name, HirExpr::Lit(HirLit::Str(property.clone()))),
                push(&readable_name, HirExpr::Lit(HirLit::Bool(getter.is_some()))),
                push(&writable_name, HirExpr::Lit(HirLit::Bool(setter.is_some()))),
                push(&accessor_name, HirExpr::Lit(HirLit::Bool(true))),
                push(&methods_name, HirExpr::Lit(HirLit::Str(self.signatures[
                    getter.as_ref().or(setter.as_ref()).unwrap()
                ].accessor_owner.clone().unwrap()))),
            ]);
        }
        for property in &private_slots {
            body.extend([
                push(&keys_name, HirExpr::Lit(HirLit::Str(property.clone()))),
                push(&readable_name, HirExpr::Lit(HirLit::Bool(true))),
                push(&writable_name, HirExpr::Lit(HirLit::Bool(false))),
                push(&accessor_name, HirExpr::Lit(HirLit::Bool(false))),
                push(&methods_name, HirExpr::Lit(HirLit::Str("method".into()))),
            ]);
        }
        // One entry past the last key: the wrapper is a live reference to a native owner
        // (not the projection of a fresh `any` literal, which converts to a mutable copy).
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![HirExpr::Var(accessor_name.clone()), HirExpr::Lit(HirLit::Bool(live))],
        )));
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
        let methods = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::Var(methods_name),
        )?;
        let mut argument_list = vec![keys, readable, writable, accessors, methods];
        if class_methods {
            // The class name tells the builder to expose the methods as
            // non-enumerable prototype members of a named class.
            // The whole ancestry (`Derived\u{1f}Base...`), most derived first.
            let ancestry = match &source_type {
                HirType::Object(fields) => fields.first()
                    .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
                    .unwrap_or("Object").to_string(),
                _ => "Object".to_string(),
            };
            argument_list.push(self.coerce_to_declared(
                &HirType::Json, HirExpr::Lit(HirLit::Str(ancestry)),
            )?);
        }
        let arguments = self.coerce_to_declared(
            &HirType::Json,
            HirExpr::ArrayLit(argument_list),
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
        // Keep the physical layout authoritative for callback receiver
        // recovery, including a later structural view in different order.
        // Registration checks arena ownership before any callback is acquired.
        body.push(HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_object_layout".into())),
            vec![HirExpr::Var(source_name.clone()), HirExpr::Lit(HirLit::Bool(true))],
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
        for (_, symbol) in &proto_methods {
            let signature = self.signatures.get(symbol).cloned()
                .ok_or_else(|| format!("class method `{symbol}` has no signature"))?;
            let Some((_, rest)) = signature.params.split_first() else { continue };
            let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
                HirType::Promise(Box::new(signature.ret.clone()))
            } else {
                signature.ret.clone()
            };
            let mut method_params = Vec::new();
            let mut arguments = vec![HirExpr::Var(this_name.into())];
            for (index, ty) in rest.iter().enumerate() {
                let name = format!("__thaw_class_method_arg_{}_{}", index, self.next_binding);
                self.scope.insert(name.clone(), ty.clone());
                arguments.push(HirExpr::Var(name.clone()));
                method_params.push(HirParam { name, ty: ty.clone() });
            }
            self.next_binding += 1;
            let mut method_body = HirExpr::Call(Box::new(HirExpr::Var(symbol.clone())), arguments);
            let mut result = result;
            // Under an `any` slot a native array a method hands back stays a live view of its
            // owner (`v.inner.get().push(x)` mutates `inner`'s array), as a field read does.
            if let (true, HirType::Array(element)) = (self.alias_native_arrays, &result) {
                if let Ok(view) = self.lower_native_array_as_dynamic_view(method_body.clone(), element) {
                    method_body = view;
                    result = HirType::JsValue;
                }
            }
            // ... and so does a native class instance / object a method hands back.
            if let (true, HirType::Object(nested)) = (self.alias_native_arrays, &result) {
                if self.live_projection_supported_with_classes(&result)
                    && !self.projecting_class_methods.contains(&result)
                    && result != source_type
                {
                    let nested_class = nested.iter().any(|(name, _)|
                        name.starts_with("__thaw_class_identity_\u{1e}"));
                    let nested = nested.clone();
                    self.projecting_class_methods.push(source_type.clone());
                    let projected = self.lower_fixed_object_as_dynamic_accessor_object(
                        method_body, &nested, false, true, nested_class);
                    self.projecting_class_methods.pop();
                    method_body = projected?;
                    result = HirType::JsValue;
                }
            }
            if self.alias_native_arrays && Self::native_collection_view_supported(&result) {
                method_body = self.lower_native_collection_as_dynamic_view(method_body, &result)?;
                result = HirType::JsValue;
            }
            let bound = HirExpr::Lambda(
                vec![HirParam { name: this_name.into(), ty: source_type.clone() }],
                method_params, result.clone(),
                Box::new(method_body),
            );
            let bound_type = match (&signature.native_rest, rest.last()) {
                (Some(element), Some(HirType::Array(last))) if last.as_ref() == element =>
                    HirType::CallableFunction(
                        rest[..rest.len() - 1].to_vec(), HirOptionalMask::default(),
                        Some(Box::new(element.clone())), Box::new(result),
                    ),
                _ => HirType::Function(rest.to_vec(), Box::new(result)),
            };
            let getter = self.projected_read_callback(bound, bound_type, &source_type, this_name)?;
            body.push(push(&callbacks_name, getter));
            body.push(push(&callbacks_name, HirExpr::Call(
                Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
            )));
        }
        for (_, getter, setter) in &proto_accessors {
            for symbol in [getter, setter] {
                let callback = if let Some(symbol) = symbol {
                    let signature = self.signatures[symbol].clone();
                    let mut params = vec![HirParam { name: this_name.into(), ty: source_type.clone() }];
                    let mut args = vec![HirExpr::Var(this_name.into())];
                    for (index, ty) in signature.params.iter().skip(1).enumerate() {
                        let name = format!("__thaw_accessor_arg_{}_{}", self.next_binding, index);
                        self.scope.insert(name.clone(), ty.clone());
                        args.push(HirExpr::Var(name.clone()));
                        params.push(HirParam { name, ty: ty.clone() });
                    }
                    self.next_binding += 1;
                    let call = HirExpr::Call(Box::new(HirExpr::Var(symbol.clone())), args);
                    let (result, result_type) = if signature.ret == HirType::Void {
                        (call, HirType::Void)
                    } else {
                        (self.coerce_to_declared_aliasing(&HirType::Json, call)?, HirType::Json)
                    };
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_register_native_method_callback_graph".into())),
                        vec![HirExpr::Lambda(Vec::new(), params, result_type, Box::new(result))],
                    )
                } else {
                    HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str("undefined".into()))])
                };
                body.push(push(&callbacks_name, callback));
            }
        }
        for _ in &private_slots {
            for _ in 0..2 {
                body.push(push(&callbacks_name, HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
                )));
            }
        }
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
        for _ in proto_methods.iter().map(|_| ()).chain(private_slots.iter().map(|_| ())) {
            body.push(push(&callbacks_name, HirExpr::Call(
                Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
            )));
        }
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

    /// Element types whose JS values can be written back into a native array.
    fn array_view_element_writable(element: &HirType) -> bool {
        match element {
            HirType::F64 | HirType::Str | HirType::Bool | HirType::Json | HirType::JsValue
            | HirType::Array(_) | HirType::Tuple(_) => true,
            // A class instance written from JS is stored through its native owner (see the
            // owner lookup in `lower_native_array_as_dynamic_view`); a plain copy would lose the class.
            HirType::Object(_) => true,
            _ => false,
        }
    }

    /// A native array crossing into `any` as a live view: the JS side reads and writes the
    /// native array through callbacks, so `push`, `length` and index writes are shared both ways.
    /// Returns a retained `JsValue` handle (same shape as the object projection).
    pub(super) fn lower_native_array_as_dynamic_view(
        &mut self,
        value: HirExpr,
        element: &HirType,
    ) -> Result<HirExpr, String> {
        let array_type = HirType::Array(Box::new(element.clone()));
        let id = self.next_binding;
        self.next_binding += 1;
        let source = format!("__thaw_array_view_source_{id}");
        let callbacks = format!("__thaw_array_view_callbacks_{id}");
        let cached = format!("__thaw_array_view_cached_{id}");
        let builder = format!("__thaw_array_view_builder_{id}");
        let args = format!("__thaw_array_view_args_{id}");
        let failure = format!("__thaw_array_view_failure_{id}");
        let index = format!("__thaw_array_view_index_{id}");
        let item = format!("__thaw_array_view_item_{id}");
        let count = format!("__thaw_array_view_count_{id}");
        self.scope.insert(source.clone(), array_type.clone());
        self.scope.insert(callbacks.clone(), HirType::Array(Box::new(HirType::JsValue)));
        self.scope.insert(index.clone(), HirType::F64);
        self.scope.insert(item.clone(), element.clone());
        self.scope.insert(count.clone(), HirType::F64);
        let writable = Self::array_view_element_writable(element);
        let layout = format!("array:{element:?}");
        let capture = || vec![HirParam { name: source.clone(), ty: array_type.clone() }];
        let source_var = || HirExpr::Var(source.clone());
        let register = |lambda: HirExpr| HirExpr::Call(
            Box::new(HirExpr::Var("registerNativeCallbackGraph".into())), vec![lambda]);
        let void_body = |expr: HirExpr| HirExpr::Block(vec![HirStmt::Expr(expr), HirStmt::Return(None)]);
        let length = register(HirExpr::Lambda(
            capture(), Vec::new(), HirType::F64, Box::new(HirExpr::ArrayLen(Box::new(source_var()))),
        ));
        let read = {
            let nested = std::mem::replace(&mut self.alias_native_arrays, true);
            let converted = self.coerce_to_declared(&HirType::Json, HirExpr::TypedIndex(
                Box::new(source_var()), Box::new(HirExpr::Var(index.clone())), element.clone(),
            ));
            self.alias_native_arrays = nested;
            // State 2 is an explicit `undefined` element (state 0, a hole, never reaches here).
            let explicit_undefined = HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_index_state".into())),
                    vec![source_var(), HirExpr::Var(index.clone())],
                )),
                Box::new(HirExpr::Lit(HirLit::F64(2.0))),
            );
            let json_undefined = || HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_undefined".into())), Vec::new());
            let mut converted = converted?;
            if *element == HirType::Str {
                // A null `Str` slot is a tagged template's invalid-escape cooked entry.
                converted = HirExpr::Conditional(
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                        vec![HirExpr::TypedIndex(
                            Box::new(source_var()), Box::new(HirExpr::Var(index.clone())), HirType::Str,
                        )],
                    )),
                    Box::new(json_undefined()),
                    Box::new(converted),
                    HirType::Json,
                );
            }
            // A nested flat array that still mirrors a Json array reads back as that Json array
            // (so `v[0] === inner` holds; see thaw-std `json/native_links.rs`).
            let readable_link = matches!(element, HirType::Array(_) | HirType::Object(_))
                && native_link_layout(element).is_some();
            if readable_link {
                let slot = || HirExpr::TypedIndex(
                    Box::new(source_var()), Box::new(HirExpr::Var(index.clone())), element.clone());
                converted = HirExpr::Conditional(
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_has_native_link".into())), vec![slot()])),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_linked_array".into())), vec![slot()])),
                    Box::new(converted),
                    HirType::Json,
                );
            }
            let body = HirExpr::Conditional(
                Box::new(explicit_undefined),
                Box::new(json_undefined()),
                Box::new(converted),
                HirType::Json,
            );
            register(HirExpr::Lambda(
                capture(), vec![HirParam { name: index.clone(), ty: HirType::F64 }],
                HirType::Json, Box::new(body),
            ))
        };
        let undefined = || HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
        );
        // Objects and arrays written back are first resolved to the native owner behind a live
        // view (so `v[1] = c` stores `c` itself); anything else is decoded as a copy, which a
        // class instance cannot be.
        let owner_layout = match element {
            HirType::Object(fields) => Some(crate::native_object_layout_token(fields)),
            HirType::Array(inner) => Some(format!("array:{inner:?}")),
            _ => None,
        };
        let (assign, push) = if let (true, Some(owner_layout)) = (writable, owner_layout) {
            let is_class = matches!(element, HirType::Object(fields) if fields.iter()
                .any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")));
            let item_js = format!("__thaw_array_view_value_{id}");
            let target = format!("__thaw_array_view_target_{id}");
            self.scope.insert(item_js.clone(), HirType::JsValue);
            self.scope.insert(target.clone(), HirType::Optional(Box::new(element.clone())));
            // Bind the fallback first: it may fail to lower for exotic element types.
            let fallback = if is_class { Some(HirExpr::Lit(HirLit::Bool(false))) } else { None };
            let decoded = self.coerce_to_declared(element, HirExpr::Var(item_js.clone())).ok();
            // A JS value with no native owner is adopted as a fresh instance of the class: its
            // own fields are copied into a record that carries the class identity.
            let decoded = match (is_class, decoded, element) {
                (true, Some(copy), HirType::Object(fields)) => {
                    let marker = fields[0].0.clone();
                    let copy_name = format!("__thaw_array_view_adopt_{id}");
                    self.scope.insert(copy_name.clone(), element.clone());
                    let record = HirExpr::ObjectLit(fields.iter().map(|(name, _)| {
                        if *name == marker {
                            (name.clone(), HirExpr::Lit(HirLit::Bool(true)))
                        } else {
                            (name.clone(), HirExpr::PropAccess(
                                Box::new(HirExpr::Var(copy_name.clone())), element.clone(), name.clone()))
                        }
                    }).collect());
                    let adopted = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_set_class_identity".into())),
                        vec![record, HirExpr::Lit(HirLit::Str(marker))],
                    );
                    self.wrap_call_argument_bindings(adopted, &[(copy_name, element.clone(), copy)]).ok()
                }
                (_, decoded, _) => decoded,
            };
            // A flat array of primitives written from a Json array stays linked to it, so
            // both sides keep sharing one array (see thaw-std `json/native_links.rs`).
            // `key\u{1f}offset\u{1f}kind` per field, `\u{1e}`-separated, for a record of primitive
            // 8-byte slots (any other shape keeps the plain copy).
            let object_link = match element {
                HirType::Object(fields) if !fields.is_empty() => fields.iter().enumerate()
                    .map(|(slot, (name, ty))| {
                        let kind = match ty {
                            HirType::F64 => 0,
                            HirType::Bool => 1,
                            HirType::Str => 2,
                            _ => return None,
                        };
                        (!name.starts_with("__thaw_") && !name.contains(['\u{1e}', '\u{1f}']))
                            .then(|| format!("{name}\u{1f}{}\u{1f}{kind}", slot * 8))
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|entries| entries.join("\u{1e}")),
                _ => None,
            };
            let link_kind = match element {
                HirType::Array(inner) => match inner.as_ref() {
                    HirType::F64 => Some(0.0),
                    HirType::Bool => Some(1.0),
                    HirType::Str => Some(2.0),
                    _ => None,
                },
                _ => None,
            };
            // Nested shapes (arrays of arrays, records with array/record fields) link recursively.
            let tree_layout = native_link_layout(element);
            let linked_copy = format!("__thaw_array_view_linked_{id}");
            self.scope.insert(linked_copy.clone(), element.clone());
            let resolve = |store: &dyn Fn(HirExpr) -> HirExpr| HirExpr::Block(vec![
                HirStmt::Let(target.clone(), HirType::Optional(Box::new(element.clone())),
                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_native_view_target".into())), vec![
                        HirExpr::Var(item_js.clone()),
                        HirExpr::Lit(HirLit::Str(owner_layout.clone())),
                        HirExpr::TypedIndex(Box::new(source_var()),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))), element.clone()),
                    ])),
                HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(HirExpr::Var(target.clone())), element.clone()),
                    match (&fallback, &decoded) {
                        (_, Some(copy)) if object_link.is_some() => vec![
                            HirStmt::Let(linked_copy.clone(), element.clone(), copy.clone()),
                            HirStmt::Expr(store(HirExpr::Var(linked_copy.clone()))),
                            HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_link_native_object".into())),
                                vec![
                                    HirExpr::Var(item_js.clone()),
                                    HirExpr::Var(linked_copy.clone()),
                                    HirExpr::Lit(HirLit::Str(object_link.clone().unwrap_or_default())),
                                ],
                            )),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                        ],
                        (_, Some(copy)) if link_kind.is_some() => vec![
                            HirStmt::Let(linked_copy.clone(), element.clone(), copy.clone()),
                            HirStmt::Expr(store(HirExpr::Var(linked_copy.clone()))),
                            HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_link_native".into())),
                                vec![
                                    HirExpr::Var(item_js.clone()),
                                    HirExpr::Var(linked_copy.clone()),
                                    HirExpr::Lit(HirLit::F64(link_kind.unwrap_or(0.0))),
                                ],
                            )),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                        ],
                        (_, Some(copy)) if tree_layout.is_some() => vec![
                            HirStmt::Let(linked_copy.clone(), element.clone(), copy.clone()),
                            HirStmt::Expr(store(HirExpr::Var(linked_copy.clone()))),
                            HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_link_native_tree".into())),
                                vec![
                                    HirExpr::Var(item_js.clone()),
                                    HirExpr::Var(linked_copy.clone()),
                                    HirExpr::Lit(HirLit::Str(tree_layout.clone().unwrap_or_default())),
                                ],
                            )),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                        ],
                        (_, Some(copy)) => vec![
                            HirStmt::Expr(store(copy.clone())),
                            // No link applies: the copy stays a copy (and says so once under
                            // `THAW_WARN_COPY=1`).
                            HirStmt::Expr(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_note_view_copy".into())), Vec::new())),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                        ],
                        _ => vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                    },
                    vec![
                        HirStmt::Expr(store(HirExpr::OptionalValue(
                            Box::new(HirExpr::Var(target.clone())), element.clone()))),
                        HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                    ],
                ),
            ]);
            let value_param = || HirParam { name: item_js.clone(), ty: HirType::JsValue };
            (
                register(HirExpr::Lambda(
                    capture(),
                    vec![HirParam { name: index.clone(), ty: HirType::F64 }, value_param()],
                    HirType::Bool,
                    Box::new(resolve(&|value| HirExpr::IndexAssign(
                        Box::new(source_var()), Box::new(HirExpr::Var(index.clone())), Box::new(value),
                    ))),
                )),
                register(HirExpr::Lambda(
                    capture(), vec![value_param()], HirType::Bool,
                    Box::new(resolve(&|value| HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".into())),
                        vec![source_var(), value],
                    ))),
                )),
            )
        } else if writable {
            let value_param = || HirParam { name: item.clone(), ty: element.clone() };
            (
                register(HirExpr::Lambda(
                    capture(),
                    vec![HirParam { name: index.clone(), ty: HirType::F64 }, value_param()],
                    HirType::Void,
                    Box::new(void_body(HirExpr::IndexAssign(
                        Box::new(source_var()), Box::new(HirExpr::Var(index.clone())),
                        Box::new(HirExpr::Var(item.clone())),
                    ))),
                )),
                register(HirExpr::Lambda(
                    capture(), vec![value_param()], HirType::Void,
                    Box::new(void_body(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".into())),
                        vec![source_var(), HirExpr::Var(item.clone())],
                    ))),
                )),
            )
        } else {
            (undefined(), undefined())
        };
        let resize = register(HirExpr::Lambda(
            capture(), vec![HirParam { name: count.clone(), ty: HirType::F64 }], HirType::Void,
            Box::new(void_body(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_resize".into())),
                vec![source_var(), HirExpr::Var(count.clone())],
            ))),
        ));
        let present = register(HirExpr::Lambda(
            capture(), vec![HirParam { name: index.clone(), ty: HirType::F64 }], HirType::Bool,
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_has_index".into())),
                vec![source_var(), HirExpr::Var(index.clone())],
            )),
        ));
        let hole = register(HirExpr::Lambda(
            capture(), vec![HirParam { name: index.clone(), ty: HirType::F64 }], HirType::Void,
            Box::new(void_body(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_set_hole".into())),
                vec![source_var(), HirExpr::Var(index.clone())],
            ))),
        ));
        // A tagged template's cooked strings keep their registered `.raw` sibling: the view
        // asks the runtime for it (an unregistered `string[]` yields an empty array, i.e. none).
        let raw = if *element == HirType::Str {
            let nested = std::mem::replace(&mut self.alias_native_arrays, false);
            let converted = self.coerce_to_declared(&HirType::Json, HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_template_strings_raw".into())), vec![source_var()],
            ));
            self.alias_native_arrays = nested;
            let converted = converted?;
            register(HirExpr::Lambda(capture(), Vec::new(), HirType::Json, Box::new(converted)))
        } else {
            undefined()
        };
        let cached_type = HirType::Optional(Box::new(HirType::JsValue));
        self.scope.insert(cached.clone(), cached_type.clone());
        self.scope.insert(args.clone(), HirType::Json);
        self.scope.insert(builder.clone(), HirType::JsValue);
        self.scope.insert(failure.clone(), HirType::Str);
        let push_callback = |callback: HirExpr| HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![HirExpr::Var(callbacks.clone()), callback],
        ));
        let arguments = self.coerce_to_declared(
            &HirType::Json, HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::Bool(writable))]),
        )?;
        let wrapper = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_build_native_object_wrapper".into())),
            vec![HirExpr::Var(builder.clone()), HirExpr::Var(args.clone()), HirExpr::Var(callbacks.clone())],
        );
        let interned = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_intern_native_object".into())),
            vec![source_var(), HirExpr::Lit(HirLit::Str(layout.clone())), wrapper],
        );
        let body = vec![
            HirStmt::Let(cached.clone(), cached_type,
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_lookup_native_object".into())),
                    vec![source_var(), HirExpr::Lit(HirLit::Str(layout))])),
            HirStmt::If(
                HirExpr::OptionalIsNone(Box::new(HirExpr::Var(cached.clone())), HirType::JsValue),
                Vec::new(),
                vec![HirStmt::Return(Some(HirExpr::OptionalValue(
                    Box::new(HirExpr::Var(cached)), HirType::JsValue)))],
            ),
            HirStmt::Let(callbacks.clone(), HirType::Array(Box::new(HirType::JsValue)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(args.clone(), HirType::Json, arguments),
            HirStmt::Let(builder.clone(), HirType::JsValue,
                HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("__thaw_array_with_native_view".into()))])),
            HirStmt::Try(
                [length, read, assign, push, resize, hole, present, raw].into_iter().map(push_callback).collect(),
                failure.clone(),
                vec![
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_release_native_projection_callbacks".into())),
                        vec![HirExpr::Var(callbacks.clone())],
                    )),
                    HirStmt::Throw(HirExpr::Var(failure)),
                ],
                None,
            ),
            HirStmt::Return(Some(interned)),
        ];
        self.wrap_call_argument_bindings(HirExpr::Block(body), &[(source, array_type, value)])
    }

    /// Map/Set shapes a live `any` view can serve: keys the native table can take from JS
    /// (`number`, `string`, `any`) and values that read back through the normal Json coercion.
    pub(super) fn native_collection_view_supported(ty: &HirType) -> bool {
        let key_ok = |key: &HirType| matches!(key, HirType::F64 | HirType::Str | HirType::Json);
        match ty {
            HirType::Map(key, value) => key_ok(key) && matches!(
                value.as_ref(),
                HirType::F64 | HirType::Str | HirType::Bool | HirType::Json | HirType::JsValue
                    | HirType::Array(_) | HirType::Object(_) | HirType::Dictionary(_)
            ),
            HirType::Set(element) => key_ok(element),
            _ => false,
        }
    }

    /// A native `Map`/`Set` crossing into `any` as a live view: the JS side reads and writes the
    /// native collection through callbacks (size, has, get, set/add, delete, clear, keys), so every
    /// mutation is shared both ways. Returns a retained `JsValue` handle interned per native pointer.
    pub(super) fn lower_native_collection_as_dynamic_view(
        &mut self,
        value: HirExpr,
        collection: &HirType,
    ) -> Result<HirExpr, String> {
        let (key_type, value_type) = match collection {
            HirType::Map(key, value) => (key.as_ref().clone(), Some(value.as_ref().clone())),
            HirType::Set(element) => (element.as_ref().clone(), None),
            other => return Err(format!("{other:?} is not a native collection")),
        };
        let key_suffix = map_key_intrinsic_suffix(&key_type)?;
        let id = self.next_binding;
        self.next_binding += 1;
        let source = format!("__thaw_collection_view_source_{id}");
        let callbacks = format!("__thaw_collection_view_callbacks_{id}");
        let cached = format!("__thaw_collection_view_cached_{id}");
        let builder = format!("__thaw_collection_view_builder_{id}");
        let args = format!("__thaw_collection_view_args_{id}");
        let failure = format!("__thaw_collection_view_failure_{id}");
        let key_name = format!("__thaw_collection_view_key_{id}");
        let item_name = format!("__thaw_collection_view_item_{id}");
        self.scope.insert(source.clone(), collection.clone());
        self.scope.insert(callbacks.clone(), HirType::Array(Box::new(HirType::JsValue)));
        self.scope.insert(key_name.clone(), key_type.clone());
        if let Some(value_type) = &value_type {
            self.scope.insert(item_name.clone(), value_type.clone());
        }
        let layout = format!("collection:{collection:?}");
        let capture = || vec![HirParam { name: source.clone(), ty: collection.clone() }];
        let source_var = || HirExpr::Var(source.clone());
        let key_var = || HirExpr::Var(key_name.clone());
        let key_param = || HirParam { name: key_name.clone(), ty: key_type.clone() };
        let register = |lambda: HirExpr| HirExpr::Call(
            Box::new(HirExpr::Var("registerNativeCallbackGraph".into())), vec![lambda]);
        let void_body = |expr: HirExpr| HirExpr::Block(vec![HirStmt::Expr(expr), HirStmt::Return(None)]);
        let undefined = || HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str("undefined".into()))],
        );
        let json_undefined = || HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_undefined".into())), Vec::new());
        let has_call = || HirExpr::Call(
            Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_has"))),
            vec![source_var(), key_var()],
        );
        let size = register(HirExpr::Lambda(
            capture(), Vec::new(), HirType::F64,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_map_size".into())), vec![source_var()])),
        ));
        let has = register(HirExpr::Lambda(
            capture(), vec![key_param()], HirType::Bool, Box::new(has_call()),
        ));
        let delete = register(HirExpr::Lambda(
            capture(), vec![key_param()], HirType::Bool,
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_delete"))),
                vec![source_var(), key_var()],
            )),
        ));
        let clear = register(HirExpr::Lambda(
            capture(), Vec::new(), HirType::Void,
            Box::new(void_body(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_map_clear".into())), vec![source_var()]))),
        ));
        // A snapshot of the keys (Set: the values) as a Json array; entries are read back one by
        // one with `get`, so a value that is itself a native owner stays a live view.
        let keys = {
            let keys_type = HirType::Array(Box::new(key_type.clone()));
            let snapshot = HirExpr::TypedClosure(
                keys_type,
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())), vec![source_var()])),
            );
            let nested = std::mem::replace(&mut self.alias_native_arrays, false);
            let converted = self.coerce_to_declared(&HirType::Json, snapshot);
            self.alias_native_arrays = nested;
            register(HirExpr::Lambda(
                capture(), Vec::new(), HirType::Json, Box::new(converted?),
            ))
        };
        let cursor_name = format!("__thaw_collection_view_cursor_{id}");
        let step_name = format!("__thaw_collection_view_step_{id}");
        let step_type = HirType::Tuple(vec![HirType::F64, key_type.clone()]);
        self.scope.insert(cursor_name.clone(), HirType::F64);
        self.scope.insert(step_name.clone(), step_type.clone());
        let step_var = || HirExpr::Var(step_name.clone());
        let empty = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(Vec::new()))?;
        let pair = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![
            HirExpr::TypedIndex(Box::new(step_var()), Box::new(HirExpr::Lit(HirLit::F64(0.0))), HirType::F64),
            HirExpr::TypedIndex(Box::new(step_var()), Box::new(HirExpr::Lit(HirLit::F64(1.0))), key_type.clone()),
        ]))?;
        let next_key = register(HirExpr::Lambda(
            capture(), vec![HirParam { name: cursor_name.clone(), ty: HirType::F64 }], HirType::Json,
            Box::new(HirExpr::Block(vec![
                HirStmt::Let(step_name.clone(), step_type.clone(), HirExpr::TypedClosure(step_type,
                    Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_map_iterator_next".into())),
                        vec![source_var(), HirExpr::Var(cursor_name), HirExpr::Lit(HirLit::F64(0.0))])))),
                HirStmt::Return(Some(HirExpr::Conditional(
                    Box::new(HirExpr::BinOp(BinOp::EqEqEq, Box::new(HirExpr::ArrayLen(Box::new(step_var()))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))))), Box::new(empty), Box::new(pair), HirType::Json))),
            ])),
        ));
        let (get, set, value_kind) = if let Some(value_type) = &value_type {
            let (value_suffix, needs_wrap) = map_value_get_suffix(value_type)?;
            let raw = HirExpr::Call(
                Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_get_{value_suffix}"))),
                vec![source_var(), key_var()],
            );
            let decoded = if needs_wrap {
                HirExpr::TypedClosure(value_type.clone(), Box::new(raw))
            } else {
                raw
            };
            let nested = std::mem::replace(&mut self.alias_native_arrays, true);
            let converted = self.coerce_to_declared(&HirType::Json, decoded);
            self.alias_native_arrays = nested;
            let get = register(HirExpr::Lambda(
                capture(), vec![key_param()], HirType::Json,
                Box::new(HirExpr::Conditional(
                    Box::new(has_call()), Box::new(converted?), Box::new(json_undefined()), HirType::Json,
                )),
            ));
            let value_kind = match value_type {
                HirType::F64 => "number",
                HirType::Str => "string",
                HirType::Bool => "boolean",
                HirType::Json | HirType::JsValue => "any",
                // Arrays, objects and dictionaries read back live but cannot be decoded from JS.
                _ => "none",
            };
            let set = if value_kind == "none" {
                undefined()
            } else {
                register(HirExpr::Lambda(
                    capture(),
                    vec![key_param(), HirParam { name: item_name.clone(), ty: value_type.clone() }],
                    HirType::Void,
                    Box::new(void_body(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![source_var(), key_var(), HirExpr::Var(item_name.clone())],
                    ))),
                ))
            };
            (get, set, value_kind)
        } else {
            let add = register(HirExpr::Lambda(
                capture(), vec![key_param()], HirType::Void,
                Box::new(void_body(HirExpr::Call(
                    Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                    vec![source_var(), key_var(), HirExpr::Lit(HirLit::F64(0.0))],
                ))),
            ));
            (undefined(), add, "none")
        };
        let key_kind = match &key_type {
            HirType::F64 => "number",
            HirType::Str => "string",
            _ => "any",
        };
        let cached_type = HirType::Optional(Box::new(HirType::JsValue));
        self.scope.insert(cached.clone(), cached_type.clone());
        self.scope.insert(args.clone(), HirType::Json);
        self.scope.insert(builder.clone(), HirType::JsValue);
        self.scope.insert(failure.clone(), HirType::Str);
        let push_callback = |callback: HirExpr| HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_array_push".into())),
            vec![HirExpr::Var(callbacks.clone()), callback],
        ));
        let arguments = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![
            HirExpr::Lit(HirLit::Bool(value_type.is_some())),
            HirExpr::Lit(HirLit::Str(key_kind.into())),
            HirExpr::Lit(HirLit::Str(value_kind.into())),
        ]))?;
        let wrapper = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_build_native_object_wrapper".into())),
            vec![HirExpr::Var(builder.clone()), HirExpr::Var(args.clone()), HirExpr::Var(callbacks.clone())],
        );
        let interned = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_intern_native_object".into())),
            vec![source_var(), HirExpr::Lit(HirLit::Str(layout.clone())), wrapper],
        );
        let body = vec![
            HirStmt::Let(cached.clone(), cached_type,
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_lookup_native_object".into())),
                    vec![source_var(), HirExpr::Lit(HirLit::Str(layout))])),
            HirStmt::If(
                HirExpr::OptionalIsNone(Box::new(HirExpr::Var(cached.clone())), HirType::JsValue),
                Vec::new(),
                vec![HirStmt::Return(Some(HirExpr::OptionalValue(
                    Box::new(HirExpr::Var(cached)), HirType::JsValue)))],
            ),
            HirStmt::Let(callbacks.clone(), HirType::Array(Box::new(HirType::JsValue)),
                HirExpr::ArrayLit(Vec::new())),
            HirStmt::Let(args.clone(), HirType::Json, arguments),
            HirStmt::Let(builder.clone(), HirType::JsValue,
                HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("__thaw_collection_with_native_view".into()))])),
            HirStmt::Try(
                [size, has, get, set, delete, clear, keys, next_key].into_iter().map(push_callback).collect(),
                failure.clone(),
                vec![
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_release_native_projection_callbacks".into())),
                        vec![HirExpr::Var(callbacks.clone())],
                    )),
                    HirStmt::Throw(HirExpr::Var(failure)),
                ],
                None,
            ),
            HirStmt::Return(Some(interned)),
        ];
        self.wrap_call_argument_bindings(HirExpr::Block(body), &[(source, collection.clone(), value)])
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
        let receiver = match receiver {
            HirType::NonArrowReceiver(inner) => inner.as_ref().clone(),
            other => other.clone(),
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
        let mut attribute_mask = 0u8;
        let mut attribute_bits = 0u8;
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
                "writable" | "enumerable" | "configurable" => {
                    // Only literal booleans are modelled; an absent attribute
                    // keeps the property's current flag.
                    if let swc_ecma_ast::Prop::KeyValue(property) = entry.as_ref() {
                        if let Expr::Lit(Lit::Bool(flag)) = property.value.as_ref() {
                            let bit = match name.as_str() {
                                "writable" => 1u8,
                                "enumerable" => 2,
                                _ => 4,
                            };
                            attribute_mask |= bit;
                            if flag.value { attribute_bits |= bit; }
                        }
                    }
                }
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
                // Descriptor attributes live on the owner (`thaw_object_*_property_flags`).
                // Validate the transition before any write, apply it after the value.
                let attribute_flags = |next: &mut Self, target: &HirExpr| -> (Vec<HirExpr>, Vec<HirExpr>) {
                    if attribute_mask == 0 || !key_is_plain_field(key) {
                        return (Vec::new(), Vec::new());
                    }
                    let _ = next;
                    let current = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_property_flags".into())),
                        vec![target.clone(), HirExpr::Lit(HirLit::Str(key.into()))],
                    );
                    let requested = HirExpr::BinOp(
                        BinOp::BitOr,
                        Box::new(HirExpr::BinOp(
                            BinOp::BitAnd,
                            Box::new(current),
                            Box::new(HirExpr::Lit(HirLit::F64(f64::from(7 - attribute_mask)))),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(f64::from(attribute_bits)))),
                    );
                    let call = |name: &str| HirExpr::Call(
                        Box::new(HirExpr::Var(name.into())),
                        vec![
                            target.clone(),
                            HirExpr::Lit(HirLit::Str(key.into())),
                            requested.clone(),
                        ],
                    );
                    let check = HirExpr::Block(vec![
                        HirStmt::If(
                            call("__thaw_object_can_set_property_flags"),
                            Vec::new(),
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                                "\u{1}TypeError\u{1}Cannot redefine property: {key}"
                            ))))],
                        ),
                        HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
                    ]);
                    (vec![check], vec![call("__thaw_object_set_property_flags")])
                };
                let (attribute_check, attribute_apply) = attribute_flags(self, &target);
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
                    // Only the plain `[PropAssign]` shape is special-cased by the
                    // `Reflect.defineProperty` caller; attributes widen it.
                    let mut assignments = attribute_check;
                    assignments.push(HirExpr::PropAssign(
                        Box::new(target),
                        target_type.clone(),
                        key.into(),
                        Box::new(value),
                    ));
                    assignments.extend(attribute_apply);
                    return Ok(assignments);
                }
                let mut assignments = attribute_check;
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
                assignments.extend(attribute_apply);
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

/// Hidden marker fields and accessor slots have no descriptor flags of their own.
/// Layout token of a tree of arrays/records of primitives that a Json container can stay linked
/// to (thaw-std `json/native_links.rs` `parse_kind`): `n`/`b`/`s`, `A<elem>`, and
/// `R<count>;` followed by `<keylen>:<key><offset>:<layout>` per field (8-byte slots).
fn native_link_layout(ty: &HirType) -> Option<String> {
    match ty {
        HirType::F64 => Some("n".into()),
        HirType::Bool => Some("b".into()),
        HirType::Str => Some("s".into()),
        HirType::Array(inner) => Some(format!("A{}", native_link_layout(inner)?)),
        HirType::Object(fields) if !fields.is_empty() => {
            let mut token = format!("R{};", fields.len());
            for (slot, (name, field)) in fields.iter().enumerate() {
                if name.starts_with("__thaw_") || name.contains('\0') {
                    return None;
                }
                token.push_str(&format!("{}:{}{}:{}", name.len(), name, slot * 8, native_link_layout(field)?));
            }
            Some(token)
        }
        _ => None,
    }
}

fn key_is_plain_field(key: &str) -> bool {
    !key.starts_with("__thaw_") && !key.starts_with('\u{1f}')
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
        _ => None,
    }
}

type NativeObjectMethodReceiver = (HirExpr, Option<(Symbol, HirType, HirExpr)>);

impl<'a> FnLowerer<'a> {
    /// A method's synthetic marker is type metadata, not a physical field
    /// used by its body. When all readable/writable receiver fields are an
    /// exact prefix of the real allocation, alias the original pointer under
    /// the method's narrow receiver type when its fields are an exact physical
    /// prefix of the owner. Other layouts retain the existing materialization
    /// path until they have an explicit projection.
    fn native_object_method_receiver(
        &mut self,
        source: HirExpr,
        actual: &HirType,
        fields: &[(Symbol, HirType)],
    ) -> Result<NativeObjectMethodReceiver, String> {
        let marker = "__thaw_object_method_receiver";
        let prefix = fields.iter().take_while(|(name, _)| name.as_str() != marker)
            .collect::<Vec<_>>();
        let compatible = matches!(actual, HirType::Object(actual_fields)
            if fields.last().is_some_and(|(name, ty)| name == marker
                    && matches!(ty, HirType::Bool | HirType::Undefined))
                && fields[..fields.len() - 1].iter().all(|(name, _)| name.as_str() != marker)
                && actual_fields.iter().all(|(name, _)| name.as_str() != marker)
                // An exact physical prefix shares the same slot offsets.
                && !actual_fields.iter().any(|(name, _)| is_hidden_accessor_field(name))
                && prefix.len() <= actual_fields.len()
                && prefix.iter().copied().eq(actual_fields.iter().take(prefix.len())));
        if compatible {
            let view_type = HirType::Object(fields.to_vec());
            let name = format!("__thaw_live_method_receiver_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), view_type.clone());
            let needs_full_value_projection = fields.last().is_some_and(|(name, ty)|
                name == marker && *ty == HirType::Undefined);
            let registered_source = if needs_full_value_projection && !self.lowering_native_projection_factory {
                if let HirType::Object(actual_fields) = actual {
                if Self::fixed_object_supports_live_projection(actual)
                    || self.live_projection_supported_with_classes(actual)
                {
                    self.retain_optional_native_this_projection(source, actual_fields)?
                } else {
                    source
                }
                } else { source }
            } else {
                source
            };
            return Ok((HirExpr::Var(name.clone()), Some((name, view_type, registered_source))));
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
        Ok((HirExpr::ObjectLit(copied), None))
    }

    /// A method whose body uses `this` as a value runs on a copy when the
    /// receiver cannot safely alias the native owner. After the call, store
    /// copied fields back and preserve owner identity when it returned the copy.
    fn copy_back_method_receiver(
        &mut self,
        call: HirExpr,
        owner: &str,
        owner_type: &HirType,
        copy: &str,
        view_type: &HirType,
        method_ret: &HirType,
    ) -> Result<HirExpr, String> {
        let (HirType::Object(owner_fields), HirType::Object(view_fields)) = (owner_type, view_type) else {
            return Ok(call);
        };
        let owner_var = || HirExpr::Var(owner.to_string());
        let returns_receiver = method_ret == view_type;
        let result_type = if returns_receiver { owner_type.clone() } else { method_ret.clone() };
        let mut statements = Vec::new();
        let result_name = format!("__thaw_method_result_{}", self.next_binding);
        self.next_binding += 1;
        if *method_ret == HirType::Void {
            statements.push(HirStmt::Expr(call));
        } else {
            statements.push(HirStmt::Let(result_name.clone(), method_ret.clone(), call));
        }
        // ponytail: the copy overwrites fields the method changed on the owner by another route (a captured alias) while it ran.
        // Accessor properties run setters on assignment: leave such owners alone.
        let has_accessors = owner_fields.iter().any(|(name, _)| is_hidden_accessor_field(name));
        for (name, ty) in view_fields.iter().filter(|_| !has_accessors) {
            let writable = name != "__thaw_object_method_receiver"
                && !matches!(ty, HirType::Function(..) | HirType::CallableFunction(..))
                && owner_fields.iter().any(|(field, owner_ty)| field == name && owner_ty == ty);
            if writable {
                statements.push(HirStmt::Expr(HirExpr::PropAssign(
                    Box::new(owner_var()),
                    owner_type.clone(),
                    name.clone(),
                    Box::new(HirExpr::PropAccess(
                        Box::new(HirExpr::Var(copy.to_string())), view_type.clone(), name.clone())),
                )));
            }
        }
        statements.push(HirStmt::Return(match (returns_receiver, method_ret) {
            (true, _) => Some(owner_var()),
            (false, HirType::Void) => None,
            (false, _) => Some(HirExpr::Var(result_name)),
        }));
        let body = HirExpr::Block(statements);
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&body, &mut referenced);
        let captures = referenced
            .into_iter()
            .filter_map(|name| self.scope.get(&name).cloned().map(|ty| HirParam { name, ty }))
            .collect();
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(captures, Vec::new(), result_type, Box::new(body))),
            Vec::new(),
        ))
    }

    /// A receiver-aware method (its body reads `this`) read as a value (`const f = o.method`)
    /// has no receiver once extracted, so calling it throws a `TypeError`, as in Node; the
    /// extraction itself succeeds. A method that never uses `this` is an ordinary function
    /// value and stays as read.
    /// ponytail: the message names the method, not the property the body would have read.
    fn bind_extracted_method(&mut self, read: HirExpr) -> Result<HirExpr, String> {
        let HirExpr::PropAccess(owner, owner_type, name) = &read else {
            return Ok(read);
        };
        let HirType::Object(fields) = owner_type else {
            return Ok(read);
        };
        let Ok(HirType::Function(params, ret)) = Self::fixed_object_property_read_type(fields, name)
        else {
            return Ok(read);
        };
        let Some(receiver_fields) = params.first().and_then(Self::object_method_receiver_fields) else {
            return Ok(read);
        };
        if !receiver_fields.iter().any(|(field, _)| field == "__thaw_object_method_receiver") {
            return Ok(read);
        }
        let id = self.next_binding;
        self.next_binding += 1;
        // The extracted value is a `this`-taking function (the same value a `function () { this }`
        // expression makes): a plain call has no receiver and throws, while `.call/.apply/.bind`
        // hand it one, which becomes the method's receiver view.
        let marker = "__thaw_object_method_receiver";
        let prefix = receiver_fields.iter().take_while(|(field, _)| field != marker).cloned().collect::<Vec<_>>();
        let this_type = HirType::Object(prefix);
        let rest = params[1..].iter().enumerate()
            .map(|(index, ty)| HirParam { name: format!("__thaw_extracted_arg_{id}_{index}"), ty: ty.clone() })
            .collect::<Vec<_>>();
        let owner_name = format!("__thaw_extracted_owner_{id}");
        self.scope.insert(owner_name.clone(), owner_type.clone());
        let mut statements = Vec::new();
        let mut arguments = vec![HirExpr::Var("__thaw_this".into())];
        arguments.extend(rest.iter().map(|param| HirExpr::Var(param.name.clone())));
        let call = HirExpr::Call(
            Box::new(HirExpr::PropAccess(Box::new(HirExpr::Var(owner_name.clone())), owner_type.clone(), name.clone())),
            arguments,
        );
        statements.push(HirStmt::Return(Some(call)));
        let mut lambda_params = vec![HirParam { name: "__thaw_this".into(), ty:
            HirType::NonArrowReceiver(Box::new(HirType::Object(receiver_fields.to_vec()))) }];
        lambda_params.extend(rest);
        let inner = HirExpr::NonArrowFunction(Box::new(HirExpr::Lambda(
            vec![HirParam { name: owner_name.clone(), ty: owner_type.clone() }],
            lambda_params,
            ret.as_ref().clone(),
            Box::new(HirExpr::Block(statements)),
        )));
        let closure_type = HirType::FunctionWithThis(
            Box::new(this_type),
            Box::new(HirType::Function(params[1..].to_vec(), ret.clone())),
        );
        // Evaluate the owner expression once, as the read itself would.
        let holder = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam { name: owner_name, ty: owner_type.clone() }],
            closure_type,
            Box::new(HirExpr::Block(vec![HirStmt::Return(Some(inner))])),
        );
        Ok(HirExpr::Call(Box::new(holder), vec![owner.as_ref().clone()]))
    }

    /// For a method field whose first parameter is the receiver-view object,
    /// returns a closure over `__thaw_this` taking only the remaining
    /// parameters, calling the method with a live receiver view of the owner.
    fn bind_projected_method_receiver(
        &mut self,
        method: &HirExpr,
        method_type: &HirType,
        owner_type: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        let HirType::Function(params, ret) = method_type else {
            return Ok(None);
        };
        let Some(receiver_fields) = params.first().and_then(Self::object_method_receiver_fields) else {
            return Ok(None);
        };
        if !receiver_fields.iter().any(|(name, _)| name == "__thaw_object_method_receiver") {
            return Ok(None);
        }
        let this = HirExpr::Var("__thaw_this".into());
        let (this_arg, view_binding) =
            self.native_object_method_receiver(this, owner_type, receiver_fields)?;
        let captures = vec![HirParam { name: "__thaw_this".into(), ty: owner_type.clone() }];
        let mut statements = Vec::new();
        if let Some((name, ty, value)) = view_binding {
            statements.push(HirStmt::Let(name, ty, value));
        }
        let mut rest_params = Vec::new();
        let mut arguments = vec![HirExpr::NonArrowReceiverPack {
            value: Box::new(this_arg), declared: HirType::Object(receiver_fields.to_vec()),
        }];
        for (index, ty) in params[1..].iter().enumerate() {
            let name = format!("__thaw_projected_method_arg_{}_{}", index, self.next_binding);
            self.scope.insert(name.clone(), ty.clone());
            arguments.push(HirExpr::Var(name.clone()));
            rest_params.push(HirParam { name, ty: ty.clone() });
        }
        self.next_binding += 1;
        let call = HirExpr::Call(Box::new(method.clone()), arguments);
        statements.push(HirStmt::Return(Some(call)));
        let bound = HirExpr::Lambda(
            captures, rest_params, ret.as_ref().clone(), Box::new(HirExpr::Block(statements)),
        );
        Ok(Some((bound, HirType::Function(params[1..].to_vec(), ret.clone()))))
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
        self.invoke_object_conversion_method(
            value,
            fields,
            well_known_symbol_key("toPrimitive"),
            Some(hint),
        )
    }

    /// Whether `String(value)` of this object type runs user code (`toString`, `valueOf` or
    /// `[Symbol.toPrimitive]` on a class instance, or the same members of an object literal),
    /// so a join over such elements cannot use the fixed `[object Object]` text.
    fn object_has_custom_string_conversion(&self, ty: &HirType) -> bool {
        let HirType::Object(fields) = ty else {
            return false;
        };
        if let Some(class) = class_name_from_type(ty) {
            return ["toString", "valueOf", &well_known_symbol_key("toPrimitive")]
                .iter()
                .any(|method| self.signatures.contains_key(&class_method_symbol(class, method)));
        }
        fields.iter().any(|(name, field)| {
            (name == "toString" || *name == well_known_symbol_key("toPrimitive"))
                && matches!(field, HirType::Function(..) | HirType::CallableFunction(..))
        })
    }

    /// An object literal's own `toString()` method (the string-conversion fallback after
    /// `[Symbol.toPrimitive]`): called with no arguments, its result widened to `Json`.
    fn invoke_object_to_string_method(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
    ) -> Result<Option<HirExpr>, String> {
        self.invoke_object_conversion_method(value, fields, "toString".to_string(), None)
    }

    fn invoke_object_conversion_method(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        key: Symbol,
        hint: Option<&str>,
    ) -> Result<Option<HirExpr>, String> {
        let hint_args = || -> Vec<HirExpr> {
            hint.map(|hint| vec![HirExpr::Lit(HirLit::Str(hint.to_string()))]).unwrap_or_default()
        };
        let Some((_, method_ty)) = fields.iter().find(|(name, _)| *name == key) else {
            return Ok(None);
        };
        let HirType::Function(params, ret) = method_ty.clone() else {
            return Ok(None);
        };
        let object_type = HirType::Object(fields.to_vec());
        let name = format!("__thaw_to_primitive_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), object_type.clone());
        let bound = HirExpr::Var(name.clone());
        let method = HirExpr::PropAccess(Box::new(bound.clone()), object_type.clone(), key);
        // A method that never reads `this` has no receiver parameter: it takes
        // only the hint.
        let Some(receiver_param) = params.first() else {
            let call = HirExpr::Call(
                Box::new(method),
                hint_args(),
            );
            // Callers expect the raw `Json` result; a typed return is widened.
            let call = if *ret == HirType::Json { call } else { self.coerce_to_declared(&HirType::Json, call)? };
            let result = self.wrap_call_argument_bindings(call, &[(name, object_type, value)])?;
            return Ok(Some(result));
        };
        let Some(receiver_fields) = Self::object_method_receiver_fields(receiver_param) else {
            if matches!(receiver_param, HirType::NonArrowReceiver(inner) if **inner == HirType::JsValue) {
                let js_receiver = self.coerce_to_declared(&HirType::JsValue, bound.clone())?;
                let call = HirExpr::Call(
                    Box::new(method),
                    std::iter::once(HirExpr::NonArrowReceiverPack {
                        value: Box::new(js_receiver),
                        declared: HirType::JsValue,
                    })
                    .chain(hint_args())
                    .collect(),
                );
                let call = if *ret == HirType::Json {
                    call
                } else {
                    self.coerce_to_declared(&HirType::Json, call)?
                };
                let result = self.wrap_call_argument_bindings(
                    call,
                    &[(name, object_type, value)],
                )?;
                return Ok(Some(result));
            }
            let call = HirExpr::Call(Box::new(method), hint_args());
            let call = if *ret == HirType::Json { call } else { self.coerce_to_declared(&HirType::Json, call)? };
            let result = self.wrap_call_argument_bindings(call, &[(name, object_type, value)])?;
            return Ok(Some(result));
        };
        let (this_arg, view_binding) = self.native_object_method_receiver(
            bound.clone(), &object_type, receiver_fields,
        )?;
        let call = HirExpr::Call(
            Box::new(method),
            std::iter::once(HirExpr::NonArrowReceiverPack {
                value: Box::new(this_arg), declared: HirType::Object(receiver_fields.to_vec()),
            }).chain(hint_args()).collect(),
        );
        // Callers expect the raw `Json` result; a typed return is widened.
        let call = if *ret == HirType::Json { call } else { self.coerce_to_declared(&HirType::Json, call)? };
        let mut bindings = vec![(name, object_type, value)];
        if let Some(binding) = view_binding { bindings.push(binding); }
        let result = self.wrap_call_argument_bindings(call, &bindings)?;
        Ok(Some(result))
    }

    /// `ToPrimitive(value)` with hint `"default"` for an `Object`-typed value:
    /// its `[Symbol.toPrimitive]` (object literal or class), else a user-defined
    /// `valueOf`/`toString` in ordinary order. `None` when it has none of these.
    fn object_default_to_primitive(
        &mut self,
        value: HirExpr,
        fields: &[(Symbol, HirType)],
        object_type: &HirType,
    ) -> Result<Option<HirExpr>, String> {
        if let Some(result) = self.invoke_object_to_primitive(value.clone(), fields, "default")? {
            return Ok(Some(result));
        }
        if let Some(result) = self.invoke_class_to_primitive(value.clone(), object_type, "default")? {
            return Ok(Some(result));
        }
        self.invoke_class_ordinary_to_primitive(value, object_type, "default")
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
    /// `OrdinaryToPrimitive` for a class instance with no `[Symbol.toPrimitive]`:
    /// the user-defined `valueOf`/`toString` methods in spec order (`toString`
    /// first for hint `"string"`, `valueOf` first otherwise), each with a
    /// primitive declared return. Inherited `Object.prototype` methods are not
    /// user-defined, so they are skipped. `None` means no such method exists and
    /// the caller keeps its `[object Object]` fallback.
    fn invoke_class_ordinary_to_primitive(
        &mut self,
        value: HirExpr,
        object_type: &HirType,
        hint: &str,
    ) -> Result<Option<HirExpr>, String> {
        let Some(class_name) = class_name_from_type(object_type) else {
            return Ok(None);
        };
        let order: [&str; 2] = if hint == "string" { ["toString", "valueOf"] } else { ["valueOf", "toString"] };
        for method in order {
            let symbol = class_method_symbol(class_name, method);
            let Some(signature) = self.signatures.get(&symbol).cloned() else {
                continue;
            };
            if matches!(signature.ret, HirType::Object(_)) {
                continue;
            }
            let call = HirExpr::Call(
                Box::new(HirExpr::FunctionRef(
                    symbol,
                    signature.params.clone(),
                    signature.ret.clone(),
                )),
                vec![value],
            );
            return Ok(Some(self.coerce_to_declared(&HirType::Json, call)?));
        }
        Ok(None)
    }

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
        let args = vec![value, HirExpr::Lit(HirLit::Str(hint.to_string()))];
        let args = signature
            .params
            .iter()
            .zip(args)
            .enumerate()
            .map(|(index, (declared, value))| {
                self.coerce_primitive_array_argument_aliasing(value, declared)
                    .map_err(|error| {
                        format!("argument {} of `{symbol}` is invalid: {error}", index + 1)
                    })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let call = HirExpr::Call(
            Box::new(HirExpr::FunctionRef(
                symbol,
                signature.params.clone(),
                signature.ret.clone(),
            )),
            args,
        );
        Ok(Some(call))
    }
}

/// Early error for `{__proto__: a, __proto__: b}`, which is a SyntaxError in JavaScript.
const PROTO_SETTER_DUPLICATE: &str =
    "duplicate `__proto__` fields are not allowed in object literals";

/// Whether an object literal has a `__proto__: v` prototype-setter property; its static
/// shape is unknown, so it is lowered (and typed) as a dynamic object.
fn object_literal_has_proto_setter(object: &SwcObjectLit) -> bool {
    object.props.iter().any(|property| matches!(property, PropOrSpread::Prop(prop)
        if matches!(prop.as_ref(), Prop::KeyValue(KeyValueProp { key, .. }) if is_proto_setter_key(key))))
}

/// Non-computed `__proto__` key (identifier or string literal). Computed keys are excluded
/// because `{["__proto__"]: v}` defines an ordinary own property; the class normalizer keeps
/// static `__proto__` computed keys computed, so they never reach here as `Str`.
fn is_proto_setter_key(key: &PropName) -> bool {
    match key {
        PropName::Ident(name) => &*name.sym == "__proto__",
        PropName::Str(name) => name.value.to_string_lossy() == "__proto__",
        _ => false,
    }
}
