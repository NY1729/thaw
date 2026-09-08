impl<'a> FnLowerer<'a> {
    /// `get`/`set`/`has`/`delete`/`add`/`clear` are common enough method
    /// names that a user's own class/object could plausibly define them
    /// too (unlike e.g. `charCodeAt`), so unlike every other native
    /// instance builtin, these can't be claimed by property name alone --
    /// `is_native_instance_builtin` doesn't see the receiver. This reads
    /// the receiver's type via already-established scope/interface data
    /// with no lowering (and so no risk of double-evaluating a
    /// side-effecting receiver expression like a function call), so a
    /// `Map`/`Set` variable, `this.field`, or a nested `a.b.c` member
    /// chain is recognized; anything else (for example a receiver that is
    /// itself a call, like `getMap().get(x)`) safely falls through to
    /// ordinary property/method-call handling instead.
    fn peek_type_without_lowering(&self, expr: &Expr) -> Option<HirType> {
        match expr {
            Expr::Ident(ident) => {
                let resolved = self.resolve_binding(ident.sym.as_ref());
                self.scope.get(&resolved).cloned()
            }
            Expr::This(_) => {
                let resolved = self.resolve_binding("this");
                self.scope.get(&resolved).cloned()
            }
            Expr::Member(member) => {
                let MemberProp::Ident(field) = &member.prop else {
                    return None;
                };
                let HirType::Object(fields) = self.peek_type_without_lowering(&member.obj)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find(|(name, _)| name == field.sym.as_ref())
                    .map(|(_, ty)| ty)
            }
            // `map.set(k, v).set(k2, v2)`/`set.add(a).add(b)`: `set`/`add`
            // return the receiver itself for chaining, so a call to either
            // one has the same type as ITS OWN receiver -- recurse without
            // looking at the call's arguments (irrelevant to the type, and
            // this must stay lowering-free).
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let Expr::Member(member) = callee.as_ref() else {
                    return None;
                };
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                if !matches!(property.sym.as_ref(), "set" | "add") {
                    return None;
                }
                let receiver_type = self.peek_type_without_lowering(&member.obj)?;
                matches!(receiver_type, HirType::Map(_, _) | HirType::Set(_))
                    .then_some(receiver_type)
            }
            _ => None,
        }
    }

    fn receiver_is_map_or_set(&self, expr: &Expr) -> bool {
        matches!(
            self.peek_type_without_lowering(expr),
            Some(HirType::Map(_, _) | HirType::Set(_))
        )
    }

    /// Prepares a `Map`/`Set` key/element value for the native intrinsic
    /// `map_key_intrinsic_suffix` selected: `number`/`string` keys coerce
    /// the way every other native method's arguments do, but a reference
    /// key (an object, array, function, ...) is never coerced -- only
    /// exact-type-matched, since coercing would silently change *which*
    /// object the key names.
    fn coerce_map_key(&mut self, key_type: &HirType, key: HirExpr) -> Result<HirExpr, String> {
        match key_type {
            HirType::F64 => self.coerce_primitive_to_number(key),
            HirType::Str => self.coerce_primitive_to_string(key),
            _ => {
                self.expect_type(key_type, &key, "Map/Set key")?;
                Ok(key)
            }
        }
    }

    /// ES2024 `Set.prototype.union`/`.intersection`/`.difference`. Builds a
    /// fresh `Set` the same way `structuredClone`'s `Map`/`Set` cloning
    /// does (`__thaw_map_new` plus the `.add()`/`.set()` intrinsic), fed by
    /// `__thaw_map_snapshot_keys` snapshots of the receiver and/or `other`
    /// rather than a live iterator -- consistent with every other Map/Set
    /// method here, none of which expose real iterator objects.
    fn lower_set_combine(
        &mut self,
        receiver: HirExpr,
        other: HirExpr,
        element_type: HirType,
        op: &str,
        extra_bindings: Vec<LoweredBinding>,
    ) -> Result<HirExpr, String> {
        let key_suffix = map_key_intrinsic_suffix(&element_type)?;
        let set_type = HirType::Set(Box::new(element_type.clone()));
        let elements_type = HirType::Array(Box::new(element_type.clone()));
        let has_intrinsic = format!("__thaw_map_{key_suffix}_has");
        let set_intrinsic = format!("__thaw_map_{key_suffix}_set");
        let snapshot_intrinsic = "__thaw_map_snapshot_keys".to_string();

        let receiver_name = format!("__thaw_set_combine_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let other_name = format!("__thaw_set_combine_other_{}", self.next_binding);
        self.next_binding += 1;
        let elements_name = format!("__thaw_set_combine_elements_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_set_combine_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_set_combine_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_set_combine_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_set_combine_element_{}", self.next_binding);
        self.next_binding += 1;

        self.scope.insert(receiver_name.clone(), set_type.clone());
        self.scope.insert(other_name.clone(), set_type.clone());
        self.scope
            .insert(elements_name.clone(), elements_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), set_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let var = |name: &str| HirExpr::Var(name.to_string());
        let snapshot = |source_name: &str| {
            HirExpr::TypedClosure(
                elements_type.clone(),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var(snapshot_intrinsic.clone())),
                    vec![var(source_name)],
                )),
            )
        };
        let insert_element = HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var(set_intrinsic)),
            vec![
                var(&result_name),
                var(&element_name),
                HirExpr::Lit(HirLit::F64(0.0)),
            ],
        ));
        let advance_index = HirStmt::Expr(HirExpr::Assign(
            index_name.clone(),
            Box::new(HirExpr::BinOp(
                BinOp::Add,
                Box::new(var(&index_name)),
                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
            )),
        ));
        let load_element = HirStmt::Let(
            element_name.clone(),
            element_type.clone(),
            HirExpr::TypedIndex(
                Box::new(var(&elements_name)),
                Box::new(var(&index_name)),
                element_type.clone(),
            ),
        );
        let loop_condition = || {
            HirExpr::BinOp(
                BinOp::Lt,
                Box::new(var(&index_name)),
                Box::new(var(&length_name)),
            )
        };

        let mut body_stmts = vec![
            HirStmt::Let(
                elements_name.clone(),
                elements_type.clone(),
                snapshot(&receiver_name),
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
        ];
        // `union` passes over both operands unconditionally. `symmetric-
        // Difference` passes over both too, but on each pass keeps an
        // element only when the *other* operand doesn't also have it (so a
        // value present in both is dropped from both passes).
        // `intersection`/`difference` need only the one pass over the
        // receiver, handled in the final `else` branch below.
        if op == "union" || op == "symmetricDifference" {
            let keep_if_other_lacks = op == "symmetricDifference";
            let guarded_insert = |other_name: &str| {
                if keep_if_other_lacks {
                    let membership = HirExpr::Call(
                        Box::new(HirExpr::Var(has_intrinsic.clone())),
                        vec![var(other_name), var(&element_name)],
                    );
                    vec![HirStmt::If(membership, Vec::new(), vec![insert_element.clone()])]
                } else {
                    vec![insert_element.clone()]
                }
            };
            let mut first_pass = vec![load_element.clone()];
            first_pass.extend(guarded_insert(&other_name));
            first_pass.push(advance_index.clone());
            body_stmts.push(HirStmt::While(loop_condition(), first_pass));

            body_stmts.push(HirStmt::Let(
                elements_name.clone(),
                elements_type.clone(),
                snapshot(&other_name),
            ));
            body_stmts.push(HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&elements_name))),
            ));
            body_stmts.push(HirStmt::Expr(HirExpr::Assign(
                index_name.clone(),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )));
            let mut second_pass = vec![load_element.clone()];
            second_pass.extend(guarded_insert(&receiver_name));
            second_pass.push(advance_index.clone());
            body_stmts.push(HirStmt::While(loop_condition(), second_pass));
        } else {
            // `intersection`: keep a receiver element only if `other` also
            // has it. `difference`: keep it only if `other` doesn't.
            let membership = HirExpr::Call(
                Box::new(HirExpr::Var(has_intrinsic)),
                vec![var(&other_name), var(&element_name)],
            );
            let (then_branch, else_branch) = if op == "intersection" {
                (vec![insert_element], Vec::new())
            } else {
                (Vec::new(), vec![insert_element])
            };
            body_stmts.push(HirStmt::While(
                loop_condition(),
                vec![
                    load_element,
                    HirStmt::If(membership, then_branch, else_branch),
                    advance_index,
                ],
            ));
        }
        body_stmts.push(HirStmt::Return(Some(var(&result_name))));
        let body = HirExpr::Block(body_stmts);

        let mut bindings = vec![(receiver_name, set_type.clone(), receiver)];
        bindings.extend(extra_bindings);
        bindings.push((other_name, set_type, other));
        self.wrap_call_argument_bindings(body, &bindings)
    }

    /// ES2024 `Set.prototype.isSubsetOf`/`.isSupersetOf`/`.isDisjointFrom`,
    /// sharing `lower_set_combine`'s snapshot/intrinsic shape but returning
    /// a boolean via a single short-circuiting scan instead of building a
    /// new `Set`. `scan_side`/`target_side` each name which operand
    /// (`"receiver"` or `"other"`) to iterate and to `.has()`-test against:
    /// `isSubsetOf` scans the receiver and tests the argument (every
    /// receiver element must be present in it); `isSupersetOf(other)` is
    /// exactly `other.isSubsetOf(receiver)`, so it swaps which side plays
    /// each role instead of duplicating the scan; `isDisjointFrom` scans
    /// the receiver too but asks the opposite question -- every element
    /// must be *absent* from the argument (`expect_present: false`).
    #[allow(clippy::too_many_arguments)]
    fn lower_set_predicate(
        &mut self,
        receiver: HirExpr,
        other: HirExpr,
        element_type: HirType,
        scan_side: &str,
        target_side: &str,
        expect_present: bool,
        extra_bindings: Vec<LoweredBinding>,
    ) -> Result<HirExpr, String> {
        let key_suffix = map_key_intrinsic_suffix(&element_type)?;
        let set_type = HirType::Set(Box::new(element_type.clone()));
        let elements_type = HirType::Array(Box::new(element_type.clone()));
        let has_intrinsic = format!("__thaw_map_{key_suffix}_has");

        let receiver_name = format!("__thaw_set_predicate_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let other_name = format!("__thaw_set_predicate_other_{}", self.next_binding);
        self.next_binding += 1;
        let elements_name = format!("__thaw_set_predicate_elements_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_set_predicate_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_set_predicate_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_set_predicate_element_{}", self.next_binding);
        self.next_binding += 1;

        self.scope.insert(receiver_name.clone(), set_type.clone());
        self.scope.insert(other_name.clone(), set_type.clone());
        self.scope
            .insert(elements_name.clone(), elements_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let var = |name: &str| HirExpr::Var(name.to_string());
        let scan_name = if scan_side == "receiver" {
            receiver_name.clone()
        } else {
            other_name.clone()
        };
        let target_name = if target_side == "receiver" {
            receiver_name.clone()
        } else {
            other_name.clone()
        };

        let membership = HirExpr::Call(
            Box::new(HirExpr::Var(has_intrinsic)),
            vec![var(&target_name), var(&element_name)],
        );
        let mismatch = if expect_present {
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(membership),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
            )
        } else {
            membership
        };

        let body = HirExpr::Block(vec![
            HirStmt::Let(
                elements_name.clone(),
                elements_type.clone(),
                HirExpr::TypedClosure(
                    elements_type,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                        vec![var(&scan_name)],
                    )),
                ),
            ),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&elements_name))),
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
                        element_name.clone(),
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&elements_name)),
                            Box::new(var(&index_name)),
                            element_type,
                        ),
                    ),
                    HirStmt::If(
                        mismatch,
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(false))))],
                        Vec::new(),
                    ),
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
            HirStmt::Return(Some(HirExpr::Lit(HirLit::Bool(true)))),
        ]);

        let mut bindings = vec![(receiver_name, set_type.clone(), receiver)];
        bindings.extend(extra_bindings);
        bindings.push((other_name, set_type, other));
        self.wrap_call_argument_bindings(body, &bindings)
    }

    /// `Map.groupBy(items, keyfn)`, built entirely from the same generic
    /// primitives `.get()`/`.set()`/`.push()` already lower to (an
    /// intrinsic call per operation, plus `__thaw_map_new` for an empty
    /// map and `ArrayAlloc`/`ArrayLen`/`TypedIndex` for the source array),
    /// the same "no new codegen" approach `lower_array_from_length` used
    /// for `Array.from({ length })`. `keyfn` receives `(item, index)`,
    /// matching the specification (unlike `Array`'s `map`/`forEach`/etc,
    /// this one has no third "receiver array" parameter to offer). Each
    /// bucket starts as a fresh empty array and is grown in place with
    /// `.push()`'s own handle-mutation, so repeated keys accumulate
    /// correctly without re-inserting into the map on every match.
    fn lower_map_group_by(
        &mut self,
        items: HirExpr,
        item_type: HirType,
        key_fn: HirExpr,
    ) -> Result<HirExpr, String> {
        let HirType::Function(_, key_type) = self.infer_expr_type(&key_fn)? else {
            unreachable!("Map.groupBy key function was validated as a function")
        };
        let key_type = key_type.as_ref().clone();
        let key_suffix = map_key_intrinsic_suffix(&key_type)?;
        let (value_suffix, needs_type_wrap) =
            map_value_get_suffix(&HirType::Array(Box::new(item_type.clone())))?;
        let items_name = format!("__thaw_group_by_items_{}", self.next_binding);
        self.next_binding += 1;
        let key_fn_name = format!("__thaw_group_by_key_fn_{}", self.next_binding);
        self.next_binding += 1;
        let items_type = HirType::Array(Box::new(item_type.clone()));
        let key_fn_type = HirType::Function(
            vec![item_type.clone(), HirType::F64],
            Box::new(key_type.clone()),
        );
        self.scope.insert(items_name.clone(), items_type.clone());
        self.scope.insert(key_fn_name.clone(), key_fn_type.clone());
        let length_name = format!("__thaw_group_by_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_group_by_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_group_by_index_{}", self.next_binding);
        self.next_binding += 1;
        let item_name = format!("__thaw_group_by_item_{}", self.next_binding);
        self.next_binding += 1;
        let key_name = format!("__thaw_group_by_key_{}", self.next_binding);
        self.next_binding += 1;
        let bucket_name = format!("__thaw_group_by_bucket_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Map(Box::new(key_type.clone()), Box::new(items_type.clone()));
        let bucket_type = items_type.clone();
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(item_name.clone(), item_type.clone());
        self.scope.insert(key_name.clone(), key_type.clone());
        self.scope.insert(bucket_name.clone(), bucket_type.clone());
        let var = |name: &str| HirExpr::Var(name.to_string());
        let has_intrinsic = format!("__thaw_map_{key_suffix}_has");
        let set_intrinsic = format!("__thaw_map_{key_suffix}_set");
        let get_intrinsic = format!("__thaw_map_{key_suffix}_get_{value_suffix}");
        let raw_get = HirExpr::Call(
            Box::new(HirExpr::Var(get_intrinsic)),
            vec![var(&result_name), var(&key_name)],
        );
        let bucket_value = if needs_type_wrap {
            HirExpr::TypedClosure(bucket_type.clone(), Box::new(raw_get))
        } else {
            raw_get
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&items_name))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_map_new".to_string())), Vec::new()),
            ),
            HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Let(
                        item_name.clone(),
                        item_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(var(&items_name)),
                            Box::new(var(&index_name)),
                            item_type.clone(),
                        ),
                    ),
                    HirStmt::Let(
                        key_name.clone(),
                        key_type.clone(),
                        HirExpr::Call(
                            Box::new(var(&key_fn_name)),
                            vec![var(&item_name), var(&index_name)],
                        ),
                    ),
                    HirStmt::If(
                        HirExpr::Call(
                            Box::new(HirExpr::Var(has_intrinsic)),
                            vec![var(&result_name), var(&key_name)],
                        ),
                        Vec::new(),
                        vec![HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var(set_intrinsic)),
                            vec![
                                var(&result_name),
                                var(&key_name),
                                HirExpr::ArrayAlloc(
                                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                    item_type.clone(),
                                ),
                            ],
                        ))],
                    ),
                    HirStmt::Let(bucket_name.clone(), bucket_type, bucket_value),
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".to_string())),
                        vec![var(&bucket_name), var(&item_name)],
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
        self.wrap_call_argument_bindings(
            body,
            &[
                (items_name, items_type, items),
                (key_fn_name, key_fn_type, key_fn),
            ],
        )
    }

}
