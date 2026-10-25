impl<'ctx> HirCompiler<'ctx> {
    fn compile_dynamic_iterator_step_exact(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [iterator, mode, error_text] = args else {
            return Err("exact iterator step expects iterator, mode, and error text".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        // The iterator result is captured as a live graph; reading its `done`/`value`
        // later needs the host operations, like the other dynamic call paths.
        self.compile_register_js_callback_host_operations()?;
        let iterator = self.compile_expr(iterator)?.into_int_value();
        let mode = self.compile_expr(mode)?.into_int_value();
        let mode = self.builder.build_int_truncate(mode, self.context.i8_type(),
            "iterator_step_mode").map_err(|error| error.to_string())?;
        let error_text = self.compile_expr(error_text)?.into_pointer_value();
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_iterator_step_exact_result").unwrap(),
            &[iterator.into(), mode.into(), error_text.into()], "iterator_step_exact",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact iterator step returned no result")?.into_struct_value();
        let value = self.compile_exact_iterator_handle_result(result, "iterator_step")?;
        self.compile_original_quickjs_exception_handle(value, true)
    }

    fn compile_dynamic_property_exact(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [receiver, key] = args else {
            return Err("getDynamicProperty expects a receiver and property key".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        // A getter may enter native callbacks and graph capture on this first
        // property read, so publish the host operations before evaluating it.
        self.compile_register_js_callback_host_operations()?;
        let receiver = self.compile_expr(receiver)?.into_int_value();
        let key = self.compile_expr(key)?.into_pointer_value();
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_get_property_exact_result").unwrap(),
            &[receiver.into(), key.into()], "dynamic_property_exact",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact property read returned no result")?.into_struct_value();
        self.compile_exact_iterator_handle_result(result, "property_read")
    }

    fn compile_to_iterator_exact(
        &mut self, args: &[HirExpr], asynchronous: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("exact iterator acquisition expects one source".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.compile_register_js_callback_host_operations()?;
        let source = self.compile_expr(source)?.into_int_value();
        let function = if asynchronous {
            "thaw_js_to_async_iterator_exact_result"
        } else {
            "thaw_js_to_iterator_exact_result"
        };
        let result = self.builder.build_call(
            self.module.get_function(function).unwrap(),
            &[source.into()], "to_iterator_exact",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact iterator acquisition returned no result")?.into_struct_value();
        self.compile_exact_iterator_handle_result(result, "iterator_acquire")
    }

    pub(super) fn compile_exact_iterator_handle_result(
        &mut self, result: inkwell::values::StructValue<'ctx>, _label: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.builder.build_extract_value(result, 0, "iterator_step_record")
            .map_err(|error| error.to_string())?;
        let thrown = self.builder.build_extract_value(result, 1, "iterator_step_thrown")
            .map_err(|error| error.to_string())?.into_int_value();
        let native_error = self.builder.build_extract_value(result, 3, "iterator_step_native_error")
            .map_err(|error| error.to_string())?;
        let has_thrown = self.builder.build_int_compare(IntPredicate::NE, thrown,
            self.context.i64_type().const_zero(), "iterator_step_has_thrown")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let exact_throw = self.context.append_basic_block(function, "iterator_step_exact_throw");
        let native_check = self.context.append_basic_block(function, "iterator_step_native_check");
        self.builder.build_conditional_branch(has_thrown, exact_throw, native_check)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(exact_throw);
        self.compile_throw_original_quickjs_exception_handle(thrown.into(), true)?;
        self.builder.build_unconditional_branch(native_check)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(native_check);
        self.builder.build_store(self.pending_exception().as_pointer_value(), native_error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(native_error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }
    fn compile_dynamic_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        Some(match name {
            "loadScript" => self.compile_load_script(args),
            "loadThawJs" => self.compile_load_thaw_js(args),
            "__thaw_to_iterator_exact" => self.compile_to_iterator_exact(args, false),
            "__thaw_to_async_iterator_exact" => self.compile_to_iterator_exact(args, true),
            "__thaw_dynamic_iterator_step_exact" => self.compile_dynamic_iterator_step_exact(args),
            "callDynamic" => self.compile_call_dynamic(args),
            "getDynamicValue" => self.compile_get_dynamic_value(args),
            "newDynamicFunction" => self.compile_new_dynamic_function(args),
            "callDynamicValue" => self.compile_call_dynamic_value(args),
            "callNativeAddonValue" => self.compile_call_native_addon_value(args),
            "callDynamicValueHandle" => self.compile_call_dynamic_value_handle(args),
            "callDynamicValueWithValue" => self.compile_call_dynamic_value_with_value(args),
            "releaseDynamicValue" => self.compile_release_dynamic_value(args),
            "getDynamicProperty" => self.compile_dynamic_property_exact(args),
            "deleteDynamicProperty" => self.compile_dynamic_property_predicate(
                "thaw_js_delete_property_result",
                args,
            ),
            "hasDynamicProperty" => {
                self.compile_dynamic_property_predicate("thaw_js_has_property_result", args)
            }
            "setDynamicProperty" => self.compile_set_dynamic_property(args),
            "setDynamicPropertyJson" => self.compile_set_dynamic_property_json(args),
            "callDynamicMethod" => self.compile_call_dynamic_method(args),
            "__thaw_call_selected_dynamic_method" => self.compile_selected_call_dynamic_method(args),
            "__thaw_call_selected_dynamic_method_handle" => self.compile_selected_call_dynamic_method_handle(args, false),
            "__thaw_call_selected_dynamic_method_raw" => self.compile_selected_call_dynamic_method_handle(args, true),
            "callDynamicMethodHandle" => self.compile_call_dynamic_method_handle(args, false),
            "callDynamicMethodHandleRaw" => self.compile_call_dynamic_method_handle(args, true),
            "readDynamicValue" => self.compile_read_dynamic_value(args),
            "retainDynamicJson" => self.compile_retain_dynamic_json(args),
            "retainDynamicNative" => self.compile_retain_dynamic_native(args),
            "retainEventKey" => self.compile_retain_event_key(args),
            "__thaw_mark_typed_wrapper_json_argument" => {
                let [kind, key, argument] = args else {
                    return Some(Err("typed-wrapper JSON ownership marker expects owner kind, key, and value".into()));
                };
                let owner = match (kind, key) {
                    (HirExpr::Lit(HirLit::Str(kind)), HirExpr::Lit(HirLit::Str(key))) => {
                        match kind.as_str() {
                            "wrapper" => TypedJsonArgumentOwner::ProvenWrapper(key.clone()),
                            "direct-quickjs" => TypedJsonArgumentOwner::DirectQuickJs(key.clone()),
                            "napi-constructor" => TypedJsonArgumentOwner::NapiConstructor(key.clone()),
                            _ => return Some(Err("typed-wrapper JSON ownership marker has unknown owner kind".into())),
                        }
                    }
                    _ => return Some(Err("typed-wrapper JSON ownership marker owner must be literal".into())),
                };
                (|| {
                    let value = self.compile_expr(argument)?.into_pointer_value();
                    self.track_typed_wrapper_json_argument(value, &owner)?;
                    Ok(value.into())
                })()
            }
            "@@thaw_retain_borrowed_dynamic_handle" =>
                self.compile_retain_borrowed_dynamic_handle(args),
            "__thaw_json_host_from_dynamic" => self.compile_host_json_from_dynamic(args),
            "__thaw_json_from_consumed_dynamic" => self.compile_consumed_json_from_dynamic(args),
            "__thaw_json_host_from_borrowed_dynamic" =>
                self.compile_host_json_from_borrowed_dynamic(args),
            "__thaw_lookup_native_object" => self.compile_lookup_native_object(args),
            "__thaw_lookup_native_projector" => self.compile_lookup_native_projector(args),
            "__thaw_native_view_target" => self.compile_native_view_target(args),
            "__thaw_json_link_native" | "__thaw_json_link_native_object" | "__thaw_json_link_native_tree" =>
                self.compile_json_link_native(name, args),
            "__thaw_json_note_view_copy" => (|| {
                self.builder.build_call(
                    self.module.get_function("thaw_json_note_view_copy").unwrap(), &[], "note_view_copy",
                ).map_err(|error| error.to_string())?;
                Ok(self.context.i8_type().const_zero().into())
            })(),
            "__thaw_json_has_native_link" | "__thaw_json_linked_array" => {
                let [array] = args else {
                    return Some(Err("native link query expects one array".into()));
                };
                let function = if name == "__thaw_json_has_native_link" {
                    "thaw_json_has_native_link"
                } else {
                    "thaw_json_linked_array"
                };
                (|| {
                    let array = self.compile_expr(array)?.into_pointer_value();
                    let result = self.builder.build_call(
                        self.module.get_function(function).unwrap(), &[array.into()], function,
                    ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or_else(|| "native link query returned no value".to_string())?;
                    if name != "__thaw_json_has_native_link" {
                        return Ok(result);
                    }
                    let zero = self.context.i8_type().const_zero();
                    self.builder.build_int_compare(IntPredicate::NE, result.into_int_value(), zero, "has_native_link")
                        .map(Into::into).map_err(|error| error.to_string())
                })()
            }
            "__thaw_register_native_exception_describer" =>
                self.compile_register_native_exception_describer(args),
            "__thaw_lookup_native_exception_describer" =>
                self.compile_lookup_native_exception_describer(args),
            "__thaw_register_native_object_projector" =>
                self.compile_register_native_object_projector(args),
            "__thaw_register_native_object_layout" =>
                self.compile_register_native_object_layout(args),
            "__thaw_require_native_owner" => self.compile_require_native_owner(args),
            "__thaw_release_native_projection_callbacks" =>
                self.compile_release_native_projection_callbacks(args),
            "__thaw_intern_native_object" => self.compile_intern_native_object(args),
            "resolveDynamicValue" => self.compile_dynamic_handle_operation(
                "thaw_js_resolve_handle_handle_result",
                args,
            ),
            "callDynamicValueMixed" => self.compile_call_dynamic_value_mixed(args),
            "callDynamicValueMixedExact" => self.compile_call_dynamic_value_mixed_exact(args),
            "callDynamicValueMixedNativeJson" => self.compile_call_dynamic_value_mixed_native_json(args),
            "callDynamicValueMixedHandle" => {
                self.compile_call_dynamic_value_mixed_handle(args)
            }
            "__thaw_build_native_object_wrapper" => {
                self.compile_build_native_object_wrapper(args)
            }
            "constructDynamicValue" => self.compile_construct_dynamic_value(args),
            "loadNativeAddon" => self.compile_load_native_addon(args),
            "loadNativeAddonEmbedded" => self.compile_load_embedded_native_addon(args),
            "embedExecutable" => self.compile_embed_executable(args),
            "setProcessEnv" => self.compile_set_process_env(args),
            "loadNativeSharedLibraryEmbedded" => self.compile_load_embedded_native_dependency(args),
            "loadNativeSharedLibrary" => self.compile_load_native_dependency(args),
            "callNativeAddon" => self.compile_call_native_addon(args),
            "callNativeAddonWithCallback" => self.compile_call_native_addon_with_callback(args),
            "pollNativeAddonEvents" => self.compile_poll_native_addon_events(args),
            "registerNativeCallback" => self.compile_register_native_callback(args, false),
            "registerNativeCallbackGraph" => self.compile_register_native_callback(args, true),
            "__thaw_register_native_method_callback_graph" => {
                self.compile_register_native_method_callback_graph(args)
            }
            _ => return None,
        })
    }

    fn compile_dynamic_property_predicate(
        &mut self,
        symbol: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_dynamic_handle_operation(symbol, args)?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i64_type().const_zero(),
                "dynamic_property_predicate",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_set_dynamic_property(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_dynamic_handle_operation("thaw_js_set_property_result", args)?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i64_type().const_zero(),
                "dynamic_property_set",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    /// A thaw-js routed build holds no QuickJS values, so a caught JsValue / live native projection
    /// (the arms of the caught-exception Json adapter) cannot occur there: evaluate the operand and
    /// yield an empty object instead of depending on the QuickJS host. A program that does throw such
    /// a value registers a projector through a QuickJS operation, which sends the build back to the
    /// unrouted path.
    fn compile_routed_no_host_json(&mut self, source: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_expr(source)?;
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_object_new").unwrap(), &[], "routed_no_host_json",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_object_new returned no value")?;
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    fn compile_host_json_from_dynamic(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("live JSON host conversion expects one handle".into());
        };
        if self.thaw_js_routed {
            return self.compile_routed_no_host_json(source);
        }
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.tracks_owned_json_roots = true;
        let handle = self.compile_expr(source)?.into_int_value();
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_host_from_borrowed_handle").unwrap(),
            &[handle.into()], "live_json_host",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("live JSON host conversion returned no value")?;
        // The Host lease retained its own reference. Discard the temporary
        // handle even when the host conversion reported an error.
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[handle.into()], "release_live_json_builder_handle")
            .map_err(|error| error.to_string())?;
        let json = self.compile_check_json_host_error(json, Some("thaw_json_destroy"))?;
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    /// Consumes the freshly projected handle after the generic Json
    /// conversion has retained/materialized its own value. Unlike the
    /// host-only helper above, this preserves the runtime's copy-vs-live
    /// policy for native object projections.
    fn compile_consumed_json_from_dynamic(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("consuming JSON conversion expects one handle".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.tracks_owned_json_roots = true;
        let handle = self.compile_expr(source)?.into_int_value();
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_from_borrowed_handle").unwrap(),
            &[handle.into()], "consumed_dynamic_json",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("consuming JSON conversion returned no value")?;
        // This projection call produced a fresh retained handle. The Json
        // conversion owns an independent copy/lease, so release the temporary
        // on both success and HostError paths.
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(),
            &[handle.into()], "release_consumed_dynamic_projection",
        ).map_err(|error| error.to_string())?;
        let json = self.compile_check_json_host_error(json, Some("thaw_json_destroy"))?;
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    fn compile_host_json_from_borrowed_dynamic(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("borrowed live JSON host conversion expects one handle".into());
        };
        if self.thaw_js_routed {
            return self.compile_routed_no_host_json(source);
        }
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.tracks_owned_json_roots = true;
        let handle = self.compile_expr(source)?.into_int_value();
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_host_from_borrowed_handle").unwrap(),
            &[handle.into()], "borrowed_live_json_host",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("borrowed live JSON host conversion returned no value")?;
        // The source JsValue is borrowed. The std helper retained an
        // independent host lease; the lexical catch carrier keeps its own.
        let json = self.compile_check_json_host_error(json, Some("thaw_json_destroy"))?;
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    /// The exact mixed call consumes each direct handle. A source JsValue is
    /// borrowed, so give the call its own registry reference before staging.
    fn compile_retain_borrowed_dynamic_handle(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("@@thaw_retain_borrowed_dynamic_handle expects one JsValue".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let handle = self.compile_expr(value)?.into_int_value();
        let retained = self.builder.build_call(
            self.module.get_function("thaw_js_retain_handle").unwrap(),
            &[handle.into()], "retain_exact_mixed_borrowed_handle",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_js_retain_handle returned no status")?.into_int_value();
        let valid = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, retained,
            self.context.i8_type().const_zero(), "exact_mixed_borrowed_retained",
        ).map_err(|error| error.to_string())?;
        let ready = self.context.append_basic_block(
            self.current_function(), "exact_mixed_borrowed_ready");
        let invalid = self.context.append_basic_block(
            self.current_function(), "exact_mixed_borrowed_invalid");
        self.builder.build_conditional_branch(valid, ready, invalid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.compile_throw_type_error("Invalid borrowed JavaScript value handle")?;
        self.builder.position_at_end(ready);
        Ok(handle.into())
    }
    fn compile_retain_dynamic_json(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("retainDynamicJson expects exactly one argument".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let compiling_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        // A caller may reach this intrinsic with a bare native object
        // literal (`retainDynamicJson({})`, an internal-builtin call some
        // tests exercise directly) rather than the already-`Json`-coerced
        // argument the compiler's own emit sites pass -- compile such a
        // literal through the JSON builder so the runtime sees a real
        // shared `Json` value, not a fixed-layout native object.
        let value = match value {
            HirExpr::ObjectLit(fields) => self.compile_json_object_lit(fields, &HirType::Json),
            _ => self.compile_expr(value),
        };
        self.compiling_quickjs_dynamic_arguments = compiling_dynamic_arguments;
        let value = value?.into_pointer_value();
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[value.into()],
                "retained_dynamic_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let json = self.compile_check_json_stringify_error(json)?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_retain_graph_result").unwrap(),
                &[json.into()],
                "retain_dynamic_json_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let retained = self
            .builder
            .build_extract_value(result, 0, "retained_dynamic_handle")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "retain_dynamic_json_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[json.into()],
                "destroy_retained_dynamic_json",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(retained)
    }

    fn compile_retain_dynamic_native(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("retainDynamicNative expects exactly one argument".into());
        };
        let source_type = self.expr_hir_type(source)
            .ok_or("retainDynamicNative source has no inferred native type")?;
        if !matches!(&source_type, HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)) {
            return Err(format!("retainDynamicNative does not support {source_type:?}"));
        }
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.tracks_owned_json_roots = true;
        let native = self.compile_expr(source)?;
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let entry = self.current_function().get_first_basic_block().unwrap();
        let entry_builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            entry_builder.position_before(&first);
        } else {
            entry_builder.position_at_end(entry);
        }
        let ledger = entry_builder.build_alloca(ptr_ty, "native_symbol_handle_ledger")
            .map_err(|error| error.to_string())?;
        let capacity = entry_builder.build_alloca(self.context.i64_type(), "native_symbol_handle_capacity")
            .map_err(|error| error.to_string())?;
        entry_builder.build_store(ledger, ptr_ty.const_null())
            .map_err(|error| error.to_string())?;
        entry_builder.build_store(capacity, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        let ledger = NativeHandleLedger { buffer: ledger, capacity };
        self.begin_live_native_projection_scope(ledger);

        let projection_function = self.current_function().get_name().to_string_lossy().into_owned();
        let previous_projection = self.live_native_projection.replace((projection_function, ledger));
        let previous_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let projection = match &source_type {
            HirType::Array(element) => self.compile_native_array_to_json_with_undefined(
                native.into_pointer_value(), element, true,
            ),
            HirType::Tuple(elements) => self.compile_native_tuple_to_json_with_undefined(
                native.into_pointer_value(), elements, true,
            ),
            HirType::Object(_) => self.compile_native_object_to_json_with_undefined(
                native.into_pointer_value(), &source_type, true,
            ),
            _ => unreachable!(),
        };
        self.compiling_quickjs_dynamic_arguments = previous_dynamic_arguments;
        self.live_native_projection = previous_projection;
        let projected = projection?;
        let wire = self.builder.build_call(
            self.module.get_function("thaw_json_graph_encode").unwrap(),
            &[projected.into()], "native_projection_graph_wire",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projection graph encoder returned no wire")?;
        let wire = self.compile_check_json_graph_result_error(wire, &[projected], &[], &[])?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(), &[projected.into()],
            "destroy_native_projection_tree",
        ).map_err(|error| error.to_string())?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_retain_graph_result").unwrap(),
            &[wire.into()], "native_projection_graph_result",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projection graph decoder returned no result")?.into_struct_value();
        let retained = self.builder.build_extract_value(result, 0, "native_projection_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "native_projection_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(), &[wire.into()],
            "destroy_native_projection_wire",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        // Success transfers all nested values into the retained JS graph.
        self.release_native_handle_ledger(ledger)?;
        self.finish_live_native_projection_scope()?;
        // If this projection occurs while evaluating a typed QuickJS call
        // argument, the returned object handle belongs to that outer call's
        // lease scope. This makes a later argument exception release it.
        self.track_fresh_event_key_handle(retained.into_int_value())?;
        Ok(retained)
    }

    fn compile_retain_event_key(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value, is_symbol] = args else {
            return Err("retainEventKey expects a native scalar and kind flag".into());
        };
        let value_type = self.expr_hir_type(value).ok_or("native scalar has no inferred type")?;
        if !matches!(value_type, HirType::Str | HirType::Symbol) {
            return Err("retainEventKey only accepts native strings and Symbols".into());
        }
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let value = self.compile_expr(value)?.into_pointer_value();
        let is_symbol = self.compile_expr(is_symbol)?.into_int_value();
        let is_symbol = self.builder.build_int_z_extend(
            is_symbol, self.context.i8_type(), "native_scalar_is_symbol_i8",
        ).map_err(|error| error.to_string())?;
        let array = self.builder.build_call(
            self.module.get_function("thaw_json_array_new").unwrap(), &[], "native_scalar_wire",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_array_new returned no native scalar wire")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_array_push_native_wtf8_string").unwrap(),
            &[array.into(), value.into()], "native_scalar_wire_push",
        ).map_err(|error| error.to_string())?;
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_stringify").unwrap(),
            &[array.into()], "native_scalar_wire_json",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_stringify returned no native scalar JSON")?;
        let json = self.compile_check_json_stringify_error_with_cleanup(json, &[array])?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_event_key_handle_result").unwrap(),
            &[json.into(), is_symbol.into()], "native_scalar_handle_result",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native scalar bridge returned no result")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "native_scalar_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "native_scalar_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(), &[json.into()],
            "destroy_native_scalar_wire_json",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(), &[array.into()],
            "destroy_native_scalar_wire_array",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        self.track_fresh_event_key_handle(handle.into_int_value())?;
        Ok(handle)
    }

    fn compile_set_dynamic_property_json(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [receiver, name, value] = args else {
            return Err("setDynamicPropertyJson expects receiver, name, and value".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let receiver = self.compile_expr(receiver)?.into_int_value();
        let name = self.compile_expr(name)?.into_pointer_value();
        let value = self.compile_expr(value)?.into_pointer_value();
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "dynamic_set_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[array.into(), value.into()],
                "push_dynamic_set_value",
            )
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[array.into()],
                "dynamic_set_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[array.into()])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_set_property_json_graph_result")
                    .unwrap(),
                &[receiver.into(), name.into(), args_json.into()],
                "dynamic_property_json_set",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let returned = self
            .builder
            .build_extract_value(result, 0, "dynamic_property_json_set_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[returned.into()],
                "destroy_dynamic_set_result",
            )
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_property_json_set_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[args_json.into()], "destroy_dynamic_set_args_string",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[array.into()], "destroy_dynamic_set_args_json",
        ).map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(self.context.bool_type().const_int(1, false).into())
    }

    fn compile_lookup_native_object(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source, HirExpr::Lit(HirLit::Str(layout))] = args else {
            return Err("native object lookup expects object and static layout".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let source = self.compile_expr(source)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "cached_native_object_layout")
            .map_err(|error| error.to_string())?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_lookup_native_object").unwrap(),
            &[source.into(), layout.as_pointer_value().into()], "lookup_native_object",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native object lookup returned no result")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "cached_native_object_handle")
            .map_err(|error| error.to_string())?.into_int_value();
        let error = self.builder.build_extract_value(result, 1, "cached_native_object_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let present = self.builder.build_int_compare(IntPredicate::NE, handle,
            self.context.i64_type().const_zero(), "cached_native_object_present")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(HirType::JsValue)))?
            .into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "cached_native_object_tag")
            .map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, handle, 1, "cached_native_object_value")
            .map(|value| value.into_struct_value().into()).map_err(|error| error.to_string())
    }

    /// Links the Json array behind a JS shadow (`item`) to the typed native array `array` just
    /// stored from it, so both sides keep sharing one array (`kind`: 0 number, 1 boolean, 2 string).
    fn compile_json_link_native(&mut self, name: &str, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [item, native, detail] = args else {
            return Err("json link expects a handle, a native value and a kind".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let handle = self.compile_expr(item)?.into_int_value();
        let native = self.compile_expr(native)?.into_pointer_value();
        let serial = self.builder.build_call(
            self.module.get_function("thaw_js_origin_serial").unwrap(), &[handle.into()], "origin_serial",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("origin serial returned no value")?;
        let (function, detail): (&str, BasicMetadataValueEnum<'ctx>) = match (name, detail) {
            ("__thaw_json_link_native", HirExpr::Lit(HirLit::F64(kind))) =>
                ("thaw_json_link_native_by_serial",
                 self.context.i8_type().const_int(*kind as u64, false).into()),
            ("__thaw_json_link_native_object" | "__thaw_json_link_native_tree", HirExpr::Lit(HirLit::Str(layout))) => (
                if name == "__thaw_json_link_native_tree" {
                    "thaw_json_link_native_tree_by_serial"
                } else {
                    "thaw_json_link_native_object_by_serial"
                },
                self.builder.build_global_string_ptr(layout, "native_link_layout")
                    .map_err(|error| error.to_string())?.as_pointer_value().into(),
            ),
            _ => return Err("json link detail must be a literal".into()),
        };
        self.builder.build_call(
            self.module.get_function(function).unwrap(),
            &[serial.into(), native.into(), detail], "link_native",
        ).map_err(|error| error.to_string())?;
        Ok(self.context.i8_type().const_zero().into())
    }

    /// Recovers the native owner behind a live-view wrapper (`item`) if it was interned with
    /// `layout`; the third argument only supplies the owner's static type.
    fn compile_native_view_target(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [item, HirExpr::Lit(HirLit::Str(layout)), witness] = args else {
            return Err("native view target expects handle, static layout, and type witness".into());
        };
        let owner_type = self.expr_hir_type(witness)
            .ok_or("native view target requires a typed witness")?;
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let handle = self.compile_expr(item)?.into_int_value();
        let layout = self.builder.build_global_string_ptr(layout, "native_view_target_layout")
            .map_err(|error| error.to_string())?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_native_object_pointer").unwrap(),
            &[handle.into(), layout.as_pointer_value().into()], "native_view_target_pointer",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native view target returned no result")?.into_struct_value();
        let pointer = self.builder.build_extract_value(result, 0, "native_view_target_word")
            .map_err(|error| error.to_string())?.into_int_value();
        let error = self.builder.build_extract_value(result, 1, "native_view_target_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let present = self.builder.build_int_compare(IntPredicate::NE, pointer,
            self.context.i64_type().const_zero(), "native_view_target_present")
            .map_err(|error| error.to_string())?;
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let owner = self.builder.build_int_to_ptr(pointer, ptr_type, "native_view_target_owner")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(owner_type)))?.into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "native_view_target_tag").map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, owner, 1, "native_view_target_value")
            .map(|value| value.into_struct_value().into()).map_err(|error| error.to_string())
    }

    fn compile_require_native_owner(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner] = args else {
            return Err("native owner check expects one object".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let valid = self.builder.build_call(
            self.module.get_function("thaw_arena_contains_allocation").unwrap(),
            &[owner.into()], "native_owner_allocation",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native owner check returned no value")?.into_int_value();
        let missing = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, valid, self.context.i8_type().const_zero(),
            "native_owner_missing",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_block = self.context.append_basic_block(function, "native_owner_error");
        let continuation = self.context.append_basic_block(function, "native_owner_ready");
        self.builder.build_conditional_branch(missing, error_block, continuation)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_block);
        self.compile_throw_builtin_error("TypeError", "Live native object requires arena-owned storage")?;
        self.builder.position_at_end(continuation);
        Ok(self.context.i8_type().const_zero().into())
    }

    pub(super) fn compile_register_native_object_layout(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Bool(required))] = args else {
            return Err("native layout registration expects owner and required flag".into());
        };
        let Some(HirType::Object(fields)) = self.expr_hir_type(owner) else {
            return Err("native layout registration requires a typed object".into());
        };
        let mut layout = String::new();
        for (index, (name, _)) in fields.iter().enumerate() {
            layout.push_str(&format!("{}:", object_field_offset(&fields, index)?));
            for byte in name.as_bytes() {
                layout.push_str(&format!("{byte:02x}"));
            }
            layout.push(';');
        }
        let descriptor = thaw_hir::native_object_layout_token(&fields);
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(&layout, "native_field_offsets")
            .map_err(|error| error.to_string())?;
        let descriptor = self.builder.build_global_string_ptr(&descriptor, "native_field_descriptor")
            .map_err(|error| error.to_string())?;
        let accepted = self.builder.build_call(
            self.module.get_function("thaw_object_register_field_offsets").unwrap(),
            &[owner.into(), layout.as_pointer_value().into(), descriptor.as_pointer_value().into()],
            "register_native_field_offsets",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native layout registration returned no value")?.into_int_value();
        if *required {
            let rejected = self.builder.build_not(accepted, "native_layout_rejected")
                .map_err(|error| error.to_string())?;
            let function = self.current_function();
            let invalid = self.context.append_basic_block(function, "native_layout_invalid");
            let ready = self.context.append_basic_block(function, "native_layout_ready");
            self.builder.build_conditional_branch(rejected, invalid, ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(invalid);
            self.compile_throw_builtin_error("TypeError", "Nonprefix native object requires arena-owned storage")?;
            self.builder.position_at_end(ready);
        }
        Ok(accepted.into())
    }

    fn compile_register_native_object_projector(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Str(layout)), factory] = args else {
            return Err("native projector registration expects owner, layout, and factory".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        if !self.projects_native_objects {
            // The full typed factory is never emitted into native-only
            // programs, even though HIR records an erasure site.
            return Ok(self.context.bool_type().const_int(1, false).into());
        }
        let layout = self.builder.build_global_string_ptr(layout, "native_projector_layout")
            .map_err(|error| error.to_string())?;
        let factory = self.compile_expr(factory)?.into_pointer_value();
        let accepted = self.builder.build_call(
            self.module.get_function("thaw_object_register_projector").unwrap(),
            &[owner.into(), layout.as_pointer_value().into(), factory.into()],
            "register_native_projector",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projector registration returned no value")?.into_int_value();
        let failed = self.builder.build_not(accepted, "native_projector_failed")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_block = self.context.append_basic_block(function, "native_projector_error");
        let continuation = self.context.append_basic_block(function, "native_projector_ready");
        self.builder.build_conditional_branch(failed, error_block, continuation)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_block);
        self.compile_throw_builtin_error("TypeError", "Incompatible native object projection")?;
        self.builder.position_at_end(continuation);
        Ok(accepted.into())
    }

    fn compile_release_native_projection_callbacks(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [callbacks] = args else {
            return Err("native projection cleanup expects its callback array".into());
        };
        let callbacks = self.compile_expr(callbacks)?.into_pointer_value();
        let callbacks = self.compile_array_data(callbacks)?;
        self.builder.build_call(
            self.module.get_function("thaw_js_release_native_handle_array").unwrap(),
            &[callbacks.into()], "release_unbuilt_native_projection",
        ).map_err(|error| error.to_string())?;
        Ok(self.context.i8_type().const_zero().into())
    }

    fn compile_register_native_exception_describer(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, factory] = args else {
            return Err("exception describer registration expects owner and factory".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let factory = self.compile_expr(factory)?.into_pointer_value();
        Ok(self.builder.build_call(
            self.module.get_function("thaw_object_register_describer").unwrap(),
            &[owner.into(), factory.into()], "register_exception_describer",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exception describer registration returned no value")?)
    }

    fn compile_lookup_native_exception_describer(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner] = args else {
            return Err("exception describer lookup expects an owner".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let describer = self.builder.build_call(
            self.module.get_function("thaw_object_describer").unwrap(),
            &[owner.into()], "lookup_exception_describer",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exception describer lookup returned no value")?.into_pointer_value();
        let present = self.builder.build_is_not_null(describer, "exception_describer_present")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(HirType::Function(
            Vec::new(), Box::new(HirType::Json)))))?.into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "exception_describer_tag").map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, describer, 1, "exception_describer_value")
            .map(|value| value.into_struct_value().into()).map_err(|error| error.to_string())
    }

    fn compile_lookup_native_projector(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Str(layout))] = args else {
            return Err("native projector lookup expects owner and static layout".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "native_projector_query")
            .map_err(|error| error.to_string())?;
        let projector = self.builder.build_call(
            self.module.get_function("thaw_object_projector").unwrap(),
            &[owner.into(), layout.as_pointer_value().into()], "lookup_native_projector",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projector lookup returned no value")?.into_pointer_value();
        let present = self.builder.build_is_not_null(projector, "native_projector_present")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(HirType::Function(
            Vec::new(), Box::new(HirType::JsValue)))))?.into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "native_projector_tag").map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, projector, 1, "native_projector_value")
            .map(|value| value.into_struct_value().into()).map_err(|error| error.to_string())
    }

    fn compile_intern_native_object(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source, HirExpr::Lit(HirLit::Str(layout)), candidate] = args else {
            return Err("native object interning expects object, static layout, and wrapper".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let source = self.compile_expr(source)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "native_object_layout")
            .map_err(|error| error.to_string())?;
        let candidate = self.compile_expr(candidate)?.into_int_value();
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_intern_native_object").unwrap(),
            &[source.into(), layout.as_pointer_value().into(), candidate.into()],
            "intern_native_object",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native object interning returned no value")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "interned_native_object_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "interned_native_object_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(handle)
    }
}
