fn regex_object_type() -> HirType {
    HirType::Object(vec![
        ("source".to_string(), HirType::Str),
        ("flags".to_string(), HirType::Str),
        ("lastIndex".to_string(), HirType::F64),
    ])
}

/// `Date` is a fixed native object with a single millisecond-since-epoch
/// `timestamp` field (reusing the existing object machinery, like
/// `regex_object_type`, rather than a new value representation).
/// `thaw-runtime`'s calendar math is UTC-only -- there is no host timezone
/// database, so "local" `Date` methods alias their UTC counterparts.
fn date_object_type() -> HirType {
    HirType::Object(vec![("timestamp".to_string(), HirType::F64)])
}

/// `Map`/`Set`'s key/element type selects which family of native
/// `__thaw_map_*` intrinsics to call -- `"num"` (`SameValueZero`-equal
/// numbers), `"str"` (content-hashed strings), or `"ref"` (hashed and
/// compared by its own pointer value -- reference identity, exactly like
/// JavaScript's `SameValueZero` degenerates to `===` for non-primitive
/// keys). `Optional`/`Nullable`/`Nullish`/`Union` don't fit `"ref"`:
/// they're inline tagged structs, not a single pointer, so there's no
/// stable identity word to hash. `Function`/`CallableFunction` don't
/// either, for a different reason: referencing the same top-level named
/// function as a value builds a fresh closure-ABI wrapper each time
/// (confirmed empirically -- `f === f` is observably `false` here), so
/// there is no stable identity to key by even though the value is
/// pointer-shaped. Any other key type is rejected at `Map<K, V>`/
/// `Set<T>` resolution time (`type_resolution.rs`) and at
/// `new Map<K, V>()`/`new Set<T>()` construction time, so this should
/// never actually fail once a `Map`/`Set` value exists, but a lowering
/// bug elsewhere producing one anyway shouldn't panic.
fn map_key_intrinsic_suffix(key_type: &HirType) -> Result<&'static str, String> {
    match key_type {
        HirType::F64 => Ok("num"),
        HirType::Str => Ok("str"),
        // `Map<any, V>`/`Set<any>` -- a `Json` key can be a real
        // primitive at runtime, unlike every other "ref" case below, so
        // it gets its own key kind (`AnyKey`, thaw-runtime's `maps.rs`)
        // hashing/comparing by value for one, by pointer identity for an
        // array/object -- exactly like real JavaScript's `SameValueZero`
        // degenerates to `===` reference equality there. This also
        // correctly makes `WeakMap<any, _>`/`WeakSet<any>` a compile
        // error (`weak_key_intrinsic_suffix` below only accepts `"ref"`)
        // instead of silently compiling with the same by-reference bug a
        // primitive key would have had under plain `"ref"` treatment --
        // real `WeakMap`/`WeakSet` require a genuine object key, and a
        // statically `any`-typed one can't promise that.
        HirType::Json => Ok("any"),
        HirType::Array(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Dictionary(_)
        | HirType::Promise(_)
        | HirType::Map(_, _)
        | HirType::WeakMap(_, _)
        | HirType::Set(_)
        | HirType::WeakSet(_) => Ok("ref"),
        other => Err(format!(
            "Map/Set keys must be `number`, `string`, or a reference type \
             (object, array, ...), got {other:?}"
        )),
    }
}

/// `WeakMap`/`WeakSet` reuse `map_key_intrinsic_suffix`'s own type
/// classification. `"ref"` is a statically known reference type,
/// trivially valid. `"any"` (a `Json`-typed key -- covers both a real
/// `any`/`unknown` annotation and TS's own `object` keyword, which
/// `type_resolution.rs` also lowers to `Json`) is accepted too, but
/// unlike a plain `Map`/`Set` (where a primitive key is perfectly
/// legal), a `WeakMap`/`WeakSet` key that turns out to be a primitive
/// at runtime is a genuine spec violation -- real `WeakMap.prototype.
/// set`/`WeakSet.prototype.add` throw a `TypeError` for one, while
/// `.has`/`.delete` just report `false` (confirmed against real Node).
/// `.set`/`.add`'s own lowering (`map_set_methods.rs`) adds the runtime
/// guard this promise requires; every other reference type keeps
/// today's `"ref"` classification unchanged, and only `HirType::Str`/
/// `HirType::F64` (a *statically* known primitive) stays a hard
/// compile-time rejection, since those can never be valid regardless
/// of any runtime check.
fn weak_key_intrinsic_suffix(key_type: &HirType) -> Result<&'static str, String> {
    match map_key_intrinsic_suffix(key_type) {
        Ok(suffix @ ("ref" | "any")) => Ok(suffix),
        _ => Err(format!(
            "WeakMap/WeakSet keys must be a reference type (object, array, ...), got {key_type:?}"
        )),
    }
}

impl<'a> FnLowerer<'a> {
    fn lower_union_array_sequence(
        &mut self,
        receiver: HirExpr,
        members: &[HirType],
    ) -> Result<(HirExpr, HirType), String> {
        let mut element_members = Vec::new();
        for member in members {
            let HirType::Array(element) = member else {
                return Err(format!("sequence receiver union member {member:?} is not an array"));
            };
            Self::flatten_property_union_members(element, &mut element_members)?;
        }
        let element_type = match element_members.as_slice() {
            [] => return Err("sequence receiver cannot be an empty union".into()),
            [element] => element.clone(),
            [left, right] if left == &HirType::Undefined => {
                HirType::Optional(Box::new(right.clone()))
            }
            [left, right] if right == &HirType::Undefined => {
                HirType::Optional(Box::new(left.clone()))
            }
            _ => HirType::Union(element_members),
        };
        let array_type = HirType::Array(Box::new(element_type.clone()));
        let receiver_type = HirType::Union(members.to_vec());
        let receiver_name = format!("__thaw_union_sequence_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), receiver_type.clone());

        let mut branches = Vec::with_capacity(members.len());
        for (index, member) in members.iter().enumerate() {
            let HirType::Array(source_element) = member else { unreachable!() };
            let parameter = format!("__thaw_union_sequence_element_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(parameter.clone(), source_element.as_ref().clone());
            let converted = self.coerce_to_declared(
                &element_type,
                HirExpr::Var(parameter.clone()),
            )?;
            let callback = HirExpr::Lambda(
                Vec::new(),
                vec![HirParam {
                    name: parameter,
                    ty: source_element.as_ref().clone(),
                }],
                element_type.clone(),
                Box::new(converted),
            );
            branches.push(self.lower_array_map(
                HirExpr::UnionValue(
                    Box::new(HirExpr::Var(receiver_name.clone())),
                    index,
                    members.to_vec(),
                ),
                member.clone(),
                source_element.as_ref().clone(),
                source_element.as_ref().clone(),
                callback,
                None,
            )?);
        }
        let result = self.merge_union_array_method_branches(&receiver_name, members, branches)?;
        Ok((
            self.wrap_call_argument_bindings(
                result,
                &[(receiver_name, receiver_type, receiver)],
            )?,
            array_type,
        ))
    }

    fn merge_union_array_method_branches(
        &mut self,
        receiver_name: &str,
        members: &[HirType],
        branches: Vec<HirExpr>,
    ) -> Result<HirExpr, String> {
        let mut branch_types = Vec::with_capacity(branches.len());
        for branch in &branches {
            branch_types.push(self.infer_expr_type(branch)?);
        }
        let Some(first) = branch_types.first() else {
            return Err("cannot call an array method on an empty union".into());
        };
        let result_type = if branch_types.iter().all(|branch| branch == first) {
            first.clone()
        } else {
            let mut result_members = Vec::new();
            for branch in &branch_types {
                Self::flatten_property_union_members(branch, &mut result_members)?;
            }
            match result_members.as_slice() {
                [single] => single.clone(),
                _ => HirType::Union(result_members),
            }
        };
        let receiver = HirExpr::Var(receiver_name.into());
        let mut result = None;
        for (index, (branch, branch_type)) in branches
            .into_iter()
            .zip(branch_types)
            .enumerate()
            .rev()
        {
            let branch = if branch_type == result_type {
                branch
            } else {
                self.coerce_to_declared(&result_type, branch)?
            };
            result = Some(match result {
                None => branch,
                Some(rest) => HirExpr::Conditional(
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(receiver.clone()), members.to_vec())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    )),
                    Box::new(branch),
                    Box::new(rest),
                    result_type.clone(),
                ),
            });
        }
        result.ok_or_else(|| "cannot call an array method on an empty union".into())
    }
}

/// Chooses which `__thaw_map_{num,str}_get_*` variant decodes a `Map`
/// value of `value_type` correctly, and whether the raw call result needs
/// wrapping in `HirExpr::TypedClosure` to recover a pointer-shaped type
/// the intrinsic name alone can't carry (unlike `F64`/`Bool`, where the
/// result type is always the same regardless of context).
fn map_value_get_suffix(value_type: &HirType) -> Result<(&'static str, bool), String> {
    match value_type {
        HirType::F64 => Ok(("f64", false)),
        HirType::Bool | HirType::Undefined | HirType::Null => Ok(("bool", false)),
        // Both already fit the same opaque 64-bit storage word as `F64`/
        // `Bool` (see `functions.rs`'s LLVM-type mapping: `I64`/`JsValue`
        // are both a plain `i64`, no boxing/pointer indirection), so no
        // `TypedClosure` wrap is needed either -- same as `F64`/`Bool`.
        HirType::I64 | HirType::JsValue => Ok(("i64", false)),
        _ => Ok(("ptr", true)),
    }
}
