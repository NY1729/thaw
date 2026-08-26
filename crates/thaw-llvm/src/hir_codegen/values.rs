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
                let array = self.compile_expr(array)?.into_pointer_value();
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
                    .build_store(array, length)
                    .map_err(|error| error.to_string())?;
                Ok(array.into())
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
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
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
            HirExpr::JsonSet(obj, key, value, element) => {
                self.compile_json_set(obj, key, value, element)
            }
            HirExpr::JsonAsNumber(inner) => self.compile_json_as(inner, "thaw_json_as_number"),
            HirExpr::JsonAsString(inner) => self.compile_json_as(inner, "thaw_json_as_string"),
            HirExpr::JsonAsBool(inner) => self.compile_json_as_bool(inner),

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

    fn compile_optional(
        &mut self,
        value: &HirExpr,
        payload: &HirType,
        present: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_expr(value)?;
        self.build_optional_value(value, payload, present)
    }

    fn build_union_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        index: usize,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let member = elements
            .get(index)
            .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
        let payload = match (member, value) {
            (HirType::F64, BasicValueEnum::FloatValue(value)) => self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "union_float_bits")
                .map_err(|error| error.to_string())?
                .into_int_value(),
            (
                HirType::Bool | HirType::Undefined | HirType::Null,
                BasicValueEnum::IntValue(value),
            ) => self
                .builder
                .build_int_z_extend(value, self.context.i64_type(), "union_bool_bits")
                .map_err(|error| error.to_string())?,
            (HirType::I64 | HirType::JsValue, BasicValueEnum::IntValue(value)) => value,
            (_, BasicValueEnum::PointerValue(value)) => self
                .builder
                .build_ptr_to_int(value, self.context.i64_type(), "union_pointer_bits")
                .map_err(|error| error.to_string())?,
            _ => return Err(format!("cannot pack {member:?} into a union payload")),
        };
        let union_type = self
            .basic_type(&HirType::Union(elements.to_vec()))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(
                union_type.get_undef(),
                self.context.i8_type().const_int(index as u64, false),
                0,
                "union_with_tag",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, payload, 1, "union_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn unpack_union_payload(
        &mut self,
        payload: IntValue<'ctx>,
        member: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match member {
            HirType::F64 => self
                .builder
                .build_bit_cast(payload, self.context.f64_type(), "union_float")
                .map_err(|error| error.to_string()),
            HirType::Bool | HirType::Undefined | HirType::Null => self
                .builder
                .build_int_truncate(payload, self.context.bool_type(), "union_bool")
                .map(Into::into)
                .map_err(|error| error.to_string()),
            HirType::I64 | HirType::JsValue => Ok(payload.into()),
            _ => self
                .builder
                .build_int_to_ptr(
                    payload,
                    self.context.ptr_type(AddressSpace::default()),
                    "union_pointer",
                )
                .map(Into::into)
                .map_err(|error| error.to_string()),
        }
    }

    fn compile_union_member_equality(
        &mut self,
        union: &HirExpr,
        member: &HirExpr,
        index: usize,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let member_type = elements
            .get(index)
            .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
        let union = self.compile_expr(union)?.into_struct_value();
        let member = self.compile_expr(member)?;
        let tag = self
            .builder
            .build_extract_value(union, 0, "union_equality_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let tag_matches = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(index as u64, false),
                "union_equality_tag_matches",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let matched = self
            .context
            .append_basic_block(function, "union_equality_matched");
        let mismatched = self
            .context
            .append_basic_block(function, "union_equality_mismatched");
        let merge = self
            .context
            .append_basic_block(function, "union_equality_merge");
        let result = self
            .builder
            .build_alloca(self.context.bool_type(), "union_equality_result")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(tag_matches, matched, mismatched)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(mismatched);
        self.builder
            .build_store(result, self.context.bool_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(matched);
        let payload = self
            .builder
            .build_extract_value(union, 1, "union_equality_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self.unpack_union_payload(payload, member_type)?;
        let payload_matches = self.compile_native_strict_equality(payload, member, member_type)?;
        self.builder
            .build_store(result, payload_matches)
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge);
        self.builder
            .build_load(self.context.bool_type(), result, "union_member_equal")
            .map_err(|error| error.to_string())
    }

    fn compile_native_strict_equality(
        &mut self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<IntValue<'ctx>, String> {
        match (left, right) {
            (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) => self
                .builder
                .build_float_compare(FloatPredicate::OEQ, left, right, "union_float_equal")
                .map_err(|error| error.to_string()),
            (BasicValueEnum::IntValue(left), BasicValueEnum::IntValue(right)) => self
                .builder
                .build_int_compare(IntPredicate::EQ, left, right, "union_int_equal")
                .map_err(|error| error.to_string()),
            (BasicValueEnum::PointerValue(left), BasicValueEnum::PointerValue(right)) => {
                if ty == &HirType::Str {
                    let compared = self
                        .builder
                        .build_call(
                            self.module.get_function("strcmp").unwrap(),
                            &[left.into(), right.into()],
                            "union_string_compare",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or("strcmp returned no union string comparison")?
                        .into_int_value();
                    self.builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            compared,
                            self.context.i32_type().const_zero(),
                            "union_string_equal",
                        )
                        .map_err(|error| error.to_string())
                } else {
                    self.builder
                        .build_int_compare(IntPredicate::EQ, left, right, "union_pointer_equal")
                        .map_err(|error| error.to_string())
                }
            }
            _ => Err(format!("cannot compare union member with type {ty:?}")),
        }
    }

    fn compile_union_equality(
        &mut self,
        left: &HirExpr,
        right: &HirExpr,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if elements.is_empty() {
            return Err("cannot compare empty unions".into());
        }
        let left = self.compile_expr(left)?.into_struct_value();
        let right = self.compile_expr(right)?.into_struct_value();
        let left_tag = self
            .builder
            .build_extract_value(left, 0, "union_left_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_tag = self
            .builder
            .build_extract_value(right, 0, "union_right_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let tags_equal = self
            .builder
            .build_int_compare(IntPredicate::EQ, left_tag, right_tag, "union_tags_equal")
            .map_err(|error| error.to_string())?;
        let left_payload = self
            .builder
            .build_extract_value(left, 1, "union_left_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_payload = self
            .builder
            .build_extract_value(right, 1, "union_right_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let dispatch = self
            .context
            .append_basic_block(function, "union_equal_dispatch");
        let mismatch = self
            .context
            .append_basic_block(function, "union_tags_differ");
        let merge = self
            .context
            .append_basic_block(function, "union_equal_merge");
        let result = self
            .builder
            .build_alloca(self.context.bool_type(), "union_equal_result")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(tags_equal, dispatch, mismatch)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(mismatch);
        self.builder
            .build_store(result, self.context.bool_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(dispatch);
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "union_equal_member");
            let next = (index + 1 != elements.len()).then(|| {
                self.context
                    .append_basic_block(function, "union_equal_next")
            });
            if let Some(next) = next {
                let is_member = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        left_tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "union_equal_member_tag",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_member, matched, next)
                    .map_err(|error| error.to_string())?;
            } else {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let left = self.unpack_union_payload(left_payload, member)?;
            let right = self.unpack_union_payload(right_payload, member)?;
            let equal = self.compile_native_strict_equality(left, right, member)?;
            self.builder
                .build_store(result, equal)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if let Some(next) = next {
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        self.builder
            .build_load(self.context.bool_type(), result, "union_equal")
            .map_err(|error| error.to_string())
    }

    fn compile_optional_none(&mut self, payload: &HirType) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_zero_value(payload)?;
        self.build_optional_value(value, payload, false)
    }

    fn compile_zero_value(&mut self, ty: &HirType) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
                ) =>
            {
                let i64_type = self.context.i64_type();
                let allocation = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_arena_alloc").unwrap(),
                        &[
                            i64_type.const_int(ARRAY_HEADER_BYTES, false).into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "zero_array_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_arena_alloc returned no zero-array pointer")?
                    .into_pointer_value();
                self.builder
                    .build_store(allocation, i64_type.const_zero())
                    .map_err(|error| error.to_string())?;
                Ok(allocation.into())
            }
            HirType::Object(fields) => {
                let i64_type = self.context.i64_type();
                let allocation = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_arena_alloc").unwrap(),
                        &[
                            i64_type
                                .const_int(object_storage_bytes(fields).max(1), false)
                                .into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "zero_object_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_arena_alloc returned no zero-object pointer")?
                    .into_pointer_value();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let field = self.compile_zero_value(field_type)?;
                    let pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                allocation,
                                &[i64_type.const_int(object_field_offset(fields, index), false)],
                                "zero_object_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    self.builder
                        .build_store(pointer, field)
                        .map_err(|error| error.to_string())?;
                }
                Ok(allocation.into())
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                let value = self.compile_zero_value(payload)?;
                self.build_optional_value(value, payload, false)
            }
            HirType::Nullish(payload) => {
                let value = self.compile_zero_value(payload)?;
                self.build_nullish_value(value, payload, 2)
            }
            other => Ok(self.basic_type(other)?.const_zero()),
        }
    }

    fn build_optional_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        present: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let optional_type = self
            .basic_type(&HirType::Optional(Box::new(payload.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(
                optional_type.get_undef(),
                self.context
                    .bool_type()
                    .const_int(u64::from(present), false),
                0,
                "optional_with_tag",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, value, 1, "optional_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn compile_nullish_none(
        &mut self,
        payload: &HirType,
        tag: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_zero_value(payload)?;
        self.build_nullish_value(value, payload, tag)
    }

    fn build_nullish_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        tag: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.build_nullish_tagged_value(
            value,
            payload,
            self.context.i8_type().const_int(tag, false),
        )
    }

    fn build_nullish_tagged_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        tag: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let nullish_type = self
            .basic_type(&HirType::Nullish(Box::new(payload.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(nullish_type.get_undef(), tag, 0, "nullish_with_tag")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, value, 1, "nullish_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn compile_nullish_tag_test(
        &mut self,
        value: &HirExpr,
        expected: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let nullish = self.compile_expr(value)?.into_struct_value();
        let tag = self
            .builder
            .build_extract_value(nullish, 0, "nullish_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(expected, false),
                "nullish_tag_matches",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_recursive_closure(
        &mut self,
        name: &str,
        ty: &HirType,
        closure: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let llvm_ty = self.basic_type(ty)?;
        let cell = self.allocate_variable_cell(llvm_ty, name)?;
        self.variables.insert(name.to_string(), (cell, llvm_ty));
        self.variable_hir_types.insert(name.to_string(), ty.clone());
        let value = self.compile_expr(closure)?;
        self.builder
            .build_store(cell, value)
            .map_err(|error| error.to_string())?;
        Ok(value)
    }

    fn allocate_lambda_environment(
        &mut self,
        function: FunctionValue<'ctx>,
        this_adapter: FunctionValue<'ctx>,
        captures: &[HirParam],
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type
                        .const_int(CLOSURE_CAPTURE_BASE + captures.len() as u64 * 8, false)
                        .into(),
                    i64_type.const_int(8, false).into(),
                ],
                "closure_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a closure")?
            .into_pointer_value();
        self.builder
            .build_store(closure, function.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "closure_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(
                this_entry,
                this_adapter.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        for (index, capture) in captures.iter().enumerate() {
            let (variable_cell, _) = self
                .variables
                .get(&capture.name)
                .copied()
                .ok_or_else(|| format!("missing captured variable `{}`", capture.name))?;
            let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + index as u64 * 8, false);
            let slot = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), closure, &[offset], "capture_slot")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(slot, variable_cell)
                .map_err(|error| error.to_string())?;
        }
        Ok(closure)
    }

    fn compile_async_lambda(
        &mut self,
        captures: &[HirParam],
        params: &[HirParam],
        resolved: &HirType,
        body: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parent_block = self
            .builder
            .get_insert_block()
            .ok_or("async lambda must be emitted inside a function")?;
        let name = format!("__thaw_async_lambda_{}", self.next_lambda);
        self.next_lambda += 1;
        let mut lifted_params = captures.to_vec();
        lifted_params.extend_from_slice(params);
        let lifted = HirFunction {
            name: name.clone(),
            params: lifted_params,
            ret: resolved.clone(),
            is_async: true,
            body: match body {
                HirExpr::Block(statements) => statements.clone(),
                expression => vec![HirStmt::Return(Some(expression.clone()))],
            },
        };
        self.frame_async_functions
            .insert(name.clone(), resolved.clone());
        self.declare_function(&lifted)
            .map_err(|error| format!("async lambda `{name}`: {error}"))?;

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let compiled = self.compile_function_body(&lifted);
        self.variables = saved_variables;
        self.variable_hir_types = saved_variable_hir_types;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.builder.position_at_end(parent_block);
        compiled.map_err(|error| format!("async lambda `{name}`: {error}"))?;

        let promise_type = HirType::Promise(Box::new(resolved.clone()));
        let param_types = params
            .iter()
            .map(|parameter| parameter.ty.clone())
            .collect::<Vec<_>>();
        let adapter_name = format!("{name}__closure");
        let adapter = self.module.add_function(
            &adapter_name,
            self.function_type(&param_types, &promise_type)?,
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let environment = adapter.get_first_param().unwrap().into_pointer_value();
        let i64_type = self.context.i64_type();
        let mut arguments = Vec::with_capacity(captures.len() + params.len());
        for (index, capture) in captures.iter().enumerate() {
            let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + index as u64 * 8, false);
            let capture_slot = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        environment,
                        &[offset],
                        "async_capture",
                    )
                    .map_err(|error| error.to_string())?
            };
            let cell = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    capture_slot,
                    "async_capture_cell",
                )
                .map_err(|error| error.to_string())?
                .into_pointer_value();
            arguments.push(
                self.builder
                    .build_load(self.basic_type(&capture.ty)?, cell, "async_capture_value")
                    .map_err(|error| error.to_string())?
                    .into(),
            );
        }
        arguments.extend(
            adapter
                .get_param_iter()
                .skip(1)
                .map(BasicMetadataValueEnum::from),
        );
        let target = self.module.get_function(&name).unwrap();
        let promise = self
            .builder
            .build_call(target, &arguments, "invoke_async_lambda")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("async lambda did not return a promise")?;
        self.builder
            .build_return(Some(&promise))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(parent_block);
        let this_adapter = self.compile_ignored_this_adapter(
            adapter,
            &param_types,
            &promise_type,
            &format!("{adapter_name}__thaw_this_adapter"),
        )?;
        Ok(self
            .allocate_lambda_environment(adapter, this_adapter, captures)?
            .into())
    }

    fn compile_lambda(
        &mut self,
        captures: &[HirParam],
        params: &[HirParam],
        ret: &HirType,
        body: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let HirType::Promise(resolved) = ret {
            let frame_functions = self
                .frame_async_functions
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let suspends = match body {
                HirExpr::Block(statements) => statements
                    .iter()
                    .any(|statement| Self::stmt_awaits_frame_source(statement, &frame_functions)),
                expression => Self::expr_awaits_frame_source(expression, &frame_functions),
            };
            if suspends {
                return self.compile_async_lambda(captures, params, resolved, body);
            }
        }
        let parent_block = self
            .builder
            .get_insert_block()
            .ok_or("lambda must be emitted inside a function")?;
        let param_types = params
            .iter()
            .map(|param| param.ty.clone())
            .collect::<Vec<_>>();
        let function_type = self.function_type(&param_types, ret)?;
        let name = format!("__thaw_lambda_{}", self.next_lambda);
        self.next_lambda += 1;
        let function = self
            .module
            .add_function(&name, function_type, Some(Linkage::Internal));
        let this_adapter = self.compile_ignored_this_adapter(
            function,
            &param_types,
            ret,
            &format!("{name}__thaw_this_adapter"),
        )?;

        let i64_type = self.context.i64_type();
        // Closure captures retain their variable cells so mutations remain
        // visible when the function value is invoked later.
        let closure = self.allocate_lambda_environment(function, this_adapter, captures)?;

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let result = (|| -> Result<(), String> {
            let entry = self.context.append_basic_block(function, "entry");
            self.builder.position_at_end(entry);
            let environment = function
                .get_first_param()
                .ok_or("closure function is missing its environment")?
                .into_pointer_value();
            for (index, capture) in captures.iter().enumerate() {
                let ty = self.basic_type(&capture.ty)?;
                let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + (index as u64 * 8), false);
                let capture_slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            environment,
                            &[offset],
                            "captured",
                        )
                        .map_err(|error| error.to_string())?
                };
                let variable_cell = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        capture_slot,
                        "capture_cell",
                    )
                    .map_err(|error| error.to_string())?;
                self.variables.insert(
                    capture.name.clone(),
                    (variable_cell.into_pointer_value(), ty),
                );
                self.variable_hir_types
                    .insert(capture.name.clone(), capture.ty.clone());
            }
            for (value, param) in function.get_param_iter().skip(1).zip(params) {
                let ty = self.basic_type(&param.ty)?;
                let slot = self.allocate_variable_cell(ty, &param.name)?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|error| error.to_string())?;
                self.variables.insert(param.name.clone(), (slot, ty));
                self.variable_hir_types
                    .insert(param.name.clone(), param.ty.clone());
            }

            match body {
                HirExpr::Block(stmts) => {
                    let terminated = self.compile_block(stmts)?;
                    if !terminated {
                        if *ret == HirType::Void {
                            self.builder
                                .build_return(None)
                                .map_err(|error| error.to_string())?;
                        } else {
                            return Err(format!(
                                "lambda `{name}` does not return a value on all paths"
                            ));
                        }
                    }
                }
                expr => {
                    if *ret == HirType::Void {
                        self.compile_expr(expr)?;
                        self.builder
                            .build_return(None)
                            .map_err(|error| error.to_string())?;
                    } else {
                        let value = self.compile_expr(expr)?;
                        self.builder
                            .build_return(Some(&value))
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
            Ok(())
        })();
        self.variables = saved_variables;
        self.variable_hir_types = saved_variable_hir_types;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.builder.position_at_end(parent_block);
        result?;
        Ok(closure.into())
    }

    fn compile_function_ref(
        &mut self,
        name: &str,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let target = self
            .module
            .get_function(&Self::llvm_symbol_for(name))
            .ok_or_else(|| format!("function value `{name}` is not declared"))?;
        let adapter_name = format!("__thaw_function_ref_{}", self.next_lambda);
        self.next_lambda += 1;
        let adapter = self.module.add_function(
            &adapter_name,
            self.function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let args = adapter
            .get_param_iter()
            .skip(1)
            .map(BasicMetadataValueEnum::from)
            .collect::<Vec<_>>();
        let call = self
            .builder
            .build_call(target, &args, "invoke_function_ref")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("function reference returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(parent);
        let this_adapter = self.compile_ignored_this_adapter(
            adapter,
            params,
            ret,
            &format!("{adapter_name}__thaw_this_adapter"),
        )?;
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(CLOSURE_CAPTURE_BASE, false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "function_ref_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("function reference closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, adapter.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "function_ref_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(
                this_entry,
                this_adapter.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        Ok(closure.into())
    }

    /// Allocates `[i64 length][f64 elem0]...[f64 elemN-1]` from the arena
    /// and returns a pointer to the start of the buffer (the array value).
    fn compile_method_ref(
        &mut self,
        unbound: &str,
        explicit: &str,
        params: &[HirType],
        ret: &HirType,
        is_static: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let unbound_target = self
            .module
            .get_function(&Self::llvm_symbol_for(unbound))
            .ok_or_else(|| format!("unbound method entry `{unbound}` is not declared"))?;
        let explicit_target = self
            .module
            .get_function(&Self::llvm_symbol_for(explicit))
            .ok_or_else(|| format!("explicit method entry `{explicit}` is not declared"))?;
        let ordinary_name = format!("__thaw_method_ref_{}", self.next_lambda);
        self.next_lambda += 1;
        let ordinary = self.module.add_function(
            &ordinary_name,
            self.function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(ordinary, "entry");
        self.builder.position_at_end(entry);
        let arguments = ordinary
            .get_param_iter()
            .skip(1)
            .map(BasicMetadataValueEnum::from)
            .collect::<Vec<_>>();
        let call = self
            .builder
            .build_call(unbound_target, &arguments, "invoke_unbound_method")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("unbound method entry returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let this_entry = self.module.add_function(
            &format!("{ordinary_name}__thaw_this_adapter"),
            self.function_type(&this_params, ret)?,
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(this_entry, "entry");
        self.builder.position_at_end(entry);
        let mut arguments = Vec::with_capacity(params.len() + usize::from(!is_static));
        if !is_static {
            let receiver = this_entry.get_nth_param(1).unwrap().into_int_value();
            let receiver = self
                .builder
                .build_int_to_ptr(
                    receiver,
                    self.context.ptr_type(AddressSpace::default()),
                    "method_receiver",
                )
                .map_err(|error| error.to_string())?;
            arguments.push(BasicMetadataValueEnum::from(receiver));
        }
        arguments.extend(
            this_entry
                .get_param_iter()
                .skip(2)
                .map(BasicMetadataValueEnum::from),
        );
        let call = self
            .builder
            .build_call(explicit_target, &arguments, "invoke_method_with_this")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("explicit method entry returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(parent);
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(CLOSURE_CAPTURE_BASE, false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "method_ref_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("method reference closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, ordinary.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "method_ref_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(this_slot, this_entry.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        Ok(closure.into())
    }
}
