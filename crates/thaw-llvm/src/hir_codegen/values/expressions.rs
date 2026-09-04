impl<'ctx> HirCompiler<'ctx> {
    fn current_function(&self) -> FunctionValue<'ctx> {
        self.builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("builder must be positioned inside a function")
    }

    fn compile_expr(&mut self, expr: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(n)) => Ok(self.context.f64_type().const_float(*n).into()),
            HirExpr::Lit(HirLit::Bool(b)) => {
                Ok(self.context.bool_type().const_int(*b as u64, false).into())
            }
            HirExpr::Lit(HirLit::Undefined) => Ok(self.context.bool_type().const_zero().into()),
            HirExpr::Lit(HirLit::Null) => Ok(self.context.bool_type().const_int(1, false).into()),
            HirExpr::Lit(HirLit::Str(s)) => {
                let global = self
                    .builder
                    .build_global_string_ptr(s, "strlit")
                    .map_err(|e| e.to_string())?;
                Ok(global.as_pointer_value().into())
            }

            HirExpr::Var(name) => {
                let (ptr, ty) = *self
                    .variables
                    .get(name)
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                self.builder
                    .build_load(ty, ptr, name)
                    .map_err(|e| e.to_string())
            }

            HirExpr::Assign(name, value) => {
                let val = self.compile_expr(value)?;
                let (ptr, _ty) = *self
                    .variables
                    .get(name)
                    .ok_or_else(|| format!("assignment to undeclared variable `{name}`"))?;
                self.builder
                    .build_store(ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }

            HirExpr::OptionalSome(value, payload) => {
                self.compile_optional(value.as_ref(), payload, true)
            }
            HirExpr::OptionalNone(payload) => self.compile_optional_none(payload),
            HirExpr::OptionalIsNone(value, _) => {
                let optional = self.compile_expr(value)?.into_struct_value();
                let present = self
                    .builder
                    .build_extract_value(optional, 0, "optional_present")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.builder
                    .build_not(present, "optional_is_none")
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            HirExpr::OptionalValue(value, _) => {
                let optional = self.compile_expr(value)?.into_struct_value();
                self.builder
                    .build_extract_value(optional, 1, "optional_value")
                    .map_err(|error| error.to_string())
            }
            HirExpr::NullableSome(value, payload) => {
                self.compile_optional(value.as_ref(), payload, true)
            }
            HirExpr::NullableNone(payload) => self.compile_optional_none(payload),
            HirExpr::NullableIsNone(value, _) => {
                let nullable = self.compile_expr(value)?.into_struct_value();
                let present = self
                    .builder
                    .build_extract_value(nullable, 0, "nullable_present")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.builder
                    .build_not(present, "nullable_is_none")
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            HirExpr::NullableValue(value, _) => {
                let nullable = self.compile_expr(value)?.into_struct_value();
                self.builder
                    .build_extract_value(nullable, 1, "nullable_value")
                    .map_err(|error| error.to_string())
            }
            HirExpr::NullishSome(value, payload) => {
                let value = self.compile_expr(value)?;
                self.build_nullish_value(value, payload, 0)
            }
            HirExpr::NullishNull(payload) => self.compile_nullish_none(payload, 1),
            HirExpr::NullishUndefined(payload) => self.compile_nullish_none(payload, 2),
            HirExpr::NullishIsNull(value, _) => self.compile_nullish_tag_test(value, 1),
            HirExpr::NullishIsUndefined(value, _) => self.compile_nullish_tag_test(value, 2),
            HirExpr::NullishIsNone(value, _) => {
                let nullish = self.compile_expr(value)?.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(nullish, 0, "nullish_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.builder
                    .build_int_compare(
                        IntPredicate::NE,
                        tag,
                        self.context.i8_type().const_zero(),
                        "nullish_is_none",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            HirExpr::UnionInject(value, index, elements) => {
                let value = self.compile_expr(value)?;
                self.build_union_value(value, *index, elements)
            }
            HirExpr::UnionTag(value, _) => {
                let union = self.compile_expr(value)?.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(union, 0, "union_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.builder
                    .build_unsigned_int_to_float(tag, self.context.f64_type(), "union_tag_number")
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            HirExpr::UnionValue(value, index, elements) => {
                let member = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                let union = self.compile_expr(value)?.into_struct_value();
                let payload = self
                    .builder
                    .build_extract_value(union, 1, "union_payload")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.unpack_union_payload(payload, member)
            }
            HirExpr::UnionMemberIsEqual(union, member, index, elements) => {
                self.compile_union_member_equality(union, member, *index, elements)
            }
            HirExpr::UnionIsEqual(left, right, elements) => {
                self.compile_union_equality(left, right, elements)
            }
            HirExpr::NullishValue(value, _) => {
                let nullish = self.compile_expr(value)?.into_struct_value();
                self.builder
                    .build_extract_value(nullish, 1, "nullish_value")
                    .map_err(|error| error.to_string())
            }

            HirExpr::BinOp(op, lhs, rhs) => self.compile_binop(*op, lhs, rhs),

            HirExpr::Call(callee, args) => self.compile_call(callee, args),
            HirExpr::FunctionCallWithThis(callee, this_arg, args, params, ret) => {
                self.compile_function_call_with_this(callee, this_arg, args, params, ret)
            }
            HirExpr::FunctionBindThis(callee, this_arg, args, params, ret) => {
                self.compile_function_bind_this(callee, this_arg, args, params, ret)
            }
            HirExpr::PromiseAll(args, element) => self.compile_promise_all(args, element),
            HirExpr::PromiseAllArray(array, element) => {
                self.compile_promise_all_array(array, element)
            }
            HirExpr::PromiseAllTuple(args, elements) => {
                self.compile_promise_all_tuple(args, elements)
            }
            HirExpr::PromiseRace(args, _) => self.compile_promise_race(args),
            HirExpr::PromiseRaceArray(array, _) => self.compile_promise_race_array(array),
            HirExpr::PromiseAny(args, _) => self.compile_promise_any(args),
            HirExpr::PromiseAnyArray(array, _) => self.compile_promise_any_array(array),
            HirExpr::PromiseAllSettled(args, element) => {
                self.compile_promise_all_settled(args, element)
            }
            HirExpr::PromiseAllSettledArray(array, element) => {
                self.compile_promise_all_settled_array(array, element)
            }
            HirExpr::Lambda(captures, params, ret, body) => {
                self.compile_lambda(captures, params, ret, body)
            }
            HirExpr::RecursiveClosure(name, ty, closure) => {
                self.compile_recursive_closure(name, ty, closure)
            }
            HirExpr::TypedClosure(_, closure) => self.compile_expr(closure),
            HirExpr::FunctionRef(name, params, ret) => self.compile_function_ref(name, params, ret),
            HirExpr::MethodRef(unbound, explicit, params, ret, is_static) => {
                self.compile_method_ref(unbound, explicit, params, ret, *is_static)
            }
            HirExpr::FfiCall(sig, args) => self.compile_ffi_call(sig, args)?.ok_or_else(|| {
                format!(
                    "the void result of FFI function `{}` cannot be used as a value",
                    sig.symbol
                )
            }),
            HirExpr::DynamicCall(sig, args) => self.compile_typed_dynamic_call(sig, args),

            HirExpr::ArrayLit(elems) => self.compile_array_lit(elems),
            HirExpr::ArrayConcat(parts, element) => self.compile_array_concat(parts, element),
            HirExpr::ArrayAlloc(length, element) => self.compile_array_alloc(length, element),
            HirExpr::ArraySetLen(array, length, _) => {
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let length = self.compile_expr(length)?.into_float_value();
                let length = self
                    .builder
                    .build_float_to_signed_int(
                        length,
                        self.context.i64_type(),
                        "array_updated_length",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(buffer, length)
                    .map_err(|error| error.to_string())?;
                // The buffer shrinks in place; the handle (this
                // expression's own value, same reference as before) is
                // unchanged.
                Ok(handle.into())
            }
            HirExpr::Index(arr, idx) => {
                let elem_ptr = self.compile_element_ptr(arr, idx)?;
                self.builder
                    .build_load(self.context.f64_type(), elem_ptr, "elem")
                    .map_err(|e| e.to_string())
            }
            HirExpr::TypedIndex(arr, idx, element) => {
                let elem_ptr = self.compile_element_ptr(arr, idx)?;
                self.builder
                    .build_load(self.basic_type(element)?, elem_ptr, "typed_elem")
                    .map_err(|e| e.to_string())
            }
            HirExpr::IndexAssign(arr, idx, value) => {
                let elem_ptr = self.compile_element_ptr(arr, idx)?;
                let val = self.compile_expr(value)?;
                self.builder
                    .build_store(elem_ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }
            HirExpr::ArrayLen(arr) => {
                let handle = self.compile_expr(arr)?.into_pointer_value();
                let arr_ptr = self.compile_array_data(handle)?;
                let len = self
                    .builder
                    .build_load(self.context.i64_type(), arr_ptr, "arrlen_i64")
                    .map_err(|e| e.to_string())?
                    .into_int_value();
                self.builder
                    .build_signed_int_to_float(len, self.context.f64_type(), "arrlen")
                    .map(Into::into)
                    .map_err(|e| e.to_string())
            }

            HirExpr::EnvVar(name) => self.compile_env_var(name),

            HirExpr::JsonGet(obj, field) => self.compile_json_get(obj, field),
            HirExpr::JsonIndex(obj, idx) => self.compile_json_index(obj, idx),
            HirExpr::JsonKey(obj, key) => self.compile_json_key(obj, key),
            HirExpr::JsonSet(obj, key, value, element, preserve_undefined) => {
                self.compile_json_set(obj, key, value, element, *preserve_undefined)
            }
            HirExpr::JsonIndexSet(obj, index, value) => {
                self.compile_json_index_set(obj, index, value)
            }
            HirExpr::JsonDelete(obj, key) => self.compile_json_delete(obj, key),
            HirExpr::JsonAsNumber(inner) => self.compile_json_as(inner, "thaw_json_as_number"),
            HirExpr::JsonAsString(inner) => self.compile_json_as(inner, "thaw_json_as_string"),
            HirExpr::JsonAsBool(inner) => self.compile_json_as_bool(inner),
            HirExpr::JsonAsNative(inner, ty) => {
                let json = self.compile_expr(inner)?;
                self.compile_json_to_native(json, ty)
            }

            // Real suspension points are extracted by the async frame plan.
            // These arms compile only legacy/direct awaits that remain in an
            // ordinary expression path.
            HirExpr::Await(inner) => self.compile_await(inner),
            HirExpr::AwaitPromise(inner, resolved) => {
                self.compile_typed_blocking_await(inner, resolved)
            }
            HirExpr::PromiseNew(executor, resolved, assimilates) => {
                self.compile_promise_new(executor, resolved, *assimilates)
            }
            HirExpr::PromiseThen(source, callback, input, output, on_rejected, flatten) => {
                self.compile_promise_then(source, callback, input, output, *on_rejected, *flatten)
            }
            HirExpr::PromiseFinally(source, callback, input, callback_return) => {
                self.compile_promise_finally(source, callback, input, callback_return)
            }

            HirExpr::ObjectLit(fields) => self.compile_object_lit(fields),
            HirExpr::JsonObjectLit(fields, element) => {
                self.compile_json_object_lit(fields, element)
            }
            HirExpr::ObjectAlloc(object_type) => self.compile_object_alloc(object_type),
            HirExpr::PropAccess(obj, object_ty, field) => {
                let field_ty = self.field_type(object_ty, field)?;
                let llvm_ty = self.basic_type(&field_ty)?;
                let field_ptr = self.compile_field_ptr(obj, object_ty, field)?;
                self.builder
                    .build_load(llvm_ty, field_ptr, "field")
                    .map_err(|e| e.to_string())
            }
            HirExpr::DynamicPropAccess(obj, key, fields, result) => {
                self.compile_dynamic_prop_access(obj, key, fields, result)
            }
            HirExpr::EnumReverseLookup(index, entries) => {
                self.compile_enum_reverse_lookup(index, entries)
            }
            HirExpr::PropAssign(obj, object_ty, field, value) => {
                let field_ptr = self.compile_field_ptr(obj, object_ty, field)?;
                let val = self.compile_expr(value)?;
                self.builder
                    .build_store(field_ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }

            HirExpr::ThrowValue(error, fallback) => {
                let value = self.compile_expr(error)?;
                if let Some(completion) = self.active_async_completion {
                    self.builder
                        .build_call(
                            self.module.get_function("thaw_promise_reject").unwrap(),
                            &[completion.into(), value.into()],
                            "reject_throw_value",
                        )
                        .map_err(|error| error.to_string())?;
                    let function = self.current_function();
                    if function.get_type().get_return_type().is_some() {
                        self.builder
                            .build_return(Some(&completion))
                            .map_err(|error| error.to_string())?;
                    } else {
                        self.builder
                            .build_return(None)
                            .map_err(|error| error.to_string())?;
                    }
                    let unreachable = self
                        .context
                        .append_basic_block(function, "after_async_throw_value");
                    self.builder.position_at_end(unreachable);
                    return self.compile_expr(fallback);
                }
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), value)
                    .map_err(|error| error.to_string())?;
                if let Some(catch_block) = self.catch_stack.last().copied() {
                    self.builder
                        .build_unconditional_branch(catch_block)
                        .map_err(|error| error.to_string())?;
                } else {
                    self.build_default_return()?;
                }
                let function = self.current_function();
                let unreachable = self
                    .context
                    .append_basic_block(function, "after_throw_value");
                self.builder.position_at_end(unreachable);
                self.compile_expr(fallback)
            }

            other => Err(format!("Phase 1/2 codegen does not support {other:?} yet")),
        }
    }

    fn compile_dynamic_prop_access(
        &mut self,
        object: &HirExpr,
        key: &HirExpr,
        fields: &[(String, HirType)],
        result: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self.compile_expr(object)?.into_pointer_value();
        let key = self.compile_expr(key)?.into_pointer_value();
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or("dynamic property access is outside a function")?;
        if fields.is_empty() {
            return Err("dynamic property access requires at least one field".into());
        }
        let result_type = self.basic_type(result)?;
        let result_slot = self
            .builder
            .build_alloca(result_type, "dynamic_property_result")
            .map_err(|error| error.to_string())?;
        let none = match result {
            HirType::Optional(payload) => self.compile_optional_none(payload)?,
            HirType::Nullish(payload) => self.compile_nullish_none(payload, 2)?,
            HirType::Union(elements) => {
                let index = elements
                    .iter()
                    .position(|element| element == &HirType::Undefined)
                    .ok_or("dynamic property union result needs an undefined member")?;
                self.build_union_value(
                    self.context.bool_type().const_zero().into(),
                    index,
                    elements,
                )?
            }
            _ => {
                return Err(
                    "dynamic property access requires an optional, nullish, or union result".into(),
                )
            }
        };
        self.builder
            .build_store(result_slot, none)
            .map_err(|error| error.to_string())?;
        let merge = self
            .context
            .append_basic_block(function, "dynamic_property_merge");
        for (index, (name, source)) in fields.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "dynamic_property_match");
            let next = self
                .context
                .append_basic_block(function, "dynamic_property_next");
            let expected = self
                .compile_expr(&HirExpr::Lit(HirLit::Str(name.clone())))?
                .into_pointer_value();
            let comparison = self
                .builder
                .build_call(
                    self.module.get_function("strcmp").unwrap(),
                    &[key.into(), expected.into()],
                    "dynamic_property_compare",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("strcmp returned no value")?
                .into_int_value();
            let is_match = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    comparison,
                    comparison.get_type().const_zero(),
                    "dynamic_property_is_match",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_match, matched, next)
                .map_err(|error| error.to_string())?;

            self.builder.position_at_end(matched);
            let field_pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        object,
                        &[self
                            .context
                            .i64_type()
                            .const_int(object_field_offset(fields, index), false)],
                        "dynamic_property_field",
                    )
                    .map_err(|error| error.to_string())?
            };
            let value = self
                .builder
                .build_load(
                    self.basic_type(source)?,
                    field_pointer,
                    "dynamic_property_value",
                )
                .map_err(|error| error.to_string())?;
            let some = match (source, result) {
                (HirType::Optional(source_payload), HirType::Optional(result_payload))
                    if source_payload == result_payload =>
                {
                    value
                }
                (HirType::Nullish(source_payload), HirType::Nullish(result_payload))
                    if source_payload == result_payload =>
                {
                    value
                }
                (HirType::Nullable(source_payload), HirType::Nullish(result_payload))
                    if source_payload == result_payload =>
                {
                    let nullable = value.into_struct_value();
                    let present = self
                        .builder
                        .build_extract_value(nullable, 0, "dynamic_property_nullable_tag")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    let payload = self
                        .builder
                        .build_extract_value(nullable, 1, "dynamic_property_nullable_payload")
                        .map_err(|error| error.to_string())?;
                    let tag = self
                        .builder
                        .build_select(
                            present,
                            self.context.i8_type().const_zero(),
                            self.context.i8_type().const_int(1, false),
                            "dynamic_property_nullish_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    self.build_nullish_tagged_value(payload, source_payload, tag)?
                }
                (source, HirType::Optional(result_payload))
                    if source == result_payload.as_ref() =>
                {
                    self.build_optional_value(value, source, true)?
                }
                (
                    source @ (HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)),
                    HirType::Union(elements),
                ) => self.flatten_tagged_field_to_union(value, source, elements)?,
                (source, HirType::Union(elements)) => {
                    let index = elements
                        .iter()
                        .position(|element| element == source)
                        .ok_or("dynamic property field is missing from its union result")?;
                    self.build_union_value(value, index, elements)?
                }
                _ => {
                    return Err(
                        "dynamic property access has incompatible field and result types".into(),
                    )
                }
            };
            self.builder
                .build_store(result_slot, some)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(next);
        }
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        self.builder
            .build_load(result_type, result_slot, "dynamic_property_optional")
            .map_err(|error| error.to_string())
    }

    fn flatten_tagged_field_to_union(
        &mut self,
        value: BasicValueEnum<'ctx>,
        source: &HirType,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let tagged = value.into_struct_value();
        let payload_type = match source {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                payload.as_ref()
            }
            _ => return Err("dynamic property field is not tagged".into()),
        };
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "dynamic_property_tagged_payload")
            .map_err(|error| error.to_string())?;
        let payload_index = elements
            .iter()
            .position(|element| element == payload_type)
            .ok_or("dynamic property payload is missing from its union result")?;
        let payload = self.build_union_value(payload, payload_index, elements)?;

        let absence = |compiler: &mut Self, ty: HirType| {
            let index = elements
                .iter()
                .position(|element| element == &ty)
                .ok_or("dynamic property absence is missing from its union result")?;
            compiler.build_union_value(
                compiler.context.bool_type().const_zero().into(),
                index,
                elements,
            )
        };
        match source {
            HirType::Optional(_) | HirType::Nullable(_) => {
                let present = self
                    .builder
                    .build_extract_value(tagged, 0, "dynamic_property_present")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let absent_type = if matches!(source, HirType::Nullable(_)) {
                    HirType::Null
                } else {
                    HirType::Undefined
                };
                let absent = absence(self, absent_type)?;
                self.builder
                    .build_select(present, payload, absent, "dynamic_property_flattened")
                    .map_err(|error| error.to_string())
            }
            HirType::Nullish(_) => {
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "dynamic_property_nullish_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let null = absence(self, HirType::Null)?;
                let undefined = absence(self, HirType::Undefined)?;
                let is_null = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(1, false),
                        "dynamic_property_is_null",
                    )
                    .map_err(|error| error.to_string())?;
                let absent = self
                    .builder
                    .build_select(is_null, null, undefined, "dynamic_property_absence")
                    .map_err(|error| error.to_string())?;
                let is_present = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_zero(),
                        "dynamic_property_has_payload",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_select(
                        is_present,
                        payload,
                        absent,
                        "dynamic_property_flattened_nullish",
                    )
                    .map_err(|error| error.to_string())
            }
            _ => unreachable!(),
        }
    }

    fn compile_enum_reverse_lookup(
        &mut self,
        index: &HirExpr,
        entries: &[(f64, String)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let index = self.compile_expr(index)?.into_float_value();
        let function = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or("enum reverse lookup is outside a function")?;
        let result_type = self.basic_type(&HirType::Optional(Box::new(HirType::Str)))?;
        let result = self
            .builder
            .build_alloca(result_type, "enum_reverse_result")
            .map_err(|error| error.to_string())?;
        let none = self.compile_optional_none(&HirType::Str)?;
        self.builder
            .build_store(result, none)
            .map_err(|error| error.to_string())?;
        let merge = self
            .context
            .append_basic_block(function, "enum_reverse_merge");
        for (number, name) in entries {
            let matched = self
                .context
                .append_basic_block(function, "enum_reverse_match");
            let next = self
                .context
                .append_basic_block(function, "enum_reverse_next");
            let matches = self
                .builder
                .build_float_compare(
                    FloatPredicate::OEQ,
                    index,
                    self.context.f64_type().const_float(*number),
                    "enum_reverse_equals",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(matches, matched, next)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(matched);
            let name = self.compile_expr(&HirExpr::Lit(HirLit::Str(name.clone())))?;
            let some = self.build_optional_value(name, &HirType::Str, true)?;
            self.builder
                .build_store(result, some)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(next);
        }
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        self.builder
            .build_load(result_type, result, "enum_reverse_optional")
            .map_err(|error| error.to_string())
    }

}
