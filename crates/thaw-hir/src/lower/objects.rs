impl<'a> FnLowerer<'a> {
    fn lower_object_lit(&mut self, obj_lit: &SwcObjectLit) -> Result<HirExpr, String> {
        struct AwaitFinder(bool);
        impl Visit for AwaitFinder {
            fn visit_await_expr(&mut self, _: &AwaitExpr) {
                self.0 = true;
            }
        }
        let mut finder = AwaitFinder(false);
        obj_lit.visit_with(&mut finder);
        if finder.0 {
            return self.lower_ordered_await_object_lit(obj_lit);
        }
        let mut fields = Vec::new();
        let mut evaluated_spreads: Vec<(Symbol, HirType, HirExpr)> = Vec::new();
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
                    if let HirExpr::ObjectLit(source_values) = &source {
                        source_values.clone()
                    } else if matches!(spread.expr.as_ref(), Expr::Ident(_)) {
                        source_fields
                            .iter()
                            .map(|(name, _)| {
                                (
                                    name.clone(),
                                    HirExpr::PropAccess(
                                        Box::new(source.clone()),
                                        source_type.clone(),
                                        name.clone(),
                                    ),
                                )
                            })
                            .collect::<Vec<_>>()
                    } else {
                        let temporary = format!("__thaw_object_spread_{}", self.next_binding);
                        self.next_binding += 1;
                        let additions = source_fields
                            .iter()
                            .map(|(name, _)| {
                                (
                                    name.clone(),
                                    HirExpr::PropAccess(
                                        Box::new(HirExpr::Var(temporary.clone())),
                                        source_type.clone(),
                                        name.clone(),
                                    ),
                                )
                            })
                            .collect();
                        evaluated_spreads.push((temporary, source_type, source));
                        additions
                    }
                }
                PropOrSpread::Prop(prop) => match prop.as_ref() {
                    Prop::KeyValue(KeyValueProp { key, value }) => {
                        let name = match key {
                            PropName::Ident(ident) => ident.sym.to_string(),
                            PropName::Str(s) => s.value.to_string_lossy().into_owned(),
                            PropName::Computed(computed) => match computed.expr.as_ref() {
                                Expr::Lit(Lit::Str(value)) => {
                                    value.value.to_string_lossy().into_owned()
                                }
                                _ => {
                                    return Err(
                                        "computed object literal keys must be string literals"
                                            .to_string(),
                                    )
                                }
                            },
                            _ => return Err("unsupported object literal key".to_string()),
                        };
                        vec![(name, self.lower_expr(value)?)]
                    }
                    Prop::Shorthand(ident) => vec![(
                        ident.sym.to_string(),
                        self.lower_expr(&Expr::Ident(ident.clone()))?,
                    )],
                    _ => {
                        return Err(
                            "only data properties are supported in object literals".to_string()
                        )
                    }
                },
            };
            for (name, value) in additions {
                if let Some((_, existing)) =
                    fields.iter_mut().find(|(existing, _)| existing == &name)
                {
                    *existing = value;
                } else {
                    fields.push((name, value));
                }
            }
        }
        let mut property_bindings = Vec::new();
        if evaluated_spreads.is_empty() && fields.iter().any(|(_, value)| contains_await(value)) {
            for (position, (_, value)) in fields.iter_mut().enumerate() {
                let source = value.clone();
                let ty = self.infer_expr_type(&source)?;
                let name = format!("__thaw_object_field_{}_{}", position, self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                *value = HirExpr::Var(name.clone());
                property_bindings.push((name, ty, source));
            }
        }
        let mut result = HirExpr::ObjectLit(fields);
        let result_type = self.infer_expr_type(&result)?;
        for index in (0..evaluated_spreads.len()).rev() {
            let (name, ty, source) = &evaluated_spreads[index];
            let captures = evaluated_spreads[..index]
                .iter()
                .map(|(name, ty, _)| HirParam {
                    name: name.clone(),
                    ty: ty.clone(),
                })
                .collect();
            result = HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    vec![HirParam {
                        name: name.clone(),
                        ty: ty.clone(),
                    }],
                    result_type.clone(),
                    Box::new(result),
                )),
                vec![source.clone()],
            );
        }
        self.wrap_call_argument_bindings(result, &property_bindings)
    }

    fn lower_ordered_await_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
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
                    source_fields
                        .iter()
                        .map(|(field, _)| {
                            (
                                field.clone(),
                                HirExpr::PropAccess(
                                    Box::new(HirExpr::Var(name.clone())),
                                    source_type.clone(),
                                    field.clone(),
                                ),
                            )
                        })
                        .collect::<Vec<_>>()
                }
                PropOrSpread::Prop(prop) => {
                    let (field, source) = match prop.as_ref() {
                        Prop::KeyValue(KeyValueProp { key, value }) => {
                            let field = match key {
                                PropName::Ident(ident) => ident.sym.to_string(),
                                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => {
                                    match computed.expr.as_ref() {
                                        Expr::Lit(Lit::Str(value)) => {
                                            value.value.to_string_lossy().into_owned()
                                        }
                                        _ => return Err(
                                            "computed object literal keys must be string literals"
                                                .into(),
                                        ),
                                    }
                                }
                                _ => return Err("unsupported object literal key".into()),
                            };
                            (field, self.lower_expr(value)?)
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
                    let name = format!("__thaw_object_value_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, source));
                    vec![(field, HirExpr::Var(name))]
                }
            };
            for (name, value) in additions {
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
            HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
            HirType::Nullable(payload) => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
            HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
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

    fn lower_unbound_this_member(&self, property: &str) -> Result<HirExpr, String> {
        let ty = self.unbound_this_member_type(property).ok_or_else(|| {
            format!(
                "class `{}` has no native member `{property}`",
                self.class_context.as_deref().unwrap_or("<unknown>")
            )
        })?;
        self.lower_unbound_this_error(property, &ty)
    }

    fn typed_dictionary_read(value: HirExpr, element: &HirType) -> Result<HirExpr, String> {
        match element {
            HirType::F64 => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::Str => Ok(HirExpr::JsonAsString(Box::new(value))),
            HirType::Bool => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::Json => Ok(value),
            HirType::Object(_) | HirType::Array(_) => {
                Ok(HirExpr::JsonAsNative(Box::new(value), element.clone()))
            }
            other => Err(format!(
                "dictionary reads do not yet support value type {other:?}"
            )),
        }
    }

    fn lower_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        if self.unbound_this_context && matches!(member.obj.as_ref(), Expr::This(_)) {
            let property = member_property_name(&member.prop)
                .ok_or("unbound `this` member access requires a statically known property")?;
            return self.lower_unbound_this_member(&property);
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
                    if let Some(class_name) = class_name_from_type(&receiver_type) {
                        let symbol = class_getter_symbol(class_name, &property, false);
                        if self.signatures.contains_key(&symbol) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![HirExpr::Var(binding)],
                            ));
                        }
                    }
                }
            }
            if matches!(member.obj.as_ref(), Expr::This(_)) {
                let binding = self.resolve_binding("this");
                if let Some(receiver_type) = self.scope.get(&binding).cloned() {
                    if let Some(class_name) = class_name_from_type(&receiver_type) {
                        let symbol = class_getter_symbol(class_name, &property, false);
                        if self.signatures.contains_key(&symbol) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![HirExpr::Var(binding)],
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

        match &member.prop {
            MemberProp::Computed(computed) => {
                let obj = self.lower_expr(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                match obj_ty {
                    HirType::Array(element) => {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
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
                    HirType::Object(fields) => {
                        if let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() {
                            let key = key.value.to_string_lossy().into_owned();
                            if fields.iter().any(|(name, _)| name == &key) {
                                return Ok(HirExpr::PropAccess(
                                    Box::new(obj),
                                    HirType::Object(fields),
                                    key,
                                ));
                            }
                            return Err(format!("object has no field `{key}`"));
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
                            let key = self.lower_expr(&computed.expr)?;
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
                        let key = self.lower_expr(&computed.expr)?;
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
                        Self::typed_dictionary_read(
                            HirExpr::JsonKey(Box::new(obj), Box::new(key)),
                            element.as_ref(),
                        )
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
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            MemberProp::Ident(prop) => {
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
                let obj = self.lower_expr(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                match &obj_ty {
                    HirType::Array(_) | HirType::Tuple(_) if prop.sym == *"length" => {
                        Ok(HirExpr::ArrayLen(Box::new(obj)))
                    }
                    HirType::Str if prop.sym == *"length" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".to_string())),
                        vec![obj],
                    )),
                    HirType::Object(fields) => {
                        if fields.iter().any(|(name, _)| name == prop.sym.as_str()) {
                            Ok(HirExpr::PropAccess(
                                Box::new(obj),
                                obj_ty.clone(),
                                prop.sym.to_string(),
                            ))
                        } else {
                            Err(format!("object has no field `{}`", prop.sym))
                        }
                    }
                    HirType::Union(elements) => {
                        self.lower_union_property_read(obj, elements, prop.sym.as_ref())
                    }
                    HirType::Json => Ok(HirExpr::JsonGet(Box::new(obj), prop.sym.to_string())),
                    HirType::Dictionary(element) => Self::typed_dictionary_read(
                        HirExpr::JsonGet(Box::new(obj), prop.sym.to_string()),
                        element.as_ref(),
                    ),
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
                    HirExpr::PropAccess(Box::new(unwrapped), payload.as_ref().clone(), field),
                    field_type,
                )
            }
            HirType::Array(element) => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => {
                    (HirExpr::ArrayLen(Box::new(unwrapped)), HirType::F64)
                }
                MemberProp::Computed(computed) => {
                    let index = self.lower_expr(&computed.expr)?;
                    self.expect_type(&HirType::F64, &index, "optional array index")?;
                    (
                        HirExpr::TypedIndex(Box::new(unwrapped), Box::new(index), *element.clone()),
                        *element.clone(),
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
                    Self::typed_dictionary_read(
                        HirExpr::JsonKey(Box::new(unwrapped), Box::new(key)),
                        element.as_ref(),
                    )?,
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
        let (result_payload, present_value) = match &field_type {
            HirType::Optional(inner) => (inner.as_ref().clone(), None),
            other => (other.clone(), Some(other.clone())),
        };
        let present_value = match present_value {
            Some(payload) => HirExpr::OptionalSome(Box::new(access), payload),
            None => access,
        };
        let is_none = match absence_kind {
            0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
            1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
            2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            is_none,
            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                result_payload.clone(),
            )))],
            vec![HirStmt::Return(Some(present_value))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, object_type, object)])
    }

}
