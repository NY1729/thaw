/// Types that cross the thaw-js JSON boundary without losing identity or behavior.
pub(crate) fn thaw_js_plain_type(ty: &HirType) -> bool {
    match ty {
        HirType::F64
        | HirType::Bool
        | HirType::Str
        | HirType::Json
        | HirType::Null
        | HirType::Undefined
        | HirType::Void => true,
        HirType::Array(element) | HirType::Dictionary(element) => thaw_js_plain_type(element),
        HirType::Union(members) => members.iter().all(thaw_js_plain_type),
        HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
            thaw_js_plain_type(inner)
        }
        HirType::Object(fields) => fields.iter().all(|(_, ty)| thaw_js_plain_type(ty)),
        _ => false,
    }
}

/// The native `Date` object type (authenticated identity marker + `timestamp`).
fn thaw_js_date_type(ty: &HirType) -> bool {
    matches!(ty, HirType::Object(fields) if matches!(fields.as_slice(),
        [(marker, HirType::Bool), (timestamp, HirType::F64)]
            if marker == "__thaw_class_identity_\u{1e}Date" && timestamp == "timestamp"))
}

/// A callback parameter thaw-js can call back into: a plain-data function that returns plain data
/// synchronously (no Promise), optionally absent.
pub(crate) fn thaw_js_callback_type(ty: &HirType) -> bool {
    match ty {
        HirType::Function(params, ret) => {
            params.iter().all(thaw_js_plain_type) && thaw_js_plain_type(ret)
        }
        HirType::Optional(inner) => matches!(inner.as_ref(), HirType::Function(..)) && thaw_js_callback_type(inner),
        _ => false,
    }
}

/// A parameter type that can cross into thaw-js: plain data or a plain callback.
pub(crate) fn thaw_js_param_type(ty: &HirType) -> bool {
    thaw_js_plain_type(ty) || thaw_js_callback_type(ty) || *ty == HirType::JsValue
}

fn ordinary_quickjs_typed_call(signature: &DynamicSignature) -> bool {
    signature.backend == DynamicBackend::QuickJs
        && ![
            "$getter$",
            "$setter$",
            "$method$",
            "$methodvoid$",
            "$staticgetter$",
            "$staticsetter$",
            "$staticmethod$",
            "$staticmethodvoid$",
            "$new$",
        ]
        .iter()
        .any(|prefix| signature.symbol.starts_with(prefix))
}

impl<'ctx> HirCompiler<'ctx> {
    fn begin_fresh_event_key_argument_scope(&mut self) {
        self.begin_fresh_event_key_argument_scope_with_owner(None);
    }

    fn begin_fresh_event_key_argument_scope_with_owner(
        &mut self,
        typed_json_owner: Option<TypedJsonArgumentOwner>,
    ) {
        let catch_depth = self.catch_stack.len();
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        self.fresh_event_key_argument_scopes.push(FreshEventKeyArgumentScope {
            catch_depth,
            function_name,
            typed_json_owner,
            handles: Vec::new(),
            json_values: Vec::new(),
            argument_array: None,
            native_handle_ledgers: Vec::new(),
        });
    }

    fn track_typed_dynamic_argument_array(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let function = self.current_function();
        let function_name = function.get_name().to_string_lossy().into_owned();
        let scope_index = self.fresh_event_key_argument_scopes.iter().rposition(
            |scope| scope.function_name == function_name,
        ).ok_or_else(|| "typed dynamic argument array has no active owner scope".to_string())?;
        if self.fresh_event_key_argument_scopes[scope_index].argument_array.is_some() {
            return Err("typed dynamic argument array was already registered".into());
        }
        let entry = function.get_first_basic_block().unwrap();
        let entry_builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            entry_builder.position_before(&first);
        } else {
            entry_builder.position_at_end(entry);
        }
        let active = entry_builder.build_alloca(self.context.i8_type(), "typed_dynamic_argument_array_active")
            .map_err(|error| error.to_string())?;
        entry_builder.build_store(active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_store(active, self.context.i8_type().const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.fresh_event_key_argument_scopes[scope_index].argument_array = Some(FreshJsonArgumentArray {
            value: value.into_pointer_value(),
            active,
        });
        Ok(())
    }

    fn disarm_typed_dynamic_argument_array(
        &mut self,
        array: FreshJsonArgumentArray<'ctx>,
    ) -> Result<(), String> {
        self.builder.build_store(array.active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn release_typed_dynamic_argument_array_if_active(
        &mut self,
        array: FreshJsonArgumentArray<'ctx>,
    ) -> Result<(), String> {
        let active = self.builder.build_load(
            self.context.i8_type(), array.active, "typed_dynamic_argument_array_is_active",
        ).map_err(|error| error.to_string())?.into_int_value();
        let should_release = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, active, self.context.i8_type().const_zero(),
            "typed_dynamic_argument_array_should_release",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_typed_dynamic_argument_array");
        let done = self.context.append_basic_block(function, "typed_dynamic_argument_array_released");
        self.builder.build_conditional_branch(should_release, release, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[array.value.into()], "destroy_unwound_typed_dynamic_argument_array",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(array.active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn finish_fresh_event_key_argument_scope(
        &mut self,
    ) -> Result<Vec<FreshEventKeyHandle<'ctx>>, String> {
        let (handles, array) = self.finish_fresh_event_key_argument_scope_with_array()?;
        if let Some(array) = array {
            self.disarm_typed_dynamic_argument_array(array)?;
        }
        Ok(handles)
    }

    fn finish_fresh_event_key_argument_scope_with_array(
        &mut self,
    ) -> Result<(Vec<FreshEventKeyHandle<'ctx>>, Option<FreshJsonArgumentArray<'ctx>>), String> {
        let scope = self.fresh_event_key_argument_scopes.pop()
            .ok_or_else(|| "fresh event-key argument scope underflow".to_string())?;
        Ok((scope.handles, scope.argument_array))
    }

    fn begin_fresh_event_key_transfer_scope(
        &mut self,
        handles: &[FreshEventKeyHandle<'ctx>],
        argument_array: Option<FreshJsonArgumentArray<'ctx>>,
    ) {
        let catch_depth = self.catch_stack.len();
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        self.fresh_event_key_argument_scopes.push(FreshEventKeyArgumentScope {
            catch_depth,
            function_name,
            typed_json_owner: None,
            handles: handles.to_vec(),
            json_values: Vec::new(),
            argument_array,
            native_handle_ledgers: Vec::new(),
        });
    }

    fn track_fresh_event_key_handle(&mut self, handle: inkwell::values::IntValue<'ctx>) -> Result<(), String> {
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        if self.fresh_event_key_argument_scopes.iter().any(|scope| scope.function_name == function_name) {
            let entry = self.current_function().get_first_basic_block().unwrap();
            let entry_builder = self.context.create_builder();
            if let Some(first) = entry.get_first_instruction() {
                entry_builder.position_before(&first);
            } else {
                entry_builder.position_at_end(entry);
            }
            let active = entry_builder.build_alloca(self.context.i8_type(), "fresh_event_key_active")
                .map_err(|error| error.to_string())?;
            entry_builder.build_store(active, self.context.i8_type().const_zero())
                .map_err(|error| error.to_string())?;
            self.builder.build_store(active, self.context.i8_type().const_int(1, false))
                .map_err(|error| error.to_string())?;
            if let Some(scope) = self.fresh_event_key_argument_scopes.iter_mut().rev()
                .find(|scope| scope.function_name == function_name)
            {
                scope.handles.push(FreshEventKeyHandle { handle, active });
            }
        }
        Ok(())
    }

    fn track_typed_wrapper_json_argument(
        &mut self,
        value: inkwell::values::PointerValue<'ctx>,
        owner: &TypedJsonArgumentOwner,
    ) -> Result<(), String> {
        let function = self.current_function();
        let function_name = function.get_name().to_string_lossy().into_owned();
        let Some(scope_index) = self.fresh_event_key_argument_scopes.iter()
            .rposition(|scope| scope.function_name == function_name)
        else {
            return Ok(());
        };
        if self.fresh_event_key_argument_scopes[scope_index].typed_json_owner.as_ref() != Some(owner) {
            return Ok(());
        }
        let entry = function.get_first_basic_block().unwrap();
        let entry_builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            entry_builder.position_before(&first);
        } else {
            entry_builder.position_at_end(entry);
        }
        let active = entry_builder.build_alloca(self.context.i8_type(), "typed_wrapper_json_active")
            .map_err(|error| error.to_string())?;
        entry_builder.build_store(active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_store(active, self.context.i8_type().const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.fresh_event_key_argument_scopes[scope_index]
            .json_values.push(FreshJsonArgument { value, active });
        Ok(())
    }

    fn release_fresh_json_argument_if_active(
        &mut self,
        owned: &FreshJsonArgument<'ctx>,
    ) -> Result<(), String> {
        let active = self.builder.build_load(self.context.i8_type(), owned.active, "typed_wrapper_json_is_active")
            .map_err(|error| error.to_string())?.into_int_value();
        let should_release = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, active, self.context.i8_type().const_zero(),
            "typed_wrapper_json_should_release",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_typed_wrapper_json_argument");
        let done = self.context.append_basic_block(function, "typed_wrapper_json_released");
        self.builder.build_conditional_branch(should_release, release, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[owned.value.into()], "destroy_typed_wrapper_json_argument",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(owned.active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn finish_typed_wrapper_argument_scope_for_constructor(
        &mut self,
        expected_owner: &TypedJsonArgumentOwner,
    ) -> Result<FreshConstructorArguments<'ctx>, String> {
        if !matches!(expected_owner, TypedJsonArgumentOwner::NapiConstructor(_)) {
            return Err("constructor argument scope has the wrong JSON owner kind".into());
        }
        let scope = self.fresh_event_key_argument_scopes.pop()
            .ok_or_else(|| "typed-wrapper argument scope underflow".to_string())?;
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        if scope.function_name != function_name
            || scope.typed_json_owner.as_ref() != Some(expected_owner)
            || !scope.native_handle_ledgers.is_empty()
        {
            return Err("typed-wrapper constructor argument scope ownership mismatch".into());
        }
        let array = scope.argument_array
            .ok_or_else(|| "N-API constructor argument scope has no tracked array".to_string())?;
        Ok((scope.handles, scope.json_values, Some(array)))
    }

    fn with_typed_wrapper_argument_scope<F>(
        &mut self,
        owner: TypedJsonArgumentOwner,
        invoke: F,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, String>
    where
        F: FnOnce(&mut Self) -> Result<inkwell::values::BasicValueEnum<'ctx>, String>,
    {
        self.begin_fresh_event_key_argument_scope_with_owner(Some(owner.clone()));
        let result = invoke(self);
        let scope = self.finish_typed_wrapper_argument_scope(&owner);
        match (result, scope) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(value), Ok((handles, json_values))) => {
                for json in &json_values {
                    self.release_fresh_json_argument_if_active(json)?;
                }
                self.release_fresh_event_key_handles(&handles)?;
                Ok(value)
            }
        }
    }

    fn finish_typed_wrapper_argument_scope(
        &mut self,
        expected_owner: &TypedJsonArgumentOwner,
    ) -> Result<(Vec<FreshEventKeyHandle<'ctx>>, Vec<FreshJsonArgument<'ctx>>), String> {
        let scope = self.fresh_event_key_argument_scopes.pop()
            .ok_or_else(|| "typed-wrapper argument scope underflow".to_string())?;
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        if scope.function_name != function_name
            || scope.typed_json_owner.as_ref() != Some(expected_owner)
            || !scope.native_handle_ledgers.is_empty()
        {
            return Err("typed-wrapper argument scope ownership mismatch".into());
        }
        if let Some(array) = scope.argument_array {
            self.disarm_typed_dynamic_argument_array(array)?;
        }
        Ok((scope.handles, scope.json_values))
    }

    fn release_fresh_event_key_if_active(
        &mut self,
        owned: &FreshEventKeyHandle<'ctx>,
    ) -> Result<(), String> {
        let active = self.builder.build_load(self.context.i8_type(), owned.active, "fresh_event_key_is_active")
            .map_err(|error| error.to_string())?.into_int_value();
        let should_release = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, active, self.context.i8_type().const_zero(),
            "fresh_event_key_should_release",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_fresh_event_key");
        let done = self.context.append_basic_block(function, "fresh_event_key_released");
        self.builder.build_conditional_branch(should_release, release, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(),
            &[owned.handle.into()], "release_transferred_event_key",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(owned.active, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    pub(super) fn begin_live_native_projection_scope(
        &mut self,
        ledger: NativeHandleLedger<'ctx>,
    ) {
        let catch_depth = self.catch_stack.len();
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        self.fresh_event_key_argument_scopes.push(FreshEventKeyArgumentScope {
            catch_depth,
            function_name,
            typed_json_owner: None,
            handles: Vec::new(),
            json_values: Vec::new(),
            argument_array: None,
            native_handle_ledgers: vec![ledger],
        });
    }

    pub(super) fn finish_live_native_projection_scope(&mut self) -> Result<(), String> {
        let scope = self.fresh_event_key_argument_scopes.pop()
            .ok_or_else(|| "live native projection scope underflow".to_string())?;
        if !scope.handles.is_empty() || scope.native_handle_ledgers.len() != 1 {
            return Err("live native projection scope ownership mismatch".into());
        }
        Ok(())
    }

    pub(super) fn release_native_handle_ledger(
        &mut self,
        ledger: NativeHandleLedger<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let buffer = self.builder.build_load(ptr_ty, ledger.buffer, "native_handle_ledger_buffer")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let present = self.builder.build_is_not_null(buffer, "native_handle_ledger_present")
            .map_err(|error| error.to_string())?;
        let release = self.context.append_basic_block(self.current_function(), "release_native_handle_ledger");
        let done = self.context.append_basic_block(self.current_function(), "native_handle_ledger_released");
        self.builder.build_conditional_branch(present, release, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(
            self.module.get_function("thaw_js_release_native_handle_array").unwrap(),
            &[buffer.into()], "release_native_projection_handles",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(ledger.buffer, ptr_ty.const_null())
            .map_err(|error| error.to_string())?;
        self.builder.build_store(ledger.capacity, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    pub(super) fn release_live_native_projection_ledgers_before_throw(&mut self) -> Result<(), String> {
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        let ledgers = self.fresh_event_key_argument_scopes.iter()
            .filter(|scope| scope.function_name == function_name)
            .flat_map(|scope| scope.native_handle_ledgers.iter().copied())
            .collect::<Vec<_>>();
        for ledger in ledgers {
            self.release_native_handle_ledger(ledger)?;
        }
        Ok(())
    }

    fn emit_fresh_event_key_cleanup_before_catch(
        &mut self,
        catch_index: Option<usize>,
    ) -> Result<(), String> {
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        let owned = self.fresh_event_key_argument_scopes.iter()
            .filter(|scope| scope.function_name == function_name)
            .filter(|scope| catch_index.is_none_or(|index| scope.catch_depth > index))
            .flat_map(|scope| scope.handles.iter().rev().cloned())
            .collect::<Vec<_>>();
        for handle in &owned {
            self.release_fresh_event_key_if_active(handle)?;
        }
        let json_values = self.fresh_event_key_argument_scopes.iter()
            .filter(|scope| scope.function_name == function_name)
            .filter(|scope| catch_index.is_none_or(|index| scope.catch_depth > index))
            .flat_map(|scope| scope.json_values.iter().rev().copied())
            .collect::<Vec<_>>();
        for value in &json_values {
            self.release_fresh_json_argument_if_active(value)?;
        }
        let arrays = self.fresh_event_key_argument_scopes.iter().rev()
            .filter(|scope| scope.function_name == function_name)
            .filter(|scope| catch_index.is_none_or(|index| scope.catch_depth > index))
            .filter_map(|scope| scope.argument_array)
            .collect::<Vec<_>>();
        for array in arrays {
            self.release_typed_dynamic_argument_array_if_active(array)?;
        }
        let ledgers = self.fresh_event_key_argument_scopes.iter()
            .filter(|scope| scope.function_name == function_name)
            .filter(|scope| catch_index.is_none_or(|index| scope.catch_depth > index))
            .flat_map(|scope| scope.native_handle_ledgers.iter().copied())
            .collect::<Vec<_>>();
        for ledger in ledgers {
            self.release_native_handle_ledger(ledger)?;
        }
        Ok(())
    }

    fn release_fresh_event_key_handles(
        &mut self,
        handles: &[FreshEventKeyHandle<'ctx>],
    ) -> Result<(), String> {
        for handle in handles {
            self.release_fresh_event_key_if_active(handle)?;
        }
        Ok(())
    }

    fn fresh_event_key_raw_handles(
        handles: &[FreshEventKeyHandle<'ctx>],
    ) -> Vec<inkwell::values::IntValue<'ctx>> {
        handles.iter().map(|owned| owned.handle).collect()
    }

    fn compile_typed_dynamic_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if signature.backend == DynamicBackend::ThawJs {
            return self.compile_typed_thaw_js_call(signature, args);
        }
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
        if signature.backend == DynamicBackend::QuickJs
            && signature.symbol.starts_with("$setter$")
        {
            return self.compile_typed_quickjs_setter(signature, args);
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
            let dynamic_object_query = if signature.params.contains(&HirType::JsValue) {
                self.module
                    .get_function("thaw_js_dynamic_object_query")
                    .unwrap()
                    .as_global_value()
                    .as_pointer_value()
            } else {
                self.context.ptr_type(Default::default()).const_null()
            };
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
                        dynamic_object_query.into(),
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
            self.clear_pending_native_text()?;
            self.mark_pending_native_text(error)?;
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
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                // A JIT array member's payload is a tagged raw buffer
                // (`buffer | 1`) or a handle cell; app array values are
                // `{buffer, presence}` handles, so normalize it to a
                // fresh handle whenever the runtime tag selects an array
                // member.
                let array_tags = elements
                    .iter()
                    .filter(|ty| matches!(ty, HirType::Array(_)))
                    .map(|ty| jit_union_member_tag(ty).unwrap())
                    .collect::<Vec<_>>();
                let payload = if array_tags.is_empty() {
                    payload
                } else {
                    let mut selected = self.context.bool_type().const_zero();
                    for runtime in array_tags {
                        let matches = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                runtime_tag,
                                self.context.i64_type().const_int(runtime, false),
                                "jit_union_array_selected",
                            )
                            .map_err(|error| error.to_string())?;
                        selected = self
                            .builder
                            .build_or(selected, matches, "jit_union_any_array_selected")
                            .map_err(|error| error.to_string())?;
                    }
                    let function = self.current_function();
                    let array_block = self.context.append_basic_block(function, "jit_union_array");
                    let scalar_block = self.context.append_basic_block(function, "jit_union_scalar");
                    let merge_block = self.context.append_basic_block(function, "jit_union_payload_end");
                    self.builder
                        .build_conditional_branch(selected, array_block, scalar_block)
                        .map_err(|error| error.to_string())?;

                    self.builder.position_at_end(array_block);
                    let normalized = self.compile_jit_array_result(payload)?;
                    let normalized = self
                        .builder
                        .build_ptr_to_int(
                            normalized,
                            self.context.i64_type(),
                            "jit_union_array_handle",
                        )
                        .map_err(|error| error.to_string())?;
                    let array_end = self.builder.get_insert_block().unwrap();
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .map_err(|error| error.to_string())?;

                    self.builder.position_at_end(scalar_block);
                    let scalar_end = self.builder.get_insert_block().unwrap();
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .map_err(|error| error.to_string())?;

                    self.builder.position_at_end(merge_block);
                    let merged = self
                        .builder
                        .build_phi(self.context.i64_type(), "jit_union_payload")
                        .map_err(|error| error.to_string())?;
                    merged.add_incoming(&[(&normalized, array_end), (&payload, scalar_end)]);
                    merged.as_basic_value().into_int_value()
                };
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
                if matches!(return_type, HirType::Array(_)) {
                    self.compile_jit_array_result(bits)?.into()
                } else {
                    self.builder
                        .build_int_to_ptr(
                            bits,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "jit_string_value",
                        )
                        .map_err(|error| error.to_string())?
                        .into()
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
                HirType::Function(params, ret) => {
                    Some((index, params.clone(), ret.as_ref().clone(), false, None))
                }
                HirType::CallableFunction(params, _, rest, ret) => {
                    let mut abi_params = params.clone();
                    let rest_start = rest.as_ref().map(|_| abi_params.len());
                    if let Some(rest) = rest {
                        abi_params.push(HirType::Array(rest.clone()));
                    }
                    Some((index, abi_params, ret.as_ref().clone(), false, rest_start))
                }
                HirType::Optional(inner) => match inner.as_ref() {
                    HirType::Function(params, ret) => {
                        Some((index, params.clone(), ret.as_ref().clone(), true, None))
                    }
                    HirType::CallableFunction(params, _, rest, ret) => {
                        let mut abi_params = params.clone();
                        let rest_start = rest.as_ref().map(|_| abi_params.len());
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        Some((index, abi_params, ret.as_ref().clone(), true, rest_start))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
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
        let mut returned_bytes_argument = None;
        let mut callback_values = Vec::with_capacity(function_argument.len());
        let typed_json_owner = if ordinary_quickjs_typed_call(signature) {
            Some(TypedJsonArgumentOwner::DirectQuickJs(signature.symbol.clone()))
        } else if signature.backend == DynamicBackend::Napi
            && signature.ret == HirType::JsValue
            && napi_constructor_export_name(&signature.symbol).is_some()
        {
            Some(TypedJsonArgumentOwner::NapiConstructor(signature.symbol.clone()))
        } else {
            None
        };
        self.begin_fresh_event_key_argument_scope_with_owner(typed_json_owner.clone());
        self.track_typed_dynamic_argument_array(array)?;
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            if signature.backend == DynamicBackend::Napi
                && function_argument
                    .iter()
                    .any(|(function_index, ..)| *function_index == index)
            {
                callback_values.push(self.compile_expr(arg)?);
                continue;
            }
            if signature.backend == DynamicBackend::QuickJs && quickjs_callback_type(ty) {
                self.compile_quickjs_callback_argument(array, arg)?;
                continue;
            }
            let (value, marshalled_type) = {
                // A live `JsValue` handle (e.g. an Observable passed as an
                // `any`/`Json`-declared package parameter) must cross as
                // the `{"__thaw_js_handle_id__": N}` placeholder, not a
                // raw `i64` in a JSON slot. `compile_typed_dynamic_
                // argument`'s fall-through (`compile_json_array_push_
                // native`) has no `JsValue` arm and would otherwise push
                // the bare handle id as a number -- a real "invalid or
                // released dynamic value handle N" on the far side. An
                // `i64` where a `Json` pointer is expected is the same
                // unambiguous marker `compile_json_object_set_native_with_
                // undefined` already checks.
                let value = self.compile_expr(arg)?;
                if value.is_int_value() && matches!(ty, HirType::Json | HirType::Dynamic) {
                    (
                        self.compile_dynamic_value_placeholder_unchecked(value)?,
                        &HirType::Json,
                    )
                } else {
                    (value, ty)
                }
            };
            if index == 0
                && *ty == HirType::Bytes
                && signature.ret == HirType::Bytes
                && signature.symbol.ends_with("randomFillSync")
            {
                returned_bytes_argument = Some(value.into_pointer_value());
            }
            self.compile_typed_dynamic_argument(array, value, marshalled_type)
                .map_err(|error| {
                    format!("typed dynamic argument {} ({ty:?}): {error}", index + 1)
                })?;
        }
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let preserves_array_for_constructor_lookup =
            signature.backend == DynamicBackend::Napi
                && signature.ret == HirType::JsValue
                && napi_constructor_export_name(&signature.symbol).is_some();
        let (fresh_argument_handles, fresh_json_arguments, fresh_argument_array) =
            if preserves_array_for_constructor_lookup {
                let owner = typed_json_owner.as_ref()
                    .ok_or_else(|| "N-API constructor has no typed JSON owner".to_string())?;
                self.finish_typed_wrapper_argument_scope_for_constructor(owner)?
            } else if let Some(owner) = typed_json_owner.as_ref() {
                let (handles, json_values) = self.finish_typed_wrapper_argument_scope(owner)?;
                (handles, json_values, None)
            } else {
                (self.finish_fresh_event_key_argument_scope()?, Vec::new(), None)
            };
        // The argument array now owns each cloned Host lease independently.
        // Release the marked source boxes for every result type. Partial
        // argument failures still unwind through the active owner scope above.
        for argument in &fresh_json_arguments {
            self.release_fresh_json_argument_if_active(argument)?;
        }
        let name = self
            .builder
            .build_global_string_ptr(&signature.symbol, "dynamic_symbol")
            .map_err(|error| error.to_string())?;
        if signature.backend == DynamicBackend::Napi && signature.ret == HirType::JsValue {
            let Some(constructor_name) = napi_constructor_export_name(&signature.symbol) else {
                let args_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_graph_encode").unwrap(),
                        &[array.into()],
                        "napi_handle_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let raw_fresh_handles = Self::fresh_event_key_raw_handles(&fresh_argument_handles);
                let args_json = self.compile_check_json_stringify_error_with_inputs_and_handles(
                    args_json, &[array], &[], &[], &raw_fresh_handles,
                )?;
                self.release_fresh_event_key_handles(&fresh_argument_handles)?;
                let result = if !function_argument.is_empty() {
                    let functions =
                        self.compile_napi_function_arguments(&callback_values, &function_argument)?;
                    self.builder.build_call(
                        self.module
                            .get_function(
                                "thaw_napi_call_export_handle_with_functions_typed_graph_args_result",
                            )
                            .unwrap(),
                        &[
                            name.as_pointer_value().into(),
                            args_json.into(),
                            functions.into(),
                            self.context
                                .i64_type()
                                .const_int(function_argument.len() as u64, false)
                                .into(),
                        ],
                        "napi_export_function_handle_result",
                    )
                } else {
                    self.builder.build_call(
                        self.module
                            .get_function("thaw_napi_call_export_handle_typed_graph_args_result")
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
                self.destroy_typed_host_arguments(array, args_json)?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.clear_pending_native_text()?;
                self.mark_pending_native_text(error)?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            };
            let constructor_name = self
                .builder
                .build_global_string_ptr(constructor_name, "napi_constructor_name")
                .map_err(|error| error.to_string())?;
            // Looking up a missing N-API constructor can set a pending
            // exception before argument graph encoding starts. Keep fresh
            // scalar handles and the argument array visible to that exception
            // edge; normal graph-error cleanup owns the array after lookup.
            self.begin_fresh_event_key_transfer_scope(&fresh_argument_handles, fresh_argument_array);
            let constructor = self.compile_napi_export_handle(constructor_name.as_pointer_value())?;
            let _ = self.finish_fresh_event_key_argument_scope()?;
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_graph_encode").unwrap(),
                    &[array.into()],
                    "napi_constructor_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let raw_fresh_handles = Self::fresh_event_key_raw_handles(&fresh_argument_handles);
            let args_json = self.compile_check_json_stringify_error_with_inputs_and_handles(
                args_json, &[array], &[], &[], &raw_fresh_handles,
            )?;
            self.release_fresh_event_key_handles(&fresh_argument_handles)?;
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_construct_handle_typed_graph_args_result")
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
            self.destroy_typed_host_arguments(array, args_json)?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.clear_pending_native_text()?;
            self.mark_pending_native_text(error)?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::QuickJs && signature.ret == HirType::JsValue {
            // The constructor route below predates the exact-call helper and needs
            // QuickJS handle support even though it does not enter that helper.
            self.uses_quickjs_handles = true;
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
                        self.module.get_function("thaw_json_graph_encode").unwrap(),
                        &[array.into()],
                        "typed_dynamic_construct_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let raw_fresh_handles = Self::fresh_event_key_raw_handles(&fresh_argument_handles);
                let args_json = self.compile_check_json_stringify_error_with_inputs_and_handles(
                    args_json, &[array], &[], &[], &raw_fresh_handles,
                )?;
                self.release_fresh_event_key_handles(&fresh_argument_handles)?;
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_construct_handle_graph_args_result")
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
                self.destroy_typed_host_arguments(array, args_json)?;
                self.builder.build_call(
                    self.module.get_function("thaw_js_release_handle").unwrap(),
                    &[constructor.into()],
                    "release_typed_dynamic_constructor",
                ).map_err(|error| error.to_string())?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.clear_pending_native_text()?;
                self.mark_pending_native_text(error)?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            }
            let value = self.compile_typed_quickjs_exact_call(
                name.as_pointer_value(), array, false, &fresh_argument_handles,
            )?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::Napi && !function_argument.is_empty() {
            let functions = self.compile_napi_function_arguments(&callback_values, &function_argument)?;
            let count = self
                .context
                .i64_type()
                .const_int(function_argument.len() as u64, false);
            let json = self.compile_json_backend_values_with_extra(
                name.as_pointer_value().into(),
                array,
                "thaw_napi_call_with_functions_typed_graph_result",
                &[functions.into(), count.into()],
                &fresh_argument_handles,
            )?;
            return self.compile_typed_dynamic_result(json, &signature.ret);
        }
        let result = if signature.backend == DynamicBackend::QuickJs {
            let handle = self.compile_typed_quickjs_exact_call(
                name.as_pointer_value(), array, true, &fresh_argument_handles,
            )?;
            let json = self.compile_original_quickjs_exception_handle(handle, true)?;
            self.compile_typed_dynamic_result(json, &signature.ret)?
        } else {
            let backend = match signature.backend {
                DynamicBackend::Jit => unreachable!("JIT calls return before JSON marshalling"),
                DynamicBackend::QuickJs => unreachable!("QuickJS calls use exact exception transport"),
                DynamicBackend::ThawJs => unreachable!("thaw-js calls return before this match"),
                DynamicBackend::Napi => "thaw_napi_call_typed_graph_result",
            };
            let json = self.compile_json_backend_values_with_extra(
                name.as_pointer_value().into(), array, backend, &[], &fresh_argument_handles,
            )?;
            self.compile_typed_dynamic_result(json, &signature.ret)?
        };
        if let Some(argument) = returned_bytes_argument {
            self.builder
                .build_call(
                    self.module.get_function("thaw_bytes_copy").unwrap(),
                    &[
                        result.into_pointer_value().into(),
                        argument.into(),
                        self.context.f64_type().const_zero().into(),
                        self.context.f64_type().const_zero().into(),
                        self.context.f64_type().const_float(-1.0).into(),
                    ],
                    "sync_returned_bytes_argument",
                )
                .map_err(|error| error.to_string())?;
            return Ok(argument.into());
        }
        Ok(result)
    }

    /// `DynamicBackend::ThawJs`: plain JSON text both ways through `thaw_tjs_call_typed_result`
    /// (`"<package>::<export>"`, JSON argument array). Only plain-data signatures are accepted
    /// (see `thaw_js_plain_type`); the shim generator routes nothing else here.
    fn compile_typed_thaw_js_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if signature.symbol.starts_with('$') {
            return self.compile_typed_thaw_js_class_call(signature, args);
        }
        self.uses_thaw_js = true;
        if let Some(bad) = signature
            .params
            .iter()
            .find(|ty| !thaw_js_param_type(ty))
            .or_else(|| {
                (!thaw_js_plain_type(&signature.ret) && signature.ret != HirType::JsValue)
                    .then_some(&signature.ret)
            })
        {
            self.compile_thaw_js_unsupported_stub(&format!(
                "\u{1}thaw-js: `{}` has the non-plain-data type {bad:?}, which cannot cross the thaw-js boundary",
                signature.symbol
            ))?;
            return Ok(self.basic_type(&signature.ret)?.const_zero());
        }
        let array = self
            .builder
            .build_call(self.module.get_function("thaw_json_array_new").unwrap(), &[], "thaw_js_args")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            self.thaw_js_call_arguments = true;
            let value = self.compile_expr(arg);
            self.thaw_js_call_arguments = false;
            let value = value?;
            if value.is_int_value() && matches!(ty, HirType::Json) {
                return Err(format!(
                    "thaw-js call `{}`: argument {} is a live JsValue handle",
                    signature.symbol,
                    index + 1
                ));
            }
            if thaw_js_callback_type(ty) {
                self.compile_thaw_js_callback_argument(array, value, ty)
                    .map_err(|error| format!("thaw-js callback argument {}: {error}", index + 1))?;
                continue;
            }
            if *ty == HirType::JsValue {
                let marker = self.compile_thaw_js_handle_marker(value.into_int_value())?;
                self.compile_json_array_push_owned(array, marker)?;
                continue;
            }
            self.compile_typed_dynamic_argument(array, value, ty)
                .map_err(|error| format!("thaw-js argument {} ({ty:?}): {error}", index + 1))?;
        }
        // `\u{2}` asks for the result as a handle id (a `JsValue` result), whatever its kind;
        // `\u{3}` for a `Date` result as its `{"__thaw_tjs_date__": ms}` wire node.
        let symbol = if signature.ret == HirType::JsValue {
            format!("\u{2}{}", signature.symbol)
        } else if thaw_js_date_type(&signature.ret) {
            format!("\u{3}{}", signature.symbol)
        } else {
            signature.symbol.clone()
        };
        let name = self
            .builder
            .build_global_string_ptr(&symbol, "thaw_js_symbol")
            .map_err(|error| error.to_string())?;
        let mut json = self.compile_json_backend_values_with_extra(
            name.as_pointer_value().into(),
            array,
            "thaw_tjs_call_typed_result",
            &[],
            &[],
        )?;
        if signature.ret == HirType::JsValue {
            let id = self.compile_json_value_to_native(json, &HirType::F64)?.into_float_value();
            return Ok(self
                .builder
                .build_float_to_signed_int(id, self.context.i64_type(), "thaw_js_result_handle")
                .map_err(|error| error.to_string())?
                .into());
        }
        if let HirType::Object(fields) = &signature.ret {
            if thaw_js_date_type(&signature.ret) {
                // `{"__thaw_tjs_date__": ms}` -> a native Date.
                self.compile_check_json_host_error(json, None)?;
                let key = self
                    .builder
                    .build_global_string_ptr("__thaw_tjs_date__", "thaw_js_date_key")
                    .map_err(|error| error.to_string())?;
                let ms = self.builder.build_call(
                    self.module.get_function("thaw_json_get").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "thaw_js_date_ms_json",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("thaw_json_get returned no value")?;
                self.compile_check_json_host_error(ms, Some("thaw_json_destroy"))?;
                let ms = self.compile_json_as_value(ms, "thaw_json_as_number")?.into_float_value();
                return self.compile_native_date_from_timestamp(fields, ms);
            }
        }
        if signature.ret == HirType::Json {
            // A class instance or function result comes back as a handle marker: adopt it as a host lease.
            self.uses_thaw_js_handles = true;
            json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_adopt_handle_marker").unwrap(),
                    &[json.into()],
                    "thaw_js_adopted_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_adopt_handle_marker returned no value")?;
        }
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    /// `thaw_tjs_frame_error(error)` when a thaw-js call returned an error string (null stays null).
    fn compile_thaw_js_frame_error(
        &mut self,
        error: inkwell::values::PointerValue<'ctx>,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let function = self.current_function();
        let before = self.builder.get_insert_block().ok_or("no insertion block")?;
        let failed = self.context.append_basic_block(function, "thaw_js_error_present");
        let done = self.context.append_basic_block(function, "thaw_js_error_checked");
        let present = self
            .builder
            .build_is_not_null(error, "thaw_js_error_is_present")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_conditional_branch(present, failed, done)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(failed);
        let framed = self
            .builder
            .build_call(
                self.module.get_function("thaw_tjs_frame_error").unwrap(),
                &[error.into()],
                "thaw_js_framed_error",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_tjs_frame_error returned no value")?
            .into_pointer_value();
        let failed_end = self.builder.get_insert_block().ok_or("no insertion block")?;
        self.builder.build_unconditional_branch(done).map_err(|e| e.to_string())?;
        self.builder.position_at_end(done);
        let merged = self
            .builder
            .build_phi(error.get_type(), "thaw_js_error")
            .map_err(|e| e.to_string())?;
        merged.add_incoming(&[(&error, before), (&framed, failed_end)]);
        Ok(merged.as_basic_value().into_pointer_value())
    }

    /// Calling an unsupported thaw-js signature throws instead of failing the build (every generated
    /// wrapper is compiled, called or not).
    fn compile_thaw_js_unsupported_stub(&mut self, message: &str) -> Result<(), String> {
        let name = self
            .builder
            .build_global_string_ptr(message, "thaw_js_unsupported")
            .map_err(|error| error.to_string())?;
        let empty = self
            .builder
            .build_call(self.module.get_function("thaw_json_array_new").unwrap(), &[], "thaw_js_no_args")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.compile_json_backend_values_with_extra(
            name.as_pointer_value().into(),
            empty,
            "thaw_tjs_call_typed_result",
            &[],
            &[],
        )?;
        Ok(())
    }

    /// A thaw-js class operation: `new Class(..)`, an instance method or a static method (symbols
    /// `$new$..`, `$method$..`, `$staticmethod$..`). Instances are `JsValue` handles into the thaw-js
    /// heap; arguments and results cross as JSON, with a handle marker for instances.
    fn compile_typed_thaw_js_class_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_thaw_js = true;
        let symbol = signature.symbol.as_str();
        let is_value = symbol.starts_with("$value$");
        let is_new = symbol.starts_with("$new$") || is_value;
        let is_static = symbol.starts_with("$staticmethod$") || symbol.starts_with("$staticmethodvoid$");
        let is_method = symbol.starts_with("$method$") || symbol.starts_with("$methodvoid$");
        let handle_param = |ty: &HirType| *ty == HirType::JsValue;
        let ret_handle = signature.ret == HirType::JsValue;
        let supported = (is_new || is_static || is_method)
            && (ret_handle || (!is_new && thaw_js_plain_type(&signature.ret)))
            && signature.params.iter().skip(usize::from(is_method)).all(|ty| thaw_js_plain_type(ty) || handle_param(ty))
            && (!is_method || signature.params.first().is_some_and(handle_param))
            && args.len() == signature.params.len();
        if !supported {
            // Class operations are only emitted where the program uses them (unlike the wrappers of
            // plain functions), so an unsupported one is a build error the CLI answers by rebuilding
            // the program on QuickJS.
            return Err(format!(
                "thaw-js: class operation `{}` has a signature that cannot cross the thaw-js boundary",
                signature.symbol
            ));
        }
        let (receiver_expr, rest) = if is_method { (Some(&args[0]), &args[1..]) } else { (None, args) };
        let receiver = match receiver_expr {
            Some(expr) => self.compile_expr(expr)?.into_int_value(),
            None => self.context.i64_type().const_zero(),
        };
        let array = self
            .builder
            .build_call(self.module.get_function("thaw_json_array_new").unwrap(), &[], "thaw_js_class_args")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        for (index, (arg, ty)) in rest.iter().zip(signature.params.iter().skip(usize::from(is_method))).enumerate() {
            self.thaw_js_call_arguments = true;
            let value = self.compile_expr(arg);
            self.thaw_js_call_arguments = false;
            let value = value?;
            if handle_param(ty) {
                let marker = self.compile_thaw_js_handle_marker(value.into_int_value())?;
                self.compile_json_array_push_owned(array, marker)?;
                continue;
            }
            if value.is_int_value() && matches!(ty, HirType::Json) {
                return Err(format!(
                    "thaw-js class call `{}`: argument {} is a live JsValue handle",
                    signature.symbol,
                    index + 1
                ));
            }
            self.compile_typed_dynamic_argument(array, value, ty)
                .map_err(|error| format!("thaw-js class argument {} ({ty:?}): {error}", index + 1))?;
        }
        let name = self
            .builder
            .build_global_string_ptr(symbol, "thaw_js_class_symbol")
            .map_err(|error| error.to_string())?;
        let want_handle = self.context.i8_type().const_int(u64::from(ret_handle), false);
        let json = self.compile_json_backend_values_with_extra(
            name.as_pointer_value().into(),
            array,
            "thaw_tjs_class_call",
            &[receiver.into(), want_handle.into()],
            &[],
        )?;
        if ret_handle {
            let id = self.compile_json_value_to_native(json, &HirType::F64)?.into_float_value();
            return Ok(self
                .builder
                .build_float_to_signed_int(id, self.context.i64_type(), "thaw_js_class_handle")
                .map_err(|error| error.to_string())?
                .into());
        }
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    /// `{"__thaw_tjs_handle__": id}` for an instance handle passed back into thaw-js.
    fn compile_thaw_js_handle_marker(
        &mut self,
        handle: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let id = self
            .builder
            .build_signed_int_to_float(handle, self.context.f64_type(), "thaw_js_handle_id")
            .map_err(|error| error.to_string())?;
        let marker = self
            .builder
            .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_handle_marker")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self
            .builder
            .build_global_string_ptr("__thaw_tjs_handle__", "thaw_js_handle_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_object_set_number").unwrap(),
                &[marker.into(), key.as_pointer_value().into(), id.into()],
                "thaw_js_handle_marker_id",
            )
            .map_err(|error| error.to_string())?;
        Ok(marker)
    }

    /// Registers a compiled closure with thaw-js (`thaw_tjs_register_callback`) and pushes the
    /// `{"__thaw_tjs_cb__": id}` marker thaw-js turns into a callable function. An absent optional
    /// callback becomes the `undefined` marker.
    fn compile_thaw_js_callback_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(), String> {
        let (optional, function_type) = match ty {
            HirType::Optional(inner) => (true, inner.as_ref()),
            other => (false, other),
        };
        let HirType::Function(params, ret) = function_type else {
            return Err("not a function type".into());
        };
        let (closure, present) = if optional {
            let tagged = value.into_struct_value();
            let present = self.builder.build_extract_value(tagged, 0, "thaw_js_callback_present")
                .map_err(|error| error.to_string())?.into_int_value();
            let closure = self.builder.build_extract_value(tagged, 1, "thaw_js_callback_closure")
                .map_err(|error| error.to_string())?.into_pointer_value();
            (closure, Some(present))
        } else {
            (value.into_pointer_value(), None)
        };
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "thaw_js_callback_done");
        let absent_block = present.map(|present| {
            let register = self.context.append_basic_block(function, "thaw_js_callback_register");
            let absent = self.context.append_basic_block(function, "thaw_js_callback_absent");
            (present, register, absent)
        });
        if let Some((present, register, absent)) = absent_block {
            self.builder.build_conditional_branch(present, register, absent)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(absent);
            let undefined = self.compile_napi_undefined_json()?;
            self.compile_json_array_push_owned(array, undefined)?;
            self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
            self.builder.position_at_end(register);
        }
        let marker = self.compile_thaw_js_callback_marker(closure, params, ret)?;
        self.compile_json_array_push_owned(array, marker)?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Registers the closure with thaw-js and returns the `{"__thaw_tjs_cb__": id}` marker Json.
    fn compile_thaw_js_callback_marker(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_thaw_js = true;
        // JSON text both ways (`graph_wire` off): thaw-js has no live-handle graph protocol.
        let (adapter, context, _) =
            self.compile_value_callback_from_closure(closure, params, ret, false, None, false, false, false)?;
        let i64_type = self.context.i64_type();
        let id = self.builder.build_call(
            self.module.get_function("thaw_tjs_register_callback").unwrap(),
            &[adapter.into(), context.into(), i64_type.const_int(params.len() as u64, false).into()],
            "thaw_js_callback_id",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_tjs_register_callback returned no value")?.into_int_value();
        let id = self.builder.build_signed_int_to_float(id, self.context.f64_type(), "thaw_js_callback_id_f64")
            .map_err(|error| error.to_string())?;
        let marker = self.builder.build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_callback_marker")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self.builder.build_global_string_ptr("__thaw_tjs_cb__", "thaw_js_callback_key")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_object_set_number").unwrap(),
            &[marker.into(), key.as_pointer_value().into(), id.into()],
            "thaw_js_callback_marker_id",
        ).map_err(|error| error.to_string())?;
        Ok(marker)
    }

    /// `JsValueAsJson(registerNativeCallback(closure))` inside a thaw-js call: the coercion of a
    /// plain callback into a `Json` parameter. Anything else keeps the QuickJS meaning.
    fn try_compile_thaw_js_callback_marker(
        &mut self,
        inner: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let HirExpr::Call(callee, args) = inner else { return Ok(None) };
        let (HirExpr::Var(name), [closure_expr @ HirExpr::TypedClosure(HirType::Function(params, ret), _)]) =
            (callee.as_ref(), args.as_slice())
        else {
            return Ok(None);
        };
        if name != "registerNativeCallback"
            || !thaw_js_callback_type(&HirType::Function(params.clone(), ret.clone()))
        {
            return Ok(None);
        }
        let closure = self.compile_expr(closure_expr)?.into_pointer_value();
        let marker = self.compile_thaw_js_callback_marker(closure, params, ret)?;
        self.tracks_owned_json_roots = true;
        self.mark_fresh_owned_json_result(marker)?;
        Ok(Some(marker))
    }

    fn compile_typed_quickjs_exact_call(
        &mut self,
        name: inkwell::values::PointerValue<'ctx>,
        array: BasicValueEnum<'ctx>,
        resolve_success: bool,
        fresh_argument_handles: &[FreshEventKeyHandle<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.js_mark_handles_used();
        if resolve_success {
            // (the thaw-js twin does not await a returned Promise)
            self.uses_quickjs = true;
        }
        self.compile_register_js_callback_host_operations()?;
        let callable = self.builder.build_call(
            self.module.get_function("thaw_js_get_global").unwrap(),
            &[name.into()], "typed_exact_callable",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic().ok_or("QuickJS global lookup returned no handle")?
            .into_int_value();
        let args_json = self.builder.build_call(
            self.module.get_function("thaw_json_graph_encode").unwrap(),
            &[array.into()], "typed_exact_args_json",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic().ok_or("QuickJS argument graph encoding returned no string")?;
        let mut cleanup_handles = vec![callable];
        cleanup_handles.extend(Self::fresh_event_key_raw_handles(fresh_argument_handles));
        let args_json = self.compile_check_json_stringify_error_with_inputs_and_handles(
            args_json, &[array], &[], &[], &cleanup_handles,
        )?;
        self.release_fresh_event_key_handles(fresh_argument_handles)?;
        let i64_type = self.context.i64_type();
        let handles = self.builder.build_alloca(
            i64_type.array_type(1), "typed_exact_empty_handles",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(handles, i64_type.const_zero())
            .map_err(|error| error.to_string())?;
        let function = if resolve_success {
            "thaw_js_call_handle_mixed_exact_resolved_consuming_result"
        } else {
            "thaw_js_call_handle_mixed_exact_consuming_result"
        };
        let result = self.builder.build_call(
            self.module.get_function(function).unwrap(),
            &[callable.into(), args_json.into(), handles.into()],
            "typed_exact_call_result",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic().ok_or("exact QuickJS call returned no result")?
            .into_struct_value();
        self.destroy_typed_host_arguments(array, args_json)?;
        self.compile_exact_iterator_handle_result(result, "typed exact call")
    }

}
