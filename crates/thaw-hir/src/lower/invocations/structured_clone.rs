impl<'a> FnLowerer<'a> {
    /// `structuredClone`'s dispatch on `value_type`, extracted so `Map`/`Set`
    /// cloning (below) can recurse into it for each entry's own value/
    /// element without duplicating the scalar/Array/Tuple/Object cases.
    fn structured_clone_expr(
        &mut self,
        value: HirExpr,
        value_type: HirType,
    ) -> Result<HirExpr, String> {
        match &value_type {
            // Scalars are already copied by value; no cloning needed.
            HirType::F64 | HirType::Str | HirType::Bool | HirType::Null | HirType::Undefined => {
                Ok(value)
            }
            HirType::Array(element) => self.lower_structured_clone_array(
                value,
                value_type.clone(),
                element.as_ref().clone(),
            ),
            HirType::Tuple(elements) => {
                let mut cloned = Vec::with_capacity(elements.len());
                for (index, element) in elements.iter().enumerate() {
                    cloned.push(self.structured_clone_expr(
                        HirExpr::TypedIndex(
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element.clone(),
                        ),
                        element.clone(),
                    )?);
                }
                Ok(HirExpr::ArrayLit(cloned))
            }
            HirType::Object(fields) => {
                let source_name = format!("__thaw_clone_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), value_type.clone());
                let source = HirExpr::Var(source_name.clone());
                let mut cloned = Vec::new();
                for index in ecmascript_field_order(fields) {
                    let name = &fields[index].0;
                    let read = self.lower_fixed_object_property_read(
                        source.clone(), fields, name,
                    )?;
                    let read_type = Self::fixed_object_property_read_type(fields, name)?;
                    let clone = self.structured_clone_expr(read, read_type)?;
                    let clone_type = self.infer_expr_type(&clone)?;
                    cloned.push((index, name.clone(), clone_type, clone));
                }
                let result_type = HirType::Object(cloned.iter()
                    .map(|(_, name, ty, _)| (name.clone(), ty.clone())).collect());
                let result_name = format!("__thaw_clone_result_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(result_name.clone(), result_type.clone());
                let result = HirExpr::Var(result_name.clone());
                let mut body = vec![HirStmt::Let(result_name, result_type.clone(),
                    HirExpr::ObjectAlloc(result_type.clone()))];
                body.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_order_begin".into())),
                    vec![result.clone()],
                )));
                for (_, name, _, _) in &cloned {
                    if name.starts_with("__thaw_class_identity_\u{1e}") {
                        body.push(HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_object_hide_marker".into())),
                            vec![result.clone(), HirExpr::Lit(HirLit::Str(name.clone()))],
                        )));
                    }
                }
                let actions = cloned.into_iter().map(|(index, name, _, clone)| {
                    let write = HirStmt::Expr(HirExpr::PropAssign(
                        Box::new(result.clone()), result_type.clone(), name.clone(),
                        Box::new(clone),
                    ));
                    let seed = HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_object_order_seed".into())),
                        vec![result.clone(), HirExpr::Lit(HirLit::Str(name))],
                    ));
                    (index, vec![write, seed])
                }).collect();
                body.extend(self.lower_ordered_fixed_field_statements(source, fields, actions)?);
                body.push(HirStmt::Return(Some(result)));
                self.wrap_call_argument_bindings(
                    HirExpr::Block(body),
                    &[(source_name, value_type, value)],
                )
            }
            // A dynamic (`any`/`Json`-typed) value -- `structuredClone`
            // on plain data objects/arrays typed loosely as `any` is the
            // common case in real code, not the fixed-shape native
            // `Object`/`Array` case above. `Value`'s derived `Clone` is
            // already a full recursive deep copy (`thaw_json_clone`).
            HirType::Json => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_clone".to_string())),
                vec![value],
            )),
            HirType::Map(key_type, val_type) => self.lower_structured_clone_map(
                value,
                key_type.as_ref().clone(),
                val_type.as_ref().clone(),
            ),
            HirType::Set(element_type) => {
                self.lower_structured_clone_set(value, element_type.as_ref().clone())
            }
            HirType::Union(members)
                if members
                    .iter()
                    .all(|member| matches!(member, HirType::Array(_))) =>
            {
                let source_name = format!("__thaw_clone_union_array_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(source_name.clone(), value_type.clone());
                let branches = members
                    .iter()
                    .enumerate()
                    .map(|(index, member)| {
                        let HirType::Array(element) = member else {
                            unreachable!()
                        };
                        self.lower_structured_clone_array(
                            HirExpr::UnionValue(
                                Box::new(HirExpr::Var(source_name.clone())),
                                index,
                                members.clone(),
                            ),
                            member.clone(),
                            element.as_ref().clone(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let result = self.merge_union_array_method_branches(
                    &source_name,
                    members,
                    branches,
                )?;
                self.wrap_call_argument_bindings(
                    result,
                    &[(source_name, value_type, value)],
                )
            }
            other => Err(format!("`structuredClone` does not support {other:?}")),
        }
    }

    fn lower_structured_clone_array(
        &mut self,
        value: HirExpr,
        array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        let element_name = format!("__thaw_clone_array_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let cloned = self.structured_clone_expr(
            HirExpr::Var(element_name.clone()),
            element_type.clone(),
        )?;
        let cloned_type = self.infer_expr_type(&cloned)?;
        self.lower_array_map(
            value,
            array_type,
            element_type.clone(),
            element_type.clone(),
            HirExpr::Lambda(
                Vec::new(),
                vec![HirParam {
                    name: element_name,
                    ty: element_type.clone(),
                }],
                cloned_type,
                Box::new(cloned),
            ),
            None,
        )
    }

    /// Builds a fresh `Map` and inserts a structurally-cloned copy of every
    /// entry, snapshotting the source via the same
    /// `__thaw_map_snapshot_entries` conversion `[...map]`/`Array.from(map)`
    /// already use. `K` is required to be `number`/`string` (unlike the
    /// specification, which clones object/array keys too) -- cloning a `Map`
    /// keyed by a fresh, differently-identitied object copy raises questions
    /// (would the clone even remain reachable by any key a caller could
    /// still produce?) this compiler's `Map`/`Set` model has no existing
    /// answer for, so it's left a compile-time error rather than guessed at.
    fn lower_structured_clone_map(
        &mut self,
        value: HirExpr,
        key_type: HirType,
        val_type: HirType,
    ) -> Result<HirExpr, String> {
        if map_key_intrinsic_suffix(&key_type)? == "ref" {
            return Err(
                "`structuredClone` of a Map keyed by an object/array/etc. is not supported yet"
                    .into(),
            );
        }
        let key_suffix = map_key_intrinsic_suffix(&key_type)?;
        let map_type = HirType::Map(Box::new(key_type.clone()), Box::new(val_type.clone()));
        let pair_type = HirType::Tuple(vec![key_type.clone(), val_type.clone()]);
        let entries_type = HirType::Array(Box::new(pair_type.clone()));

        let source_name = format!("__thaw_clone_map_source_{}", self.next_binding);
        self.next_binding += 1;
        let entries_name = format!("__thaw_clone_map_entries_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_clone_map_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_clone_map_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_clone_map_index_{}", self.next_binding);
        self.next_binding += 1;
        let entry_name = format!("__thaw_clone_map_entry_{}", self.next_binding);
        self.next_binding += 1;
        let key_name = format!("__thaw_clone_map_key_{}", self.next_binding);
        self.next_binding += 1;
        let raw_value_name = format!("__thaw_clone_map_raw_value_{}", self.next_binding);
        self.next_binding += 1;

        self.scope.insert(source_name.clone(), map_type.clone());
        self.scope.insert(entries_name.clone(), entries_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), map_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(entry_name.clone(), pair_type.clone());
        self.scope.insert(key_name.clone(), key_type.clone());
        self.scope.insert(raw_value_name.clone(), val_type.clone());

        let var = |name: &str| HirExpr::Var(name.to_string());
        let cloned_value = self.structured_clone_expr(var(&raw_value_name), val_type.clone())?;

        let body = HirExpr::Block(vec![
            HirStmt::Let(
                entries_name.clone(),
                entries_type.clone(),
                HirExpr::TypedClosure(
                    entries_type,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_entries".to_string())),
                        vec![var(&source_name)],
                    )),
                ),
            ),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&entries_name))),
            ),
            HirStmt::Let(
                result_name.clone(),
                map_type.clone(),
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                    Vec::new(),
                ),
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
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Let(
                        entry_name.clone(),
                        pair_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&entries_name)),
                            Box::new(var(&index_name)),
                            pair_type.clone(),
                        ),
                    ),
                    HirStmt::Let(
                        key_name.clone(),
                        key_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&entry_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                            key_type.clone(),
                        ),
                    ),
                    HirStmt::Let(
                        raw_value_name.clone(),
                        val_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&entry_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                            val_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![var(&result_name), var(&key_name), cloned_value],
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
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(body, &[(source_name, map_type, value)])
    }

    /// `Set`'s side of `lower_structured_clone_map`, sharing its snapshot/
    /// rebuild shape but with `.add()`'s own `__thaw_map_{suffix}_set(set,
    /// element, 0.0)` convention (a `Set` is a `Map` with a dummy value
    /// slot) in place of a real key/value pair.
    fn lower_structured_clone_set(
        &mut self,
        value: HirExpr,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        if map_key_intrinsic_suffix(&element_type)? == "ref" {
            return Err(
                "`structuredClone` of a Set of objects/arrays/etc. is not supported yet".into(),
            );
        }
        let key_suffix = map_key_intrinsic_suffix(&element_type)?;
        let set_type = HirType::Set(Box::new(element_type.clone()));
        let elements_type = HirType::Array(Box::new(element_type.clone()));

        let source_name = format!("__thaw_clone_set_source_{}", self.next_binding);
        self.next_binding += 1;
        let elements_name = format!("__thaw_clone_set_elements_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_clone_set_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_clone_set_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_clone_set_index_{}", self.next_binding);
        self.next_binding += 1;
        let raw_element_name = format!("__thaw_clone_set_raw_element_{}", self.next_binding);
        self.next_binding += 1;

        self.scope.insert(source_name.clone(), set_type.clone());
        self.scope
            .insert(elements_name.clone(), elements_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), set_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(raw_element_name.clone(), element_type.clone());

        let var = |name: &str| HirExpr::Var(name.to_string());
        let cloned_element =
            self.structured_clone_expr(var(&raw_element_name), element_type.clone())?;

        let body = HirExpr::Block(vec![
            HirStmt::Let(
                elements_name.clone(),
                elements_type.clone(),
                HirExpr::TypedClosure(
                    elements_type,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                        vec![var(&source_name)],
                    )),
                ),
            ),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&elements_name))),
            ),
            HirStmt::Let(
                result_name.clone(),
                set_type.clone(),
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_map_new".to_string())),
                    Vec::new(),
                ),
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
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Let(
                        raw_element_name.clone(),
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&elements_name)),
                            Box::new(var(&index_name)),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![
                            var(&result_name),
                            cloned_element,
                            HirExpr::Lit(HirLit::F64(0.0)),
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
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(body, &[(source_name, set_type, value)])
    }

}
