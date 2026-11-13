/// Types that cross the thaw-js JSON boundary without losing identity or behavior.
pub(crate) fn thaw_js_plain_type(ty: &HirType) -> bool {
    match ty {
        HirType::F64
        | HirType::Bool
        | HirType::Str
        | HirType::Json
        | HirType::Null
        | HirType::Undefined
        | HirType::Void
        | HirType::Bytes => true,
        HirType::Array(element) | HirType::Dictionary(element) => thaw_js_plain_type(element),
        HirType::Union(members) => members.iter().all(thaw_js_plain_type),
        HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
            thaw_js_plain_type(inner)
        }
        HirType::Object(fields) => fields.iter().all(|(_, ty)| thaw_js_plain_type(ty)),
        _ => false,
    }
}

/// Result types that cross as plain JSON although `thaw_js_plain_type` is stricter: tuples of plain
/// members, and unions whose `Promise<T>` members are already-resolved plain `T`. `Dynamic` is not
/// included: the result decoder has no `Dynamic` arm, so it would fail the build instead of stubbing.
pub(crate) fn thaw_js_result_type(ty: &HirType) -> bool {
    match ty {
        HirType::Tuple(members) => members.iter().all(thaw_js_result_type),
        HirType::Union(members) => members.iter().all(|m| match m {
            HirType::Promise(inner) => thaw_js_plain_type(inner),
            other => thaw_js_result_type(other),
        }),
        HirType::Array(e) | HirType::Dictionary(e) | HirType::Optional(e) | HirType::Nullable(e)
        | HirType::Nullish(e) => thaw_js_result_type(e),
        other => thaw_js_plain_type(other),
    }
}

/// The type a thaw-js call yields once awaited: `Promise<T>` is awaited on the JS side and returned as
/// a resolved native Promise of `T`, so only `T` has to cross the boundary.
fn thaw_js_awaited_type(ty: &HirType) -> &HirType {
    match ty {
        HirType::Promise(inner) => inner,
        _ => ty,
    }
}

/// Promise<T> results the thaw-js side may hand back as a pending marker (`\u{4}` calls).
pub(crate) fn thaw_js_pending_ok(awaited: &HirType) -> bool {
    matches!(awaited, HirType::Void | HirType::Str | HirType::F64 | HirType::Bool | HirType::Json)
        || matches!(awaited, HirType::Array(inner) if thaw_js_plain_type(inner))
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
        HirType::Optional(inner) => {
            matches!(inner.as_ref(), HirType::Function(..) | HirType::CallableFunction(..))
                && thaw_js_callback_type(inner)
        }
        _ => thaw_js_callback_abi(ty).is_some_and(|(params, ret, _)| {
            params.iter().all(|p| thaw_js_plain_type(p) && !thaw_js_contains_bytes(p))
                && thaw_js_plain_type(&ret)
                && !thaw_js_contains_bytes(&ret)
        }),
    }
}

/// A callback whose parameters may also be live values: `JsValue` leaves and nested callbacks cross as handle
/// leases, so it can only be registered on the graph route (`thaw_tjs_register_native_callback_graph`), never as
/// a JSON-text `__thaw_tjs_cb__` marker. Superset of `thaw_js_callback_type`.
pub(crate) fn thaw_js_graph_callback_type(ty: &HirType) -> bool {
    match ty {
        HirType::Optional(inner) => {
            matches!(inner.as_ref(), HirType::Function(..) | HirType::CallableFunction(..))
                && thaw_js_graph_callback_type(inner)
        }
        _ => thaw_js_callback_abi(ty).is_some_and(|(params, ret, _)| {
            params.iter().all(|p| {
                (thaw_js_param_leaf_type(p) && !thaw_js_contains_bytes(p)) || thaw_js_ctor_nested_callback(p)
            }) && thaw_js_plain_type(&ret)
                && !thaw_js_contains_bytes(&ret)
        }),
    }
}

/// A constructor option object whose callbacks are receiver handles (`new Writable({ write })`): every
/// field is plain data or such a callback, and at least one field is a callback. It crosses as the
/// live object literal of `compile_thaw_js_object_literal_argument`, never as a typed struct.
pub(crate) fn thaw_js_object_with_callbacks_type(ty: &HirType) -> bool {
    let ty = match ty {
        HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => inner.as_ref(),
        other => other,
    };
    match ty {
        HirType::Object(fields) => {
            fields.iter().all(|(_, field)| thaw_js_param_leaf_type(field) || thaw_js_ctor_option_callback(field))
                && fields.iter().any(|(_, field)| thaw_js_ctor_option_callback(field))
        }
        _ => false,
    }
}

/// A callback field of a constructor option object: a function (optionally absent) whose parameters are
/// plain data (JsValue leaves allowed) or a nested callback, returning plain data or nothing.
fn thaw_js_ctor_option_callback(ty: &HirType) -> bool {
    let ty = match ty {
        HirType::Optional(inner) => inner.as_ref(),
        other => other,
    };
    let (HirType::Function(params, ret) | HirType::CallableFunction(params, _, None, ret)) = ty else {
        return false;
    };
    params.iter().all(|param| {
        thaw_js_param_leaf_type(param) || thaw_js_ctor_nested_callback(param)
    }) && (**ret == HirType::Void || (thaw_js_param_leaf_type(ret) && !thaw_js_contains_bytes(ret)))
}

/// A nested callback parameter (`cb` of `write(chunk, enc, cb)`): no-argument-or-plain params, no result.
fn thaw_js_ctor_nested_callback(ty: &HirType) -> bool {
    matches!(ty, HirType::Function(params, ret) | HirType::CallableFunction(params, _, None, ret)
        if **ret == HirType::Void && params.iter().all(thaw_js_param_leaf_type))
}

/// Whether `ty` holds a `Bytes` anywhere. Only a bare `Bytes` argument is encoded by
/// `compile_thaw_js_bytes_argument`; a nested one would go through the generic JSON encoder as a
/// `{"type":"Buffer"}` object, which thaw-js would hand over as a plain object.
pub(crate) fn thaw_js_contains_bytes(ty: &HirType) -> bool {
    match ty {
        HirType::Bytes => true,
        HirType::Array(inner)
        | HirType::Dictionary(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => thaw_js_contains_bytes(inner),
        HirType::Union(members) => members.iter().any(thaw_js_contains_bytes),
        HirType::Object(fields) => fields.iter().any(|(_, ty)| thaw_js_contains_bytes(ty)),
        _ => false,
    }
}

/// The physical callback ABI of a function type: its parameters (a rest parameter is one trailing
/// array), the result and the index of that rest array.
fn thaw_js_callback_abi(ty: &HirType) -> Option<(Vec<HirType>, HirType, Option<usize>)> {
    match ty {
        HirType::Function(params, ret) => Some((params.clone(), ret.as_ref().clone(), None)),
        HirType::CallableFunction(params, _, rest, ret) => {
            let mut abi = params.clone();
            let rest_start = rest.as_ref().map(|_| abi.len());
            if let Some(rest) = rest {
                abi.push(HirType::Array(rest.clone()));
            }
            Some((abi, ret.as_ref().clone(), rest_start))
        }
        _ => None,
    }
}

/// Like `thaw_js_plain_type`, but a `JsValue` may sit at any leaf (zod's `z.object({ name: z.string() })`
/// has a live schema in each field): it crosses as a handle marker inside the plain container.
fn thaw_js_param_leaf_type(ty: &HirType) -> bool {
    match ty {
        HirType::JsValue => true,
        HirType::Array(element) | HirType::Dictionary(element) => thaw_js_param_leaf_type(element),
        HirType::Union(members) => members.iter().all(thaw_js_param_leaf_type),
        HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
            thaw_js_param_leaf_type(inner)
        }
        HirType::Object(fields) => fields.iter().all(|(_, ty)| thaw_js_param_leaf_type(ty)),
        // an unresolved generic (`spec?: S`) is passed as a dynamic handle, like JsValue
        HirType::Dynamic => true,
        _ => thaw_js_plain_type(ty),
    }
}

/// A parameter type that can cross into thaw-js: plain data (JsValue leaves allowed) or a plain callback.
pub(crate) fn thaw_js_param_type(ty: &HirType) -> bool {
    thaw_js_param_leaf_type(ty) || thaw_js_callback_type(ty)
}

/// The object literal under a `JsValue` thaw-js argument: `JsonAsNative(literal, JsValue)` is the
/// coercion `coerce_to_declared` adds to a `Json`-typed literal, and its codegen would turn the
/// literal into a handle id (0 for a members object). Returns the literal call, which the argument
/// is sent as plain Json instead. `as any` lowers to the literal itself (no wrapper), so nothing
/// else is peeled.
pub(crate) fn thaw_js_object_literal_for_jsvalue(arg: &HirExpr) -> Option<&HirExpr> {
    let HirExpr::JsonAsNative(inner, HirType::JsValue) = arg else {
        return None;
    };
    match inner.as_ref() {
        literal @ HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_dynamic_object_literal") =>
        {
            Some(literal)
        }
        _ => None,
    }
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
            self.note_quickjs("typed_calls:quickjs_backend");
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
            let name = self.intern_string(&signature.symbol, "jit_symbol")
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
        let name = self.intern_string(&signature.symbol, "dynamic_symbol")
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
            let constructor_name = self.intern_string(constructor_name, "napi_constructor_name")
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
                let constructor_name = self.intern_string(constructor_name, "typed_dynamic_constructor_name")
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
            let source = self.compile_array_data(result.into_pointer_value())?;
            let target = self.compile_array_data(argument)?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_bytes_copy").unwrap(),
                    &[
                        source.into(),
                        target.into(),
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
        let awaited = thaw_js_awaited_type(&signature.ret);
        let pending_site = matches!(signature.ret, HirType::Promise(_)) && thaw_js_pending_ok(awaited);
        if let Some(bad) = signature
            .params
            .iter()
            .find(|ty| !thaw_js_param_type(ty) || (**ty != HirType::Bytes && thaw_js_contains_bytes(ty)))
            .or_else(|| {
                // a `Promise<Date>` would decode as a plain object: only plain `Promise<T>` routes
                let unroutable = (!thaw_js_result_type(awaited) && signature.ret != HirType::JsValue)
                    || (awaited != &signature.ret && thaw_js_date_type(awaited));
                unroutable.then_some(&signature.ret)
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
        // `randomFillSync(buf)` returns the filled bytes; they are copied back into the caller's buffer
        // (the thaw-js side only ever sees a copy), as the QuickJS lane does.
        let mut returned_bytes_argument = None;
        // Owner types of `__thaw_tjs_live_owner_arg` arguments: a returned handle of one of these types maps back to its owner.
        let mut live_owner_types: Vec<HirType> = Vec::new();
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            if let HirExpr::Call(callee, call_args) = arg {
                if let (HirExpr::Var(name), [projected, original]) = (callee.as_ref(), call_args.as_slice()) {
                    if name == "__thaw_tjs_live_owner_arg" {
                        self.uses_thaw_js_handles = true;
                        self.js_mark_handles_used();
                        let handle = self.compile_expr(projected)?.into_int_value();
                        let marker = self.compile_thaw_js_handle_marker(handle)?;
                        self.compile_json_array_push_owned(array, marker)?;
                        if let Some(owner_type) = self.expr_hir_type(original) {
                            live_owner_types.push(owner_type);
                        }
                        continue;
                    }
                }
            }
            let reads_receiver = self.callback_expr_reads_receiver(arg) || self.closure_returns_promise(arg);
            let live_listener = self.live_json_listener(arg, ty);
            if thaw_js_callback_type(ty) && (reads_receiver || live_listener.is_some()) {
                let target = if reads_receiver { arg } else { live_listener.unwrap_or(arg) };
                self.compile_thaw_js_receiver_callback_argument(array, target, !reads_receiver)
                    .map_err(|error| format!("thaw-js callback argument {}: {error}", index + 1))?;
                continue;
            }
            if let Some(literal) = thaw_js_object_literal_for_jsvalue(arg).filter(|_| *ty == HirType::JsValue) {
                // a literal has no handle yet: send it as Json, as a `Json` parameter gets it
                let object = self.compile_thaw_js_object_literal_argument(literal)?;
                self.compile_typed_dynamic_argument(array, object, &HirType::Json)
                    .map_err(|error| format!("thaw-js argument {} (object literal): {error}", index + 1))?;
                continue;
            }
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
            if *ty == HirType::Bytes {
                if index == 0 && signature.ret == HirType::Bytes && signature.symbol.ends_with("randomFillSync") {
                    returned_bytes_argument = Some(value.into_pointer_value());
                }
                self.compile_thaw_js_bytes_argument(array, value)
                    .map_err(|error| format!("thaw-js argument {} (Bytes): {error}", index + 1))?;
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
        } else if thaw_js_date_type(awaited) {
            format!("\u{3}{}", signature.symbol)
        } else if pending_site {
            format!("\u{4}{}", signature.symbol)
        } else {
            signature.symbol.clone()
        };
        let name = self.intern_string(&symbol, "thaw_js_symbol")
            .map_err(|error| error.to_string())?;
        let mut json = self.compile_json_backend_values_with_extra(
            name.as_pointer_value().into(),
            array,
            "thaw_tjs_call_typed_result",
            &[],
            &[],
        )?;
        if signature.ret == HirType::JsValue {
            self.uses_thaw_js_handles = true;
            let id = self.compile_json_value_to_native(json, &HirType::F64)?.into_float_value();
            return Ok(self
                .builder
                .build_float_to_signed_int(id, self.context.i64_type(), "thaw_js_result_handle")
                .map_err(|error| error.to_string())?
                .into());
        }
        // A `\u{4}` call may answer with `{"__thaw_tjs_pending__": handle}` instead of the value: the
        // Promise is then settled later by the host, through the generated settler.
        let mut pending_edge = None;
        if pending_site {
            let function = self.current_function();
            let pending_block = self.context.append_basic_block(function, "tjs_pending_marker");
            let resolved_block = self.context.append_basic_block(function, "tjs_resolved");
            let merge_block = self.context.append_basic_block(function, "tjs_pending_merge");
            let key = self.intern_string("__thaw_tjs_pending__", "tjs_pending_key")
                .map_err(|error| error.to_string())?;
            let has = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_has_own").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "tjs_pending_has",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_has_own returned no value")?
                .into_int_value();
            let is_pending = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    has,
                    self.context.i8_type().const_zero(),
                    "tjs_is_pending",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_pending, pending_block, resolved_block)
                .map_err(|error| error.to_string())?;

            self.builder.position_at_end(pending_block);
            let marker = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_get").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "tjs_pending_marker_json",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_get returned no value")?;
            self.compile_check_json_host_error(marker, Some("thaw_json_destroy"))?;
            let handle = self.compile_json_as_value(marker, "thaw_json_as_number")?.into_float_value();
            let handle = self
                .builder
                .build_float_to_signed_int(handle, self.context.i64_type(), "tjs_pending_handle")
                .map_err(|error| error.to_string())?;
            let pending_promise = self
                .builder
                .build_call(self.module.get_function("thaw_promise_new").unwrap(), &[], "tjs_pending_promise")
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_promise_new returned no value")?
                .into_pointer_value();
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_retain").unwrap(),
                    &[pending_promise.into()],
                    "tjs_pending_watch_retain",
                )
                .map_err(|error| error.to_string())?;
            let marker_insert = self.builder.get_insert_block().unwrap();
            let settler = self.compile_thaw_js_pending_settler(awaited)?;
            self.builder.position_at_end(marker_insert);
            self.builder
                .build_call(
                    self.module.get_function("thaw_tjs_watch_promise").unwrap(),
                    &[handle.into(), pending_promise.into(), settler.into()],
                    "tjs_watch_pending_promise",
                )
                .map_err(|error| error.to_string())?;
            let pending_end = self.builder.get_insert_block().unwrap();
            self.builder
                .build_call(
                    self.module.get_function("thaw_json_destroy").unwrap(),
                    &[json.into()],
                    "destroy_tjs_pending_marker",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(resolved_block);
            pending_edge = Some((pending_promise, pending_end, merge_block));
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
        if *awaited == HirType::Json {
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
        let result = match self.compile_live_owner_result(json, &signature.ret, &live_owner_types)? {
            Some(owner) => owner,
            None => self.compile_typed_dynamic_result(json, &signature.ret)?,
        };
        if let Some((pending_promise, pending_end, merge_block)) = pending_edge {
            let resolved_end = self.builder.get_insert_block().unwrap();
            self.builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(merge_block);
            let phi = self
                .builder
                .build_phi(result.get_type(), "tjs_promise_or_pending")
                .map_err(|error| error.to_string())?;
            phi.add_incoming(&[
                (&result as &dyn BasicValue<'ctx>, resolved_end),
                (&pending_promise as &dyn BasicValue<'ctx>, pending_end),
            ]);
            return Ok(phi.as_basic_value());
        }
        if let Some(argument) = returned_bytes_argument {
            let source = self.compile_array_data(result.into_pointer_value())?;
            let target = self.compile_array_data(argument)?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_bytes_copy").unwrap(),
                    &[
                        source.into(),
                        target.into(),
                        self.context.f64_type().const_zero().into(),
                        self.context.f64_type().const_zero().into(),
                        self.context.f64_type().const_float(-1.0).into(),
                    ],
                    "thaw_js_returned_bytes_argument",
                )
                .map_err(|error| error.to_string())?;
            return Ok(argument.into());
        }
        Ok(result)
    }

    /// A `Bytes` argument for thaw-js: `{"__thaw_tjs_bytes__": {"k": "Buffer" | "Uint8Array" | "ArrayBuffer" | "DataView", "b64": "<base64>"}}`,
    /// base64 text rather than number slots. `k` follows the runtime kind (`thaw_bytes_console_info`
    /// is `1` for an ArrayBuffer, `2` for a DataView, `3` for a plain `Uint8Array`). A plain `{"type":"Buffer"}` object is not used here, since
    /// thaw-js would have no way to tell it from user data of that shape.
    fn compile_thaw_js_bytes_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let record = self.compile_array_data(value.into_pointer_value())?;
        let encoding = self.intern_string("base64", "thaw_js_bytes_encoding").map_err(|error| error.to_string())?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_bytes_to_string").unwrap(),
                &[record.into(), encoding.as_pointer_value().into()],
                "thaw_js_bytes_base64",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_bytes_to_string returned no value")?;
        let kind_code = self
            .builder
            .build_call(
                self.module.get_function("thaw_bytes_console_info").unwrap(),
                &[record.into(), self.context.f64_type().const_zero().into()],
                "thaw_js_bytes_kind_code",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_bytes_console_info returned no value")?
            .into_float_value();
        let is_u8 = self
            .builder
            .build_float_compare(
                inkwell::FloatPredicate::OEQ,
                kind_code,
                self.context.f64_type().const_float(3.0),
                "thaw_js_bytes_is_u8",
            )
            .map_err(|error| error.to_string())?;
        let inner = self
            .builder
            .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_bytes_inner")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let kind_key = self.intern_string("k", "thaw_js_bytes_kind_key").map_err(|error| error.to_string())?;
        let buffer_name = self.intern_string("Buffer", "thaw_js_bytes_kind").map_err(|error| error.to_string())?;
        let u8_name = self.intern_string("Uint8Array", "thaw_js_bytes_kind_u8").map_err(|error| error.to_string())?;
        // Kind codes from `thaw_bytes_console_info`: 1 ArrayBuffer, 2 DataView, 3 Uint8Array, else Buffer.
        let ab_name = self.intern_string("ArrayBuffer", "thaw_js_bytes_kind_ab").map_err(|error| error.to_string())?;
        let dv_name = self.intern_string("DataView", "thaw_js_bytes_kind_dv").map_err(|error| error.to_string())?;
        let is_ab = self
            .builder
            .build_float_compare(
                inkwell::FloatPredicate::OEQ,
                kind_code,
                self.context.f64_type().const_float(1.0),
                "thaw_js_bytes_is_ab",
            )
            .map_err(|error| error.to_string())?;
        let is_dv = self
            .builder
            .build_float_compare(
                inkwell::FloatPredicate::OEQ,
                kind_code,
                self.context.f64_type().const_float(2.0),
                "thaw_js_bytes_is_dv",
            )
            .map_err(|error| error.to_string())?;
        let buffer_or_ab = self
            .builder
            .build_select(is_ab, ab_name.as_pointer_value(), buffer_name.as_pointer_value(), "thaw_js_bytes_kind_ab_select")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let not_dv = self
            .builder
            .build_select(is_dv, dv_name.as_pointer_value(), buffer_or_ab, "thaw_js_bytes_kind_dv_select")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let kind = self
            .builder
            .build_select(is_u8, u8_name.as_pointer_value(), not_dv, "thaw_js_bytes_kind_select")
            .map_err(|error| error.to_string())?;
        let b64_key = self.intern_string("b64", "thaw_js_bytes_b64_key").map_err(|error| error.to_string())?;
        for (key, value) in [(kind_key, kind), (b64_key, text)] {
            self.builder
                .build_call(
                    self.module.get_function("thaw_json_object_set_string").unwrap(),
                    &[inner.into(), key.as_pointer_value().into(), value.into()],
                    "thaw_js_bytes_set_field",
                )
                .map_err(|error| error.to_string())?;
        }
        let outer = self
            .builder
            .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_bytes_outer")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let tag_key = self.intern_string("__thaw_tjs_bytes__", "thaw_js_bytes_tag_key").map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_object_set_json").unwrap(),
                &[outer.into(), tag_key.as_pointer_value().into(), inner.into()],
                "thaw_js_bytes_set_tag",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(self.module.get_function("thaw_json_destroy").unwrap(), &[inner.into()], "thaw_js_bytes_destroy_inner")
            .map_err(|error| error.to_string())?;
        self.compile_json_array_push_owned(array, outer)
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
        let name = self.intern_string(message, "thaw_js_unsupported")
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
        // `$setter$` takes the receiver handle and the new value, like a one-argument method
        let is_method = symbol.starts_with("$method$")
            || symbol.starts_with("$methodvoid$")
            || symbol.starts_with("$setter$");
        let handle_param = |ty: &HirType| *ty == HirType::JsValue;
        let ret_handle = signature.ret == HirType::JsValue;
        let is_promise = matches!(signature.ret, HirType::Promise(_));
        let awaited = thaw_js_awaited_type(&signature.ret);
        // `Promise<JsValue>` (a method result): the host settles the Promise and hands back the value's handle,
        // which is wrapped in a resolved native promise (bit 0 and bit 1 together)
        let promise_handle = is_promise && *awaited == HirType::JsValue && !is_new;
        let supported = (is_new || is_static || is_method)
            && (ret_handle
                || promise_handle
                || ((!is_new || is_value)
                    && thaw_js_plain_type(awaited)
                    && !(is_promise && thaw_js_date_type(awaited))))
            && signature.params.iter().skip(usize::from(is_method)).all(|ty| {
                thaw_js_plain_type(ty)
                    || handle_param(ty)
                    || thaw_js_graph_callback_type(ty)
                    || (is_new && !is_value && thaw_js_object_with_callbacks_type(ty))
            })
            && !signature.params.iter().any(|ty| *ty != HirType::Bytes && thaw_js_contains_bytes(ty))
            && (!is_method || signature.params.first().is_some_and(handle_param))
            && args.len() == signature.params.len();
        if !supported {
            // Class operations are only emitted where the program uses them (unlike the wrappers of
            // plain functions), so an unsupported one is a build error the CLI answers by rebuilding
            // the program on QuickJS.
            return Err(format!(
                "thaw-js: class operation `{}` has a signature that cannot cross the thaw-js boundary ({:?} -> {:?})",
                signature.symbol, signature.params, signature.ret
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
            let reads_receiver = self.callback_expr_reads_receiver(arg) || self.closure_returns_promise(arg);
            let live_listener = self.live_json_listener(arg, ty);
            // a callback with a live (handle) parameter has no text route: it is always a graph callback
            let graph_only = thaw_js_graph_callback_type(ty) && !thaw_js_callback_type(ty);
            if (thaw_js_callback_type(ty) && (reads_receiver || live_listener.is_some())) || graph_only {
                let target = if reads_receiver { arg } else { live_listener.unwrap_or(arg) };
                self.compile_thaw_js_receiver_callback_argument(array, target, !reads_receiver)
                    .map_err(|error| format!("thaw-js class callback argument {}: {error}", index + 1))?;
                continue;
            }
            if let Some(literal) = thaw_js_object_literal_for_jsvalue(arg).filter(|_| handle_param(ty)) {
                let object = self.compile_thaw_js_object_literal_argument(literal)?;
                self.compile_typed_dynamic_argument(array, object, &HirType::Json)
                    .map_err(|error| format!("thaw-js class argument {} (object literal): {error}", index + 1))?;
                continue;
            }
            if is_new && !is_value && thaw_js_object_with_callbacks_type(ty) {
                let literal = match arg {
                    HirExpr::Call(callee, _)
                        if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_dynamic_object_literal") =>
                    {
                        arg
                    }
                    _ => {
                        return Err(format!(
                            "thaw-js: constructor option object with callbacks must be an object literal ({})",
                            signature.symbol
                        ))
                    }
                };
                let object = self.compile_thaw_js_object_literal_argument(literal)?;
                self.compile_typed_dynamic_argument(array, object, &HirType::Json)
                    .map_err(|error| format!("thaw-js class argument {} (object literal): {error}", index + 1))?;
                continue;
            }
            self.thaw_js_call_arguments = true;
            let value = self.compile_expr(arg);
            self.thaw_js_call_arguments = false;
            let value = value?;
            if thaw_js_callback_type(ty) {
                self.compile_thaw_js_callback_argument(array, value, ty)
                    .map_err(|error| format!("thaw-js class callback argument {}: {error}", index + 1))?;
                continue;
            }
            if handle_param(ty) {
                self.uses_thaw_js_handles = true;
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
            if *ty == HirType::Bytes {
                self.compile_thaw_js_bytes_argument(array, value)
                    .map_err(|error| format!("thaw-js class argument {} (Bytes): {error}", index + 1))?;
                continue;
            }
            self.compile_typed_dynamic_argument(array, value, ty)
                .map_err(|error| format!("thaw-js class argument {} ({ty:?}): {error}", index + 1))?;
        }
        let name = self.intern_string(symbol, "thaw_js_class_symbol")
            .map_err(|error| error.to_string())?;
        // bit 0: the result is a handle id; bit 1: await a returned Promise (see `class_call`);
        // bit 2: await a returned Promise only for a union return (`$await` runtime-key suffix)
        let union_promise = ret_handle && symbol.ends_with("$await");
        let want_handle = self.context.i8_type().const_int(
            u64::from(ret_handle || promise_handle)
                | (u64::from(is_promise) << 1)
                | (u64::from(union_promise) << 2),
            false,
        );
        let json = self.compile_json_backend_values_with_extra(
            name.as_pointer_value().into(),
            array,
            "thaw_tjs_class_call",
            &[receiver.into(), want_handle.into()],
            &[],
        )?;
        if ret_handle || promise_handle {
            self.uses_thaw_js_handles = true;
            let id = self.compile_json_value_to_native(json, &HirType::F64)?.into_float_value();
            let handle = self
                .builder
                .build_float_to_signed_int(id, self.context.i64_type(), "thaw_js_class_handle")
                .map_err(|error| error.to_string())?;
            if ret_handle {
                return Ok(handle.into());
            }
            let promise = self
                .builder
                .build_call(self.module.get_function("thaw_promise_new").unwrap(), &[], "thaw_js_class_promise")
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_promise_new returned no value")?
                .into_pointer_value();
            let slot = self.allocate_arena_cell(self.basic_type(&HirType::JsValue)?, "thaw_js_class_promise_result")?;
            self.builder.build_store(slot, handle).map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_resolve").unwrap(),
                    &[promise.into(), slot.into()],
                    "resolve_thaw_js_class_promise",
                )
                .map_err(|error| error.to_string())?;
            return Ok(promise.into());
        }
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    /// The object literal call of a `JsValue` argument, built as Json with the thaw-js argument flag
    /// set (see `compile_dynamic_object_literal_call`). The result is the same owned Json a `Json`
    /// argument gets, so the caller pushes it with `compile_typed_dynamic_argument` and no extra destroy.
    fn compile_thaw_js_object_literal_argument(
        &mut self,
        literal: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.thaw_js_call_arguments = true;
        let object = self.compile_expr(literal);
        self.thaw_js_call_arguments = false;
        object
    }

    /// A result of the same object type as a live-owner argument that comes back as its handle marker
    /// maps to the interned native owner; any other result decodes by value as before. Emits nothing
    /// (returns None) unless a live-owner argument of this type was passed.
    fn compile_live_owner_result(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ret: &HirType,
        live_owner_types: &[HirType],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let HirType::Object(fields) = ret else { return Ok(None) };
        if live_owner_types.is_empty() || !live_owner_types.contains(ret) || thaw_js_date_type(ret) {
            return Ok(None);
        }
        self.js_mark_handles_used();
        let function = self.current_function();
        let check_block = self.context.append_basic_block(function, "tjs_live_owner_check");
        let owner_block = self.context.append_basic_block(function, "tjs_live_owner");
        let decode_block = self.context.append_basic_block(function, "tjs_live_owner_decode");
        let merge_block = self.context.append_basic_block(function, "tjs_live_owner_merge");
        let key = self.intern_string("__thaw_tjs_handle__", "tjs_live_owner_key")
            .map_err(|error| error.to_string())?;
        let has = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_has_own").unwrap(),
                &[json.into(), key.as_pointer_value().into()],
                "tjs_live_owner_has",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_has_own returned no value")?
            .into_int_value();
        let is_marker = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, has, self.context.i8_type().const_zero(), "tjs_live_owner_is_marker")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_marker, check_block, decode_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(check_block);
        let marker = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_get").unwrap(),
                &[json.into(), key.as_pointer_value().into()],
                "tjs_live_owner_marker_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_get returned no value")?;
        self.compile_check_json_host_error(marker, Some("thaw_json_destroy"))?;
        let id = self.compile_json_as_value(marker, "thaw_json_as_number")?.into_float_value();
        let id = self
            .builder
            .build_float_to_signed_int(id, self.context.i64_type(), "tjs_live_owner_id")
            .map_err(|error| error.to_string())?;
        let layout = self
            .intern_string(&thaw_hir::native_object_layout_token(fields), "tjs_live_owner_layout")
            .map_err(|error| error.to_string())?;
        let lookup = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_native_object_pointer").unwrap(),
                &[id.into(), layout.as_pointer_value().into()],
                "tjs_live_owner_pointer",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_native_object_pointer returned no result")?
            .into_struct_value();
        let word = self
            .builder
            .build_extract_value(lookup, 0, "tjs_live_owner_word")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let error = self
            .builder
            .build_extract_value(lookup, 1, "tjs_live_owner_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let present = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, word, self.context.i64_type().const_zero(), "tjs_live_owner_present")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(present, owner_block, decode_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(owner_block);
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let owner = self
            .builder
            .build_int_to_ptr(word, ptr_type, "tjs_live_owner_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(self.module.get_function("thaw_json_destroy").unwrap(), &[json.into()], "destroy_tjs_live_owner_json")
            .map_err(|error| error.to_string())?;
        let owner_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(decode_block);
        let decoded = self.compile_typed_dynamic_result(json, ret)?;
        let decode_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge_block);
        let phi = self
            .builder
            .build_phi(decoded.get_type(), "tjs_live_owner_result")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[
            (&owner as &dyn BasicValue<'ctx>, owner_end),
            (&decoded as &dyn BasicValue<'ctx>, decode_end),
        ]);
        Ok(Some(phi.as_basic_value()))
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
        let key = self.intern_string("__thaw_tjs_handle__", "thaw_js_handle_key")
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
        let Some((params, ret, rest_start)) = thaw_js_callback_abi(function_type) else {
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
        let marker = self.compile_thaw_js_callback_marker(closure, &params, &ret, rest_start)?;
        self.compile_json_array_push_owned(array, marker)?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// A listener declared with a rest signature (`(...args: any[]) => void`) whose own closure takes a
    /// bare `Json` or `JsValue` parameter (an `any` the closure uses as a live value) must see live arguments, so it takes the graph route by its own type. The
    /// rest-to-fixed adapter HIR wraps such a closure in is peeled: the user closure is registered.
    fn live_json_listener<'a>(&self, arg: &'a HirExpr, declared: &HirType) -> Option<&'a HirExpr> {
        let rest_declared = matches!(declared, HirType::CallableFunction(_, _, Some(_), _))
            || matches!(declared, HirType::Optional(inner) if matches!(inner.as_ref(), HirType::CallableFunction(_, _, Some(_), _)));
        let takes_live_json = |closure: &HirExpr| {
            matches!(
                self.expr_hir_type(closure),
                Some(HirType::Function(params, _)) if params.iter().any(|param| matches!(param, HirType::Json | HirType::JsValue))
            )
        };
        if !rest_declared {
            return None;
        }
        // peel every coercion adapter (`(callback) => (...args) => callback(...)` wrappers) down to the user closure
        let mut inner = arg;
        while let HirExpr::Call(callee, args) = inner {
            match (callee.as_ref(), args.as_slice()) {
                (HirExpr::Lambda(_, params, _, _), [next])
                    if matches!(params.as_slice(), [param] if param.name.starts_with("__thaw_") && param.name.contains("_callback_")) =>
                {
                    inner = next;
                }
                _ => break,
            }
        }
        takes_live_json(inner).then_some(inner)
    }

    /// A closure whose own result is a Promise: registered through the graph route (handle marker) so
    /// the compiled side's native Promise reaches JS as a Promise (`finish`); the JSON marker route has none.
    fn closure_returns_promise(&self, arg: &HirExpr) -> bool {
        matches!(
            self.expr_hir_type(arg),
            Some(HirType::Function(_, ret) | HirType::CallableFunction(_, _, _, ret))
                if matches!(*ret, HirType::Promise(_))
        )
    }

    /// A callback that reads the call-time receiver: registered through the graph path (its `this`
    /// arrives with each call) and passed as a `{"__thaw_tjs_handle__": id}` function handle.
    fn compile_thaw_js_receiver_callback_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        arg: &HirExpr,
        graph_mode: bool,
    ) -> Result<(), String> {
        self.thaw_js_call_arguments = true;
        let handle = self.compile_register_native_callback(std::slice::from_ref(arg), graph_mode);
        self.thaw_js_call_arguments = false;
        let marker = self.compile_thaw_js_handle_marker(handle?.into_int_value())?;
        self.compile_json_array_push_owned(array, marker)
    }

    /// Registers the closure with thaw-js and returns the `{"__thaw_tjs_cb__": id}` marker Json.
    fn compile_thaw_js_callback_marker(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        rest_start: Option<usize>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_thaw_js = true;
        // JSON text both ways (`graph_wire` off): thaw-js has no live-handle graph protocol.
        let (adapter, context, _) =
            self.compile_value_callback_from_closure(closure, params, ret, false, rest_start, false, false, false)?;
        let i64_type = self.context.i64_type();
        // The alias-resolved closure is the callback's identity: `off(listener)` after `on(listener)` finds it.
        let identity = self.resolve_closure_alias(closure)?;
        self.builder.build_call(
            self.module.get_function("thaw_tjs_callback_identity").unwrap(),
            &[identity.into()],
            "thaw_js_callback_identity",
        ).map_err(|error| error.to_string())?;
        let id = self.builder.build_call(
            self.module.get_function("thaw_tjs_register_callback").unwrap(),
            &[adapter.into(), context.into(), i64_type.const_int(if rest_start.is_some() { i64::MAX as u64 } else { params.len() as u64 }, false).into()],
            "thaw_js_callback_id",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_tjs_register_callback returned no value")?.into_int_value();
        let id = self.builder.build_signed_int_to_float(id, self.context.f64_type(), "thaw_js_callback_id_f64")
            .map_err(|error| error.to_string())?;
        let marker = self.builder.build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_callback_marker")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self.intern_string("__thaw_tjs_cb__", "thaw_js_callback_key")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_object_set_number").unwrap(),
            &[marker.into(), key.as_pointer_value().into(), id.into()],
            "thaw_js_callback_marker_id",
        ).map_err(|error| error.to_string())?;
        Ok(marker)
    }

    /// `__thaw_dynamic_object_literal(builder, [kind, key, key kind, value, callback]*)`: a live
    /// object literal (`lower/objects.rs`). As an argument of a thaw-js call it is built here as
    /// plain Json: data members as they are, methods as `{"__thaw_tjs_handle__": id}` receiver
    /// callbacks. Getters, setters, `__proto__` and spreads stay loud (an unsupported stub), and
    /// computed or symbol keys keep the QuickJS builder. Anywhere else the builder is the meaning.
    pub(crate) fn compile_dynamic_object_literal_call(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Some((builder, members)) = args.split_first() else {
            return Err("live object literal marker expects its builder block".into());
        };
        if members.len() % 5 != 0 {
            return Err("live object literal marker expects five operands per member".into());
        }
        if !(self.thaw_js_routed && self.thaw_js_call_arguments) {
            return self.compile_expr(builder);
        }
        // the length was checked to be a multiple of five, so no remainder is left
        let (members, _) = members.as_chunks::<5>();
        for member in members {
            if let HirExpr::Lit(HirLit::Str(kind)) = &member[0] {
                if matches!(kind.as_str(), "getter" | "setter" | "proto" | "spread") {
                    self.compile_thaw_js_unsupported_stub(&format!(
                        "\u{1}thaw-js: object literal {kind} members cannot cross the thaw-js boundary"
                    ))?;
                    return self.compile_json_null();
                }
            }
        }
        let plain_keys = members.iter().all(|member| {
            matches!(
                (&member[1], &member[2]),
                (HirExpr::Lit(HirLit::Str(_)), HirExpr::Lit(HirLit::Bool(false)))
            )
        });
        if !plain_keys {
            return self.compile_expr(builder);
        }
        let object = self
            .builder
            .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "thaw_js_object_literal")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        for member in members {
            let [kind, key, _, value, callback] = member;
            let HirExpr::Lit(HirLit::Str(key)) = key else {
                return Err("live object literal key is not a literal".into());
            };
            let key = self.intern_string(key, "thaw_js_object_key").map_err(|error| error.to_string())?;
            let saved = self.thaw_js_call_arguments;
            let is_method = matches!(kind, HirExpr::Lit(HirLit::Str(kind)) if kind.as_str() == "method");
            let field = if is_method {
                // a method forwards its receiver, so it is a receiver callback (as a handle)
                self.thaw_js_call_arguments = true;
                // A method closure takes its receiver as physical param 0 (`NonArrowReceiver`): register it
                // with the method ABI, or every argument shifts by one slot.
                let receiver_first = matches!(
                    self.expr_hir_type(callback),
                    Some(HirType::Function(ref params, _))
                        if matches!(params.first(), Some(HirType::NonArrowReceiver(_)))
                );
                let handle = if receiver_first {
                    self.compile_register_native_method_callback_graph(std::slice::from_ref(callback))
                } else {
                    self.compile_register_native_callback(std::slice::from_ref(callback), false)
                };
                self.thaw_js_call_arguments = saved;
                let marker = self.compile_thaw_js_handle_marker(handle?.into_int_value())?;
                self.compile_json_object_set_native(object, key.as_pointer_value(), marker, &HirType::Json)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[marker.into()],
                        "destroy_marshaled_object_method",
                    )
                    .map_err(|error| error.to_string())?;
                continue;
            } else {
                self.thaw_js_call_arguments = true;
                let value = self.compile_expr(value);
                self.thaw_js_call_arguments = saved;
                value?
            };
            self.compile_json_object_set_native(object, key.as_pointer_value(), field, &HirType::Json)?;
        }
        self.mark_fresh_owned_json_result(object)?;
        Ok(object)
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
        // A closure with a bare `Json` parameter (an `any` it may call methods on) must receive live
        // arguments: register it on the graph route (handle marker), as the QuickJS lane does. Bytes and
        // number closures keep the text route.
        if params.contains(&HirType::Json) && json_callback_wants_graph(params) {
            self.thaw_js_call_arguments = true;
            let handle = self.compile_register_native_callback(std::slice::from_ref(closure_expr), false);
            self.thaw_js_call_arguments = false;
            let marker = self.compile_thaw_js_handle_marker(handle?.into_int_value())?;
            self.tracks_owned_json_roots = true;
            self.mark_fresh_owned_json_result(marker)?;
            return Ok(Some(marker));
        }
        let closure = self.compile_expr(closure_expr)?.into_pointer_value();
        let marker = self.compile_thaw_js_callback_marker(closure, params, ret, None)?;
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
