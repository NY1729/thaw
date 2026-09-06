fn napi_constructor_export_name(symbol: &str) -> Option<&str> {
    let constructor = symbol.strip_prefix("$new$")?;
    Some(
        constructor
            .rsplit_once("$arity")
            .map_or(constructor, |(name, _)| name),
    )
}

fn dynamic_json_collection_element_supported(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Str | HirType::Bool | HirType::Json => true,
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            dynamic_json_collection_element_supported(payload)
        }
        HirType::Array(element) => dynamic_json_collection_element_supported(element),
        HirType::Tuple(elements) => elements
            .iter()
            .all(dynamic_json_collection_element_supported),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| dynamic_json_collection_element_supported(field)),
        _ => false,
    }
}

fn jit_parameter_slots(ty: &HirType) -> Option<usize> {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str => Some(1),
        HirType::Union(elements) if jit_argument_tagged_union(elements) => Some(2),
        HirType::Array(element)
            if jit_array_result_element_supported(element) =>
        {
            Some(1)
        }
        HirType::Dictionary(element)
            if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str) =>
        {
            Some(1)
        }
        HirType::Object(fields) => fields.iter().try_fold(0usize, |slots, (_, ty)| {
            jit_parameter_slots(ty).map(|count| slots + count)
        }),
        HirType::Tuple(elements) => elements.iter().try_fold(0usize, |slots, ty| {
            jit_parameter_slots(ty).map(|count| slots + count)
        }),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            jit_parameter_slots(payload).map(|slots| slots + 1)
        }
        _ => None,
    }
}

fn jit_array_result_element_supported(ty: &HirType) -> bool {
    matches!(ty, HirType::F64 | HirType::Bool | HirType::Str)
        || matches!(
            ty,
            HirType::Tuple(elements)
                if matches!(elements.as_slice(), [HirType::Str, HirType::F64 | HirType::Bool | HirType::Str])
        )
}

fn jit_tagged_union(elements: &[HirType]) -> bool {
    jit_argument_tagged_union(elements)
}

fn jit_argument_tagged_union(elements: &[HirType]) -> bool {
    (2..=9).contains(&elements.len())
        && elements.iter().all(|element| jit_union_member_tag(element).is_some())
        && elements
            .iter()
            .enumerate()
            .all(|(index, element)| !elements[..index].contains(element))
        && elements
            .iter()
            .filter(|element| matches!(element, HirType::Object(_)))
            .count()
            <= 1
        && elements
            .iter()
            .filter(|element| matches!(element, HirType::Tuple(_)))
            .count()
            <= 1
        && (elements.contains(&HirType::Str)
            || elements
                .iter()
                .any(|element| {
                    matches!(
                        element,
                        HirType::Array(_)
                            | HirType::Dictionary(_)
                            | HirType::Object(_)
                            | HirType::Tuple(_)
                    )
                }))
}

fn jit_union_member_tag(ty: &HirType) -> Option<u64> {
    match ty {
        HirType::F64 => Some(1),
        HirType::Str => Some(2),
        HirType::Bool => Some(3),
        HirType::Array(element) => match element.as_ref() {
            HirType::F64 => Some(4),
            HirType::Bool => Some(5),
            HirType::Str => Some(6),
            _ => None,
        },
        HirType::Dictionary(element) => match element.as_ref() {
            HirType::F64 => Some(7),
            HirType::Bool => Some(8),
            HirType::Str => Some(9),
            _ => None,
        },
        HirType::Object(_) => Some(10),
        HirType::Tuple(_) => Some(11),
        _ => None,
    }
}

impl<'ctx> HirCompiler<'ctx> {
    fn compile_jit_argument_slots(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        path: &str,
        output: &mut Vec<BasicValueEnum<'ctx>>,
    ) -> Result<(), String> {
        if let HirType::Union(elements) = ty {
            if !jit_argument_tagged_union(elements) {
                return Err(format!("unsupported JIT union argument {ty:?}"));
            }
            let union = value.into_struct_value();
            let tag = self
                .builder
                .build_extract_value(union, 0, &format!("{path}_tag"))
                .map_err(|error| error.to_string())?
                .into_int_value();
            let payload = self
                .builder
                .build_extract_value(union, 1, &format!("{path}_payload"))
                .map_err(|error| error.to_string())?
                .into_int_value();
            let mut runtime_tag = self.context.i8_type().const_zero();
            for (index, member) in elements.iter().enumerate() {
                let semantic = jit_union_member_tag(member).unwrap();
                let selected = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        &format!("{path}_is_{index}"),
                    )
                    .map_err(|error| error.to_string())?;
                runtime_tag = self
                    .builder
                    .build_select(
                        selected,
                        self.context.i8_type().const_int(semantic, false),
                        runtime_tag,
                        &format!("{path}_runtime_tag"),
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
            }
            output.push(
                self.builder
                    .build_unsigned_int_to_float(
                        runtime_tag,
                        self.context.f64_type(),
                        &format!("{path}_tag_slot"),
                    )
                    .map_err(|error| error.to_string())?
                    .into(),
            );
            output.push(
                self.builder
                    .build_bit_cast(
                        payload,
                        self.context.f64_type(),
                        &format!("{path}_payload_slot"),
                    )
                    .map_err(|error| error.to_string())?,
            );
            return Ok(());
        }
        if let HirType::Object(fields) = ty {
            let object = value.into_pointer_value();
            let mut offset = 0u64;
            for (field, field_type) in fields {
                let field_path = format!("{path}_{field}");
                let pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            object,
                            &[self.context.i64_type().const_int(offset, false)],
                            &field_path,
                        )
                        .map_err(|error| error.to_string())?
                };
                let field_value = self
                    .builder
                    .build_load(
                        self.basic_type(field_type)?,
                        pointer,
                        &format!("{field_path}_value"),
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_jit_argument_slots(field_value, field_type, &field_path, output)?;
                offset += object_field_storage_bytes(field_type);
            }
            return Ok(());
        }
        if let HirType::Tuple(elements) = ty {
            let tuple = self.compile_array_data(value.into_pointer_value())?;
            let stride = elements
                .iter()
                .map(array_element_storage_bytes)
                .max()
                .unwrap_or(ARRAY_ELEM_BYTES);
            for (index, element_type) in elements.iter().enumerate() {
                let element_path = format!("{path}_{index}");
                let pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            tuple,
                            &[self.context.i64_type().const_int(
                                ARRAY_HEADER_BYTES + stride * index as u64,
                                false,
                            )],
                            &element_path,
                        )
                        .map_err(|error| error.to_string())?
                };
                let element = self
                    .builder
                    .build_load(
                        self.basic_type(element_type)?,
                        pointer,
                        &format!("{element_path}_value"),
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_jit_argument_slots(
                    element,
                    element_type,
                    &element_path,
                    output,
                )?;
            }
            return Ok(());
        }
        output.push(if *ty == HirType::Bool {
            self.builder
                .build_unsigned_int_to_float(
                    value.into_int_value(),
                    self.context.f64_type(),
                    "jit_boolean_slot",
                )
                .map_err(|error| error.to_string())?
                .into()
        } else {
            value
        });
        Ok(())
    }

    fn compile_napi_value_callback(
        &mut self,
        callback: &HirExpr,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let closure = self.compile_expr(callback)?.into_pointer_value();
        self.compile_napi_value_callback_from_closure(closure, params, ret)
    }

    fn compile_napi_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let (adapter, closure, _) =
            self.compile_value_callback_from_closure(closure, params, ret, false)?;
        Ok((adapter, closure))
    }

    fn compile_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        defer_promise: bool,
    ) -> Result<
        (
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            Option<PointerValue<'ctx>>,
        ),
        String,
    > {
        let callback_name = format!("__thaw_napi_value_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        let adapter = self
            .module
            .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        // `adapter`'s own body is a *separate* LLVM function from whatever
        // is currently being compiled -- if that outer compilation is
        // itself inside a user `try` block (real example: a database call
        // wrapped in `try { ... } catch { ... }`, awaited inside an async
        // function that also registers an async callback via this same
        // adapter), `self.catch_stack` still holds the *outer* function's
        // own catch block. `branch_on_pending_exception` (called below,
        // via `drive_promise_to_resolved_value`/`drive_promise_to_
        // completion` for a `Promise`-returning callback) would otherwise
        // branch straight into that unrelated block from inside this
        // adapter -- an illegal cross-function edge ("Referring to a basic
        // block in another function!", an LLVM module-verification
        // failure). `adapter` has no enclosing `try` of its own, so it
        // must see an empty catch stack (propagate/default-return on
        // failure, matching a top-level function with no active catch).
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let args_string = adapter.get_nth_param(1).unwrap();
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[args_string.into()],
                "napi_value_callback_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let null_key = ptr_type.const_null();
        let mut callback_args = vec![context.into()];
        for (index, param) in params.iter().enumerate() {
            let argument = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        args_json.into(),
                        self.context.f64_type().const_float(index as f64).into(),
                        null_key.into(),
                    ],
                    "napi_value_callback_argument",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            callback_args.push(self.compile_json_value_to_native(argument, param)?.into());
        }
        let code = self
            .builder
            .build_load(ptr_type, context, "napi_value_callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = self.function_type(params, ret)?;
        let call = self
            .builder
            .build_indirect_call(closure_type, code, &callback_args, "invoke_napi_value_callback")
            .map_err(|error| error.to_string())?;
        if defer_promise && matches!(ret, HirType::Promise(_)) {
            let promise = call
                .try_as_basic_value()
                .basic()
                .ok_or("async value callback must return a Promise")?
                .into_pointer_value();
            self.builder
                .build_return(Some(&promise))
                .map_err(|error| error.to_string())?;
            self.catch_stack = outer_catch_stack;
            self.builder.position_at_end(return_block);
            let HirType::Promise(resolved) = ret else {
                unreachable!()
            };
            let finish = self.compile_native_promise_callback_finisher(resolved)?;
            return Ok((
                adapter.as_global_value().as_pointer_value(),
                closure,
                Some(finish),
            ));
        }
        // A `void`-returning closure (real example: zod's own
        // `superRefine((val, ctx) => { ctx.addIssue(...); })` -- the
        // predicate mutates `ctx` and returns nothing at all) has no
        // return value to marshal; encoded as a bare JSON `null` result
        // instead of running it through the ordinary array-wrap-then-
        // index dance below, which requires a real value to push.
        let result_json = if matches!(ret, HirType::Undefined | HirType::Void) {
            self.compile_json_null()?
        } else {
            let result = call
                .try_as_basic_value()
                .basic()
                .ok_or("N-API value callback must return a value")?;
            // An `async` callback's declared return type is always
            // `Promise<T>` here (see `discover_frame_async_functions`'s
            // "every async function must expose the same Promise-handle
            // ABI" invariant), so `result` is a genuine, resolvable
            // `ThawPromise` pointer -- never a raw unwrapped value in
            // disguise. Drive it to its resolved value (propagating a
            // rejection as a pending thaw exception, same as an ordinary
            // `await`) before marshaling `T`, not `Promise<T>`, to JSON.
            let (result, ret) = match ret {
                HirType::Promise(resolved) if **resolved == HirType::Void => {
                    self.drive_promise_to_completion(result.into_pointer_value())?;
                    (result, resolved.as_ref())
                }
                HirType::Promise(resolved) => {
                    let resolved_value = self.drive_promise_to_resolved_value(
                        result.into_pointer_value(),
                        resolved,
                    )?;
                    (resolved_value, resolved.as_ref())
                }
                other => (result, other),
            };
            if matches!(ret, HirType::Undefined | HirType::Void) {
                self.compile_json_null()?
            } else {
                let result_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_array_new").unwrap(),
                        &[],
                        "napi_value_callback_result_array",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                self.compile_json_array_push_native(result_json, result, ret)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_index").unwrap(),
                        &[
                            result_json.into(),
                            self.context.f64_type().const_zero().into(),
                            null_key.into(),
                        ],
                        "napi_value_callback_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
            }
        };
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[result_json.into()],
                "napi_value_callback_result_string",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_return(Some(&result))
            .map_err(|error| error.to_string())?;
        self.catch_stack = outer_catch_stack;
        self.builder.position_at_end(return_block);
        Ok((adapter.as_global_value().as_pointer_value(), closure, None))
    }

    fn compile_native_promise_callback_finisher(
        &mut self,
        resolved: &HirType,
    ) -> Result<PointerValue<'ctx>, String> {
        let name = format!("__thaw_native_promise_finish_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            &name,
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            Some(Linkage::Internal),
        );
        let return_block = self.builder.get_insert_block().unwrap();
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let promise = function.get_nth_param(0).unwrap().into_pointer_value();
        self.builder
            .build_call(
                self.module.get_function("thaw_runtime_poll_one").unwrap(),
                &[],
                "poll_native_promise",
            )
            .map_err(|error| error.to_string())?;
        let state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[promise.into()],
                "native_promise_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_state returned no value")?
            .into_int_value();
        let pending = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_zero(),
                "native_promise_pending",
            )
            .map_err(|error| error.to_string())?;
        let pending_block = self.context.append_basic_block(function, "pending");
        let settled_block = self.context.append_basic_block(function, "settled");
        self.builder
            .build_conditional_branch(pending, pending_block, settled_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(pending_block);
        self.builder
            .build_return(Some(&ptr_type.const_null()))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(settled_block);
        let result_json = if *resolved == HirType::Void {
            self.drive_promise_to_completion(promise)?;
            self.compile_json_null()?
        } else {
            let value = self.drive_promise_to_resolved_value(promise, resolved)?;
            let array = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_array_new").unwrap(),
                    &[],
                    "native_promise_result_array",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            self.compile_json_array_push_native(array, value, resolved)?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        array.into(),
                        self.context.f64_type().const_zero().into(),
                        ptr_type.const_null().into(),
                    ],
                    "native_promise_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[result_json.into()],
                "native_promise_result_string",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_return(Some(&result))
            .map_err(|error| error.to_string())?;
        self.catch_stack = outer_catch_stack;
        self.builder.position_at_end(return_block);
        Ok(function.as_global_value().as_pointer_value())
    }

    /// Wraps a real compiled (native) closure as a live, retained QuickJS
    /// value the Fallback dynamic-call path can pass around like any
    /// other `JsValue` -- real example: zod's `z.number().refine((n:
    /// number) => n > 0, {...})`, whose predicate has nowhere to go
    /// without this (`coerce_to_declared`'s own doc comment on the
    /// thaw-hir side explains why a bare pass-through, the way `JsValue`
    /// itself gets, doesn't work here: there is no existing *live* QuickJS
    /// value yet, only a native function pointer that needs bridging into
    /// one first).
    ///
    /// Reuses `compile_napi_value_callback` as-is for the hard part (a
    /// generic `(context, args_json) -> result_json` adapter around the
    /// real closure, entirely backend-agnostic despite the name) rather
    /// than duplicating it -- the *only* new piece is handing that
    /// `(adapter, closure)` pair to QuickJS-NG (`thaw_js_register_native_
    /// callback`, thaw-quickjs) instead of to N-API, which wraps it in a
    /// real `rquickjs::Function` backed by a Rust closure that marshals a
    /// JS call into exactly the JSON-string-in/JSON-string-out shape the
    /// adapter expects, then retains and returns it the same way any
    /// other live value gets a permanent handle.
    fn compile_register_native_callback(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [closure_expr] = args else {
            return Err("registerNativeCallback expects exactly one argument".into());
        };
        let (params, ret) = match self.expr_hir_type(closure_expr) {
            Some(HirType::Function(params, ret)) => (params, *ret),
            _ => {
                return Err(
                    "registerNativeCallback: could not determine the callback's own function type"
                        .into(),
                );
            }
        };
        // Tells the JS-side wrapper (`thaw_js_register_native_callback`,
        // thaw-quickjs) which argument positions to retain as a live
        // handle (encoded as the same `{"__thaw_js_handle_id__": N}`
        // marker `compile_dynamic_value_placeholder` builds for the
        // opposite direction) instead of naively `JSON.stringify`-ing --
        // see `compile_json_value_to_native`'s own new `HirType::JsValue`
        // case, the matching native-side decoder. A `u64` is plenty (32
        // real params would already be an extraordinary callback), and
        // JS's own bitwise operators only ever work on 32 bits anyway.
        let closure = self.compile_expr(closure_expr)?.into_pointer_value();
        self.compile_register_native_callback_from_closure(closure, &params, &ret)
    }

    pub(super) fn compile_register_native_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let jsvalue_param_mask: u64 = params
            .iter()
            .enumerate()
            .filter(|(_, param)| **param == HirType::JsValue)
            .map(|(index, _)| 1u64 << index)
            .sum();
        let (adapter, closure, finish) =
            self.compile_value_callback_from_closure(closure, params, ret, true)?;
        let jsvalue_param_mask = self
            .context
            .i64_type()
            .const_int(jsvalue_param_mask, false);
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_register_native_callback")
                    .unwrap(),
                &[
                    adapter.into(),
                    closure.into(),
                    jsvalue_param_mask.into(),
                    finish.unwrap_or_else(|| self.context.ptr_type(AddressSpace::default()).const_null()).into(),
                ],
                "register_native_callback",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "register_native_callback_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "register_native_callback_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_napi_undefined_json(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "napi_undefined_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self
            .builder
            .build_global_string_ptr("$__thaw_napi_undefined$", "napi_undefined_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_bool")
                    .unwrap(),
                &[
                    json.into(),
                    key.as_pointer_value().into(),
                    self.context.i8_type().const_int(1, false).into(),
                ],
                "set_napi_undefined_tag",
            )
            .map_err(|error| error.to_string())?;
        Ok(json)
    }

    fn compile_json_is_napi_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let key = self
            .builder
            .build_global_string_ptr("$__thaw_napi_undefined$", "napi_undefined_test_key")
            .map_err(|error| error.to_string())?;
        let tagged = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_has_own").unwrap(),
                &[json.into(), key.as_pointer_value().into()],
                "json_is_napi_undefined",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_has_own returned no value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                tagged,
                self.context.i8_type().const_zero(),
                "json_is_napi_undefined_bool",
            )
            .map_err(|error| error.to_string())
    }

    fn compile_typed_dynamic_tagged_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        three_state: bool,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "napi_argument_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "napi_argument_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "napi_argument_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "napi_argument_value");
        let absent_block = self.context.append_basic_block(function, "napi_argument_absent");
        let done = self.context.append_basic_block(function, "napi_argument_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_array_push_native_with_undefined(array, payload, payload_type, true)?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = if three_state {
            let null_block = self.context.append_basic_block(function, "napi_argument_null");
            let undefined_block = self
                .context
                .append_basic_block(function, "napi_argument_undefined");
            let absent_done = self
                .context
                .append_basic_block(function, "napi_argument_absent_done");
            let is_null = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_int(1, false),
                    "napi_argument_is_null",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_null, null_block, undefined_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(null_block);
            let null = self.compile_json_null()?;
            self.builder
                .build_unconditional_branch(absent_done)
                .map_err(|error| error.to_string())?;
            let null_end = self.builder.get_insert_block().ok_or("lost N-API null block")?;
            self.builder.position_at_end(undefined_block);
            let undefined = self.compile_napi_undefined_json()?;
            self.builder
                .build_unconditional_branch(absent_done)
                .map_err(|error| error.to_string())?;
            let undefined_end = self
                .builder
                .get_insert_block()
                .ok_or("lost N-API undefined block")?;
            self.builder.position_at_end(absent_done);
            let result = self
                .builder
                .build_phi(self.context.ptr_type(AddressSpace::default()), "napi_absent_json")
                .map_err(|error| error.to_string())?;
            result.add_incoming(&[(&null, null_end), (&undefined, undefined_end)]);
            result.as_basic_value()
        } else {
            self.compile_napi_undefined_json()?
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[array.into(), absent.into()],
                "push_napi_absent_argument",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_typed_dynamic_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(), String> {
        match ty {
            HirType::Optional(payload) => self.compile_typed_dynamic_tagged_argument(
                array,
                value.into_struct_value(),
                payload,
                false,
            ),
            HirType::Nullish(payload) => self.compile_typed_dynamic_tagged_argument(
                array,
                value.into_struct_value(),
                payload,
                true,
            ),
            HirType::Array(element) => {
                let json = self.compile_native_array_to_json_with_undefined(
                    value.into_pointer_value(),
                    element,
                    true,
                )?;
                self.compile_json_array_push_native(array, json, &HirType::Json)
            }
            HirType::Tuple(elements) => {
                let json = self.compile_native_tuple_to_json_with_undefined(
                    value.into_pointer_value(),
                    elements,
                    true,
                )?;
                self.compile_json_array_push_native(array, json, &HirType::Json)
            }
            HirType::Object(_) => {
                let json = self.compile_native_object_to_json_with_undefined(
                    value.into_pointer_value(),
                    ty,
                    true,
                )?;
                self.compile_json_array_push_native(array, json, &HirType::Json)
            }
            _ => self.compile_json_array_push_native(array, value, ty),
        }
    }

    fn compile_napi_optional_result_container(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, PointerValue<'ctx>), String> {
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "napi_optional_result_object",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self
            .builder
            .build_global_string_ptr("value", "napi_optional_result_key")
            .map_err(|error| error.to_string())?
            .as_pointer_value();
        let is_undefined = self.compile_json_is_napi_undefined(json)?;
        let function = self.current_function();
        let absent = self.context.append_basic_block(function, "napi_result_undefined");
        let present = self.context.append_basic_block(function, "napi_result_present");
        let done = self.context.append_basic_block(function, "napi_result_optional_done");
        self.builder
            .build_conditional_branch(is_undefined, absent, present)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(absent);
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(present);
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_json")
                    .unwrap(),
                &[object.into(), key.into(), json.into()],
                "set_napi_optional_result",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok((object, key))
    }

    fn compile_typed_dynamic_result(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Json => Ok(json),
            HirType::Dictionary(_) => Ok(json),
            HirType::Optional(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(json)?;
                self.compile_json_to_optional_field(object, key, json, payload, false)
            }
            HirType::Nullable(payload) => self.compile_json_to_nullable_field(json, payload),
            HirType::Nullish(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(json)?;
                self.compile_json_to_nullish_field(object, key, json, payload)
            }
            HirType::Array(element) if **element == HirType::F64 => {
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_to_number_array")
                            .unwrap(),
                        &[json.into()],
                        "dynamic_number_array_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string())?
                    .into_pointer_value();
                Ok(self.compile_array_wrap(result)?.into())
            }
            HirType::Array(element) if dynamic_json_collection_element_supported(element) => {
                self.compile_json_to_native_array(json, element)
            }
            HirType::Tuple(elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, ty),
            // A `void`-returning dynamic call still marshals a JSON result
            // back across the boundary (there's no "no value" JSON
            // representation to special-case on the JS side), but the
            // caller has nothing to do with it: a `void`-declared
            // function's own return codegen discards whatever value its
            // return expression produced and emits a bare `build_return
            // (None)` regardless (see `compile_ignored_this_adapter` for
            // the same pattern), so any placeholder value is fine here.
            HirType::Void => Ok(self.context.i32_type().const_zero().into()),
            other => Err(format!("typed dynamic return does not support {other:?} yet")),
        }
    }

    /// `loadScript(source): boolean`, via thaw-quickjs's `thaw_js_load`.
    /// Same `i8` -> `i1` conversion as `compile_json_as_bool` and for the
    /// same reason (the extern function avoids relying on `bool`'s C ABI
    /// shape).
    fn compile_load_script(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        let [source] = args else {
            return Err("loadScript expects exactly one argument".to_string());
        };
        let source_val = self.compile_expr(source)?;
        let function = self.module.get_function("thaw_js_load").unwrap();
        let call = self
            .builder
            .build_call(function, &[source_val.into()], "load_script_u8")
            .map_err(|e| e.to_string())?;
        let u8_val = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_load did not return a value")?
            .into_int_value();
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "load_script_ok")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

    fn compile_load_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        if !(1..=2).contains(&args.len()) {
            return Err("loadNativeAddon expects a path and optional root export name".to_string());
        }
        let path = self.compile_expr(&args[0])?;
        let mut call_args = vec![path.into()];
        let symbol = if args.len() == 2 {
            call_args.push(self.compile_expr(&args[1])?.into());
            "thaw_napi_load_named"
        } else {
            "thaw_napi_load"
        };
        let function = self.module.get_function(symbol).unwrap();
        let loaded = self
            .builder
            .build_call(function, &call_args, "load_napi_u8")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_napi_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_load_embedded_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        if args.len() != 2 {
            return Err(
                "loadNativeAddonEmbedded expects addon bytes and a root export name".into(),
            );
        }
        let bytes = self.compile_expr(&args[0])?;
        let root = self.compile_expr(&args[1])?;
        let function = self
            .module
            .get_function("thaw_napi_load_embedded_hex")
            .unwrap();
        let loaded = self
            .builder
            .build_call(
                function,
                &[bytes.into(), root.into()],
                "load_embedded_napi_u8",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load_embedded_hex did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_embedded_napi_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    /// `callDynamic(name, args): Json` -- the QuickJS-NG fallback path
    /// (docs/design/bridge.md section 7). Composes thaw-std's
    /// `thaw_json_stringify`/`thaw_json_parse` with thaw-quickjs's
    /// `thaw_js_call` so a `Json` value flows in and out without this
    /// module needing to know thaw-quickjs's internals (or vice versa).
    fn compile_call_dynamic(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_result", "callDynamic")
    }

    fn compile_get_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.compile_single_arg_call("thaw_js_get_global", args, "getDynamicValue")
    }

    fn compile_call_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_handle_result", "callDynamicValue")
    }

    fn compile_call_native_addon_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        self.compile_json_backend_call(
            args,
            "thaw_napi_call_handle_typed_result",
            "callNativeAddonValue",
        )
    }

    fn compile_call_dynamic_value_handle(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, call_args] = args else {
            return Err("callDynamicValueHandle expects exactly two arguments".into());
        };
        let handle = self.compile_expr(handle)?;
        let call_args = self.compile_expr(call_args)?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "dynamic_handle_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_handle_result")
                    .unwrap(),
                &[handle.into(), args_json.into()],
                "dynamic_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_call_dynamic_value_with_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, argument] = args else {
            return Err("callDynamicValueWithValue expects exactly two arguments".into());
        };
        let callable = self.compile_expr(callable)?;
        let argument = self.compile_expr(argument)?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_value_result")
                    .unwrap(),
                &[callable.into(), argument.into()],
                "dynamic_value_argument_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_value_argument_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_value_argument_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "dynamic_value_argument_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_release_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let released = self
            .compile_single_arg_call("thaw_js_release_handle", args, "releaseDynamicValue")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                released,
                self.context.i8_type().const_zero(),
                "released_dynamic_value",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_dynamic_handle_operation(
        &mut self,
        symbol: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let values = args
            .iter()
            .map(|argument| self.compile_expr(argument).map(Into::into))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .builder
            .build_call(self.module.get_function(symbol).unwrap(), &values, symbol)
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_operation_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_operation_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    /// Compiles a `.method()` call's receiver expression, used by both
    /// `compile_call_dynamic_method` and `compile_call_dynamic_method_handle`.
    /// A receiver that is itself a `.method()` call is always lowered as
    /// the `"callDynamicMethodHandle"` intrinsic (`lower_dynamic_value_
    /// method_call` always requests `JsValue` for a receiver, regardless
    /// of what the *outer* call's own dispatch ends up being) -- so this
    /// structurally detects "I am not the terminal link in this chain" and
    /// compiles that inner call with `chain_intermediate: true`, meaning
    /// its own result must not have a pending thenable resolved early
    /// (see `compile_call_dynamic_method_handle`'s doc comment and the
    /// plan this fix came from: eagerly resolving every chain link, not
    /// just the terminal one, invokes a lazy query-builder's `.then` --
    /// e.g. drizzle-orm's -- before later links like `.where(...)` apply).
    fn compile_dynamic_method_receiver(
        &mut self,
        handle: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match handle {
            HirExpr::Call(callee, inner_args)
                if matches!(callee.as_ref(), HirExpr::Var(n) if n == "callDynamicMethodHandle") =>
            {
                self.compile_call_dynamic_method_handle(inner_args, true)
            }
            _ => self.compile_expr(handle),
        }
    }

    fn compile_call_dynamic_method(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethod expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        // Set for the same reason `compile_typed_dynamic_call` sets it
        // around its own args-marshaling: `call_args` (the pre-built
        // `HirExpr::ArrayLit` of already-Json-coerced arguments -- see
        // `lower_dynamic_value_method_call`) can itself contain a
        // `JsValue`-typed element (a live handle nested in the arg array,
        // not just a bare top-level arg -- real example: zod's `.refine(
        // (n: number) => n > 0, ...)`, whose predicate becomes a `JsValue`
        // via `registerNativeCallback`), and `compile_dynamic_value_
        // placeholder` refuses to encode one unless this flag is set.
        // Method-call args previously never set it at all (only the typed
        // ambient-declaration dispatch path did), so a `JsValue` nested in
        // a method call's own arguments -- as opposed to a top-level
        // function call's -- failed outright until now.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let call_args = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let call_args = call_args?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "dynamic_method_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_method_result")
                    .unwrap(),
                &[handle.into(), name.into(), args_json.into()],
                "dynamic_method_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "dynamic_method_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    /// Like `compile_call_dynamic_method`, but for a method whose own
    /// result is itself a `JsValue` (e.g. a schema instance's chained
    /// method returning another schema instance) rather than plain data:
    /// retains the result as a handle via `thaw_js_call_method_handle_result`
    /// instead of JSON-decoding it. See `callDynamicMethodHandle`'s doc
    /// comment (thaw-hir's `inference/types.rs`) for how a call chooses
    /// between the two.
    fn compile_call_dynamic_method_handle(
        &mut self,
        args: &[HirExpr],
        chain_intermediate: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethodHandle expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        // See the matching comment in `compile_call_dynamic_method`.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let call_args = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let call_args = call_args?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "dynamic_method_handle_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let chain_intermediate_flag = self
            .context
            .bool_type()
            .const_int(chain_intermediate as u64, false);
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_method_handle_result")
                    .unwrap(),
                &[
                    handle.into(),
                    name.into(),
                    args_json.into(),
                    chain_intermediate_flag.into(),
                ],
                "dynamic_method_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_read_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle] = args else {
            return Err("readDynamicValue expects exactly one argument".into());
        };
        let handle = self.compile_expr(handle)?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_resolve_handle_result")
                    .unwrap(),
                &[handle.into()],
                "read_dynamic_value",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "read_dynamic_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "read_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "read_dynamic_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_call_dynamic_value_mixed(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, json_args, handles] = args else {
            return Err(
                "callDynamicValueMixed expects callable, JSON arguments, and handle array".into(),
            );
        };
        let callable = self.compile_expr(callable)?;
        let json_args = self.compile_expr(json_args)?;
        let handles = self.compile_expr(handles)?.into_pointer_value();
        let handles = self.compile_array_data(handles)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[json_args.into()],
                "mixed_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_mixed_result")
                    .unwrap(),
                &[callable.into(), text.into(), handles.into()],
                "mixed_dynamic_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "mixed_dynamic_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "mixed_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "mixed_dynamic_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_construct_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [constructor, json_args] = args else {
            return Err("constructDynamicValue expects constructor and JSON arguments".into());
        };
        let constructor = self.compile_expr(constructor)?;
        let json_args = self.compile_expr(json_args)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[json_args.into()],
                "constructor_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_construct_handle_result")
                    .unwrap(),
                &[constructor.into(), text.into()],
                "construct_dynamic_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "constructed_dynamic_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "construct_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_typed_dynamic_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if signature.backend == DynamicBackend::QuickJs {
            self.uses_quickjs = true;
        }
        if signature.backend == DynamicBackend::Napi {
            self.uses_napi = true;
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$getter$")
                || signature.symbol.starts_with("$staticgetter$"))
        {
            return self.compile_typed_napi_getter(signature, args);
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$setter$")
                || signature.symbol.starts_with("$staticsetter$"))
        {
            return self.compile_typed_napi_setter(signature, args);
        }
        if (signature.backend == DynamicBackend::Napi || signature.backend == DynamicBackend::QuickJs)
            && (signature.symbol.starts_with("$method$")
                || signature.symbol.starts_with("$methodvoid$")
                || signature.symbol.starts_with("$staticmethod$")
                || signature.symbol.starts_with("$staticmethodvoid$"))
        {
            return self.compile_typed_napi_method(signature, args);
        }
        if signature.backend == DynamicBackend::Jit {
            let return_type = match &signature.ret {
                HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload)
                    if matches!(payload.as_ref(), HirType::F64 | HirType::Bool | HirType::Str)
                        || matches!(payload.as_ref(), HirType::Array(element) if jit_array_result_element_supported(element))
                        || matches!(payload.as_ref(), HirType::Dictionary(element) if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str))
                        || matches!(payload.as_ref(), HirType::Union(elements) if jit_tagged_union(elements)) =>
                {
                    payload.as_ref()
                }
                ty @ (HirType::F64 | HirType::Bool | HirType::Str) => ty,
                ty @ HirType::Union(elements)
                    if jit_tagged_union(elements) => ty,
                ty @ HirType::Array(element)
                    if jit_array_result_element_supported(element) => ty,
                ty @ HirType::Dictionary(element)
                    if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str) => ty,
                _ => {
                    return Err(format!(
                        "JIT calls currently return supported primitives, tagged unions, arrays, dictionaries, or optional supported values, not {:?}",
                        signature.ret
                    ));
                }
            };
            let argument_slots = signature
                .params
                .iter()
                .map(jit_parameter_slots)
                .collect::<Option<Vec<_>>>()
                .ok_or("JIT calls require supported primitive, tagged union, array, dictionary, object, or tuple arguments")?
                .into_iter()
                .sum::<usize>();
            if argument_slots > 16
                || args.len() != signature.params.len()
            {
                return Err(
                    "JIT calls currently require supported arguments fitting 16 ABI slots"
                        .into(),
                );
            }
            let name = self
                .builder
                .build_global_string_ptr(&signature.symbol, "jit_symbol")
                .map_err(|error| error.to_string())?;
            let argument_storage = self
                .builder
                .build_alloca(
                    self.context
                        .i8_type()
                        .array_type((argument_slots * 8) as u32),
                    "jit_arguments",
                )
                .map_err(|error| error.to_string())?;
            let mut argument_values = Vec::with_capacity(argument_slots);
            for (index, argument) in args.iter().enumerate() {
                let value = self.compile_expr(argument)?;
                if let HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload) = &signature.params[index]
                {
                    let value = value.into_struct_value();
                    let tag = self
                        .builder
                        .build_extract_value(value, 0, "jit_tagged_argument_tag")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    let present = if matches!(&signature.params[index], HirType::Nullish(_)) {
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                tag,
                                tag.get_type().const_zero(),
                                "jit_nullish_argument_present",
                            )
                            .map_err(|error| error.to_string())?
                    } else {
                        tag
                    };
                    argument_values.push(
                        self.builder
                            .build_unsigned_int_to_float(
                                tag,
                                self.context.f64_type(),
                                "jit_tagged_argument_tag_slot",
                            )
                            .map_err(|error| error.to_string())?
                            .into(),
                    );
                    let payload_value = self
                        .builder
                        .build_extract_value(value, 1, "jit_tagged_argument_payload")
                        .map_err(|error| error.to_string())?;
                    if matches!(payload.as_ref(), HirType::Object(_) | HirType::Tuple(_)) {
                        let function = self.current_function();
                        let present_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_present");
                        let absent_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_absent");
                        let merge_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_merge");
                        self.builder
                            .build_conditional_branch(present, present_block, absent_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(present_block);
                        let mut present_values = Vec::new();
                        self.compile_jit_argument_slots(
                            payload_value,
                            payload,
                            &format!("jit_optional_argument_{index}"),
                            &mut present_values,
                        )?;
                        let present_end = self.builder.get_insert_block().unwrap();
                        self.builder
                            .build_unconditional_branch(merge_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(absent_block);
                        let absent_values = present_values
                            .iter()
                            .map(|value| match value {
                                BasicValueEnum::FloatValue(value) => {
                                    Ok(value.get_type().const_zero().into())
                                }
                                BasicValueEnum::IntValue(value) => {
                                    Ok(value.get_type().const_zero().into())
                                }
                                BasicValueEnum::PointerValue(value) => {
                                    Ok(value.get_type().const_null().into())
                                }
                                _ => Err("JIT aggregate slots require scalar or pointer leaves"),
                            })
                            .collect::<Result<Vec<BasicValueEnum<'ctx>>, _>>()?;
                        let absent_end = self.builder.get_insert_block().unwrap();
                        self.builder
                            .build_unconditional_branch(merge_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(merge_block);
                        for (present_value, absent_value) in
                            present_values.iter().zip(&absent_values)
                        {
                            let phi = self
                                .builder
                                .build_phi(present_value.get_type(), "jit_optional_aggregate_slot")
                                .map_err(|error| error.to_string())?;
                            phi.add_incoming(&[
                                (present_value, present_end),
                                (absent_value, absent_end),
                            ]);
                            argument_values.push(phi.as_basic_value());
                        }
                        continue;
                    }
                    self.compile_jit_argument_slots(
                        payload_value,
                        payload,
                        &format!("jit_optional_argument_{index}"),
                        &mut argument_values,
                    )?;
                    continue;
                }
                self.compile_jit_argument_slots(
                    value,
                    &signature.params[index],
                    &format!("jit_argument_{index}"),
                    &mut argument_values,
                )?;
            }
            for (index, value) in argument_values.into_iter().enumerate() {
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            argument_storage,
                            &[self.context.i64_type().const_int((index * 8) as u64, false)],
                            "jit_argument",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(slot, value)
                    .map_err(|error| error.to_string())?;
            }
            let result = self
                .builder
                .build_call(
                    self.module.get_function("thaw_jit_call_f64").unwrap(),
                    &[
                        name.as_pointer_value().into(),
                        argument_storage.into(),
                        self.context
                            .i64_type()
                            .const_int(argument_slots as u64, false)
                            .into(),
                        self.module
                            .get_function("thaw_arena_alloc")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_number_to_string")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_to_number")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_parse_float")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_parse_int")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_format_number")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_search")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_format")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_normalize")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_split")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_slice")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_concat")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_append")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_to_reversed")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_to_sorted")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_reverse")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_sort")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_fill")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_copy_within")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_push")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_unshift")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_remove")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_splice")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_set")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_with")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_math_random")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_date_now")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_performance_now")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_process_pid")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_process_ppid")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_to_array")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_from_char_code")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_from_code_point")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_get")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_mutate")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_query")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                    ],
                    "jit_numeric_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "jit_numeric_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "jit_numeric_error")
                .map_err(|error| error.to_string())?;
            let error = error.into_pointer_value();
            let status = self
                .builder
                .build_ptr_to_int(
                    error,
                    self.context.i64_type(),
                    "jit_status",
                )
                .map_err(|error| error.to_string())?;
            let undefined = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    status,
                    self.context.i64_type().const_int(1, false),
                    "jit_value_absent",
                )
                .map_err(|error| error.to_string())?;
            let null = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    status,
                    self.context.i64_type().const_int(2, false),
                    "jit_value_null",
                )
                .map_err(|error| error.to_string())?;
            let absent = self
                .builder
                .build_or(undefined, null, "jit_value_nullish")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_select(
                    absent,
                    self.context.ptr_type(inkwell::AddressSpace::default()).const_null(),
                    error,
                    "jit_error_without_absence_tag",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            let value = if *return_type == HirType::Bool {
                self.builder
                    .build_float_compare(
                        FloatPredicate::ONE,
                        value.into_float_value(),
                        self.context.f64_type().const_zero(),
                        "jit_boolean_value",
                    )
                    .map(BasicValueEnum::from)
                    .map_err(|error| error.to_string())?
            } else if let HirType::Union(elements) = return_type {
                if !jit_tagged_union(elements) {
                    return Err(format!(
                        "JIT tagged result has unsupported members {return_type:?}"
                    ));
                }
                let bits = self
                    .builder
                    .build_bit_cast(
                        value.into_float_value(),
                        self.context.i64_type(),
                        "jit_dynamic_bits",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let pointer = self
                    .builder
                    .build_int_to_ptr(
                        bits,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "jit_dynamic_value",
                    )
                    .map_err(|error| error.to_string())?;
                let pointer = if matches!(
                    signature.ret,
                    HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)
                ) {
                    let storage_type = self.context.i64_type().array_type(2);
                    let absent_storage = self
                        .builder
                        .build_alloca(storage_type, "jit_absent_dynamic_value")
                        .map_err(|error| error.to_string())?;
                    self.builder
                        .build_store(absent_storage, storage_type.const_zero())
                        .map_err(|error| error.to_string())?;
                    self.builder
                        .build_select(
                            absent,
                            absent_storage,
                            pointer,
                            "jit_present_dynamic_value",
                        )
                        .map_err(|error| error.to_string())?
                        .into_pointer_value()
                } else {
                    pointer
                };
                let payload_pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i64_type(),
                            pointer,
                            &[self.context.i64_type().const_int(1, false)],
                            "jit_dynamic_payload_pointer",
                        )
                        .map_err(|error| error.to_string())?
                };
                let runtime_tag = self
                    .builder
                    .build_load(self.context.i64_type(), pointer, "jit_dynamic_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let mut tag = self.context.i8_type().const_zero();
                for (member, ty) in elements.iter().enumerate() {
                    let runtime = jit_union_member_tag(ty).unwrap();
                    let selected = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_tag,
                            self.context.i64_type().const_int(runtime, false),
                            &format!("jit_dynamic_is_{runtime}"),
                        )
                        .map_err(|error| error.to_string())?;
                    tag = self
                        .builder
                        .build_select(
                            selected,
                            self.context.i8_type().const_int(member as u64, false),
                            tag,
                            "jit_union_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                }
                let payload = self
                    .builder
                    .build_load(
                        self.context.i64_type(),
                        payload_pointer,
                        "jit_union_payload",
                    )
                    .map_err(|error| error.to_string())?;
                let union_type = self.basic_type(return_type)?.into_struct_type();
                let union = self
                    .builder
                    .build_insert_value(union_type.get_undef(), tag, 0, "jit_union_with_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                self.builder
                    .build_insert_value(union, payload, 1, "jit_union_with_payload")
                    .map_err(|error| error.to_string())?
                    .into_struct_value()
                    .into()
            } else if matches!(
                return_type,
                HirType::Str | HirType::Array(_) | HirType::Dictionary(_)
            ) {
                let bits = self
                    .builder
                    .build_bit_cast(
                        value.into_float_value(),
                        self.context.i64_type(),
                        "jit_string_bits",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let pointer = self.builder
                    .build_int_to_ptr(
                        bits,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "jit_string_value",
                    )
                    .map_err(|error| error.to_string())?;
                if matches!(return_type, HirType::Array(_)) {
                    self.compile_array_wrap(pointer)?.into()
                } else {
                    pointer.into()
                }
            } else {
                value
            };
            if let HirType::Optional(payload) | HirType::Nullable(payload) = &signature.ret {
                let tagged_type = self.basic_type(&signature.ret)?.into_struct_type();
                let present = self
                    .builder
                    .build_not(absent, "jit_optional_present")
                    .map_err(|error| error.to_string())?;
                let tagged = self
                    .builder
                    .build_insert_value(tagged_type.get_undef(), present, 0, "jit_optional_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                return self
                    .builder
                    .build_insert_value(tagged, value, 1, "jit_optional_payload")
                    .map(|value| value.into_struct_value().into())
                    .map_err(|error| format!("JIT optional {payload:?} result: {error}"));
            }
            if let HirType::Nullish(payload) = &signature.ret {
                let tagged_type = self.basic_type(&signature.ret)?.into_struct_type();
                let absent_tag = self
                    .builder
                    .build_select(
                        null,
                        self.context.i8_type().const_int(1, false),
                        self.context.i8_type().const_int(2, false),
                        "jit_nullish_absent_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let tag = self
                    .builder
                    .build_select(
                        absent,
                        absent_tag,
                        self.context.i8_type().const_zero(),
                        "jit_nullish_tag",
                    )
                    .map_err(|error| error.to_string())?;
                let tagged = self
                    .builder
                    .build_insert_value(tagged_type.get_undef(), tag, 0, "jit_nullish_with_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                return self
                    .builder
                    .build_insert_value(tagged, value, 1, "jit_nullish_with_payload")
                    .map(|value| value.into_struct_value().into())
                    .map_err(|error| format!("JIT nullish {payload:?} result: {error}"));
            }
            return Ok(value);
        }
        let function_argument = signature
            .params
            .iter()
            .take(args.len())
            .enumerate()
            .filter_map(|(index, ty)| match ty {
                HirType::Function(params, ret) => Some((index, params.as_slice(), ret.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();
        if function_argument.len() > 1 {
            return Err("typed N-API calls support at most one function argument".into());
        }
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "dynamic_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        // Saved and restored around the loop (rather than just cleared
        // afterward) because `compile_expr(arg)` below can itself recurse
        // into another dynamic call (e.g. zod's `optional(string())`,
        // where `string()`'s own call is compiled while marshaling
        // `optional`'s argument list) -- without this, that nested call
        // (or one for an unrelated backend) would leave the flag in
        // whatever state its own call left it, rather than this call's.
        // See `compile_dynamic_value_placeholder` (json_bridge.rs), which
        // reads the flag while marshaling each argument below.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = signature.backend == DynamicBackend::QuickJs;
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            if function_argument
                .first()
                .is_some_and(|(function_index, _, _)| *function_index == index)
            {
                continue;
            }
            let value = self.compile_expr(arg)?;
            self.compile_typed_dynamic_argument(array, value, ty)
                .map_err(|error| format!("typed dynamic argument {}: {error}", index + 1))?;
        }
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let name = self
            .builder
            .build_global_string_ptr(&signature.symbol, "dynamic_symbol")
            .map_err(|error| error.to_string())?;
        if signature.backend == DynamicBackend::Napi && signature.ret == HirType::JsValue {
            let Some(constructor_name) = napi_constructor_export_name(&signature.symbol) else {
                let args_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify").unwrap(),
                        &[array.into()],
                        "napi_handle_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let result = if let Some((index, params, ret)) = function_argument.first() {
                    let (callback, context) =
                        self.compile_napi_value_callback(&args[*index], params, ret)?;
                    self.builder.build_call(
                        self.module
                            .get_function("thaw_napi_call_export_handle_with_function_typed_result")
                            .unwrap(),
                        &[
                            name.as_pointer_value().into(),
                            args_json.into(),
                            self.context.i64_type().const_int(*index as u64, false).into(),
                            callback.into(),
                            context.into(),
                        ],
                        "napi_export_function_handle_result",
                    )
                } else {
                    self.builder.build_call(
                        self.module
                            .get_function("thaw_napi_call_export_handle_typed_result")
                            .unwrap(),
                        &[name.as_pointer_value().into(), args_json.into()],
                        "napi_export_handle_result",
                    )
                }
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_struct_value();
                let value = self
                    .builder
                    .build_extract_value(result, 0, "napi_export_handle")
                    .map_err(|error| error.to_string())?;
                let error = self
                    .builder
                    .build_extract_value(result, 1, "napi_export_handle_error")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            };
            let constructor_name = self
                .builder
                .build_global_string_ptr(constructor_name, "napi_constructor_name")
                .map_err(|error| error.to_string())?;
            let constructor = self
                .builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[constructor_name.as_pointer_value().into()],
                    "napi_constructor",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "napi_constructor_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_construct_handle_typed_result")
                        .unwrap(),
                    &[constructor.into(), args_json.into()],
                    "napi_construct_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "napi_constructed_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "napi_construct_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::QuickJs && signature.ret == HirType::JsValue {
            self.uses_quickjs_handles = true;
            // `napi_constructor_export_name` is plain `"$new$Class..."`
            // runtime-key parsing, not actually NAPI-specific despite the
            // name -- reused here for a Fallback (QuickJS-NG) class's own
            // `new Class(...)` construction (real example: hono's
            // `Hono`), the same way the `DynamicBackend::Napi` branch
            // above uses it for a native addon's.
            if let Some(constructor_name) = napi_constructor_export_name(&signature.symbol) {
                let constructor_name = self
                    .builder
                    .build_global_string_ptr(constructor_name, "typed_dynamic_constructor_name")
                    .map_err(|error| error.to_string())?;
                let constructor = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_js_get_global").unwrap(),
                        &[constructor_name.as_pointer_value().into()],
                        "typed_dynamic_constructor",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let args_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify").unwrap(),
                        &[array.into()],
                        "typed_dynamic_construct_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_construct_handle_result")
                            .unwrap(),
                        &[constructor.into(), args_json.into()],
                        "typed_dynamic_construct_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_struct_value();
                let value = self
                    .builder
                    .build_extract_value(result, 0, "typed_dynamic_constructed_value")
                    .map_err(|error| error.to_string())?;
                let error = self
                    .builder
                    .build_extract_value(result, 1, "typed_dynamic_construct_error")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            }
            let callable = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_get_global").unwrap(),
                    &[name.as_pointer_value().into()],
                    "typed_dynamic_callable",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "typed_dynamic_callable_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_js_call_handle_handle_result")
                        .unwrap(),
                    &[callable.into(), args_json.into()],
                    "typed_dynamic_callable_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "typed_dynamic_callable_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "typed_dynamic_callable_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        let backend = match signature.backend {
            DynamicBackend::Jit => unreachable!("JIT calls return before JSON marshalling"),
            DynamicBackend::QuickJs => "thaw_js_call_result",
            DynamicBackend::Napi => "thaw_napi_call_typed_result",
        };
        let json =
            self.compile_json_backend_values(name.as_pointer_value().into(), array, backend)?;
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    fn compile_typed_napi_setter(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticsetter$");
        let (receiver_expr, assigned) = if is_static {
            let [assigned] = args else {
                return Err("typed N-API static setter expects one value".into());
            };
            (None, assigned)
        } else {
            let [receiver, assigned] = args else {
                return Err("typed N-API setter expects a receiver and value".into());
            };
            (Some(receiver), assigned)
        };
        let receiver = if let Some(receiver) = receiver_expr {
            self.compile_expr(receiver)?
        } else {
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed N-API static setter symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "napi_static_setter_class_name")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[class.as_pointer_value().into()],
                    "napi_static_setter_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let assigned_type = signature
            .params
            .get(usize::from(!is_static))
            .ok_or("typed N-API setter is missing its value type")?;
        let assigned_value = self.compile_expr(assigned)?;
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "napi_setter_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.compile_typed_dynamic_argument(array, assigned_value, assigned_type)
            .map_err(|error| format!("N-API setter value: {error}"))?;
        let property = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API setter symbol")?;
        let property = self
            .builder
            .build_global_string_ptr(property, "napi_setter_name")
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[array.into()],
                "napi_setter_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_set_property_typed_result")
                    .unwrap(),
                &[
                    receiver.into(),
                    property.as_pointer_value().into(),
                    args_json.into(),
                ],
                "napi_setter_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_setter_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_setter_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_setter_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no setter value".to_string())?;
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    fn compile_typed_napi_getter(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticgetter$");
        let receiver = if is_static {
            if !args.is_empty() {
                return Err("typed N-API static getter expects no arguments".into());
            }
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed N-API static getter symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "napi_static_getter_class_name")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[class.as_pointer_value().into()],
                    "napi_static_getter_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        } else {
            let [receiver] = args else {
                return Err("typed N-API getter expects one receiver".into());
            };
            self.compile_expr(receiver)?
        };
        let property = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API getter symbol")?;
        let property = self
            .builder
            .build_global_string_ptr(property, "napi_getter_name")
            .map_err(|error| error.to_string())?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_get_property_typed_result")
                    .unwrap(),
                &[receiver.into(), property.as_pointer_value().into()],
                "napi_getter_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_getter_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_getter_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_getter_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no getter value".to_string())?;
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    fn compile_typed_napi_method(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticmethod$")
            || signature.symbol.starts_with("$staticmethodvoid$");
        let (receiver_expr, method_args) = if is_static {
            (None, args)
        } else {
            let [receiver, method_args @ ..] = args else {
                return Err("typed N-API method expects a receiver".into());
            };
            (Some(receiver), method_args)
        };
        let callback = (signature.backend == DynamicBackend::Napi
            && matches!(signature.params.last(), Some(HirType::Function(_, _))))
            .then(|| method_args.last())
            .flatten();
        let marshalled_args = if callback.is_some() {
            &method_args[..method_args.len() - 1]
        } else {
            method_args
        };
        let receiver = if let Some(receiver) = receiver_expr {
            self.compile_expr(receiver)?
        } else {
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed dynamic static method symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "dynamic_static_class_name")
                .map_err(|error| error.to_string())?;
            // A static method's receiver is the class's own export/global
            // binding rather than an instance -- looked up the same way
            // each backend already looks up any other named value (an
            // N-API module export vs. a QuickJS-NG global), matching
            // `thaw_napi_get_export`'s role for the `$new$.../ordinary
            // export-handle path above with `thaw_js_get_global`'s (real
            // example: a Fallback class's static method, e.g. a
            // hypothetical `Mime.define(...)`, once such a method is
            // generated by thaw-cli's shims.rs).
            let lookup = if signature.backend == DynamicBackend::QuickJs {
                "thaw_js_get_global"
            } else {
                "thaw_napi_get_export"
            };
            self.builder
                .build_call(
                    self.module.get_function(lookup).unwrap(),
                    &[class.as_pointer_value().into()],
                    "dynamic_static_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "napi_method_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = signature.backend == DynamicBackend::QuickJs;
        for (index, (argument, ty)) in marshalled_args
            .iter()
            .zip(signature.params.iter().skip(usize::from(!is_static)))
            .enumerate()
        {
            let (value, marshalled_type) = if signature.backend == DynamicBackend::QuickJs
                && matches!(ty, HirType::Function(_, _) | HirType::CallableFunction(..))
            {
                (
                    self.compile_register_native_callback(std::slice::from_ref(argument))?,
                    &HirType::JsValue,
                )
            } else {
                (self.compile_expr(argument)?, ty)
            };
            self.compile_typed_dynamic_argument(array, value, marshalled_type)
                .map_err(|error| format!("N-API method argument {}: {error}", index + 1))?;
        }
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let method = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API method symbol")?;
        let method = self
            .builder
            .build_global_string_ptr(method, "napi_method_name")
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[array.into()],
                "napi_method_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        if signature.backend == DynamicBackend::QuickJs && signature.ret == HirType::JsValue {
            self.uses_quickjs_handles = true;
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_js_call_method_handle_result")
                        .unwrap(),
                    &[
                        receiver.into(),
                        method.as_pointer_value().into(),
                        args_json.into(),
                        self.context.bool_type().const_zero().into(),
                    ],
                    "typed_dynamic_method_handle_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "typed_dynamic_method_handle_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "typed_dynamic_method_handle_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        let result = if let Some(callback) = callback {
            self.compile_typed_napi_method_callback(
                receiver,
                method.as_pointer_value(),
                args_json,
                callback,
                signature.symbol.starts_with("$methodvoid$")
                    || signature.symbol.starts_with("$staticmethodvoid$"),
            )?
        } else {
            let call_method = if signature.backend == DynamicBackend::QuickJs {
                "thaw_js_call_method_result"
            } else {
                "thaw_napi_call_method_typed_result"
            };
            self.builder
                .build_call(
                    self.module.get_function(call_method).unwrap(),
                    &[
                        receiver.into(),
                        method.as_pointer_value().into(),
                        args_json.into(),
                    ],
                    "dynamic_method_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value()
        };
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_method_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_method_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_method_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no method value".to_string())?;
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    fn compile_typed_napi_method_callback(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        method: PointerValue<'ctx>,
        args_json: BasicValueEnum<'ctx>,
        callback: &HirExpr,
        discard_result: bool,
    ) -> Result<StructValue<'ctx>, String> {
        let callback_type = match callback {
            HirExpr::Lambda(_, params, ret, _) => HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            ),
            HirExpr::Var(name) => self
                .variable_hir_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown callback `{name}`"))?,
            _ => return Err("typed N-API method callback must be a function value".into()),
        };
        let HirType::Function(params, ret) = callback_type else {
            return Err("typed N-API method callback must be a function".into());
        };
        if params.len() > 2
            || params.iter().any(|param| *param != HirType::Json)
            || !matches!(*ret, HirType::Json | HirType::Void)
        {
            return Err(
                "native addon method callback must take zero to two Json arguments and return Json or void"
                    .into(),
            );
        }
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let callback_name = format!("__thaw_napi_method_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        let adapter =
            self.module
                .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let adapter_entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(adapter_entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let error_string = adapter.get_nth_param(1).unwrap();
        let result_string = adapter.get_nth_param(2).unwrap();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let error_json = self
            .builder
            .build_call(parse, &[error_string.into()], "method_callback_error")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result_json = self
            .builder
            .build_call(parse, &[result_string.into()], "method_callback_result")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let code = self
            .builder
            .build_load(ptr_type, context, "method_callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = self.function_type(&params, &ret)?;
        let mut callback_args = vec![context.into()];
        if !params.is_empty() {
            callback_args.push(error_json.into());
        }
        if params.len() == 2 {
            callback_args.push(result_json.into());
        }
        self.builder
            .build_indirect_call(
                closure_type,
                code,
                &callback_args,
                "invoke_thaw_method_callback",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_napi_call_method_with_callback_result")
                    .unwrap(),
                &[
                    receiver.into(),
                    method.into(),
                    args_json.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                    self.context
                        .i8_type()
                        .const_int(discard_result as u64, false)
                        .into(),
                ],
                "napi_method_callback_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .map(|value| value.into_struct_value())
            .ok_or_else(|| "native method callback returned no result".into())
    }

    fn compile_call_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_napi_call_result", "callNativeAddon")
    }

    fn compile_poll_native_addon_events(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if !args.is_empty() {
            return Err("pollNativeAddonEvents expects no arguments".into());
        }
        self.uses_napi = true;
        let count = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_poll_async_work")
                    .unwrap(),
                &[],
                "poll_napi_events",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        self.builder
            .build_unsigned_int_to_float(count, self.context.f64_type(), "napi_event_count")
            .map(BasicValueEnum::FloatValue)
            .map_err(|error| error.to_string())
    }

    fn compile_call_native_addon_with_callback(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [name, call_args, callback] = args else {
            return Err(
                "callNativeAddonWithCallback expects a name, Json args, and callback".into(),
            );
        };
        let callback_type = match callback {
            HirExpr::Lambda(_, params, ret, _) => HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            ),
            HirExpr::Var(name) => self
                .variable_hir_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown callback `{name}`"))?,
            _ => {
                return Err("callNativeAddonWithCallback callback must be a function value".into())
            }
        };
        let HirType::Function(params, ret) = callback_type else {
            return Err("callNativeAddonWithCallback third argument must be a function".into());
        };
        if params != vec![HirType::Json, HirType::Json] || *ret != HirType::Json {
            return Err("native addon callback must have type (Json, Json) => Json".into());
        }

        self.uses_napi = true;
        let name = self.compile_expr(name)?;
        let args_json = self.compile_expr(call_args)?;
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let stringify = self.module.get_function("thaw_json_stringify").unwrap();
        let args_string = self
            .builder
            .build_call(stringify, &[args_json.into()], "napi_callback_args")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();

        let callback_name = format!("__thaw_napi_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        let adapter =
            self.module
                .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let adapter_entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(adapter_entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let error_string = adapter.get_nth_param(1).unwrap();
        let result_string = adapter.get_nth_param(2).unwrap();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let error_json = self
            .builder
            .build_call(parse, &[error_string.into()], "callback_error")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result_json = self
            .builder
            .build_call(parse, &[result_string.into()], "callback_result")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let code = self
            .builder
            .build_load(ptr_type, context, "callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = self.function_type(&params, &ret)?;
        self.builder
            .build_indirect_call(
                closure_type,
                code,
                &[context.into(), error_json.into(), result_json.into()],
                "invoke_thaw_callback",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);

        let call = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_call_with_callback_result")
                    .unwrap(),
                &[
                    name.into(),
                    args_string.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                ],
                "call_napi_with_callback",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(call, 0, "napi_callback_value")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let error = self
            .builder
            .build_extract_value(call, 1, "napi_callback_error")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_callback_queued_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse did not return a value".into())
    }
}
