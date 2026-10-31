/// Hidden member of an interface view naming the class instance it projects.
const VIEW_SOURCE: &str = "__thaw_view_source";

/// Opaque static type of that instance (any native object pointer).
fn view_source_type() -> HirType {
    HirType::Object(vec![("__thaw_view_opaque".into(), HirType::Bool)])
}

fn promise_resolve_value_fits(actual: &HirType, resolved: &HirType) -> bool {
    if actual == resolved {
        return true;
    }
    if *resolved == HirType::Void && *actual == HirType::Undefined {
        return true;
    }
    match resolved {
        HirType::Optional(inner) => {
            actual == &HirType::Undefined || promise_resolve_value_fits(actual, inner)
        }
        HirType::Nullable(inner) => {
            actual == &HirType::Null || promise_resolve_value_fits(actual, inner)
        }
        HirType::Nullish(inner) => {
            matches!(actual, HirType::Null | HirType::Undefined)
                || promise_resolve_value_fits(actual, inner)
        }
        HirType::Union(members) => match actual {
            HirType::Union(source) => source.iter().all(|member| members.contains(member)),
            _ => members.iter().any(|member| promise_resolve_value_fits(actual, member)),
        },
        _ => false,
    }
}

fn native_type_contains_symbol(ty: &HirType) -> bool {
    match ty {
        HirType::Symbol => true,
        // Dictionary is physically a boxed Json value, not a native
        // key/value buffer. Its values cannot carry this native Symbol ABI;
        // leave it on the pre-existing Json projection route.
        HirType::Array(element) | HirType::Optional(element)
        | HirType::Nullable(element) | HirType::Nullish(element) => {
            native_type_contains_symbol(element)
        }
        HirType::Tuple(elements) | HirType::Union(elements) => {
            elements.iter().any(native_type_contains_symbol)
        }
        HirType::Object(fields) => fields.iter().any(|(_, field)| native_type_contains_symbol(field)),
        _ => false,
    }
}

fn absent_after(value: HirExpr, absent: HirExpr) -> HirExpr {
    if matches!(value, HirExpr::Lit(_)) {
        absent
    } else {
        HirExpr::EvalThen(Box::new(value), Box::new(absent))
    }
}

impl<'a> FnLowerer<'a> {
    /// A named function whose last parameter is a rest parameter registers as a rest callable,
    /// so JS callers' trailing arguments are collected into that array.
    fn rest_callable_type(&self, callback: &HirExpr, callback_type: HirType) -> HirType {
        match (callback, &callback_type) {
            (HirExpr::FunctionRef(name, ..), HirType::Function(params, ret)) if !params.is_empty() => {
                match self.signatures.get(name).and_then(|signature| signature.native_rest.clone()) {
                    Some(element) => HirType::CallableFunction(
                        params[..params.len() - 1].to_vec(),
                        HirOptionalMask::default(),
                        Some(Box::new(element)),
                        ret.clone(),
                    ),
                    None => callback_type,
                }
            }
            _ => callback_type,
        }
    }

    /// Preserve the active member with the same HIR adapter used by compiler-
    /// generated Promise reason materializers. The adapter returns an
    /// arena-managed Json cell; callers must share before consuming it.
    pub(crate) fn caught_carrier_as_json(
        &mut self, value: HirExpr,
    ) -> Result<HirExpr, String> {
        Ok(HirExpr::Call(
            Box::new(crate::caught_exception_json_adapter()), vec![value],
        ))
    }

    /// Preserve the allocation's full typed view before structural narrowing.
    /// Nonprefix views also record physical field offsets before type erasure;
    /// the QuickJS factory itself remains lazy in native-only programs.
    /// `Call(Lambda(.., body), args)` wrappers (evaluation-order temporaries) around an `ObjectLit`.
    fn is_wrapped_object_literal(value: &HirExpr) -> bool {
        match value {
            HirExpr::Call(callee, _) => match callee.as_ref() {
                HirExpr::Lambda(_, _, _, body) => {
                    matches!(body.as_ref(), HirExpr::ObjectLit(_)) || Self::is_wrapped_object_literal(body)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Nested wrapper lambdas capture the outer field temporaries by type; keep them in step.
    fn retype_wrapper_captures(body: &mut HirExpr, name: &str, ty: &HirType) {
        if let HirExpr::Call(callee, _) = body {
            if let HirExpr::Lambda(captures, _, _, inner) = callee.as_mut() {
                for capture in captures.iter_mut().filter(|capture| capture.name == name) {
                    capture.ty = ty.clone();
                }
                Self::retype_wrapper_captures(inner, name, ty);
            }
        }
    }

    /// The field of the innermost wrapped literal whose value is the temporary `param`.
    fn wrapped_field_for_param(body: &HirExpr, param: &str) -> Option<Symbol> {
        match body {
            HirExpr::ObjectLit(fields) => fields.iter().find_map(|(name, value)| {
                matches!(value, HirExpr::Var(var) if var == param).then(|| name.clone())
            }),
            HirExpr::Call(callee, _) => match callee.as_ref() {
                HirExpr::Lambda(_, _, _, inner) => Self::wrapped_field_for_param(inner, param),
                _ => None,
            },
            _ => None,
        }
    }

    /// Coerce the innermost literal's fields to `declared`, retyping each wrapper.
    fn coerce_wrapped_object_literal(
        &mut self,
        declared: &HirType,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        match value {
            HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Lambda(..)) => {
                let HirExpr::Lambda(captures, mut params, _, mut body) = *callee else { unreachable!() };
                // Each field temporary takes its declared field type, so a literal argument
                // (`["a", 1]` for a `(string | number)[]` field) is coerced at its source.
                let mut args = args;
                if let HirType::Object(declared_fields) = declared {
                    for (param, arg) in params.iter_mut().zip(args.iter_mut()) {
                        let Some(name) = Self::wrapped_field_for_param(&body, &param.name) else { continue };
                        let Some((_, field_type)) = declared_fields.iter().find(|(field, _)| *field == name) else { continue };
                        if param.ty != *field_type {
                            if let Ok(coerced) = self.coerce_to_declared(field_type, arg.clone()) {
                                *arg = coerced;
                                param.ty = field_type.clone();
                                self.scope.insert(param.name.clone(), field_type.clone());
                                Self::retype_wrapper_captures(&mut body, &param.name, field_type);
                            }
                        }
                    }
                }
                let body = self.coerce_wrapped_object_literal(declared, *body)?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(captures, params, declared.clone(), Box::new(body))),
                    args,
                ))
            }
            other => self.coerce_to_declared(declared, other),
        }
    }

    /// An object literal whose fields were bound to temporaries (to keep evaluation order)
    /// arrives as nested `Call(Lambda(.., body), args)` wrappers around the `ObjectLit`;
    /// retype each wrapper and coerce the literal's fields at the innermost body.
    fn coerce_object_literal_to_dictionary(
        &mut self,
        element: &HirType,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        match value {
            HirExpr::ObjectLit(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| Ok((name, self.coerce_to_declared(element, value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(HirExpr::JsonObjectLit(fields, element.clone()))
            }
            HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Lambda(..)) => {
                let HirExpr::Lambda(captures, mut params, _, mut body) = *callee else { unreachable!() };
                // Every field temporary holds a dictionary value: coerce its argument at the source.
                let mut args = args;
                for (param, arg) in params.iter_mut().zip(args.iter_mut()) {
                    if Self::wrapped_field_for_param(&body, &param.name).is_none() || param.ty == *element {
                        continue;
                    }
                    if let Ok(coerced) = self.coerce_to_declared(element, arg.clone()) {
                        *arg = coerced;
                        param.ty = element.clone();
                        self.scope.insert(param.name.clone(), element.clone());
                        Self::retype_wrapper_captures(&mut body, &param.name, element);
                    }
                }
                let body = self.coerce_object_literal_to_dictionary(element, *body)?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures, params, HirType::Dictionary(Box::new(element.clone())), Box::new(body),
                    )),
                    args,
                ))
            }
            _ => Err(format!(
                "dictionary value must be an object literal with {element:?} values"
            )),
        }
    }

    /// `value as C` where `value` is interface-typed and `C` a native class: the instance behind
    /// the value, checked against `C`'s identity.
    fn lower_interface_view_class_cast(
        &mut self,
        assertion: &swc_ecma_ast::TsAsExpr,
    ) -> Result<Option<HirExpr>, String> {
        let TsType::TsTypeRef(reference) = assertion.type_ann.as_ref() else { return Ok(None) };
        let swc_ecma_ast::TsEntityName::Ident(class) = &reference.type_name else { return Ok(None) };
        let Some(target) = self.interfaces.get(class.sym.as_ref()).cloned() else { return Ok(None) };
        if class_name_from_type(&target) != Some(class.sym.as_ref()) {
            return Ok(None);
        }
        let Some(source) = self.peek_type_without_lowering(&assertion.expr) else {
            return Ok(None);
        };
        let HirType::Object(fields) = &source else { return Ok(None) };
        if fields.is_empty() || fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}")) {
            return Ok(None);
        }
        let value = self.lower_expr(&assertion.expr)?;
        let unwrapped = self.view_unwrap(value, &source)?;
        let checked = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_assert_class_identity".into())),
            vec![unwrapped, HirExpr::Lit(HirLit::Str(class.sym.to_string()))],
        );
        let name = format!("__thaw_view_cast_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), target.clone());
        self.wrap_call_argument_bindings(HirExpr::Var(name.clone()), &[(name, target, checked)]).map(Some)
    }

    /// The instance behind an interface view (or `value` itself when it is not a view), typed
    /// as the opaque source object. Evaluates `value` once.
    fn view_unwrap(&mut self, value: HirExpr, value_type: &HirType) -> Result<HirExpr, String> {
        let HirType::Object(declared) = value_type else {
            return Err("interface view unwrap needs an object".into());
        };
        let name = format!("__thaw_view_unwrap_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), value_type.clone());
        let mut receiver_fields = declared.clone();
        receiver_fields.push(("__thaw_object_method_receiver".into(), HirType::Undefined));
        let mut probe = declared.clone();
        probe.push((VIEW_SOURCE.into(), view_source_type()));
        probe.push((
            format!("__thaw_getter_{VIEW_SOURCE}"),
            HirType::Function(
                vec![HirType::NonArrowReceiver(Box::new(HirType::Object(receiver_fields)))],
                Box::new(view_source_type()),
            ),
        ));
        let read = HirExpr::PropAccess(
            Box::new(HirExpr::Var(name.clone())), HirType::Object(probe), VIEW_SOURCE.into(),
        );
        let has_view = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_object_has_accessor".into())),
            vec![HirExpr::Var(name.clone()), HirExpr::Lit(HirLit::Str(VIEW_SOURCE.into())), HirExpr::Lit(HirLit::Bool(false))],
        );
        let unwrapped = HirExpr::Conditional(
            Box::new(has_view), Box::new(read), Box::new(HirExpr::Var(name.clone())), view_source_type(),
        );
        self.wrap_call_argument_bindings(unwrapped, &[(name, value_type.clone(), value)])
    }

    /// A structural view of a class instance whose fields are accessors over
    /// the instance (`get`: current value, `set`: store through to the field).
    fn live_widened_structural_view(
        &mut self,
        value: HirExpr,
        actual_fields: &[(Symbol, HirType)],
        declared_fields: &[(Symbol, HirType)],
    ) -> Result<HirExpr, String> {
        let object_type = HirType::Object(actual_fields.to_vec());
        let source = format!("__thaw_live_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source.clone(), object_type.clone());
        let mut receiver_fields = declared_fields.to_vec();
        receiver_fields.push(("__thaw_object_method_receiver".into(), HirType::Undefined));
        let receiver = HirType::NonArrowReceiver(Box::new(HirType::Object(receiver_fields)));
        let capture = HirParam { name: source.clone(), ty: object_type.clone() };
        let mut visible = Vec::new();
        let mut hidden = Vec::new();
        for (name, expected) in declared_fields {
            // A declared member the instance provides as a prototype method becomes a closure
            // that calls the class method on the (live) instance.
            if !actual_fields.iter().any(|(n, _)| n == name) {
                let (Some(class), HirType::Function(params, ret)) = (class_name_from_type(&object_type), expected) else {
                    return Err(format!("cannot project method `{name}` of a class instance"));
                };
                let lambda_params: Vec<HirParam> = params.iter().enumerate().map(|(i, ty)| HirParam {
                    name: format!("__thaw_method_view_arg_{}_{i}", self.next_binding), ty: ty.clone(),
                }).collect();
                self.next_binding += 1;
                let mut args = vec![HirExpr::Var(source.clone())];
                args.extend(lambda_params.iter().map(|p| HirExpr::Var(p.name.clone())));
                let call = HirExpr::Call(Box::new(HirExpr::Var(class_method_symbol(class, name))), args);
                visible.push((name.clone(), HirExpr::Lambda(
                    vec![capture.clone()], lambda_params, ret.as_ref().clone(), Box::new(call),
                )));
                continue;
            }
            let read = HirExpr::PropAccess(Box::new(HirExpr::Var(source.clone())), object_type.clone(), name.clone());
            let read = self.lenient_uninit_read_as(read, f64::from_bits(crate::UNSET_NUMBER_BITS))?;
            let this_param = HirParam { name: format!("__thaw_live_this_{}", self.next_binding), ty: receiver.clone() };
            self.next_binding += 1;
            let getter = HirExpr::Lambda(vec![capture.clone()], vec![this_param.clone()], expected.clone(), Box::new(read));
            let stored = actual_fields.iter().find(|(n, _)| n == name).map(|(_, ty)| ty.clone()).unwrap_or_else(|| expected.clone());
            let new_value = HirParam { name: format!("__thaw_live_value_{}", self.next_binding), ty: expected.clone() };
            self.next_binding += 1;
            let mut assigned = HirExpr::Var(new_value.name.clone());
            if let HirType::Optional(payload) = &stored {
                assigned = HirExpr::OptionalSome(Box::new(assigned), payload.as_ref().clone());
            }
            let store = HirExpr::PropAssign(Box::new(HirExpr::Var(source.clone())), object_type.clone(), name.clone(), Box::new(assigned));
            let setter = HirExpr::Lambda(
                vec![capture.clone()], vec![this_param, new_value], HirType::Void,
                Box::new(HirExpr::Block(vec![HirStmt::Expr(store)])),
            );
            visible.push((name.clone(), Self::unreachable_value(expected)?));
            hidden.push((format!("__thaw_getter_{name}"), getter));
            hidden.push((format!("__thaw_setter_{name}"), setter));
        }
        // The view remembers the instance it projects so `instanceof`/narrowing can recover it.
        let this_param = HirParam { name: format!("__thaw_live_this_{}", self.next_binding), ty: receiver.clone() };
        self.next_binding += 1;
        hidden.push((
            format!("__thaw_getter_{VIEW_SOURCE}"),
            HirExpr::Lambda(vec![capture.clone()], vec![this_param], view_source_type(), Box::new(HirExpr::Var(source.clone()))),
        ));
        visible.extend(hidden);
        let literal = HirExpr::ObjectLit(visible);
        self.wrap_call_argument_bindings(literal, &[(source, object_type, value)])
    }

    fn retain_full_native_object_projection(
        &mut self, value: HirExpr, fields: &[(Symbol, HirType)], nonprefix: bool,
    ) -> Result<HirExpr, String> {
        let full_type = HirType::Object(fields.to_vec());
        let owner = format!("__thaw_full_native_owner_{}", self.next_binding);
        self.next_binding += 1;
        let previous = self.scope.insert(owner.clone(), full_type.clone());
        let was_building_factory = std::mem::replace(&mut self.lowering_native_projection_factory, true);
        let projection = self.lower_fixed_object_as_dynamic_accessor_object(
            HirExpr::Var(owner.clone()), fields, true, true, false,
        );
        self.lowering_native_projection_factory = was_building_factory;
        let projection = projection?;
        let factory = HirExpr::Lambda(
            vec![HirParam { name: owner.clone(), ty: full_type.clone() }],
            Vec::new(), HirType::JsValue, Box::new(projection),
        );
        let register = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_object_projector".into())),
            vec![HirExpr::Var(owner.clone()),
                HirExpr::Lit(HirLit::Str(crate::native_object_layout_token(fields))), factory],
        );
        // The first view is still the allocation's physical layout, even
        // when it happens to be a prefix. A later reordered alias must not
        // seed the offset map from that earlier apparent view. A borrowed
        // external prefix stays usable natively; it simply cannot acquire
        // an arena-owned live projection.
        let register = HirExpr::EvalThen(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_object_layout".into())),
            vec![HirExpr::Var(owner.clone()), HirExpr::Lit(HirLit::Bool(nonprefix))],
        )), Box::new(register));
        let result = self.wrap_call_argument_bindings(
            HirExpr::EvalThen(Box::new(register), Box::new(HirExpr::Var(owner.clone()))),
            &[(owner.clone(), full_type, value)],
        );
        match previous {
            Some(previous) => { self.scope.insert(owner, previous); }
            None => { self.scope.remove(&owner); }
        }
        result
    }

    fn retain_optional_native_this_projection(
        &mut self, value: HirExpr, fields: &[(Symbol, HirType)],
    ) -> Result<HirExpr, String> {
        let full_type = HirType::Object(fields.to_vec());
        let owner = format!("__thaw_optional_this_owner_{}", self.next_binding);
        self.next_binding += 1;
        let previous = self.scope.insert(owner.clone(), full_type.clone());
        let class_methods = self.live_projection_supported_with_classes(&full_type);
        let was_building_factory = std::mem::replace(&mut self.lowering_native_projection_factory, true);
        let projection = self.lower_fixed_object_as_dynamic_accessor_object(
            HirExpr::Var(owner.clone()), fields, true, true, class_methods,
        );
        self.lowering_native_projection_factory = was_building_factory;
        let projection = projection?;
        let factory = HirExpr::Lambda(
            vec![HirParam { name: owner.clone(), ty: full_type.clone() }],
            Vec::new(), HirType::JsValue, Box::new(projection),
        );
        let register = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_object_projector".into())),
            vec![HirExpr::Var(owner.clone()),
                HirExpr::Lit(HirLit::Str(crate::native_object_layout_token(fields))), factory],
        );
        let registered = format!("{owner}_layout_registered");
        let body = HirExpr::Block(vec![
            HirStmt::Let(registered.clone(), HirType::Bool, HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_register_native_object_layout".into())),
                vec![HirExpr::Var(owner.clone()), HirExpr::Lit(HirLit::Bool(false))],
            )),
            HirStmt::If(HirExpr::Var(registered), vec![HirStmt::Expr(register)], Vec::new()),
            HirStmt::Return(Some(HirExpr::Var(owner.clone()))),
        ]);
        let wrapper = HirExpr::Lambda(
            Vec::new(), vec![HirParam { name: owner.clone(), ty: full_type.clone() }],
            full_type.clone(), Box::new(body),
        );
        let result = HirExpr::Call(Box::new(wrapper), vec![value]);
        match previous {
            Some(previous) => { self.scope.insert(owner, previous); }
            None => { self.scope.remove(&owner); }
        }
        Ok(result)
    }

    /// Permutes an (optionally evaluation-order-wrapped) accessor object
    /// literal into declared field order with the hidden accessor fields
    /// last. Only reorders already-bound fields, so effects keep their order.
    fn reorder_wrapped_accessor_literal(
        value: &HirExpr,
        declared_fields: &[(Symbol, HirType)],
    ) -> Option<HirExpr> {
        fn order<T: Clone>(
            fields: &[(Symbol, T)],
            declared_fields: &[(Symbol, HirType)],
        ) -> Option<Vec<(Symbol, T)>> {
            let mut ordered = Vec::with_capacity(fields.len());
            for (name, _) in declared_fields {
                ordered.push(fields.iter().find(|(key, _)| key == name)?.clone());
            }
            for field in fields {
                if is_hidden_accessor_field(&field.0) {
                    ordered.push(field.clone());
                }
            }
            (ordered.len() == fields.len()).then_some(ordered)
        }
        match value {
            HirExpr::ObjectLit(fields) => Some(HirExpr::ObjectLit(order(fields, declared_fields)?)),
            HirExpr::Call(callee, args) => {
                let HirExpr::Lambda(captures, params, ret, body) = callee.as_ref() else {
                    return None;
                };
                let body = Self::reorder_wrapped_accessor_literal(body, declared_fields)?;
                let ret = match ret {
                    HirType::Object(fields) => HirType::Object(order(fields, declared_fields)?),
                    other => other.clone(),
                };
                Some(HirExpr::Call(
                    Box::new(HirExpr::Lambda(captures.clone(), params.clone(), ret, Box::new(body))),
                    args.clone(),
                ))
            }
            _ => None,
        }
    }

    /// Capture source fields before assembling the declared storage layout.
    /// ObjectLit codegen visits its fields in storage order, which can differ
    /// from the order of effects in the original object literal.
    fn reorder_object_literal_in_source_order(
        &mut self,
        lit_fields: &[(Symbol, HirExpr)],
        declared_fields: &[(Symbol, HirType)],
        include_hidden: bool,
    ) -> Result<HirExpr, String> {
        let mut bindings: Vec<LoweredBinding> = Vec::with_capacity(lit_fields.len());
        let mut saved = Vec::with_capacity(lit_fields.len());
        let result = (|| {
            for (name, source) in lit_fields {
                let value = if let Some((_, expected)) = declared_fields.iter().find(|(key, _)| key == name) {
                    self.coerce_to_declared(expected, source.clone())
                        .map_err(|error| format!("field `{name}`: {error}"))?
                } else {
                    source.clone()
                };
                let ty = self.infer_expr_type(&value)?;
                let temp = format!("__thaw_ordered_object_field_{}", self.next_binding);
                self.next_binding += 1;
                saved.push((temp.clone(), self.scope.insert(temp.clone(), ty.clone())));
                bindings.push((temp, ty, value));
            }
            let mut reordered = Vec::with_capacity(declared_fields.len() + lit_fields.len());
            for (name, expected) in declared_fields {
                let value = if let Some(index) = lit_fields.iter().position(|(key, _)| key == name) {
                    HirExpr::Var(bindings[index].0.clone())
                } else {
                    omitted_parameter_value(expected)
                        .map_err(|_| format!("object literal is missing required property `{name}`"))?
                };
                reordered.push((name.clone(), value));
            }
            if include_hidden {
                for (index, (name, _)) in lit_fields.iter().enumerate() {
                    if is_hidden_accessor_field(name) {
                        reordered.push((name.clone(), HirExpr::Var(bindings[index].0.clone())));
                    }
                }
            }
            self.wrap_call_argument_bindings(HirExpr::ObjectLit(reordered), &bindings)
        })();
        for (name, previous) in saved {
            match previous {
                Some(previous) => { self.scope.insert(name, previous); }
                None => { self.scope.remove(&name); }
            }
        }
        result
    }

    /// Coerces a value into its declared native layout.
    /// Whether a declared slot stores `any` values (directly or as an element).
    fn type_holds_any(declared: &HirType) -> bool {
        match declared {
            HirType::Json => true,
            HirType::Array(inner) | HirType::Optional(inner) | HirType::Nullable(inner)
            | HirType::Nullish(inner) | HirType::Dictionary(inner) => Self::type_holds_any(inner),
            HirType::Tuple(members) | HirType::Union(members) =>
                members.iter().any(Self::type_holds_any),
            _ => false,
        }
    }

    /// `coerce_to_declared` for a user-visible `any` slot (declaration or assignment): a native
    /// array stored there stays a live view of the array instead of becoming a copy.
    pub(in crate::lower) fn coerce_to_declared_aliasing(&mut self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        // Restore (not clear): an assignment nested inside another aliasing coercion must not
        // switch off the outer one.
        let outer = std::mem::replace(&mut self.alias_native_arrays, Self::type_holds_any(declared));
        let result = self.coerce_to_declared(declared, value);
        self.alias_native_arrays = outer;
        result
    }

    /// An `async` method stores its awaited result type as `ret`; the interface member
    /// declares the `Promise` of it.
    fn async_signature_returns(is_async: bool, ret: &HirType, declared: &HirType) -> bool {
        is_async && matches!(declared, HirType::Promise(inner) if inner.as_ref() == ret)
    }

    fn class_method_signature_compatible(
        &self,
        class: &str,
        name: &str,
        expected: &HirType,
        source_contract: bool,
    ) -> bool {
        let symbol = class_method_symbol(class, name);
        if *expected == HirType::Dynamic
            && self.signatures.keys().any(|candidate| {
                candidate == &symbol || candidate.starts_with(&format!("{symbol}__thaw_"))
            })
        {
            return true;
        }
        let Some(signature) = self.signatures.get(&symbol) else {
            return false;
        };
        let stored_params = signature.params.get(1..).unwrap_or_default();
        let source_start = signature.source_range.0;
        let params = if source_contract {
            storage_source_callable_parameters(source_start, stored_params)
        } else {
            stored_params.to_vec()
        };
        let fixed = if signature.native_rest.is_some() {
            &params[..params.len().saturating_sub(1)]
        } else {
            &params
        };
        let native_rest = if source_contract {
            params.last().and_then(|parameter| match parameter {
                HirType::Array(element) => Some(element.as_ref().clone()),
                _ => None,
            }).or_else(|| signature.native_rest.clone())
        } else {
            signature.native_rest.clone()
        };
        let ret = if source_contract {
            storage_source_return_type(source_start).unwrap_or_else(|| signature.ret.clone())
        } else {
            signature.ret.clone()
        };
        match expected {
            HirType::Dynamic if !signature.generic_type_params.is_empty() => true,
            HirType::Function(expected_params, result) => {
                fixed == expected_params
                    && (&ret == result.as_ref()
                        // An unannotated method's return is still the `Dynamic` placeholder
                        // until a later pass infers it from the body.
                        || ret == HirType::Dynamic
                        || Self::async_signature_returns(signature.is_async, &ret, result))
            }
            HirType::CallableFunction(expected_params, _, expected_rest, result) => {
                fixed.len() == expected_params.len()
                    && fixed.iter().zip(expected_params).all(|(actual, expected)| {
                        matches!(expected, HirType::Optional(inner) if inner.as_ref() == actual)
                            || expected == actual
                    })
                    && native_rest.as_ref() == expected_rest.as_deref()
                    && (&ret == result.as_ref()
                        || Self::async_signature_returns(signature.is_async, &ret, result))
            }
            _ => false,
        }
    }

    fn class_method_signature_exists(&self, class: &str, name: &str) -> bool {
        let symbol = class_method_symbol(class, name);
        self.signatures.keys().any(|candidate| {
            candidate == &symbol || candidate.starts_with(&format!("{symbol}__thaw_"))
        })
    }

    /// A callback with typed parameters (`(n: number) => ...`) stored in a `(...args: any[]) => R`
    /// slot is entered with one rest array of `Json` values: wrap it so each argument is decoded
    /// to the callback's own parameter type (an absent one reads `undefined`).
    fn adapt_callback_to_json_rest(
        &mut self,
        declared: &HirType,
        value: &HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let HirType::CallableFunction(fixed, _, Some(rest), ret) = declared else {
            return Ok(None);
        };
        if !fixed.is_empty() || **rest != HirType::Json {
            return Ok(None);
        }
        // A callback with an optional parameter (`(x: number, y?: string) => ...`) is a
        // rest-free `CallableFunction`; its physical ABI is the fixed prefix.
        let inferred = self.infer_expr_type(value);
        let (params, actual_ret, callback_type) = match inferred {
            Ok(HirType::Function(params, ret)) => {
                (params.clone(), ret.clone(), HirType::Function(params, ret))
            }
            Ok(callback @ HirType::CallableFunction(..)) => {
                let HirType::CallableFunction(params, _, None, ret) = &callback else {
                    return Ok(None);
                };
                (params.clone(), ret.clone(), callback.clone())
            }
            _ => return Ok(None),
        };
        let rest_array = HirType::Array(rest.clone());
        if params.is_empty()
            || params == [rest_array.clone()]
            || !(**ret == HirType::Void || *ret == actual_ret)
            || params.iter().any(|ty| matches!(ty, HirType::Function(..) | HirType::CallableFunction(..)))
        {
            return Ok(None);
        }
        let id = self.next_binding;
        self.next_binding += 1;
        let callback_name = format!("__thaw_rest_callback_{id}");
        let args_name = format!("__thaw_rest_args_{id}");
        self.scope.insert(callback_name.clone(), callback_type.clone());
        self.scope.insert(args_name.clone(), rest_array.clone());
        let mut arguments = Vec::with_capacity(params.len());
        for (index, ty) in params.iter().enumerate() {
            let present = HirExpr::BinOp(
                BinOp::Gt,
                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(args_name.clone())))),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
            );
            let element = HirExpr::TypedIndex(
                Box::new(HirExpr::Var(args_name.clone())),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                HirType::Json,
            );
            let absent = self.coerce_to_declared(&HirType::Json, HirExpr::Lit(HirLit::Undefined))?;
            let json = HirExpr::Conditional(Box::new(present), Box::new(element), Box::new(absent), HirType::Json);
            arguments.push(self.coerce_to_declared(ty, json)?);
        }
        let call = HirExpr::Call(Box::new(HirExpr::Var(callback_name.clone())), arguments);
        let body = if **ret == HirType::Void {
            HirExpr::Block(vec![HirStmt::Expr(call)])
        } else {
            call
        };
        let adapter = HirExpr::Lambda(
            vec![HirParam { name: callback_name.clone(), ty: callback_type.clone() }],
            vec![HirParam { name: args_name, ty: rest_array }],
            ret.as_ref().clone(),
            Box::new(body),
        );
        let closure = HirExpr::TypedClosure(declared.clone(), Box::new(adapter));
        self.wrap_call_argument_bindings(closure, &[(callback_name, callback_type, value.clone())])
            .map(Some)
    }

    fn coerce_to_declared(&mut self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        // A slot typed through a class back-edge coerces to the layout it names; a value that
        // already has that layout (modulo back-edges) needs no conversion.
        if declared.contains_recursive() {
            if let Ok(actual) = self.infer_expr_type(&value) {
                if actual.expand() == declared.expand() {
                    return Ok(value);
                }
            }
            return self.coerce_to_declared_expanded(&declared.expand(), value);
        }
        if let Ok(actual) = self.infer_expr_type(&value) {
            if actual.contains_recursive() && actual.expand() == *declared {
                return Ok(value);
            }
        }
        self.coerce_to_declared_expanded(declared, value)
    }

    fn coerce_to_declared_expanded(&mut self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        // The function-name wrapper is transparent to a callable slot: coerce the closure and
        // keep its label (other slots wrap the labelled closure like any function value).
        if matches!(declared, HirType::Function(..) | HirType::CallableFunction(..) | HirType::OverloadedFunction(..)) {
            if let Some((closure, label)) = split_named_function(&value) {
                let (closure, label) = (closure.clone(), label.to_owned());
                let coerced = self.coerce_to_declared(declared, closure)?;
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_named_function".into())),
                    vec![coerced, HirExpr::Lit(HirLit::Str(label))],
                ));
            }
        }
        if let Some(adapted) = self.adapt_callback_to_json_rest(declared, &value)? {
            return Ok(adapted);
        }
        // An observed read of a maybe-uninitialised class field arrives with
        // its raw `Optional`; a slot that wants the bare payload unwraps it
        // like `undefined` coerced to `T`.
        if let HirExpr::PropAccess(_, HirType::Object(fields), field) = &value {
            let widened = fields.iter().find(|(name, _)| name == field).map(|(_, ty)| ty);
            let wants_payload = match widened {
                Some(HirType::Optional(payload)) => payload.as_ref() == declared,
                Some(HirType::Nullish(payload)) => {
                    matches!(declared, HirType::Nullable(inner) if inner == payload)
                }
                _ => false,
            };
            if wants_payload && is_uninit_class_field(fields, field) {
                let value = self.lenient_uninit_read(value)?;
                return self.coerce_to_declared(declared, value);
            }
        }
        // Keep an indexed read's bounds check when its destination can represent undefined.
        let value = match value {
            HirExpr::TypedIndex(array, index, element)
                if matches!(element, HirType::F64 | HirType::Str | HirType::Bool | HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_))
                    && (matches!(declared, HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined | HirType::Json | HirType::JsValue | HirType::Dynamic)
                        || matches!(declared, HirType::Union(members) if members.contains(&HirType::Undefined))) =>
            {
                let array_type = self.infer_expr_type(&array)?;
                if matches!(&array_type, HirType::Array(inner) if **inner == element) {
                    self.lower_array_index(*array, array_type, element, *index)?
                } else {
                    HirExpr::TypedIndex(array, index, element)
                }
            }
            value => value,
        };
        if let HirType::NonArrowReceiver(receiver) = declared {
            if let HirExpr::NonArrowThis {
                payload,
                declared: actual_receiver,
                access: NonArrowThisAccess::Value,
            } = &value
            {
                if actual_receiver == receiver.as_ref() {
                    return Ok(payload.as_ref().clone());
                }
            }
            let value = self.coerce_this_for_receiver(Some(receiver.as_ref()), value)?;
            if self.infer_expr_type(&value)? == *declared {
                return Ok(value);
            }
            return Ok(HirExpr::NonArrowReceiverPack {
                value: Box::new(value),
                declared: receiver.as_ref().clone(),
            });
        }
        if let HirExpr::NonArrowThis {
            payload,
            declared: receiver_type,
            access: NonArrowThisAccess::Value,
        } = &value
        {
            if receiver_type == declared
                && !matches!(declared, HirType::Json | HirType::Dictionary(_) | HirType::Dynamic)
            {
                return Ok(HirExpr::NonArrowThis {
                    payload: payload.clone(),
                    declared: receiver_type.clone(),
                    access: NonArrowThisAccess::TypedValue,
                });
            }
        }
        if let HirType::Promise(target) = declared {
            if target.as_ref() == &HirType::Json {
                if let HirType::Promise(source) = self.infer_expr_type(&value)? {
                    if source.as_ref() != target.as_ref() {
                        let (params, body) = if source.as_ref() == &HirType::Void {
                            (Vec::new(), self.coerce_to_declared_aliasing(
                                &HirType::Json, HirExpr::Lit(HirLit::Undefined),
                            )?)
                        } else {
                            let parameter = format!("__thaw_promise_json_value_{}", self.next_binding);
                            self.next_binding += 1;
                            let previous = self.scope.insert(parameter.clone(), source.as_ref().clone());
                            let body = self.coerce_to_declared_aliasing(
                                &HirType::Json, HirExpr::Var(parameter.clone()),
                            );
                            match previous {
                                Some(previous) => { self.scope.insert(parameter.clone(), previous); }
                                None => { self.scope.remove(&parameter); }
                            }
                            (vec![HirParam { name: parameter, ty: source.as_ref().clone() }], body?)
                        };
                        let callback = HirExpr::Lambda(
                            Vec::new(), params, HirType::Json, Box::new(body),
                        );
                        return Ok(HirExpr::PromiseThen(
                            Box::new(value), Box::new(callback), source.as_ref().clone(),
                            HirType::Json, false, false,
                        ));
                    }
                }
            }
        }
        if *declared == HirType::Dynamic {
            return Ok(value);
        }
        // An element read that only looks optional, handed to a slot declared as the method-bearing
        // object itself: unwrap it (keeping the runtime `undefined` guard).
        if matches!(declared, HirType::Object(fields) if class_name_from_type(declared).is_some()
            || fields.iter().any(|(_, ty)| matches!(ty, HirType::Function(..) | HirType::CallableFunction(..))))
        {
            if let Ok(HirType::Optional(payload)) = self.infer_expr_type(&value) {
                if payload.as_ref() == declared {
                    let (unwrapped, _) = self.unwrap_required_optional_member(
                        value, HirType::Optional(payload), "value", false,
                    )?;
                    return Ok(unwrapped);
                }
            }
        }
        // A `Map`/`Set`/`Array` subclass instance passed where its native collection
        // is declared (alone or as a union member): hand over the wrapped collection
        // (shared, so mutation through the callee stays visible on the instance).
        let mut value = value;
        let target = match declared {
            HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => inner.as_ref(),
            other => other,
        };
        if matches!(target, HirType::Map(..) | HirType::Set(_) | HirType::Array(_) | HirType::Union(_)) {
            if let Ok(actual) = self.infer_expr_type(&value) {
                if let Some(base) = native_base_type(&actual) {
                    let accepted = match target {
                        HirType::Union(members) => members.contains(base),
                        other => other == base,
                    };
                    if accepted {
                        let class = match &actual {
                            HirType::Object(fields) => fields.first().and_then(|(marker, _)| {
                                marker.strip_prefix("__thaw_class_identity_\u{1e}")
                                    .and_then(|chain| chain.split('\u{1f}').next())
                            }),
                            _ => None,
                        }.map(str::to_string);
                        value = self.project_native_base(value)?;
                        if let Some(class) = class {
                            value = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_native_subclass_mark".into())),
                                vec![value, HirExpr::Lit(HirLit::Str(class))],
                            );
                        }
                    }
                }
            }
        }
        // A `Map`/`Set` (or a subclass instance) handed to the entries/values array of a
        // native collection constructor: iterate it, like `new Map(other)` does.
        if let HirType::Array(element) = target {
            if let Ok(actual) = self.infer_expr_type(&value) {
                let source = native_base_type(&actual).unwrap_or(&actual);
                let iterable = match source {
                    HirType::Map(key, entry) => {
                        **element == HirType::Tuple(vec![key.as_ref().clone(), entry.as_ref().clone()])
                    }
                    HirType::Set(item) => element == item,
                    _ => false,
                };
                if iterable {
                    value = self.native_collection_entries(value)?;
                }
            }
        }
        if matches!(declared, HirType::Function(..)
            | HirType::CallableFunction(_, _, None, _))
        {
            let actual = self.infer_expr_type(&value)?;
            if matches!(actual, HirType::Json | HirType::JsValue) {
                let json = if actual == HirType::JsValue {
                    self.coerce_to_declared(&HirType::Json, value)?
                } else {
                    value
                };
                return Ok(HirExpr::JsonAsNative(Box::new(json), declared.clone()));
            }
        }
        if *declared == HirType::Void
            && self.infer_expr_type(&value)? == HirType::Json
        {
            return Ok(HirExpr::JsonAsNative(Box::new(value), declared.clone()));
        }
        if let HirExpr::Var(name) = &value {
            if self.native_class_aliases.get(name) == Some(declared) {
                return Ok(value);
            }
        }
        if *declared == HirType::Str {
            let actual = self.infer_expr_type(&value)?;
            if object_type_is_error_family(&actual) {
                return self.coerce_primitive_to_string(value);
            }
        }
        // The symmetric case to the `HirType::Json` branch just below --
        // a dynamic method call with no `JsValue` hint (`lower_dynamic_
        // value_method_call`'s own default) always comes back `Json`-
        // typed, even when the caller's own declared slot is a concrete
        // scalar. Real trigger: `const body: string = await res.text()`
        // (real hono) used to fail outright ("value has type Json,
        // expected Str") with no way to consume the result except by
        // keeping it `JsValue`-typed and manually decoding it later.
        // Reuses the exact `JsonAsNumber`/`JsonAsString`/`JsonAsBool`
        // nodes `dictionary_value_from_json` (objects.rs) already builds
        // for the identical "decode a Json field into its declared
        // scalar type" problem -- already fully supported by type
        // inference and both codegen backends, so no new HIR node or
        // codegen path is needed here.
        if matches!(declared, HirType::F64 | HirType::Str | HirType::Bool) {
            let actual = self.infer_expr_type(&value)?;
            if matches!(actual, HirType::Json | HirType::JsValue) {
                let value = if actual == HirType::JsValue {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    )
                } else {
                    value
                };
                return Ok(match declared {
                    HirType::F64 => HirExpr::JsonAsNumber(Box::new(value)),
                    HirType::Str => HirExpr::JsonAsString(Box::new(value)),
                    HirType::Bool => HirExpr::JsonAsBool(Box::new(value)),
                    _ => unreachable!(),
                });
            }
        }
        if matches!(
            declared,
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Bytes
        ) {
            let actual = self.infer_expr_type(&value)?;
            // Object.values/entries expose every runtime own value through
            // a dynamic array. A contextual tuple/array annotation is a
            // typed view of that same source, not permission to truncate its
            // runtime tail. Route through the existing JSON array decoder;
            // tuple decoding retains its complete dynamic authority.
            if matches!(declared, HirType::Array(_) | HirType::Tuple(_))
                && declared != &actual
                && !matches!(value, HirExpr::ArrayLit(_))
                && matches!(&actual, HirType::Array(element)
                    if matches!(element.as_ref(), HirType::Json | HirType::Tuple(_)))
            {
                let json = self.wrap_native_value_as_json(value, actual)?;
                return Ok(HirExpr::JsonAsNative(Box::new(json), declared.clone()));
            }
            // `Bytes` is a view record, not an `f64`-slot array: crossing the
            // two copies (a `Buffer[]` list converts one level deep).
            if bytes_layout_eq(declared, &actual) {
                let inner = self.infer_expr_type_inner(&value)?;
                let intrinsic = match (declared, &inner) {
                    (HirType::Bytes, HirType::Array(element)) if **element == HirType::F64 => {
                        Some("__thaw_bytes_from_array")
                    }
                    (HirType::Array(element), HirType::Bytes) if **element == HirType::F64 => {
                        Some("__thaw_bytes_to_array")
                    }
                    (HirType::Array(want), HirType::Array(have))
                        if **want == HirType::Bytes
                            && matches!(have.as_ref(), HirType::Array(e) if **e == HirType::F64) =>
                    {
                        Some("__thaw_bytes_narrow_list")
                    }
                    (HirType::Array(want), HirType::Array(have))
                        if **have == HirType::Bytes
                            && matches!(want.as_ref(), HirType::Array(e) if **e == HirType::F64) =>
                    {
                        Some("__thaw_bytes_widen_list")
                    }
                    _ => None,
                };
                if let Some(intrinsic) = intrinsic {
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(intrinsic.into())), vec![value]));
                }
            }
            if declared != &actual && bytes_layout_eq(declared, &actual) {
                return Ok(value);
            }
            if matches!(actual, HirType::Json | HirType::JsValue) {
                let value = if actual == HirType::JsValue {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    )
                } else {
                    value
                };
                return Ok(HirExpr::JsonAsNative(Box::new(value), declared.clone()));
            }
        }
        // `arr[i]`'s hole/out-of-bounds-aware read is typed `Optional<T>`,
        // so a callback that returns one makes `.map`/`.flatMap` produce
        // `Array<Optional<T>>`. Assigning that to a declared `T[]` has no
        // natural coercion -- real trigger: `const words: string[] =
        // Array.from(s.matchAll(re)).map((m) => m[0])`, which broke once
        // array reads became hole-aware. Unwrap element-wise through the
        // existing array-map lowerer, reusing the same `Optional<T> -> T`
        // scalar coercion (`undefined` -> "undefined"/NaN) already applied
        // to `const value: T = arr[i]` assignments.
        if let HirType::Array(declared_element) = declared {
            if !matches!(value, HirExpr::ArrayLit(_)) {
                if let HirType::Array(actual_element) = &self.infer_expr_type(&value)? {
                    if let HirType::Optional(payload) = actual_element.as_ref() {
                        if payload.as_ref() == declared_element.as_ref()
                            && matches!(payload.as_ref(), HirType::F64 | HirType::Str)
                        {
                            let parameter = format!("__thaw_array_unwrap_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope
                                .insert(parameter.clone(), actual_element.as_ref().clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let body = match payload.as_ref() {
                                HirType::F64 => self.coerce_primitive_to_number(bound)?,
                                _ => self.coerce_primitive_to_string(bound)?,
                            };
                            let callback = HirExpr::Lambda(
                                Vec::new(),
                                vec![HirParam {
                                    name: parameter,
                                    ty: actual_element.as_ref().clone(),
                                }],
                                payload.as_ref().clone(),
                                Box::new(body),
                            );
                            return self.lower_array_map(
                                value,
                                HirType::Array(actual_element.clone()),
                                actual_element.as_ref().clone(),
                                actual_element.as_ref().clone(),
                                callback,
                                None,
                            );
                        }
                    }
                }
            }
        }
        let inferred = self.infer_expr_type(&value)?;
        if *declared == HirType::JsValue
            && (inferred == HirType::Str || inferred == HirType::Symbol)
        {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("retainEventKey".to_string())),
                vec![value, HirExpr::Lit(HirLit::Bool(inferred == HirType::Symbol))],
            ));
        }
        // A callback value whose parameter object types are a width-subtype
        // (prefix) of the declared callback's -- e.g. a `createServer`
        // handler annotated with a narrower request/response than
        // `IncomingMessage`/`ServerResponse` -- is layout-compatible and
        // needs no adapter. Deliberately narrower than
        // `callable_value_compatible`: an `Optional`/`CallableFunction`
        // difference still falls through to the adapter path (e.g. a
        // default-argument wrapper).
        if let (HirType::Function(params, ret), HirType::CallableFunction(..)) = (declared, &inferred) {
            self.validate_promise_callback_value(&value, params, Some(ret))?;
            return self.normalize_promise_callback(value, params);
        }
        if callable_abi_compatible(declared, &inferred) {
            return Ok(value);
        }
        if *declared == HirType::JsValue
            && matches!(inferred, HirType::Function(_, _) | HirType::CallableFunction(..) | HirType::FunctionWithThis(_, _))
        {
            let (callback, callback_type) = if matches!(inferred, HirType::Function(_, _)) {
                self.adapt_callable_result_for_live_projection(value, inferred.clone())?
            } else {
                (value, inferred.clone())
            };
            let graph_callback = callback_type != inferred;
            let callback_type = self.rest_callable_type(&callback, callback_type);
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var(if graph_callback {
                    "registerNativeCallbackGraph".to_string()
                } else {
                    "registerNativeCallback".to_string()
                })),
                vec![HirExpr::TypedClosure(callback_type, Box::new(callback))],
            ));
        }
        if *declared == HirType::JsValue
            && matches!(
                inferred,
                HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Dictionary(_)
            )
        {
            if native_type_contains_symbol(&inferred) {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("retainDynamicNative".to_string())),
                    vec![value],
                ));
            }
            let json = self.coerce_to_declared(&HirType::Json, value)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("retainDynamicJson".to_string())),
                vec![json],
            ));
        }
        // The symmetric case to the `HirType::JsValue`-into-`Json` branch
        // above: a value whose own real, live type genuinely is `JsValue`
        // (e.g. a real npm class instance) can end up statically typed
        // `Json` instead -- real trigger: `const rs: any = new ReadStream
        // (...)`, an explicit `: any` annotation, whose declared type
        // lowers to `HirType::Json` regardless of the initializer's own
        // real type. Calling a method on it later needs a real `JsValue`
        // handle again, but `rs`'s stored type says `Json`. `JsValueAsJson`
        // above always encodes a live handle as exactly the same
        // `{"__thaw_js_handle_id__": id}` placeholder object
        // `compile_dynamic_value_placeholder` builds for the opposite
        // direction -- `thaw_json_handle_id` (thaw-std) is the existing,
        // already-wired native-side reader for that exact shape (used
        // today only for a `JsValue`-typed native-callback argument
        // decoded from real JSON, `compile_json_value_to_native`'s own
        // `HirType::JsValue` arm) -- reused here via `JsonAsNative`'s own
        // codegen (`compile_json_to_native`, extended with the identical
        // one case) to recover the live handle a `Json`-typed value that
        // is genuinely this placeholder shape was never really without.
        if *declared == HirType::JsValue && self.infer_expr_type(&value)? == HirType::Json {
            return Ok(HirExpr::JsonAsNative(Box::new(value), HirType::JsValue));
        }
        if let (HirType::Object(declared_fields), HirType::Object(actual_fields)) =
            (declared, self.infer_expr_type(&value)?)
        {
            // A user class extending Error has its own first marker name,
            // but the first three physical slots retain the same Bool/Json/Json
            // prefix. An Error-annotated alias must keep its allocation
            // pointer; nominal ancestry remains checked in the native table.
            if *declared == builtin_error_object_type()
                && object_type_is_error_family(&HirType::Object(actual_fields.clone()))
                && actual_fields.len() >= 3
                && actual_fields[0].1 == HirType::Bool
                && actual_fields[1] == ("message".to_string(), HirType::Json)
                && actual_fields[2] == ("name".to_string(), HirType::Json)
            {
                return Ok(value);
            }
            if actual_fields.as_slice() != declared_fields.as_slice()
                && actual_fields.iter().any(|(name, _)| is_hidden_accessor_field(name))
            {
                if let HirExpr::ObjectLit(fields) = &value {
                    return self.reorder_object_literal_in_source_order(
                        fields, declared_fields, true,
                    );
                }
                // The literal's evaluation-order temporaries wrap the object
                // literal in a `Call(Lambda ..)`; its storage still interleaves
                // the accessor closures with the data fields, so permute the
                // inner literal into declared order (accessor closures last).
                if let Some(reordered) = Self::reorder_wrapped_accessor_literal(&value, declared_fields) {
                    return Ok(reordered);
                }
                let visible = actual_fields
                    .iter()
                    .filter(|(name, _)| !is_hidden_accessor_field(name))
                    .collect::<Vec<_>>();
                if visible.len() == declared_fields.len()
                    && visible.iter().zip(declared_fields).all(|((name, actual), (declared, expected))|
                        name == declared && actual == expected) {
                    return Ok(value);
                }
                let compatible = declared_fields.iter().all(|(declared, expected)|
                    visible.iter().any(|(name, actual)| name == declared && actual == expected));
                if compatible && Self::fixed_object_supports_live_projection(
                    &HirType::Object(actual_fields.clone())) {
                    let prefix = actual_fields.starts_with(declared_fields);
                    return self.retain_full_native_object_projection(value, &actual_fields, !prefix);
                }
                return Err("cannot reinterpret a non-literal accessor object layout".into());
            }
            if actual_fields.starts_with(declared_fields) {
                if actual_fields.len() > declared_fields.len()
                    && Self::fixed_object_supports_live_projection(
                        &HirType::Object(actual_fields.clone())) {
                    return self.retain_full_native_object_projection(value, &actual_fields, false);
                }
                return Ok(value);
            }
            if HirType::Object(actual_fields.clone()) == date_object_type() {
                return Err("native Date cannot be reinterpreted as a different fixed object layout".into());
            }
            let same_fields = declared_fields.iter().all(|(declared, expected)|
                actual_fields.iter().any(|(name, actual)| name == declared && actual == expected));
            // A class instance whose fields are all plain data (no methods,
            // accessors or nested class instances) passes to a structural
            // parameter as a live view: reads and writes go to the instance.
            let plain_class_data = matches!(actual_fields.first(), Some((marker, HirType::Bool))
                    if marker.starts_with("__thaw_class_identity_\u{1e}"))
                && Self::fixed_object_supports_live_projection(
                    &HirType::Object(actual_fields[1..].to_vec()));
            if same_fields && !matches!(value, HirExpr::ObjectLit(_))
                && (plain_class_data || Self::fixed_object_supports_live_projection(
                    &HirType::Object(actual_fields.clone()))) {
                return self.retain_full_native_object_projection(value, &actual_fields, true);
            }
            if let Some(class) = class_name_from_type(&HirType::Object(actual_fields.clone())) {
                let compatible = declared_fields.iter().all(|(name, expected)| {
                    if actual_fields
                        .iter()
                        .any(|(actual_name, actual)| actual_name == name && actual == expected)
                    {
                        return true;
                    }
                    self.class_method_signature_compatible(class, name, expected, false)
                });
                if compatible {
                    // Interface members satisfied by prototype methods are not in the instance
                    // layout, so the interface view needs bound closures, not the raw pointer.
                    let provided_by_method = |name: &String, expected: &HirType| {
                        matches!(expected, HirType::Function(..))
                            && !actual_fields.iter().any(|(n, _)| n == name)
                            && self.signatures.contains_key(&class_method_symbol(class, name))
                    };
                    if declared_fields.iter().any(|(n, t)| provided_by_method(n, t)) {
                        self.class_method_view_made = true;
                        return self.live_widened_structural_view(value, &actual_fields, declared_fields);
                    }
                    return Ok(value);
                }
                // A structural target naming accessor properties of the class
                // is a fresh snapshot: each field is read through the class's
                // own member access (getters run now, so a throwing getter
                // propagates its original exception to the converting site).
                let object_type = HirType::Object(actual_fields.clone());
                // Every declared field is plain class data and some are
                // widened (`Optional`) because they may be uninitialised: the
                // structural view is an accessor object over the instance, so
                // reads see the current value and writes reach the instance.
                let widened = |name: &String, expected: &HirType| actual_fields.iter().find_map(|(n, actual)| {
                    (n == name).then(|| match actual {
                        HirType::Optional(payload) => payload.as_ref() == expected && is_uninit_class_field(&actual_fields, name),
                        other => other == expected,
                    })
                });
                if declared_fields.iter().all(|(name, expected)| widened(name, expected) == Some(true))
                    && declared_fields.iter().any(|(name, expected)|
                        actual_fields.iter().any(|(n, actual)| n == name && actual != expected))
                {
                    return self.live_widened_structural_view(value, &actual_fields, declared_fields);
                }
                let binding = format!("__thaw_accessor_source_{}", self.next_binding);
                let mut reads = Vec::with_capacity(declared_fields.len());
                let mut uses_accessor = false;
                for (name, expected) in declared_fields {
                    if let Some((_, actual)) = actual_fields.iter().find(|(n, _)| n == name) {
                        // A field widened to `Optional` because it may be
                        // uninitialised no longer shares the interface's
                        // layout: the structural view is a snapshot whose
                        // absent value reads like `undefined` coerced to `T`.
                        let uninit = matches!(actual, HirType::Optional(payload) if payload.as_ref() == expected)
                            && is_uninit_class_field(&actual_fields, name);
                        if actual != expected && !uninit {
                            reads.clear();
                            break;
                        }
                        let read = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(binding.clone())),
                            object_type.clone(),
                            name.clone(),
                        );
                        if uninit {
                            uses_accessor = true;
                            reads.push((name.clone(), self.lenient_uninit_read(read)?));
                        } else {
                            reads.push((name.clone(), read));
                        }
                        continue;
                    }
                    let Some(owner) = self.class_instance_accessor_owner(&object_type, name) else {
                        reads.clear();
                        break;
                    };
                    let symbol = class_getter_symbol(&owner, name, false);
                    if self.signatures.get(&symbol).is_none_or(|s| &s.ret != expected) {
                        reads.clear();
                        break;
                    }
                    uses_accessor = true;
                    reads.push((name.clone(), HirExpr::Call(
                        Box::new(HirExpr::Var(symbol)),
                        vec![self.assert_class_accessor_receiver(
                            HirExpr::Var(binding.clone()), &owner)],
                    )));
                }
                if uses_accessor && reads.len() == declared_fields.len() {
                    self.next_binding += 1;
                    self.scope.insert(binding.clone(), object_type.clone());
                    let literal = HirExpr::ObjectLit(reads);
                    return self.wrap_call_argument_bindings(
                        literal,
                        &[(binding, object_type, value)],
                    );
                }
            }
        }
        if *declared == HirType::Json {
            let actual = self.infer_expr_type(&value)?;
            if matches!(&actual, HirType::Array(element) if matches!(element.as_ref(), HirType::Promise(_))) {
                return Err("cannot coerce a native Promise array to a dynamic (any) value".into());
            }
            // A `Dictionary` is physically the same boxed Json object (element writes already
            // encode to Json), so crossing into `any` is an alias: wrapping it in a temporary
            // object would hand its box to an owning setter and free it with the temporary.
            if actual == HirType::Json || matches!(actual, HirType::Dictionary(_)) {
                return Ok(value);
            }
            if actual == crate::caught_exception_carrier_type() {
                return self.caught_carrier_as_json(value);
            }
            // A `JsValue` (an opaque handle to a live QuickJS-retained
            // object, e.g. zod's `z.string()` returning a `ZodString`
            // schema instance) has no direct JSON representation. Wrapped
            // via `HirExpr::JsValueAsJson` -- a real, valid `Json` value
            // (the same `{"__thaw_js_handle_id__": <id>}` placeholder
            // object `compile_dynamic_value_placeholder` already builds
            // for a `JsValue` crossing into a dynamic call's own JSON
            // argument array; the QuickJS-side reviver splices the real
            // live value back in whenever this placeholder is later
            // JSON-parsed there, same as it already does for a `Date`) --
            // rather than left as the raw, unwrapped handle. `HirType::
            // Json` and `HirType::JsValue` have different native layouts
            // (an opaque pointer to a boxed `serde_json::Value` vs. a
            // plain `i64` handle): simply returning `value` unchanged here
            // produced a value whose own inferred type stayed `JsValue`
            // while the declared slot said `Json` -- undetected by a
            // `let`/`const` declaration (nothing re-validates a coerced
            // initializer's own type against the variable's declared
            // one), and a segfault the moment anything later dereferenced
            // the raw handle bits as if they were a real `Json` pointer
            // (real trigger: csv-parse's `parser.read(): Json` assigned
            // to an already-`Json`-typed local, then passed to `Array.
            // push`/`JSON.stringify`). thaw-llvm's own dynamic-call
            // argument marshaling (`json_bridge.rs`'s own call sites)
            // still separately detects a bare `JsValue` handle reaching a
            // `Json`-declared parameter slot by LLVM value kind (an `i64`
            // where a pointer is expected) and wraps it the same way --
            // this node just does the identical wrapping earlier, at
            // `coerce_to_declared` time, so every other consumer (not
            // just a dynamic call's own argument list) gets a genuinely
            // valid `Json` value too.
            //
            // A statically-`JsValue`-typed value can still turn out, at
            // runtime, to genuinely *be* `null` or `undefined` (real
            // trigger: `Readable.read(): Json`, whose real runtime
            // result -- despite the declared type -- is `null` once the
            // stream is exhausted, the exact value Node's own docs say
            // to loop on: `while ((record = parser.read()) !== null)`).
            // The placeholder-object encoding above is only correct for
            // a genuinely live object; `null`/`undefined` need the real
            // `Json` literal instead, or `record !== null` would never
            // observe termination (a wrapped placeholder object is never
            // `===` to a literal `null`) -- an infinite loop, not a
            // crash, but just as wrong. Checked at runtime (the same
            // live-engine query `dynamic_value_is_null`/`_undefined`
            // already use for `JsValue === null`/`undefined` comparisons
            // elsewhere) since nothing static can know which case a
            // given call actually returns.
            if actual == HirType::JsValue && crate::lower::thaw_js_routed() {
                // A thaw-js build's `JsValue` is a handle into the thaw-js heap, never null.
                return Ok(HirExpr::JsValueAsJson(Box::new(value)));
            }
            if actual == HirType::JsValue {
                let temp = format!("__thaw_json_wrap_source_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(temp.clone(), HirType::JsValue);
                let null_json =
                    self.wrap_native_value_as_json(HirExpr::Lit(HirLit::Null), HirType::Null)?;
                let undefined_json = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![(
                            "$__thaw_napi_undefined$".to_string(),
                            HirExpr::Lit(HirLit::Bool(true)),
                        )],
                        HirType::Bool,
                    )],
                );
                let is_null = self.dynamic_value_is_null(HirExpr::Var(temp.clone()));
                let is_undefined = self.dynamic_value_is_undefined(HirExpr::Var(temp.clone()));
                let body = HirExpr::Conditional(
                    Box::new(is_null),
                    Box::new(null_json),
                    Box::new(HirExpr::Conditional(
                        Box::new(is_undefined),
                        Box::new(undefined_json),
                        Box::new(HirExpr::JsValueAsJson(Box::new(HirExpr::Var(temp.clone())))),
                        HirType::Json,
                    )),
                    HirType::Json,
                );
                return self.wrap_call_argument_bindings(body, &[(temp, HirType::JsValue, value)]);
            }
            // A real compiled (native) closure (e.g. `(n: number) => n >
            // 0` passed to zod's `z.number().refine(...)`) has no JSON
            // representation either -- but unlike `JsValue` above, it
            // can't just pass through unchanged: there is no existing
            // *live* value on the QuickJS side to reference yet, only a
            // native function pointer thaw-llvm needs to bridge into one.
            // Wrapped in a manual `registerNativeCallback` call (the same
            // convention `getDynamicProperty`/`callDynamicMethod`/etc.
            // already use for a hand-built intrinsic call thaw-llvm
            // recognizes by name -- see `compile_register_native_
            // callback`, thaw-llvm's `dynamic_host.rs`), which builds a
            // native-to-JS adapter (reusing the existing N-API `compile_
            // napi_value_callback`, itself backend-agnostic) and registers
            // it as a real, retained QuickJS value -- yielding an ordinary
            // `JsValue` handle from here on, so everything downstream
            // (`compile_dynamic_value_placeholder`'s own `is_int_value()`
            // detection, the args-JSON reviver, ...) treats it exactly
            // like any other live value crossing into a dynamic call.
            // A generator's internal "producer" function (first parameter an
            // `i64` resume mode, see `is_generator_producer_type`) is never
            // handed to the generic native-closure wrapping below.  It is
            // reified as a protocol object (`next`/`return`/`throw`
            // lambdas that call the producer) so the `any` view behaves
            // like a JS Generator.
            if is_generator_producer_type(&actual) {
                if !matches!(&actual, HirType::Function(_, result) if matches!(result.as_ref(), HirType::Array(_))) {
                    return Err("cannot coerce an async Generator to a dynamic (any) value".into());
                }
                let mut bindings = Vec::new();
                let object = self.generator_protocol_object(
                    &value, &actual, swc_common::DUMMY_SP, swc_common::SyntaxContext::empty(),
                    &mut bindings, true)?;
                let object = self.wrap_call_argument_bindings(object, &bindings)?;
                let object = self.coerce_to_declared(&HirType::Json, object)?;
                // `g[Symbol.iterator]()` returns the generator itself.
                let name = format!("__thaw_generator_any_{}", self.next_binding);
                self.next_binding += 1;
                let iterator = HirExpr::Lambda(
                    vec![HirParam { name: name.clone(), ty: HirType::Json }],
                    Vec::new(),
                    HirType::Json,
                    Box::new(HirExpr::Var(name.clone())),
                );
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam { name: name.clone(), ty: HirType::Json }],
                        HirType::Json,
                        Box::new(HirExpr::Block(vec![
                            HirStmt::Expr(HirExpr::JsonSet(
                                Box::new(HirExpr::Var(name.clone())),
                                Box::new(HirExpr::Lit(HirLit::Str("\u{1f}@@iterator".into()))),
                                Box::new(iterator),
                                HirType::Function(Vec::new(), Box::new(HirType::Json)),
                                false,
                            )),
                            HirStmt::Return(Some(HirExpr::Var(name))),
                        ])),
                    )),
                    vec![object],
                ));
            }
            // A native `bigint` is a Json BigInt, the same value an array/object push builds.
            if matches!(actual, HirType::I64 | HirType::BigInt) {
                return self.wrap_native_value_as_json(value, actual);
            }
            if matches!(&actual, HirType::Function(_, _) | HirType::CallableFunction(..) | HirType::FunctionWithThis(_, _)) {
                // `registerNativeCallback`'s own inferred type is always
                // `HirType::JsValue` (`inference/types.rs`'s hardcoded
                // intrinsic-name case) -- a raw `i64` handle, not the
                // pointer representation a genuinely `Json`-typed value
                // needs. Reaching a dynamic call's own JSON argument
                // array/object field already tolerates this raw shape
                // (that call site's own codegen separately detects an
                // `i64` reaching a `Json`-declared slot via `is_int_
                // value()` and wraps it there, see `compile_json_
                // object_set_native_with_undefined`'s `HirType::Json`
                // arm) -- but nothing else does. A `let`/`const c: any
                // = someFunctionOrClass;` (real trigger: any plain
                // function *or* class reference -- `new User(...)`
                // itself desugars to an ordinary function call, so a
                // bare `User` reference hits this exact branch too --
                // coerced into an `any`-typed local) stored this raw
                // `i64` directly into a slot whose declared type says
                // `Json` (a pointer), corrupting it -- a real,
                // reproducible segfault the moment anything (`typeof`,
                // `console.log`, ...) later read that slot back as if
                // it held a real `serde_json::Value` pointer. `JsValueAsJson`
                // (the same wrapping the sibling `actual == JsValue`
                // case below already applies) produces the identical
                // `{"__thaw_js_handle_id__": id}` placeholder `compile_
                // dynamic_value_placeholder`'s own `is_int_value()` path
                // would otherwise build downstream -- doing it here
                // instead makes every consumer correct, the dynamic-
                // call-argument case included (its own `is_int_value()`
                // check now simply sees an already-wrapped pointer and
                // skips, never double-wrapping).
                let name = format!("__thaw_function_json_source_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), actual.clone());
                let bound = HirExpr::Var(name.clone());
                let absent = self.coerce_to_declared(
                    &HirType::Json, HirExpr::Lit(HirLit::Undefined),
                )?;
                let (callback, callback_type) = if matches!(actual, HirType::Function(_, _)) {
                    self.adapt_callable_result_for_live_projection(bound.clone(), actual.clone())?
                } else {
                    (bound.clone(), actual.clone())
                };
                let graph_callback = callback_type != actual;
                let callback_type = self.rest_callable_type(&value, callback_type);
                let registered = HirExpr::Call(
                    Box::new(HirExpr::Var(if graph_callback {
                        "registerNativeCallbackGraph".to_string()
                    } else {
                        "registerNativeCallback".to_string()
                    })),
                    vec![HirExpr::TypedClosure(callback_type, Box::new(callback))],
                );
                let present = if graph_callback {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                        vec![registered],
                    )
                } else {
                    HirExpr::JsValueAsJson(Box::new(registered))
                };
                let result = HirExpr::Conditional(
                    Box::new(Self::function_pointer_is_undefined(bound, &actual)),
                    Box::new(absent), Box::new(present), HirType::Json,
                );
                return self.wrap_call_argument_bindings(result, &[(name, actual, value)]);
            }
            // A bare `undefined` literal (`schema.safeParse(undefined)`,
            // real zod) has no JSON representation either -- JSON has no
            // `undefined`, only the case a nested field can omit itself
            // entirely (already handled: see `compile_json_object_set_
            // native_with_undefined`'s own `HirType::Undefined => Ok(())`
            // arm), which doesn't apply to a *standalone* value with no
            // field to omit. Encoded instead as the same `{"$__thaw_
            // napi_undefined$": true}` sentinel `compile_napi_undefined_
            // json` already uses for a NAPI return value -- a shared,
            // already-recognized-in-principle shape rather than a new
            // one, even though this crosses a different boundary
            // (`callDynamic`'s JSON argument array, not a NAPI result).
            // The branded wrapper becomes an `u` token in the trusted
            // graph codec. The plain JSON boundary treats the same user
            // object shape as ordinary data.
            // Built via `HirExpr::JsonObjectLit` -- an existing, already-
            // exercised construct (a dictionary literal, `Bool` element
            // type covers the one `true` field) -- rather than new
            // codegen, avoiding the same int-value-kind ambiguity with
            // `JsValue`'s own placeholder detection (both would compile
            // to a plain integer at the LLVM level) a bare pass-through
            // like the `JsValue` case above would risk. `value` itself
            // is still evaluated once for any side effect it might have
            // (`wrap_call_argument_bindings`), even though `Undefined`
            // has only one possible runtime value and the result doesn't
            // reference it.
            if actual == HirType::Undefined {
                let temp = format!("__thaw_json_undefined_source_{}", self.next_binding);
                self.next_binding += 1;
                let sentinel = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![(
                            "$__thaw_napi_undefined$".to_string(),
                            HirExpr::Lit(HirLit::Bool(true)),
                        )],
                        HirType::Bool,
                    )],
                );
                return self.wrap_call_argument_bindings(
                    sentinel,
                    &[(temp, HirType::Undefined, value)],
                );
            }
            // A native Map/Set that may be aliased stays one object behind `any`: a live view.
            if self.alias_native_arrays && Self::native_collection_view_supported(&actual) {
                let fresh = matches!(&value, HirExpr::Call(callee, _)
                        if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_map_new"))
                    || matches!(&value, HirExpr::Var(name) if self.fresh_object_bindings.contains(name));
                if !fresh {
                    if let Ok(view) = self.lower_native_collection_as_dynamic_view(value.clone(), &actual) {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                            vec![view],
                        ));
                    }
                }
            }
            if let HirType::Map(key, element) = &actual {
                if !json_convertible_native_type(key) || !json_convertible_native_type(element) {
                    return Err(format!(
                        "Map<{key:?}, {element:?}> cannot cross a dynamic boundary"
                    ));
                }
                let entries_type = HirType::Array(Box::new(HirType::Tuple(vec![
                    key.as_ref().clone(),
                    element.as_ref().clone(),
                ])));
                let entries = HirExpr::TypedClosure(
                    entries_type.clone(),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_entries".into())),
                        vec![value],
                    )),
                );
                let entries = self.wrap_native_value_as_json(entries, entries_type)?;
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![("__thaw_map_entries__".into(), entries)],
                        HirType::Json,
                    )],
                ));
            }
            if let HirType::Set(element) = &actual {
                if !json_convertible_native_type(element) {
                    return Err(format!(
                        "Set<{element:?}> cannot cross a dynamic boundary"
                    ));
                }
                let values_type = HirType::Array(element.clone());
                let values = HirExpr::TypedClosure(
                    values_type.clone(),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                        vec![value],
                    )),
                );
                let values = self.wrap_native_value_as_json(values, values_type)?;
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![("__thaw_set_values__".into(), values)],
                        HirType::Json,
                    )],
                ));
            }
            if actual == regex_object_type() {
                let pattern = self.wrap_native_value_as_json(value, actual)?;
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![("__thaw_regexp__".into(), pattern)],
                        HirType::Json,
                    )],
                ));
            }
            // A `Buffer`/`Uint8Array` (`HirType::Bytes`) has no direct
            // JSON representation either -- real trigger: archiver's own
            // `Archiver.append(source: Readable | Buffer | string, ...)`,
            // called with a real `Buffer.from(...)` value. `Bytes` is a
            // lowering-time-only distinction (`lower/bytes_erasure.rs`
            // rewrites every one, program-wide, to a plain `Array(F64)`
            // *after* lowering finishes) -- deliberately *not* added to
            // `json_convertible_native_type`'s own generic native-array
            // path below, since that would just JSON-encode the bytes as
            // a bare `[n1, n2, ...]` array, indistinguishable from an
            // ordinary `number[]` argument and silently losing "this was
            // a Buffer" the same way it already was. Wrapped instead as
            // the same `{"type":"Buffer","data":[...]}` shape Node's own
            // `Buffer.prototype.toJSON`/`JSON.stringify` already use --
            // The trusted graph codec recognizes this branded shape as a
            // Buffer node while leaving an ordinary user object with the
            // same keys alone. The `data` field's own elements erasing to plain
            // JSON numbers is correct and expected here -- the `"type"`
            // wrapper is what carries "this was a Buffer", not the
            // erased element type, so losing `Bytes` as a type tag after
            // lowering doesn't lose the information this fix depends on.
            // `infer_expr_type` (used for `actual` above) deliberately
            // normalizes `Bytes` to `Array(F64)` for every other consumer
            // (see its own doc comment) -- `infer_expr_type_inner` is the
            // one place that still sees the real, un-normalized tag,
            // needed here specifically to tell a `Buffer`/`Uint8Array`
            // apart from an ordinary `number[]` of the same runtime
            // layout.
            if self.infer_expr_type_inner(&value)? == HirType::Bytes {
                let widened = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_to_array".into())),
                    vec![value],
                );
                let data = self.wrap_native_value_as_json(widened, HirType::Array(Box::new(HirType::F64)))?;
                let kind = self.wrap_native_value_as_json(
                    HirExpr::Lit(HirLit::Str("Buffer".to_string())),
                    HirType::Str,
                )?;
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_brand_wrapper".into())),
                    vec![HirExpr::JsonObjectLit(
                        vec![("type".into(), kind), ("data".into(), data)],
                        HirType::Json,
                    )],
                ));
            }
            if let HirType::Object(fields) = &actual {
                if !object_type_is_error_family(&actual)
                    && self.class_instance_supports_live_methods(&actual)
                {
                    // Same copy-or-live policy as a plain object below, but the
                    // view also carries the class's prototype methods.
                    let live = !(matches!(value, HirExpr::ObjectLit(_))
                        || Self::is_wrapped_object_literal(&value)
                        || matches!(&value, HirExpr::Var(name)
                            if self.fresh_object_bindings.contains(name)));
                    let projected = self.lower_fixed_object_as_dynamic_accessor_object(
                        value, fields, false, live, true,
                    )?;
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                        vec![projected],
                    ));
                }
            }
            // An instance of a *decorated* class (see `module/globals.rs`'s
            // `lower_class_decorator_tokens`/`DECORATOR_CLASS_TOKENS`) gets
            // one extra `constructor` field pointing at that class's own
            // real, live "class token" `JsValue` -- the exact same token a
            // decorator's own `target` argument was built from
            // (`lower_member_decorator_call`/`lower_class_decorator_call`).
            // That shared identity is what lets real `object.constructor`-
            // keyed metadata storage (class-validator's own
            // `MetadataStorage`, keyed by real `Map` reference equality)
            // find its own registration again once the instance crosses
            // this same dynamic-call boundary a second time (e.g.
            // `validate(instance)`). Scoped to only classes that actually
            // have a decorator -- an ordinary class's instances keep
            // marshaling exactly as before, so this can't regress any
            // existing (pre-decorator) dynamic-call argument shape.
            // Error-family allocations must reach the live pointer-keyed
            // projection below; a decorator snapshot would split their
            // caught/Promise aliases from the original object.
            if let HirType::Object(fields) = &actual {
              if !object_type_is_error_family(&actual) {
                if let Some(token_symbol) = fields.first().and_then(|(marker, _)| {
                    DECORATOR_CLASS_TOKENS.with(|tokens| tokens.borrow().get(marker).cloned())
                }) {
                    let object_json = self.wrap_native_value_as_json(value, actual.clone())?;
                    let temp = format!("__thaw_decorator_instance_json_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(temp.clone(), HirType::Json);
                    let token_as_json =
                        self.coerce_to_declared(&HirType::Json, HirExpr::Var(token_symbol))?;
                    let result = HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::JsonSet(
                            Box::new(HirExpr::Var(temp.clone())),
                            Box::new(HirExpr::Lit(HirLit::Str("constructor".to_string()))),
                            Box::new(token_as_json),
                            HirType::Json,
                            false,
                        )),
                        HirStmt::Return(Some(HirExpr::Var(temp.clone()))),
                    ]);
                    return self.wrap_call_argument_bindings(
                        result,
                        &[(temp, HirType::Json, object_json)],
                    );
                }
              }
            }
            if let HirType::Object(fields) = &actual {
                // A plain fixed object crossing into Json can be borrowed by
                // a dynamic caller. Project field operations over the source
                // arena pointer and let the existing JsValue-to-Json wrapper
                // keep the live identity; class instances retain their
                // dedicated nominal/Date conversion path.
                // An Error-family ClassAlloc is also an identity-bearing
                // object: a Json snapshot would turn Promise rejection
                // aliases into unrelated plain records. Keep its original
                // pointer behind the live Host projection instead.
                if (object_type_is_error_family(&actual)
                    || !fields.iter().any(|(name, _)|
                        name.starts_with("__thaw_class_identity_\u{1e}")))
                    && self.live_projection_supported_with_classes(&actual) {
                    // A fresh `any` literal converts to a mutable native copy; a projection of an
                    // existing native owner stays a live reference to it.
                    let live = !(matches!(value, HirExpr::ObjectLit(_))
                        || Self::is_wrapped_object_literal(&value)
                        || matches!(&value, HirExpr::Var(name)
                            if self.fresh_object_bindings.contains(name)));
                    // A fresh literal handed to a thaw-js package is a plain-data copy: no QuickJS-hosted
                    // projection is needed (a live reference to an existing owner keeps the projection).
                    if !live && crate::lower::thaw_js_routed() {
                        return self.wrap_native_value_as_json(value, actual.clone());
                    }
                    let projected = self.lower_fixed_object_as_dynamic_accessor_object(value, fields, false, live, false)?;
                    // A projected object lookup yields a newly retained JS
                    // handle. The consuming Json conversion applies the
                    // runtime's copy-or-live policy and releases that handle.
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                        vec![projected],
                    ));
                }
            }
            if self.alias_native_arrays {
                if let HirType::Array(element) = &actual {
                    // A fresh literal is just a copy, unless its elements are class instances whose
                    // methods and identity must stay reachable through the array.
                    let fresh = (matches!(value, HirExpr::ArrayLit(_))
                            && !self.class_instance_supports_live_methods(element))
                        || matches!(&value, HirExpr::Var(name) if self.fresh_object_bindings.contains(name));
                    if !fresh {
                        if let Ok(view) = self.lower_native_array_as_dynamic_view(value.clone(), element) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                                vec![view],
                            ));
                        }
                    }
                }
            }
            if json_convertible_native_type(&actual) {
                return self.wrap_native_value_as_json(value, actual);
            }
        }
        if matches!(declared, HirType::Function(..)) {
            if let Some(adapted) = self.adapt_overloaded_callable_value(declared, value.clone())? {
                return Ok(adapted);
            }
        }
        if let Some(adapted) = self.adapt_named_function_to_required_function(declared, &value)? {
            return Ok(adapted);
        }
        if let Some(adapted) = self.adapt_named_function_to_callable(declared, &value)? {
            return Ok(adapted);
        }
        if let HirType::CallableFunction(fixed, _, rest, ret) = declared {
            let actual = self.infer_expr_type(&value)?;
            let mut abi_params = fixed.clone();
            if let Some(rest) = rest {
                abi_params.push(HirType::Array(rest.clone()));
            }
            if actual == HirType::Function(abi_params, ret.clone()) {
                return Ok(HirExpr::TypedClosure(declared.clone(), Box::new(value)));
            }
        }
        if let HirType::Dictionary(element) = declared {
            if self.infer_expr_type(&value)? == *declared {
                return Ok(value);
            }
            let actual = self.infer_expr_type(&value)?;
            // `Dictionary<T>` shares `Json`'s own native layout at the
            // LLVM level (see `compile_json_value_to_native`'s own
            // `HirType::Dictionary(_) => Ok(json)` case, thaw-llvm's
            // `json_bridge/decoding.rs`) -- decoding a `Json`/`JsValue`
            // source into a `Dictionary` target is a pure type-level
            // relabeling, the same "readDynamicValue, then pass through
            // unchanged" shape the `Json`-target branch above already
            // uses for a `JsValue` source. Without this, a `Dictionary`-
            // declared slot fed a genuinely dynamic value (e.g. `new
            // Proxy(target, handler)`'s own `target` re-read through a
            // live QuickJS handle) hit the `ObjectLit`-only check below
            // and hard-errored instead.
            if matches!(actual, HirType::Json | HirType::JsValue) {
                return if actual == HirType::JsValue {
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    ))
                } else {
                    Ok(value)
                };
            }
            // A dictionary of arrays (`Object.groupBy`) is the same Json-backed value as the
            // `Dictionary(Json)` that `Record<string, T[]>` now lowers to (see `dictionary_hir_type`).
            if **element == HirType::Json
                && matches!(&actual, HirType::Dictionary(inner) if matches!(inner.as_ref(), HirType::Array(_)))
            {
                return Ok(value);
            }
            return self.coerce_object_literal_to_dictionary(element, value);
        }
        // A referenced union alias can nest an absence wrapper inside
        // another wrapper or a union. Flatten those tags before assigning
        // the canonical result layout; each branch still owns one value.
        if matches!(declared, HirType::Union(_) | HirType::Optional(_)
            | HirType::Nullable(_) | HirType::Nullish(_)) {
            let actual = self.infer_expr_type(&value)?;
            let nested_wrapper = match &actual {
                HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
                    matches!(inner.as_ref(), HirType::Optional(_) | HirType::Nullable(_)
                        | HirType::Nullish(_))
                    || matches!(inner.as_ref(), HirType::Union(parts)
                        if parts.iter().any(|part| matches!(part, HirType::Optional(_)
                            | HirType::Nullable(_) | HirType::Nullish(_))))
                }
                _ => false,
            };
            let union_into_wrapper = matches!((&actual, declared),
                (HirType::Union(_), HirType::Optional(_) | HirType::Nullable(_)
                    | HirType::Nullish(_)));
            let union_with_nested_wrapper = matches!((&actual, declared),
                (HirType::Union(parts), HirType::Union(_)) if parts.iter().any(|part|
                    matches!(part, HirType::Optional(_) | HirType::Nullable(_)
                        | HirType::Nullish(_))));
            let direct_member = matches!(declared, HirType::Union(elements)
                if elements.contains(&actual));
            if actual != *declared && !direct_member && (nested_wrapper || union_into_wrapper || union_with_nested_wrapper) {
                return self.coerce_composite_to_declared(declared, value, actual);
            }
        }
        if let HirType::Union(elements) = declared {
            let actual = self.infer_expr_type(&value)?;
            if &actual == declared {
                return Ok(value);
            }
            // The Promise constructor's mixed resolver has one value arm and
            // one adoption arm. A value may itself need the usual native
            // absence/union coercion before it can occupy the value arm.
            if let [resolved, HirType::Promise(promised)] = elements.as_slice() {
                if promised.as_ref() == resolved {
                    if promise_resolve_value_fits(&actual, resolved) {
                        let value = self.coerce_to_declared(resolved, value)?;
                        return Ok(HirExpr::UnionInject(
                            Box::new(value), 0, elements.clone(),
                        ));
                    }
                    if let HirType::Promise(actual_result) = &actual {
                        if actual_result.as_ref() != resolved
                            && !matches!(resolved, HirType::Void)
                            && promise_resolve_value_fits(actual_result, resolved)
                        {
                            let name = format!("__thaw_promise_widen_{}", self.next_binding);
                            self.next_binding += 1;
                            let previous = self.scope.insert(name.clone(), actual_result.as_ref().clone());
                            let converted = self.coerce_to_declared(resolved, HirExpr::Var(name.clone()));
                            match previous {
                                Some(previous) => { self.scope.insert(name.clone(), previous); }
                                None => { self.scope.remove(&name); }
                            }
                            let callback = HirExpr::Lambda(
                                Vec::new(),
                                vec![HirParam { name, ty: actual_result.as_ref().clone() }],
                                resolved.clone(),
                                Box::new(converted?),
                            );
                            let widened = HirExpr::PromiseThen(
                                Box::new(value), Box::new(callback),
                                actual_result.as_ref().clone(), resolved.clone(), false, false,
                            );
                            return Ok(HirExpr::UnionInject(
                                Box::new(widened), 1, elements.clone(),
                            ));
                        }
                    }
                }
            }
            if let HirType::Union(source) = &actual {
                // Widen a source union as well as reordering equivalent
                // members. Logical expressions can add a right-hand type
                // while retaining every left-hand alternative.
                if source.iter().all(|member| elements.contains(member)) {
                    let parameter = "__thaw_union_retag_value".to_string();
                    let mut statements = Vec::with_capacity(source.len());
                    for (source_index, member) in source.iter().enumerate() {
                        let target_index = elements
                            .iter()
                            .position(|target| target == member)
                            .expect("equivalent union contains every source member");
                        let converted = HirExpr::UnionInject(
                            Box::new(HirExpr::UnionValue(
                                Box::new(HirExpr::Var(parameter.clone())),
                                source_index,
                                source.clone(),
                            )),
                            target_index,
                            elements.clone(),
                        );
                        if source_index + 1 == source.len() {
                            statements.push(HirStmt::Return(Some(converted)));
                        } else {
                            statements.push(HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(parameter.clone())),
                                        source.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(source_index as f64))),
                                ),
                                vec![HirStmt::Return(Some(converted))],
                                Vec::new(),
                            ));
                        }
                    }
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam {
                            name: parameter,
                            ty: actual,
                        }],
                        declared.clone(),
                        Box::new(HirExpr::Block(statements)),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            if let Some(index) = elements.iter().position(|element| element == &actual) {
                return Ok(HirExpr::UnionInject(
                    Box::new(value),
                    index,
                    elements.clone(),
                ));
            }
            // A `T | null` / `T | undefined` (`T | null | undefined`)
            // value flowing into a wider union that already covers every
            // one of those cases -- e.g. a `.d.ts` overload whose own
            // return is `string | null` reaching the merged
            // `string | number | null` signature thaw builds for the
            // whole function. Re-tag it: test the presence flag, then
            // `UnionInject` the payload or the absent literal into the
            // matching slot.
            if let Some(retagged) = self.retag_nullish_into_union(&actual, elements, &value)? {
                return Ok(retagged);
            }
            if matches!(value, HirExpr::ObjectLit(_)) || Self::is_wrapped_object_literal(&value) {
                for (index, member) in elements.iter().enumerate() {
                    if !matches!(member, HirType::Object(_)) {
                        continue;
                    }
                    if let Ok(adapted) = self.coerce_to_declared(member, value.clone()) {
                        return Ok(HirExpr::UnionInject(
                            Box::new(adapted),
                            index,
                            elements.clone(),
                        ));
                    }
                }
            }
            // A live `JsValue` handle reaching a union that has a `Json` member
            // (`resolve(value)` of a `Promise<Json>`): wrap it as `Json` first.
            if actual == HirType::JsValue {
                if let Some(index) = elements.iter().position(|element| element == &HirType::Json) {
                    let json = self.coerce_to_declared(&HirType::Json, value)?;
                    return Ok(HirExpr::UnionInject(Box::new(json), index, elements.clone()));
                }
            }
            return Err(format!(
                "value has type {actual:?}, which is not a member of {declared:?}"
            ));
        }
        if let HirType::Optional(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Undefined => Ok(absent_after(
                    value, HirExpr::OptionalNone(payload.as_ref().clone()),
                )),
                HirType::Void if matches!(payload.as_ref(), HirType::Promise(result)
                    if result.as_ref() == &HirType::Void) => {
                    Ok(absent_after(value, HirExpr::OptionalNone(payload.as_ref().clone())))
                }
                actual if actual == *declared => Ok(value),
                _ => {
                    let mut value = self.coerce_to_declared(payload.as_ref(), value)?;
                    // An empty `[]` infers as `number[]`; `OptionalSome` checks its
                    // operand against the payload, so type the empty array.
                    if let (HirExpr::ArrayLit(elements), HirType::Array(element)) =
                        (&value, payload.as_ref())
                    {
                        if elements.is_empty() && element.as_ref() != &HirType::F64 {
                            value = HirExpr::ArrayAlloc(
                                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                element.as_ref().clone(),
                            );
                        }
                    }
                    Ok(HirExpr::OptionalSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullable(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(absent_after(
                    value, HirExpr::NullableNone(payload.as_ref().clone()),
                )),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullableSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullish(payload) = declared {
            let actual = self.infer_expr_type(&value)?;
            // Widen a one-absence wrapper without losing which absence it
            // carried. This is needed when the two sides of `&&` contribute
            // null and undefined separately.
            if let HirType::Optional(source) | HirType::Nullable(source) = &actual {
                if source == payload {
                    let parameter = "__thaw_nullish_widen".to_string();
                    let left = HirExpr::Var(parameter.clone());
                    let (absent, absent_value, present_value) = match &actual {
                        HirType::Optional(_) => (
                            HirExpr::OptionalIsNone(Box::new(left.clone()), source.as_ref().clone()),
                            HirExpr::NullishUndefined(payload.as_ref().clone()),
                            HirExpr::OptionalValue(Box::new(left), source.as_ref().clone()),
                        ),
                        HirType::Nullable(_) => (
                            HirExpr::NullableIsNone(Box::new(left.clone()), source.as_ref().clone()),
                            HirExpr::NullishNull(payload.as_ref().clone()),
                            HirExpr::NullableValue(Box::new(left), source.as_ref().clone()),
                        ),
                        _ => unreachable!(),
                    };
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam { name: parameter, ty: actual }],
                        declared.clone(),
                        Box::new(HirExpr::Block(vec![
                            HirStmt::If(absent, vec![HirStmt::Return(Some(absent_value))], Vec::new()),
                            HirStmt::Return(Some(HirExpr::NullishSome(
                                Box::new(present_value), payload.as_ref().clone(),
                            ))),
                        ])),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            // `null | undefined` (`Optional(Null)`) carries only absences: the
            // wrapper's own tag says which one.
            if let HirType::Optional(source) = &actual {
                if **source == HirType::Null {
                    let parameter = "__thaw_nullish_widen".to_string();
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam { name: parameter.clone(), ty: actual.clone() }],
                        declared.clone(),
                        Box::new(HirExpr::Block(vec![
                            HirStmt::If(
                                HirExpr::OptionalIsNone(Box::new(HirExpr::Var(parameter)), HirType::Null),
                                vec![HirStmt::Return(Some(HirExpr::NullishUndefined(payload.as_ref().clone())))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::NullishNull(payload.as_ref().clone()))),
                        ])),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            return match actual {
                HirType::Null => Ok(absent_after(
                    value, HirExpr::NullishNull(payload.as_ref().clone()),
                )),
                HirType::Undefined => Ok(absent_after(
                    value, HirExpr::NullishUndefined(payload.as_ref().clone()),
                )),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullishSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let (HirType::Array(element), HirExpr::ArrayLit(values)) = (declared, &value) {
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    if matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                        return Ok(value.clone());
                    }
                    self.coerce_array_insert_value(value.clone(), element)
                        .map_err(|error| format!("array element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return self.lower_native_array_literal(values, element.as_ref().clone());
        }
        if let (HirType::Tuple(expected), HirExpr::ArrayLit(values)) = (declared, &value) {
            let required = expected
                .iter()
                .take_while(|ty| !matches!(ty, HirType::Optional(_)))
                .count();
            if values.len() < required || values.len() > expected.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {required}..={}",
                    values.len(),
                    expected.len()
                ));
            }
            let values = expected
                .iter()
                .zip(values.iter().cloned().chain(std::iter::repeat(HirExpr::Lit(
                    HirLit::Undefined,
                ))))
                .enumerate()
                .map(|(index, (expected, value))| {
                    self.coerce_to_declared(expected, value.clone())
                        .map_err(|error| format!("tuple element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        if matches!(declared, HirType::Object(_)) && Self::is_wrapped_object_literal(&value) {
            return self.coerce_wrapped_object_literal(declared, value);
        }
        let (HirType::Object(declared_fields), HirExpr::ObjectLit(lit_fields)) = (declared, &value)
        else {
            self.expect_type(declared, &value, "value")?;
            return Ok(value);
        };

        if lit_fields.len() > declared_fields.len()
            || lit_fields
                .iter()
                .any(|(name, _)| !declared_fields.iter().any(|(declared, _)| declared == name))
        {
            return Err(format!(
                "object literal has {} field(s), expected {} for this type",
                lit_fields.len(),
                declared_fields.len()
            ));
        }

        self.reorder_object_literal_in_source_order(lit_fields, declared_fields, false)
    }

    /// Normalize nested alias wrappers and unions one tag at a time. The
    /// value is captured as one lambda argument before any branch inspects
    /// it; recursive coercion only evaluates the selected payload.
    fn coerce_composite_to_declared(
        &mut self,
        declared: &HirType,
        value: HirExpr,
        source: HirType,
    ) -> Result<HirExpr, String> {
        let parameter = format!("__thaw_flatten_absence_{}", self.next_binding);
        self.next_binding += 1;
        let previous = self.scope.insert(parameter.clone(), source.clone());
        let result = (|| -> Result<HirExpr, String> {
            let current = HirExpr::Var(parameter.clone());
            let mut statements = Vec::new();
            match &source {
                HirType::Optional(payload) => {
                    let absent = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Undefined))?;
                    statements.push(HirStmt::If(HirExpr::OptionalIsNone(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(absent))], Vec::new()));
                    let inner = HirExpr::OptionalValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Nullable(payload) => {
                    let absent = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Null))?;
                    statements.push(HirStmt::If(HirExpr::NullableIsNone(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(absent))], Vec::new()));
                    let inner = HirExpr::NullableValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Nullish(payload) => {
                    let null = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Null))?;
                    statements.push(HirStmt::If(HirExpr::NullishIsNull(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(null))], Vec::new()));
                    let undefined = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Undefined))?;
                    statements.push(HirStmt::If(HirExpr::NullishIsUndefined(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(undefined))], Vec::new()));
                    let inner = HirExpr::NullishValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Union(members) => {
                    if members.is_empty() {
                        return Err("cannot coerce an empty union".into());
                    }
                    for (index, _member) in members.iter().enumerate() {
                        let inner = HirExpr::UnionValue(Box::new(current.clone()), index, members.clone());
                        let adapted = self.coerce_to_declared(declared, inner)?;
                        if index + 1 == members.len() {
                            statements.push(HirStmt::Return(Some(adapted)));
                        } else {
                            statements.push(HirStmt::If(HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(current.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64)))),
                                vec![HirStmt::Return(Some(adapted))], Vec::new()));
                        }
                    }
                }
                _ => unreachable!("composite coercion requires a wrapper or union"),
            }
            let adapter = HirExpr::Lambda(Vec::new(), vec![HirParam { name: parameter.clone(), ty: source }],
                declared.clone(), Box::new(HirExpr::Block(statements)));
            Ok(HirExpr::Call(Box::new(adapter), vec![value]))
        })();
        match previous {
            Some(previous) => { self.scope.insert(parameter, previous); }
            None => { self.scope.remove(&parameter); }
        }
        result
    }

    /// Re-tags an `Optional`/`Nullable`/`Nullish` value into a `Union`
    /// that already contains its payload type and its absent form(s)
    /// (`Undefined` / `Null`). Returns `None` when `actual` isn't one of
    /// those, or the union doesn't fully cover it (so the caller can fall
    /// through to its existing error).
    fn retag_nullish_into_union(
        &mut self,
        actual: &HirType,
        elements: &[HirType],
        value: &HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let slot = |ty: &HirType| elements.iter().position(|element| element == ty);
        // (payload type, absent-form arms as (is-none test, injected literal))
        let (payload, arms): (&HirType, Vec<(HirExpr, HirLit)>) = match actual {
            HirType::Nullable(payload) => (
                payload.as_ref(),
                vec![(
                    HirExpr::NullableIsNone(
                        Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                        payload.as_ref().clone(),
                    ),
                    HirLit::Null,
                )],
            ),
            HirType::Optional(payload) => (
                payload.as_ref(),
                vec![(
                    HirExpr::OptionalIsNone(
                        Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                        payload.as_ref().clone(),
                    ),
                    HirLit::Undefined,
                )],
            ),
            HirType::Nullish(payload) => (
                payload.as_ref(),
                vec![
                    (
                        HirExpr::NullishIsNull(
                            Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                            payload.as_ref().clone(),
                        ),
                        HirLit::Null,
                    ),
                    (
                        HirExpr::NullishIsUndefined(
                            Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                            payload.as_ref().clone(),
                        ),
                        HirLit::Undefined,
                    ),
                ],
            ),
            _ => return Ok(None),
        };
        // An alias can make the payload itself a union. The target may
        // spell its leaves separately, so re-tag that payload through the
        // ordinary subset-union adapter instead of requiring one slot.
        let payload_slot = slot(payload);
        if payload_slot.is_none() && !matches!(payload, HirType::Union(parts)
            if parts.iter().all(|part| slot(part).is_some())) {
            return Ok(None);
        }
        let mut absent_slots = Vec::with_capacity(arms.len());
        for (_, literal) in &arms {
            let absent_ty = match literal {
                HirLit::Null => HirType::Null,
                HirLit::Undefined => HirType::Undefined,
                _ => unreachable!("retag arms only carry Null/Undefined"),
            };
            match slot(&absent_ty) {
                Some(index) => absent_slots.push(index),
                None => return Ok(None),
            }
        }

        let payload_value = match actual {
            HirType::Nullable(payload) => HirExpr::NullableValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            HirType::Optional(payload) => HirExpr::OptionalValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            HirType::Nullish(payload) => HirExpr::NullishValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            _ => unreachable!(),
        };
        let mut statements = Vec::with_capacity(arms.len() + 1);
        for ((test, literal), absent_slot) in arms.into_iter().zip(absent_slots) {
            statements.push(HirStmt::If(
                test,
                vec![HirStmt::Return(Some(HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(literal)),
                    absent_slot,
                    elements.to_vec(),
                )))],
                Vec::new(),
            ));
        }
        let tagged_payload = if let Some(payload_slot) = payload_slot {
            HirExpr::UnionInject(Box::new(payload_value), payload_slot, elements.to_vec())
        } else {
            let parameter = "__thaw_nullish_retag".to_string();
            let previous = self.scope.insert(parameter.clone(), actual.clone());
            let result = self.coerce_to_declared(&HirType::Union(elements.to_vec()), payload_value);
            match previous {
                Some(previous) => { self.scope.insert(parameter, previous); }
                None => { self.scope.remove(&parameter); }
            }
            result?
        };
        statements.push(HirStmt::Return(Some(tagged_payload)));
        let adapter = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam {
                name: "__thaw_nullish_retag".into(),
                ty: actual.clone(),
            }],
            HirType::Union(elements.to_vec()),
            Box::new(HirExpr::Block(statements)),
        );
        Ok(Some(HirExpr::Call(Box::new(adapter), vec![value.clone()])))
    }
}
