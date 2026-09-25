impl<'a> FnLowerer<'a> {
    fn lower_native_map_set_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"get" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    // A `Map` value stored in `any` -- `Map<K, V>` is
                    // generic and a bare `any` receiver carries no
                    // record of what `K`/`V` originally were, so this
                    // can't decode into a real native `HirType::Map`
                    // the way `RegExp`'s fixed shape does. Instead
                    // reads the wrapped `__thaw_map_entries__` array
                    // directly at runtime (`thaw_json_map_or_set_get`,
                    // thaw-std's `json.rs`) -- a linear scan, same
                    // SameValue-based key comparison every other
                    // JSON-value-equality check in this codebase uses.
                    // Result is `any`/`Json` (no static `V` to decode
                    // into), matching what a real dynamic `.get()`
                    // genuinely returns.
                    if receiver_type == HirType::Json {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Map.get")?;
                        let [key] = arguments.as_slice() else {
                            return Err("native `.get()` expects exactly one argument".into());
                        };
                        let key = self.coerce_to_declared(&HirType::Json, key.clone())?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_get".to_string())),
                            vec![receiver, key],
                        );
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                    }
                    let (key_type, value_type) = match &receiver_type {
                        HirType::Map(key_type, value_type)
                        | HirType::WeakMap(key_type, value_type) => (key_type, value_type),
                        _ => {
                            return Err(format!(
                                "native `.get()` requires a Map receiver, got {receiver_type:?}"
                            ))
                        }
                    };
                    let key_type = key_type.as_ref().clone();
                    let value_type = value_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (value_suffix, needs_type_wrap) = map_value_get_suffix(&value_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Map.get")?;
                    let [key] = arguments.as_slice() else {
                        return Err("native `.get()` expects exactly one argument".into());
                    };
                    let key = self.coerce_map_key(&key_type, key.clone())?;
                    let receiver_name = format!("__thaw_map_get_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_get_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let has_intrinsic = format!("__thaw_map_{key_suffix}_has");
                    let get_intrinsic = format!("__thaw_map_{key_suffix}_get_{value_suffix}");
                    let raw_get = HirExpr::Call(
                        Box::new(HirExpr::Var(get_intrinsic)),
                        vec![var(&receiver_name), var(&key_name)],
                    );
                    let decoded = if needs_type_wrap {
                        HirExpr::TypedClosure(value_type.clone(), Box::new(raw_get))
                    } else {
                        raw_get
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var(has_intrinsic)),
                                vec![var(&receiver_name), var(&key_name)],
                            ),
                            Vec::new(),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                value_type.clone(),
                            )))],
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(decoded),
                            value_type.clone(),
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(value_type));
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
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"set" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    // `.set()` on a `Map` value stored in `any` -- Stage
                    // B. Unlike Stage A's read-only methods, a mutation
                    // has to be visible to every other reader of the
                    // same variable afterward, but `Json`/`any` values
                    // in this compiler are snapshots, not references.
                    // Only supported when the receiver is a plain local
                    // variable: computes the whole new sentinel object
                    // (`__thaw_json_map_or_set_set`, thaw-std's
                    // `json.rs`) and assigns it straight back onto that
                    // variable's own binding -- a closure capturing a
                    // local retains that local's real storage cell
                    // (`allocate_lambda_environment`'s own doc comment,
                    // thaw-llvm's `closures.rs`), so the `Assign` below
                    // genuinely writes back, not just to a copy. A
                    // receiver that's a property chain (or any other
                    // non-identifier expression) is a narrower,
                    // documented limitation, matching this session's
                    // established scope-drawing precedent (round8's
                    // RegExp `lastIndex`).
                    if receiver_type == HirType::Json {
                        let Expr::Ident(ident) = member.obj.as_ref() else {
                            return Err(
                                "`.set()` on a Map stored in `any` is only supported when the \
                                 receiver is a plain local variable, not a property chain"
                                    .into(),
                            );
                        };
                        let var_name = self.resolve_binding(ident.sym.as_ref());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Map.set")?;
                        let [key, value] = arguments.as_slice() else {
                            return Err("native `.set()` expects exactly two arguments".into());
                        };
                        let key = self.coerce_to_declared(&HirType::Json, key.clone())?;
                        let value = self.coerce_to_declared(&HirType::Json, value.clone())?;
                        let key_name = format!("__thaw_map_set_any_key_{}", self.next_binding);
                        self.next_binding += 1;
                        let value_name = format!("__thaw_map_set_any_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(key_name.clone(), HirType::Json);
                        self.scope.insert(value_name.clone(), HirType::Json);
                        let var = |name: &str| HirExpr::Var(name.to_string());
                        let new_value = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_set".to_string())),
                            vec![var(&var_name), var(&key_name), var(&value_name)],
                        );
                        let body = HirExpr::Block(vec![
                            HirStmt::Expr(HirExpr::Assign(var_name.clone(), Box::new(new_value))),
                            HirStmt::Return(Some(var(&var_name))),
                        ]);
                        let mut bindings = vec![(key_name, HirType::Json, key)];
                        bindings.push((value_name, HirType::Json, value));
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(body, &bindings);
                    }
                    let (key_type, value_type) = match &receiver_type {
                        HirType::Map(key_type, value_type)
                        | HirType::WeakMap(key_type, value_type) => (key_type, value_type),
                        _ => {
                            return Err(format!(
                                "native `.set()` requires a Map receiver, got {receiver_type:?}"
                            ))
                        }
                    };
                    let key_type = key_type.as_ref().clone();
                    let value_type = value_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Map.set")?;
                    let [key, value] = arguments.as_slice() else {
                        return Err("native `.set()` expects exactly two arguments".into());
                    };
                    let key = self.coerce_map_key(&key_type, key.clone())?;
                    let value = self
                        .coerce_to_declared(&value_type, value.clone())
                        .map_err(|error| format!("Map.set value: {error}"))?;
                    let receiver_name = format!("__thaw_map_set_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_set_key_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_map_set_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    self.scope.insert(value_name.clone(), value_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(key_name.clone()),
                            HirExpr::Var(value_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    bindings.push((value_name, value_type, value.clone()));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"getOrInsert" || property.sym == *"getOrInsertComputed" {
                    let label = format!("Map.{}", property.sym);
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (key_type, value_type) = match &receiver_type {
                        HirType::Map(key_type, value_type)
                        | HirType::WeakMap(key_type, value_type) => (key_type, value_type),
                        _ => {
                            return Err(format!(
                                "native `.{}()` requires a Map receiver, got {receiver_type:?}",
                                property.sym
                            ))
                        }
                    };
                    let key_type = key_type.as_ref().clone();
                    let value_type = value_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (value_suffix, needs_type_wrap) = map_value_get_suffix(&value_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, &label)?;
                    let [key, second] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly two arguments",
                            property.sym
                        ));
                    };
                    let key = self.coerce_map_key(&key_type, key.clone())?;
                    let receiver_name =
                        format!("__thaw_map_get_or_insert_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_get_or_insert_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let has_intrinsic = format!("__thaw_map_{key_suffix}_has");
                    let get_intrinsic = format!("__thaw_map_{key_suffix}_get_{value_suffix}");
                    let set_intrinsic = format!("__thaw_map_{key_suffix}_set");
                    let raw_get = HirExpr::Call(
                        Box::new(HirExpr::Var(get_intrinsic)),
                        vec![var(&receiver_name), var(&key_name)],
                    );
                    let decoded = if needs_type_wrap {
                        HirExpr::TypedClosure(value_type.clone(), Box::new(raw_get))
                    } else {
                        raw_get
                    };
                    let mut body = vec![HirStmt::If(
                        HirExpr::Call(
                            Box::new(HirExpr::Var(has_intrinsic)),
                            vec![var(&receiver_name), var(&key_name)],
                        ),
                        vec![HirStmt::Return(Some(decoded))],
                        Vec::new(),
                    )];
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name.clone(), key_type, key.clone()));
                    let inserted = if property.sym == *"getOrInsertComputed" {
                        // The callback runs only when the key is absent
                        // (after the early return above) and is called with
                        // the key.
                        let computed_name =
                            format!("__thaw_map_get_or_insert_computed_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(computed_name.clone(), value_type.clone());
                        let call = HirExpr::Call(Box::new(second.clone()), vec![key]);
                        let call = self
                            .coerce_to_declared(&value_type, call)
                            .map_err(|error| format!("Map.getOrInsertComputed callback result: {error}"))?;
                        body.push(HirStmt::Let(computed_name.clone(), value_type.clone(), call));
                        var(&computed_name)
                    } else {
                        let value = self
                            .coerce_to_declared(&value_type, second.clone())
                            .map_err(|error| format!("Map.getOrInsert value: {error}"))?;
                        let value_name =
                            format!("__thaw_map_get_or_insert_value_{}", self.next_binding);
                        self.next_binding += 1;
                        bindings.push((value_name.clone(), value_type.clone(), value));
                        self.scope.insert(value_name.clone(), value_type.clone());
                        var(&value_name)
                    };
                    body.push(HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var(set_intrinsic)),
                        vec![
                            var(&receiver_name),
                            var(&key_name),
                            inserted.clone(),
                        ],
                    )));
                    body.push(HirStmt::Return(Some(inserted)));
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
                            value_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"add" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    // `.add()` on a Set stored in `any` -- same
                    // lvalue-write-back rationale as `.set()`'s own
                    // `Json` branch just above.
                    if receiver_type == HirType::Json {
                        let Expr::Ident(ident) = member.obj.as_ref() else {
                            return Err(
                                "`.add()` on a Set stored in `any` is only supported when the \
                                 receiver is a plain local variable, not a property chain"
                                    .into(),
                            );
                        };
                        let var_name = self.resolve_binding(ident.sym.as_ref());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Set.add")?;
                        let [element] = arguments.as_slice() else {
                            return Err("native `.add()` expects exactly one argument".into());
                        };
                        let element = self.coerce_to_declared(&HirType::Json, element.clone())?;
                        let element_name = format!("__thaw_set_add_any_element_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(element_name.clone(), HirType::Json);
                        let var = |name: &str| HirExpr::Var(name.to_string());
                        let new_value = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_add".to_string())),
                            vec![var(&var_name), var(&element_name)],
                        );
                        let body = HirExpr::Block(vec![
                            HirStmt::Expr(HirExpr::Assign(var_name.clone(), Box::new(new_value))),
                            HirStmt::Return(Some(var(&var_name))),
                        ]);
                        let mut bindings = vec![(element_name, HirType::Json, element)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(body, &bindings);
                    }
                    let element_type = match &receiver_type {
                        HirType::Set(element_type) | HirType::WeakSet(element_type) => element_type,
                        _ => {
                            return Err(format!(
                                "native `.add()` requires a Set receiver, got {receiver_type:?}"
                            ))
                        }
                    };
                    let element_type = element_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&element_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Set.add")?;
                    let [element] = arguments.as_slice() else {
                        return Err("native `.add()` expects exactly one argument".into());
                    };
                    let element = self.coerce_map_key(&element_type, element.clone())?;
                    let receiver_name = format!("__thaw_set_add_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let element_name = format!("__thaw_set_add_element_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(element_name.clone(), element_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(element_name.clone()),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((element_name, element_type, element));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "has" | "delete") {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    // `.has()` on a `Map`/`Set` value stored in `any` --
                    // same rationale as `.get()`'s own `Json` branch
                    // just above: a linear scan over whichever sentinel
                    // array is present (`thaw_json_map_or_set_has`,
                    // thaw-std's `json.rs`), checking a Map's own keys
                    // or a Set's own values as appropriate.
                    if property.sym == *"has" && receiver_type == HirType::Json {
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Map/Set.has")?;
                        let [key] = arguments.as_slice() else {
                            return Err("native `.has()` expects exactly one argument".into());
                        };
                        let key = self.coerce_to_declared(&HirType::Json, key.clone())?;
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_has".to_string())),
                            vec![receiver, key],
                        );
                        return self.wrap_call_argument_bindings(result, &spread_bindings);
                    }
                    // `.delete()` (mutating) -- Stage B, same
                    // lvalue-write-back rationale as `.set()`/`.add()`.
                    // Returns whether a matching key/element was
                    // actually present (`.has()`, evaluated on the
                    // *original* value before the write-back), matching
                    // real `Map.prototype.delete`/`Set.prototype.delete`.
                    if property.sym == *"delete" && receiver_type == HirType::Json {
                        let Expr::Ident(ident) = member.obj.as_ref() else {
                            return Err(
                                "`.delete()` on a Map/Set stored in `any` is only supported when \
                                 the receiver is a plain local variable, not a property chain"
                                    .into(),
                            );
                        };
                        let var_name = self.resolve_binding(ident.sym.as_ref());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Map/Set.delete")?;
                        let [key] = arguments.as_slice() else {
                            return Err("native `.delete()` expects exactly one argument".into());
                        };
                        let key = self.coerce_to_declared(&HirType::Json, key.clone())?;
                        let key_name = format!("__thaw_map_delete_any_key_{}", self.next_binding);
                        self.next_binding += 1;
                        let existed_name =
                            format!("__thaw_map_delete_any_existed_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(key_name.clone(), HirType::Json);
                        self.scope.insert(existed_name.clone(), HirType::Bool);
                        let var = |name: &str| HirExpr::Var(name.to_string());
                        let had_key = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_has".to_string())),
                            vec![var(&var_name), var(&key_name)],
                        );
                        let new_value = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_delete".to_string())),
                            vec![var(&var_name), var(&key_name)],
                        );
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(existed_name.clone(), HirType::Bool, had_key),
                            HirStmt::Expr(HirExpr::Assign(var_name.clone(), Box::new(new_value))),
                            HirStmt::Return(Some(var(&existed_name))),
                        ]);
                        let mut bindings = vec![(key_name, HirType::Json, key)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(body, &bindings);
                    }
                    let key_type = match &receiver_type {
                        HirType::Map(key_type, _) => key_type.as_ref().clone(),
                        HirType::WeakMap(key_type, _) => key_type.as_ref().clone(),
                        HirType::Set(element_type) => element_type.as_ref().clone(),
                        HirType::WeakSet(element_type) => element_type.as_ref().clone(),
                        other => {
                            return Err(format!(
                                "native `.{}()` requires a Map or Set receiver, got {other:?}",
                                property.sym
                            ))
                        }
                    };
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Map/Set.{}", property.sym),
                    )?;
                    let [key] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly one argument",
                            property.sym
                        ));
                    };
                    let key = self.coerce_map_key(&key_type, key.clone())?;
                    let intrinsic = if property.sym == *"has" {
                        format!("__thaw_map_{key_suffix}_has")
                    } else {
                        format!("__thaw_map_{key_suffix}_delete")
                    };
                    let receiver_name =
                        format!("__thaw_map_set_query_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_set_query_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic)),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(key_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"clear" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    // `.clear()` on a Map/Set stored in `any` -- same
                    // lvalue-write-back rationale as `.set()`/`.add()`/
                    // `.delete()`.
                    if receiver_type == HirType::Json {
                        let Expr::Ident(ident) = member.obj.as_ref() else {
                            return Err(
                                "`.clear()` on a Map/Set stored in `any` is only supported when \
                                 the receiver is a plain local variable, not a property chain"
                                    .into(),
                            );
                        };
                        if !call.args.is_empty() {
                            return Err("native `.clear()` expects no arguments".into());
                        }
                        let var_name = self.resolve_binding(ident.sym.as_ref());
                        let new_value = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_clear".to_string())),
                            vec![HirExpr::Var(var_name.clone())],
                        );
                        return Ok(HirExpr::Assign(var_name, Box::new(new_value)));
                    }
                    if !matches!(receiver_type, HirType::Map(_, _) | HirType::Set(_)) {
                        return Err(format!(
                            "native `.clear()` requires a Map or Set receiver, got {receiver_type:?}"
                        ));
                    }
                    if !call.args.is_empty() {
                        return Err("native `.clear()` expects no arguments".into());
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_clear".to_string())),
                        vec![receiver],
                    ));
                }
                if matches!(
                    property.sym.as_ref(),
                    "union" | "intersection" | "difference" | "symmetricDifference"
                ) {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Set(element_type) = &receiver_type else {
                        return Err(format!(
                            "native `.{}()` requires a Set receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element_type.as_ref().clone();
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Set.{}", property.sym),
                    )?;
                    let [other] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly one argument",
                            property.sym
                        ));
                    };
                    let other = other.clone();
                    let other_type = self.infer_expr_type(&other)?;
                    if !set_argument_is_compatible(&other_type, &element_type) {
                        return Err(format!(
                            "native `.{}()` requires a Set or Map argument with element/key type \
                             {element_type:?}, got {other_type:?}",
                            property.sym
                        ));
                    }
                    return self.lower_set_combine(
                        receiver,
                        other,
                        element_type,
                        other_type,
                        property.sym.as_ref(),
                        spread_bindings,
                    );
                }
                if matches!(
                    property.sym.as_ref(),
                    "isSubsetOf" | "isSupersetOf" | "isDisjointFrom"
                ) {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Set(element_type) = &receiver_type else {
                        return Err(format!(
                            "native `.{}()` requires a Set receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element_type.as_ref().clone();
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Set.{}", property.sym),
                    )?;
                    let [other] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly one argument",
                            property.sym
                        ));
                    };
                    let other = other.clone();
                    let other_type = self.infer_expr_type(&other)?;
                    if !set_argument_is_compatible(&other_type, &element_type) {
                        return Err(format!(
                            "native `.{}()` requires a Set or Map argument with element/key type \
                             {element_type:?}, got {other_type:?}",
                            property.sym
                        ));
                    }
                    // `isSupersetOf(other)` is `other.isSubsetOf(this)`;
                    // `isDisjointFrom` asks the same "does the OTHER set
                    // lack every element I test" question `isSubsetOf` asks
                    // for its own elements, just against the receiver's
                    // absence instead of presence.
                    let (scan_name, test_target, expect_present) = match property.sym.as_ref() {
                        "isSubsetOf" => ("receiver", "other", true),
                        "isSupersetOf" => ("other", "receiver", true),
                        _ => ("receiver", "other", false),
                    };
                    return self.lower_set_predicate(
                        receiver,
                        other,
                        element_type,
                        other_type,
                        scan_name,
                        test_target,
                        expect_present,
                        spread_bindings,
                    );
                }
                if property.sym == *"keys" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.keys()` expects no arguments".into());
                    }
                    if let HirType::Array(_) = &receiver_type {
                        return self.lower_array_keys(receiver, receiver_type);
                    }
                    // A `Map`/`Set` value stored in `any` -- unlike the
                    // static case's real lazy iterator (`lower_map_
                    // iterator`, below), this returns a plain `any[]`
                    // eagerly (`thaw_json_map_or_set_keys`, thaw-std's
                    // `json.rs` -- see its own doc comment for the
                    // "ponytail" scope note). Good enough for `for
                    // (const k of m.keys())`/`[...m.keys()]`, which
                    // only need something iterable.
                    if receiver_type == HirType::Json {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_keys".to_string())),
                            vec![receiver],
                        ));
                    }
                    let key_type = match &receiver_type {
                        HirType::Map(key_type, _) => key_type.as_ref().clone(),
                        HirType::Set(element_type) => element_type.as_ref().clone(),
                        other => {
                            return Err(format!(
                                "native `.keys()` requires a Map, Set, or Array receiver, got {other:?}"
                            ))
                        }
                    };
                    return self.lower_map_iterator(receiver, receiver_type, key_type, 0.0);
                }
                if property.sym == *"values" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.values()` expects no arguments".into());
                    }
                    if let HirType::Array(element_type) = &receiver_type {
                        return self.lower_array_values(
                            receiver,
                            receiver_type.clone(),
                            element_type.as_ref().clone(),
                        );
                    }
                    if receiver_type == HirType::Json {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_map_or_set_values".to_string())),
                            vec![receiver],
                        ));
                    }
                    let (value_type, mode) = match &receiver_type {
                        HirType::Map(_, value_type) => (value_type.as_ref().clone(), 1.0),
                        // A Set's elements ARE its "values" -- it has no
                        // separate value to snapshot.
                        HirType::Set(element_type) => {
                            (element_type.as_ref().clone(), 0.0)
                        }
                        other => {
                            return Err(format!(
                                "native `.values()` requires a Map, Set, or Array receiver, got {other:?}"
                            ))
                        }
                    };
                    return self.lower_map_iterator(receiver, receiver_type, value_type, mode);
                }
                if property.sym == *"entries" {
                    let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.entries()` expects no arguments".into());
                    }
                    if let HirType::Array(element_type) = &receiver_type {
                        let element_type = element_type.as_ref().clone();
                        return self.lower_array_entries(receiver, receiver_type, element_type);
                    }
                    if receiver_type == HirType::Json {
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(
                                "__thaw_json_map_or_set_entries_view".to_string(),
                            )),
                            vec![receiver],
                        ));
                    }
                    let (pair_type, mode) = match &receiver_type {
                        HirType::Map(key_type, value_type) => (
                            HirType::Tuple(vec![key_type.as_ref().clone(), value_type.as_ref().clone()]),
                            2.0,
                        ),
                        HirType::Set(element_type) => (
                            HirType::Tuple(vec![element_type.as_ref().clone(), element_type.as_ref().clone()]),
                            3.0,
                        ),
                        other => {
                            return Err(format!(
                                "native `.entries()` requires a Map, Set, or Array receiver, got {other:?}"
                            ))
                        }
                    };
                    return self.lower_map_iterator(receiver, receiver_type, pair_type, mode);
                }
        unreachable!("instance builtin category was checked before lowering")
    }

    fn lower_map_iterator(
        &mut self,
        receiver: HirExpr,
        receiver_type: HirType,
        yielded_type: HirType,
        mode: f64,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_map_iterator_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let cursor_name = format!("__thaw_map_iterator_cursor_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_map_iterator_result_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Tuple(vec![HirType::F64, yielded_type.clone()]);
        let generated_type = HirType::Array(Box::new(yielded_type.clone()));
        let producer_type = generator_function_type(
            false,
            yielded_type.clone(),
            HirType::Undefined,
            HirType::Undefined,
        );
        let params = [HirType::I64, HirType::Str, HirType::Undefined]
            .into_iter()
            .chain(std::iter::repeat_n(
                HirType::Array(Box::new(HirType::Undefined)),
                3,
            ))
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_map_iterator_arg_{index}_{}", self.next_binding),
                ty,
            })
            .collect::<Vec<_>>();
        self.next_binding += 1;
        let control_name = params[0].name.clone();
        let var = |name: &str| HirExpr::Var(name.into());
        let done = || HirStmt::Return(Some(HirExpr::ArrayLit(Vec::new())));
        let producer = HirExpr::Lambda(
            vec![
                HirParam {
                    name: receiver_name.clone(),
                    ty: receiver_type.clone(),
                },
                HirParam {
                    name: cursor_name.clone(),
                    ty: HirType::F64,
                },
            ],
            params,
            generated_type,
            Box::new(HirExpr::Block(vec![
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(var(&control_name)),
                        Box::new(HirExpr::Lit(HirLit::I64(0))),
                    ),
                    Vec::new(),
                    vec![done()],
                ),
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::Lt,
                        Box::new(var(&cursor_name)),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![done()],
                    Vec::new(),
                ),
                HirStmt::Let(
                    result_name.clone(),
                    result_type.clone(),
                    HirExpr::TypedClosure(
                        result_type,
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_iterator_next".into())),
                            vec![
                                var(&receiver_name),
                                var(&cursor_name),
                                HirExpr::Lit(HirLit::F64(mode)),
                            ],
                        )),
                    ),
                ),
                HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::ArrayLen(Box::new(var(&result_name)))),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            cursor_name.clone(),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )),
                        done(),
                    ],
                    Vec::new(),
                ),
                HirStmt::Expr(HirExpr::Assign(
                    cursor_name.clone(),
                    Box::new(HirExpr::TypedIndex(
                        Box::new(var(&result_name)),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        HirType::F64,
                    )),
                )),
                HirStmt::Return(Some(HirExpr::ArrayLit(vec![HirExpr::TypedIndex(
                    Box::new(var(&result_name)),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    yielded_type,
                )]))),
            ])),
        );
        let iterator = HirExpr::Call(
            Box::new(HirExpr::Lambda(
                vec![HirParam {
                    name: receiver_name.clone(),
                    ty: receiver_type.clone(),
                }],
                Vec::new(),
                producer_type,
                Box::new(HirExpr::Block(vec![
                    HirStmt::Let(cursor_name, HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
                    HirStmt::Return(Some(producer)),
                ])),
            )),
            Vec::new(),
        );
        self.wrap_call_argument_bindings(iterator, &[(receiver_name, receiver_type, receiver)])
    }
}
