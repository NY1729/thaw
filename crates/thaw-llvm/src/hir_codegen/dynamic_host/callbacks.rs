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

    fn compile_napi_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        rest_start: Option<usize>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let (adapter, closure, _) =
            self.compile_value_callback_from_closure(closure, params, ret, false, rest_start)?;
        Ok((adapter, closure))
    }

    fn compile_napi_function_arguments(
        &mut self,
        args: &[HirExpr],
        functions: &[NapiFunctionArgument],
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        if functions.is_empty() {
            return Ok(ptr_type.const_null());
        }
        let descriptor_type = self.context.struct_type(
            &[
                self.context.i64_type().into(),
                ptr_type.into(),
                ptr_type.into(),
            ],
            false,
        );
        let descriptors = self
            .builder
            .build_alloca(
                descriptor_type.array_type(functions.len() as u32),
                "napi_function_arguments",
            )
            .map_err(|error| error.to_string())?;
        for (slot, (index, params, ret, optional, rest_start)) in functions.iter().enumerate() {
            let callback = self.compile_expr(&args[*index])?;
            let (adapter, context) = if *optional {
                let callback = callback.into_struct_value();
                let present = self
                    .builder
                    .build_extract_value(callback, 0, "optional_napi_callback_present")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let closure = self
                    .builder
                    .build_extract_value(callback, 1, "optional_napi_callback")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let (adapter, context) =
                    self.compile_napi_value_callback_from_closure(
                        closure,
                        params,
                        ret,
                        *rest_start,
                    )?;
                let adapter = self
                    .builder
                    .build_select(present, adapter, ptr_type.const_null(), "napi_callback")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                (adapter, context)
            } else {
                self.compile_napi_value_callback_from_closure(
                    callback.into_pointer_value(),
                    params,
                    ret,
                    *rest_start,
                )?
            };
            let descriptor = self
                .builder
                .build_insert_value(
                    descriptor_type.get_undef(),
                    self.context.i64_type().const_int(*index as u64, false),
                    0,
                    "napi_function_argument_index",
                )
                .map_err(|error| error.to_string())?
                .into_struct_value();
            let descriptor = self
                .builder
                .build_insert_value(descriptor, adapter, 1, "napi_function_argument_callback")
                .map_err(|error| error.to_string())?
                .into_struct_value();
            let descriptor = self
                .builder
                .build_insert_value(descriptor, context, 2, "napi_function_argument_context")
                .map_err(|error| error.to_string())?;
            let target = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        descriptor_type,
                        descriptors,
                        &[self.context.i64_type().const_int(slot as u64, false)],
                        "napi_function_argument",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(target, descriptor)
                .map_err(|error| error.to_string())?;
        }
        Ok(descriptors)
    }

    fn compile_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        defer_promise: bool,
        rest_start: Option<usize>,
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
            let argument = if rest_start == Some(index) {
                self.builder.build_call(
                    self.module.get_function("thaw_json_array_slice").unwrap(),
                    &[
                        args_json.into(),
                        self.context.i64_type().const_int(index as u64, false).into(),
                    ],
                    "napi_value_callback_rest",
                )
            } else {
                self.builder.build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        args_json.into(),
                        self.context.f64_type().const_float(index as f64).into(),
                        null_key.into(),
                    ],
                    "napi_value_callback_argument",
                )
            }
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
        let rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_int(2, false),
                "native_promise_rejected",
            )
            .map_err(|error| error.to_string())?;
        let rejected_block = self.context.append_basic_block(function, "rejected");
        let fulfilled_block = self.context.append_basic_block(function, "fulfilled");
        self.builder
            .build_conditional_branch(rejected, rejected_block, fulfilled_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected_block);
        let error = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_run_until_resolved")
                    .unwrap(),
                &[promise.into()],
                "native_promise_error",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("rejected native Promise has no error")?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[promise.into()],
                "destroy_rejected_native_promise",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_return(Some(&error))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(fulfilled_block);
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
        let (params, ret, optional) = match self.expr_hir_type(closure_expr) {
            Some(HirType::Function(params, ret)) => (params, *ret, false),
            Some(HirType::Optional(inner)) => match *inner {
                HirType::Function(params, ret) => (params, *ret, true),
                _ => {
                    return Err(
                        "registerNativeCallback: could not determine the callback's own function type"
                            .into(),
                    );
                }
            },
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
        let closure = self.compile_expr(closure_expr)?;
        let closure = if optional {
            self.builder
                .build_extract_value(closure.into_struct_value(), 1, "optional_native_callback")
                .map_err(|error| error.to_string())?
        } else {
            closure
        }
        .into_pointer_value();
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
            self.compile_value_callback_from_closure(closure, params, ret, true, None)?;
        let jsvalue_param_mask = self
            .context
            .i64_type()
            .const_int(jsvalue_param_mask, false);
        let param_count = self
            .context
            .i64_type()
            .const_int(params.len() as u64, false);
        let void_result = matches!(ret, HirType::Void)
            || matches!(ret, HirType::Promise(value) if **value == HirType::Void);
        let void_result = self.context.i8_type().const_int(void_result as u64, false);
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
                    param_count.into(),
                    void_result.into(),
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

}
