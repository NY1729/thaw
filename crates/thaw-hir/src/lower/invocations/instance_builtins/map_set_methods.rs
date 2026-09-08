impl<'a> FnLowerer<'a> {
    fn lower_native_map_set_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"get" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Map(key_type, value_type) = &receiver_type else {
                        return Err(format!(
                            "native `.get()` requires a Map receiver, got {receiver_type:?}"
                        ));
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
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Map(key_type, value_type) = &receiver_type else {
                        return Err(format!(
                            "native `.set()` requires a Map receiver, got {receiver_type:?}"
                        ));
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
                    self.expect_type(&value_type, value, "Map.set value")?;
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
                if property.sym == *"add" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Set(element_type) = &receiver_type else {
                        return Err(format!(
                            "native `.add()` requires a Set receiver, got {receiver_type:?}"
                        ));
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
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let key_type = match &receiver_type {
                        HirType::Map(key_type, _) => key_type.as_ref().clone(),
                        HirType::Set(element_type) => element_type.as_ref().clone(),
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
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
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
                    let receiver = self.lower_expr(&member.obj)?;
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
                    if other_type != receiver_type {
                        return Err(format!(
                            "native `.{}()` requires a {receiver_type:?} argument, got {other_type:?}",
                            property.sym
                        ));
                    }
                    return self.lower_set_combine(
                        receiver,
                        other,
                        element_type,
                        property.sym.as_ref(),
                        spread_bindings,
                    );
                }
                if matches!(
                    property.sym.as_ref(),
                    "isSubsetOf" | "isSupersetOf" | "isDisjointFrom"
                ) {
                    let receiver = self.lower_expr(&member.obj)?;
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
                    if other_type != receiver_type {
                        return Err(format!(
                            "native `.{}()` requires a {receiver_type:?} argument, got {other_type:?}",
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
                        scan_name,
                        test_target,
                        expect_present,
                        spread_bindings,
                    );
                }
                if property.sym == *"keys" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.keys()` expects no arguments".into());
                    }
                    if let HirType::Array(_) = &receiver_type {
                        return self.lower_array_keys(receiver, receiver_type);
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
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(key_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                            vec![receiver],
                        )),
                    ));
                }
                if property.sym == *"values" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.values()` expects no arguments".into());
                    }
                    if let HirType::Array(_) = &receiver_type {
                        // The specification returns a fresh iterator, but
                        // the receiver is already a real `Array(_)` value
                        // -- already iterable, so this is just identity,
                        // the same simplification `Map`/`Set` methods here
                        // already make by eagerly snapshotting instead of
                        // returning a lazy iterator.
                        return Ok(receiver);
                    }
                    let (value_type, intrinsic) = match &receiver_type {
                        HirType::Map(_, value_type) => {
                            (value_type.as_ref().clone(), "__thaw_map_snapshot_values")
                        }
                        // A Set's elements ARE its "values" -- it has no
                        // separate value to snapshot.
                        HirType::Set(element_type) => {
                            (element_type.as_ref().clone(), "__thaw_map_snapshot_keys")
                        }
                        other => {
                            return Err(format!(
                                "native `.values()` requires a Map, Set, or Array receiver, got {other:?}"
                            ))
                        }
                    };
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(value_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![receiver],
                        )),
                    ));
                }
                if property.sym == *"entries" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !call.args.is_empty() {
                        return Err("native `.entries()` expects no arguments".into());
                    }
                    if let HirType::Array(element_type) = &receiver_type {
                        let element_type = element_type.as_ref().clone();
                        return self.lower_array_entries(receiver, receiver_type, element_type);
                    }
                    let (pair_type, intrinsic) = match &receiver_type {
                        HirType::Map(key_type, value_type) => (
                            HirType::Tuple(vec![key_type.as_ref().clone(), value_type.as_ref().clone()]),
                            "__thaw_map_snapshot_entries",
                        ),
                        HirType::Set(element_type) => (
                            HirType::Tuple(vec![element_type.as_ref().clone(), element_type.as_ref().clone()]),
                            "__thaw_set_snapshot_entries",
                        ),
                        other => {
                            return Err(format!(
                                "native `.entries()` requires a Map, Set, or Array receiver, got {other:?}"
                            ))
                        }
                    };
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(pair_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![receiver],
                        )),
                    ));
                }
        unreachable!("instance builtin category was checked before lowering")
    }
}

