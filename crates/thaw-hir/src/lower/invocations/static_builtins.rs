/// Types the native-to-`Json` conversion in `compile_json_object_set_native`
/// (thaw-llvm's `json_bridge.rs`, reached below through `HirExpr::JsonSet`)
/// already knows how to encode as an object field, and so can also encode
/// as a `JSON.stringify` argument via `wrap_native_value_as_json`. Anything
/// else (`Function`, `Promise`, `Map`/`Set`, `Union`, ...) has no such
/// encoding and is rejected exactly as before.
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
    )
}

impl<'a> FnLowerer<'a> {
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
        let intrinsic = if is_min { "__thaw_math_min" } else { "__thaw_math_max" };
        let seed = if is_min { f64::INFINITY } else { f64::NEG_INFINITY };
        let body = HirExpr::Block(vec![
            HirStmt::Let(acc_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(seed))),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
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
                                HirExpr::TypedIndex(
                                    Box::new(var(&source_name)),
                                    Box::new(var(&index_name)),
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
            Box::new(HirExpr::Lambda(captures, Vec::new(), HirType::F64, Box::new(body))),
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
                | ("Buffer", "from" | "alloc" | "concat" | "byteLength" | "isBuffer")
                | ("BigInt", "asIntN" | "asUintN")
                | ("Error", "isError")
                | ("Proxy", "revocable")
                | ("Intl", "getCanonicalLocales" | "supportedValuesOf")
                | ("Map", "groupBy")
                | ("Object", "groupBy" | "keys" | "getOwnPropertyNames" | "values" | "entries" | "fromEntries" | "assign" | "hasOwn" | "is" | "freeze" | "seal" | "preventExtensions" | "isFrozen" | "isSealed" | "isExtensible" | "getOwnPropertyDescriptor" | "getOwnPropertyDescriptors" | "defineProperty" | "defineProperties" | "create" | "getPrototypeOf" | "setPrototypeOf" | "getOwnPropertySymbols")
                | ("JSON", "stringify" | "parse")
                | ("Iterator", "from" | "concat" | "zip" | "zipKeyed")
                | ("RegExp", "escape")
                | ("Reflect", "ownKeys" | "has" | "get" | "set" | "deleteProperty" | "apply" | "construct" | "defineProperty" | "getOwnPropertyDescriptor" | "getPrototypeOf" | "setPrototypeOf" | "isExtensible" | "preventExtensions")
                | ("Symbol", "for" | "keyFor")
                | ("Number", "parseFloat" | "parseInt" | "isNaN" | "isFinite" | "isInteger" | "isSafeInteger")
                | ("String", "fromCharCode" | "fromCodePoint" | "raw")
                | ("Math", "sumPrecise" | "random" | "abs" | "floor" | "ceil" | "trunc" | "sqrt" | "exp" | "log" | "log2" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "cbrt" | "acosh" | "asinh" | "atanh" | "expm1" | "log1p" | "f16round" | "fround" | "clz32" | "pow" | "min" | "max" | "sign" | "round" | "atan2" | "hypot" | "imul")
                | ("Date", "now" | "UTC" | "parse")
                | ("performance", "now")
        )
    }

    /// Records freeze/seal/preventExtensions state for the binding a value
    /// refers to, when it's a simple variable. See `object_states`.
    fn mark_object_state(
        &mut self,
        value: &HirExpr,
        frozen: bool,
        sealed: bool,
        nonextensible: bool,
    ) {
        if let HirExpr::Var(name) = value {
            let state = self.object_states.entry(name.clone()).or_insert(ObjectState {
                frozen: false,
                sealed: false,
                nonextensible: false,
            });
            state.frozen |= frozen;
            state.sealed |= sealed;
            state.nonextensible |= nonextensible;
        }
    }

    /// The state an inline `Object.freeze(...)`/`seal(...)`/
    /// `preventExtensions(...)` argument implies, for a query applied
    /// directly to such a call (`Object.isFrozen(Object.freeze(x))`).
    fn state_setting_call(expr: &Expr) -> Option<ObjectState> {
        let expr = match expr {
            Expr::Paren(paren) => paren.expr.as_ref(),
            other => other,
        };
        let Expr::Call(call) = expr else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let Expr::Ident(object) = member.obj.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        match (object.sym.as_ref(), property.sym.as_ref()) {
            ("Object", "freeze") => Some(ObjectState {
                frozen: true,
                sealed: true,
                nonextensible: true,
            }),
            ("Object", "seal") => Some(ObjectState {
                frozen: false,
                sealed: true,
                nonextensible: true,
            }),
            ("Object", "preventExtensions") => Some(ObjectState {
                frozen: false,
                sealed: false,
                nonextensible: true,
            }),
            _ => None,
        }
    }

    /// The tracked freeze/seal/extensibility state of a value (a fresh,
    /// extensible one when it isn't a tracked binding).
    fn object_state(&self, value: &HirExpr) -> ObjectState {
        if let HirExpr::Var(name) = value {
            if let Some(state) = self.object_states.get(name) {
                return *state;
            }
        }
        ObjectState {
            frozen: false,
            sealed: false,
            nonextensible: false,
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
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
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
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![value],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if object.sym == *"Object"
                        && matches!(
                            property.sym.as_ref(),
                            "freeze" | "seal" | "preventExtensions"
                        )
                    {
                        let label = format!("Object.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        let value = value.clone();
                        // thaw's native objects are fixed-layout values, so
                        // there is no runtime mutability to change; record
                        // the observable state instead (see `object_states`).
                        // The call returns its argument, exactly as JS's do.
                        match property.sym.as_ref() {
                            "freeze" => self.mark_object_state(&value, true, true, true),
                            "seal" => self.mark_object_state(&value, false, true, true),
                            _ => self.mark_object_state(&value, false, false, true),
                        }
                        return self.wrap_call_argument_bindings(value, &bindings);
                    }
                    if object.sym == *"Object"
                        && matches!(
                            property.sym.as_ref(),
                            "isFrozen" | "isSealed" | "isExtensible"
                        )
                    {
                        let label = format!("Object.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        // Reflect the tracked freeze/seal/extensibility state
                        // a prior `Object.freeze`/`seal`/`preventExtensions`
                        // (or `Reflect` equivalent) recorded for this binding,
                        // or that an inline `Object.freeze(...)` argument
                        // itself implies; an untracked object is a fresh,
                        // extensible one.
                        let state = call
                            .args
                            .first()
                            .and_then(|argument| Self::state_setting_call(&argument.expr))
                            .unwrap_or_else(|| self.object_state(value));
                        let result = match property.sym.as_ref() {
                            "isFrozen" => state.frozen,
                            "isSealed" => state.sealed,
                            _ => !state.nonextensible,
                        };
                        return self
                            .wrap_call_argument_bindings(HirExpr::Lit(HirLit::Bool(result)), &bindings);
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"getOwnPropertyDescriptor"
                    {
                        let (arguments, bindings) = self.lower_native_spread_values(
                            &call.args,
                            "Object.getOwnPropertyDescriptor",
                        )?;
                        let [target, key] = arguments.as_slice() else {
                            return Err(
                                "`Object.getOwnPropertyDescriptor` expects exactly two arguments"
                                    .into(),
                            );
                        };
                        let HirExpr::Lit(HirLit::Str(key)) = key else {
                            return Err(
                                "`Object.getOwnPropertyDescriptor` currently requires a string-literal key"
                                    .into(),
                            );
                        };
                        let key = key.clone();
                        let target_type = self.infer_expr_type(target)?;
                        let HirType::Object(fields) = &target_type else {
                            return Err(format!(
                                "`Object.getOwnPropertyDescriptor` currently requires a fixed \
                                 object, got {target_type:?}"
                            ));
                        };
                        if !fields.iter().any(|(name, _)| name == &key) {
                            return self.wrap_call_argument_bindings(
                                HirExpr::Lit(HirLit::Undefined),
                                &bindings,
                            );
                        }
                        // All fields thaw models are own, writable,
                        // enumerable, and configurable.
                        let descriptor = HirExpr::ObjectLit(vec![
                            (
                                "value".to_string(),
                                HirExpr::PropAccess(Box::new(target.clone()), target_type, key),
                            ),
                            ("writable".to_string(), HirExpr::Lit(HirLit::Bool(true))),
                            ("enumerable".to_string(), HirExpr::Lit(HirLit::Bool(true))),
                            ("configurable".to_string(), HirExpr::Lit(HirLit::Bool(true))),
                        ]);
                        return self.wrap_call_argument_bindings(descriptor, &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"create" {
                        // Approx: a fresh object with no modeled prototype,
                        // returned as an empty dictionary so dynamic
                        // `obj[key]` reads/writes and `Object.keys` work.
                        // `Object.create(null)` (or with a prototype) is
                        // treated the same -- thaw has no prototype chain.
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.create")?;
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err("`Object.create` expects one or two arguments".into());
                        }
                        let result = HirExpr::JsonObjectLit(Vec::new(), HirType::Json);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"getPrototypeOf"
                    {
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, "Object.getPrototypeOf")?;
                        let [_value] = arguments.as_slice() else {
                            return Err(
                                "`Object.getPrototypeOf` expects exactly one argument".into()
                            );
                        };
                        // Approx: thaw models no prototype chain, so report
                        // `null` (correct for an `Object.create(null)` map).
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Null),
                            &bindings,
                        );
                    }
                    if object.sym == *"Object" && property.sym == *"setPrototypeOf" {
                        let (arguments, _bindings) = self
                            .lower_native_spread_values(&call.args, "Object.setPrototypeOf")?;
                        let [_target, _prototype] = arguments.as_slice() else {
                            return Err(
                                "`Object.setPrototypeOf` expects exactly two arguments".into()
                            );
                        };
                        return Err(
                            "`Object.setPrototypeOf` is not supported for fixed-layout native objects"
                                .into(),
                        );
                    }
                    if object.sym == *"Object" && property.sym == *"getOwnPropertySymbols" {
                        let (arguments, bindings) = self.lower_native_spread_values(
                            &call.args,
                            "Object.getOwnPropertySymbols",
                        )?;
                        let [target] = arguments.as_slice() else {
                            return Err(
                                "`Object.getOwnPropertySymbols` expects exactly one argument".into()
                            );
                        };
                        let symbols = match self.infer_expr_type(target)? {
                            HirType::Object(fields) => fields
                                .into_iter()
                                .filter_map(|(name, _)| {
                                    name.strip_prefix("\u{1f}@@").map(|symbol| {
                                        HirExpr::Lit(HirLit::Str(format!("\u{1f}@@{symbol}")))
                                    })
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayLit(symbols),
                            &bindings,
                        );
                    }
                    if (object.sym == *"Object" || object.sym == *"Reflect")
                        && property.sym == *"defineProperty"
                    {
                        // A `value`-only descriptor, and only for a field
                        // the receiver already has: thaw's objects are
                        // fixed-layout, so it can reassign an existing
                        // field but cannot add a new one, and it has no
                        // accessor (get/set) properties.
                        let [target, key, descriptor] = call.args.as_slice() else {
                            return Err(
                                "`Object.defineProperty` expects exactly three arguments".into()
                            );
                        };
                        let Expr::Lit(Lit::Str(key)) = key.expr.as_ref() else {
                            return Err(
                                "`Object.defineProperty` currently requires a string-literal key"
                                    .into(),
                            );
                        };
                        let key = key.value.to_string_lossy().into_owned();
                        let Expr::Object(descriptor) = descriptor.expr.as_ref() else {
                            return Err(
                                "`Object.defineProperty` requires an object-literal descriptor"
                                    .into(),
                            );
                        };
                        let mut descriptor_value = None;
                        for property in &descriptor.props {
                            let swc_ecma_ast::PropOrSpread::Prop(property) = property else {
                                return Err(
                                    "`Object.defineProperty` descriptor must not spread".into()
                                );
                            };
                            let swc_ecma_ast::Prop::KeyValue(property) = property.as_ref() else {
                                return Err(
                                    "`Object.defineProperty` descriptor must use `key: value` entries"
                                        .into(),
                                );
                            };
                            let name = match &property.key {
                                swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                                swc_ecma_ast::PropName::Str(value) => {
                                    value.value.to_string_lossy().into_owned()
                                }
                                _ => continue,
                            };
                            match name.as_str() {
                                "value" => descriptor_value = Some(property.value.as_ref()),
                                "writable" | "enumerable" | "configurable" => {}
                                "get" | "set" => {
                                    return Err(
                                        "`Object.defineProperty` accessors (get/set) are not supported"
                                            .into(),
                                    )
                                }
                                _ => {}
                            }
                        }
                        let Some(descriptor_value) = descriptor_value else {
                            return Err(
                                "`Object.defineProperty` requires a `value` in its descriptor"
                                    .into(),
                            );
                        };
                        let value = self.lower_expr(descriptor_value)?;
                        let target_value = self.lower_expr(target.expr.as_ref())?;
                        let target_type = self.infer_expr_type(&target_value)?;
                        let assign = match &target_type {
                            HirType::Object(fields) => {
                                let Some((_, field_type)) =
                                    fields.iter().find(|(name, _)| name == &key)
                                else {
                                    return Err(format!(
                                        "`Object.defineProperty` cannot add the new field `{key}` to a fixed object"
                                    ));
                                };
                                let value = self.coerce_to_declared(field_type, value)?;
                                HirExpr::PropAssign(
                                    Box::new(target_value.clone()),
                                    target_type.clone(),
                                    key,
                                    Box::new(value),
                                )
                            }
                            HirType::Json => {
                                let value_type = self.infer_expr_type(&value)?;
                                HirExpr::JsonSet(
                                    Box::new(target_value.clone()),
                                    Box::new(HirExpr::Lit(HirLit::Str(key))),
                                    Box::new(value),
                                    value_type,
                                    true,
                                )
                            }
                            other => {
                                return Err(format!(
                                    "`Object.defineProperty` currently requires a fixed object or \
                                     JSON receiver, got {other:?}"
                                ))
                            }
                        };
                        // `Object.defineProperty` returns the object;
                        // `Reflect.defineProperty` returns a boolean. Run
                        // the assignment for its side effect and hand back
                        // the appropriate result.
                        let (return_value, return_type) = if object.sym == *"Reflect" {
                            (HirExpr::Lit(HirLit::Bool(true)), HirType::Bool)
                        } else {
                            (target_value, target_type)
                        };
                        let body = HirExpr::Block(vec![
                            HirStmt::Expr(assign),
                            HirStmt::Return(Some(return_value)),
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
                                return_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        return Ok(result);
                    }
                    if object.sym == *"Object" && property.sym == *"getOwnPropertyDescriptors" {
                        let (arguments, bindings) = self.lower_native_spread_values(
                            &call.args,
                            "Object.getOwnPropertyDescriptors",
                        )?;
                        let [target] = arguments.as_slice() else {
                            return Err(
                                "`Object.getOwnPropertyDescriptors` expects exactly one argument"
                                    .into(),
                            );
                        };
                        let target_type = self.infer_expr_type(target)?;
                        let HirType::Object(fields) = &target_type else {
                            return Err(format!(
                                "`Object.getOwnPropertyDescriptors` currently requires a fixed \
                                 object, got {target_type:?}"
                            ));
                        };
                        let descriptors = fields
                            .iter()
                            .map(|(name, _)| {
                                let descriptor = HirExpr::ObjectLit(vec![
                                    (
                                        "value".to_string(),
                                        HirExpr::PropAccess(
                                            Box::new(target.clone()),
                                            target_type.clone(),
                                            name.clone(),
                                        ),
                                    ),
                                    (
                                        "writable".to_string(),
                                        HirExpr::Lit(HirLit::Bool(true)),
                                    ),
                                    (
                                        "enumerable".to_string(),
                                        HirExpr::Lit(HirLit::Bool(true)),
                                    ),
                                    (
                                        "configurable".to_string(),
                                        HirExpr::Lit(HirLit::Bool(true)),
                                    ),
                                ]);
                                (name.clone(), descriptor)
                            })
                            .collect::<Vec<_>>();
                        return self
                            .wrap_call_argument_bindings(HirExpr::ObjectLit(descriptors), &bindings);
                    }
                    if object.sym == *"Object" && property.sym == *"defineProperties" {
                        // Desugars to a sequence of `value`-only
                        // `Object.defineProperty` assignments over the
                        // descriptor object's literal keys (same limits:
                        // existing field on a fixed object, or a JSON key).
                        let [target, descriptors] = call.args.as_slice() else {
                            return Err(
                                "`Object.defineProperties` expects exactly two arguments".into()
                            );
                        };
                        let Expr::Object(descriptors) = descriptors.expr.as_ref() else {
                            return Err(
                                "`Object.defineProperties` requires an object-literal descriptor"
                                    .into(),
                            );
                        };
                        let target_value = self.lower_expr(target.expr.as_ref())?;
                        let target_type = self.infer_expr_type(&target_value)?;
                        let mut body = Vec::new();
                        for property in &descriptors.props {
                            let swc_ecma_ast::PropOrSpread::Prop(property) = property else {
                                return Err(
                                    "`Object.defineProperties` descriptors must not spread".into()
                                );
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
                                _ => {
                                    return Err(
                                        "`Object.defineProperties` requires literal keys".into()
                                    )
                                }
                            };
                            let Expr::Object(descriptor) = property.value.as_ref() else {
                                return Err(format!(
                                    "`Object.defineProperties` descriptor for `{key}` must be an object literal"
                                ));
                            };
                            let mut descriptor_value = None;
                            for entry in &descriptor.props {
                                let swc_ecma_ast::PropOrSpread::Prop(entry) = entry else {
                                    return Err(
                                        "`Object.defineProperties` descriptor must not spread"
                                            .into(),
                                    );
                                };
                                let swc_ecma_ast::Prop::KeyValue(entry) = entry.as_ref() else {
                                    return Err(
                                        "`Object.defineProperties` descriptor must use `key: value` entries"
                                            .into(),
                                    );
                                };
                                let name = match &entry.key {
                                    swc_ecma_ast::PropName::Ident(ident) => ident.sym.to_string(),
                                    swc_ecma_ast::PropName::Str(value) => {
                                        value.value.to_string_lossy().into_owned()
                                    }
                                    _ => continue,
                                };
                                if name == "value" {
                                    descriptor_value = Some(entry.value.as_ref());
                                } else if name == "get" || name == "set" {
                                    return Err(
                                        "`Object.defineProperties` accessors (get/set) are not supported"
                                            .into(),
                                    );
                                }
                            }
                            let Some(descriptor_value) = descriptor_value else {
                                return Err(format!(
                                    "`Object.defineProperties` requires a `value` for `{key}`"
                                ));
                            };
                            let value = self.lower_expr(descriptor_value)?;
                            let assign = match &target_type {
                                HirType::Object(fields) => {
                                    let Some((_, field_type)) =
                                        fields.iter().find(|(name, _)| name == &key)
                                    else {
                                        return Err(format!(
                                            "`Object.defineProperties` cannot add the new field `{key}` to a fixed object"
                                        ));
                                    };
                                    let value = self.coerce_to_declared(field_type, value)?;
                                    HirExpr::PropAssign(
                                        Box::new(target_value.clone()),
                                        target_type.clone(),
                                        key,
                                        Box::new(value),
                                    )
                                }
                                HirType::Json => {
                                    let value_type = self.infer_expr_type(&value)?;
                                    HirExpr::JsonSet(
                                        Box::new(target_value.clone()),
                                        Box::new(HirExpr::Lit(HirLit::Str(key))),
                                        Box::new(value),
                                        value_type,
                                        true,
                                    )
                                }
                                other => {
                                    return Err(format!(
                                        "`Object.defineProperties` currently requires a fixed object or \
                                         JSON receiver, got {other:?}"
                                    ))
                                }
                            };
                            body.push(HirStmt::Expr(assign));
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
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                target_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        ));
                    }
                    if object.sym == *"Reflect"
                        && matches!(
                            property.sym.as_ref(),
                            "setPrototypeOf" | "isExtensible" | "preventExtensions"
                        )
                    {
                        let label = format!("Reflect.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let expected = if property.sym == *"setPrototypeOf" { 2 } else { 1 };
                        if arguments.len() != expected {
                            return Err(format!("`{label}` expects {expected} argument(s)"));
                        }
                        // thaw models no prototype chain, but it does track
                        // extensibility: `preventExtensions` records it, and
                        // `isExtensible`/`setPrototypeOf` report/respect it
                        // (both return `false` for a non-extensible target,
                        // matching the spec).
                        let target = arguments[0].clone();
                        let result = match property.sym.as_ref() {
                            "preventExtensions" => {
                                self.mark_object_state(&target, false, false, true);
                                true
                            }
                            "isExtensible" => !self.object_state(&target).nonextensible,
                            // `setPrototypeOf`: false only when changing the
                            // prototype of a non-extensible object. thaw's
                            // prototype is always reported as `null`, so a
                            // `null` request is a no-op (and succeeds).
                            _ => {
                                let proto_is_null = matches!(
                                    arguments.get(1),
                                    Some(HirExpr::Lit(HirLit::Null))
                                );
                                !self.object_state(&target).nonextensible || proto_is_null
                            }
                        };
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(result)),
                            &bindings,
                        );
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
                            return Err(
                                "`Reflect.construct` expects exactly two arguments".into()
                            );
                        };
                        let Expr::Array(array) = arguments.expr.as_ref() else {
                            return Err(
                                "`Reflect.construct` currently requires an array literal".into()
                            );
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
                        let [target, key, value] = arguments.as_slice() else {
                            return Err("`Reflect.set` expects exactly three arguments".into());
                        };
                        // A frozen object rejects the write (`Reflect.set`
                        // returns `false` rather than throwing).
                        if self.object_state(target).frozen {
                            return self.wrap_call_argument_bindings(
                                HirExpr::Lit(HirLit::Bool(false)),
                                &bindings,
                            );
                        }
                        let HirExpr::Lit(HirLit::Str(key)) = key else {
                            return Err(
                                "`Reflect.set` currently requires a string-literal key".into()
                            );
                        };
                        let key = key.clone();
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
                                    key,
                                    Box::new(value.clone()),
                                )
                            }
                            HirType::Json => HirExpr::JsonSet(
                                Box::new(target.clone()),
                                Box::new(HirExpr::Lit(HirLit::Str(key))),
                                Box::new(value.clone()),
                                value_type,
                                true,
                            ),
                            other => {
                                return Err(format!(
                                    "`Reflect.set` currently requires a fixed object or JSON \
                                     receiver, got {other:?}"
                                ))
                            }
                        };
                        // `Reflect.set` returns a boolean (true on success),
                        // so run the assignment for its side effect and
                        // return `true` from a zero-argument closure.
                        let body = HirExpr::Block(vec![
                            HirStmt::Expr(assign),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
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
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [target, key] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects exactly two arguments"));
                        };
                        if property.sym == *"has" {
                            let result = self.lower_has_own_value(target.clone(), key.clone())?;
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        // A literal key resolves against a fixed object's
                        // own fields at lowering time; a `Json` receiver
                        // defers to the runtime.
                        let key_literal = match key {
                            HirExpr::Lit(HirLit::Str(value)) => Some(value.clone()),
                            _ => None,
                        };
                        let target_type = self.infer_expr_type(target)?;
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
                                let result = HirExpr::JsonDelete(
                                    Box::new(target.clone()),
                                    Box::new(HirExpr::Lit(HirLit::Str(key))),
                                );
                                return self.wrap_call_argument_bindings(result, &bindings);
                            }
                            _ => {
                                return Err(format!(
                                    "`{label}` currently requires a fixed object or JSON \
                                     receiver and a string-literal key, got {target_type:?}"
                                ))
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
                                let result = self.dynamic_value_check(
                                    "__thaw_is_buffer_dynamic_value",
                                    value,
                                );
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
                            let (arguments, bindings) = self
                                .lower_native_spread_values(&call.args, "Buffer.byteLength")?;
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
                                    Box::new(HirExpr::Var(
                                        "__thaw_bytes_byte_length".to_string(),
                                    )),
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
                                [list, total] => {
                                    (list, self.coerce_primitive_to_number(total.clone())?)
                                }
                                _ => {
                                    return Err(
                                        "`Buffer.concat` expects a list and an optional totalLength"
                                            .into(),
                                    )
                                }
                            };
                            let list_type = self.infer_expr_type(list)?;
                            if !matches!(&list_type, HirType::Array(element)
                                if matches!(element.as_ref(),
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
                        if property.sym == *"alloc" {
                            let (arguments, bindings) =
                                self.lower_native_spread_values(&call.args, "Buffer.alloc")?;
                            let [size] = arguments.as_slice() else {
                                return Err("`Buffer.alloc` expects a size".into());
                            };
                            let size = self.coerce_primitive_to_number(size.clone())?;
                            return self.wrap_call_argument_bindings(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_bytes_alloc".to_string())),
                                    vec![size],
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
                        if matches!(&source_type, HirType::Array(elem) if **elem == HirType::F64) {
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
                    if object.sym == *"JSON" && property.sym == *"parse" {
                        if call.args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("`JSON.parse` does not support spread arguments".into());
                        }
                        if call.args.is_empty() || call.args.len() > 2 {
                            return Err("`JSON.parse` expects one or two arguments".into());
                        }
                        let text = self.lower_expr(&call.args[0].expr)?;
                        if call.args.len() == 1 {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("JSON.parse".to_string())),
                                vec![text],
                            ));
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
                                let hint_params = [HirType::Str, HirType::Json];
                                let params = if arrow.params.len() == hint_params.len() {
                                    hint_params.to_vec()
                                } else {
                                    vec![HirType::JsValue; arrow.params.len()]
                                };
                                self.lower_contextual_arrow(
                                    arrow,
                                    &params,
                                    Some(&HirType::Json),
                                )?
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
                    if object.sym == *"JSON" && property.sym == *"stringify" {
                        let (arguments, mut bindings) =
                            self.lower_native_spread_values(&call.args, "JSON.stringify")?;
                        if !(1..=3).contains(&arguments.len()) {
                            return Err("`JSON.stringify` expects one to three arguments".into());
                        }
                        let value = arguments[0].clone();
                        let value_type = self.infer_expr_type(&value)?;
                        let value = if value_type == HirType::JsValue {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("readDynamicValue".into())),
                                vec![value],
                            )
                        } else if matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            value
                        } else if json_convertible_native_type(&value_type) {
                            self.wrap_native_value_as_json(value, value_type.clone())?
                        } else {
                            return Err(format!(
                                "`JSON.stringify` requires a JSON or dictionary value, got {value_type:?}"
                            ));
                        };
                        let mut replacer_array = None;
                        let mut replacer_function = None;
                        if let Some(replacer) = arguments.get(1) {
                            let replacer_type = self.infer_expr_type(replacer)?;
                            match &replacer_type {
                                HirType::Array(element) if element.as_ref() == &HirType::Str => {
                                    replacer_array = Some(replacer.clone());
                                }
                                HirType::Function(_, _) | HirType::CallableFunction(_, _, _, _) => {
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
                        let space = match arguments.get(2) {
                            None => None,
                            Some(space) => match self.infer_expr_type(space)? {
                                HirType::Null | HirType::Undefined => {
                                    if !matches!(space, HirExpr::Lit(_)) {
                                        let ty = self.infer_expr_type(space)?;
                                        let name = format!(
                                            "__thaw_json_stringify_space_{}",
                                            self.next_binding
                                        );
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
                                    ))
                                }
                            },
                        };
                        if let Some(replacer) = replacer_function {
                            let space = space
                                .map(|(value, _)| value)
                                .unwrap_or(HirExpr::Lit(HirLit::Null));
                            let arguments = self.coerce_to_declared(
                                &HirType::Json,
                                HirExpr::ArrayLit(vec![value, space]),
                            )?;
                            let result = HirExpr::JsonAsString(Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("callDynamicValueMixed".into())),
                                vec![
                                    HirExpr::Call(
                                        Box::new(HirExpr::Var("getDynamicValue".into())),
                                        vec![HirExpr::Lit(HirLit::Str(
                                            "__thaw_json_stringify_replacer".into(),
                                        ))],
                                    ),
                                    arguments,
                                    HirExpr::ArrayLit(vec![HirExpr::Call(
                                        Box::new(HirExpr::Var("registerNativeCallback".into())),
                                        vec![replacer],
                                    )]),
                                ],
                            )));
                            return self.wrap_call_argument_bindings(result, &bindings);
                        }
                        let result = match (replacer_array, space) {
                            (None, None) => HirExpr::Call(
                                Box::new(HirExpr::Var("JSON.stringify".into())),
                                vec![value],
                            ),
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
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(runtime.into())),
                                    vec![value, space],
                                )
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
                        let args = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![value]),
                        )?;
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
                            if matches!(params.as_slice(), [HirType::I64, HirType::Str, _, HirType::Array(_), HirType::Array(_), HirType::Array(_)])
                                && matches!(result.as_ref(), HirType::Promise(generated) if matches!(generated.as_ref(), HirType::Array(_))))
                        {
                            return Err("`Iterator.from` does not accept an async generator".into());
                        }
                        let source = if matches!(&source_type, HirType::Function(params, result)
                            if matches!(params.as_slice(), [HirType::I64, HirType::Str, _, HirType::Array(_), HirType::Array(_), HirType::Array(_)])
                                && matches!(result.as_ref(), HirType::Array(_)))
                        {
                            let producer = format!("__thaw_iterator_producer_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(producer.clone(), source_type.clone());
                            bindings.push((producer.clone(), source_type.clone(), source.clone()));
                            let return_parameter =
                                format!("__thaw_iterator_return_{}", self.next_binding);
                            self.next_binding += 1;
                            let throw_parameter =
                                format!("__thaw_iterator_throw_{}", self.next_binding);
                            self.next_binding += 1;
                            let next_parameter =
                                format!("__thaw_iterator_next_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut lower_protocol_method =
                                |method: &str, parameter: Option<(String, HirType)>| {
                                let protocol_call = CallExpr {
                                    span: call.span,
                                    ctxt: call.ctxt,
                                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                        span: call.span,
                                        obj: Box::new(Expr::Ident(
                                            swc_ecma_ast::Ident::new_no_ctxt(
                                                producer.clone().into(),
                                                call.span,
                                            ),
                                        )),
                                        prop: MemberProp::Ident(IdentName::new(
                                            method.into(),
                                            call.span,
                                        )),
                                    }))),
                                    args: parameter
                                        .as_ref()
                                        .map(|(name, _)| {
                                            vec![swc_ecma_ast::ExprOrSpread {
                                                spread: None,
                                                expr: Box::new(Expr::Ident(
                                                    swc_ecma_ast::Ident::new_no_ctxt(
                                                        name.clone().into(),
                                                        call.span,
                                                    ),
                                                )),
                                            }]
                                        })
                                        .unwrap_or_default(),
                                    type_args: None,
                                };
                                let previous = parameter.as_ref().and_then(|(name, ty)| {
                                    self.scope.insert(name.clone(), ty.clone())
                                });
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
                                        HirType::F64 => HirExpr::JsonAsNumber(Box::new(
                                            HirExpr::Var(name.clone()),
                                        )),
                                        HirType::Str => HirExpr::JsonAsString(Box::new(
                                            HirExpr::Var(name.clone()),
                                        )),
                                        HirType::Bool => HirExpr::JsonAsBool(Box::new(
                                            HirExpr::Var(name.clone()),
                                        )),
                                        HirType::Undefined => {
                                            HirExpr::Lit(HirLit::Undefined)
                                        }
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
                            HirExpr::ObjectLit(vec![
                                (
                                    "__thawNativeIterator".into(),
                                    HirExpr::Lit(HirLit::Bool(true)),
                                ),
                                ("next".into(), lower_protocol_method("next", None)?),
                                (
                                    "__thawNext".into(),
                                    lower_protocol_method(
                                        "next",
                                        Some((next_parameter, params[2].clone())),
                                    )?,
                                ),
                                ("return".into(), lower_protocol_method("return", None)?),
                                (
                                    "__thawReturn".into(),
                                    lower_protocol_method(
                                        "return",
                                        Some((
                                            return_parameter,
                                            return_values.as_ref().clone(),
                                        )),
                                    )?,
                                ),
                                (
                                    "__thawThrow".into(),
                                    lower_protocol_method(
                                        "throw",
                                        Some((throw_parameter, HirType::Str)),
                                    )?,
                                ),
                            ])
                        } else {
                            source.clone()
                        };
                        let source = self.coerce_to_declared(&HirType::Json, source)?;
                        let args = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![source]),
                        )?;
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
                                if !pending.is_empty() {
                                    parts.push(HirExpr::ArrayLit(std::mem::take(&mut pending)));
                                }
                                // Same snapshot conversion array-literal spreads
                                // (`[...str]`/`[...map]`/`[...set]`) already use.
                                let spread_source_type = self.infer_expr_type(&value)?;
                                if spread_source_type == HirType::Str {
                                    value = HirExpr::Call(
                                        Box::new(HirExpr::Var("__thaw_string_to_array".into())),
                                        vec![value],
                                    );
                                } else if let HirType::Map(key_type, value_type) =
                                    &spread_source_type
                                {
                                    let pair_type = HirType::Tuple(vec![
                                        key_type.as_ref().clone(),
                                        value_type.as_ref().clone(),
                                    ]);
                                    let array_type = HirType::Array(Box::new(pair_type));
                                    value = HirExpr::TypedClosure(
                                        array_type,
                                        Box::new(HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_map_snapshot_entries".into(),
                                            )),
                                            vec![value],
                                        )),
                                    );
                                } else if let HirType::Set(set_element) = &spread_source_type {
                                    let array_type = HirType::Array(set_element.clone());
                                    value = HirExpr::TypedClosure(
                                        array_type,
                                        Box::new(HirExpr::Call(
                                            Box::new(HirExpr::Var(
                                                "__thaw_map_snapshot_keys".into(),
                                            )),
                                            vec![value],
                                        )),
                                    );
                                }
                                let ty = self.infer_expr_type(&value)?;
                                let HirType::Array(element) = ty else {
                                    return Err(
                                        "`Array.of` spread requires a homogeneous array, string, Map, or Set".into()
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
                                        let params = match self.infer_expr_type(&callback)? {
                                            HirType::Function(params, _)
                                            | HirType::CallableFunction(params, _, _, _) => params,
                                            _ => {
                                                return Err(
                                                    "Array.from mapper is not a function value"
                                                        .into(),
                                                )
                                            }
                                        };
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
                                    } else {
                                        Some(self.lower_array_from_callback(
                                            &call.args[1].expr,
                                            &HirType::Undefined,
                                        )?)
                                    };
                                    let this_arg = if has_spread {
                                        spread_arguments.get(2).cloned()
                                    } else {
                                        call.args
                                            .get(2)
                                            .map(|argument| self.lower_expr(&argument.expr))
                                            .transpose()?
                                    };
                                    let result = self.lower_array_from_length(
                                        length, callback, this_arg,
                                    )?;
                                    return self
                                        .wrap_call_argument_bindings(result, &spread_bindings);
                                }
                            }
                        }
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
                                        Box::new(HirExpr::Var(
                                            "__thaw_map_snapshot_entries".into(),
                                        )),
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
                                let iterator_name =
                                    format!("__thaw_array_from_iterator_{}", self.next_binding);
                                self.next_binding += 1;
                                let array_name =
                                    format!("__thaw_array_from_result_{}", self.next_binding);
                                self.next_binding += 1;
                                let result_name =
                                    format!("__thaw_array_from_next_{}", self.next_binding);
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
                                    HirStmt::While(
                                        HirExpr::Lit(HirLit::Bool(true)),
                                        loop_body,
                                    ),
                                    HirStmt::Return(Some(HirExpr::Var(array_name))),
                                ]);
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
                                    vec![source],
                                );
                                (source, array_type, element_type)
                            }
                            other => {
                                return Err(format!(
                                    "native `Array.from` requires a homogeneous array, string, Map, or Set, got {other:?}"
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
                    if matches!(object.sym.as_ref(), "Map" | "Object")
                        && property.sym == *"groupBy"
                    {
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
                        let items = if has_spread {
                            arguments[0].clone()
                        } else {
                            self.lower_expr(&call.args[0].expr)?
                        };
                        let items_type = self.infer_expr_type(&items)?;
                        let HirType::Array(item_type) = &items_type else {
                            return Err(format!(
                                "`{label}` requires a homogeneous array, got {items_type:?}"
                            ));
                        };
                        let item_type = item_type.as_ref().clone();
                        let key_fn = if has_spread {
                            let key_fn = arguments[1].clone();
                            let params = match self.infer_expr_type(&key_fn)? {
                                HirType::Function(params, _)
                                | HirType::CallableFunction(params, _, _, _) => params,
                                _ => {
                                    return Err(
                                        format!("{label} key function is not a function value")
                                    )
                                }
                            };
                            if params.len() > 2 {
                                return Err(format!(
                                    "{label} key function accepts at most two parameters, got {}",
                                    params.len()
                                ));
                            }
                            let available = [item_type.clone(), HirType::F64];
                            self.validate_promise_callback_value(
                                &key_fn,
                                &available[..params.len()],
                                None,
                            )?;
                            key_fn
                        } else {
                            // Reuses `Array.from`'s own contextual-typing helper: its
                            // "zero-to-two-argument (element, index)" shape is exactly
                            // `Map.groupBy`'s own `(item, index) => key` callback.
                            self.lower_array_from_callback(&call.args[1].expr, &item_type)?
                        };
                        let result =
                            self.lower_group_by(items, item_type, key_fn, object_result)?;
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
                        let json_arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(json_arguments),
                        )?;
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
                        let source_types = sources
                            .iter()
                            .map(|source| self.infer_expr_type(source))
                            .collect::<Result<Vec<_>, _>>()?;
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
                            && source_types.iter().all(|source_type| *source_type == target_type);
                        let mut result = target.clone();
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
                    if object.sym == *"Iterator"
                        && matches!(property.sym.as_ref(), "concat" | "zip" | "zipKeyed")
                    {
                        // Lazy iterator composition lives in the JS realm
                        // (the bundled QuickJS provides
                        // `Iterator.concat`/`zip`/`zipKeyed`), so keep the
                        // result a live handle and let its own
                        // `.toArray()`/`.next()` run there too.
                        let label = format!("Iterator.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let json_arguments = arguments
                            .iter()
                            .map(|argument| {
                                self.coerce_to_declared(&HirType::Json, argument.clone())
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        let json_arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(json_arguments),
                        )?;
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
                        let json_arguments = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![target, handler]),
                        )?;
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
                                return Err(
                                    "`Intl.supportedValuesOf` requires a string-literal key".into()
                                );
                            };
                            let values = supported_values_of(&key.value.to_string_lossy());
                            return Ok(HirExpr::ArrayLit(
                                values
                                    .into_iter()
                                    .map(|value| HirExpr::Lit(HirLit::Str(value)))
                                    .collect(),
                            ));
                        }
                        let canonical = |value: &str| {
                            HirExpr::Lit(HirLit::Str(canonicalize_locale(value)))
                        };
                        match argument {
                            Expr::Lit(Lit::Str(value)) => {
                                return Ok(HirExpr::ArrayLit(vec![canonical(
                                    &value.value.to_string_lossy(),
                                )]));
                            }
                            Expr::Array(array) => {
                                let mut locales = Vec::with_capacity(array.elems.len());
                                for element in &array.elems {
                                    let Some(element) = element else {
                                        return Err(
                                            "`Intl.getCanonicalLocales` does not support holes"
                                                .into(),
                                        );
                                    };
                                    if element.spread.is_some() {
                                        return Err(
                                            "`Intl.getCanonicalLocales` does not support spread"
                                                .into(),
                                        );
                                    }
                                    let Expr::Lit(Lit::Str(value)) = element.expr.as_ref() else {
                                        return Err(
                                            "`Intl.getCanonicalLocales` requires string-literal locales"
                                                .into(),
                                        );
                                    };
                                    locales.push(canonical(&value.value.to_string_lossy()));
                                }
                                return Ok(HirExpr::ArrayLit(locales));
                            }
                            _ => {
                                return Err(
                                    "`Intl.getCanonicalLocales` requires a string or string-array literal"
                                        .into(),
                                )
                            }
                        }
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
                                            .strip_prefix("__thaw_class_identity_")
                                            .is_some_and(|chain| {
                                                chain.split('$').any(|name| matches!(
                                                    name,
                                                    "Error"
                                                        | "TypeError"
                                                        | "RangeError"
                                                        | "SyntaxError"
                                                        | "ReferenceError"
                                                        | "EvalError"
                                                        | "URIError"
                                                        | "AggregateError"
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
                    if object.sym == *"BigInt"
                        && matches!(property.sym.as_ref(), "asIntN" | "asUintN")
                    {
                        // `BigInt.asIntN(bits, value)` / `asUintN(bits, value)`:
                        // thaw's BigInt is a fixed-width `i64`, so these are
                        // native bit-mask operations (see the runtime's
                        // `thaw_i64_as_int_n`/`thaw_i64_as_uint_n`), not the
                        // unbounded-precision wraparound real JS performs.
                        let label = format!("BigInt.{}", property.sym);
                        let (arguments, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        let [bits, value] = arguments.as_slice() else {
                            return Err(format!("`{label}` expects two arguments"));
                        };
                        let bits = self.coerce_primitive_to_number(bits.clone())?;
                        let value = value.clone();
                        self.expect_type(&HirType::I64, &value, &format!("{label} value"))?;
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
                    if object.sym == *"Number"
                        && matches!(property.sym.as_ref(), "parseFloat" | "parseInt")
                    {
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
                        let args = self.coerce_to_declared(
                            &HirType::Json,
                            HirExpr::ArrayLit(vec![value]),
                        )?;
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
                    if object.sym == *"Math" && matches!(property.sym.as_ref(), "min" | "max") {
                        if let [argument] = call.args.as_slice() {
                            if argument.spread.is_some() {
                                let source = self.lower_expr(&argument.expr)?;
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
                                if self.infer_expr_type(&source)?
                                    == HirType::Array(Box::new(HirType::F64))
                                    && !matches!(source, HirExpr::ArrayLit(_))
                                {
                                    return self.lower_math_extreme_of_array(
                                        source,
                                        property.sym == *"min",
                                    );
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
                        let name = format!("__thaw_number_predicate_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name, ty, value));
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(false)),
                            &bindings,
                        );
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
                                "`String.raw` currently requires an object-literal strings argument"
                                    .into(),
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
                            return Err(
                                "`String.raw` currently requires a `raw` array literal".into()
                            );
                        };
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
                                    "`String.raw` currently requires string-literal raw segments"
                                        .into(),
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
                                    if substitution.spread.is_some() {
                                        return Err(
                                            "`String.raw` does not support a spread substitution"
                                                .into(),
                                        );
                                    }
                                    let value = self.lower_expr(&substitution.expr)?;
                                    parts.push(self.coerce_primitive_to_string(value)?);
                                }
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
                        let result_name =
                            format!("__thaw_from_code_point_result_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(result_name.clone(), HirType::Str);
                        let var = |name: &str| HirExpr::Var(name.into());
                        let mut stmts = vec![HirStmt::Let(
                            result_name.clone(),
                            HirType::Str,
                            HirExpr::Lit(HirLit::Str(String::new())),
                        )];
                        for point in points {
                            let raw_name =
                                format!("__thaw_from_code_point_raw_{}", self.next_binding);
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
                                    "Invalid code point".into(),
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
