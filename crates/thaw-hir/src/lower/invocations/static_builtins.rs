impl<'a> FnLowerer<'a> {
    fn is_static_builtin_call(object: &str, property: &str) -> bool {
        matches!(
            (object, property),
            ("Array", "of" | "from" | "isArray")
                | ("Object", "keys" | "getOwnPropertyNames" | "values" | "entries" | "fromEntries" | "assign" | "hasOwn" | "is")
                | ("JSON", "stringify")
                | ("Reflect", "ownKeys")
                | ("Number", "parseFloat" | "parseInt" | "isNaN" | "isFinite" | "isInteger" | "isSafeInteger")
                | ("Math", "random" | "abs" | "floor" | "ceil" | "trunc" | "sqrt" | "exp" | "log" | "log2" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "cbrt" | "acosh" | "asinh" | "atanh" | "expm1" | "log1p" | "fround" | "clz32" | "pow" | "min" | "max" | "sign" | "round" | "atan2" | "hypot" | "imul")
        )
    }

    fn lower_static_builtin_call(
        &mut self,
        object: &swc_ecma_ast::Ident,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                    if object.sym == *"JSON" && property.sym == *"stringify" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "JSON.stringify")?;
                        if !(1..=3).contains(&arguments.len()) {
                            return Err("`JSON.stringify` expects one to three arguments".into());
                        }
                        let value = arguments[0].clone();
                        let value_type = self.infer_expr_type(&value)?;
                        if !matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "`JSON.stringify` requires a JSON or dictionary value, got {value_type:?}"
                            ));
                        }
                        if let Some(replacer) = arguments.get(1) {
                            let replacer_type = self.infer_expr_type(replacer)?;
                            if !matches!(replacer_type, HirType::Null | HirType::Undefined) {
                                return Err(
                                    "`JSON.stringify` replacer functions and arrays are not supported"
                                        .into(),
                                );
                            }
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
                        let result = match arguments.get(2) {
                            None | Some(HirExpr::Lit(HirLit::Null | HirLit::Undefined)) => {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("JSON.stringify".into())),
                                    vec![value],
                                )
                            }
                            Some(space) => {
                                let runtime = match self.infer_expr_type(space)? {
                                    HirType::F64 => "__thaw_json_stringify_number_space",
                                    HirType::Str => "__thaw_json_stringify_string_space",
                                    other => {
                                        return Err(format!(
                                            "`JSON.stringify` space must be number, string, null or undefined, got {other:?}"
                                        ))
                                    }
                                };
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(runtime.into())),
                                    vec![value, space.clone()],
                                )
                            }
                        };
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
                            let value = self.lower_expr(&argument.expr)?;
                            if argument.spread.is_some() {
                                if !pending.is_empty() {
                                    parts.push(HirExpr::ArrayLit(std::mem::take(&mut pending)));
                                }
                                let ty = self.infer_expr_type(&value)?;
                                let HirType::Array(element) = ty else {
                                    return Err(
                                        "`Array.of` spread requires a homogeneous array".into()
                                    );
                                };
                                if let Some(expected) = &element_type {
                                    if expected != element.as_ref() {
                                        return Err(format!(
                                            "`Array.of` spread element has type {element:?}, expected {expected:?}"
                                        ));
                                    }
                                } else {
                                    element_type = Some(element.as_ref().clone());
                                }
                                parts.push(value);
                            } else {
                                let ty = self.infer_expr_type(&value)?;
                                if let Some(expected) = &element_type {
                                    if expected != &ty {
                                        return Err(format!(
                                            "`Array.of` element has type {ty:?}, expected {expected:?}"
                                        ));
                                    }
                                } else {
                                    element_type = Some(ty);
                                }
                                pending.push(value);
                            }
                        }
                        if !pending.is_empty() {
                            parts.push(HirExpr::ArrayLit(pending));
                        }
                        let element_type = element_type.ok_or(
                            "empty `Array.of()` requires an explicit element type argument",
                        )?;
                        return Ok(match parts.len() {
                            0 => HirExpr::ArrayAlloc(
                                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                element_type,
                            ),
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
                        let explicit_types = call
                            .type_args
                            .as_ref()
                            .map(|type_args| {
                                type_args
                                    .params
                                    .iter()
                                    .map(|ty| {
                                        lower_ts_type(ty, self.interfaces, self.generic_interfaces)
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        if explicit_types.len() > 2 {
                            return Err("`Array.from` expects at most two type arguments".into());
                        }
                        let source = if has_spread {
                            spread_arguments[0].clone()
                        } else {
                            self.lower_expr(&call.args[0].expr)?
                        };
                        let source_type = self.infer_expr_type(&source)?;
                        let (source, source_type, element_type) = match source_type {
                            HirType::Array(element) => {
                                let element_type = element.as_ref().clone();
                                (source, HirType::Array(element), element_type)
                            }
                            HirType::Str => (
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_to_array".into())),
                                    vec![source],
                                ),
                                HirType::Array(Box::new(HirType::Str)),
                                HirType::Str,
                            ),
                            other => {
                                return Err(format!(
                                    "native `Array.from` requires a homogeneous array or string, got {other:?}"
                                ))
                            }
                        };
                        if let Some(expected) = explicit_types.first() {
                            if expected != &element_type {
                                return Err(format!(
                                    "`Array.from` source element has type {element_type:?}, expected {expected:?}"
                                ));
                            }
                        }
                        if argument_count == 1 {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                vec![
                                    source,
                                    HirExpr::Lit(HirLit::F64(0.0)),
                                    HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                                ],
                            );
                            return self.wrap_call_argument_bindings(result, &spread_bindings);
                        }
                        let callback = if has_spread {
                            let callback = spread_arguments[1].clone();
                            let params = match self.infer_expr_type(&callback)? {
                                HirType::Function(params, _)
                                | HirType::CallableFunction(params, _, _, _) => params,
                                _ => return Err("Array.from mapper is not a function value".into()),
                            };
                            if params.len() > 2 {
                                return Err(format!(
                                    "Array.from mapper accepts at most two parameters, got {}",
                                    params.len()
                                ));
                            }
                            let available = [element_type.clone(), HirType::F64];
                            self.validate_promise_callback_value(
                                &callback,
                                &available[..params.len()],
                                None,
                            )?;
                            callback
                        } else {
                            self.lower_array_from_callback(&call.args[1].expr, &element_type)?
                        };
                        if let Some(expected) = explicit_types.get(1).or(explicit_types.first()) {
                            let HirType::Function(_, output) = self.infer_expr_type(&callback)?
                            else {
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
                            element_type,
                            callback,
                            this_arg,
                        )?;
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                    }
                    if object.sym == *"Array" && property.sym == *"isArray" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Array.isArray")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Array.isArray` expects exactly one argument".into());
                        };
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
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
                            HirExpr::Lit(HirLit::Bool(matches!(
                                ty,
                                HirType::Array(_) | HirType::Tuple(_)
                            ))),
                            &bindings,
                        );
                    }
                    if (object.sym == *"Object"
                        && matches!(property.sym.as_ref(), "keys" | "getOwnPropertyNames"))
                        || (object.sym == *"Reflect" && property.sym == *"ownKeys")
                    {
                        let label = format!("{}.{}", object.sym, property.sym);
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
                        if matches!(ty, HirType::Json | HirType::Dictionary(_)) {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_keys".to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        if matches!(ty, HirType::Array(_) | HirType::Tuple(_)) {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_keys".to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`{label}` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let keys = HirExpr::ArrayLit(
                            fields
                                .iter()
                                .map(|(name, _)| HirExpr::Lit(HirLit::Str(name.clone())))
                                .collect(),
                        );
                        let name = format!("__thaw_object_keys_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(keys, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"values" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Object.values")?;
                        let [value] = arguments.as_slice() else {
                            return Err("`Object.values` expects exactly one argument".into());
                        };
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
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
                                    ))
                                }
                            };
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(runtime.to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.values` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let field_names = fields
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_object_values_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let values = HirExpr::ArrayLit(
                            field_names
                                .into_iter()
                                .map(|field| {
                                    HirExpr::PropAccess(
                                        Box::new(HirExpr::Var(name.clone())),
                                        ty.clone(),
                                        field,
                                    )
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
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
                        if ty == HirType::Json {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_entries".to_string())),
                                vec![value],
                            );
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
                                    ))
                                }
                            };
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(runtime.to_string())),
                                vec![value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.entries` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let entry_fields = fields
                            .iter()
                            .map(|(name, field_type)| (name.clone(), field_type.clone()))
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
                                                HirExpr::PropAccess(
                                                    Box::new(HirExpr::Var(name.clone())),
                                                    ty.clone(),
                                                    field,
                                                ),
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
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.fromEntries")?;
                        let [entries] = arguments.as_slice() else {
                            return Err("`Object.fromEntries` expects exactly one argument".into());
                        };
                        let entries = entries.clone();
                        let ty = self.infer_expr_type(&entries)?;
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
                                ))
                            }
                        };
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(runtime.to_string())),
                            vec![entries],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"assign" {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.assign")?;
                        let Some((target, sources)) = arguments.split_first() else {
                            return Err("`Object.assign` expects at least one argument".into());
                        };
                        let target_type = self.infer_expr_type(target)?;
                        if !matches!(target_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "`Object.assign` requires a JSON or dictionary target, got {target_type:?}"
                            ));
                        }
                        let mut result = target.clone();
                        for source in sources {
                            let source_type = self.infer_expr_type(source)?;
                            if source_type != target_type {
                                return Err(format!(
                                    "`Object.assign` source type {source_type:?} does not match target {target_type:?}"
                                ));
                            }
                            result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                                vec![result, source.clone()],
                            );
                        }
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"hasOwn" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Object.hasOwn")?;
                        let [object_value, key_value] = arguments.as_slice() else {
                            return Err("`Object.hasOwn` expects exactly two arguments".into());
                        };
                        let object_value = object_value.clone();
                        let object_type = self.infer_expr_type(&object_value)?;
                        let key_value = self.coerce_primitive_to_string(key_value.clone())?;
                        if matches!(object_type, HirType::Json | HirType::Dictionary(_)) {
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_has_own".to_string())),
                                vec![object_value, key_value],
                            );
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let HirType::Object(fields) = &object_type else {
                            return Err(format!(
                                "`Object.hasOwn` currently requires a fixed object, got {object_type:?}"
                            ));
                        };
                        let field_names = fields
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>();
                        let object_name = format!("__thaw_has_own_object_{}", self.next_binding);
                        self.next_binding += 1;
                        let key_name = format!("__thaw_has_own_key_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(object_name.clone(), object_type.clone());
                        self.scope.insert(key_name.clone(), HirType::Str);
                        let mut comparisons = field_names.into_iter().map(|field| {
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(key_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::Str(field))),
                            )
                        });
                        let mut result = comparisons
                            .next()
                            .unwrap_or(HirExpr::Lit(HirLit::Bool(false)));
                        for comparison in comparisons {
                            result = self.lower_logical_expr(result, comparison, false)?;
                        }
                        bindings.push((object_name, object_type, object_value));
                        bindings.push((key_name, HirType::Str, key_value));
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
                            if runtime.is_empty() {
                                HirExpr::Lit(HirLit::Bool(false))
                            } else {
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(runtime.into())),
                                    vec![json, native],
                                )
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
                    if object.sym == *"Number"
                        && matches!(property.sym.as_ref(), "parseFloat" | "parseInt")
                    {
                        return self.lower_parse_call(call, property.sym == *"parseInt");
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
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
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
                    if object.sym == *"Math"
                        && matches!(
                            property.sym.as_ref(),
                            "pow" | "min" | "max" | "sign" | "round" | "atan2" | "hypot" | "imul"
                        )
                    {
                        let label = format!("Math.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
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
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!(
                                "`Number.{}` expects exactly one argument",
                                property.sym
                            ));
                        };
                        let value = value.clone();
                        let ty = self.infer_expr_type(&value)?;
                        if ty == HirType::F64 {
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
                        let name = format!("__thaw_number_predicate_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(false)),
                            &bindings,
                        );
                    }
        unreachable!("static builtin dispatch was checked before lowering")
    }
}
